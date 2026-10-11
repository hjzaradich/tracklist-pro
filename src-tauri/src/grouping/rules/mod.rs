//! The grouping rules: what two files are to each other (1bB-1 to 1bB-5;
//! ROADMAP 1.4, 1.5).
//!
//! A pure decision library: no database, no files, no jobs, no IPC. Given
//! what is known about two files, [`decide`] answers with one [`Outcome`]
//! (duplicate, cut link, rework link, different, or needs review) and the
//! [`Reason`]s for it. A reason is a code plus parameters, never English
//! text; its wording arrives with the Review screen (1bD).
//!
//! Nothing here is wired in yet: the grouping job (1bC-1) calls it and
//! stores what it says. The rules don't decide *when* a pair is asked
//! about, or whether to wait for a fingerprint that hasn't been made yet:
//! that is the job's business. Asked about a file with no fingerprint,
//! they answer for what is known ([`Audio::Unknown`]).
//!
//! # The two inputs
//!
//! **The audio** ([`Audio`]), sorted into one of seven cases:
//!
//! | Case | Means |
//! |---|---|
//! | identical | the same `audio_hash`, or the same `blake3` |
//! | full | coverage ≥ 90% on both files and a difference score ≤ 4 (ROADMAP 1.4, unchanged; the main alignment only) |
//! | one inside the other | ≥ 90% of exactly one file is found in the other, over every alignment, with a difference score ≤ 4 over everything found |
//! | partial | anything else `matching` reports as matched ([`MIN_SHARED_ITEMS`]) |
//! | none | the fingerprints were compared and nothing matched |
//! | not compared | both files have fingerprints, and nobody compared them |
//! | unknown | a file has no fingerprint |
//!
//! **"None" is a finding; "not compared" isn't.** The matching pass only
//! compares the pairs its block index proposes, and it rarely proposes a
//! weak match: two files can share a good part of their audio and have
//! no stored comparison. So a missing comparison never counts as "no audio
//! shared"
//! ([`Audio::NotCompared`]). For a pair the names link (the same names, a
//! mashup and a part it names), the caller runs a direct comparison
//! ([`crate::matching::compare()`]) before asking, so that such a pair is
//! judged on its audio. Asked without one, the same names go to review,
//! never to "different".
//!
//! **The names** ([`FileFacts`]): each file's title tag and file name, both
//! parsed, and its artist tag. [`crate::versions::compare`] says how two
//! names relate (the same, another cut, another rework, another title, a
//! mashup naming the other title as a part, or unclear), and whether the
//! artists agree, differ, or differ only because one file is credited to
//! the remixer.
//!
//! # Two readings of the names
//!
//! A pair's names are read twice ([`Reading`]): by title tag (a file with
//! none stands in with its file name), and by file name. The two can say
//! different things: a pair tagged "(Clean)" and "(Dirty)" can have file
//! names with nothing in common. One reading is picked, and the table is
//! applied to it:
//!
//! - **When the audio says "one production"** (identical, full, one inside
//!   the other), the most cautious reading wins, so that nothing merges
//!   that either reading would keep apart: a reading that says *another
//!   cut* first, then one that says *another rework*, *mashup part* or
//!   *unclear*, then *same*. *Another title* comes last: it never hides
//!   what the other reading says.
//! - **Otherwise the names decide**, and the reading that tells the files
//!   apart wins: *another rework*, then *another cut*, then *same*, then
//!   *mashup part*, *unclear*, *another title*. A version label one
//!   reading carries is a fact about the file that the other reading
//!   merely lacks; and a rework comes before a cut, as it does within one
//!   name, because a rework is never a stand-in.
//!
//! Of two readings that say the same kind of thing, the tags' wins.
//! [`Decision::reading`] says which one the reasons are about.
//!
//! # The table
//!
//! | Audio | Names | Outcome | Deciding reason |
//! |---|---|---|---|
//! | identical | any | duplicate | `identicalAudio` |
//! | full | same, or another title | duplicate | `sameAudio` |
//! | full | another cut | cut link | `versionVeto` |
//! | full | another rework, unclear, mashup part | needs review | `sameAudioNamesDisagree` |
//! | one inside the other | same, another cut, unclear | cut link | `audioInsideOther` |
//! | one inside the other | another rework, another title | needs review | `audioInsideOtherNamesDisagree` |
//! | one inside the other, partial | mashup part | rework link (mashup) | `mashupContains` |
//! | partial | same | needs review | `sameTagsPartialAudio` |
//! | partial | unclear, or the artists differ | needs review | `partialAudioNamesUnclear` |
//! | partial, none, not compared, unknown | another cut | cut link | `namesSayCut` |
//! | partial, none, not compared, unknown | another rework | rework link | `namesSayRework` |
//! | none | same | different | `fingerprintVeto` |
//! | not compared | same | needs review | `notCompared` |
//! | unknown | same | needs review | `noFingerprint` |
//! | none, not compared, unknown | same title, the artists differ | different | `sameTitleDifferentArtists` |
//! | none, not compared, unknown | mashup part | different | `mashupPart` |
//! | none, not compared, unknown | unclear | different | `namesUnclear` |
//! | partial, none, not compared, unknown | another title | different | `titlesDiffer` |
//!
//! The first reason of a [`Decision`] is the deciding one. The ones after
//! it are facts worth showing beside it: `namesDiffer` on a duplicate whose
//! names don't agree, `artistsDiffer`, `creditedToRemixer`, and what the
//! names said when the audio overruled or doubted them.
//!
//! A link made by names alone (`namesSayCut`, `namesSayRework`) needs the
//! artists to agree: the same title can be a different song. One file being
//! credited to the remixer the names give counts as agreeing.
//!
//! # What only merges on audio
//!
//! Nothing is ever a duplicate on names. Only identical or full audio
//! merges, and full audio doesn't when either reading of the names says
//! the two are different versions: Clean and Dirty fingerprint identically.
//!
//! # Known limits
//!
//! - **A truncated copy looks like a cut.** A file cut short holds audio
//!   that sits inside the whole file, so with the same names it gets a cut
//!   link. The rules have no quality input; the grouping job (1bC) has the
//!   quality verdict and decides what to do with such a pair.
//!   [`Pair`] is built through [`Pair::new`] so that a new input doesn't
//!   break its callers.
//! - **The score line for "one inside the other" is 4**, the duplicate
//!   rule's. A real cut whose found share is over 90% at a weaker score is
//!   a partial match, and is linked only if the names say it is a cut. The
//!   line is one constant ([`DUPLICATE_SCORE`]).
//! - **A remix credited to its remixer** agrees with any track of the same
//!   title, whoever made it: the names can't say which song was remixed.

