//! The waveform job: decodes each file once and stores its peaks
//! (ROADMAP 1.11 "Audio"; 1cB-9).
//!
//! The job works through every Library track's shown file that is due (see [`ledger`]),
//! **one file at a time**, on one thread taken from the fingerprint jobs'
//! shared budget ([`FirstUp`]) at below-normal priority. It is the
//! lowest-priority job there is ([`PRIORITY`]) and stays behind everything
//! else:
//!
//! - **Between two files** it looks at the job table. If a job of a higher
//!   priority is *queued*, it stops and queues the rest as one more run
//!   behind it. If one is *running* (a fingerprint run an idle worker took
//!   at once, a quality check, matching), it gives its thread back to the
//!   budget and waits, a poll at a time, until nothing above it runs; it
//!   takes a thread again only when one is free. So it never holds a
//!   fingerprint thread while a fingerprint job runs. It makes way between
//!   files, not inside one: a very long file is decoded to its end.
//! - **A file asked for** ([`waveform_job_for`], the player's loaded file,
//!   1cB-9) is drawn first, by a job of its own at user priority that takes
//!   no thread from the budget and waits for no other job. It is dropped if
//!   the player has asked for another file since ([`Wanted`]), so skipping
//!   through tracks leaves no queue of decodes behind.
//! - **One decode at a time in the whole app.** Every decode, the
//!   background run's and a file asked for, takes the same lock, so memory
//!   stays capped whatever asked: a file asked for waits for the decode in
//!   progress, at most one file's time.
//!
//! Per file: if another file already holds the waveform of the very same
//! audio (`audio_hash`), the peaks are copied and nothing is opened.
//! Otherwise: resolve its `\\?\` path, skip it if its volume went offline
//! since the job listed it, check it may be read (OneDrive), open it
//! read-only, check it's the size and age its row says, decode it once
//! ([`compute`]) and store the result through the writer. A file whose
//! content can't be decoded gets a row saying why, and isn't tried again
//! until its audio changes; a decoder panic counts against that file alone.
//! A file that couldn't be reached or read (offline, locked, an I/O error)
//! gets nothing recorded, so it's tried again next time. A cancel stops
//! mid-file and writes nothing for it.
//!
//! Memory is bounded: a packet's samples and the accumulator's few
//! thousand slots, whatever the length of the track.

use std::collections::HashMap;
use std::fs::File;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread;
use std::time::Duration;

use super::ledger::{self, Row};
use super::peaks::compute;
use crate::fingerprint::decode::Stopped;
use crate::fingerprint::{default_threads, lower_priority, FirstUp};
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::paths::{RelPath, Volumes};
use crate::quality::{unchanged, Failure};
use crate::scan::folders::{self, StoredFolder};
use crate::scan::{MusicFolderId, ReadGate};

/// The job's priority: below the roles job's, which is below matching's,
/// which is below the rest of the chain, so waveforms are drawn when
/// nothing else is waiting.
pub const PRIORITY: Priority = Priority(crate::file_roles::PRIORITY.0 - 10);

/// A waveform job over every file that's due.
pub fn waveform_job() -> NewJob {
    NewJob::new(JobKind::Waveform).priority(PRIORITY)
}

/// A waveform job for `file_ids` only, at user priority: the file the player
/// has loaded, drawn before the rest.
pub fn waveform_job_for(file_ids: Vec<i64>) -> NewJob {
    NewJob::new(JobKind::Waveform)
        .priority(Priority::USER)
        .target(serde_json::json!({ "file_ids": file_ids }))
}

/// The files a job's target names: all due files for no target. `None` if
/// the target isn't one [`waveform_job_for`] makes.
fn wanted(target: Option<&serde_json::Value>) -> Option<Option<Vec<i64>>> {
    let Some(target) = target else {
        return Some(None);
    };
    let ids = target.get("file_ids")?.as_array()?;
    ids.iter()
        .map(|id| id.as_i64())
        .collect::<Option<Vec<_>>>()
        .map(Some)
}

/// How often a job waiting for a free thread checks again.
const THREAD_POLL: Duration = Duration::from_millis(100);

/// How often a job paused behind a running job of a higher priority checks
/// whether it has ended.
const PAUSE_POLL: Duration = Duration::from_millis(500);

/// Jobs of a higher priority than the waveform job's, by where they are.
struct Above {
    queued: bool,
    running: bool,
}

