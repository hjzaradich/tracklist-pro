//! Reads cut and rework labels (1bA-7, 1bA-8) out of a bracket's content,
//! a " - " segment or the end of a title. The spellings are in
//! [`super::tables`].
//!
//! Labels are read from the end backwards, a whole spelling at a time, so
//! "Quill Ashby Extended Remix" is a remix (by Quill Ashby) and an
//! extended cut. Whatever is left in front must be a name a rework label
//! can take; if it isn't, the content isn't labels at all.

use super::tables::{
    BY_WORDS, CUT_LABELS, FEAT_WORDS, LABEL_SEPARATORS, LIVE_PLACE_WORDS, MASHUP_SEPARATORS,
    MAX_LABEL_WORDS, NAMED_MIX_WORDS, NON_NAME_WORDS, REWORK_LABELS,
};
use super::tokenize::is_dash;
use super::{MarkerKind, VersionClass};

/// A label found in some text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Found {
    pub kind: MarkerKind,
    pub detail: Option<String>,
}

/// A word of the text, or a separator between labels.
struct Word {
    /// Lowercase, without dots, apostrophes and inner hyphens. Empty for a
    /// separator.
    plain: String,
    end: usize,
}

impl Word {
    fn is_separator(&self) -> bool {
        self.plain.is_empty()
    }
}

