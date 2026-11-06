//! Reading a wishlist's source: a pasted tracklist, or a CSV file such as
//! Exportify's (ROADMAP 2.3). Text in, tracks out; nothing here touches the
//! database or the disk.
//!
//! A line or row that gives no artist and title is never dropped silently:
//! it comes back in [`Parsed::skipped`] with its place and why.

use serde::Serialize;
use specta::Type;

use super::matching::normalize_isrc;

/// One track as a source gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WishTrack {
    pub artist: String,
    pub title: String,
    /// The version in a last bracket ("Extended Mix"), or a version column.
    pub version: Option<String>,
    pub isrc: Option<String>,
}

/// Why a line or row wasn't taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WishSkipReason {
    /// A pasted line with no " - " between an artist and a title.
    NoSeparator,
    MissingArtist,
    MissingTitle,
    /// A CSV row that couldn't be read.
    Unreadable,
}

/// A line or row that wasn't taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WishSkippedLine {
    /// Its line in the pasted text, or its row in the file (the header is
    /// row 1), counting from 1.
    pub line: u32,
    /// What it said, cut to a short length.
    pub text: String,
    pub reason: WishSkipReason,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub tracks: Vec<WishTrack>,
    /// The first [`MAX_SKIPPED_SHOWN`] lines or rows not taken.
    pub skipped: Vec<WishSkippedLine>,
    /// How many were not taken in all.
    pub skipped_total: u32,
}

/// How many skipped lines are kept to show; the rest are only counted.
pub const MAX_SKIPPED_SHOWN: usize = 100;

impl Parsed {
    fn skip(&mut self, line: WishSkippedLine) {
        self.skipped_total = self.skipped_total.saturating_add(1);
        if self.skipped.len() < MAX_SKIPPED_SHOWN {
            self.skipped.push(line);
        }
    }
}

/// Why a CSV file can't be read as a wishlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsvProblem {
    NoArtistColumn,
    NoTitleColumn,
    Unreadable,
}

const SHOWN_LEN: usize = 120;

fn shown(text: &str) -> String {
    text.chars().take(SHOWN_LEN).collect()
}

fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Turns a record's byte position into its row, in one pass over the file:
/// positions only move forward, so the newlines are counted from where the
/// last count stopped. The reader leaves blank lines out and reports a record
/// at the end of the one before, so the blank lines in front are counted too.
struct Rows<'a> {
    bytes: &'a [u8],
    at: usize,
    newlines: usize,
    /// Bytes looked at, in all.
    scanned: usize,
}

impl<'a> Rows<'a> {
    fn new(text: &'a str) -> Self {
        Rows {
            bytes: text.as_bytes(),
            at: 0,
            newlines: 0,
            scanned: 0,
        }
    }

    fn row_at(&mut self, position: &::csv::Position) -> u32 {
        let at = usize::try_from(position.byte())
            .unwrap_or(usize::MAX)
            .min(self.bytes.len());
        if at < self.at {
            (self.at, self.newlines) = (0, 0);
        }
        let new = &self.bytes[self.at..at];
        self.scanned += new.len();
        self.newlines += new.iter().filter(|b| **b == b'\n').count();
        self.at = at;
        let mut row = self.newlines + 1;
        for b in &self.bytes[at..] {
            self.scanned += 1;
            match b {
                b'\n' => row += 1,
                b'\r' => {}
                _ => break,
            }
        }
        line_number(row - 1)
    }
}

fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

// ---- paste ----------------------------------------------------------------

