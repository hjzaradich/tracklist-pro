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
//!   stage with nothing to try isn't queued at all ([`any_to_try`]).
//! - **One job per kind and target at a time** ([`queue_once`]): a job
//!   isn't queued while an identical one (same kind and target) is
//!   queued or running. A running one may have listed its files before
//!   the latest walk, so it's asked to **run once more** when it ends
//!   (the [`Chained`] wrapper does that), and every further request while
//!   it runs folds into that one rerun. So a long first fingerprint run
//!   never gets a second job parked beside it, holding a worker (a parked
//!   job waits for the fingerprint thread budget).
//! - **No loop:** nothing here queues a walk, and a stage whose only due
//!   files are ones it can't try (online-only without the opt-in; or, on
//!   a watcher's rescan, files it already found unreachable, unchanged
//!   since) isn't queued again. Never [`crate::scan_state::count_due`],
//!   which never reaches 0 while files are skipped.
//! - A job that stops early (cancelled, failed, the app closing) queues
//!   nothing and drops its rerun; the next walk picks up where it left off.
//! - **Relink after the read:** a finished read has new durations and
//!   tags, so rekordbox tracks may match files now. It asks for a relink
//!   through [`crate::relink::request`], which never queues a second one
//!   while one waits, and queues one more after one that's running (the
//!   relink job isn't [`Chained`], so [`queue_once`]'s rerun doesn't apply
//!   to it).
//!
//! The chain wraps each stage's handler ([`after_walk`], [`after_read`],
//! [`after_hash`], [`after_fingerprint`]) where the handlers are
//! registered (`jobs::start`), so the stages themselves know nothing
//! about it and run alone in their own tests. The manual commands
//! (`read_files`, `hash_music_folders`) go through the same handlers, so
//! they chain onward too.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use rusqlite::Connection;

use super::folders::MusicFolderId;
use super::ReadGate;
use crate::db::{DbError, Writer};
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, NewJob, Priority};
use crate::scan_state::{Scope, Stage};

/// The key in a stage job's target that marks a watcher's rescan (as
/// opposed to a scan at app start, on a drive's return, or asked for):
/// then a file a stage already found unreachable, unchanged since, isn't
/// worth another try (it's retried on the other kinds of scan). The walk
/// and the stages ignore the key.
pub const RESCAN_KEY: &str = "rescan";

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

/// `fingerprinter`, run once more if a walk asked for it meanwhile.
pub fn after_fingerprint<H: JobHandler>(fingerprinter: H) -> Chained<H> {
    Chained {
        inner: fingerprinter,
        next: Next::Nothing,
    }
}

/// A stage's handler with the next stage queued after it finishes, and
/// the stage itself queued once more if that was asked for while it ran.
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
    /// The last stage.
    Nothing,
}

impl<H: JobHandler> JobHandler for Chained<H> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let me = NewJob {
            kind: job.kind(),
            target: job.target().cloned(),
            priority: Priority::BACKGROUND,
        };
        let key = key(job.writer(), &me);
        // This run lists its files now: a rerun asked for from here on is
        // for what comes after.
        state().ending.remove(&key);
        if let Err(e) = self.inner.run(job) {
            // Stopped early: whoever asked for a rerun waits for the next
            // walk, like everything else this job left undone. From here
            // on a request queues a job of its own (see `rerun_if_asked`).
            let mut state = state();
            state.asked.remove(&key);
            state.ending.insert(key);
            return Err(e);
        }
        // The stage's own work is done and written. If what follows can't
        // be queued, the job still says why.
        let followed = queue_next(job, self.next).and_then(|()| rerun_if_asked(job, me));
        followed.map_err(|e| match e {
            JobError::Cancelled => JobError::Cancelled,
            JobError::Failed(e) => JobError::failed(format!(
                "the {} job finished, but what follows it couldn't be queued: {e}",
                job.kind()
            )),
        })
    }
}