use serde::{Serialize, Serializer};

use crate::matching;
use crate::versions::{self, MarkerKind, ParsedName};

#[cfg(test)]
mod tests;

/// The duplicate rule's coverage: at least this share of a file, 0 to 1
/// (ROADMAP 1.4).
pub const DUPLICATE_COVERAGE: f32 = 0.9;

/// The duplicate rule's difference score: at most this many differing bits
/// per fingerprint item (ROADMAP 1.4).
pub const DUPLICATE_SCORE: f64 = 4.0;

/// Two files share audio when at least this many fingerprint items match,
/// over every alignment. It is 1, the least `matching` ever reports: its
/// main alignment keeps any stretch of one item or more (about 0.12 s),
/// and a secondary alignment only stretches of
/// [`matching::compare::MIN_SECONDARY_ITEMS`] or more. The one place to
/// raise the floor if short chance matches turn out to matter.
pub const MIN_SHARED_ITEMS: usize = 1;

/// One of the two files of a [`Pair`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    A,
    B,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::A => Side::B,
            Side::B => Side::A,
        }
    }
}

impl From<versions::Side> for Side {
    fn from(side: versions::Side) -> Side {
        match side {
            versions::Side::A => Side::A,
            versions::Side::B => Side::B,
        }
    }
}

/// What `matching` measured between two files' full-track fingerprints.
/// The definitions are in [`crate::matching`]'s docs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measured {
    /// The share of A the main alignment matched, 0 to 1.
    pub coverage_a: f32,
    /// The share of B the main alignment matched, 0 to 1.
    pub coverage_b: f32,
    /// The difference score over the main alignment.
    pub score: f64,
    /// The share of A found anywhere in B, over every alignment.
    pub found_a: f32,
    /// The share of B found anywhere in A, over every alignment.
    pub found_b: f32,
    /// The difference score over everything found: every alignment's
    /// stretches, weighted by length. Equal to `score` when the main
    /// alignment is the only one.
    pub found_score: f64,
    /// How many fingerprint items matched, over every alignment.
    pub matched_items: usize,
}

impl Measured {
    /// The same measurements with the two files swapped.
    pub fn swapped(self) -> Measured {
        Measured {
            coverage_a: self.coverage_b,
            coverage_b: self.coverage_a,
            found_a: self.found_b,
            found_b: self.found_a,
            ..self
        }
    }
}

impl From<&matching::Comparison> for Measured {
    fn from(comparison: &matching::Comparison) -> Measured {
        let matched_items: usize = comparison.segments.iter().map(|s| s.items).sum();
        let found_score = match matched_items {
            0 => matching::compare::NO_MATCH_SCORE,
            _ => {
                comparison
                    .segments
                    .iter()
                    .map(|s| s.score * s.items as f64)
                    .sum::<f64>()
                    / matched_items as f64
            }
        };
        Measured {
            coverage_a: comparison.coverage_a,
            coverage_b: comparison.coverage_b,
            score: comparison.score,
            found_a: comparison.found_a(),
            found_b: comparison.found_b(),
            found_score,
            matched_items,
        }
    }
}

