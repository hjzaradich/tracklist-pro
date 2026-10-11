//! The best file of a track, and where each tag comes from (ROADMAP 1.7;
//! 1bC-2 and 1bC-3).
//!
//! A pure decision library: no database, no job, no files, no text a user
//! reads. Given the facts about each file of one track ([`FileFacts`]),
//! [`rank`] orders them and says why the first is best, [`why_ahead`] says
//! why one file is ahead of another, and [`pick`] chooses, for every tag
//! and for the cover art, the value to hand on and the file it came from.
//! A reason is a code with the facts behind it ("a FLAC against your 320
//! kbps MP3" is [`Reason::BetterFormat`] with both formats); its wording
//! arrives with the screen that shows it. Nothing here is wired in yet: the
//! callers (1bC-4, the Review comparison) read the rows into [`FileFacts`].
//!
//! # The order
//!
//! Each step only breaks the ties of the steps before it. A file that is
//! ahead at an earlier step stays ahead whatever it has at the later ones.
//!
//! | Step | Better | Reason |
//! |---|---|---|
//! | 1 on disk | on disk, over gone | `notOnDisk` |
//! | 2 verdict | ok, then not checked, then flagged (low bitrate or suspect transcode), then truncated, then broken | `betterVerdict` |
//! | 3 plays in the DJ app | yes, over no | `playable` |
//! | 4 cutoff | the higher measured cutoff, in 1 kHz steps, all of 19 kHz and up counting as one; measured over not measured | `higherCutoff` |
//! | 5 decode errors | fewer | `fewerDecodeErrors` |
//! | 6 format | lossless over lossy over the rest; inside a class, the order of [`LOSSLESS_ORDER`] or [`LOSSY_ORDER`]; among lossy files the higher header bitrate | `betterFormat` |
//! | 7 tags | more of [`COMPLETENESS_FIELDS`] filled | `moreTags` |
//! | 8 art | has embedded art | `hasArt` |
//! | 9 rekordbox | known by a trusted match, then a probable one, then not known | `knownToRekordbox` |
//! | 10 place | volume connected, scan folder over Inbox, internal over external over network, then the folder added first | `betterPlace` |
//! | 11 tie | the lower file id | `tied` |
//!
//! The order is total: it doesn't depend on the order the files are given
//! in.
//!
//! **Real quality, not the header.** Step 2 is the quality verdict
//! ([`crate::quality::verdict`]), step 4 the measured cutoff. A FLAC made
//! from a 320 kbps MP3 and a real 128 kbps MP3 are both "flagged"; the
//! cutoff, not the header bitrate, then separates them. The header bitrate
//! only breaks ties among lossy files of one format class (step 6), because
//! a lossless file's bitrate says nothing about its quality. A cutoff counts
//! only for a file with a verdict: without one, nothing vouches that it was
//! measured from this audio.
//!
//! **A flagged file never outranks a file that is ok or not checked, as
//! long as both are on disk.** The one exception is step 1: a file that is
//! gone can't be played or linked, so it is behind a flagged file that is
//! there. (An "ok" file on an unplugged drive is on disk: ROADMAP 1.1.)
//!
//! **Two different facts, two names.** [`FileFacts::on_disk`] is whether the
//! file exists as far as the app knows: a file on an unplugged drive is on
//! disk, because unplugged isn't missing (ROADMAP 1.1, 1.8).
//! [`Place::volume_online`] is whether its volume is connected right now.
//! Step 1 reads the first and step 10 the second.
//!
//! **Plays in the DJ app** is the caller's fact: the caller knows which
//! formats the send step can give a `Kind`. This module doesn't name
//! rekordbox's formats.
//!
//! # Picking tags and art
//!
//! Audio and tags are picked separately: the best file decides the audio
//! only. For each tag field ([`TagField`]) the value comes from the
//! best-ranked file that has one. There is no majority vote: two lesser
//! copies that agree don't outvote the best file. Values that differ only in
//! case, spacing or Unicode form are the same value; the spelling taken is
//! the picked file's. Every other file's differing value is returned with
//! the pick, so Review can show the disagreement.
//!
//! BPM, key and energy aren't picked: `analysis` is their source of truth
//! (ROADMAP §2), and the tags' readings of them are only inputs to it. Tag
//! BPM and key don't feed `analysis` today, and aren't counted in tag
//! completeness either.
//!
//! **Art.** The picture with the most pixels wins; where pixels aren't
//! known (or are equal), the larger one in bytes; then the better-ranked
//! file. The tags layer records only an art's size in bytes, so without a
//! hash two pictures are the same picture when their size in bytes is the
//! same, and a different size counts as a disagreement. A hash, when the
//! caller has one, decides instead.
//!
//! # Proposed, for the owner
//!
//! These are one named constant each, and the owner may reorder them:
//! [`LOSSLESS_ORDER`], [`LOSSY_ORDER`], [`COMPLETENESS_FIELDS`], the
//! cutoff steps ([`CUTOFF_STEP_HZ`], [`FULL_BAND_FROM_HZ`]) and the order of
//! the place sub-steps.

