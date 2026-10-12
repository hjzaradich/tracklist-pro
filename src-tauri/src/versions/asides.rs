//! Brackets that cannot be a version (1bD-4): a copy counter "(1)", a
//! year "(2011)", a key "(8A)", a tempo "(124)" or "[124 BPM]", a
//! catalogue number "[ABC123]". The parser sets them apart as
//! [`Aside`]s instead of listing them among the brackets it doesn't
//! understand, so two names that differ only by them are not "unclear".
//!
//! Called harmless only when nothing in it can be a version. When in
//! doubt, a bracket stays unrecognized:
//!
//! - **A bare number can be a version's number** ("Velo (Dub) (2)" is
//!   perhaps the second dub; "Lo+Fi Run, Pt. (2)" is the second part).
//!   So a copy counter, a year or a bare tempo is an aside only when no
//!   label, no unrecognized bracket and no word like "Part" or "Vol."
//!   comes directly before it ([`blocks_a_number`]). A key with its
//!   unit-like shape, a tempo with "BPM" and a catalogue code say what
//!   they are, so they hold anywhere.
//! - **A year beside "Remaster" is a version** (two years are two
//!   cuts). Grouping puts the years of a file whose names mention a
//!   remaster back among the unrecognized ([`super::ParsedName::with_years_unsettled`]).
//! - **Only a run of them at the very end** is peeled off, from the end
//!   backwards; the first bracket that isn't harmless stops it, and what
//!   is in front of that stays as it was.
//! - Nothing here makes a name more alike than it was except by
//!   brackets that say nothing about the recording: two names whose
//!   asides of one kind disagree (two years, two keys, two tempos, two
//!   codes) are still "unclear". Copy counters never are.

use super::tables::{ASIDE_NUMBER_BLOCKERS, CATALOGUE_STOP_WORDS};
use super::{labels, Aside, AsideKind};

/// What a bracket's text is, if it is an aside: the kind, its value (the
/// same for the same thing however it is written), and whether it is a
/// bare number, which a label before it can turn into a version number.
pub(super) fn read(content: &str) -> Option<(AsideKind, String, bool)> {
    let text = content.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(value) = camelot_key(text) {
        return Some((AsideKind::Key, value, false));
    }
    if let Some(value) = tempo_with_unit(text) {
        return Some((AsideKind::Tempo, value, false));
    }
    if let Some(value) = catalogue_number(text) {
        return Some((AsideKind::CatalogueNumber, value, false));
    }
    let digits = text.chars().all(|c| c.is_ascii_digit());
    if !digits || text.starts_with('0') {
        return None;
    }
    let number: u32 = text.parse().ok()?;
    match (text.len(), number) {
        (1..=2, 1..=99) => Some((AsideKind::CopyCounter, text.to_string(), true)),
        (3, 100..=250) => Some((AsideKind::Tempo, text.to_string(), true)),
        (4, 1900..=2099) => Some((AsideKind::Year, text.to_string(), true)),
        _ => None,
    }
}

/// A Camelot key: 1 to 12 and A or B. "8A", "12b".
fn camelot_key(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let letter = chars.next_back()?.to_ascii_lowercase();
    let number = chars.as_str();
    let valid = matches!(letter, 'a' | 'b')
        && number.chars().all(|c| c.is_ascii_digit())
        && !number.starts_with('0')
        && number.parse::<u8>().is_ok_and(|n| (1..=12).contains(&n));
    valid.then(|| format!("{number}{letter}"))
}

/// A tempo that says it is one: "124 BPM", "124bpm", "98.5 bpm".
fn tempo_with_unit(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let number = lower.strip_suffix("bpm")?.trim();
    let (whole, fraction) = match number.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (number, None),
    };
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let valid = digits(whole)
        && (2..=3).contains(&whole.len())
        && !whole.starts_with('0')
        && fraction.is_none_or(|f| digits(f) && f.len() <= 2)
        && whole.parse::<u32>().is_ok_and(|n| (40..=250).contains(&n));
    valid.then(|| number.to_string())
}

/// A catalogue number: one word of 2 to 5 capital letters and 3 to 5
/// digits, "ABC123" or "ABC-123". Not when the letters are a label
/// ("RMX007"), a word that counts things ("VOL001") or another word a
/// name can end in ("DEMO123").
fn catalogue_number(text: &str) -> Option<String> {
    // The letters are capitals as written: "Demo123" and "abc123" are
    // words with a number, not a code.
    let letters: String = text.chars().take_while(char::is_ascii_alphabetic).collect();
    if !letters.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let rest = text[letters.len()..].trim_start_matches('-');
    let valid = (2..=5).contains(&letters.len())
        && (3..=5).contains(&rest.len())
        && rest.chars().all(|c| c.is_ascii_digit())
        && text.len() - letters.len() - rest.len() <= 1;
    if !valid {
        return None;
    }
    let lower = letters.to_ascii_lowercase();
    if CATALOGUE_STOP_WORDS.contains(&lower.as_str()) || labels::kind_of(&lower).is_some() {
        return None;
    }
    Some(format!("{}{rest}", letters.to_ascii_uppercase()))
}

/// Whether the last word of a title's text says that a number after it
/// belongs to it ("Pt.", "Vol.", "Part", "No."): then "(2)" is the
/// title's own.
pub(super) fn blocks_a_number(title: &str) -> bool {
    let last = title
        .split_whitespace()
        .next_back()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric() && c != '#')
                .to_lowercase()
        })
        .unwrap_or_default();
    ASIDE_NUMBER_BLOCKERS.contains(&last.as_str()) || labels::bare_tail(title).is_some()
}

impl Aside {
    pub(super) fn new(kind: AsideKind, value: String, text: &str) -> Aside {
        Aside {
            kind,
            value,
            text: text.to_string(),
        }
    }
}
