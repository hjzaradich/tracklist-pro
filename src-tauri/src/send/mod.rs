//! The send flow: Library → rekordbox XML, as a guided checklist (1aF-1;
//! ROADMAP 1.9, §5.2, §6).
//!
//! A send has two steps, each a job ([`job`]):
//!
//! 1. **Prepare.** rekordbox's export is read again, always (a send never
//!    uses an old read), its tracks are matched to files, and the
//!    [`Preflight`] is built: what would be sent, what's left out and why,
//!    and anything that needs the user's say. Nothing is written but the
//!    database's own snapshot of the read.
//! 2. **Write**, on the user's go. The send is built again and must be,
//!    byte for byte, the one the preflight described, from the same read of
//!    an export that hasn't changed since; then the file is written
//!    ([`file`]) and the send recorded (rule 8). Anything else is refused
//!    and changes nothing. A preflight is good for one send.
//!
//! A send job never outlives the run of the app that queued it: a write
//! only ever follows a click in this run, and the preflight it needs is
//! in memory. Each job carries the run it was queued in
//! ([`SendFlow::prepare_job`]) and the handler does nothing for a job of
//! another run, whenever it meets one; [`drop_unfinished_jobs`] also ends
//! such jobs at startup, so they don't linger in the queue.
//!
//! The file is one fixed file in the app data folder ([`file::send_path`]),
//! replaced on every send: the app changes no bytes anywhere else (§6).
//!
//! The values come from [`crate::send_values`] and the file's rules from
//! [`crate::rekordbox_write`]; nothing here decides what a track's values
//! are. A send isn't an operation in the undo log (see `record_send`).

pub mod file;
pub mod job;
mod preflight;
mod tree;

use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::ReadPool;
use crate::ipc::{ErrorKind, IpcError};
use crate::jobs::{JobId, JobQueue};
use crate::library::LibraryTrackId;
use crate::rekordbox::source::stored_source;
use crate::write_guard::WriteGuard;

pub use job::{drop_unfinished_jobs, sender, Sender};
pub use preflight::{review, Reviewed};
pub use tree::crate_tree;

/// A Library track, as the preflight's lists name it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrackLabel {
    pub library_track: LibraryTrackId,
    /// `None` when the track has none; the list then shows the file's name.
    pub title: Option<String>,
    pub artist: Option<String>,
    /// The linked file's name, e.g. `a.mp3`.
    pub file_name: Option<String>,
}

/// Why a Library track is left out of a send. A code; the wording is in
/// the locale file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LeftOutReason {
    /// It has no linked file.
    NoFile,
    /// Its linked file isn't on disk.
    FileMissing,
    /// Its file's path can't be worked out (its drive or folder doesn't
    /// read back).
    NoPath,
    /// Its path can't be written as a rekordbox `Location`.
    PathNotSendable,
    /// One of its values holds a character XML can't carry
    /// ([`LeftOut::attribute`] names the field).
    UnsendableCharacter,
    /// An earlier track of this send is the same file.
    SameFileAsAnother,
    /// rekordbox's own entry for it can't be sent back as it is (its
    /// `TrackID` or an attribute is unusable or repeated).
    RekordboxEntry,
}

/// A Library track a send leaves out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LeftOut {
    pub track: TrackLabel,
    pub reason: LeftOutReason,
    /// The rekordbox field at fault, where the reason names one.
    pub attribute: Option<String>,
}

/// A Library track whose linked file rekordbox doesn't have, while it
/// already has another file of the same track: sending adds a second
/// rekordbox entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherFile {
    pub track: TrackLabel,
    /// The Library track's file, which the send adds.
    pub library_file: Option<String>,
    /// The file rekordbox already has.
    pub rekordbox_file: Option<String>,
}

/// Whether a node is under `Crates` or `Playlists`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TreeKind {
    Crate,
    Playlist,
}

/// A crate or playlist that will arrive in rekordbox with entries left out
/// (their tracks aren't sent). Importing it replaces rekordbox's playlist
/// of that name, so those entries are lost there (§5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LosesEntries {
    pub kind: TreeKind,
    /// The folder names down to it, below `Crates` or `Playlists`.
    pub path: Vec<String>,
    /// How many of its entries are left out.
    pub lost: u32,
    /// How many entries it has in the app.
    pub entries: u32,
}