/// What is known about the two files' audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Audio {
    /// The very same audio: the files share an `audio_hash`, or a `blake3`
    /// (which also covers files with no `audio_hash`).
    Identical,
    /// `matching` compared the two fingerprints, and this is what it
    /// found. A comparison that found nothing is the only way to say "no
    /// audio shared".
    Compared(Measured),
    /// Both files have fingerprints, but nobody compared them: the block
    /// index didn't propose the pair. That is no finding at all (the index
    /// rarely proposes weak matches), so it never counts as "no audio
    /// shared".
    NotCompared,
    /// A file has no fingerprint, so the audio can't be compared.
    Unknown,
}

impl Audio {
    /// The same with the two files swapped.
    pub fn swapped(self) -> Audio {
        match self {
            Audio::Compared(measured) => Audio::Compared(measured.swapped()),
            other => other,
        }
    }
}

/// One name of a file, parsed, with and without its credits.
#[derive(Debug, Clone)]
struct Name {
    /// With the artist tag's artists among its credits.
    credited: ParsedName,
    /// With no credits at all: what the name alone says.
    uncredited: ParsedName,
}

impl Name {
    fn new(parsed: ParsedName, artist_tag: Option<&str>) -> Name {
        let mut credited = parsed.clone();
        if let Some(tag) = artist_tag {
            credited.credits.extend(versions::parse_artists(tag));
        }
        let mut uncredited = parsed;
        uncredited.credits.clear();
        Name {
            credited,
            uncredited,
        }
    }
}

/// What is known about one file's names: its title tag, if it has one, and
/// its file name, each parsed, and its artist tag.
#[derive(Debug, Clone)]
pub struct FileFacts {
    tagged: Option<Name>,
    file_name: Name,
}

impl FileFacts {
    /// `title_tag` and `artist_tag` are the file's tags as written (a blank
    /// title tag counts as none); `file_name` is its name on disk. The
    /// artist tag is split into artists as the artist part of a file name
    /// is, and counts for both names.
    pub fn new(title_tag: Option<&str>, artist_tag: Option<&str>, file_name: &str) -> FileFacts {
        let tagged = title_tag
            .filter(|title| !title.trim().is_empty())
            .map(|title| Name::new(versions::parse_title(title), artist_tag));
        FileFacts {
            tagged,
            file_name: Name::new(versions::parse_file_name(file_name), artist_tag),
        }
    }
}

/// Which of a pair's names a decision went by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reading {
    /// The title tags; for a file with none, its file name.
    Tags,
    /// The file names.
    FileNames,
}

/// Two files and what is known about them. Built with [`Pair::new`], so a
/// later input (the quality verdict, 1bC) can be added without touching
/// the callers.
#[derive(Debug, Clone, Copy)]
pub struct Pair<'a> {
    audio: Audio,
    a: &'a FileFacts,
    b: &'a FileFacts,
}

impl<'a> Pair<'a> {
    /// `audio` is seen from `a`: its A is `a`, its B is `b`.
    pub fn new(audio: Audio, a: &'a FileFacts, b: &'a FileFacts) -> Pair<'a> {
        Pair { audio, a, b }
    }

    /// The same pair with the two files swapped.
    pub fn swapped(&self) -> Pair<'a> {
        Pair {
            audio: self.audio.swapped(),
            a: self.b,
            b: self.a,
        }
    }
}

fn kind_name<S: Serializer>(kind: &MarkerKind, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(kind.as_str())
}

fn kind_names<S: Serializer>(kinds: &[MarkerKind], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(kinds.iter().map(|kind| kind.as_str()))
}

/// A rework as a name states it: its kind and whose it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rework {
    #[serde(serialize_with = "kind_name")]
    pub kind: MarkerKind,
    /// The remixer of a remix, the handle of a bootleg edit. As written.
    pub whose: Option<String>,
}

impl From<&versions::ReworkRef> for Rework {
    fn from(rework: &versions::ReworkRef) -> Rework {
        Rework {
            kind: rework.kind,
            whose: rework.detail.clone(),
        }
    }
}

