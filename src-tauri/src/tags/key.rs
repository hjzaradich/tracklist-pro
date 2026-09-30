//! Musical key parsing and normalization.
//!
//! DJ libraries carry key in at least three mutually incompatible notations,
//! often within the same folder:
//!
//! - Camelot:   `8A`, `08A`, `12B`
//! - Open Key:  `1m`, `1d`
//! - Classical: `A minor`, `Am`, `A min`, `Abm`, `F#m`, `Bb`
//!
//! Everything is parsed to a canonical (pitch class, mode) pair and stored as
//! Camelot. The original string is preserved separately so a user's own
//! notation is never destroyed by normalization.
//!
//! Ported from musicmanager's `tags/key.rs` (ROADMAP §0.2), with fixes:
//! `CM` parses as C major (musicmanager read it as C minor), and trailing
//! junk (`A1`, `E7`, `Bm 123`) is rejected instead of ignored. Its doc also
//! claimed raw pitch-class integers were parsed; they never were, and a
//! bare number is too ambiguous to guess at. Added
//! here: the way back from Camelot to the other notations, for the global
//! key-notation setting (ROADMAP §1.1 "Key notation", [`KeyNotation`]).

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Major,
    Minor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MusicalKey {
    /// Pitch class, C = 0 through B = 11.
    pub pitch_class: u8,
    pub mode: Mode,
}

/// Camelot number for each pitch class in major.  C = 8B, G = 9B, ...
const CAMELOT_MAJOR: [u8; 12] = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1];
/// Camelot number for each pitch class in minor.  Am = 8A, Em = 9A, ...
const CAMELOT_MINOR: [u8; 12] = [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10];

impl MusicalKey {
    pub fn to_camelot(self) -> String {
        let (table, suffix) = match self.mode {
            Mode::Major => (&CAMELOT_MAJOR, 'B'),
            Mode::Minor => (&CAMELOT_MINOR, 'A'),
        };
        format!("{}{}", table[self.pitch_class as usize], suffix)
    }

    /// Musical notation in the standard spelling: `Am`, `F#m`, `Eb`, `C`.
    pub fn to_musical(self) -> String {
        self.spell(&STANDARD)
    }

    fn spell(self, spelling: &Spelling) -> String {
        let names = match self.mode {
            Mode::Major => &spelling.major,
            Mode::Minor => &spelling.minor,
        };
        names[self.pitch_class as usize].to_string()
    }

    /// This key in the chosen notation.
    pub fn format(self, notation: KeyNotation) -> String {
        match notation {
            KeyNotation::Camelot => self.to_camelot(),
            KeyNotation::MusicalStandard => self.spell(&STANDARD),
            KeyNotation::MusicalRekordbox => self.spell(&REKORDBOX),
            KeyNotation::MusicalSharps => self.spell(&SHARPS),
            KeyNotation::MusicalFlats => self.spell(&FLATS),
        }
    }

    /// Every key, in Camelot order: 1A to 12A, then 1B to 12B.
    pub fn all() -> impl Iterator<Item = MusicalKey> {
        [(Mode::Minor, &CAMELOT_MINOR), (Mode::Major, &CAMELOT_MAJOR)]
            .into_iter()
            .flat_map(|(mode, table)| {
                (1..=12u8).map(move |n| MusicalKey {
                    pitch_class: table.iter().position(|&c| c == n).unwrap() as u8,
                    mode,
                })
            })
    }
}

/// How keys are shown: one app-wide setting (ROADMAP §1.1), stored in the
/// `setting` table (see `crate::settings`). Camelot is the default.
///
/// To add a notation (e.g. Open Key, or Serato's or Mixed In Key's
/// spellings): add a variant here and to [`KeyNotation::ALL`], and a match
/// arm in [`MusicalKey::format`]. The stored name, the TypeScript type and
/// the frontend's name table follow from those.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum KeyNotation {
    /// `8A`, `12B`.
    #[default]
    Camelot,
    /// Conventional key-signature spelling: the enharmonic name with the
    /// fewest sharps or flats (`C#m`, not `Dbm`, which would need 8 flats).
    MusicalStandard,
    /// rekordbox's own spelling, from the rekordbox behavior check.
    MusicalRekordbox,
    /// Every black key as a sharp: `C#`, `D#m`, `A#`.
    MusicalSharps,
    /// Every black key as a flat: `Db`, `Ebm`, `Bb`.
    MusicalFlats,
}

