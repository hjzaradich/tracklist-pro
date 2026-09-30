//! The scan stages, chained (1aC-8, ROADMAP 1.1).
//!
//! A walk (stage 1) that finishes queues a read (stage 2) of the folders
//! it walked; a read that finishes queues their hashes, and the hashes
//! the fingerprints (stage 3). One stage at a time: hashes first, so a
//! fingerprint job can skip a file whose audio hash it has already
//! fingerprinted (1aC-10), and two jobs never read the same disk at once.
//! Every chained job runs at background priority, so anything the user
//! asks for meanwhile (a scan, a rekordbox read, the tracks on screen)
//! goes first.
//!
//! - **Only what's new or changed:** each stage works from `file_stage`
//!   ([`crate::scan_state`]), so a chained job reads, hashes or
//!   fingerprints just the files the walk found new or changed, and a
//!   stage with nothing left to try isn't queued at all.
//! - **No duplicates:** a job isn't queued while an identical one (same
//!   kind and target) is still waiting in the queue. One that's already
//!   running may have listed its files before the latest walk, so a second
//!   one is queued behind it; further walks see that one waiting and add
//!   nothing.
//! - **No loop:** nothing here queues a walk, and a stage that has only
//!   online-only files left (which stay due, since they're skipped, not
//!   done) isn't queued again: [`any_to_try`] looks for a file the stage
//!   may actually read, never at [`crate::scan_state::count_due`].
//! - A job that stops early (cancelled, failed, the app closing) queues
//!   nothing; the next walk picks up where it left off.
//!
//! The chain wraps each stage's handler ([`after_walk`], [`after_read`],
//! [`after_hash`]) where the handlers are registered (`jobs::start`), so
//! the stages themselves know nothing about it and run alone in their own
//! tests.

use rusqlite::{Connection, OptionalExtension};

use super::folders::MusicFolderId;
use super::ReadGate;
use crate::db::{DbError, Writer};
use crate::jobs::{JobContext, JobError, JobHandler, JobId, NewJob, Priority};
use crate::scan_state::{self, Scope, Stage};

/// Due files are looked at this many at a time by [`any_to_try`].
const PAGE: usize = 1000;

/// `walker`, followed by a read of the folders it walked.
pub fn after_walk<H: JobHandler>(walker: H) -> Chained<H> {
    Chained {
        inner: walker,
        next: Next::Read,
    }
}

/// `reader`, followed by the hashes of the folders it read.
pub fn after_read<H: JobHandler>(reader: H) -> Chained<H> {
    Chained {
        inner: reader,
        next: Next::Hash,
    }
}

/// `hasher`, followed by the fingerprints.
pub fn after_hash<H: JobHandler>(hasher: H) -> Chained<H> {
    Chained {
        inner: hasher,
        next: Next::Fingerprint,
    }
}

/// A stage's handler with the next stage queued after it finishes.
pub struct Chained<H> {
    inner: H,
    next: Next,
}

/// What follows a finished stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    /// Stage 2: read tags and properties.
    Read,
    /// Stage 3, first the hashes…
    Hash,
    /// …then the fingerprints.
    Fingerprint,
}

impl<H: JobHandler> JobHandler for Chained<H> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        self.inner.run(job)?;
        // The stage's own work is done and written. If the next stage
        // can't be queued, the job still says why.
        queue_next(job, self.next).map_err(|e| match e {
            JobError::Cancelled => JobError::Cancelled,
            JobError::Failed(e) => JobError::failed(format!(
                "the {} job finished, but the next stage couldn't be queued: {e}",
                job.kind()
            )),
        })
    }
}