/// A pasted tracklist: one track per line, "Artist - Title (Version)".
/// A dash, en dash or em dash with a space each side separates the two.
/// A number in front ("1.", "12)") and a time in front ("0:00", "[01:02:03]")
/// are set aside. Blank lines are ignored.
pub fn paste(text: &str) -> Parsed {
    let mut out = Parsed::default();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if line.is_empty() {
            continue;
        }
        let skipped = |reason| WishSkippedLine {
            line: line_number(index),
            text: shown(line),
            reason,
        };
        let body = strip_lead(line);
        let Some((artist, title)) = split_artist_title(body) else {
            out.skip(skipped(WishSkipReason::NoSeparator));
            continue;
        };
        let (artist, title) = (tidy(artist), tidy(title));
        if artist.is_empty() {
            out.skip(skipped(WishSkipReason::MissingArtist));
        } else if title.is_empty() {
            out.skip(skipped(WishSkipReason::MissingTitle));
        } else {
            let (title, version) = split_version(&title);
            out.tracks.push(WishTrack {
                artist,
                title,
                version,
                isrc: None,
            });
        }
    }
    out
}

/// The text after a leading number or time, whichever come first (twice at
/// most: "1. 00:00 Artist - Title").
fn strip_lead(line: &str) -> &str {
    let mut rest = line;
    for _ in 0..2 {
        let next = strip_time(rest)
            .or_else(|| strip_number(rest))
            .map(str::trim_start);
        match next {
            Some(next) => rest = next,
            None => break,
        }
    }
    rest
}

/// `n` to `max` ASCII digits at the start of `s`, and what follows.
fn digits(s: &str, min: usize, max: usize) -> Option<&str> {
    let count = s.bytes().take_while(u8::is_ascii_digit).count();
    (min..=max).contains(&count).then(|| &s[count..])
}

/// "1. " or "12) " in front.
fn strip_number(s: &str) -> Option<&str> {
    let rest = digits(s, 1, 4)?;
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    rest.starts_with(char::is_whitespace).then_some(rest)
}

/// "0:00", "12:34", "1:02:03", in brackets or not, then an optional dash.
fn strip_time(s: &str) -> Option<&str> {
    let (open, body) = match s.chars().next()? {
        '[' => (Some(']'), &s[1..]),
        '(' => (Some(')'), &s[1..]),
        _ => (None, s),
    };
    let mut rest = digits(body, 1, 2)?.strip_prefix(':')?;
    rest = digits(rest, 2, 2)?;
    if let Some(more) = rest.strip_prefix(':') {
        rest = digits(more, 2, 2)?;
    }
    if let Some(close) = open {
        rest = rest.strip_prefix(close)?;
    }
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    let trimmed = rest.trim_start();
    for dash in ['-', '\u{2013}', '\u{2014}'] {
        if let Some(after) = trimmed.strip_prefix(dash) {
            if after.starts_with(char::is_whitespace) {
                return Some(after);
            }
        }
    }
    Some(rest)
}

/// The text either side of the first spaced dash.
fn split_artist_title(body: &str) -> Option<(&str, &str)> {
    ["-", "\u{2013}", "\u{2014}"]
        .iter()
        .filter_map(|dash| {
            let separator = format!(" {dash} ");
            body.find(&separator)
                .map(|at| (at, &body[..at], &body[at + separator.len()..]))
        })
        .min_by_key(|(at, ..)| *at)
        .map(|(_, artist, title)| (artist, title))
}

/// "Title (Extended Mix)" as "Title" and "Extended Mix". A last bracket
/// that is a featured artist stays in the title, and so does a title that is
/// only a bracket.
fn split_version(title: &str) -> (String, Option<String>) {
    let unsplit = || (title.to_owned(), None);
    let Some(open_char) = title
        .strip_suffix(')')
        .map(|_| '(')
        .or_else(|| title.strip_suffix(']').map(|_| '['))
    else {
        return unsplit();
    };
    let closer = if open_char == '(' { ')' } else { ']' };
    let mut depth = 0usize;
    let mut open_at = None;
    for (at, c) in title.char_indices().rev() {
        if c == closer {
            depth += 1;
        } else if c == open_char {
            depth -= 1;
            if depth == 0 {
                open_at = Some(at);
                break;
            }
        }
    }
    let Some(at) = open_at else {
        return unsplit();
    };
    let inner = title[at + 1..title.len() - 1].trim();
    let before = title[..at].trim();
    let lower = inner.to_lowercase();
    let featuring = ["feat.", "feat ", "ft.", "ft ", "featuring", "with "]
        .iter()
        .any(|word| lower.starts_with(word));
    if inner.is_empty() || before.is_empty() || featuring {
        return unsplit();
    }
    (before.to_owned(), Some(inner.to_owned()))
}