impl KeyNotation {
    /// Every notation, in the order a settings screen would list them.
    pub const ALL: [KeyNotation; 5] = [
        KeyNotation::Camelot,
        KeyNotation::MusicalStandard,
        KeyNotation::MusicalRekordbox,
        KeyNotation::MusicalSharps,
        KeyNotation::MusicalFlats,
    ];

    /// The name it's stored and sent under: `camelot`, `musical_standard`.
    pub fn as_str(self) -> &'static str {
        match self {
            KeyNotation::Camelot => "camelot",
            KeyNotation::MusicalStandard => "musical_standard",
            KeyNotation::MusicalRekordbox => "musical_rekordbox",
            KeyNotation::MusicalSharps => "musical_sharps",
            KeyNotation::MusicalFlats => "musical_flats",
        }
    }

    /// The notation stored under `name`, or `None` for a name this version
    /// doesn't know.
    pub fn from_name(name: &str) -> Option<KeyNotation> {
        KeyNotation::ALL.into_iter().find(|n| n.as_str() == name)
    }
}

/// The 24 names of one notation, in Camelot order (1A to 12A, then 1B to
/// 12B). The frontend gets every notation's table as the `KEY_NAMES`
/// constant in the generated bindings, so it formats keys without a call per
/// key, from the same tables as the backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct KeyNames {
    pub notation: KeyNotation,
    pub names: Vec<String>,
}

/// Every notation's name table, for the frontend.
pub fn key_names() -> Vec<KeyNames> {
    KeyNotation::ALL
        .into_iter()
        .map(|notation| KeyNames {
            notation,
            names: MusicalKey::all().map(|k| k.format(notation)).collect(),
        })
        .collect()
}

/// The musical name of every key in one spelling, indexed by pitch class
/// (C = 0). ASCII `#` and `b`, so every name parses back to its key.
struct Spelling {
    major: [&'static str; 12],
    minor: [&'static str; 12],
}

/// Conventional key-signature spelling: the enharmonic name with the fewer
/// sharps or flats (`Db` with 5 flats, not `C#` with 7 sharps). Two keys
/// tie at 6 either way; they take the more common name: `F#` (over
/// `Gb`) and `Ebm` (over `D#m`).
const STANDARD: Spelling = Spelling {
    major: [
        "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
    ],
    minor: [
        "Cm", "C#m", "Dm", "Ebm", "Em", "Fm", "F#m", "Gm", "G#m", "Am", "Bbm", "Bm",
    ],
};

/// How rekordbox 7 spells keys in its Classic key display: the names the
/// Camelot wheel gives them (1A is `Abm`, 12A is `Dbm`). Captured 2026-09-28
/// from rekordbox 7.2.19 in a real collection. 22 keys were observed there;
/// F# (2B) and Bb (6B) were confirmed on 2026-09-30.
const REKORDBOX: Spelling = Spelling {
    major: [
        "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
    ],
    minor: [
        "Cm", "Dbm", "Dm", "Ebm", "Em", "Fm", "F#m", "Gm", "Abm", "Am", "Bbm", "Bm",
    ],
};

const SHARPS: Spelling = Spelling {
    major: [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ],
    minor: [
        "Cm", "C#m", "Dm", "D#m", "Em", "Fm", "F#m", "Gm", "G#m", "Am", "A#m", "Bm",
    ],
};

const FLATS: Spelling = Spelling {
    major: [
        "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
    ],
    minor: [
        "Cm", "Dbm", "Dm", "Ebm", "Em", "Fm", "Gbm", "Gm", "Abm", "Am", "Bbm", "Bm",
    ],
};

/// Semitone offset of each natural note above C.
fn natural_pitch_class(c: char) -> Option<u8> {
    match c.to_ascii_uppercase() {
        'C' => Some(0),
        'D' => Some(2),
        'E' => Some(4),
        'F' => Some(5),
        'G' => Some(7),
        'A' => Some(9),
        'B' => Some(11),
        _ => None,
    }
}

/// Parses any supported notation. Returns `None` for empty or unrecognized
/// input, which is treated as "no key" rather than an error — plenty of files
/// have junk in the key field, and that should not fail a scan.
pub fn parse(raw: &str) -> Option<MusicalKey> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }

    parse_camelot(s)
        .or_else(|| parse_open_key(s))
        .or_else(|| parse_classical(s))
}

/// `8A`, `08A`, `12B`, and the lowercase variants.
fn parse_camelot(s: &str) -> Option<MusicalKey> {
    let bytes = s.as_bytes();
    let last = *bytes.last()? as char;
    let mode = match last.to_ascii_uppercase() {
        'A' => Mode::Minor,
        'B' => Mode::Major,
        _ => return None,
    };

    let number: u8 = s[..s.len() - 1].trim().parse().ok()?;
    if !(1..=12).contains(&number) {
        return None;
    }

    // A bare "A" or "B" is classical, not Camelot. Require the digits.
    let table = match mode {
        Mode::Major => &CAMELOT_MAJOR,
        Mode::Minor => &CAMELOT_MINOR,
    };
    let pitch_class = table.iter().position(|&n| n == number)? as u8;

    Some(MusicalKey { pitch_class, mode })
}

/// Open Key notation: `1m` through `12m` (minor), `1d` through `12d` (major).
///
/// Open Key n corresponds to Camelot ((n + 6) mod 12) + 1.
fn parse_open_key(s: &str) -> Option<MusicalKey> {
    let last = *s.as_bytes().last()? as char;
    let mode = match last.to_ascii_lowercase() {
        'm' => Mode::Minor,
        'd' => Mode::Major,
        _ => return None,
    };

    let number: u8 = s[..s.len() - 1].trim().parse().ok()?;
    if !(1..=12).contains(&number) {
        return None;
    }

    let camelot = ((number as u16 + 6) % 12) as u8 + 1;
    let table = match mode {
        Mode::Major => &CAMELOT_MAJOR,
        Mode::Minor => &CAMELOT_MINOR,
    };
    let pitch_class = table.iter().position(|&n| n == camelot)? as u8;

    Some(MusicalKey { pitch_class, mode })
}