fn jobs_above(conn: &rusqlite::Connection) -> rusqlite::Result<Above> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM job WHERE status = 'queued' AND priority > ?1),
                EXISTS (SELECT 1 FROM job WHERE status = 'running' AND priority > ?1)",
    )?
    .query_row([PRIORITY.0], |r| {
        Ok(Above {
            queued: r.get(0)?,
            running: r.get(1)?,
        })
    })
}

/// How often a job waiting for the one decode at a time checks again.
const DECODE_POLL: Duration = Duration::from_millis(20);

/// The file the player asked for last ([`waveform_job_for`]). A job for one
/// file drops out when another has been asked for since: the player moved
/// on, so a run of skipped tracks never piles up decodes behind each other.
/// Cheap to clone; clones share the answer.
#[derive(Clone, Default)]
pub struct Wanted(Arc<Mutex<Option<i64>>>);

impl Wanted {
    fn lock(&self) -> MutexGuard<'_, Option<i64>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes that the player wants file `id` now.
    pub(crate) fn ask(&self, id: i64) {
        *self.lock() = Some(id);
    }

    /// Whether `id` is still the file asked for last.
    fn is(&self, id: i64) -> bool {
        *self.lock() == Some(id)
    }
}

/// Test hooks: called with each file id as the job takes it, with each file
/// id it decodes (and when that decode is over), whenever the job waits
/// behind a running job, and when a decode waits for the one in progress.
type FileHook = Box<dyn Fn(i64) + Send + Sync>;
type PauseHook = Box<dyn Fn() + Send + Sync>;

/// The waveform job's handler. One handler serves every waveform job, so
/// what it holds is shared by all of them.
pub struct Waveformer<V> {
    /// The volumes mounted now, asked once per job.
    volumes: Box<dyn Fn() -> V + Send + Sync>,
    first: FirstUp,
    budget: usize,
    /// Held while a file is decoded: **one decode at a time in the whole
    /// app**, whichever job asked for it (the background run, or a file the
    /// player asked for), so memory stays capped.
    decoding: Mutex<()>,
    wanted: Wanted,
    on_file: Option<FileHook>,
    on_decode: Option<FileHook>,
    on_decoded: Option<FileHook>,
    on_pause: Option<PauseHook>,
    on_decode_wait: Option<FileHook>,
}

impl<V: Volumes + Sync + 'static> Waveformer<V> {
    /// A handler on `first`'s thread budget, which the fingerprint jobs
    /// share.
    pub fn new(volumes: impl Fn() -> V + Send + Sync + 'static, first: FirstUp) -> Waveformer<V> {
        Waveformer {
            volumes: Box::new(volumes),
            first,
            budget: default_threads(),
            decoding: Mutex::new(()),
            wanted: Wanted::default(),
            on_file: None,
            on_decode: None,
            on_decoded: None,
            on_pause: None,
            on_decode_wait: None,
        }
    }

    /// Uses `wanted` as the record of the file asked for last, shared with
    /// whoever asks (the `waveform_first` command).
    pub fn wanted(mut self, wanted: Wanted) -> Self {
        self.wanted = wanted;
        self
    }

    /// The record of the file asked for last.
    #[cfg(all(test, windows))]
    pub(crate) fn wanted_handle(&self) -> Wanted {
        self.wanted.clone()
    }

    /// Calls `hook` with each file id when its decode is over (still inside
    /// the one decode at a time).
    #[cfg(all(test, windows))]
    pub(crate) fn on_decoded(mut self, hook: impl Fn(i64) + Send + Sync + 'static) -> Self {
        self.on_decoded = Some(Box::new(hook));
        self
    }

    /// Calls `hook` with a file id once, when its decode has to wait for the
    /// decode in progress.
    #[cfg(all(test, windows))]
    pub(crate) fn on_decode_wait(mut self, hook: impl Fn(i64) + Send + Sync + 'static) -> Self {
        self.on_decode_wait = Some(Box::new(hook));
        self
    }

    /// Calls `hook` with each file id as the job takes it.
    #[cfg(all(test, windows))]
    pub(crate) fn on_file(mut self, hook: impl Fn(i64) + Send + Sync + 'static) -> Self {
        self.on_file = Some(Box::new(hook));
        self
    }

    /// Calls `hook` with each file id as the job starts decoding it (not for
    /// one whose audio another file already holds).
    #[cfg(all(test, windows))]
    pub(crate) fn on_decode(mut self, hook: impl Fn(i64) + Send + Sync + 'static) -> Self {
        self.on_decode = Some(Box::new(hook));
        self
    }

    /// Calls `hook` each time the job waits behind a running job of a
    /// higher priority, after it has given its thread back.
    #[cfg(all(test, windows))]
    pub(crate) fn on_pause(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_pause = Some(Box::new(hook));
        self
    }

    /// How many threads the budget holds. At least 1.
    #[cfg(all(test, windows))]
    pub(crate) fn budget(mut self, budget: usize) -> Self {
        self.budget = budget.max(1);
        self
    }
}

