//! Ranking the files of one track, with the reason for each place (1bC-2).
//! The order and what each step means are in the module docs.

use std::cmp::{Ordering, Reverse};

use serde::Serialize;

use super::pick::tags_filled;
use super::{FileFacts, FolderRole, Format, Place, RekordboxLink, VolumeKind};
use crate::quality::verdict::{Verdict, POSSIBLY_LOWER_BELOW_HZ};
use crate::read::Codec;

/// A cutoff counts in steps of this many Hz: two files whose cutoffs are
/// within one step are the same quality, because a cutoff is measured, not
/// known, and an encoder's low-pass wobbles by a few hundred Hz.
pub const CUTOFF_STEP_HZ: u32 = 1_000;

/// From this cutoff up a file is full band: all such cutoffs count as one
/// (the verdict's own line: below it a high-bitrate MP3 is "possibly lower
/// quality").
pub const FULL_BAND_FROM_HZ: u32 = POSSIBLY_LOWER_BELOW_HZ;

/// Lossless codecs, best first. FLAC, then WAV and AIFF (both are PCM), then
/// ALAC, then the formats rekordbox doesn't play. A proposal: when files of
/// equal quality differ only here, the choice matters little, so the order
/// follows tag support and how widely the format plays.
pub const LOSSLESS_ORDER: [Codec; 6] = [
    Codec::Flac,
    Codec::Pcm,
    Codec::PcmFloat,
    Codec::Alac,
    Codec::WavPack,
    Codec::Ape,
];

/// Lossy codecs, best first: the ones every player and rekordbox play, then
/// the ones they don't. A proposal.
pub const LOSSY_ORDER: [Codec; 5] = [
    Codec::Mp3,
    Codec::Aac,
    Codec::Vorbis,
    Codec::Opus,
    Codec::Wma,
];

/// The steps of the order, first to last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    OnDisk,
    Verdict,
    Playable,
    Cutoff,
    DecodeErrors,
    Format,
    Tags,
    Art,
    Rekordbox,
    Place,
    Tied,
}

/// Why one file is ahead of another: the first step that separates them,
/// and the facts of both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum Reason {
    /// The track has one file; nothing to compare it to.
    OnlyFile,
    /// The other file is gone and this one is there.
    NotOnDisk,
    /// A better verdict (`None`: not checked).
    BetterVerdict {
        best: Option<Verdict>,
        other: Option<Verdict>,
    },
    /// This file plays in the DJ app and the other doesn't.
    Playable,
    /// A higher measured cutoff (`None`: not measured).
    #[serde(rename_all = "camelCase")]
    HigherCutoff {
        best_hz: Option<u32>,
        other_hz: Option<u32>,
    },
    /// Fewer packets failed to decode.
    FewerDecodeErrors { best: u32, other: u32 },
    /// A better format: "a FLAC against your 320 kbps MP3".
    BetterFormat { best: Format, other: Format },
    /// More of the descriptive tags filled in.
    MoreTags { best: u32, other: u32 },
    /// It has embedded art and the other doesn't.
    HasArt,
    /// rekordbox knows this file better.
    KnownToRekordbox {
        best: RekordboxLink,
        other: RekordboxLink,
    },
    /// It lives in a better place.
    BetterPlace { best: Place, other: Place },
    /// Nothing separates them; the lower file id.
    Tied,
}

impl Reason {
    /// The step that decided, or `None` for [`Reason::OnlyFile`].
    pub fn step(&self) -> Option<Step> {
        Some(match self {
            Reason::OnlyFile => return None,
            Reason::NotOnDisk => Step::OnDisk,
            Reason::BetterVerdict { .. } => Step::Verdict,
            Reason::Playable => Step::Playable,
            Reason::HigherCutoff { .. } => Step::Cutoff,
            Reason::FewerDecodeErrors { .. } => Step::DecodeErrors,
            Reason::BetterFormat { .. } => Step::Format,
            Reason::MoreTags { .. } => Step::Tags,
            Reason::HasArt => Step::Art,
            Reason::KnownToRekordbox { .. } => Step::Rekordbox,
            Reason::BetterPlace { .. } => Step::Place,
            Reason::Tied => Step::Tied,
        })
    }
}

