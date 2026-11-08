//! The embed job: listens to each Library track's file with the audio model
//! and keeps what it heard (ROADMAP 3.8; 5A-A2b).
//!
//! It works like the analyze job ([`crate::estimate`]), and beside it:
//!
//! - **Only while the switch is on** ([`Feature::AudioModel`], off by
//!   default). With it off the job ends at once and the model is never
//!   loaded; turning it off cancels the job ([`super::control`]).
//! - **A few threads, at below-normal priority**, at a priority just under
//!   the analyze job's, so it runs when nothing else is waiting. Before each
//!   file a thread looks at the job table: if a job of a higher priority is
//!   queued the job stops and queues the rest as one more run behind it; if
//!   one is running the thread waits until nothing above it runs.
//! - **The model is loaded once per run** and shared by the threads. If it
//!   can't be loaded the job ends failed with the reason, and nothing else
//!   is affected.
//! - **Resumable.** Every file's result is its own transaction and the job
//!   works from what's still due ([`ledger`]), so a run cut short (closed,
//!   cancelled, made way) is picked up where it stopped. A cancel stops
//!   mid-file and writes nothing for that file.
//! - **A bad file never stops the run.** A file the model can't listen to
//!   gets a failure row saying why ([`Reason`]) and isn't tried again until
//!   its audio changes; a panic counts against that file alone.

use std::collections::HashMap;
use std::fs::File;
use std::io::ErrorKind;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::ledger::{self, Outcome, Reason};
use super::subset;
use super::{AudioModel, ListenError, ModelError};
use crate::fingerprint::lower_priority;
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::paths::{RelPath, Volumes};
use crate::quality::unchanged;
use crate::scan::folders::{self, StoredFolder};
use crate::scan::{MusicFolderId, ReadGate};
use crate::settings::{read_feature_enabled, Feature};

/// The job's priority: just under the analyze job's (the lowest tier), so
/// the cheaper measurements are made first and neither makes the laptop busy
/// while anything else waits.
pub const PRIORITY: Priority = Priority(crate::estimate::PRIORITY.0 - 1);

/// An embed job over every file that's due.
pub fn embed_job() -> NewJob {
    NewJob::new(JobKind::Embed).priority(PRIORITY)
}

/// How many worker threads a run uses: one fewer than the cores, at most 4,
/// at least 1.
pub fn default_workers() -> usize {
    let cores = thread::available_parallelism().map_or(1, |n| n.get());
    cores.saturating_sub(1).clamp(1, 4)
}

/// How often a job paused behind a running job of a higher priority checks
/// whether it has ended.
const PAUSE_POLL: Duration = Duration::from_millis(500);

/// A test hook, called with each file id as a thread takes it.
type FileHook = Box<dyn Fn(i64) + Send + Sync>;

/// How the model is loaded: the app's bundled file, or a path in tests.
type Loader = Box<dyn Fn() -> Result<Arc<AudioModel>, ModelError> + Send + Sync>;

/// The embed job's handler.
pub struct Embedder<V> {
    /// The volumes mounted now, asked once per job.
    volumes: Box<dyn Fn() -> V + Send + Sync>,
    loader: Loader,
    workers: usize,
    on_file: Option<FileHook>,
}

impl<V: Volumes + Sync + 'static> Embedder<V> {
    pub fn new(
        volumes: impl Fn() -> V + Send + Sync + 'static,
        loader: impl Fn() -> Result<Arc<AudioModel>, ModelError> + Send + Sync + 'static,
    ) -> Embedder<V> {
        Embedder {
            volumes: Box::new(volumes),
            loader: Box::new(loader),
            workers: default_workers(),
            on_file: None,
        }
    }

    /// Calls `hook` with each file id as a thread takes it, before it
    /// listens.
    #[cfg(test)]
    pub(crate) fn on_file(mut self, hook: impl Fn(i64) + Send + Sync + 'static) -> Self {
        self.on_file = Some(Box::new(hook));
        self
    }

    /// How many worker threads. At least 1.
    #[cfg(test)]
    pub(crate) fn workers(mut self, workers: usize) -> Self {
        self.workers = workers.max(1);
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
    /// The next index into `ids` to take.
    next: AtomicUsize,
    /// How many files are finished (listened to, failed or skipped).
    done: AtomicUsize,
    /// Set when the run should end: an error, the switch off, made way.
    stop: AtomicBool,
    made_way: AtomicBool,
    error: Mutex<Option<JobError>>,
}

