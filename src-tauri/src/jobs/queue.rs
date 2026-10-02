//! The queue and its worker pool (0E-1, 0E-2, 0E-4).
//!
//! Workers are plain threads, never the DB writer thread. Each takes the
//! highest-priority queued job, runs its kind's [`JobHandler`], and writes
//! through the [`Writer`] like any other caller, so a long job never holds
//! up other writes.
//!
//! One lock (`State`) orders every change a job goes through: enqueue,
//! claim, progress, finish and cancel each take it, then update the
//! database, then the in-memory list of active jobs, then hand a
//! [`JobUpdate`] with the next `seq` to the dispatch thread. So the
//! database, the list and the updates always agree, updates reach the
//! frontend in the order things happened, and a snapshot matches its `seq`.
//!
//! Code running on the writer thread (inside a `Writer::call`) can't use
//! the queue: it gets [`DbError::Reentrant`] at once. Waiting for the lock
//! there could deadlock, because the lock's holder may be waiting for the
//! writer.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::db::{on_a_writer_thread, DbError, Writer};

use super::dispatch::{self, EventSink};
use super::events::{ActivitySnapshot, CancelOutcome, JobUpdate};
use super::model::{JobId, JobKind, JobStatus, NewJob};
use super::store::{self, Claimed, Ending};

/// Progress changes smaller than this aren't stored or sent, so a job can
/// report after every item without flooding the writer or the frontend.
pub const PROGRESS_STEP: f64 = 0.01;

/// How long a worker waits before retrying after the database refused to
/// hand out a job.
const RETRY_AFTER_DB_ERROR: Duration = Duration::from_secs(1);

/// How long [`JobQueue::shutdown`] waits by default for running jobs to stop
/// before it gives up on them, so a stuck job can't hold up closing the app.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// How often shutdown checks whether the workers have stopped.
const SHUTDOWN_POLL: Duration = Duration::from_millis(5);

/// Why a job stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    /// It was cancelled, or the app is closing. Return this as soon as
    /// [`JobContext::check_cancelled`] or [`JobContext::progress`] does.
    Cancelled,
    /// It failed; the message is stored in `job.error`.
    Failed(String),
}

impl JobError {
    /// A failure with `message`.
    pub fn failed(message: impl fmt::Display) -> JobError {
        JobError::Failed(message.to_string())
    }
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobError::Cancelled => f.write_str("cancelled"),
            JobError::Failed(e) => f.write_str(e),
        }
    }
}

impl From<DbError> for JobError {
    fn from(e: DbError) -> Self {
        JobError::failed(e)
    }
}

impl From<rusqlite::Error> for JobError {
    fn from(e: rusqlite::Error) -> Self {
        JobError::failed(e)
    }
}

impl From<std::io::Error> for JobError {
    fn from(e: std::io::Error) -> Self {
        JobError::failed(e)
    }
}

/// The work for one kind of job.
///
/// It runs on a worker thread. It should call [`JobContext::progress`] or
/// [`JobContext::check_cancelled`] often (every file, say) and stop with
/// the error they return, so cancelling is prompt.
pub trait JobHandler: Send + Sync + 'static {
    fn run(&self, job: &JobContext) -> Result<(), JobError>;
}

impl<F> JobHandler for F
where
    F: Fn(&JobContext) -> Result<(), JobError> + Send + Sync + 'static,
{
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        self(job)
    }
}

/// Cancel flag values.
const GO: u8 = 0;
const CANCEL: u8 = 1;
const SHUTDOWN: u8 = 2;

/// A running job, as its handler sees it.
pub struct JobContext {
    id: JobId,
    kind: JobKind,
    target: Option<serde_json::Value>,
    flag: Arc<AtomicU8>,
    shared: Arc<Shared>,
    /// The last progress stored, as f64 bits; NaN before the first.
    last_progress: AtomicU64,
}

impl JobContext {
    pub fn id(&self) -> JobId {
        self.id
    }

    pub fn kind(&self) -> JobKind {
        self.kind
    }

    /// What the job works on, as given to [`NewJob::target`].
    pub fn target(&self) -> Option<&serde_json::Value> {
        self.target.as_ref()
    }

    /// The database writer. Write through it like any other caller; the
    /// handler itself runs off the writer thread.
    pub fn writer(&self) -> &Writer {
        &self.shared.writer
    }