/// What two files are to each other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    /// The same track: the files merge.
    Duplicate,
    /// Two cuts of one production: linked, never merged. `labels` are the
    /// cuts the names tell apart (Clean and Dirty; Extended), in
    /// [`MarkerKind`]'s order; empty when only the audio says so.
    CutLink {
        #[serde(serialize_with = "kind_names")]
        labels: Vec<MarkerKind>,
    },
    /// Two productions of one song: linked, never merged. `labels` are the
    /// kinds of rework the names tell apart (remix, flip, mashup…), in
    /// [`MarkerKind`]'s order; never empty.
    ReworkLink {
        #[serde(serialize_with = "kind_names")]
        labels: Vec<MarkerKind>,
    },
    /// Nothing to each other.
    Different,
    /// The signals disagree, or can't settle it: the user decides.
    NeedsReview,
}

/// Why [`decide`] answered as it did: a code ([`Reason::code`]) and its
/// parameters. Never English text.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Reason {
    // ---- Deciding reasons: one of these comes first ----
    /// The files hold the very same audio.
    IdenticalAudio,
    /// The audio matches in full, both ways (the duplicate rule).
    SameAudio {
        coverage_a: f32,
        coverage_b: f32,
        score: f64,
    },
    /// The audio matches in full, but the names give two different cuts:
    /// the cuts only one name has.
    VersionVeto {
        #[serde(serialize_with = "kind_names")]
        only_a: Vec<MarkerKind>,
        #[serde(serialize_with = "kind_names")]
        only_b: Vec<MarkerKind>,
    },
    /// The audio matches in full, but the names say it shouldn't, or don't
    /// settle whether these are two versions.
    SameAudioNamesDisagree,
    /// `inner`'s audio sits inside the other file's.
    AudioInsideOther {
        inner: Side,
        found_a: f32,
        found_b: f32,
    },
    /// `inner`'s audio sits inside the other file's, but the names say
    /// these aren't two cuts of one production.
    AudioInsideOtherNamesDisagree {
        inner: Side,
        found_a: f32,
        found_b: f32,
    },
    /// The names are the same, but only part of the audio is shared:
    /// identical tags can hide a different recording.
    SameTagsPartialAudio { found_a: f32, found_b: f32 },
    /// Part of the audio is shared, and the names don't settle what the
    /// files are to each other.
    PartialAudioNamesUnclear { found_a: f32, found_b: f32 },
    /// The names give two different cuts: the cuts only one name has.
    NamesSayCut {
        #[serde(serialize_with = "kind_names")]
        only_a: Vec<MarkerKind>,
        #[serde(serialize_with = "kind_names")]
        only_b: Vec<MarkerKind>,
    },
    /// The names give two different reworks: the ones only one name has.
    NamesSayRework {
        only_a: Vec<Rework>,
        only_b: Vec<Rework>,
    },
    /// `mashup` names the other file's title as one of its parts, and the
    /// two share audio.
    MashupContains { mashup: Side, part: String },
    /// The names are the same, but the fingerprints share nothing.
    FingerprintVeto,
    /// The names are the same, and a file has no fingerprint to check
    /// them against.
    NoFingerprint,
    /// The names are the same, and the two fingerprints were never
    /// compared.
    NotCompared,
    /// The same title under different artists, with no audio shared (or
    /// none to compare): the same title can be a different song.
    SameTitleDifferentArtists,
    /// `mashup` names the other file's title as one of its parts. Only a
    /// hint: part names match unrelated tracks too.
    MashupPart { mashup: Side, part: String },
    /// The names don't settle what the files are to each other. `doubts`
    /// are the codes of [`crate::versions::Reason`] the names gave.
    NamesUnclear { doubts: Vec<&'static str> },
    /// The titles differ.
    TitlesDiffer,

    // ---- Only ever beside a deciding reason ----
    /// A duplicate whose names don't agree.
    NamesDiffer,
    /// The files credit different artists.
    ArtistsDiffer,
    /// The artists differ, but the artist of each of `sides` is the remixer
    /// the names give: a remix filed under the remixer alone. A first.
    CreditedToRemixer { sides: Vec<Side> },
}