/// How a word is spelled for matching: "V.I.P." is `vip`, "Re-Fix" is
/// `refix`.
pub(super) fn plain(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '@' | '#'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn words(text: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut start = None;
    let push = |start: usize, end: usize, words: &mut Vec<Word>| {
        let piece = &text[start..end];
        // A dash standing alone ("Intro - Clean") separates labels.
        let plain = if piece.chars().all(is_dash) {
            String::new()
        } else {
            plain(piece)
        };
        words.push(Word { plain, end });
    };
    for (at, c) in text.char_indices() {
        let separator = LABEL_SEPARATORS.contains(&c);
        if c.is_whitespace() || separator {
            if let Some(from) = start.take() {
                push(from, at, &mut words);
            }
            if separator {
                words.push(Word {
                    plain: String::new(),
                    end: at + c.len_utf8(),
                });
            }
        } else if start.is_none() {
            start = Some(at);
        }
    }
    if let Some(from) = start {
        push(from, text.len(), &mut words);
    }
    words
}

/// The kind a spelling stands for.
fn lookup(spelling: &str) -> Option<MarkerKind> {
    CUT_LABELS
        .iter()
        .chain(REWORK_LABELS)
        .find(|(_, spellings)| spellings.iter().any(|s| table_spelling(s) == spelling))
        .map(|&(kind, _)| kind)
}

/// A table row, spelled the way [`words`] spells text.
fn table_spelling(row: &str) -> String {
    row.split(' ').map(plain).collect::<Vec<_>>().join(" ")
}

/// The label that ends just before word `end`, and how many words it is.
fn label_before(words: &[Word], end: usize) -> Option<(MarkerKind, usize)> {
    (1..=MAX_LABEL_WORDS.min(end)).rev().find_map(|n| {
        let run = &words[end - n..end];
        if run.iter().any(Word::is_separator) {
            return None;
        }
        let spelling = run
            .iter()
            .map(|w| w.plain.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        lookup(&spelling).map(|kind| (kind, n))
    })
}

/// A year, as written beside "Remaster".
fn is_year(word: &Word) -> bool {
    word.plain.len() == 4
        && word.plain.chars().all(|c| c.is_ascii_digit())
        && (word.plain.starts_with("19") || word.plain.starts_with("20"))
}

/// Reads labels from the end of `words` backwards. Returns the kinds, in
/// the order written, and how many words in front weren't labels. A year
/// on either side of a remaster label belongs to it.
fn labels_at_end(words: &[Word]) -> (Vec<MarkerKind>, usize) {
    let mut end = words.len();
    let mut kinds = Vec::new();
    loop {
        let mut cursor = end;
        while cursor > 0 && words[cursor - 1].is_separator() {
            cursor -= 1;
        }
        let year_after = cursor > 1 && is_year(&words[cursor - 1]);
        let matched = label_before(words, cursor).or_else(|| {
            year_after
                .then(|| label_before(words, cursor - 1))
                .flatten()
                .filter(|(kind, _)| *kind == MarkerKind::Remaster)
                .map(|(kind, n)| (kind, n + 1))
        });
        let Some((kind, n)) = matched else { break };
        kinds.push(kind);
        end = cursor - n;
        if kind == MarkerKind::Remaster && end > 0 && is_year(&words[end - 1]) {
            end -= 1;
        }
    }
    kinds.reverse();
    (kinds, end)
}

/// Intro and Outro together are Intro-Outro; a kind is listed once.
fn tidy(kinds: Vec<MarkerKind>) -> Vec<MarkerKind> {
    let both = kinds.contains(&MarkerKind::Intro) && kinds.contains(&MarkerKind::Outro);
    let mut out: Vec<MarkerKind> = Vec::new();
    for kind in kinds {
        let kind = match kind {
            MarkerKind::Intro | MarkerKind::Outro if both => MarkerKind::IntroOutro,
            other => other,
        };
        if !out.contains(&kind) {
            out.push(kind);
        }
    }
    out
}

/// A name a rework label can take: it has a letter or a digit.
fn is_name(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

/// Whether the words in front of a label can be whose rework it is: at
/// least one of them is neither an ordinary word ("Main", "DJ", "12") nor a
/// label word ("Extended").
fn is_person(text: &str) -> bool {
    words(text).iter().any(|word| {
        word.plain.chars().any(char::is_alphanumeric)
            && !NON_NAME_WORDS.contains(&word.plain.as_str())
            && lookup(&word.plain).is_none()
    })
}

/// Where the text in front of word `end` stops, separators at its end
/// left out.
fn name_end(words: &[Word], mut end: usize) -> usize {
    while end > 0 && words[end - 1].is_separator() {
        end -= 1;
    }
    match end {
        0 => 0,
        _ => words[end - 1].end,
    }
}

/// Reads the content of a bracket or a " - " segment as labels. `None`
/// when it isn't made of labels (and, at most, one name in front of a
/// rework label).
pub(super) fn parse_content(text: &str) -> Option<Vec<Found>> {
    let words = words(text);
    if words.is_empty() {
        return None;
    }
    if let Some(found) = label_then_name(text, &words) {
        return Some(vec![found]);
    }
    let (mut kinds, rest) = labels_at_end(&words);
    if kinds.is_empty() {
        return named_mix(text, &words).map(|found| vec![found]);
    }
    let name = text[..name_end(&words, rest)].trim();
    let detail = if name.is_empty() {
        None
    } else if !is_person(name) {
        return None;
    } else if kinds.iter().any(|k| k.class() == VersionClass::Rework) {
        // Words in front of a rework label are whose rework it is.
        Some(name.to_string())
    } else if kinds[0] == MarkerKind::Edit {
        // "(Vey Sun Edit)", "(@handle edit)": an edit with a name on it is
        // a bootleg by that name.
        kinds[0] = MarkerKind::Bootleg;
        Some(name.to_string())
    } else {
        // A cut takes no name: this isn't made of labels.
        return None;
    };
    Some(
        tidy(kinds)
            .into_iter()
            .map(|kind| Found {
                kind,
                detail: match kind.class() {
                    VersionClass::Rework => detail.clone(),
                    VersionClass::Cut => None,
                },
            })
            .collect(),
    )
}

/// "Remix by Quill Ashby", "Live at Harbor Hall": a rework label, a joining
/// word, then the name.
fn label_then_name(text: &str, words: &[Word]) -> Option<Found> {
    (1..=MAX_LABEL_WORDS.min(words.len().saturating_sub(2)))
        .rev()
        .find_map(|n| {
            let run = &words[..n];
            if run.iter().any(Word::is_separator) {
                return None;
            }
            let spelling = run
                .iter()
                .map(|w| w.plain.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let kind = lookup(&spelling).filter(|k| k.class() == VersionClass::Rework)?;
            let joiner = words[n].plain.as_str();
            let joins = BY_WORDS.contains(&joiner)
                || (kind == MarkerKind::Live && LIVE_PLACE_WORDS.contains(&joiner));
            let name = text[words[n].end..].trim();
            (joins && is_name(name)).then(|| Found {
                kind,
                detail: Some(name.to_string()),
            })
        })
}

/// "Vey Sun Mix": a name and "mix" is a remix by that name.
fn named_mix(text: &str, words: &[Word]) -> Option<Found> {
    let (last, front) = words.split_last()?;
    let name = text[..name_end(front, front.len())].trim();
    (NAMED_MIX_WORDS.contains(&last.plain.as_str()) && is_person(name)).then(|| Found {
        kind: MarkerKind::Remix,
        detail: Some(name.to_string()),
    })
}

/// Labels written as plain words at the end of a title: "Paper Harbor
/// Extended Mix". Returns the title in front, the label words as written,
/// and the kinds. The title in front may be empty ("Intro").
pub(super) fn bare_tail(text: &str) -> Option<(String, String, Vec<Found>)> {
    let words = words(text);
    let (kinds, rest) = labels_at_end(&words);
    if kinds.is_empty() {
        return None;
    }
    let cut = name_end(&words, rest);
    let title = text[..cut].trim();
    let tail = text[cut..].trim();
    let found = tidy(kinds)
        .into_iter()
        .map(|kind| Found { kind, detail: None })
        .collect();
    Some((title.to_string(), tail.to_string(), found))
}

/// Splits "Title feat. Someone" at the featuring word. Returns the text in
/// front and the names after it.
pub(super) fn split_feat(text: &str) -> Option<(String, String)> {
    let mut offset = 0;
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim();
        if FEAT_WORDS.contains(&plain(word).as_str())
            && word.chars().all(|c| c.is_alphabetic() || c == '.')
        {
            let names = text[offset + piece.len()..].trim();
            if is_name(names) {
                return Some((text[..offset].to_string(), names.to_string()));
            }
        }
        offset += piece.len();
    }
    None
}

/// Splits "A, B & C" into names. With `joined_acts`, "x" and "vs" split
/// too, as they do between the artists of a collaboration.
pub(super) fn split_names(text: &str, joined_acts: bool) -> Vec<String> {
    let mut names = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let flush = |current: &mut Vec<&str>, names: &mut Vec<String>| {
        let name = current.join(" ");
        let name = name.trim_matches(|c: char| c == ',' || c == '&' || c.is_whitespace());
        if !name.is_empty() {
            names.push(name.to_string());
        }
        current.clear();
    };
    for word in text.split_whitespace() {
        let splits =
            word == "&" || (joined_acts && MASHUP_SEPARATORS.contains(&plain(word).as_str()));
        if splits {
            flush(&mut current, &mut names);
        } else if let Some(before) = word.strip_suffix(',') {
            current.push(before);
            flush(&mut current, &mut names);
        } else {
            current.push(word);
        }
    }
    flush(&mut current, &mut names);
    names
}

/// The titles a mashup names: "A x B", "A vs B". Empty unless there are at
/// least two. Returns them with the separator as written.
pub(super) fn mashup_parts(title: &str) -> Option<(Vec<String>, String)> {
    let mut parts = Vec::new();
    let mut separator = None;
    let mut current: Vec<&str> = Vec::new();
    for word in title.split_whitespace() {
        let is_separator = MASHUP_SEPARATORS.contains(&plain(word).as_str())
            && word.chars().all(|c| c.is_alphabetic() || c == '.');
        if is_separator {
            if current.is_empty() {
                return None;
            }
            parts.push(current.join(" "));
            current.clear();
            separator.get_or_insert_with(|| word.to_string());
        } else {
            current.push(word);
        }
    }
    if current.is_empty() || parts.is_empty() {
        return None;
    }
    parts.push(current.join(" "));
    Some((parts, separator?))
}