/// One row of a [`Ranking`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranked {
    pub file_id: i64,
    /// For the best file: why it is ahead of the second. For every other
    /// file: why the best file is ahead of it.
    pub reason: Reason,
}

/// The files of a track, best first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranking {
    pub order: Vec<Ranked>,
}

impl Ranking {
    /// The best file's id; `None` for a track with no files.
    pub fn best(&self) -> Option<i64> {
        self.order.first().map(|row| row.file_id)
    }
}

/// Orders `files`, best first, with a reason for every row. The same files
/// in any order give the same ranking.
pub fn rank(files: &[FileFacts]) -> Ranking {
    let sorted = sorted(files);
    let mut order = Vec::with_capacity(sorted.len());
    if let Some(best) = sorted.first() {
        let against_best = |other: &FileFacts| why_ahead(best, other).unwrap_or(Reason::Tied);
        order.push(Ranked {
            file_id: best.file_id,
            reason: sorted
                .get(1)
                .map_or(Reason::OnlyFile, |second| against_best(second)),
        });
        for other in &sorted[1..] {
            order.push(Ranked {
                file_id: other.file_id,
                reason: against_best(other),
            });
        }
    }
    Ranking { order }
}

/// Why `a` is ahead of `b`: the step that separates them, with both sides'
/// facts. `None` when `b` is ahead (ask the other way round) or when they
/// are the same file.
pub fn why_ahead(a: &FileFacts, b: &FileFacts) -> Option<Reason> {
    let (step, ordering) = first_difference(a, b)?;
    (ordering == Ordering::Greater).then(|| reason(step, a, b))
}

/// The files best first, as references.
pub(super) fn sorted(files: &[FileFacts]) -> Vec<&FileFacts> {
    let mut sorted: Vec<&FileFacts> = files.iter().collect();
    sorted.sort_by(|a, b| compare(b, a));
    sorted
}

/// How `a` stands against `b`: `Greater` when `a` is ahead.
fn compare(a: &FileFacts, b: &FileFacts) -> Ordering {
    first_difference(a, b).map_or(Ordering::Equal, |(_, ordering)| ordering)
}

/// The first step that separates two files, and how `a` stands there. The
/// one place the order of the steps is written.
fn first_difference(a: &FileFacts, b: &FileFacts) -> Option<(Step, Ordering)> {
    let steps = [
        (Step::OnDisk, a.on_disk.cmp(&b.on_disk)),
        (Step::Verdict, tier(a).cmp(&tier(b))),
        (Step::Playable, a.plays_in_dj_app.cmp(&b.plays_in_dj_app)),
        (Step::Cutoff, cutoff_level(a).cmp(&cutoff_level(b))),
        (
            Step::DecodeErrors,
            b.quality.decode_errors.cmp(&a.quality.decode_errors),
        ),
        (Step::Format, format_rank(a).cmp(&format_rank(b))),
        (Step::Tags, tags_filled(&a.tags).cmp(&tags_filled(&b.tags))),
        (Step::Art, a.art.is_some().cmp(&b.art.is_some())),
        (Step::Rekordbox, a.rekordbox.cmp(&b.rekordbox)),
        (Step::Place, place_rank(&a.place).cmp(&place_rank(&b.place))),
        (Step::Tied, b.file_id.cmp(&a.file_id)),
    ];
    steps.into_iter().find(|(_, ordering)| ordering.is_ne())
}