impl Reason {
    /// The reason's code: what it serializes as, and what an i18n key is
    /// built from.
    pub fn code(&self) -> &'static str {
        match self {
            Reason::IdenticalAudio => "identicalAudio",
            Reason::SameAudio { .. } => "sameAudio",
            Reason::VersionVeto { .. } => "versionVeto",
            Reason::SameAudioNamesDisagree => "sameAudioNamesDisagree",
            Reason::AudioInsideOther { .. } => "audioInsideOther",
            Reason::AudioInsideOtherNamesDisagree { .. } => "audioInsideOtherNamesDisagree",
            Reason::SameTagsPartialAudio { .. } => "sameTagsPartialAudio",
            Reason::PartialAudioNamesUnclear { .. } => "partialAudioNamesUnclear",
            Reason::NamesSayCut { .. } => "namesSayCut",
            Reason::NamesSayRework { .. } => "namesSayRework",
            Reason::MashupContains { .. } => "mashupContains",
            Reason::FingerprintVeto => "fingerprintVeto",
            Reason::NoFingerprint => "noFingerprint",
            Reason::NotCompared => "notCompared",
            Reason::SameTitleDifferentArtists => "sameTitleDifferentArtists",
            Reason::MashupPart { .. } => "mashupPart",
            Reason::NamesUnclear { .. } => "namesUnclear",
            Reason::TitlesDiffer => "titlesDiffer",
            Reason::NamesDiffer => "namesDiffer",
            Reason::ArtistsDiffer => "artistsDiffer",
            Reason::CreditedToRemixer { .. } => "creditedToRemixer",
        }
    }

    /// The same reason with the two files swapped.
    pub fn swapped(self) -> Reason {
        match self {
            Reason::SameAudio {
                coverage_a,
                coverage_b,
                score,
            } => Reason::SameAudio {
                coverage_a: coverage_b,
                coverage_b: coverage_a,
                score,
            },
            Reason::VersionVeto { only_a, only_b } => Reason::VersionVeto {
                only_a: only_b,
                only_b: only_a,
            },
            Reason::AudioInsideOther {
                inner,
                found_a,
                found_b,
            } => Reason::AudioInsideOther {
                inner: inner.other(),
                found_a: found_b,
                found_b: found_a,
            },
            Reason::AudioInsideOtherNamesDisagree {
                inner,
                found_a,
                found_b,
            } => Reason::AudioInsideOtherNamesDisagree {
                inner: inner.other(),
                found_a: found_b,
                found_b: found_a,
            },
            Reason::SameTagsPartialAudio { found_a, found_b } => Reason::SameTagsPartialAudio {
                found_a: found_b,
                found_b: found_a,
            },
            Reason::PartialAudioNamesUnclear { found_a, found_b } => {
                Reason::PartialAudioNamesUnclear {
                    found_a: found_b,
                    found_b: found_a,
                }
            }
            Reason::NamesSayCut { only_a, only_b } => Reason::NamesSayCut {
                only_a: only_b,
                only_b: only_a,
            },
            Reason::NamesSayRework { only_a, only_b } => Reason::NamesSayRework {
                only_a: only_b,
                only_b: only_a,
            },
            Reason::MashupContains { mashup, part } => Reason::MashupContains {
                mashup: mashup.other(),
                part,
            },
            Reason::MashupPart { mashup, part } => Reason::MashupPart {
                mashup: mashup.other(),
                part,
            },
            Reason::CreditedToRemixer { sides } => {
                let mut sides: Vec<Side> = sides.into_iter().map(Side::other).collect();
                sides.sort_unstable();
                Reason::CreditedToRemixer { sides }
            }
            same @ (Reason::IdenticalAudio
            | Reason::SameAudioNamesDisagree
            | Reason::FingerprintVeto
            | Reason::NoFingerprint
            | Reason::NotCompared
            | Reason::SameTitleDifferentArtists
            | Reason::NamesUnclear { .. }
            | Reason::TitlesDiffer
            | Reason::NamesDiffer
            | Reason::ArtistsDiffer) => same,
        }
    }
}

/// [`decide`]'s answer. `reasons` is never empty, and its first is the
/// deciding one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub outcome: Outcome,
    pub reasons: Vec<Reason>,
    /// Which of the pair's names the reasons are about.
    pub reading: Reading,
}

impl Decision {
    /// The same decision with the two files swapped.
    pub fn swapped(self) -> Decision {
        Decision {
            outcome: self.outcome,
            reasons: self.reasons.into_iter().map(Reason::swapped).collect(),
            reading: self.reading,
        }
    }
}

/// The audio, sorted into the table's cases.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shared {
    Identical,
    Full(Measured),
    Inside(Side, Measured),
    Partial(Measured),
    Nothing,
    NotCompared,
    Unknown,
}

fn shared(audio: Audio) -> Shared {
    let measured = match audio {
        Audio::Identical => return Shared::Identical,
        Audio::NotCompared => return Shared::NotCompared,
        Audio::Unknown => return Shared::Unknown,
        Audio::Compared(measured) => measured,
    };
    let full = measured.coverage_a >= DUPLICATE_COVERAGE
        && measured.coverage_b >= DUPLICATE_COVERAGE
        && measured.score <= DUPLICATE_SCORE;
    if full {
        return Shared::Full(measured);
    }
    let found_a = measured.found_a >= DUPLICATE_COVERAGE;
    let found_b = measured.found_b >= DUPLICATE_COVERAGE;
    if found_a != found_b && measured.found_score <= DUPLICATE_SCORE {
        let inner = match found_a {
            true => Side::A,
            false => Side::B,
        };
        return Shared::Inside(inner, measured);
    }
    if measured.matched_items >= MIN_SHARED_ITEMS {
        return Shared::Partial(measured);
    }
    Shared::Nothing
}

