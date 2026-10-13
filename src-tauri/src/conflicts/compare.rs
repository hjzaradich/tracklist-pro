//! Comparing two values of one field (the module docs' second table).
//!
//! Each kind reuses the parser the rekordbox reader already has: a typed
//! value is read with [`collection::track`], exactly as the reader reads a
//! `TRACK` element, and `Location` with [`location::decode`].

use std::borrow::Cow;

use unicode_normalization::{is_nfc, UnicodeNormalization};

use crate::rekordbox::{collection, location, Attrs, Date, Problem, Rgb, Track};
use crate::send_values::ANALYSIS_ATTRIBUTES;

/// rekordbox's analysis, as `conflict.field` names it: two attributes and
/// two elements. The database refuses a conflict on any of them (migration
/// 0014; a test holds this list to that one).
pub const ANALYSIS_FIELDS: [&str; 4] = [
    ANALYSIS_ATTRIBUTES[0],
    ANALYSIS_ATTRIBUTES[1],
    "TEMPO",
    "POSITION_MARK",
];

/// How a field's values are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// rekordbox's analysis: never compared, never a conflict.
    Analysis,
    /// `TrackID`: not a field (rekordbox reassigns it on import).
    TrackId,
    Location,
    Rating,
    Colour,
    Date,
    Number,
    Text,
}

/// The kind of a field, by its rekordbox XML name. A name the app doesn't
/// know is text.
pub fn kind_of(field: &str) -> Kind {
    match field {
        f if ANALYSIS_FIELDS.contains(&f) => Kind::Analysis,
        "TrackID" => Kind::TrackId,
        "Location" => Kind::Location,
        "Rating" => Kind::Rating,
        "Colour" => Kind::Colour,
        "DateAdded" | "DateModified" | "LastPlayed" => Kind::Date,
        "Size" | "TotalTime" | "DiscNumber" | "TrackNumber" | "Year" | "BitRate" | "SampleRate"
        | "PlayCount" => Kind::Number,
        _ => Kind::Text,
    }
}

/// What a value is compared by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Key<'a> {
    /// No value.
    None,
    /// Text, in NFC.
    Text(Cow<'a, str>),
    /// A decoded `Location`'s match key.
    Location(String),
    Stars(u8),
    Number(u64),
    Colour(Rgb),
    Date(Date),
    /// A value that doesn't parse as its kind: itself, exactly.
    Unread(&'a str),
}

/// A typed field read the way the reader reads a `TRACK` element holding
/// only this attribute. `None` when the reader reports the value as one it
/// can't parse.
fn read(field: &str, text: &str) -> Option<Track> {
    let mut attrs = Attrs::default();
    attrs.push(field, text);
    let mut warnings = Vec::new();
    let track = collection::track(attrs, 0, &mut warnings);
    let unread = warnings
        .iter()
        .any(|w| matches!(&w.problem, Problem::BadValue { attribute, .. } if attribute == field));
    (!unread).then_some(track)
}

/// What `value` of `field` is compared by. An absent value is an empty one.
pub(super) fn key<'a>(field: &str, value: Option<&'a str>) -> Key<'a> {
    let text = value.unwrap_or("");
    let number = |n: Option<u64>| n.map_or(Key::None, Key::Number);
    let date = |d: Option<Date>| d.map_or(Key::None, Key::Date);
    let kind = kind_of(field);
    match kind {
        // Never compared ([`super::decide`] answers before it would be);
        // exact text keeps `same` honest if it's asked anyway.
        Kind::Analysis | Kind::TrackId => match text {
            "" => Key::None,
            text => Key::Unread(text),
        },
        Kind::Text if text.trim().is_empty() => Key::None,
        Kind::Text if is_nfc(text) => Key::Text(Cow::Borrowed(text)),
        Kind::Text => Key::Text(Cow::Owned(text.nfc().collect())),
        Kind::Location => match location::decode(text) {
            Ok(location) => Key::Location(location.match_key()),
            Err(location::LocationError::Empty) => Key::None,
            Err(_) => Key::Unread(text),
        },
        Kind::Rating | Kind::Colour | Kind::Date | Kind::Number => {
            let Some(track) = read(field, text) else {
                return Key::Unread(text);
            };
            match kind {
                Kind::Rating => match (track.stars(), track.rating) {
                    (Some(stars), _) => Key::Stars(stars),
                    (None, Some(other)) => Key::Number(u64::from(other)),
                    // No rating is rekordbox's zero stars.
                    (None, None) => Key::Stars(0),
                },
                Kind::Colour => track.colour.map_or(Key::None, Key::Colour),
                Kind::Date => match field {
                    "DateAdded" => date(track.date_added),
                    "DateModified" => date(track.date_modified),
                    _ => date(track.last_played),
                },
                _ => match field {
                    "Size" => number(track.size),
                    "TotalTime" => number(track.total_time.map(u64::from)),
                    "DiscNumber" => number(track.disc_number.map(u64::from)),
                    "TrackNumber" => number(track.track_number.map(u64::from)),
                    "Year" => number(track.year.map(u64::from)),
                    "BitRate" => number(track.bit_rate.map(u64::from)),
                    "SampleRate" => number(track.sample_rate.map(u64::from)),
                    _ => number(track.play_count.map(u64::from)),
                },
            }
        }
    }
}

/// Whether two values of `field` are the same value, by the field's kind.
/// `None` is a side with no value, which equals an empty one.
pub fn same(field: &str, a: Option<&str>, b: Option<&str>) -> bool {
    key(field, a) == key(field, b)
}

/// Whether `value` is one the user may give `field` by hand: it parses as
/// the field's kind (an empty value clears the field), a `Rating` is one of
/// rekordbox's six, and the field is one that is merged at all.
pub fn fits(field: &str, value: &str) -> bool {
    match (kind_of(field), key(field, Some(value))) {
        (Kind::Analysis | Kind::TrackId, _) | (_, Key::Unread(_)) => false,
        (Kind::Rating, key) => matches!(key, Key::Stars(_)),
        _ => true,
    }
}
