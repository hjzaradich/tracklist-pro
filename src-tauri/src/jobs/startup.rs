//! At startup, ask each stage whether anything is due (1cA-13).
//!
//! A job an earlier run left running is put back in the queue
//! ([`super::store::recover_interrupted`]), and a queued one just runs. That
//! covers a run that died inside a job. It doesn't cover work that is due
//! while **no job is queued**: the last run's job failed or was cancelled
//! before it chained on, or an upgrade changed a stage's version so rows are
//! due again. Nothing asks for that work until a file changes, so it would
//! wait for ever. [`start`] asks, once, a moment after the queue starts.
//!
//! - **It queues jobs, it does no work,** and it runs on a thread of its own:
//!   the window doesn't wait for it. Each check is one `EXISTS` query that
//!   stops at the first due row; when nothing is due it looks at each file
//!   once, so a library with nothing due costs a few passes over the file
//!   table.
//! - **Every check runs on a read connection** (the read pool), never on the
//!   database writer, so a user's first write doesn't wait behind them. The
//!   writer is taken only for the short statements of a stage that has work:
//!   the lookup of a queued or running twin and the insert of the job
//!   (`queue_once`), and, once after an upgrade that changed the roles'
//!   version, the file roles' one statement that makes every track due.
//! - **One list of "ask this stage" calls** ([`ASKS`]), one line per stage,
//!   in the scan chain's order (file roles, the search index and the waveforms last). Each queues through
//!   `queue_once`, so it never queues what is already queued or running,
//!   and each leaves a stage alone while an earlier one is still due (the
//!   earlier stage's job chains on to it, as in the scan chain).
//! - **Nothing runs against a drive that isn't plugged in** that the normal
//!   chain wouldn't: what is due is judged on the music folders reachable
//!   now, as a catch-up scan of them would, and the quality check is given
//!   the same view.
//! - **Reading and walking are not asked for here.** The catch-up scan every
//!   online music folder gets at startup (`scan::watch`) walks the folder and
//!   queues the read of what changed. A walk with nothing to read does not
//!   go on to the hashes or the fingerprints, which is why they are in the
//!   list.
//! - **Send jobs are not touched.** Nothing here queues an export job, and a
//!   send job an earlier run left is ended before this runs
//!   ([`crate::send::drop_unfinished_jobs`]).
//! - A library with no music folders queues nothing, and a check that fails
//!   is logged and the rest are still asked: startup never fails over it.
//!
//! **Known limit:** relink and attach are not asked for. Their requests
//! queue a job whenever there is rekordbox data, whether or not anything is
//! due, so asking at every start would run both every time. An app that
//! dies after a rekordbox read ends but before the relink or attach job it
//! asks for is queued waits for the next rekordbox read.

use std::cell::OnceCell;
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use tauri::{AppHandle, Manager, Runtime};

use super::{JobId, JobQueue, NewJob};
use crate::db::{DbError, ReadPool, Writer};
use crate::scan::chain::{any_to_try, queue_once, reach_by, Reach};
use crate::scan::folders::{self, MusicFolderId};
use crate::scan::ReadGate;
use crate::scan_state::{Scope, Stage};

/// How a job is queued: the queue's `enqueue`, or a stand-in in tests.
pub(crate) type Enqueue<'a> = &'a dyn Fn(NewJob) -> Result<JobId, DbError>;

/// One stage's question. Each is one line of [`ASKS`].
type Ask = fn(&Asking) -> Result<(), DbError>;

/// Every stage asked at startup, in the scan chain's order. To ask another
/// stage, add a line: check on a read connection (`Asking::read`), then
/// queue the stage's job with `Asking::queue` if it is due.
const ASKS: &[(&str, Ask)] = &[
    ("hash", ask_hash),
    ("group", ask_group),
    ("fingerprint", ask_fingerprint),
    ("quality", ask_quality),
    ("matching", ask_matching),
    ("decide", ask_decide),
    ("file roles", ask_file_roles),
    ("search index", ask_search_index),
    ("waveforms", ask_waveforms),
];

/// What the asks share.
pub(crate) struct Asking<'a> {
    writer: &'a Writer,
    reads: &'a ReadPool,
    enqueue: Enqueue<'a>,
    /// The music folders that can be reached now.
    reachable: Vec<i64>,
    /// Called inside every check, on its read connection, before the check
    /// runs. Tests hold a check open here.
    in_check: &'a dyn Fn(),
    read_due: OnceCell<bool>,
    hash_due: OnceCell<bool>,
    fingerprint_due: OnceCell<bool>,
}

