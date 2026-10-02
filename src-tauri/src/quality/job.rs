//! The quality job: measures each file's cutoff, durations and decode
//! errors (ROADMAP 1.6; 1bA-10, 1bA-11).
//!
//! One job works through every present file that's due (see [`ledger`]),
//! or the files its target names, on threads taken from the fingerprint
//! jobs' shared budget ([`FirstUp`]) at below-normal priority, so the app
//! stays responsive and the two kinds of job never use more threads
//! together than the budget allows.
//!
//! Per file: resolve its `\\?\` path, skip it if its volume is offline,
//! check it may be read (OneDrive), open it read-only (the same path and
//! the same checks the fingerprint job uses), check it's the size and age
//! its row says, decode it once ([`measure`]) and store the result through
//! the writer. A file whose content can't be decoded gets a row saying
//! why, and isn't tried again until its audio changes; a decoder panic
//! counts against that file alone. A file that couldn't be reached or
//! read (offline, locked, an I/O error) gets nothing recorded, so it's
//! tried again next time. A cancel stops mid-file and writes nothing for
//! it.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use super::ledger::{self, Due, Row};
use super::measure::{measure, Failure};
use crate::fingerprint::decode::Stopped;
use crate::fingerprint::{default_threads, lower_priority, FirstUp};
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::paths::{RelPath, Volumes};
use crate::scan::folders::{self, StoredFolder};
use crate::scan::walk::nanos;
use crate::scan::{MusicFolderId, ReadGate};

/// A background quality job over every file that's due, or over
/// `file_ids` only.
pub fn quality_job(file_ids: Option<Vec<i64>>) -> NewJob {
    let job = NewJob::new(JobKind::Quality).priority(Priority::BACKGROUND);
    match file_ids {
        Some(ids) => job.target(serde_json::json!({ "file_ids": ids })),
        None => job,
    }
}

/// The files a job's target names: all due files for no target. `None` if
/// the target isn't one [`quality_job`] makes.
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

/// How often a job waiting for threads checks whether it was cancelled.
const THREAD_POLL: Duration = Duration::from_millis(100);

/// Test hooks: called with each file id as a thread takes it, and before
/// each packet with the number of packets so far in that file.
type FileHook = Box<dyn Fn(i64) + Send + Sync>;
type PacketHook = Box<dyn Fn(u64) + Send + Sync>;

/// The quality job's handler.
pub struct Qualifier<V> {
    /// The volumes mounted now, asked once per job.
    volumes: Box<dyn Fn() -> V + Send + Sync>,
    first: FirstUp,
    threads: usize,
    on_file: Option<FileHook>,
    on_packet: Option<PacketHook>,
}

impl<V: Volumes + Sync + 'static> Qualifier<V> {
    /// A handler on `first`'s thread budget, which the fingerprint jobs
    /// share.
    pub fn new(volumes: impl Fn() -> V + Send + Sync + 'static, first: FirstUp) -> Qualifier<V> {
        Qualifier {
            volumes: Box::new(volumes),
            first,
            threads: default_threads(),
            on_file: None,
            on_packet: None,
        }
    }

    /// How many threads the budget holds. At least 1.
    #[cfg(test)]
    pub(crate) fn threads(mut self, threads: usize) -> Self {
        self.threads = threads.max(1);
        self
    }

    /// Calls `hook` with each file id as a thread takes it.
    #[cfg(test)]
    pub(crate) fn on_file(mut self, hook: impl Fn(i64) + Send + Sync + 'static) -> Self {
        self.on_file = Some(Box::new(hook));
        self
    }

    /// Calls `hook` before each packet, with the packets read so far.
    #[cfg(test)]
    pub(crate) fn on_packet(mut self, hook: impl Fn(u64) + Send + Sync + 'static) -> Self {
        self.on_packet = Some(Box::new(hook));
        self
    }
}

/// What a job knows while it runs.
struct Run<'a, V> {
    job: &'a JobContext,
    gate: ReadGate,
    volumes: V,
    folders: HashMap<MusicFolderId, StoredFolder>,
    own: Mutex<VecDeque<i64>>,
    finished: AtomicUsize,
    total: usize,
    /// Set when a thread stops with an error, so the others stop too.
    stopped: AtomicBool,
    /// How each thread ended, collected as they finish.
    results: Mutex<Vec<Result<(), JobError>>>,
}

