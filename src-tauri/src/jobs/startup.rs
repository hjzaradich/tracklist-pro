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
//!   once (about as long as a no-change walk's own checks), so a library with
//!   nothing due costs a few quick passes over the file table, off the UI
//!   path.
//! - **One list of "ask this stage" calls** ([`ASKS`]), one line per stage,
//!   in the scan chain's order (file roles last). Each call goes through the stage's own
//!   request (`queue_once`), so it never queues what is already queued or
//!   running, and each leaves a stage alone while an earlier one is still
//!   due (the earlier stage's job chains on to it, as in the scan chain).
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
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime};

use super::{JobId, JobQueue, NewJob};
use crate::db::{DbError, Writer};
use crate::scan::chain::{any_to_try, queue_once, reach_by, Reach};
use crate::scan::folders::{self, MusicFolderId};
use crate::scan::ReadGate;
use crate::scan_state::{Scope, Stage};

/// How a job is queued: the queue's `enqueue`, or a stand-in in tests.
pub(crate) type Enqueue<'a> = &'a dyn Fn(NewJob) -> Result<JobId, DbError>;

/// One stage's question. Each is one line of [`ASKS`].
type Ask = fn(&Asking) -> Result<(), DbError>;

/// Every stage asked at startup, in the scan chain's order. To ask another
/// stage, add a line.
const ASKS: &[(&str, Ask)] = &[
    ("hash", ask_hash),
    ("group", ask_group),
    ("fingerprint", ask_fingerprint),
    ("quality", ask_quality),
    ("matching", ask_matching),
    ("decide", ask_decide),
    ("file roles", ask_file_roles),
];

/// What the asks share.
pub(crate) struct Asking<'a> {
    writer: &'a Writer,
    enqueue: Enqueue<'a>,
    reach: &'a Reach,
    /// The music folders that can be reached now.
    reachable: Vec<i64>,
    read_due: OnceCell<bool>,
    hash_due: OnceCell<bool>,
    fingerprint_due: OnceCell<bool>,
}

impl Asking<'_> {
    /// Whether `stage` has a file worth trying in the reachable folders, as
    /// a scan of them would judge it (not a watcher's rescan).
    fn due(&self, stage: Stage, version: i64) -> Result<bool, DbError> {
        if self.reachable.is_empty() {
            return Ok(false);
        }
        let scope = Scope::Folders(self.reachable.clone());
        self.writer.call(move |c| {
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

    fn enqueue(&self, job: NewJob) -> Result<JobId, DbError> {
        (self.enqueue)(job)
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
        queue_once(a.writer, crate::hash::hash_job(Some(ids)), |j| a.enqueue(j))?;
    }
    Ok(())
}

fn ask_group(a: &Asking) -> Result<(), DbError> {
    if a.hashes_owed()? {
        return Ok(());
    }
    crate::grouping::request_group(a.writer, |j| a.enqueue(j))
}

fn ask_fingerprint(a: &Asking) -> Result<(), DbError> {
    if !a.hashes_owed()? && a.fingerprint_due()? {
        let job = crate::fingerprint::fingerprint_job(None);
        queue_once(a.writer, job, |j| a.enqueue(j))?;
    }
    Ok(())
}

fn ask_quality(a: &Asking) -> Result<(), DbError> {
    if a.fingerprints_owed()? {
        return Ok(());
    }
    crate::quality::request(a.writer, Some(a.reach), |j| a.enqueue(j))
}

fn ask_matching(a: &Asking) -> Result<(), DbError> {
    if a.fingerprints_owed()? {
        return Ok(());
    }
    crate::matching::request(a.writer, |j| a.enqueue(j))
}

fn ask_decide(a: &Asking) -> Result<(), DbError> {
    if a.fingerprints_owed()? {
        return Ok(());
    }
    crate::grouping::request_decide(a.writer, |j| a.enqueue(j))
}

/// A stage with a plain request needs only this: its own check, then the
/// job. The file roles job is the lowest priority, so it needs no waiting
/// for the stages before it.
fn ask_file_roles(a: &Asking) -> Result<(), DbError> {
    crate::file_roles::request(a.writer, |j| a.enqueue(j))
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
pub(crate) fn ask_what_is_due(writer: &Writer, enqueue: Enqueue<'_>, reach: &Reach) -> Asked {
    let mut asked = Asked::default();
    let folders = match writer.call(|c| folders::stored(c)) {
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
        enqueue,
        reach,
        reachable: reach(&folders),
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

/// Starts the asking on a thread of its own, once the job queue is in the
/// app's state. Call it after the send jobs are dropped and the interrupted
/// jobs recovered.
pub fn start<R: Runtime>(app: &AppHandle<R>, writer: Writer) {
    let handle = app.clone();
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
            let asked = ask_what_is_due(&writer, &enqueue, &reach);
            eprintln!(
                "startup: asked what is due in {:?} ({} failed)",
                started.elapsed(),
                asked.failed
            );
        });
    if let Err(e) = spawned {
        eprintln!("startup: couldn't start asking what is due: {e}");
    }
}
