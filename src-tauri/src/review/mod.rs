//! The Review queue and the comparison of one item (1bD-1, 1bD-2; ROADMAP
//! 1.12). Read-only: nothing here changes the database, a file or rekordbox.
//! The actions (accept, keep both, merge, split, dismiss, confirm a relink)
//! are the 1bE tasks.
//!
//! # What is queued
//!
//! Only Library tracks are queued ("being added" has no data of its own:
//! adding a track to the Library is instant), plus relink matches to confirm
//! (every probable match, Library or not; owner, 2026-10-12). Seven kinds of
//! item, in three groups ([`ReviewKind::group`] is the one place that says
//! which group a kind is in):
//!
//! | Kind | Defined by |
//! |---|---|
//! | `versionDoubt` | `grouping_pair.state = 'open'` between two tracks, one in the Library |
//! | `heldSet` | `state = 'set_disagrees'`: tracks joined by such pairs, one in the Library |
//! | `sameRecording` | `state = 'both_in_library'`: two Library tracks that are the same recording (a set that also holds a third copy is the same item) |
//! | `relink` | `rekordbox_track` with `relink_probable = 1` and a file |
//! | `clearBestFile` | a Library track with a `better_file` whose step is `onDisk`: its linked file is gone and a copy is on disk |
//! | `qualityProblem` | a Library track whose linked file is on disk and whose current stored verdict flags it, with no better copy |
//! | `betterFile` | a Library track with a `better_file` whose step is anything else: its linked file is still there |
//!
//! One item is one pair of tracks however many pairs of their files the
//! grouping stored (ROADMAP 1.4: the user is asked once per pair of tracks).
//! A held set or a same-recording set is one item for the whole set.
//!
//! # Cost
//!
//! Counts and pages are aggregate SQL on the read pool, over the few rows
//! that wait (partial indexes, migration 0026). The one thing done in Rust
//! is joining the tracks of held sets ([`held`]): it reads the held pairs
//! only, a pair of integers each. Their number is bounded by the pairs the
//! grouping stored in the states `set_disagrees` and `both_in_library`, which
//! is at most one per Library duplicate (about 50,000 for a library of
//! 100,000 files that holds every track twice).
//!
//! # Order
//!
//! Inside a group the kinds come in [`ReviewKind::ALL`] order, and inside a
//! kind by track id (the oldest first), the same however the rows are read.
//! The shown title isn't stored, so a title order would mean computing every
//! title.

pub mod compare;
pub mod held;
mod queue;
pub mod reasons;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::quality::verdict::Verdict;
use crate::scan::system_volumes;
use crate::suggest::{Reason, Suggestion};

pub use compare::comparison;
pub use queue::{counts, items};

/// The grouping pairs that wait for the user. The text of migration 0026's
/// index `grouping_pair_pending`, and of every read-model query that wants
/// the index: change both together (a test holds them to it).
pub(crate) const PENDING_PAIRS: &str = "state IN ('open', 'set_disagrees', 'both_in_library')";

/// The stored verdicts that flag a file. The text of `file_verdict_flagged`.
pub(crate) const FLAGGED: &str =
    "verdict IN ('low_bitrate', 'suspect_transcode', 'truncated', 'broken')";

/// A rekordbox entry whose file is only a probable match. The text of
/// `rekordbox_track_probable`.
pub(crate) const PROBABLE_RELINK: &str = "relink_probable = 1 AND file_id IS NOT NULL";

/// How many files a comparison lists at most.
pub const MAX_FILES: usize = 50;

/// How many track names an item lists in the queue.
pub const MAX_NAMES: usize = 3;

/// The queue's groups, in the order they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReviewGroup {
    NeedsCloserLook,
    ClearBestFile,
    QualityProblems,
}

impl ReviewGroup {
    /// Every group, in the order shown.
    pub const ALL: [ReviewGroup; 3] = [
        ReviewGroup::NeedsCloserLook,
        ReviewGroup::ClearBestFile,
        ReviewGroup::QualityProblems,
    ];

    /// The kinds of this group, in the order they are listed.
    pub fn kinds(self) -> impl Iterator<Item = ReviewKind> {
        ReviewKind::ALL.into_iter().filter(move |k| k.group() == self)
    }
}

/// What kind of decision an item asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReviewKind {
    VersionDoubt,
    HeldSet,
    SameRecording,
    Relink,
    ClearBestFile,
    QualityProblem,
    BetterFile,
}

impl ReviewKind {
    /// Every kind, in the order a group lists them.
    pub const ALL: [ReviewKind; 7] = [
        ReviewKind::VersionDoubt,
        ReviewKind::HeldSet,
        ReviewKind::SameRecording,
        ReviewKind::Relink,
        ReviewKind::ClearBestFile,
        ReviewKind::QualityProblem,
        ReviewKind::BetterFile,
    ];

