//! Sets two parsed names side by side.
//!
//! The answer describes the *names*. It decides nothing about merging: the
//! same title can be two different songs, and a remix can be filed under
//! the remixer alone, so grouping (1bB) weighs this with the fingerprint
//! and the credits. Length is no input here at all.
//!
//! Reasons are codes with parameters ([`Reason::code`]), never English
//! text; the UI turns them into its own words later.

use std::collections::{BTreeMap, BTreeSet};

use super::labels::split_names;
use super::tokenize::collapse_spaces;
use super::{CreditRole, Marker, MarkerKind, ParsedName, Reading, VersionClass};

/// What two names say about each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Same base title, same version.
    SameVersion,
    /// Same base title, different cut.
    DifferentCut,
    /// Same base title, different rework.
    DifferentRework,
    /// Different base titles.
    DifferentBase,
    /// The names don't settle it.
    CantTell,
}

/// Which of the two names a reason is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    A,
    B,
}

/// A rework as a name states it: its kind and whose it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReworkRef {
    pub kind: MarkerKind,
    pub detail: Option<String>,
}

/// Why [`compare`] answered as it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The base titles are the same.
    BaseTitlesMatch,
    /// The base titles differ, however the names are read.
    BaseTitlesDiffer,
    /// A name has no title to compare.
    EmptyTitle { side: Side },
    /// The titles only match if label words in this name's title are read
    /// as a marker ("Paper Harbor Extended Mix").
    TitleReadAsMarker { side: Side, text: String },
    /// The titles only match if this name's marker is read as part of its
    /// title ("Velo (dirty)" beside "Velo (dirty) (dirty)").
    MarkerReadAsTitle { side: Side, text: String },
    /// Both names carry the same markers.
    SameMarkers,
    /// The cuts differ: the kinds only one name has.
    CutDiffers {
        only_a: Vec<MarkerKind>,
        only_b: Vec<MarkerKind>,
    },
    /// The reworks differ: the ones only one name has.
    ReworkDiffers {
        only_a: Vec<ReworkRef>,
        only_b: Vec<ReworkRef>,
    },
    /// Both name a rework of this kind, but this name doesn't say whose
    /// ("(Remix)" beside "(Quill Ashby Remix)").
    ReworkDetailMissing { kind: MarkerKind, side: Side },
    /// The names differ in brackets this module doesn't understand.
    UnrecognizedDiffers {
        only_a: Vec<String>,
        only_b: Vec<String>,
    },
    /// The names credit different artists: the same title can be a
    /// different song.
    ArtistsDiffer,
    /// The artists differ, but this name's artist is the remixer the
    /// markers name: a remix filed under the remixer alone.
    CreditedToRemixer { side: Side },
    /// The titles differ, but one of this mashup's parts is the other
    /// name's title. Only a hint: part names match unrelated tracks too.
    MashupPartMatch { side: Side, part: String },
}

impl Reason {
    /// A stable code, for building an i18n key.
    pub fn code(&self) -> &'static str {
        match self {
            Reason::BaseTitlesMatch => "base_titles_match",
            Reason::BaseTitlesDiffer => "base_titles_differ",
            Reason::EmptyTitle { .. } => "empty_title",
            Reason::TitleReadAsMarker { .. } => "title_read_as_marker",
            Reason::MarkerReadAsTitle { .. } => "marker_read_as_title",
            Reason::SameMarkers => "same_markers",
            Reason::CutDiffers { .. } => "cut_differs",
            Reason::ReworkDiffers { .. } => "rework_differs",
            Reason::ReworkDetailMissing { .. } => "rework_detail_missing",
            Reason::UnrecognizedDiffers { .. } => "unrecognized_differs",
            Reason::ArtistsDiffer => "artists_differ",
            Reason::CreditedToRemixer { .. } => "credited_to_remixer",
            Reason::MashupPartMatch { .. } => "mashup_part_match",
        }
    }
}

/// [`compare`]'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub outcome: Outcome,
    pub reasons: Vec<Reason>,
}

/// A title as compared: lowercase letters and digits, single spaces, "&"
/// as "and". A title with no letters or digits compares as written.
pub(super) fn key(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c == '&' {
            out.push_str(" and ");
        } else if c != '\'' {
            out.push(' ');
        }
    }
    let out = collapse_spaces(&out);
    match out.is_empty() {
        true => collapse_spaces(&text.to_lowercase()),
        false => out,
    }
}

/// A person's name as compared: "Quill Ashby's" is "quill ashby".
fn name_key(name: &str) -> String {
    let name = name.trim();
    let name = name
        .strip_suffix("'s")
        .or_else(|| name.strip_suffix("'S"))
        .unwrap_or(name);
    key(name)
}