/// What a job knows while it runs.
struct Run<'a, V> {
    job: &'a JobContext,
    gate: ReadGate,
    volumes: V,
    folders: HashMap<MusicFolderId, StoredFolder>,
    ids: Vec<i64>,
    /// The job was asked for these files by name: no waiting for a thread or
    /// for other jobs.
    urgent: bool,
}

/// Why the loop over the files ended.
enum Ended {
    AllDone,
    MadeWay,
}

impl<V: Volumes + Sync + 'static> JobHandler for Waveformer<V> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let only = wanted(job.target())
            .ok_or_else(|| JobError::failed("a waveform job's target names no files"))?;
        let folders: HashMap<MusicFolderId, StoredFolder> = job
            .writer()
            .call(|c| folders::stored(c))?
            .into_iter()
            .map(|f| (f.id, f))
            .collect();
        let volumes = (self.volumes)();
        let urgent = only.is_some();
        let ids = match only {
            Some(ids) => ids,
            None => {
                // Due means checkable now: files on an unplugged drive aren't
                // taken (they're due again once it's back).
                let reachable: Vec<i64> = folders
                    .values()
                    .filter(|f| f.is_reachable(&volumes))
                    .map(|f| f.id.0)
                    .collect();
                job.writer()
                    .call(move |c| ledger::due_ids(c, Some(&reachable)))?
            }
        };
        // The user's OneDrive opt-in, read once for the whole job.
        let gate = job.writer().call(|c| ReadGate::for_job(c))?;
        if ids.is_empty() {
            return job.progress(1.0);
        }
        let run = Run {
            job,
            gate,
            volumes,
            folders,
            ids,
            urgent,
        };
        job.progress(0.0)?;
        // A thread of its own, so lowering its priority ends with the job.
        let ended = thread::scope(|s| {
            s.spawn(|| {
                panic::catch_unwind(AssertUnwindSafe(|| self.work(&run)))
                    .unwrap_or_else(|_| Err(JobError::failed("the waveform thread panicked")))
            })
            .join()
            .unwrap_or_else(|_| Err(JobError::failed("the waveform thread panicked")))
        });
        match ended? {
            Ended::MadeWay => make_way(job),
            Ended::AllDone => job.progress(1.0),
        }
    }
}

/// What was stored stays. One more run, at this job's priority, does the
/// rest after the waiting job: this job is still running, so it is asked to
/// run once more, and its chain wrapper queues that as it ends.
fn make_way(job: &JobContext) -> Result<(), JobError> {
    crate::scan::chain::queue_once(job.writer(), waveform_job(), |j| job.enqueue(j))?;
    Ok(())
}