/// `A minor`, `Am`, `A min`, `Abm`, `F#m`, `Bb`, `C`.
fn parse_classical(s: &str) -> Option<MusicalKey> {
    let mut chars = s.chars();
    let root = chars.next()?;
    let mut pitch_class = natural_pitch_class(root)?;

    let rest = chars.as_str();
    let rest = if let Some(stripped) = rest.strip_prefix('#') {
        pitch_class = (pitch_class + 1) % 12;
        stripped
    } else if let Some(stripped) = rest.strip_prefix('♯') {
        pitch_class = (pitch_class + 1) % 12;
        stripped
    } else if let Some(stripped) = rest.strip_prefix('b') {
        pitch_class = (pitch_class + 11) % 12;
        stripped
    } else if let Some(stripped) = rest.strip_prefix('♭') {
        pitch_class = (pitch_class + 11) % 12;
        stripped
    } else {
        rest
    };

    // Separators vary: "A minor", "A-minor", "Amin". Anything else is junk,
    // not a key: "A1", "E7", "Bm 123", "A-".
    let rest = rest.trim_end();
    let qualifier = rest.trim_start_matches([' ', '-', '_']);
    if (qualifier.is_empty() && !rest.is_empty())
        || !qualifier.chars().all(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    // A lone lowercase letter is a stray Camelot or Open Key suffix (`b`,
    // `d`), not a key.
    if s.len() == 1 && root.is_ascii_lowercase() {
        return None;
    }

    // A lone letter is case-sensitive: `M` is major, `m` is minor. Words are
    // not: `Maj`, `MIN`, `Minor`. (musicmanager lowercased first, so `CM`
    // came out as C minor.)
    let mode = match qualifier {
        "M" => Mode::Major,
        "m" => Mode::Minor,
        word => match word.to_ascii_lowercase().as_str() {
            // Bare root is conventionally major.
            "" | "maj" | "major" => Mode::Major,
            "min" | "minor" => Mode::Minor,
            _ => return None,
        },
    };

    Some(MusicalKey { pitch_class, mode })
}

/// Convenience wrapper: raw tag value to Camelot string.
pub fn to_camelot(raw: &str) -> Option<String> {
    parse(raw).map(|k| k.to_camelot())
}

/// Any notation to musical notation, standard spelling (`8A` -> `Am`).
pub fn to_musical(raw: &str) -> Option<String> {
    parse(raw).map(|k| k.to_musical())
}

/// Any notation to the chosen one. `None` for junk, as with [`parse`].
pub fn format(raw: &str, notation: KeyNotation) -> Option<String> {
    parse(raw).map(|k| k.format(notation))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camelot(raw: &str) -> Option<String> {
        to_camelot(raw)
    }

    #[test]
    fn parses_camelot_notation() {
        assert_eq!(camelot("8A").as_deref(), Some("8A"));
        assert_eq!(camelot("08A").as_deref(), Some("8A"));
        assert_eq!(camelot("12B").as_deref(), Some("12B"));
        assert_eq!(camelot("1a").as_deref(), Some("1A"));
    }

    #[test]
    fn parses_classical_notation() {
        // The anchor points of the Camelot wheel.
        assert_eq!(camelot("A minor").as_deref(), Some("8A"));
        assert_eq!(camelot("Am").as_deref(), Some("8A"));
        assert_eq!(camelot("A min").as_deref(), Some("8A"));
        assert_eq!(camelot("C").as_deref(), Some("8B"));
        assert_eq!(camelot("C major").as_deref(), Some("8B"));
        assert_eq!(camelot("Em").as_deref(), Some("9A"));
        assert_eq!(camelot("G").as_deref(), Some("9B"));
    }

    #[test]
    fn handles_accidentals_including_enharmonic_equivalents() {
        // Ab minor and G# minor are the same key: Camelot 1A.
        assert_eq!(camelot("Abm").as_deref(), Some("1A"));
        assert_eq!(camelot("G#m").as_deref(), Some("1A"));
        assert_eq!(camelot("F#m").as_deref(), Some("11A"));
        assert_eq!(camelot("Gbm").as_deref(), Some("11A"));
        assert_eq!(camelot("Bb").as_deref(), Some("6B"));
        assert_eq!(camelot("A#").as_deref(), Some("6B"));
        // Unicode accidentals appear in tags written by some taggers.
        assert_eq!(camelot("F♯m").as_deref(), Some("11A"));
    }

    #[test]
    fn parses_open_key_notation() {
        // Open Key 1m is A minor, which is Camelot 8A.
        assert_eq!(camelot("1m").as_deref(), Some("8A"));
        assert_eq!(camelot("1d").as_deref(), Some("8B"));
        assert_eq!(camelot("6m").as_deref(), Some("1A"));
        assert_eq!(camelot("12d").as_deref(), Some("7B"));
    }

    #[test]
    fn round_trips_every_key_through_camelot() {
        for pitch_class in 0..12u8 {
            for mode in [Mode::Major, Mode::Minor] {
                let key = MusicalKey { pitch_class, mode };
                let text = key.to_camelot();
                assert_eq!(parse(&text), Some(key), "{text} did not round-trip");
            }
        }
    }

    #[test]
    fn every_camelot_slot_is_used_exactly_once() {
        // Guards the lookup tables against a typo silently colliding two keys.
        let mut seen = std::collections::HashSet::new();
        for pitch_class in 0..12u8 {
            for mode in [Mode::Major, Mode::Minor] {
                assert!(
                    seen.insert(MusicalKey { pitch_class, mode }.to_camelot()),
                    "duplicate Camelot value in lookup table"
                );
            }
        }
        assert_eq!(seen.len(), 24);
    }

    #[test]
    fn rejects_junk_without_erroring() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("   "), None);
        assert_eq!(parse("unknown"), None);
        assert_eq!(parse("13A"), None);
        assert_eq!(parse("0A"), None);
        assert_eq!(parse("H minor"), None);
        assert_eq!(parse("-"), None);
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        assert_eq!(camelot("  8A  ").as_deref(), Some("8A"));
        assert_eq!(camelot(" A minor ").as_deref(), Some("8A"));
    }

    // Not in musicmanager: musicmanager lowercased the qualifier before
    // matching, so its unreachable `M` arm let `CM` parse as C minor.
    #[test]
    fn a_capital_m_means_major_and_a_small_m_means_minor() {
        assert_eq!(camelot("CM").as_deref(), Some("8B"));
        assert_eq!(camelot("Cm").as_deref(), Some("5A"));
        assert_eq!(camelot("F#M").as_deref(), Some("2B"));
        assert_eq!(camelot("F#m").as_deref(), Some("11A"));
        assert_eq!(camelot("BbM").as_deref(), Some("6B"));
        assert_eq!(camelot("Bbm").as_deref(), Some("3A"));
    }

    #[test]
    fn keys_with_trailing_junk_are_rejected_not_guessed() {
        for junk in [
            "A1", "E7", "C 123", "D-2", "A-", "Bm 123", "F#m7", "Am.", "b", "d", "a",
        ] {
            assert_eq!(parse(junk), None, "{junk}");
        }
        // Separators before a word are still fine.
        assert_eq!(camelot("A-minor").as_deref(), Some("8A"));
        assert_eq!(camelot("A_min").as_deref(), Some("8A"));
        assert_eq!(camelot("B").as_deref(), Some("1B"));
        assert_eq!(camelot("bm").as_deref(), Some("10A"));
    }

    #[test]
    fn major_and_minor_words_match_in_any_case() {
        for major in [
            "C", "Cmaj", "CMaj", "CMAJ", "C major", "C Major", "C MAJOR", "C-maj",
        ] {
            assert_eq!(camelot(major).as_deref(), Some("8B"), "{major}");
        }
        for minor in [
            "Cm", "Cmin", "CMin", "CMIN", "C minor", "C Minor", "C MINOR", "C-min",
        ] {
            assert_eq!(camelot(minor).as_deref(), Some("5A"), "{minor}");
        }
    }

    // Not in musicmanager: the way back from Camelot to every notation, for
    // the global key-notation setting (ROADMAP §1.1).

    /// Every key in every notation: Camelot, then standard, sharps, flats.
    /// rekordbox's spellings are checked separately, once they're known.
    const TABLE: [(&str, &str, &str, &str); 24] = [
        ("1A", "G#m", "G#m", "Abm"),
        ("2A", "Ebm", "D#m", "Ebm"),
        ("3A", "Bbm", "A#m", "Bbm"),
        ("4A", "Fm", "Fm", "Fm"),
        ("5A", "Cm", "Cm", "Cm"),
        ("6A", "Gm", "Gm", "Gm"),
        ("7A", "Dm", "Dm", "Dm"),
        ("8A", "Am", "Am", "Am"),
        ("9A", "Em", "Em", "Em"),
        ("10A", "Bm", "Bm", "Bm"),
        ("11A", "F#m", "F#m", "Gbm"),
        ("12A", "C#m", "C#m", "Dbm"),
        ("1B", "B", "B", "B"),
        ("2B", "F#", "F#", "Gb"),
        ("3B", "Db", "C#", "Db"),
        ("4B", "Ab", "G#", "Ab"),
        ("5B", "Eb", "D#", "Eb"),
        ("6B", "Bb", "A#", "Bb"),
        ("7B", "F", "F", "F"),
        ("8B", "C", "C", "C"),
        ("9B", "G", "G", "G"),
        ("10B", "D", "D", "D"),
        ("11B", "A", "A", "A"),
        ("12B", "E", "E", "E"),
    ];

    #[test]
    fn every_camelot_key_formats_to_its_name_in_each_musical_notation() {
        for (camelot, standard, sharps, flats) in TABLE {
            let f = |n| format(camelot, n);
            assert_eq!(f(KeyNotation::Camelot).as_deref(), Some(camelot));
            assert_eq!(
                f(KeyNotation::MusicalStandard).as_deref(),
                Some(standard),
                "{camelot}"
            );
            assert_eq!(
                f(KeyNotation::MusicalSharps).as_deref(),
                Some(sharps),
                "{camelot}"
            );
            assert_eq!(
                f(KeyNotation::MusicalFlats).as_deref(),
                Some(flats),
                "{camelot}"
            );
        }
    }

    #[test]
    fn every_name_in_every_notation_parses_back_to_the_same_key() {
        for notation in KeyNotation::ALL {
            for key in MusicalKey::all() {
                let text = key.format(notation);
                assert_eq!(
                    parse(&text),
                    Some(key),
                    "{notation:?}: {text} did not round-trip"
                );
            }
        }
    }

    #[test]
    fn each_notation_gives_24_different_names() {
        for notation in KeyNotation::ALL {
            let names: std::collections::HashSet<_> =
                MusicalKey::all().map(|k| k.format(notation)).collect();
            assert_eq!(names.len(), 24, "{notation:?}");
        }
    }

    #[test]
    fn sharps_never_uses_a_flat_and_flats_never_uses_a_sharp() {
        for key in MusicalKey::all() {
            assert!(
                !key.format(KeyNotation::MusicalSharps)[1..].contains('b'),
                "{key:?}"
            );
            assert!(
                !key.format(KeyNotation::MusicalFlats).contains('#'),
                "{key:?}"
            );
        }
    }

    #[test]
    fn standard_spelling_never_needs_more_than_six_sharps_or_flats() {
        // Sharps and flats in each major key signature, and each minor one
        // (a minor key shares its relative major's signature).
        fn accidentals(name: &str) -> i32 {
            let major = [
                ("C", 0),
                ("G", 1),
                ("D", 2),
                ("A", 3),
                ("E", 4),
                ("B", 5),
                ("F#", 6),
                ("C#", 7),
                ("F", -1),
                ("Bb", -2),
                ("Eb", -3),
                ("Ab", -4),
                ("Db", -5),
                ("Gb", -6),
                ("Cb", -7),
            ];
            let minor = [
                ("Am", 0),
                ("Em", 1),
                ("Bm", 2),
                ("F#m", 3),
                ("C#m", 4),
                ("G#m", 5),
                ("D#m", 6),
                ("Dm", -1),
                ("Gm", -2),
                ("Cm", -3),
                ("Fm", -4),
                ("Bbm", -5),
                ("Ebm", -6),
            ];
            major
                .iter()
                .chain(minor.iter())
                .find(|(n, _)| *n == name)
                .map(|(_, a)| i32::abs(*a))
                .unwrap_or_else(|| panic!("{name} is not a key signature"))
        }
        for key in MusicalKey::all() {
            let name = key.format(KeyNotation::MusicalStandard);
            assert!(accidentals(&name) <= 6, "{name}");
        }
    }

    #[test]
    fn musical_rekordbox_gives_a_name_for_every_key() {
        for key in MusicalKey::all() {
            assert!(!key.format(KeyNotation::MusicalRekordbox).is_empty());
        }
    }

    #[test]
    fn musical_rekordbox_matches_the_22_spellings_observed_in_rekordbox() {
        // From rekordbox 7.2.19, 2026-09-28. 2B and 6B weren't
        // observed (task 1aB-13 checks them).
        const OBSERVED: [(&str, &str); 22] = [
            ("1A", "Abm"),
            ("2A", "Ebm"),
            ("3A", "Bbm"),
            ("4A", "Fm"),
            ("5A", "Cm"),
            ("6A", "Gm"),
            ("7A", "Dm"),
            ("8A", "Am"),
            ("9A", "Em"),
            ("10A", "Bm"),
            ("11A", "F#m"),
            ("12A", "Dbm"),
            ("1B", "B"),
            ("3B", "Db"),
            ("4B", "Ab"),
            ("5B", "Eb"),
            ("7B", "F"),
            ("8B", "C"),
            ("9B", "G"),
            ("10B", "D"),
            ("11B", "A"),
            ("12B", "E"),
        ];
        for (camelot, name) in OBSERVED {
            let got = format(camelot, KeyNotation::MusicalRekordbox);
            assert_eq!(got.as_deref(), Some(name), "{camelot}");
        }
    }

    #[test]
    fn musical_rekordbox_fills_the_two_unobserved_keys_from_the_same_wheel() {
        assert_eq!(
            format("2B", KeyNotation::MusicalRekordbox).as_deref(),
            Some("F#")
        );
        assert_eq!(
            format("6B", KeyNotation::MusicalRekordbox).as_deref(),
            Some("Bb")
        );
    }

    #[test]
    fn all_keys_come_in_camelot_order() {
        let order: Vec<_> = MusicalKey::all().map(|k| k.to_camelot()).collect();
        let expected: Vec<_> = TABLE.iter().map(|row| row.0.to_string()).collect();
        assert_eq!(order, expected);
    }

    #[test]
    fn any_notation_converts_to_standard_musical() {
        assert_eq!(to_musical("8A").as_deref(), Some("Am"));
        assert_eq!(to_musical("08A").as_deref(), Some("Am"));
        assert_eq!(to_musical("1m").as_deref(), Some("Am"));
        assert_eq!(to_musical("A minor").as_deref(), Some("Am"));
        // Enharmonic spellings come out in the standard spelling.
        assert_eq!(to_musical("Abm").as_deref(), Some("G#m"));
        assert_eq!(to_musical("A#").as_deref(), Some("Bb"));
        assert_eq!(to_musical("junk"), None);
    }

    #[test]
    fn the_key_notation_defaults_to_camelot() {
        assert_eq!(KeyNotation::default(), KeyNotation::Camelot);
        assert_eq!(format("Am", KeyNotation::default()).as_deref(), Some("8A"));
        assert_eq!(
            format("12d", KeyNotation::MusicalStandard).as_deref(),
            Some("F")
        );
        assert_eq!(format("", KeyNotation::MusicalStandard), None);
    }

    #[test]
    fn each_notation_has_one_stored_name_that_reads_back_and_matches_the_frontend_name() {
        let mut seen = std::collections::HashSet::new();
        for notation in KeyNotation::ALL {
            let name = notation.as_str();
            assert!(seen.insert(name), "{name} used twice");
            assert_eq!(KeyNotation::from_name(name), Some(notation));
            // The name the frontend sees (serde) is the stored name.
            assert_eq!(
                serde_json::to_string(&notation).unwrap(),
                format!("\"{name}\"")
            );
        }
        assert_eq!(KeyNotation::from_name("musical"), None);
        assert_eq!(KeyNotation::from_name("open_key"), None);
    }

    #[test]
    fn the_frontend_name_table_lists_every_notation_with_24_names_in_camelot_order() {
        let tables = key_names();
        assert_eq!(
            tables.iter().map(|t| t.notation).collect::<Vec<_>>(),
            KeyNotation::ALL
        );
        for table in &tables {
            let expected: Vec<_> = MusicalKey::all()
                .map(|k| k.format(table.notation))
                .collect();
            assert_eq!(table.names, expected, "{:?}", table.notation);
        }
        let standard = &tables[1];
        assert_eq!(standard.notation, KeyNotation::MusicalStandard);
        assert_eq!(standard.names[0], "G#m"); // 1A
        assert_eq!(standard.names[12], "B"); // 1B
    }
}
