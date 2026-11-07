//! Diagnostics (ROADMAP 4.3): a small structured event log and a private
//! report built from it.
//!
//! Nothing here is telemetry. Nothing is sent, uploaded or opened; the log
//! and the reports are files in the app data folder, written through the
//! write guard, that the user can read and copy themselves.
//!
//! **The log holds an allow-list, never text.** An [`LogEvent`] has a time, a
//! kind from a fixed list, the job kind, a reason code from a fixed list,
//! a count and a duration. There is no field a message, a path, a name or a
//! key could go in, and the types that fill the fields (enums, numbers)
//! can't carry one. The file is read back through the same type, so a line
//! with anything else in it is dropped.
//!
//! The log is a ring: the newest [`MAX_EVENTS`] events, and at most
//! [`MAX_BYTES`] of file. Recording is best-effort. It never fails or waits
//! on the operation it describes: a write that fails is counted and
//! forgotten.
//!
//! Events are recorded at a few central points, not everywhere: the job
//! queue's lifecycle (`jobs::queue`), the rekordbox reads
//! (`rekordbox::source`, `rekordbox_db`), the send steps (`send::job`) and
//! startup (`lib.rs`, `jobs::startup`).

pub mod commands;
mod report;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::jobs::JobKind;
use crate::rekordbox::source::FailureReason;
use crate::rekordbox_db::DbFailure;
use crate::send::SendFailure;
use crate::write_guard::WriteGuard;

pub use report::ReportDraft;

/// The log's file, in the app data folder.
pub const EVENTS_FILE: &str = "events.jsonl";
/// The reports' folder, in the app data folder.
pub const REPORT_DIR: &str = "diagnostics";
/// The most events the ring keeps.
pub const MAX_EVENTS: usize = 2_000;
/// The most bytes the ring's file holds.
pub const MAX_BYTES: usize = 1_000_000;
/// How many reports are kept; saving another deletes the oldest.
pub const KEEP_REPORTS: usize = 5;

/// What happened. A fixed list: new kinds are added here, never built from
/// text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum EventKind {
    JobStarted,
    JobFinished,
    JobFailed,
    JobCancelled,
    RekordboxReadOk,
    RekordboxReadFailed,
    DatabaseReadOk,
    DatabaseReadFailed,
    SendPrepared,
    SendDone,
    SendRefused,
    /// The app started: `ms` is how long setup took, `count` how many jobs
    /// an earlier run left running.
    Startup,
    /// Startup asked the stages what is due: `ms` is how long it took,
    /// `count` how many stages couldn't be asked.
    StartupAsked,
}

/// Why something failed, as a code. A fixed list, so no message text can be
/// stored in its place; each source of failures converts into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum EventReason {
    // A job.
    /// The job's handler returned an error.
    JobFailed,
    /// The job's handler panicked.
    Panicked,
    /// This build has no handler for the job's kind.
    NoHandler,
    /// This build doesn't know the job's kind.
    UnknownKind,
    // A rekordbox read (the export's and the database's).
    NotFound,
    CantRead,
    Damaged,
    NotAnExport,
    TurnedOff,
    NoKey,
    WrongKey,
    NotALibrary,
    Stopped,
    // A send.
    ReadFailed,
    Cancelled,
    NoPreflight,
    ExportChanged,
    LibraryChanged,
    NotSendable,
    NotConfirmed,
    ExportOlderThanLastSend,
    CantWrite,
    CantRecord,
    Internal,
}

impl From<FailureReason> for EventReason {
    fn from(reason: FailureReason) -> EventReason {
        match reason {
            FailureReason::NotFound => EventReason::NotFound,
            FailureReason::CantRead => EventReason::CantRead,
            FailureReason::Damaged => EventReason::Damaged,
            FailureReason::NotAnExport => EventReason::NotAnExport,
        }
    }
}

impl From<DbFailure> for EventReason {
    fn from(reason: DbFailure) -> EventReason {
        match reason {
            DbFailure::TurnedOff => EventReason::TurnedOff,
            DbFailure::NoKey => EventReason::NoKey,
            DbFailure::NotFound => EventReason::NotFound,
            DbFailure::CantRead => EventReason::CantRead,
            DbFailure::WrongKey => EventReason::WrongKey,
            DbFailure::Damaged => EventReason::Damaged,
            DbFailure::NotALibrary => EventReason::NotALibrary,
            DbFailure::Stopped => EventReason::Stopped,
        }
    }
}