/// Queues what follows `job`, a finished stage.
fn queue_next(job: &JobContext, next: Next) -> Result<(), JobError> {
    let ids = folders_in(job.target());
    let rescan = is_rescan(job.target());
    let scope = match &ids {
        Some(ids) => Scope::Folders(ids.iter().map(|id| id.0).collect()),
        None => Scope::All,
    };
    let writer = job.writer();
    let enqueue = |j: NewJob| job.enqueue(j);
    // The job's own target (folders and the rescan mark) goes on to the
    // next stage.
    let target = |mut new: NewJob| {
        if rescan {
            if let Some(serde_json::Value::Object(t)) = &mut new.target {
                t.insert(RESCAN_KEY.into(), serde_json::Value::Bool(true));
            }
        }
        new.priority(Priority::BACKGROUND)
    };
    match next {
        Next::Read => {
            if to_try(
                writer,
                Stage::Read,
                crate::read::READ_VERSION,
                &scope,
                rescan,
            )? {
                queue_once(writer, target(crate::read::read_job(ids)), enqueue)?;
            }
        }
        Next::Hash => {
            // Durations and tags are in: rekordbox tracks may match files
            // now (1aC-3).
            crate::relink::request(writer, enqueue)?;
            let hash_version = i64::from(crate::hash::DEFINITION);
            if to_try(writer, Stage::Hash, hash_version, &scope, rescan)? {
                queue_once(writer, target(crate::hash::hash_job(ids)), enqueue)?;
            } else {
                // Nothing to hash: straight on to the fingerprints.
                queue_next(job, Next::Fingerprint)?;
            }
        }
        Next::Fingerprint => {
            // A fingerprint job takes every due file, whatever its folder;
            // whether one is worth queuing is judged on the folders this
            // chain walked (an offline drive's files stay due for ever).
            let fp_version = i64::from(crate::fingerprint::VERSION);
            if to_try(writer, Stage::Fingerprint, fp_version, &scope, rescan)? {
                let fp = crate::fingerprint::fingerprint_job(None).priority(Priority::BACKGROUND);
                queue_once(writer, fp, enqueue)?;
            }
        }
        Next::Nothing => {}
    }
    Ok(())
}

/// Queues `me` once more if [`queue_once`] was asked for it while it ran.
/// From here on the job counts as ending: a request that arrives before
/// the queue records its end queues a job of its own instead of asking.
fn rerun_if_asked(job: &JobContext, mut me: NewJob) -> Result<(), JobError> {
    let mut state = state();
    let key = key(job.writer(), &me);
    state.ending.insert(key.clone());
    if let Some(unmarked_wanted) = state.asked.remove(&key) {
        // A watcher's rescan mark holds only if this run had it and every
        // ask did too; otherwise the rerun retries everything.
        if unmarked_wanted {
            unmark(&mut me);
        }
        // Under the lock, so a request arriving now sees this one queued.
        job.enqueue(me)?;
        // The queued rerun isn't ending: a request seeing it claimed
        // before its own run starts asks it, rather than queuing a twin.
        state.ending.remove(&key);
    }
    Ok(())
}

/// `job`'s target without the watcher's rescan mark.
fn unmark(job: &mut NewJob) {
    if let Some(serde_json::Value::Object(t)) = &mut job.target {
        t.remove(RESCAN_KEY);
    }
}