mod pick;
mod rank;

#[cfg(test)]
mod tests;

use serde::{Serialize, Serializer};

use crate::quality::verdict::{Assessment, Finding, Verdict};
use crate::read::Codec;
use crate::tags::TagFields;

pub use pick::{
    pick, ArtPick, ArtReason, DifferentArt, Disagreement, FieldPick, FieldValue, PickReason, Picks,
    TagField, COMPLETENESS_FIELDS,
};
pub use rank::{
    rank, why_ahead, Ranked, Ranking, Reason, Step, CUTOFF_STEP_HZ, FULL_BAND_FROM_HZ,
    LOSSLESS_ORDER, LOSSY_ORDER,
};

/// What is known about one file of a track.
#[derive(Debug, Clone, PartialEq)]
pub struct FileFacts {
    /// The file's id; the last tie-break, and how a result names a file.
    pub file_id: i64,
    /// The file exists as far as the app knows: it was there when last
    /// looked for, **or its drive is unplugged** (`file.present`). Not
    /// whether its volume is connected: that is [`Place::volume_online`].
    pub on_disk: bool,
    /// The DJ app can play the file's format (the caller's fact).
    pub plays_in_dj_app: bool,
    pub format: Format,
    pub quality: Quality,
    /// The file's own tags, as its main tag block gives them.
    pub tags: TagFields,
    /// The embedded cover art, if the file has any.
    pub art: Option<Art>,
    pub rekordbox: RekordboxLink,
    pub place: Place,
}

/// A file's codec and its header's average bitrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    /// `None` when the codec isn't known.
    #[serde(serialize_with = "codec_name")]
    pub codec: Option<Codec>,
    /// `file.bitrate`, in kbps.
    pub kbps: Option<u32>,
}

fn codec_name<S: Serializer>(codec: &Option<Codec>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(codec.map_or("unknown", Codec::as_str))
}

/// What the quality check says about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quality {
    /// `None` when the file hasn't been checked.
    pub verdict: Option<Verdict>,
    /// The measured cutoff, in Hz. Counts only with a verdict.
    pub cutoff_hz: Option<u32>,
    /// Packets that failed to decode in a file that otherwise reads to its
    /// end.
    pub decode_errors: u32,
}

impl Quality {
    /// A file that hasn't been checked.
    pub const NOT_CHECKED: Quality = Quality {
        verdict: None,
        cutoff_hz: None,
        decode_errors: 0,
    };

    /// The facts from [`crate::quality::verdict::assess`]'s answer and the
    /// file's measured cutoff.
    pub fn from_assessment(assessment: &Assessment, cutoff_hz: Option<u32>) -> Quality {
        let decode_errors = assessment
            .findings
            .iter()
            .find_map(|finding| match finding {
                Finding::DecodeErrors { count } => Some(*count),
                _ => None,
            })
            .unwrap_or(0);
        Quality {
            verdict: assessment.verdict,
            cutoff_hz,
            decode_errors,
        }
    }
}

/// An embedded picture. Only its size is known to the tags layer today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Art {
    pub bytes: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// A hash of the picture's bytes, if the caller has one: it then
    /// decides whether two pictures are the same.
    #[serde(skip)]
    pub hash: Option<Vec<u8>>,
}

/// Whether rekordbox knows the file. Greater is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RekordboxLink {
    /// Not known to rekordbox.
    Unknown,
    /// A probable match (`relink_probable = 1`).
    Probable,
    /// A trusted match: rekordbox uses this very file.
    Trusted,
}

/// Where a file lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    /// The volume is connected right now. Not the same as the file being on
    /// disk ([`FileFacts::on_disk`]).
    pub volume_online: bool,
    pub folder_role: FolderRole,
    pub volume_kind: VolumeKind,
    /// Which music folder came first: its id, lower first.
    pub folder_order: i64,
}

/// A music folder's role. Greater is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderRole {
    Inbox,
    Scan,
}

/// The kind of volume. Greater is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VolumeKind {
    Network,
    External,
    Internal,
}