impl From<SendFailure> for EventReason {
    fn from(reason: SendFailure) -> EventReason {
        match reason {
            SendFailure::ReadFailed => EventReason::ReadFailed,
            SendFailure::Cancelled => EventReason::Cancelled,
            SendFailure::NoPreflight => EventReason::NoPreflight,
            SendFailure::ExportChanged => EventReason::ExportChanged,
            SendFailure::LibraryChanged => EventReason::LibraryChanged,
            SendFailure::NotSendable => EventReason::NotSendable,
            SendFailure::NotConfirmed => EventReason::NotConfirmed,
            SendFailure::ExportOlderThanLastSend => EventReason::ExportOlderThanLastSend,
            SendFailure::CantWrite => EventReason::CantWrite,
            SendFailure::CantRecord => EventReason::CantRecord,
            SendFailure::Internal => EventReason::Internal,
        }
    }
}

/// One entry of the log. Every field is a number or a fixed list: there is
/// nowhere to put text. Unknown fields in the file are refused when it is
/// read back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogEvent {
    /// When, in milliseconds since the Unix epoch.
    pub at_ms: i64,
    pub kind: EventKind,
    /// The job's kind, for the job and read events.
    pub job: Option<JobKind>,
    pub reason: Option<EventReason>,
    /// How many, where the kind says what.
    pub count: Option<u32>,
    /// How long it took, in milliseconds.
    pub ms: Option<u32>,
}

impl LogEvent {
    pub fn new(kind: EventKind) -> LogEvent {
        LogEvent {
            at_ms: now_ms(),
            kind,
            job: None,
            reason: None,
            count: None,
            ms: None,
        }
    }

    pub fn job(mut self, job: JobKind) -> LogEvent {
        self.job = Some(job);
        self
    }

    pub fn reason(mut self, reason: impl Into<EventReason>) -> LogEvent {
        self.reason = Some(reason.into());
        self
    }

    pub fn count(mut self, count: usize) -> LogEvent {
        self.count = Some(u32::try_from(count).unwrap_or(u32::MAX));
        self
    }

    pub fn took(mut self, took: Duration) -> LogEvent {
        self.ms = Some(u32::try_from(took.as_millis()).unwrap_or(u32::MAX));
        self
    }

    pub fn at(mut self, at_ms: i64) -> LogEvent {
        self.at_ms = at_ms;
        self
    }
}

/// Now, in milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// How big the ring may get.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub events: usize,
    pub bytes: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            events: MAX_EVENTS,
            bytes: MAX_BYTES,
        }
    }
}

struct Ring {
    /// Oldest first, each with the size of its line in the file.
    events: VecDeque<(LogEvent, usize)>,
    bytes: usize,
    /// What the file holds now: lines and bytes. It can be a little more
    /// than the ring (see [`Inner::room`]).
    file_lines: usize,
    file_bytes: usize,
    /// The file must be rewritten from the ring before anything is added to
    /// it: it held lines that are not events or a cut-off tail, was too big
    /// to read, or the last write to it failed.
    rewrite_next: bool,
}

struct Inner {
    guard: WriteGuard,
    path: PathBuf,
    limits: Limits,
    ring: Mutex<Ring>,
    /// Writes that failed and were forgotten.
    lost_writes: AtomicU32,
}

/// The event log. Cheap to clone; every clone is the same ring. Tauri
/// state (`State<EventLog>`).
///
/// **How it is kept.** The ring in memory is always within both limits, and
/// it is what [`Self::events`] shows. The file is added to one line at a
/// time, which is cheap and cannot empty it. Only when the file has grown a
/// tenth past the limits is it rewritten from the ring, to a new file that
/// replaces the old one in one step ([`WriteGuard::write_then_rename`]), so
/// a crash leaves the old file or the new one, never an empty one. A line
/// cut off by a crash is dropped when the file is next read.
#[derive(Clone)]
pub struct EventLog(Option<Arc<Inner>>);

impl EventLog {
    /// A log that records nothing: for queues built without one.
    pub fn off() -> EventLog {
        EventLog(None)
    }

    /// The log in the app data folder, with what an earlier run left in it.
    pub fn open(guard: &WriteGuard) -> EventLog {
        EventLog::open_with(guard, Limits::default())
    }