/// `job` with the watcher's rescan mark.
fn marked(job: &NewJob) -> NewJob {
    let mut marked = job.clone();
    if let Some(serde_json::Value::Object(t)) = &mut marked.target {
        t.insert(RESCAN_KEY.into(), serde_json::Value::Bool(true));
    }
    marked
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

/// Whether a stage job's target carries the watcher's rescan mark.
pub(crate) fn is_rescan(target: Option<&serde_json::Value>) -> bool {
    target
        .and_then(|t| t.get(RESCAN_KEY))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Whether `stage` has a file in `scope` worth trying, through the
/// writer.
fn to_try(
    writer: &Writer,
    stage: Stage,
    version: i64,
    scope: &Scope,
    rescan: bool,
) -> Result<bool, DbError> {
    let scope = scope.clone();
    writer.call(move |c| {
        let gate = ReadGate::for_job(c)?;
        any_to_try(c, stage, version, &scope, gate, rescan)
    })
}

/// Whether `stage` at `version` has a present file in `scope` worth
/// trying: one it has never done, or that changed size or mtime since,
/// or was done by another version, or that it skipped last time for a
/// reason that may have gone away; and that the gate allows (not
/// online-only without the opt-in, which would be skipped again).
///
/// A file skipped as unreachable (locked, no permission) and unchanged
/// since counts on a scan at app start, on a drive's return or asked for,
/// where it's retried; not on a watcher's `rescan`, or every burst of
/// changes would run every stage again for one locked file.
pub(crate) fn any_to_try(
    conn: &Connection,
    stage: Stage,
    version: i64,
    scope: &Scope,
    gate: ReadGate,
    rescan: bool,
) -> rusqlite::Result<bool> {
    let json = |ids: &[i64]| serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
    let (folders, files) = match scope {
        Scope::All => (None, None),
        Scope::Folders(ids) => (Some(json(ids)), None),
        Scope::Files(ids) => (None, Some(json(ids))),
    };
    conn.prepare_cached(
        "SELECT EXISTS (
             SELECT 1 FROM file f
             LEFT JOIN file_stage s ON s.file_id = f.id AND s.stage = ?1
             WHERE f.present = 1
               AND (?3 IS NULL OR f.music_folder_id IN (SELECT value FROM json_each(?3)))
               AND (?4 IS NULL OR f.id IN (SELECT value FROM json_each(?4)))
               AND (f.online_only = 0 OR ?5)
               AND (s.file_id IS NULL
                    OR s.version <> ?2
                    OR s.size IS NOT f.size
                    OR s.mtime IS NOT f.mtime
                    OR (s.status = 'skipped' AND NOT (?6 AND s.reason = ?7))))",
    )?
    .query_row(
        (
            stage.as_str(),
            version,
            folders,
            files,
            gate.allows(true),
            rescan,
            crate::scan_state::UNREACHABLE,
        ),
        |r| r.get(0),
    )
}

/// Queues `job` with `enqueue`, unless a job of the same kind and target
/// is queued or running. Queued: nothing to do, and its id is returned.
/// Running: it may already have listed its files, so it's asked to run
/// once more when it ends (its [`Chained`] wrapper does that), and its id
/// is returned; if it's already ending (its wrapper has had its last
/// look), a job of its own is queued. One lock orders the check and the
/// insert, so two stages ending at once can't both queue the same job.
pub(crate) fn queue_once<E: From<DbError>>(
    writer: &Writer,
    job: NewJob,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<JobId, E> {
    let mut state = state();
    let same = job.clone();
    let active = writer.call(move |c| active(c, &same))?;
    if let Some(queued) = active.iter().find(|(_, running)| !running) {
        return Ok(queued.0);
    }
    let key = key(writer, &job);
    match active.first() {
        Some((running, _)) if !state.ending.contains(&key) => {
            let unmarked = !is_rescan(job.target.as_ref());
            *state.asked.entry(key).or_insert(false) |= unmarked;
            Ok(*running)
        }
        _ => {
            state.asked.remove(&key);
            // The new job isn't ending (see `rerun_if_asked`).
            state.ending.remove(&key);
            enqueue(job)
        }
    }
}

/// A job's identity for [`queue_once`]: its database, kind and target
/// as stored, the watcher's rescan mark set aside (a rescan for changes
/// and a catch-up of the same folders are the same walk).
type Key = (std::path::PathBuf, JobKind, Option<String>);

/// What the chain remembers about running jobs, under one lock that also
/// orders every [`queue_once`] (the check and the insert as one step).
#[derive(Default)]
struct ChainState {
    /// Jobs asked to run once more when their current run ends, and
    /// whether an ask was without the rescan mark. In memory only: a run
    /// cut short by the app closing goes back in the queue whole, and
    /// lists its files afresh.
    asked: HashMap<Key, bool>,
    /// Jobs whose wrapper has had its last look for an ask, but that the
    /// queue may not have recorded as finished yet.
    ending: HashSet<Key>,
}

static STATE: LazyLock<Mutex<ChainState>> = LazyLock::new(Mutex::default);

fn state() -> MutexGuard<'static, ChainState> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn key(writer: &Writer, job: &NewJob) -> Key {
    let mut unmarked = job.clone();
    unmark(&mut unmarked);
    (
        writer.path().to_path_buf(),
        job.kind,
        unmarked.target.as_ref().map(|t| t.to_string()),
    )
}

/// Every queued or running job with `job`'s kind and target (with or
/// without the rescan mark): id and whether it's running, running ones
/// first.
pub(crate) fn active(conn: &Connection, job: &NewJob) -> rusqlite::Result<Vec<(JobId, bool)>> {
    let mut unmarked = job.clone();
    unmark(&mut unmarked);
    let plain = unmarked.target.as_ref().map(|t| t.to_string());
    let with_mark = marked(&unmarked).target.map(|t| t.to_string());
    let mut stmt = conn.prepare_cached(
        "SELECT id, status = 'running' FROM job
         WHERE status IN ('queued', 'running') AND kind = ?1
           AND (target IS ?2 OR target IS ?3)
         ORDER BY status = 'running' DESC, id",
    )?;
    let rows = stmt.query_map((job.kind.as_str(), plain, with_mark), |r| {
        Ok((JobId(r.get(0)?), r.get(1)?))
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::jobs::{JobKind, JobQueue};
    use crate::read::read_job;

    fn temp_writer() -> (tempfile::TempDir, Writer) {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        (dir, writer)
    }

    /// A job row that says `running`, as the queue leaves one while its
    /// handler runs (and for a moment after it returns).
    fn running(writer: &Writer, kind: JobKind) {
        writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO job (kind, priority, status, started_at)
                     VALUES (?1, -10, 'running', 'now')",
                    [kind.as_str()],
                )
            })
            .unwrap();
    }

    fn key_of(writer: &Writer, job: &NewJob) -> Key {
        key(writer, job)
    }

    fn clean(writer: &Writer, job: &NewJob) {
        let key = key_of(writer, job);
        let mut state = state();
        state.asked.remove(&key);
        state.ending.remove(&key);
    }

    #[test]
    fn a_request_while_a_job_runs_asks_it_to_run_once_more() {
        let (_dir, writer) = temp_writer();
        let job = read_job(None);
        clean(&writer, &job);
        running(&writer, JobKind::Read);
        let mut queued = 0;
        let id = queue_once(&writer, job.clone(), |_| -> Result<JobId, DbError> {
            queued += 1;
            Ok(JobId(99))
        })
        .unwrap();
        assert_eq!(queued, 0, "asked, not queued");
        assert_eq!(id, JobId(1), "the running job's id");
        assert_eq!(state().asked.get(&key_of(&writer, &job)), Some(&true));
        clean(&writer, &job);
    }

    #[test]
    fn a_request_after_a_runs_last_look_queues_its_own_job_and_the_new_job_is_not_ending() {
        // The wrapper has had its last look for an ask (or the run failed),
        // but the queue hasn't recorded the end yet: the row still says
        // running.
        let (_dir, writer) = temp_writer();
        let job = read_job(None);
        clean(&writer, &job);
        running(&writer, JobKind::Read);
        let key = key_of(&writer, &job);
        state().ending.insert(key.clone());
        let mut queued = 0;
        let id = queue_once(&writer, job.clone(), |_| -> Result<JobId, DbError> {
            queued += 1;
            Ok(JobId(99))
        })
        .unwrap();
        assert_eq!((queued, id), (1, JobId(99)), "a job of its own");
        let state = state();
        assert!(!state.asked.contains_key(&key));
        // The job just queued can be asked once it runs: it isn't ending.
        assert!(!state.ending.contains(&key));
        drop(state);
        clean(&writer, &job);
    }

    #[test]
    fn a_run_that_stops_early_drops_its_asks_and_counts_as_ending() {
        let (_dir, writer) = temp_writer();
        let queue = JobQueue::builder(writer.clone())
            .workers(1)
            .handler(
                JobKind::Read,
                after_read(|_: &JobContext| Err(JobError::failed("the drive went away"))),
            )
            .start()
            .unwrap();
        let job = read_job(None);
        let key = key_of(&writer, &job);
        clean(&writer, &job);
        // As if a request had come in while it ran.
        state().asked.insert(key.clone(), true);
        let id = queue.enqueue(job.clone()).unwrap();
        let start = std::time::Instant::now();
        loop {
            let stored = writer
                .call(move |c| crate::jobs::store::get(c, id))
                .unwrap()
                .unwrap();
            if stored.status.is_finished() {
                assert_eq!(stored.status, crate::jobs::JobStatus::Failed);
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        queue.shutdown();
        let state = state();
        assert!(!state.asked.contains_key(&key), "the ask was dropped");
        assert!(
            state.ending.contains(&key),
            "a late request queues its own job"
        );
        drop(state);
        clean(&writer, &job);
        let _: PathBuf = key.0;
    }

    use std::time::Duration;
}