impl<V: Volumes + Sync + 'static> JobHandler for Qualifier<V> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let only = wanted(job.target())
            .ok_or_else(|| JobError::failed("a quality job's target names no files"))?;
        let own = job
            .writer()
            .call(move |c| ledger::due_ids(c, only.as_deref()))?;
        let folders = job
            .writer()
            .call(|c| folders::stored(c))?
            .into_iter()
            .map(|f| (f.id, f))
            .collect();
        // The user's OneDrive opt-in, read once for the whole job.
        let gate = job.writer().call(|c| ReadGate::for_job(c))?;
        if own.is_empty() {
            return job.progress(1.0);
        }
        // Wait for a free thread of the shared budget, unless cancelled.
        let want = own.len().min(self.threads);
        let threads = loop {
            job.check_cancelled()?;
            if let Some(threads) = self.first.try_threads(want, self.threads) {
                break threads;
            }
            thread::sleep(THREAD_POLL);
        };
        let run = Run {
            job,
            gate,
            volumes: (self.volumes)(),
            folders,
            total: own.len(),
            own: Mutex::new(own.into()),
            finished: AtomicUsize::new(0),
            stopped: AtomicBool::new(false),
            results: Mutex::default(),
        };
        job.progress(0.0)?;
        thread::scope(|s| {
            for _ in 0..threads.n {
                s.spawn(|| {
                    let result = panic::catch_unwind(AssertUnwindSafe(|| self.work(&run)))
                        .unwrap_or_else(|_| Err(JobError::failed("a quality thread panicked")));
                    if result.is_err() {
                        run.stopped.store(true, Ordering::SeqCst);
                    }
                    run.results
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(result);
                });
            }
        });
        drop(threads);
        let results =
            std::mem::take(&mut *run.results.lock().unwrap_or_else(PoisonError::into_inner));
        // A failure says more than a cancel another thread saw because of it.
        let mut cancelled = false;
        for result in results {
            match result {
                Err(JobError::Failed(e)) => return Err(JobError::Failed(e)),
                Err(JobError::Cancelled) => cancelled = true,
                Ok(()) => {}
            }
        }
        if cancelled {
            return Err(JobError::Cancelled);
        }
        job.progress(1.0)
    }
}

impl<V: Volumes + Sync + 'static> Qualifier<V> {
    /// One thread: takes files until there are none left.
    fn work(&self, run: &Run<'_, V>) -> Result<(), JobError> {
        lower_priority();
        loop {
            if run.stopped.load(Ordering::SeqCst) {
                return Ok(());
            }
            let next = run
                .own
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            let Some(id) = next else {
                return Ok(());
            };
            if let Some(hook) = &self.on_file {
                hook(id);
            }
            self.one(run, id)?;
            let done = run.finished.fetch_add(1, Ordering::SeqCst) + 1;
            run.job.progress(done as f64 / run.total as f64)?;
        }
    }

    /// Measures file `id`, if it's still due, and records how it went.
    fn one(&self, run: &Run<'_, V>, id: i64) -> Result<(), JobError> {
        let job = run.job;
        job.check_cancelled()?;
        let Some(due) = job.writer().call(move |c| ledger::due(c, id))? else {
            return Ok(());
        };
        let Some(path) = run.path(&due) else {
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
        // A second handle on the same file, to check it again afterwards.
        let Ok(after) = file.try_clone() else {
            return Ok(());
        };

        let mut packets = 0u64;
        let mut keep_going = |_: Option<f64>| {
            if let Some(hook) = &self.on_packet {
                hook(packets);
            }
            packets += 1;
            !job.is_cancelled()
        };
        let measured = panic::catch_unwind(AssertUnwindSafe(|| {
            measure(Box::new(file), &mut keep_going)
        }));
        let row = match measured {
            Ok(Ok(measured)) => Row::Measured(measured),
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

impl<V: Volumes> Run<'_, V> {
    /// Where the file is now, as a `\\?\` path; `None` if its volume is
    /// offline or its row can't be placed.
    fn path(&self, due: &Due) -> Option<PathBuf> {
        let folder = self.folders.get(&due.folder)?;
        let rel = RelPath::parse(&due.rel_path).ok()?;
        folder.stored_path().join(&rel).resolve(&self.volumes).ok()
    }
}

/// Whether the open file is still the size and age its row says.
fn unchanged(file: &File, due: &Due) -> bool {
    let Ok(meta) = file.metadata() else {
        return false;
    };
    // Exactly as the walk stores them.
    let size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
    let mtime = meta.modified().map(nanos).unwrap_or(0);
    (Some(size), Some(mtime)) == (due.size, due.mtime)
}