impl<V: Volumes + Sync + 'static> Waveformer<V> {
    /// The thread: takes the files one at a time.
    fn work(&self, run: &Run<'_, V>) -> Result<Ended, JobError> {
        lower_priority();
        // The thread of the budget, held only while the job is drawing.
        let mut lease = None;
        for (done, &id) in run.ids.iter().enumerate() {
            loop {
                run.job.check_cancelled()?;
                if run.urgent {
                    break;
                }
                let above = run.job.writer().call(|c| jobs_above(c))?;
                if above.queued {
                    return Ok(Ended::MadeWay);
                }
                if above.running {
                    // Give the thread back: whatever runs above is what the
                    // budget is for.
                    lease = None;
                    if let Some(hook) = &self.on_pause {
                        hook();
                    }
                    thread::sleep(PAUSE_POLL);
                    continue;
                }
                if lease.is_none() {
                    lease = self.first.try_threads(1, self.budget);
                    if lease.is_none() {
                        thread::sleep(THREAD_POLL);
                        continue;
                    }
                }
                break;
            }
            if let Some(hook) = &self.on_file {
                hook(id);
            }
            self.one(run, id)?;
            run.job.progress((done + 1) as f64 / run.ids.len() as f64)?;
        }
        Ok(Ended::AllDone)
    }

    /// Computes file `id`'s peaks, if it's still due, and records how it
    /// went.
    fn one(&self, run: &Run<'_, V>, id: i64) -> Result<(), JobError> {
        let job = run.job;
        // The player has moved on to another file since this was asked for.
        if run.urgent && !self.wanted.is(id) {
            return Ok(());
        }
        let Some(due) = job.writer().call(move |c| ledger::due(c, id))? else {
            return Ok(());
        };
        // The very same audio is already drawn for another file: copy it.
        let same = due.clone();
        if job.writer().call(move |c| ledger::reuse(c, &same))? {
            return Ok(());
        }
        let Some(path) = run.path(&due.folder, &due.rel_path) else {
            // Offline, or a row this build can't place: it stays due.
            return Ok(());
        };
        // OneDrive (ROADMAP 1.1): an online-only file is never opened unless
        // the user opted in, since reading it downloads it.
        if !run.gate.allows(due.online_only) {
            return Ok(());
        }
        match run.gate.may_open(&path) {
            Ok(true) => {}
            // Online only, or couldn't tell: don't open it.
            Ok(false) | Err(_) => return Ok(()),
        }
        // One decode at a time in the whole app. Waiting for the turn can take
        // as long as the decode in progress; meanwhile the file may have been
        // drawn, or changed, or no longer be the one asked for.
        let Some(_turn) = self.take_turn(run, id)? else {
            return Ok(());
        };
        let Some(due) = job.writer().call(move |c| ledger::due(c, id))? else {
            return Ok(());
        };
        // Locked, gone, or the drive went away: nothing recorded, so it's
        // tried again next time.
        let Ok(file) = File::open(&path) else {
            return Ok(());
        };
        // Changed since the walk: the next walk updates the row, and then
        // it's due again. Nothing is recorded against the old row.
        if !unchanged(&file, &due) {
            return Ok(());
        }
        let Ok(after) = file.try_clone() else {
            return Ok(());
        };
        if let Some(hook) = &self.on_decode {
            hook(id);
        }
        let mut keep_going = |_: Option<f64>| !job.is_cancelled();
        let computed = panic::catch_unwind(AssertUnwindSafe(|| {
            compute(Box::new(file), &mut keep_going)
        }));
        if let Some(hook) = &self.on_decoded {
            hook(id);
        }
        let row = match computed {
            Ok(Ok(c)) => Row::Peaks {
                peaks: c.peaks,
                duration_ms: c.duration_ms,
            },
            Ok(Err(Stopped::Cancelled)) => return Err(JobError::Cancelled),
            Ok(Err(Stopped::Failed(why))) => match Failure::of(why) {
                Some(failure) => Row::Failed(failure),
                // Reading stopped partway (e.g. the drive was unplugged).
                None => return Ok(()),
            },
            Err(_) => Row::Failed(Failure::DecoderCrashed),
        };
        // Rewritten while it was decoding: what was read may be a mix of old
        // and new. Leave it for the next walk.
        if !unchanged(&after, &due) {
            return Ok(());
        }
        job.writer().call(move |c| ledger::record(c, &due, &row))?;
        Ok(())
    }
}

impl<V: Volumes + Sync + 'static> Waveformer<V> {
    /// The one decode at a time ([`Waveformer::decoding`]): waits for its
    /// turn, a poll at a time so a cancel is noticed. `None` if the job is
    /// for a file the player has moved on from, by the time it has the turn
    /// or while it waits.
    fn take_turn(&self, run: &Run<'_, V>, id: i64) -> Result<Option<MutexGuard<'_, ()>>, JobError> {
        let mut waited = false;
        loop {
            let turn = match self.decoding.try_lock() {
                Ok(turn) => Some(turn),
                Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),
                Err(TryLockError::WouldBlock) => None,
            };
            if let Some(turn) = turn {
                return Ok((!run.urgent || self.wanted.is(id)).then_some(turn));
            }
            if !waited {
                waited = true;
                if let Some(hook) = &self.on_decode_wait {
                    hook(id);
                }
            }
            run.job.check_cancelled()?;
            if run.urgent && !self.wanted.is(id) {
                return Ok(None);
            }
            thread::sleep(DECODE_POLL);
        }
    }
}

impl<V: Volumes> Run<'_, V> {
    /// Where the file is now, as a `\\?\` path; `None` if its volume is
    /// offline or its row can't be placed.
    fn path(&self, folder: &MusicFolderId, rel_path: &str) -> Option<PathBuf> {
        let folder = self.folders.get(folder)?;
        let rel = RelPath::parse(rel_path).ok()?;
        folder.stored_path().join(&rel).resolve(&self.volumes).ok()
    }
}