// ---- CSV ------------------------------------------------------------------

const ARTIST_NAMES: [&str; 5] = [
    "artist name(s)",
    "artist names",
    "artist name",
    "artists",
    "artist",
];
const TITLE_NAMES: [&str; 7] = [
    "track name",
    "track title",
    "title",
    "song name",
    "song",
    "track",
    "name",
];
const VERSION_NAMES: [&str; 2] = ["version", "mix"];
const ISRC_NAMES: [&str; 1] = ["isrc"];

/// The first of `names` (in that order) that is a header.
fn column(headers: &[String], names: &[&str]) -> Option<usize> {
    names
        .iter()
        .find_map(|name| headers.iter().position(|h| h == name))
}

/// A CSV file with a header row. Columns are found by name, in any order
/// (Exportify's "Track Name" and "Artist Name(s)", or just "artist" and
/// "title"; "ISRC" and "Version" are used when there); others are ignored.
pub fn csv(text: &str) -> Result<Parsed, CsvProblem> {
    csv_counting(text).map(|(parsed, _)| parsed)
}

/// [`csv`], and how many bytes the row counting looked at.
pub(super) fn csv_counting(text: &str) -> Result<(Parsed, usize), CsvProblem> {
    let text = text.trim_start_matches('\u{feff}');
    let mut reader = ::csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|_| CsvProblem::Unreadable)?
        .iter()
        .map(|h| h.trim().to_lowercase())
        .collect();
    let artist_at = column(&headers, &ARTIST_NAMES).ok_or(CsvProblem::NoArtistColumn)?;
    let title_at = column(&headers, &TITLE_NAMES).ok_or(CsvProblem::NoTitleColumn)?;
    let version_at = column(&headers, &VERSION_NAMES);
    let isrc_at = column(&headers, &ISRC_NAMES);

    let mut rows = Rows::new(text);
    let mut out = Parsed::default();
    for (index, record) in reader.records().enumerate() {
        // The header is row 1; blank lines are not records but are rows.
        let fallback = line_number(index + 1);
        let record = match record {
            Ok(record) => record,
            Err(error) => {
                out.skip(WishSkippedLine {
                    line: error.position().map_or(fallback, |p| rows.row_at(p)),
                    text: String::new(),
                    reason: WishSkipReason::Unreadable,
                });
                continue;
            }
        };
        let row = record.position().map_or(fallback, |p| rows.row_at(p));
        let cell = |at: usize| tidy(record.get(at).unwrap_or(""));
        let optional = |at: Option<usize>| at.map(cell).filter(|text| !text.is_empty());
        let (artist, title) = (cell(artist_at).replace(';', ", "), cell(title_at));
        let reason = if artist.is_empty() {
            Some(WishSkipReason::MissingArtist)
        } else if title.is_empty() {
            Some(WishSkipReason::MissingTitle)
        } else {
            None
        };
        if let Some(reason) = reason {
            if record.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            out.skip(WishSkippedLine {
                line: row,
                text: shown(&record.iter().collect::<Vec<_>>().join(", ")),
                reason,
            });
            continue;
        }
        out.tracks.push(WishTrack {
            artist: tidy(&artist),
            title,
            version: optional(version_at),
            isrc: optional(isrc_at).and_then(|isrc| normalize_isrc(&isrc)),
        });
    }
    Ok((out, rows.scanned))
}