    /// True once the job has been cancelled or the app is closing.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst) != GO
    }

    /// `Err(JobError::Cancelled)` once the job should stop.
    pub fn check_cancelled(&self) -> Result<(), JobError> {
        if self.is_cancelled() {
            Err(JobError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Reports how far along the job is, from 0 to 1, and checks for
    /// cancellation. Progress only goes forward: a value at or below the
    /// last one is ignored, and so is a step under [`PROGRESS_STEP`], except
    /// reaching 1. Safe to call from several threads at once.
    pub fn progress(&self, fraction: f64) -> Result<(), JobError> {
        self.check_cancelled()?;
        if fraction.is_nan() {
            return Ok(());
        }
        let fraction = fraction.clamp(0.0, 1.0);
        // Claim the step with one atomic swap, so of two threads reporting
        // at once only one goes on with a given step.
        let claimed = self
            .last_progress
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |bits| {
                let last = f64::from_bits(bits);
                let forward = last.is_nan()
                    || (fraction > last && (fraction - last >= PROGRESS_STEP || fraction == 1.0));
                forward.then_some(fraction.to_bits())
            });
        if claimed.is_err() {
            return Ok(());
        }
        #[cfg(test)]
        test_hooks::before_progress_lock();
        let id = self.id;
        let mut state = self.shared.lock()?;
        // Two threads can claim steps in one order and take the lock in the
        // other; the later step may already be stored. Never go back to it.
        let stored = state.active.get(&id).and_then(|job| job.progress);
        if stored.is_some_and(|stored| stored >= fraction) {
            return Ok(());
        }
        self.shared
            .writer
            .call(move |c| store::set_progress(c, id, fraction))?;
        if let Some(job) = state.active.get(&id) {
            let mut job = job.clone();
            job.progress = Some(fraction);
            self.shared.send(&mut state, job);
        }
        Ok(())
    }

    /// Enqueues a follow-up job, e.g. a scan asking for hashes.
    pub fn enqueue(&self, job: NewJob) -> Result<JobId, JobError> {
        Ok(self.shared.enqueue(job)?)
    }
}

/// Sets up a [`JobQueue`].
pub struct JobQueueBuilder {
    writer: Writer,
    handlers: HashMap<JobKind, Arc<dyn JobHandler>>,
    workers: usize,
    sink: EventSink,
    shutdown_timeout: Duration,
}

impl JobQueueBuilder {
    /// Runs `kind` jobs with `handler`. A job whose kind has no handler
    /// fails when a worker takes it.
    pub fn handler(mut self, kind: JobKind, handler: impl JobHandler) -> Self {
        self.handlers.insert(kind, Arc::new(handler));
        self
    }

    /// How many jobs can run at once. At least 1.
    pub fn workers(mut self, workers: usize) -> Self {
        self.workers = workers.max(1);
        self
    }

    /// How long [`JobQueue::shutdown`] waits for running jobs to stop.
    /// [`SHUTDOWN_TIMEOUT`] unless set.
    pub fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Where job updates go, in batches (see `dispatch`).
    pub fn on_updates(mut self, sink: impl Fn(&[JobUpdate]) + Send + 'static) -> Self {
        self.sink = Box::new(sink);
        self
    }

    /// Loads the jobs already queued in the database and starts the
    /// workers, which begin taking jobs at once.
    pub fn start(self) -> Result<JobQueue, DbError> {
        let queued = self.writer.call(|c| store::queued(c))?;
        let active = queued
            .into_iter()
            .filter_map(|job| Some((job.id, update(&job, JobStatus::Queued, None)?)))
            .collect();
        let outbox = dispatch::start(self.sink).map_err(DbError::Spawn)?;
        let shared = Arc::new(Shared {
            writer: self.writer,
            handlers: self.handlers,
            outbox,
            stopping: AtomicBool::new(false),
            state: Mutex::new(State {
                seq: 0,
                active,
                running: HashMap::new(),
            }),
            wake: Condvar::new(),
        });
        let mut workers = Vec::with_capacity(self.workers);
        for n in 0..self.workers {
            let worker_shared = shared.clone();
            let worker = thread::Builder::new()
                .name(format!("job-worker-{n}"))
                .spawn(move || work(&worker_shared));
            match worker {
                Ok(worker) => workers.push(worker),
                Err(e) => {
                    stop(&shared, workers, self.shutdown_timeout);
                    return Err(DbError::Spawn(e));
                }
            }
        }
        Ok(JobQueue {
            shared,
            workers: Mutex::new(workers),
            shutdown_timeout: self.shutdown_timeout,
        })
    }
}

/// The background job queue (Activity in the UI). Tauri state; reach it as
/// `State<JobQueue>`.
///
/// [`JobQueue::shutdown`] (run when the app exits, and on drop) stops the
/// workers: running jobs are asked to stop and go back in the queue for the
/// next launch, as if they had never started. It waits at most
/// [`SHUTDOWN_TIMEOUT`] for them.
///
/// Keep jobs coarse: one per folder or per batch of files, not one per
/// file. Each job is a row, a claim and a few updates, and the Activity
/// status lists every queued and running job.
pub struct JobQueue {
    shared: Arc<Shared>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    shutdown_timeout: Duration,
}

impl JobQueue {
    /// A queue writing through `writer`, with no handlers, as many workers
    /// as [`default_workers`] and updates going nowhere.
    pub fn builder(writer: Writer) -> JobQueueBuilder {
        JobQueueBuilder {
            writer,
            handlers: HashMap::new(),
            workers: default_workers(),
            sink: Box::new(|_| {}),
            shutdown_timeout: SHUTDOWN_TIMEOUT,
        }
    }

    /// Stores `job` as queued and wakes a worker. Returns its id.
    pub fn enqueue(&self, job: NewJob) -> Result<JobId, DbError> {
        self.shared.enqueue(job)
    }

    /// Cancels a job. A queued job is cancelled at once; a running one is
    /// told to stop and is marked cancelled when its handler returns.
    pub fn cancel(&self, id: JobId) -> Result<CancelOutcome, DbError> {
        let mut state = self.shared.lock()?;
        if let Some(flag) = state.running.get(&id) {
            // Don't turn a shutdown into a user's cancel, or back.
            let _ = flag.compare_exchange(GO, CANCEL, Ordering::SeqCst, Ordering::SeqCst);
            return Ok(CancelOutcome::Stopping);
        }
        if !self
            .shared
            .writer
            .call(move |c| store::cancel_queued(c, id))?
        {
            return Ok(CancelOutcome::NotActive);
        }
        if let Some(mut job) = state.active.remove(&id) {
            job.status = JobStatus::Cancelled;
            self.shared.send(&mut state, job);
        }
        Ok(CancelOutcome::Cancelled)
    }

    /// Every queued and running job right now.
    pub fn activity(&self) -> Result<ActivitySnapshot, DbError> {
        let state = self.shared.lock()?;
        let mut jobs: Vec<JobUpdate> = state.active.values().cloned().collect();
        jobs.sort_by_key(|j| {
            (
                j.status != JobStatus::Running,
                std::cmp::Reverse(j.priority),
                j.id,
            )
        });
        Ok(ActivitySnapshot {
            seq: state.seq,
            jobs,
        })
    }

    /// Stops the workers and waits for them. Running jobs are asked to
    /// stop and go back in the queue.
    ///
    /// Only the first call waits. A later call returns at once, even while
    /// the first is still waiting for the workers to stop.
    ///
    /// It waits at most the shutdown timeout ([`SHUTDOWN_TIMEOUT`] unless
    /// the builder set one), then leaves any job still running behind, on
    /// its own thread. If that job finishes before the process exits, it's
    /// recorded as usual (done, failed, or back in the queue); if the process
    /// exits first, its row stays `running` until the next start deals with it (`store::recover_interrupted`).
    ///
    /// Called from the database writer thread or from one of this queue's
    /// workers, it can't wait (that would deadlock, or join the calling
    /// thread), so it returns at once and a helper thread stops the workers.
    pub fn shutdown(&self) {
        let workers =
            std::mem::take(&mut *self.workers.lock().unwrap_or_else(PoisonError::into_inner));
        if workers.is_empty() {
            return;
        }
        let here = thread::current().id();
        let reentrant = on_a_writer_thread() || workers.iter().any(|w| w.thread().id() == here);
        if !reentrant {
            stop(&self.shared, workers, self.shutdown_timeout);
            return;
        }
        let shared = self.shared.clone();
        let timeout = self.shutdown_timeout;
        let helper = thread::Builder::new()
            .name("job-shutdown".into())
            .spawn(move || stop(&shared, workers, timeout));
        if let Err(e) = helper {
            eprintln!("job queue: could not start the shutdown thread: {e}");
            // The workers can't be waited for, but they still stop taking
            // jobs; they end with the process.
            signal_stop(&self.shared);
        }
    }
}

impl Drop for JobQueue {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// How many workers a queue gets by default: one per core, 2 to 4.
pub fn default_workers() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(2, 4)
}

struct Shared {
    writer: Writer,
    handlers: HashMap<JobKind, Arc<dyn JobHandler>>,
    /// To the dispatch thread, which sends updates on to the sink.
    outbox: mpsc::Sender<JobUpdate>,
    /// Set once the queue is shutting down; workers take no more jobs.
    /// Atomic, so it can be set without the lock (see [`signal_stop`]).
    stopping: AtomicBool,
    state: Mutex<State>,
    /// Signalled when a job is queued or the queue is shutting down.
    wake: Condvar,
}

struct State {
    /// The `seq` of the last update sent.
    seq: u64,
    /// Every queued and running job this build knows the kind of.
    active: BTreeMap<JobId, JobUpdate>,
    /// Each running job's cancel flag.
    running: HashMap<JobId, Arc<AtomicU8>>,
}

impl Shared {
    /// Takes the lock, unless this is the writer thread (see the module
    /// docs).
    fn lock(&self) -> Result<MutexGuard<'_, State>, DbError> {
        if on_a_writer_thread() {
            return Err(DbError::Reentrant);
        }
        Ok(self.lock_worker())
    }

    /// Takes the lock from a worker or the queue's owner, which are never
    /// the writer thread.
    fn lock_worker(&self) -> MutexGuard<'_, State> {
        // Nothing that runs under the lock can panic halfway through a
        // change, so a poisoned lock's state is still whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stamps `job` with the next `seq`, records it and sends it. Finished
    /// jobs leave the active list.
    fn send(&self, state: &mut State, mut job: JobUpdate) {
        state.seq += 1;
        job.seq = state.seq;
        if job.status.is_finished() {
            state.active.remove(&job.id);
        } else {
            state.active.insert(job.id, job.clone());
        }
        // Only fails once the dispatch thread is gone, i.e. never while
        // the queue is alive.
        let _ = self.outbox.send(job);
    }

    fn enqueue(&self, job: NewJob) -> Result<JobId, DbError> {
        let mut state = self.lock()?;
        let stored = job.clone();
        let id = self.writer.call(move |c| store::insert(c, &stored))?;
        let queued = JobUpdate {
            seq: 0,
            id,
            kind: job.kind,
            status: JobStatus::Queued,
            progress: None,
            priority: job.priority.0,
        };
        self.send(&mut state, queued);
        self.wake.notify_one();
        Ok(id)
    }
}