/// How the two names relate, artists aside.
#[derive(Debug, Clone, PartialEq)]
enum Relation {
    Same,
    Cut {
        only_a: Vec<MarkerKind>,
        only_b: Vec<MarkerKind>,
    },
    Rework {
        only_a: Vec<Rework>,
        only_b: Vec<Rework>,
    },
    OtherTitle,
    MashupPart {
        mashup: Side,
        part: String,
    },
    Unclear {
        doubts: Vec<&'static str>,
    },
}

/// What the credits say about two files with the same title.
#[derive(Debug, Clone, PartialEq)]
enum Credits {
    /// The artists agree, or a file names none.
    Agree,
    /// The artists differ, but these files are credited to the remixer.
    Remixer(Vec<Side>),
    Differ,
}

fn relation(names: &versions::Comparison) -> Relation {
    use versions::Reason as Said;
    match names.outcome {
        versions::Outcome::SameVersion => return Relation::Same,
        versions::Outcome::DifferentBase => return Relation::OtherTitle,
        versions::Outcome::DifferentCut => {
            for reason in &names.reasons {
                if let Said::CutDiffers { only_a, only_b } = reason {
                    return Relation::Cut {
                        only_a: only_a.clone(),
                        only_b: only_b.clone(),
                    };
                }
            }
        }
        versions::Outcome::DifferentRework => {
            for reason in &names.reasons {
                if let Said::ReworkDiffers { only_a, only_b } = reason {
                    return Relation::Rework {
                        only_a: only_a.iter().map(Rework::from).collect(),
                        only_b: only_b.iter().map(Rework::from).collect(),
                    };
                }
            }
        }
        versions::Outcome::CantTell => {
            // A mashup naming the other title as a part. Two mashups that
            // share a part aren't each other's source: that stays unclear.
            let parts: Vec<(Side, &String)> = names
                .reasons
                .iter()
                .filter_map(|reason| match reason {
                    Said::MashupPartMatch { side, part } => Some((Side::from(*side), part)),
                    _ => None,
                })
                .collect();
            if let Some(&(mashup, part)) = parts.first() {
                if parts.iter().all(|(side, _)| *side == mashup) {
                    return Relation::MashupPart {
                        mashup,
                        part: part.clone(),
                    };
                }
            }
        }
    }
    let mut doubts: Vec<&'static str> = Vec::new();
    for reason in &names.reasons {
        let code = reason.code();
        if !matches!(reason, Said::BaseTitlesMatch) && !doubts.contains(&code) {
            doubts.push(code);
        }
    }
    // Sorted, so the answer is the same whichever file comes first.
    doubts.sort_unstable();
    Relation::Unclear { doubts }
}

/// What one reading of the pair's names says.
#[derive(Debug, Clone, PartialEq)]
struct Read {
    reading: Reading,
    relation: Relation,
    credits: Credits,
}

fn read(reading: Reading, a: &Name, b: &Name) -> Read {
    use versions::Reason as Said;
    let credited = versions::compare(&a.credited, &b.credited);
    let mut remixers: Vec<Side> = credited
        .reasons
        .iter()
        .filter_map(|reason| match reason {
            Said::CreditedToRemixer { side } => Some(Side::from(*side)),
            _ => None,
        })
        .collect();
    remixers.sort_unstable();
    let (relation, credits) = if credited.reasons.contains(&Said::ArtistsDiffer) {
        // The artists make the names "can't tell"; what the names say
        // without them is the relation.
        let plain = versions::compare(&a.uncredited, &b.uncredited);
        (relation(&plain), Credits::Differ)
    } else if remixers.is_empty() {
        (relation(&credited), Credits::Agree)
    } else {
        (relation(&credited), Credits::Remixer(remixers))
    };
    Read {
        reading,
        relation,
        credits,
    }
}

/// Every reading of the pair's names, the tags' first. Two files with no
/// title tag have one reading, their file names.
fn readings(a: &FileFacts, b: &FileFacts) -> Vec<Read> {
    let by_file_name = read(Reading::FileNames, &a.file_name, &b.file_name);
    if a.tagged.is_none() && b.tagged.is_none() {
        return vec![by_file_name];
    }
    let by_tags = read(
        Reading::Tags,
        a.tagged.as_ref().unwrap_or(&a.file_name),
        b.tagged.as_ref().unwrap_or(&b.file_name),
    );
    vec![by_tags, by_file_name]
}