/// The cuts a reading names. "Original" is the same as no label, and Intro
/// with Outro is Intro-Outro.
fn cuts(markers: &[Marker]) -> BTreeSet<MarkerKind> {
    let mut cuts: BTreeSet<MarkerKind> = markers
        .iter()
        .map(|m| m.kind)
        .filter(|k| k.class() == VersionClass::Cut && *k != MarkerKind::Original)
        .collect();
    if cuts.contains(&MarkerKind::Intro) && cuts.contains(&MarkerKind::Outro) {
        cuts.remove(&MarkerKind::Intro);
        cuts.remove(&MarkerKind::Outro);
        cuts.insert(MarkerKind::IntroOutro);
    }
    cuts
}

/// The reworks a reading names, by kind and whose they are.
type Reworks = BTreeMap<(MarkerKind, Option<String>), ReworkRef>;

fn reworks(markers: &[Marker]) -> Reworks {
    markers
        .iter()
        .filter(|m| m.kind.class() == VersionClass::Rework)
        .map(|m| {
            let whose = m.detail.as_deref().map(name_key);
            let rework = ReworkRef {
                kind: m.kind,
                detail: m.detail.clone(),
            };
            ((m.kind, whose), rework)
        })
        .collect()
}

/// The names a rework's detail holds: "Nox & Vey" is the pair and each.
fn remixer_keys(markers: &[Marker]) -> BTreeSet<String> {
    markers
        .iter()
        .filter(|m| m.kind.class() == VersionClass::Rework)
        .filter_map(|m| m.detail.as_deref())
        .flat_map(|detail| {
            let mut names = split_names(detail, true);
            names.push(detail.to_string());
            names
        })
        .map(|name| name_key(&name))
        .collect()
}

fn main_artists(name: &ParsedName) -> BTreeSet<String> {
    name.credits
        .iter()
        .filter(|c| c.role == CreditRole::Main)
        .map(|c| name_key(&c.name))
        .collect()
}

/// The closest pair of readings whose titles are the same. Of two equally
/// close pairs, the one that reads fewer title words as markers wins: text
/// both names carry is more likely title than marker.
fn matching_readings<'a>(a: &'a [Reading], b: &'a [Reading]) -> Option<(&'a Reading, &'a Reading)> {
    let cost = |x: &Reading, y: &Reading| {
        (
            x.distance() + y.distance(),
            x.promoted.len() + y.promoted.len(),
        )
    };
    let mut best: Option<(&Reading, &Reading)> = None;
    for x in a {
        for y in b {
            if key(&x.base_title) != key(&y.base_title) {
                continue;
            }
            match best {
                Some((p, q)) if cost(p, q) <= cost(x, y) => {}
                _ => best = Some((x, y)),
            }
        }
    }
    best
}