    /// The group a kind is shown in. The one place to change it.
    pub fn group(self) -> ReviewGroup {
        match self {
            ReviewKind::VersionDoubt
            | ReviewKind::HeldSet
            | ReviewKind::SameRecording
            | ReviewKind::Relink => ReviewGroup::NeedsCloserLook,
            ReviewKind::ClearBestFile => ReviewGroup::ClearBestFile,
            // A "Better file available" item whose linked file is still
            // there goes with the quality problems (owner, 2026-10-12).
            ReviewKind::QualityProblem | ReviewKind::BetterFile => ReviewGroup::QualityProblems,
        }
    }
}

/// Which item. A pair item names its two tracks (`a` < `b`); a held or
/// same-recording set names its lowest track (`b` is `None`); a relink item
/// names the rekordbox entry; the others name the Library track's track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewItemId {
    pub kind: ReviewKind,
    pub a: i64,
    pub b: Option<i64>,
}

/// How many items wait, by group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCounts {
    pub needs_closer_look: u32,
    pub clear_best_file: u32,
    pub quality_problems: u32,
    /// How many need a decision in all: the three above added up. What the
    /// Overview and the sidebar can show.
    pub total: u32,
}

/// A track named for the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTrackName {
    /// `None` for rekordbox's own entry (a relink item).
    pub recording_id: Option<i64>,
    pub title: Option<String>,
    pub artist: Option<String>,
}

/// One line of the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewItem {
    pub id: ReviewItemId,
    pub kind: ReviewKind,
    /// The first [`MAX_NAMES`] tracks.
    pub tracks: Vec<ReviewTrackName>,
    /// How many more tracks the item has.
    pub more_tracks: u32,
}

/// A page of one group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPage {
    pub group: ReviewGroup,
    /// How many items the group has in all.
    pub total: u32,
    pub items: Vec<ReviewItem>,
}

/// Whether rekordbox knows a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RekordboxUse {
    Unknown,
    Probable,
    Trusted,
}

/// What a comparison row stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RowSource {
    File,
    /// rekordbox's own entry (a relink item): the file it names is gone.
    RekordboxEntry,
}

/// One row of a comparison: a file, with the same columns for every file.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileRow {
    pub file_id: Option<i64>,
    pub source: RowSource,
    /// The track the file is in; `None` for rekordbox's entry.
    pub recording_id: Option<i64>,
    pub name: String,
    pub path: Option<String>,
    /// The last scan found it on disk (a file on an unplugged drive is).
    pub on_disk: bool,
    pub drive_connected: bool,
    /// The Library track plays this file.
    pub linked: bool,
    /// The format sniffed from the bytes, e.g. `mp3`, `flac`.
    pub format: Option<String>,
    /// `None`: not checked.
    pub verdict: Option<Verdict>,
    pub cutoff_hz: Option<u32>,
    pub duration_ms: Option<u32>,
    pub tags_filled: u32,
    pub tags_total: u32,
    pub has_art: bool,
    pub rekordbox: RekordboxUse,
    // The "More details" columns (1bE-1).
    pub bitrate_kbps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub added_at: Option<String>,
}

/// The comparison for one item: its files, the suggested one first.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComparison {
    pub id: ReviewItemId,
    pub kind: ReviewKind,
    pub group: ReviewGroup,
    pub tracks: Vec<ReviewTrackName>,
    /// Why the item is in the queue, from the grouping's own reasons (a
    /// version doubt, a held set): at most three, the deciding one first.
    pub doubt: Vec<Reason>,
    /// The file the app suggests (`what` is its id) and why. `None` when
    /// there is nothing to suggest (a quality problem with no better copy).
    pub suggestion: Option<Suggestion<i64>>,
    /// The first [`MAX_FILES`] files, the suggested one first.
    pub rows: Vec<ReviewFileRow>,
    pub more_files: u32,
}

/// How many items wait, by group.
#[tauri::command]
#[specta::specta]
pub async fn review_counts(reads: State<'_, ReadPool>) -> Result<ReviewCounts, IpcError> {
    Ok(reads.read(counts)?)
}

/// A page of one group's items, the oldest first.
#[tauri::command]
#[specta::specta]
pub async fn review_items(
    reads: State<'_, ReadPool>,
    group: ReviewGroup,
    offset: u32,
    limit: u32,
) -> Result<ReviewPage, IpcError> {
    let volumes = system_volumes();
    Ok(reads.read(|conn| items(conn, &volumes, group, offset, limit))?)
}

/// The comparison for one item, or `None` if it is no longer in the queue.
#[tauri::command]
#[specta::specta]
pub async fn review_item(
    reads: State<'_, ReadPool>,
    id: ReviewItemId,
) -> Result<Option<ReviewComparison>, IpcError> {
    let volumes = system_volumes();
    Ok(reads.read(|conn| comparison(conn, &volumes, id))?)
}
