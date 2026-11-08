//! Candidates in, ranked suggestions out (pure: no database).
//!
//! The rules (module docs of [`super`]): values the same once tidied
//! ([`tidy`]: blanks only, letter case kept) are one suggestion with every
//! source; one equal to the field's current value the same way is dropped,
//! so a fix of letter case alone ("VELO" → "Velo") is offered (1.10 counts
//! case as a change); one the field can't take is dropped ([`takes`]), so
//! "Use" never refuses a value it was offered; the rest are ranked by
//! number of sources, then by their strongest source, then by their text,
//! so the order is the same every time.

use std::collections::BTreeMap;

use super::{Candidate, IdentifyField, IdentifySource, IdentifySuggestion};

/// A value as it is compared, offered and used: outer blanks dropped, runs
/// of blanks as one space. Letters are kept as written.
pub(super) fn tidy(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `field` can take `value` as the app's own (the checks "Use"
/// makes, [`super::choose`]): not blank, nothing rekordbox's XML can't
/// carry, a value of the field's kind, and a `Year` of four digits,
/// 1000-9999 (what the tag map reads as a year; rekordbox's own reading
/// would also take "0" or "99").
pub(super) fn takes(field: IdentifyField, value: &str) -> bool {
    !value.is_empty()
        && crate::rekordbox_write::uncarriable(value).is_none()
        && field
            .attribute()
            .is_none_or(|attribute| crate::conflicts::fits(attribute, value))
        && (field != IdentifyField::Year || is_year(value))
}

fn is_year(value: &str) -> bool {
    value.len() == 4 && value.bytes().all(|b| b.is_ascii_digit()) && !value.starts_with('0')
}

/// The suggestions for one field from `candidates` (all of that field),
/// without the one equal to `current`.
pub(super) fn rank(
    field: IdentifyField,
    current: Option<&str>,
    candidates: impl IntoIterator<Item = Candidate>,
) -> Vec<IdentifySuggestion> {
    let current = current.map(tidy);
    // The tidied value → its sources.
    let mut merged: BTreeMap<String, Vec<IdentifySource>> = BTreeMap::new();
    for candidate in candidates {
        let value = tidy(&candidate.value);
        if !takes(field, &value) || current.as_deref() == Some(value.as_str()) {
            continue;
        }
        let sources = merged.entry(value).or_default();
        if !sources.contains(&candidate.source) {
            sources.push(candidate.source);
        }
    }
    let mut out: Vec<IdentifySuggestion> = merged
        .into_iter()
        .map(|(value, mut sources)| {
            sources.sort_by(|a, b| a.order().cmp(&b.order()));
            IdentifySuggestion { value, sources }
        })
        .collect();
    out.sort_by(|a, b| {
        b.sources
            .len()
            .cmp(&a.sources.len())
            .then_with(|| strongest(a).cmp(&strongest(b)))
            .then_with(|| a.value.cmp(&b.value))
    });
    out
}

fn strongest(s: &IdentifySuggestion) -> u8 {
    s.sources
        .iter()
        .map(IdentifySource::strength)
        .min()
        .unwrap_or(u8::MAX)
}