/// Compares two parsed names. See [`Outcome`] and [`Reason`].
pub fn compare(a: &ParsedName, b: &ParsedName) -> Comparison {
    let mut reasons = Vec::new();
    for (side, name) in [(Side::A, a), (Side::B, b)] {
        if key(&name.base_title).is_empty() {
            reasons.push(Reason::EmptyTitle { side });
        }
    }
    if !reasons.is_empty() {
        return Comparison {
            outcome: Outcome::CantTell,
            reasons,
        };
    }

    let (readings_a, readings_b) = (a.readings(), b.readings());
    let Some((x, y)) = matching_readings(&readings_a, &readings_b) else {
        reasons.push(Reason::BaseTitlesDiffer);
        // A mashup's part that is the other name's title (or one of its
        // parts) is a hint, never a link by itself.
        let sides = [(Side::A, a, b, &readings_b), (Side::B, b, a, &readings_a)];
        for (side, mashup, other, other_readings) in sides {
            for part in &mashup.mashup_parts {
                let part_key = key(part);
                let matches = other_readings
                    .iter()
                    .any(|r| key(&r.base_title) == part_key)
                    || other.mashup_parts.iter().any(|p| key(p) == part_key);
                if matches {
                    reasons.push(Reason::MashupPartMatch {
                        side,
                        part: part.clone(),
                    });
                }
            }
        }
        let outcome = match reasons.len() {
            1 => Outcome::DifferentBase,
            _ => Outcome::CantTell,
        };
        return Comparison { outcome, reasons };
    };

    reasons.push(Reason::BaseTitlesMatch);
    let mut unsure = false;
    for (side, reading) in [(Side::A, x), (Side::B, y)] {
        for text in &reading.promoted {
            reasons.push(Reason::TitleReadAsMarker {
                side,
                text: text.clone(),
            });
        }
        for text in &reading.demoted {
            reasons.push(Reason::MarkerReadAsTitle {
                side,
                text: text.clone(),
            });
        }
    }
    let promoted = !x.promoted.is_empty() || !y.promoted.is_empty();

    // Reworks: by kind, then whose.
    let (reworks_a, reworks_b) = (reworks(&x.markers), reworks(&y.markers));
    let kinds =
        |reworks: &Reworks| -> BTreeSet<MarkerKind> { reworks.keys().map(|k| k.0).collect() };
    let mut rework_differs = kinds(&reworks_a) != kinds(&reworks_b);
    if !rework_differs {
        for kind in kinds(&reworks_a) {
            let whose = |reworks: &Reworks| -> BTreeSet<Option<String>> {
                reworks
                    .keys()
                    .filter(|k| k.0 == kind)
                    .map(|k| k.1.clone())
                    .collect()
            };
            let (whose_a, whose_b) = (whose(&reworks_a), whose(&reworks_b));
            if whose_a == whose_b {
                continue;
            }
            let unnamed = [(Side::A, &whose_a), (Side::B, &whose_b)]
                .into_iter()
                .filter(|(_, whose)| whose.contains(&None))
                .map(|(side, _)| side)
                .collect::<Vec<_>>();
            if unnamed.is_empty() {
                rework_differs = true;
            } else {
                unsure = true;
                for side in unnamed {
                    reasons.push(Reason::ReworkDetailMissing { kind, side });
                }
            }
        }
    }
    if rework_differs {
        let only = |mine: &Reworks, theirs: &Reworks| -> Vec<ReworkRef> {
            mine.iter()
                .filter(|(k, _)| !theirs.contains_key(k))
                .map(|(_, rework)| rework.clone())
                .collect()
        };
        reasons.push(Reason::ReworkDiffers {
            only_a: only(&reworks_a, &reworks_b),
            only_b: only(&reworks_b, &reworks_a),
        });
    }

    let (cuts_a, cuts_b) = (cuts(&x.markers), cuts(&y.markers));
    let cut_differs = cuts_a != cuts_b;
    if cut_differs {
        reasons.push(Reason::CutDiffers {
            only_a: cuts_a.difference(&cuts_b).copied().collect(),
            only_b: cuts_b.difference(&cuts_a).copied().collect(),
        });
    }

    // Brackets nobody understands: if they differ, so may the tracks.
    let unknown = |name: &ParsedName| -> BTreeMap<String, String> {
        name.unrecognized
            .iter()
            .map(|text| (key(text), text.clone()))
            .collect()
    };
    let (unknown_a, unknown_b) = (unknown(a), unknown(b));
    if !unknown_a.keys().eq(unknown_b.keys()) {
        unsure = true;
        let only = |mine: &BTreeMap<String, String>, theirs: &BTreeMap<String, String>| {
            mine.iter()
                .filter(|(k, _)| !theirs.contains_key(*k))
                .map(|(_, text)| text.clone())
                .collect::<Vec<_>>()
        };
        reasons.push(Reason::UnrecognizedDiffers {
            only_a: only(&unknown_a, &unknown_b),
            only_b: only(&unknown_b, &unknown_a),
        });
    }

    // Different artists: a different song, unless one of them is the
    // remixer.
    let (artists_a, artists_b) = (main_artists(a), main_artists(b));
    if !artists_a.is_empty() && !artists_b.is_empty() && artists_a.is_disjoint(&artists_b) {
        let mut remixers = remixer_keys(&x.markers);
        remixers.extend(remixer_keys(&y.markers));
        let mut explained = false;
        for (side, artists) in [(Side::A, &artists_a), (Side::B, &artists_b)] {
            if artists.iter().any(|artist| remixers.contains(artist)) {
                reasons.push(Reason::CreditedToRemixer { side });
                explained = true;
            }
        }
        if !explained {
            reasons.push(Reason::ArtistsDiffer);
            unsure = true;
        }
    }

    // Bare words read as a marker only count when the other name carries
    // the same marker; a difference resting on them isn't settled.
    if promoted && (rework_differs || cut_differs) {
        unsure = true;
    }
    let outcome = if unsure {
        Outcome::CantTell
    } else if rework_differs {
        Outcome::DifferentRework
    } else if cut_differs {
        Outcome::DifferentCut
    } else {
        reasons.push(Reason::SameMarkers);
        Outcome::SameVersion
    };
    Comparison { outcome, reasons }
}