/// Why the whole send is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RefusalReason {
    /// The export just read doesn't hold every track it says it does, so
    /// some of rekordbox's values would come from an older read.
    IncompleteExport,
    /// Two crates, playlists or folders in one folder share a name (or
    /// names rekordbox may treat as one): one would replace the other.
    SameName,
    /// A crate, playlist or folder has no name.
    NoName,
    /// A name holds a character XML can't carry.
    UnsendableName,
    /// Folders nest too deep.
    TooDeep,
    /// The file couldn't be made (a bug; nothing is written).
    Internal,
}

/// A refused send, explained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub reason: RefusalReason,
    /// Where in the tree the problem is, from `Crates` or `Playlists`
    /// down; empty when the reason isn't about the tree.
    pub path: Vec<String>,
}

/// The export a preflight was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportRead {
    pub path: String,
    /// When rekordbox saved it: the file's modified time, in milliseconds
    /// since the Unix epoch.
    pub modified_ms: i64,
    /// When the app read it (UTC, ISO 8601).
    pub read_at: String,
    /// Tracks in the export that couldn't be read. Above 0, the send
    /// needs an explicit confirm ([`Preflight::needs_confirm`]).
    pub not_stored: u32,
}

/// What a send would do, worked out without writing anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    /// Names this preflight; the write must be given it back.
    pub token: String,
    pub export: ExportRead,
    /// Tracks rekordbox doesn't have yet. No dialog for these.
    pub new_tracks: u32,
    /// Tracks rekordbox already has: one Yes/No dialog each on import.
    pub known_tracks: u32,
    pub left_out: Vec<LeftOut>,
    pub other_file: Vec<OtherFile>,
    /// Not empty: the send needs an explicit confirm
    /// ([`Preflight::needs_confirm`] says when it's needed).
    pub loses_entries: Vec<LosesEntries>,
    /// Set when the send can't go at all.
    pub refusal: Option<Refusal>,
    /// No track would be sent and there's no crate or playlist: the send
    /// can't be started.
    pub nothing_to_send: bool,
    /// Whether the user's go can start the write: the send isn't refused
    /// and there's something to send.
    pub can_send: bool,
    /// Whether the go needs the explicit confirm: a crate or playlist
    /// loses entries, or the read couldn't store some of rekordbox's
    /// tracks (a Library track on such a track's file would be sent as
    /// new, and a Yes in rekordbox would overwrite that entry).
    pub needs_confirm: bool,
}

/// Why a step of the send stopped. Nothing was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SendFailure {
    /// The export couldn't be read; the rekordbox source says why.
    ReadFailed,
    /// The step was cancelled.
    Cancelled,
    /// There's no preflight to send, or not the one named.
    NoPreflight,
    /// The export changed, or was read again, after the preflight.
    ExportChanged,
    /// The Library changed after the preflight: the send would differ
    /// from what was reviewed.
    LibraryChanged,
    /// The preflight says the send is refused, or has nothing to send.
    NotSendable,
    /// The send needs the explicit confirm and it wasn't given.
    NotConfirmed,
    /// The file couldn't be written. The file from the last send is as it
    /// was.
    CantWrite,
    /// The file was written but the send couldn't be recorded. The file
    /// from the last send is put back when there was one and it can be;
    /// otherwise the new file stays, unrecorded, until the next send
    /// replaces it.
    CantRecord,
    /// The database failed, or a bug.
    Internal,
}

/// One of the send's two steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SendStep {
    Prepare,
    Write,
}

/// A send that went through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    /// When it was recorded (UTC, ISO 8601).
    pub at: String,
    pub new_tracks: u32,
    /// How many Yes/No dialogs to expect in rekordbox.
    pub known_tracks: u32,
}

/// Everything the checklist shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendState {
    /// Goes up each time a step ends, however it ends.
    pub revision: u32,
    /// The one file a send writes, replaced every time.
    pub file_path: String,
    /// The export that will be read, if one is chosen.
    pub export_path: Option<String>,
    /// The preflight waiting for the go.
    pub preflight: Option<Preflight>,
    /// The step that ended last, in this run of the app.
    pub step: Option<SendStep>,
    /// Why the last step stopped, until the next step starts.
    pub failure: Option<SendFailure>,
    /// The last send of this run of the app, until the next read starts.
    pub sent: Option<Sent>,
}

