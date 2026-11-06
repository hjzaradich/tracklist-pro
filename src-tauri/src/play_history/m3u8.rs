//! Reading an m3u8 playlist, as rekordbox writes one from its History
//! (right-click a History list, "Export a playlist to a file"). Bytes in,
//! plays out; nothing here touches the database or the disk.
//!
//! Tolerant by design: an optional `#EXTM3U` header, `#EXTINF:<seconds>,<Artist> - <Title>`
//! lines that describe the path after them, absolute or relative paths,
//! `file://` URLs, a byte order mark and CRLF line ends are all taken. Other
//! `#` lines are comments and are passed over. A line that can't be a play
//! (not UTF-8, too long, a control character, a broken `#EXTINF`) is skipped
//! and counted, never fatal.

use crate::rekordbox::location;

/// The biggest file read, in bytes.
pub const MAX_BYTES: usize = 5 * 1024 * 1024;
/// The longest line taken as a path or `#EXTINF`, in bytes.
const MAX_LINE_BYTES: usize = 4096;

/// One play as the file gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path as written, with `file://` URLs decoded.
    pub path: String,
    pub artist: Option<String>,
    pub title: Option<String>,
}

/// What a file came to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub entries: Vec<Entry>,
    /// Lines that couldn't be read as a play or its description.
    pub skipped: u32,
}

/// Reads the plays of an m3u8 file, in order. Everything past
/// [`MAX_BYTES`] is not read; the caller refuses such a file first.
pub fn parse(bytes: &[u8]) -> Parsed {
    let bytes = &bytes[..bytes.len().min(MAX_BYTES)];
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut parsed = Parsed::default();
    // The `#EXTINF` waiting for its path.
    let mut described: Option<(Option<String>, Option<String>)> = None;
    for raw in bytes.split(|&b| b == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let Ok(line) = std::str::from_utf8(raw) else {
            parsed.skipped += 1;
            described = None;
            continue;
        };
        // A tab inside a title is a space, not a reason to drop the line.
        let line = line.replace('\t', " ");
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_BYTES || line.chars().any(char::is_control) {
            parsed.skipped += 1;
            // A skipped line may have been the path the description was for.
            described = None;
            continue;
        }
        if let Some(info) = strip_prefix_ignore_case(line, "#EXTINF:") {
            match extinf(info) {
                Some(found) => described = Some(found),
                None => parsed.skipped += 1,
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let Some(path) = path_of(line) else {
            parsed.skipped += 1;
            described = None;
            continue;
        };
        let (artist, title) = described.take().unwrap_or((None, None));
        parsed.entries.push(Entry {
            path,
            artist,
            title,
        });
    }
    parsed
}

fn strip_prefix_ignore_case<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let head = line.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &line[prefix.len()..])
}

/// `<seconds>,<display>`: the display split at its first ` - ` into artist
/// and title; without one it is all title. The seconds aren't kept, but
/// they must be a number (`-1` for unknown), or the line is broken.
fn extinf(info: &str) -> Option<(Option<String>, Option<String>)> {
    let (seconds, display) = info.split_once(',')?;
    // Extended attributes (`-1 tvg-id="x"`) may follow the number.
    let number = seconds.split_whitespace().next()?;
    number.parse::<f64>().ok().filter(|n| n.is_finite())?;
    let clean = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
    Some(match display.split_once(" - ") {
        Some((artist, title)) => (clean(artist), clean(title)),
        None => (None, clean(display)),
    })
}

/// A path line as kept: a `file://` URL decoded, anything else as written.
fn path_of(line: &str) -> Option<String> {
    if strip_prefix_ignore_case(line, "file:").is_some() {
        let decoded = location::decode(line).ok()?;
        return decoded.as_file().map(|p| p.as_str().to_owned());
    }
    // Another URL scheme (http:, a streaming id) names no file.
    if let Some((scheme, _)) = line.split_once(':') {
        let drive = scheme.len() == 1 && scheme.as_bytes()[0].is_ascii_alphabetic();
        if !drive
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        {
            return None;
        }
    }
    Some(line.to_owned())
}

/// The date in a name such as `2026-05-03` or `HISTORY 2026-05-03 (2)`:
/// the first `YYYY-MM-DD` that is a real day.
pub fn date_in(name: &str) -> Option<String> {
    let b = name.as_bytes();
    (0..b.len().saturating_sub(9)).find_map(|i| {
        let s = name.get(i..i + 10)?;
        let edge_ok = |at: Option<&u8>| !at.is_some_and(u8::is_ascii_digit);
        let alone = edge_ok(i.checked_sub(1).and_then(|j| b.get(j))) && edge_ok(b.get(i + 10));
        (alone && real_day(s)).then(|| s.to_owned())
    })
}

/// Whether `YYYY-MM-DD` names a day of the calendar.
pub fn real_day(s: &str) -> bool {
    let shaped = s.len() == 10
        && s.bytes().enumerate().all(|(i, c)| match i {
            4 | 7 => c == b'-',
            _ => c.is_ascii_digit(),
        });
    if !shaped {
        return false;
    }
    let n = |r: std::ops::Range<usize>| s[r].parse::<u32>().unwrap_or(0);
    let (y, m, d) = (n(0..4), n(5..7), n(8..10));
    if y == 0 {
        return false;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}
