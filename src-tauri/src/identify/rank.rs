//! Candidates in, ranked suggestions out (pure: no database).
//!
//! The rules (module docs of [`super`]): values the same after a light
//! normalisation ([`key`]) are one suggestion with every source; one equal
//! to the field's current value is dropped; one the field can't take is
//! dropped (a year that isn't a number, a character rekordbox's XML can't
//! carry), so "Use" never refuses a value it was offered; the rest are
//! ranked by number of sources, then by their strongest source, then by
//! their text, so the order is the same every time.

use std::collections::BTreeMap;

use super::{Candidate, IdentifyField, IdentifySource, IdentifySuggestion};

/// How two values are compared: outer blanks dropped, runs of blanks as one
/// space, letter case ignored.
pub(super) fn key(value: &str) -> String {
    tidy(value).to_lowercase()
}

/// A value as it is offered and used: outer blanks dropped, runs of blanks
/// as one space. Letters are kept as written.
pub(super) fn tidy(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `field` can take `value` as the app's own (the checks "Use"
/// makes, [`super::choose`]).
pub(super) fn takes(field: IdentifyField, value: &str) -> bool {
    !value.is_empty()
        && crate::rekordbox_write::uncarriable(value).is_none()
        && field
            .attribute()
            .is_none_or(|attribute| crate::conflicts::fits(attribute, value))
}

/// Mixed letter case reads better than all capitals or all small letters
/// ("Velo Records" over "VELO RECORDS" and "velo records").
fn well_cased(value: &str) -> bool {
    value.chars().any(char::is_uppercase) && value.chars().any(char::is_lowercase)
}

/// The suggestions for one field from `candidates` (all of that field),
/// without the one equal to `current`.
pub(super) fn rank(
    field: IdentifyField,
    current: Option<&str>,
    candidates: impl IntoIterator<Item = Candidate>,
) -> Vec<IdentifySuggestion> {
    let current = current.map(key);
    // key → the spellings seen (with their source's strength) and the sources.
    let mut merged: BTreeMap<String, Merged> = BTreeMap::new();
    for candidate in candidates {
        let value = tidy(&candidate.value);
        if !takes(field, &value) {
            continue;
        }
        let k = key(&value);
        if current.as_deref() == Some(k.as_str()) {
            continue;
        }
        let entry = merged.entry(k).or_default();
        entry.0.push((value, candidate.source.strength()));
        if !entry.1.contains(&candidate.source) {
            entry.1.push(candidate.source);
        }
    }
    let mut out: Vec<(String, IdentifySuggestion)> = merged
        .into_iter()
        .map(|(k, (spellings, mut sources))| {
            sources.sort_by(|a, b| a.order().cmp(&b.order()));
            // The best-cased spelling; among equals the strongest source's,
            // then the first seen.
            let value = spellings
                .iter()
                .enumerate()
                .max_by_key(|(i, (s, strength))| {
                    (
                        well_cased(s),
                        std::cmp::Reverse(*strength),
                        std::cmp::Reverse(*i),
                    )
                })
                .map(|(_, (s, _))| s.clone())
                .expect("a merged value has a spelling");
            (k, IdentifySuggestion { value, sources })
        })
        .collect();
    out.sort_by(|(ka, a), (kb, b)| {
        b.sources
            .len()
            .cmp(&a.sources.len())
            .then_with(|| strongest(a).cmp(&strongest(b)))
            .then_with(|| ka.cmp(kb))
            .then_with(|| a.value.cmp(&b.value))
    });
    out.into_iter().map(|(_, s)| s).collect()
}

/// One value's spellings (each with its source's strength) and sources.
type Merged = (Vec<(String, u8)>, Vec<IdentifySource>);

fn strongest(s: &IdentifySuggestion) -> u8 {
    s.sources
        .iter()
        .map(IdentifySource::strength)
        .min()
        .unwrap_or(u8::MAX)
}