/// Where a relation stands among the readings: the lowest wins, and of two
/// readings equally low, the tags'.
///
/// With the audio matched in full, or one file inside the other, the audio
/// says "one production", so the reading that would stop a merge comes
/// first: a cut, then whatever asks for review. "Another title" comes last:
/// it never hides what the other reading says.
///
/// Otherwise the names decide, and the reading that tells the two files
/// apart comes first: a label one reading carries is a fact about the file
/// that the other reading merely lacks. A rework comes before a cut, as it
/// does within one name: a rework is never a stand-in.
fn rank(audio_led: bool, relation: &Relation) -> u8 {
    match (audio_led, relation) {
        (true, Relation::Cut { .. }) => 0,
        (true, Relation::Rework { .. }) => 1,
        (true, Relation::MashupPart { .. }) => 2,
        (true, Relation::Unclear { .. }) => 3,
        (true, Relation::Same) => 4,
        (false, Relation::Rework { .. }) => 0,
        (false, Relation::Cut { .. }) => 1,
        (false, Relation::Same) => 2,
        (false, Relation::MashupPart { .. }) => 3,
        (false, Relation::Unclear { .. }) => 4,
        (_, Relation::OtherTitle) => 5,
    }
}

/// The labels of a link: the kinds either name has alone, in
/// [`MarkerKind`]'s order.
fn labels(kinds: impl Iterator<Item = MarkerKind>) -> Vec<MarkerKind> {
    let mut labels: Vec<MarkerKind> = kinds.collect();
    labels.sort();
    labels.dedup();
    labels
}

fn cut_labels(only_a: &[MarkerKind], only_b: &[MarkerKind]) -> Vec<MarkerKind> {
    labels(only_a.iter().chain(only_b).copied())
}

fn rework_labels(only_a: &[Rework], only_b: &[Rework]) -> Vec<MarkerKind> {
    labels(only_a.iter().chain(only_b).map(|rework| rework.kind))
}

/// What the names said, as a reason to show beside the deciding one.
fn said(relation: Relation) -> Option<Reason> {
    match relation {
        Relation::Same => None,
        Relation::Cut { only_a, only_b } => Some(Reason::NamesSayCut { only_a, only_b }),
        Relation::Rework { only_a, only_b } => Some(Reason::NamesSayRework { only_a, only_b }),
        Relation::OtherTitle => Some(Reason::TitlesDiffer),
        Relation::MashupPart { mashup, part } => Some(Reason::MashupPart { mashup, part }),
        Relation::Unclear { doubts } => Some(Reason::NamesUnclear { doubts }),
    }
}

/// What the credits said, as reasons to show beside the deciding one.
fn credit_notes(credits: &Credits) -> Vec<Reason> {
    match credits {
        Credits::Agree => Vec::new(),
        Credits::Differ => vec![Reason::ArtistsDiffer],
        Credits::Remixer(sides) => vec![Reason::CreditedToRemixer {
            sides: sides.clone(),
        }],
    }
}