impl<V> Run<'_, V> {
    fn end_with(&self, error: JobError) {
        let mut slot = self.error.lock().unwrap_or_else(|e| e.into_inner());
        slot.get_or_insert(error);
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Whether a thread may take its next file.
enum Go {
    Yes,
    /// The switch is off.
    Off,
    /// A job of a higher priority is queued.
    MakeWay,
}

impl<V: Volumes + Sync + 'static> JobHandler for Embedder<V> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        // Off: nothing runs and the model is never loaded.
        if !job
            .writer()
            .call(|c| read_feature_enabled(c, Feature::AudioModel))?
        {
            return Ok(());
        }
        let folders: HashMap<MusicFolderId, StoredFolder> = job
            .writer()
            .call(|c| folders::stored(c))?
            .into_iter()
            .map(|f| (f.id, f))
            .collect();
        let volumes = (self.volumes)();
        let reachable: Vec<i64> = folders
            .values()
            .filter(|f| f.is_reachable(&volumes))
            .map(|f| f.id.0)
            .collect();
        let ids = job
            .writer()
            .call(move |c| ledger::due_ids(c, Some(&reachable)))?;
        let gate = job.writer().call(|c| ReadGate::for_job(c))?;
        job.progress(0.0)?;
        if ids.is_empty() {
            return job.progress(1.0);
        }
        let model = (self.loader)()
            .map_err(|e| JobError::failed(format!("the audio model wasn't loaded: {e}")))?;
        let run = Run {
            job,
            gate,
            volumes,
            folders,
            ids,
            next: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            made_way: AtomicBool::new(false),
            error: Mutex::new(None),
        };
        let threads = self.workers.min(run.ids.len()).max(1);
        thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| {
                    // The thread's own, so lowering its priority ends with it.
                    lower_priority();
                    let ended = panic::catch_unwind(AssertUnwindSafe(|| self.work(&run, &model)));
                    if ended.is_err() {
                        run.end_with(JobError::failed("an audio model thread panicked"));
                    }
                });
            }
        });
        if let Some(error) = run.error.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return Err(error);
        }
        job.check_cancelled()?;
        if run.made_way.load(Ordering::SeqCst) {
            crate::scan::chain::queue_once(job.writer(), embed_job(), |j| job.enqueue(j))?;
            return Ok(());
        }
        job.progress(1.0)
    }
}