/// Queues what follows `job`, a finished stage.
fn queue_next(job: &JobContext, next: Next) -> Result<(), JobError> {
    let ids = folders_in(job.target());
    let scope = match &ids {
        Some(ids) => Scope::Folders(ids.iter().map(|id| id.0).collect()),
        None => Scope::All,
    };
    let writer = job.writer();
    let enqueue = |j: NewJob| job.enqueue(j);
    match next {
        Next::Read => {
            if to_try(writer, Stage::Read, crate::read::READ_VERSION, &scope)? {
                let read = crate::read::read_job(ids).priority(Priority::BACKGROUND);
                unless_queued(writer, read, enqueue)?;
            }
        }
        Next::Hash => {
            let hash_version = i64::from(crate::hash::DEFINITION);
            if to_try(writer, Stage::Hash, hash_version, &scope)? {
                let hash = crate::hash::hash_job(ids).priority(Priority::BACKGROUND);
                unless_queued(writer, hash, enqueue)?;
            } else {
                // Nothing to hash: straight on to the fingerprints.
                queue_next(job, Next::Fingerprint)?;
            }
        }
        Next::Fingerprint => {
            // A fingerprint job takes every due file, whatever its folder.
            let fp_version = i64::from(crate::fingerprint::VERSION);
            if to_try(writer, Stage::Fingerprint, fp_version, &Scope::All)? {
                let fp = crate::fingerprint::fingerprint_job(None).priority(Priority::BACKGROUND);
                unless_queued(writer, fp, enqueue)?;
            }
        }
    }
    Ok(())
}

/// The music folders a stage job's target names (`music_folder_ids`), or
/// `None` for every folder. A target this can't read means every folder:
/// the stage itself already refused a target it couldn't read.
pub(crate) fn folders_in(target: Option<&serde_json::Value>) -> Option<Vec<MusicFolderId>> {
    let ids = target?.get("music_folder_ids")?.as_array()?;
    ids.iter()
        .map(|id| id.as_i64().map(MusicFolderId))
        .collect::<Option<Vec<_>>>()
}

/// Whether `stage` has a file in `scope` it may actually try: due, and not
/// an online-only file the user hasn't opted in to reading. Those stay due
/// forever (a skip, never a result), so counting them would queue a stage
/// that has nothing to do.
fn to_try(writer: &Writer, stage: Stage, version: i64, scope: &Scope) -> Result<bool, DbError> {
    let scope = scope.clone();
    writer.call(move |c| {
        let gate = ReadGate::for_job(c)?;
        any_to_try(c, stage, version, &scope, gate)
    })
}

/// [`to_try`] on a connection: pages through the due files (never
/// starting over) until one the gate allows turns up.
pub(crate) fn any_to_try(
    conn: &Connection,
    stage: Stage,
    version: i64,
    scope: &Scope,
    gate: ReadGate,
) -> rusqlite::Result<bool> {
    let mut after = 0;
    loop {
        let page = scan_state::due(conn, stage, version, scope, after, PAGE)?;
        let Some(last) = page.last() else {
            return Ok(false);
        };
        if page.iter().any(|f| gate.allows(f.online_only)) {
            return Ok(true);
        }
        after = last.id;
    }
}

/// Queues `job` with `enqueue`, unless a job of the same kind and target
/// is already waiting in the queue; then returns that one's id and queues
/// nothing.
pub(crate) fn unless_queued<E: From<DbError>>(
    writer: &Writer,
    job: NewJob,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<JobId, E> {
    let same = job.clone();
    if let Some(waiting) = writer.call(move |c| queued(c, &same))? {
        return Ok(waiting);
    }
    enqueue(job)
}

/// A queued (not yet running) job with `job`'s kind and target, if any.
pub(crate) fn queued(conn: &Connection, job: &NewJob) -> rusqlite::Result<Option<JobId>> {
    let target = job.target.as_ref().map(|t| t.to_string());
    conn.prepare_cached(
        "SELECT id FROM job
         WHERE status = 'queued' AND kind = ?1 AND target IS ?2
         ORDER BY id LIMIT 1",
    )?
    .query_row((job.kind.as_str(), target), |r| r.get(0))
    .optional()
    .map(|id| id.map(JobId))
}