/// Decides what the pair's two files are to each other. The table is in
/// the [module docs](self). The answer doesn't depend on which file comes
/// first: swapping them swaps the sides in the reasons, nothing else.
pub fn decide(pair: &Pair<'_>) -> Decision {
    let shared = shared(pair.audio);
    let audio_led = matches!(
        shared,
        Shared::Identical | Shared::Full(_) | Shared::Inside(..)
    );
    // `min_by_key` keeps the first of equals: the tags' reading.
    let Read {
        reading,
        relation,
        credits,
    } = readings(pair.a, pair.b)
        .into_iter()
        .min_by_key(|read| rank(audio_led, &read.relation))
        .expect("a pair always has a reading");
    // The credits only say something under one title: names with another
    // title are never compared by artist.
    let artists_differ = credits == Credits::Differ;
    let notes = credit_notes(&credits);

    let (outcome, mut reasons) = match shared {
        // The very same audio can't be two versions, whatever the names
        // say: one of the files is mislabeled.
        Shared::Identical => {
            let mut reasons = vec![Reason::IdenticalAudio];
            if relation != Relation::Same {
                reasons.push(Reason::NamesDiffer);
            }
            (Outcome::Duplicate, reasons)
        }
        Shared::Full(m) => {
            let same_audio = Reason::SameAudio {
                coverage_a: m.coverage_a,
                coverage_b: m.coverage_b,
                score: m.score,
            };
            match relation {
                Relation::Same => (Outcome::Duplicate, vec![same_audio]),
                Relation::OtherTitle => (Outcome::Duplicate, vec![same_audio, Reason::NamesDiffer]),
                // The version check's veto: Clean and Dirty fingerprint
                // identically.
                Relation::Cut { only_a, only_b } => (
                    Outcome::CutLink {
                        labels: cut_labels(&only_a, &only_b),
                    },
                    vec![Reason::VersionVeto { only_a, only_b }],
                ),
                unsettled => {
                    let mut reasons = vec![Reason::SameAudioNamesDisagree];
                    reasons.extend(said(unsettled));
                    (Outcome::NeedsReview, reasons)
                }
            }
        }
        // One-sided coverage is a cut: linked, never merged.
        Shared::Inside(inner, m) => {
            let inside = Reason::AudioInsideOther {
                inner,
                found_a: m.found_a,
                found_b: m.found_b,
            };
            let disagree = Reason::AudioInsideOtherNamesDisagree {
                inner,
                found_a: m.found_a,
                found_b: m.found_b,
            };
            match relation {
                Relation::Same => (Outcome::CutLink { labels: Vec::new() }, vec![inside]),
                Relation::Cut { only_a, only_b } => (
                    Outcome::CutLink {
                        labels: cut_labels(&only_a, &only_b),
                    },
                    vec![inside, Reason::NamesSayCut { only_a, only_b }],
                ),
                Relation::Unclear { doubts } => (
                    Outcome::CutLink { labels: Vec::new() },
                    vec![inside, Reason::NamesUnclear { doubts }],
                ),
                Relation::MashupPart { mashup, part } => (
                    Outcome::ReworkLink {
                        labels: vec![MarkerKind::Mashup],
                    },
                    vec![Reason::MashupContains { mashup, part }],
                ),
                against @ (Relation::Rework { .. } | Relation::OtherTitle) => {
                    let mut reasons = vec![disagree];
                    reasons.extend(said(against));
                    (Outcome::NeedsReview, reasons)
                }
            }
        }
        Shared::Partial(m) => {
            let unclear = Reason::PartialAudioNamesUnclear {
                found_a: m.found_a,
                found_b: m.found_b,
            };
            match relation {
                Relation::OtherTitle => (Outcome::Different, vec![Reason::TitlesDiffer]),
                Relation::MashupPart { mashup, part } => (
                    Outcome::ReworkLink {
                        labels: vec![MarkerKind::Mashup],
                    },
                    vec![Reason::MashupContains { mashup, part }],
                ),
                Relation::Unclear { doubts } => (
                    Outcome::NeedsReview,
                    vec![unclear, Reason::NamesUnclear { doubts }],
                ),
                // Shared audio says "related", the artists say "another
                // song": the signals disagree.
                related if artists_differ => {
                    let mut reasons = vec![unclear];
                    reasons.extend(said(related));
                    (Outcome::NeedsReview, reasons)
                }
                // Identical tags can hide a different recording.
                Relation::Same => (
                    Outcome::NeedsReview,
                    vec![Reason::SameTagsPartialAudio {
                        found_a: m.found_a,
                        found_b: m.found_b,
                    }],
                ),
                Relation::Cut { only_a, only_b } => (
                    Outcome::CutLink {
                        labels: cut_labels(&only_a, &only_b),
                    },
                    vec![Reason::NamesSayCut { only_a, only_b }],
                ),
                Relation::Rework { only_a, only_b } => (
                    Outcome::ReworkLink {
                        labels: rework_labels(&only_a, &only_b),
                    },
                    vec![Reason::NamesSayRework { only_a, only_b }],
                ),
            }
        }
        // No audio to go on: the names decide whether it's a link, and
        // never make a duplicate.
        nothing @ (Shared::Nothing | Shared::NotCompared | Shared::Unknown) => match relation {
            Relation::OtherTitle => (Outcome::Different, vec![Reason::TitlesDiffer]),
            Relation::MashupPart { mashup, part } => (
                Outcome::Different,
                vec![Reason::MashupPart { mashup, part }],
            ),
            Relation::Unclear { doubts } => {
                (Outcome::Different, vec![Reason::NamesUnclear { doubts }])
            }
            // The same title can be a different song, even as remixes.
            _ if artists_differ => (Outcome::Different, vec![Reason::SameTitleDifferentArtists]),
            Relation::Same => match nothing {
                Shared::Unknown => (Outcome::NeedsReview, vec![Reason::NoFingerprint]),
                // Not compared is no finding: never "different".
                Shared::NotCompared => (Outcome::NeedsReview, vec![Reason::NotCompared]),
                // The fingerprint's veto on a name-based merge.
                _ => (Outcome::Different, vec![Reason::FingerprintVeto]),
            },
            Relation::Cut { only_a, only_b } => (
                Outcome::CutLink {
                    labels: cut_labels(&only_a, &only_b),
                },
                vec![Reason::NamesSayCut { only_a, only_b }],
            ),
            Relation::Rework { only_a, only_b } => (
                Outcome::ReworkLink {
                    labels: rework_labels(&only_a, &only_b),
                },
                vec![Reason::NamesSayRework { only_a, only_b }],
            ),
        },
    };

    // "Different under other artists" already says what the credits said.
    if reasons[0] != Reason::SameTitleDifferentArtists {
        reasons.extend(notes);
    }
    Decision {
        outcome,
        reasons,
        reading,
    }
}