fn reason(step: Step, best: &FileFacts, other: &FileFacts) -> Reason {
    match step {
        Step::OnDisk => Reason::NotOnDisk,
        Step::Verdict => Reason::BetterVerdict {
            best: best.quality.verdict,
            other: other.quality.verdict,
        },
        Step::Playable => Reason::Playable,
        Step::Cutoff => Reason::HigherCutoff {
            best_hz: counted_cutoff(best),
            other_hz: counted_cutoff(other),
        },
        Step::DecodeErrors => Reason::FewerDecodeErrors {
            best: best.quality.decode_errors,
            other: other.quality.decode_errors,
        },
        Step::Format => Reason::BetterFormat {
            best: best.format,
            other: other.format,
        },
        Step::Tags => Reason::MoreTags {
            best: tags_filled(&best.tags),
            other: tags_filled(&other.tags),
        },
        Step::Art => Reason::HasArt,
        Step::Rekordbox => Reason::KnownToRekordbox {
            best: best.rekordbox,
            other: other.rekordbox,
        },
        Step::Place => Reason::BetterPlace {
            best: best.place,
            other: other.place,
        },
        Step::Tied => Reason::Tied,
    }
}

/// The verdict classes. Greater is better. `Flagged` holds both `low
/// bitrate` and `suspect transcode`: they play whole, and the cutoff
/// separates them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    Broken,
    Truncated,
    Flagged,
    NotChecked,
    Ok,
}

fn tier(file: &FileFacts) -> Tier {
    match file.quality.verdict {
        Some(Verdict::Ok) => Tier::Ok,
        None => Tier::NotChecked,
        Some(Verdict::LowBitrate | Verdict::SuspectTranscode) => Tier::Flagged,
        Some(Verdict::Truncated) => Tier::Truncated,
        Some(Verdict::Broken) => Tier::Broken,
    }
}

/// The cutoff a file's rank counts: only with a verdict.
fn counted_cutoff(file: &FileFacts) -> Option<u32> {
    file.quality.verdict.and(file.quality.cutoff_hz)
}

/// The cutoff in steps, with everything full band as one. `None` (not
/// measured) is behind any measured level.
fn cutoff_level(file: &FileFacts) -> Option<u32> {
    counted_cutoff(file).map(|hz| hz.min(FULL_BAND_FROM_HZ) / CUTOFF_STEP_HZ)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    /// Codecs nobody ranked: telephony, speech, surround, and a codec the
    /// app doesn't know. The verdict has no lines for them either.
    Unrated,
    Lossy,
    Lossless,
}

/// The one table of which codec is which class. Exhaustive on purpose.
fn class(codec: Option<Codec>) -> Class {
    let Some(codec) = codec else {
        return Class::Unrated;
    };
    match codec {
        Codec::Flac | Codec::Alac | Codec::Pcm | Codec::PcmFloat | Codec::WavPack | Codec::Ape => {
            Class::Lossless
        }
        Codec::Mp3 | Codec::Aac | Codec::Vorbis | Codec::Opus | Codec::Wma => Class::Lossy,
        Codec::Ac3
        | Codec::Eac3
        | Codec::Musepack
        | Codec::AdpcmMs
        | Codec::AdpcmIma
        | Codec::Alaw
        | Codec::Mulaw
        | Codec::Speex
        | Codec::Other => Class::Unrated,
    }
}

/// Greater is better: the class, then the codec's place in its list, then
/// (lossy only) the header bitrate.
fn format_rank(file: &FileFacts) -> (Class, usize, Option<u32>) {
    let class = class(file.format.codec);
    let list: &[Codec] = match class {
        Class::Lossless => &LOSSLESS_ORDER,
        Class::Lossy => &LOSSY_ORDER,
        Class::Unrated => &[],
    };
    // Earlier in the list is better; a codec the list doesn't name is last.
    let place = file
        .format
        .codec
        .and_then(|codec| list.iter().position(|c| *c == codec))
        .map_or(0, |index| list.len() - index);
    let kbps = (class == Class::Lossy)
        .then_some(file.format.kbps)
        .flatten();
    (class, place, kbps)
}

/// Greater is better: a connected volume, the scan folder, the kind of
/// volume, the folder added first.
fn place_rank(place: &Place) -> (bool, FolderRole, VolumeKind, Reverse<i64>) {
    (
        place.volume_online,
        place.folder_role,
        place.volume_kind,
        Reverse(place.folder_order),
    )
}