/// The update for a job the database just handed back, or `None` if this
/// build doesn't know its kind.
fn update(job: &Claimed, status: JobStatus, progress: Option<f64>) -> Option<JobUpdate> {
    Some(JobUpdate {
        seq: 0,
        id: job.id,
        kind: JobKind::parse(&job.kind)?,
        status,
        progress,
        priority: job.priority.0,
    })
}

/// Tells the workers to stop, asks running jobs to stop, and waits for
/// them, for at most `timeout`. Never call it on the writer thread or on one
/// of `workers` (see [`JobQueue::shutdown`]).
fn stop(shared: &Shared, workers: Vec<JoinHandle<()>>, timeout: Duration) {
    {
        // Under the lock, so no worker is between checking `stopping` and
        // waiting: every one of them hears the wake-up.
        let state = shared.lock_worker();
        shared.stopping.store(true, Ordering::SeqCst);
        ask_running_jobs_to_stop(&state);
        shared.wake.notify_all();
    }
    // A thread can't be joined with a time limit, so wait until each has
    // finished, then join it (which returns at once).
    let deadline = Instant::now() + timeout;
    let mut stuck = 0;
    for worker in workers {
        while !worker.is_finished() && Instant::now() < deadline {
            thread::sleep(SHUTDOWN_POLL);
        }
        if worker.is_finished() {
            let _ = worker.join();
        } else {
            // Dropping the handle lets the thread run on, detached.
            stuck += 1;
        }
    }
    if stuck > 0 {
        eprintln!("job queue: {stuck} job(s) still running after {timeout:?}; leaving them");
    }
}