#[derive(Default)]
struct Steps {
    revision: u32,
    step: Option<SendStep>,
    review: Option<Preflight>,
    failure: Option<SendFailure>,
    sent: Option<Sent>,
}

/// Where the send is, kept in memory for this run of the app (in Tauri's
/// state): the preflight waiting for the go, and how the last step ended.
/// Cheap to clone.
#[derive(Clone)]
pub struct SendFlow {
    guard: WriteGuard,
    /// Names this run of the app. Every send job carries it.
    run: Arc<str>,
    steps: Arc<Mutex<Steps>>,
    /// Held by a step while it runs, so two never overlap.
    running: Arc<Mutex<()>>,
}

impl SendFlow {
    pub fn new(guard: WriteGuard) -> SendFlow {
        // Unlike any earlier run's, and any other flow's in this process.
        static FLOWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let run = format!(
            "{}-{started}-{}",
            std::process::id(),
            FLOWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        SendFlow {
            guard,
            run: run.into(),
            steps: Arc::default(),
            running: Arc::default(),
        }
    }

    pub fn guard(&self) -> &WriteGuard {
        &self.guard
    }

    fn steps(&self) -> MutexGuard<'_, Steps> {
        self.steps.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The preflight waiting for the go.
    pub fn preflight(&self) -> Option<Preflight> {
        self.steps().review.clone()
    }

    pub fn failure(&self) -> Option<SendFailure> {
        self.steps().failure
    }

    pub fn sent(&self) -> Option<Sent> {
        self.steps().sent.clone()
    }

    /// Forgets the preflight and how the last step ended: a new step is
    /// starting, and nothing made before it may be sent.
    fn start_over(&self) {
        let mut steps = self.steps();
        steps.review = None;
        steps.failure = None;
        steps.sent = None;
    }

    /// Forgets why the last step stopped, keeping the preflight and the
    /// last send: the write is starting.
    fn start_write(&self) {
        self.steps().failure = None;
    }

    /// Records how a step ended.
    fn end(&self, step: SendStep, change: impl FnOnce(&mut Steps)) {
        let mut steps = self.steps();
        change(&mut steps);
        steps.step = Some(step);
        steps.revision = steps.revision.wrapping_add(1);
    }

    fn state(&self, export_path: Option<String>) -> SendState {
        let steps = self.steps();
        SendState {
            revision: steps.revision,
            file_path: crate::scan::display_path(&file::send_path(&self.guard)),
            export_path,
            preflight: steps.review.clone(),
            step: steps.step,
            failure: steps.failure,
            sent: steps.sent.clone(),
        }
    }
}

/// Where the send is: the file's path, the preflight waiting for the go,
/// and how the last step ended.
#[tauri::command]
#[specta::specta]
pub async fn send_state(
    reads: State<'_, ReadPool>,
    flow: State<'_, SendFlow>,
) -> Result<SendState, IpcError> {
    let export = reads.read(stored_source)?.path;
    Ok(flow.state(export))
}

/// Starts a send: reads rekordbox's export again and builds the preflight,
/// as a job. With a `path`, that file is read and becomes the chosen
/// export; without one, the chosen export is read. Any earlier preflight
/// is dropped at once. Returns the job's id.
#[tauri::command]
#[specta::specta]
pub async fn prepare_send(
    reads: State<'_, ReadPool>,
    jobs: State<'_, JobQueue>,
    flow: State<'_, SendFlow>,
    path: Option<String>,
) -> Result<JobId, IpcError> {
    let path = match path {
        Some(path) => path,
        None => reads
            .read(stored_source)?
            .path
            .ok_or_else(|| IpcError::new(ErrorKind::NoRekordboxXml))?,
    };
    flow.start_over();
    Ok(jobs.enqueue(flow.prepare_job(&path))?)
}

/// The user's go: writes the send the preflight `token` names and records
/// it, as a job. `confirmed` is the explicit confirm
/// ([`Preflight::needs_confirm`] says when it's needed). Returns the job's id; the job refuses
/// anything that isn't exactly the reviewed send.
#[tauri::command]
#[specta::specta]
pub async fn write_send(
    jobs: State<'_, JobQueue>,
    flow: State<'_, SendFlow>,
    token: String,
    confirmed: bool,
) -> Result<JobId, IpcError> {
    flow.start_write();
    Ok(jobs.enqueue(flow.write_job(&token, confirmed))?)
}

#[cfg(test)]
mod tests;