impl Asking<'_> {
    /// Runs a check on a read connection. Never takes the writer.
    fn read<T>(
        &self,
        check: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, DbError> {
        self.reads.read(|c| {
            (self.in_check)();
            check(c)
        })
    }

    /// Whether `stage` has a file worth trying in the reachable folders, as
    /// a scan of them would judge it (not a watcher's rescan).
    fn due(&self, stage: Stage, version: i64) -> Result<bool, DbError> {
        if self.reachable.is_empty() {
            return Ok(false);
        }
        let scope = Scope::Folders(self.reachable.clone());
        self.read(|c| {
            let gate = ReadGate::for_job(c)?;
            any_to_try(c, stage, version, &scope, gate, false)
        })
    }

    fn read_due(&self) -> Result<bool, DbError> {
        remembered(&self.read_due, || {
            self.due(Stage::Read, crate::read::READ_VERSION)
        })
    }

    fn hash_due(&self) -> Result<bool, DbError> {
        remembered(&self.hash_due, || {
            self.due(Stage::Hash, i64::from(crate::hash::DEFINITION))
        })
    }

    fn fingerprint_due(&self) -> Result<bool, DbError> {
        remembered(&self.fingerprint_due, || {
            self.due(Stage::Fingerprint, i64::from(crate::fingerprint::VERSION))
        })
    }

    /// Whether the files still have to be read or hashed: the grouping waits.
    fn hashes_owed(&self) -> Result<bool, DbError> {
        Ok(self.read_due()? || self.hash_due()?)
    }

    /// Whether any of the stages before the comparisons still has files:
    /// quality, matching and deciding wait for the fingerprint job's chain.
    fn fingerprints_owed(&self) -> Result<bool, DbError> {
        Ok(self.hashes_owed()? || self.fingerprint_due()?)
    }

    /// Queues `job` unless a twin is queued or running (the chain's own
    /// dedup). The only part of an ask that takes the writer.
    fn queue(&self, job: NewJob) -> Result<(), DbError> {
        queue_once(self.writer, job, |j| (self.enqueue)(j)).map(|_| ())
    }
}

/// `cell`'s answer, asked for once.
fn remembered(
    cell: &OnceCell<bool>,
    ask: impl FnOnce() -> Result<bool, DbError>,
) -> Result<bool, DbError> {
    if let Some(answer) = cell.get() {
        return Ok(*answer);
    }
    let answer = ask()?;
    Ok(*cell.get_or_init(|| answer))
}

/// The read comes first: a walk queues it, and it chains on to the hashes.
fn ask_hash(a: &Asking) -> Result<(), DbError> {
    if !a.read_due()? && a.hash_due()? {
        let ids = a.reachable.iter().map(|&id| MusicFolderId(id)).collect();
        a.queue(crate::hash::hash_job(Some(ids)))?;
    }
    Ok(())
}

fn ask_group(a: &Asking) -> Result<(), DbError> {
    if a.hashes_owed()? {
        return Ok(());
    }
    if a.read(crate::grouping::any_ungrouped)? {
        a.queue(crate::grouping::group_job())?;
    }
    Ok(())
}

fn ask_fingerprint(a: &Asking) -> Result<(), DbError> {
    if !a.hashes_owed()? && a.fingerprint_due()? {
        a.queue(crate::fingerprint::fingerprint_job(None))?;
    }
    Ok(())
}

fn ask_quality(a: &Asking) -> Result<(), DbError> {
    if a.fingerprints_owed()? {
        return Ok(());
    }
    if a.read(|c| crate::quality::due(c, Some(&a.reachable)))? {
        a.queue(crate::quality::quality_job(None))?;
    }
    Ok(())
}

fn ask_matching(a: &Asking) -> Result<(), DbError> {
    if a.fingerprints_owed()? {
        return Ok(());
    }
    if a.read(crate::matching::any_due)? {
        a.queue(crate::matching::matching_job())?;
    }
    Ok(())
}

fn ask_decide(a: &Asking) -> Result<(), DbError> {
    if a.fingerprints_owed()? {
        return Ok(());
    }
    if a.read(crate::grouping::decide_due)? {
        a.queue(crate::grouping::decide_job())?;
    }
    Ok(())
}

/// The roles are of another version after an upgrade: that makes every
/// track due, one statement on the writer, once. Whether anything is due is
/// read on a read connection. The job is the lowest priority, so it needs no
/// waiting for the stages before it.
fn ask_file_roles(a: &Asking) -> Result<(), DbError> {
    if !a.read(crate::file_roles::version_is_current)? {
        a.writer.call(|c| crate::file_roles::ensure_version(c))?;
    }
    if a.read(crate::file_roles::any_due)? {
        a.queue(crate::file_roles::roles_job())?;
    }
    Ok(())
}