/// Asks every running job to stop because the queue is shutting down.
fn ask_running_jobs_to_stop(state: &State) {
    for flag in state.running.values() {
        // Don't turn a user's cancel into a shutdown.
        let _ = flag.compare_exchange(GO, SHUTDOWN, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// Stops the workers taking jobs without waiting for anything, for when
/// [`stop`] can't run. Safe on any thread, the writer's included: it never
/// waits for the lock. Running jobs are asked to stop only if the lock is
/// free. A worker waiting for a job may not wake until the next enqueue,
/// but then it stops instead of taking the job.
fn signal_stop(shared: &Shared) {
    shared.stopping.store(true, Ordering::SeqCst);
    match shared.state.try_lock() {
        Ok(state) => ask_running_jobs_to_stop(&state),
        Err(TryLockError::Poisoned(state)) => ask_running_jobs_to_stop(&state.into_inner()),
        Err(TryLockError::WouldBlock) => {}
    }
    shared.wake.notify_all();
}

/// A worker: takes jobs until the queue shuts down.
fn work(shared: &Arc<Shared>) {
    while let Some((job, flag)) = next(shared) {
        run(shared, job, flag);
    }
}

/// Waits for the next job and marks it running. `None` once the queue is
/// shutting down or the database is gone.
fn next(shared: &Shared) -> Option<(Claimed, Arc<AtomicU8>)> {
    let mut state = shared.lock_worker();
    loop {
        if shared.stopping.load(Ordering::SeqCst) {
            return None;
        }
        match shared.writer.call(|c| store::claim_next(c)) {
            Ok(Some(job)) => {
                let flag = Arc::new(AtomicU8::new(GO));
                state.running.insert(job.id, flag.clone());
                if let Some(running) = update(&job, JobStatus::Running, None) {
                    shared.send(&mut state, running);
                }
                return Some((job, flag));
            }
            Ok(None) => {
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            Err(DbError::WriterGone) => return None,
            Err(_) => {
                state = shared
                    .wake
                    .wait_timeout(state, RETRY_AFTER_DB_ERROR)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }
    }
}

/// Runs one claimed job to its end and records how it ended.
fn run(shared: &Arc<Shared>, job: Claimed, flag: Arc<AtomicU8>) {
    let id = job.id;
    let priority = job.priority;
    let kind = JobKind::parse(&job.kind);
    let ending = match kind {
        None => Ending::Failed(format!("unknown job kind {:?}", job.kind)),
        Some(kind) => match shared.handlers.get(&kind) {
            None => Ending::Failed(format!("no handler for {kind} jobs")),
            Some(handler) => {
                let context = JobContext {
                    id,
                    kind,
                    target: job.target,
                    flag: flag.clone(),
                    shared: shared.clone(),
                    last_progress: AtomicU64::new(f64::NAN.to_bits()),
                };
                match panic::catch_unwind(AssertUnwindSafe(|| handler.run(&context))) {
                    Ok(Ok(())) => Ending::Done,
                    Ok(Err(JobError::Failed(e))) => Ending::Failed(e),
                    Ok(Err(JobError::Cancelled)) if flag.load(Ordering::SeqCst) == SHUTDOWN => {
                        Ending::Requeued
                    }
                    Ok(Err(JobError::Cancelled)) => Ending::Cancelled,
                    Err(panic) => Ending::Failed(format!(
                        "the job panicked: {}",
                        panic_message(panic.as_ref())
                    )),
                }
            }
        },
    };

    // Under the lock, so the database and the active list change together:
    // nobody sees the job finished in one and running in the other.
    let mut state = shared.lock_worker();
    let stored = ending.clone();
    // If this fails the row stays `running`; the next start finds it (`store::recover_interrupted`).
    let _ = shared.writer.call(move |c| store::finish(c, id, &stored));
    state.running.remove(&id);
    let Some(kind) = kind else { return };
    let progress = state.active.get(&id).and_then(|j| j.progress);
    let (status, progress) = match ending {
        Ending::Done => (JobStatus::Done, Some(1.0)),
        Ending::Failed(_) => (JobStatus::Failed, progress),
        Ending::Cancelled => (JobStatus::Cancelled, progress),
        Ending::Requeued => (JobStatus::Queued, None),
    };
    let finished = JobUpdate {
        seq: 0,
        id,
        kind,
        status,
        progress,
        priority: priority.0,
    };
    shared.send(&mut state, finished);
}

/// The text a panic was raised with, if it has one.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    if let Some(s) = panic.downcast_ref::<&str>() {
        s
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s
    } else {
        "no message"
    }
}

/// [`signal_stop`] for tests: the path taken when shutdown can't start its
/// helper thread, which a test can't make fail.
#[cfg(test)]
pub(super) fn signal_stop_for_tests(queue: &JobQueue) {
    signal_stop(&queue.shared);
}

/// Pause points for tests that need two threads in a set order.
#[cfg(test)]
pub(super) mod test_hooks {
    use std::cell::RefCell;

    thread_local! {
        static BEFORE_PROGRESS_LOCK: RefCell<Option<Box<dyn FnOnce()>>> =
            RefCell::new(None);
    }

    /// Runs `hook` on this thread the next time a progress report has
    /// claimed its step and is about to take the queue's lock.
    pub fn pause_before_progress_lock(hook: impl FnOnce() + 'static) {
        BEFORE_PROGRESS_LOCK.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
    }

    pub(super) fn before_progress_lock() {
        if let Some(hook) = BEFORE_PROGRESS_LOCK.with(|h| h.borrow_mut().take()) {
            hook();
        }
    }
}