impl<V: Volumes + Sync + 'static> Embedder<V> {
    /// One thread: takes files until none is left or the run stops.
    fn work(&self, run: &Run<'_, V>, model: &AudioModel) {
        while !run.stop.load(Ordering::SeqCst) {
            match self.ready(run) {
                Ok(Go::Yes) => {}
                Ok(Go::Off) => {
                    run.stop.store(true, Ordering::SeqCst);
                    return;
                }
                Ok(Go::MakeWay) => {
                    run.made_way.store(true, Ordering::SeqCst);
                    run.stop.store(true, Ordering::SeqCst);
                    return;
                }
                Err(e) => return run.end_with(e),
            }
            let at = run.next.fetch_add(1, Ordering::SeqCst);
            let Some(&id) = run.ids.get(at) else {
                return;
            };
            if let Some(hook) = &self.on_file {
                hook(id);
            }
            if let Err(e) = self.one(run, model, id) {
                return run.end_with(e);
            }
            let done = run.done.fetch_add(1, Ordering::SeqCst) + 1;
            if let Err(e) = run.job.progress(done as f64 / run.ids.len() as f64) {
                return run.end_with(e);
            }
        }
    }

    /// Waits while a job of a higher priority runs; says whether to go on.
    fn ready(&self, run: &Run<'_, V>) -> Result<Go, JobError> {
        loop {
            run.job.check_cancelled()?;
            let (on, queued, running) = run.job.writer().call(|c| {
                let on = read_feature_enabled(c, Feature::AudioModel)?;
                let above = |status: &str| -> rusqlite::Result<bool> {
                    c.query_row(
                        "SELECT EXISTS (SELECT 1 FROM job WHERE status = ?1 AND priority > ?2)",
                        rusqlite::params![status, PRIORITY.0],
                        |r| r.get(0),
                    )
                };
                Ok((on, above("queued")?, above("running")?))
            })?;
            if !on {
                return Ok(Go::Off);
            }
            if queued {
                return Ok(Go::MakeWay);
            }
            if !running {
                return Ok(Go::Yes);
            }
            if run.stop.load(Ordering::SeqCst) {
                return Ok(Go::Off);
            }
            thread::sleep(PAUSE_POLL);
        }
    }

    /// Listens to file `id`, if it's still due, and records how it went.
    fn one(&self, run: &Run<'_, V>, model: &AudioModel, id: i64) -> Result<(), JobError> {
        let job = run.job;
        let Some(due) = job.writer().call(move |c| ledger::due(c, id))? else {
            return Ok(());
        };
        let Some(path) = run.path(&due.folder, &due.rel_path) else {
            return Ok(());
        };
        // OneDrive (ROADMAP 1.1): an online-only file is never opened unless
        // the user opted in, since reading it downloads it.
        if !run.gate.allows(due.online_only) {
            return Ok(());
        }
        match run.gate.may_open(&path) {
            Ok(true) => {}
            Ok(false) | Err(_) => return Ok(()),
        }
        let file = match File::open(&path) {
            Ok(file) => file,
            // Gone since the walk: the next walk says so.
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
            Err(_) => return self.store(run, &due, Outcome::Failed(Reason::Unreadable)),
        };
        if !unchanged(&file, &due) {
            return Ok(());
        }
        let Ok(after) = file.try_clone() else {
            return Ok(());
        };
        drop(file);
        let keep_going = || !job.is_cancelled() && !run.stop.load(Ordering::SeqCst);
        let heard = panic::catch_unwind(AssertUnwindSafe(|| model.listen(&path, &keep_going)));
        let outcome = match heard {
            Ok(Ok(listening)) => Outcome::Heard {
                fingerprint: listening.fingerprint,
                scores: subset::pick(&listening.scores),
            },
            Ok(Err(ListenError::Cancelled)) => {
                // The job's cancel writes nothing for this file; the run
                // ending for another reason leaves it for the next run.
                job.check_cancelled()?;
                return Ok(());
            }
            Ok(Err(e)) => Outcome::Failed(match e {
                ListenError::Unreadable(_) => Reason::Unreadable,
                ListenError::NoAudio => Reason::NoAudio,
                ListenError::Unsupported | ListenError::ImpossibleRate(_) => Reason::Unsupported,
                ListenError::Damaged => Reason::Damaged,
                ListenError::Model(_) => Reason::ModelFailed,
                ListenError::Cancelled => unreachable!("handled above"),
            }),
            Err(_) => Outcome::Failed(Reason::DecoderCrashed),
        };
        // Rewritten while it was read: leave it for the next walk.
        if !unchanged(&after, &due) {
            return Ok(());
        }
        self.store(run, &due, outcome)
    }

    fn store(
        &self,
        run: &Run<'_, V>,
        due: &crate::quality::ledger::Due,
        outcome: Outcome,
    ) -> Result<(), JobError> {
        let due = due.clone();
        run.job
            .writer()
            .call(move |c| ledger::record(c, &due, &outcome))?;
        Ok(())
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