/// The search index is behind when a track is in `search_due`. Like the
/// roles, its job needs no waiting for the stages before it.
fn ask_search_index(a: &Asking) -> Result<(), DbError> {
    if a.read(crate::search::pending)? {
        a.queue(crate::search::index_job(false))?;
    }
    Ok(())
}

/// The waveforms follow the fingerprints, as the quality check does, and are
/// the lowest-priority job there is (1cB-9). The check is one query that
/// stops at the first file with no waveform of its current audio.
fn ask_waveforms(a: &Asking) -> Result<(), DbError> {
    if a.fingerprints_owed()? {
        return Ok(());
    }
    if a.read(|c| crate::waveform::due(c, Some(&a.reachable)))? {
        a.queue(crate::waveform::waveform_job())?;
    }
    Ok(())
}

/// What [`ask_what_is_due`] did.
#[derive(Debug, Default)]
pub(crate) struct Asked {
    /// How many asks failed.
    pub failed: usize,
    /// How long each ask took, in the order asked.
    pub took: Vec<(&'static str, Duration)>,
}

/// Asks every stage in [`ASKS`] whether anything is due and queues what is.
/// Each failed ask is logged and counted; the others are still asked. With
/// no music folder, nothing is asked.
pub(crate) fn ask_what_is_due(
    writer: &Writer,
    reads: &ReadPool,
    enqueue: Enqueue<'_>,
    reach: &Reach,
) -> Asked {
    ask_with(writer, reads, enqueue, reach, &|| {})
}

/// [`ask_what_is_due`], calling `in_check` inside every check (on its read
/// connection), for tests that hold a check open.
pub(crate) fn ask_with(
    writer: &Writer,
    reads: &ReadPool,
    enqueue: Enqueue<'_>,
    reach: &Reach,
    in_check: &dyn Fn(),
) -> Asked {
    let mut asked = Asked::default();
    let folders = match reads.read(folders::stored) {
        Ok(folders) => folders,
        Err(e) => {
            eprintln!("startup: couldn't read the music folders, so nothing was asked for: {e}");
            asked.failed = 1;
            return asked;
        }
    };
    if folders.is_empty() {
        return asked;
    }
    let asking = Asking {
        writer,
        reads,
        enqueue,
        reachable: reach(&folders),
        in_check,
        read_due: OnceCell::new(),
        hash_due: OnceCell::new(),
        fingerprint_due: OnceCell::new(),
    };
    for (stage, ask) in ASKS {
        let started = Instant::now();
        if let Err(e) = ask(&asking) {
            eprintln!("startup: couldn't ask whether the {stage} stage has work: {e}");
            asked.failed += 1;
        }
        asked.took.push((stage, started.elapsed()));
    }
    asked
}

/// The thread asking what is due, kept in the app's state so a test that
/// wants the asking done can wait for it ([`StartupAsks::finished`]). The
/// app never waits for it. Tests that don't want the asking at all don't
/// start it (`setup_with` in `lib.rs`).
pub struct StartupAsks(Mutex<Option<JoinHandle<()>>>);

impl StartupAsks {
    /// Waits until every stage has been asked. Returns at once if it
    /// already has been.
    pub fn finished(&self) {
        let thread = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

/// Starts the asking on a thread of its own, once the job queue is in the
/// app's state. Call it after the send jobs are dropped and the interrupted
/// jobs recovered.
pub fn start<R: Runtime>(app: &AppHandle<R>, writer: Writer) -> StartupAsks {
    let handle = app.clone();
    let Some(reads) = app.try_state::<ReadPool>().map(|r| r.inner().clone()) else {
        eprintln!("startup: no read pool, so nothing was asked for what is due");
        return StartupAsks(Mutex::new(None));
    };
    let spawned = std::thread::Builder::new()
        .name("startup-asks".into())
        .spawn(move || {
            let enqueue = |job: NewJob| -> Result<JobId, DbError> {
                handle
                    .try_state::<JobQueue>()
                    .ok_or(DbError::WriterGone)?
                    .enqueue(job)
            };
            let reach = reach_by(crate::scan::system_volumes);
            let started = Instant::now();
            let asked = ask_what_is_due(&writer, &reads, &enqueue, &reach);
            eprintln!(
                "startup: asked what is due in {:?} ({} failed)",
                started.elapsed(),
                asked.failed
            );
        });
    match spawned {
        Ok(thread) => StartupAsks(Mutex::new(Some(thread))),
        Err(e) => {
            eprintln!("startup: couldn't start asking what is due: {e}");
            StartupAsks(Mutex::new(None))
        }
    }
}