    pub fn open_with(guard: &WriteGuard, limits: Limits) -> EventLog {
        let path = guard.app_data_dir().join(EVENTS_FILE);
        // A rewrite that a crash cut short leaves a `.part` file beside it.
        let _ = guard.remove_leftover_parts(&path);
        let mut ring = Ring {
            events: VecDeque::new(),
            bytes: 0,
            file_lines: 0,
            file_bytes: 0,
            rewrite_next: false,
        };
        // A file that can't be read, or lines that aren't events, are
        // dropped: the log starts again from what is good. A file far past
        // the limits is not read at all.
        match fs::metadata(&path) {
            Ok(meta) if meta.len() as usize > limits.bytes.saturating_mul(4) => {
                ring.rewrite_next = true;
            }
            Ok(_) => {
                if let Ok(text) = fs::read_to_string(&path) {
                    for line in text.lines() {
                        if let Ok(event) = serde_json::from_str::<LogEvent>(line) {
                            push(&mut ring, event, &limits);
                        }
                    }
                    ring.file_lines = text.lines().count();
                    ring.file_bytes = text.len();
                    // Not exactly the ring: junk, a cut-off tail, or too much.
                    ring.rewrite_next = text.len() != ring.bytes;
                } else {
                    ring.rewrite_next = true;
                }
            }
            Err(_) => {}
        }
        EventLog(Some(Arc::new(Inner {
            guard: guard.clone(),
            path,
            limits,
            ring: Mutex::new(ring),
            lost_writes: AtomicU32::new(0),
        })))
    }

    /// Adds `event` to the ring and the file. Best-effort: nothing here can
    /// fail the caller. A write that fails is counted ([`Self::lost_writes`])
    /// and the file is rewritten whole at the next event.
    pub fn record(&self, event: LogEvent) {
        let Some(inner) = &self.0 else { return };
        let mut ring = inner.lock();
        let line = push(&mut ring, event, &inner.limits);
        let (room_lines, room_bytes) = inner.room();
        let rewrite = ring.rewrite_next
            || ring.file_lines + 1 > inner.limits.events + room_lines
            || ring.file_bytes + line.len() > inner.limits.bytes + room_bytes;
        let written = if rewrite {
            let mut text = String::with_capacity(ring.bytes);
            for (event, _) in &ring.events {
                text.push_str(&serde_json::to_string(event).unwrap_or_default());
                text.push('\n');
            }
            inner.guard.write_then_rename(&inner.path, text.as_bytes())
        } else {
            inner.guard.append(&inner.path, line.as_bytes())
        };
        match written {
            Ok(()) if rewrite => {
                ring.file_lines = ring.events.len();
                ring.file_bytes = ring.bytes;
                ring.rewrite_next = false;
            }
            Ok(()) => {
                ring.file_lines += 1;
                ring.file_bytes += line.len();
            }
            Err(_) => {
                inner.lost_writes.fetch_add(1, Ordering::Relaxed);
                ring.rewrite_next = true;
            }
        }
    }

    /// The events, oldest first.
    pub fn events(&self) -> Vec<LogEvent> {
        let Some(inner) = &self.0 else {
            return Vec::new();
        };
        inner.lock().events.iter().map(|(e, _)| e.clone()).collect()
    }

    /// How many writes failed since the app started.
    pub fn lost_writes(&self) -> u32 {
        self.0
            .as_ref()
            .map_or(0, |i| i.lost_writes.load(Ordering::Relaxed))
    }
}

impl Inner {
    /// How far past the limits the file may grow before it is rewritten: a
    /// tenth, so a full ring is rewritten once per tenth of its size in
    /// events, not at every event.
    fn room(&self) -> (usize, usize) {
        (
            (self.limits.events / 10).max(1),
            (self.limits.bytes / 10).max(1),
        )
    }

    fn lock(&self) -> MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Adds `event` to the ring and drops the oldest until both bounds hold.
/// Returns the event's line for the file, newline included.
fn push(ring: &mut Ring, event: LogEvent, limits: &Limits) -> String {
    let mut line = serde_json::to_string(&event).unwrap_or_default();
    line.push('\n');
    let size = line.len();
    ring.bytes += size;
    ring.events.push_back((event, size));
    while ring.events.len() > limits.events || ring.bytes > limits.bytes {
        match ring.events.pop_front() {
            Some((_, size)) => ring.bytes -= size,
            None => break,
        }
    }
    line
}
