//! The fingerprint job: stage 3 of the scan (ROADMAP 1.1, 1.4).
//!
//! One job works through every present file that's due (see [`ledger`]),
//! or the files its target names, on a few threads of its own, at
//! background priority. Files named through [`FirstUp::raise`] (the
//! tracks on screen) jump the line: every running fingerprint job takes
//! them next, newest request first.
//!
//! Per file: if only its tags changed, carry its fingerprint forward
//! without opening it ([`ledger::carry_forward`]). Otherwise resolve its
//! `\\?\` path, skip it if the volume is offline, check it may be read
//! (OneDrive), open it read-only, check it's the
//! size and age its row says, decode and fingerprint it, and store the
//! result through the writer. A file whose content can't be fingerprinted
//! gets a NULL fingerprint and a reason, and isn't tried again until it
//! changes; a decoder panic is caught and counts against that file alone.
//! A file that couldn't be reached or read (offline, locked, an I/O error)
//! gets nothing recorded, so it's tried again next time.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use super::decode::{self, Stopped, Unfingerprintable};
use super::ledger::{self, Due};
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::paths::{RelPath, Volumes};
use crate::scan::folders::{self, StoredFolder};
use crate::scan::walk::nanos;
use crate::scan::{MusicFolderId, ReadGate};
use crate::scan_state::Outcome;

/// A background fingerprint job over every file that's due, or over
/// `file_ids` only.
pub fn fingerprint_job(file_ids: Option<Vec<i64>>) -> NewJob {
    let job = NewJob::new(JobKind::Fingerprint).priority(Priority::BACKGROUND);
    match file_ids {
        Some(ids) => job.target(serde_json::json!({ "file_ids": ids })),
        None => job,
    }
}

/// The files a job's target names: all due files for no target. `None` if
/// the target isn't one [`fingerprint_job`] makes.
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

/// How many threads all fingerprint jobs together use by default: half the
/// cores, 1 to 4, at below-normal priority, so the app stays responsive
/// (ROADMAP 1.4: background, low priority).
pub fn default_threads() -> usize {
    thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(1)
        .clamp(1, 4)
}

/// "Visible tracks first": files to fingerprint before any other, and the
/// thread budget every fingerprint job shares. Shared by every fingerprint
/// job and the `fingerprint_first` command, and by the read job, which
/// borrows from the same budget (1aC-12), so across both kinds there are
/// never more threads than the budget. Cheap to clone; clones share the
/// line.
#[derive(Clone, Default)]
pub struct FirstUp {
    line: Arc<Mutex<Line>>,
    /// Signalled when threads go back to the budget.
    freed: Arc<Condvar>,
}

#[derive(Default)]
struct Line {
    /// Files asked for first, next one at the front.
    wanted: VecDeque<i64>,
    /// Threads that will still take a file from `wanted`.
    takers: usize,
    /// Files a thread is working on right now, so no two do the same one.
    claimed: HashSet<i64>,
    /// Threads taken from the budget by every job on this line: the
    /// fingerprint jobs' and the read job's borrowed ones.
    busy: usize,
}

/// How often a job waiting for threads checks whether it was cancelled.
const THREAD_POLL: Duration = Duration::from_millis(100);

impl FirstUp {
    fn line(&self) -> MutexGuard<'_, Line> {
        self.line.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Puts `ids` at the front of the line, in the order given, ahead of
    /// any asked for earlier. Returns whether a running fingerprint job
    /// will take them; if not, start one.
    pub fn raise(&self, ids: &[i64]) -> bool {
        let mut line = self.line();
        let raised: HashSet<i64> = ids.iter().copied().collect();
        line.wanted.retain(|id| !raised.contains(id));
        let mut seen = HashSet::new();
        for &id in ids.iter().rev() {
            if seen.insert(id) {
                line.wanted.push_front(id);
            }
        }
        line.takers > 0
    }

    /// Waits until one of the `budget` threads shared by every fingerprint
    /// job is free, then takes as many as are free, up to `want`. `None` if
    /// the job is cancelled while it waits.
    fn threads(&self, want: usize, budget: usize, job: &JobContext) -> Option<Threads<'_>> {
        let mut line = self.line();
        loop {
            if job.is_cancelled() {
                return None;
            }
            let free = budget.saturating_sub(line.busy);
            if free > 0 {
                let n = free.min(want.max(1));
                line.busy += n;
                return Some(Threads { first: self, n });
            }
            line = self
                .freed
                .wait_timeout(line, THREAD_POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Takes up to `want` of the `budget` threads that are free right now,
    /// without waiting: `None` if none is. For a job that has a thread of
    /// its own and only borrows more when they're free (the read job,
    /// 1aC-12), so it never waits behind a long fingerprint run.
    pub(crate) fn try_threads(&self, want: usize, budget: usize) -> Option<Threads<'_>> {
        let n = self.take_free(want, budget);
        (n > 0).then_some(Threads { first: self, n })
    }

    /// Takes up to `want` of the `budget` threads that are free right now,
    /// without waiting, as a bare count to give back with
    /// [`FirstUp::give_back`]. A fingerprint job grows into freed threads
    /// mid-run this way (`Fingerprinter::grow`).
    fn take_free(&self, want: usize, budget: usize) -> usize {
        let mut line = self.line();
        let n = budget.saturating_sub(line.busy).min(want);
        line.busy += n;
        n
    }

    /// Returns `n` threads to the budget.
    fn give_back(&self, n: usize) {
        if n == 0 {
            return;
        }
        self.line().busy -= n;
        self.freed.notify_all();
    }

    /// The raised files nobody will take, because every taker is gone. Takes
    /// them off the line, for a new job.
    fn unattended(&self) -> Vec<i64> {
        let mut line = self.line();
        if line.takers > 0 {
            return Vec::new();
        }
        line.wanted.drain(..).collect()
    }

    /// Threads taken from the budget by every job on this line.
    #[cfg(test)]
    pub(crate) fn busy(&self) -> usize {
        self.line().busy
    }

    /// Joins the line as a taker until the returned [`Taker`] is done.
    fn taker(&self) -> Taker<'_> {
        self.line().takers += 1;
        Taker {
            first: self,
            taking: true,
        }
    }
}

/// One thread taking files: the wanted ones first, then its job's own.
struct Taker<'a> {
    first: &'a FirstUp,
    taking: bool,
}

impl Taker<'_> {
    /// Claims the next file: a wanted one if any, else the next of `own`
    /// nobody else has claimed. `None` once both are empty; from then on
    /// this thread is no longer a taker, in the same step, so a raise
    /// either lands before it (and is taken) or sees no taker (and starts
    /// a job).
    fn next(&mut self, own: &Mutex<VecDeque<i64>>) -> Option<(i64, bool)> {
        let mut line = self.first.line();
        while let Some(id) = line.wanted.pop_front() {
            if line.claimed.insert(id) {
                return Some((id, true));
            }
        }
        let mut own = own.lock().unwrap_or_else(PoisonError::into_inner);
        while let Some(id) = own.pop_front() {
            if line.claimed.insert(id) {
                return Some((id, false));
            }
        }
        line.takers -= 1;
        self.taking = false;
        None
    }

    fn release(&self, id: i64) {
        self.first.line().claimed.remove(&id);
    }

    /// Releases `id`, and if it was raised, puts it back at the front of the
    /// line: a job stopping mustn't lose what the screen asked for.
    fn give_back(&self, id: i64, raised: bool) {
        let mut line = self.first.line();
        line.claimed.remove(&id);
        if raised && !line.wanted.contains(&id) {
            line.wanted.push_front(id);
        }
    }
}

/// Threads taken from the shared budget, given back when dropped.
pub(crate) struct Threads<'a> {
    first: &'a FirstUp,
    /// How many were taken.
    pub(crate) n: usize,
}

impl Drop for Threads<'_> {
    fn drop(&mut self) {
        self.first.give_back(self.n);
    }
}

impl Drop for Taker<'_> {
    fn drop(&mut self) {
        if self.taking {
            self.first.line().takers -= 1;
        }
    }
}

/// Test hooks: called with each file id as a thread takes it, and before
/// each packet with the number of packets so far in that file.
type FileHook = Box<dyn Fn(i64) + Send + Sync>;
type PacketHook = Box<dyn Fn(u64) + Send + Sync>;

/// The fingerprint job's handler.
pub struct Fingerprinter<V> {
    /// The volumes mounted now, asked once per job.
    volumes: Box<dyn Fn() -> V + Send + Sync>,
    first: FirstUp,
    threads: usize,
    on_file: Option<FileHook>,
    on_packet: Option<PacketHook>,
}

impl<V: Volumes + Sync + 'static> Fingerprinter<V> {
    pub fn new(
        volumes: impl Fn() -> V + Send + Sync + 'static,
        first: FirstUp,
    ) -> Fingerprinter<V> {
        Fingerprinter {
            volumes: Box::new(volumes),
            first,
            threads: default_threads(),
            on_file: None,
            on_packet: None,
        }
    }

    /// How many files all fingerprint jobs together work on at once. At
    /// least 1.
    pub fn threads(mut self, threads: usize) -> Self {
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
    /// The job's own files, as listed at the start.
    listed: HashSet<i64>,
    /// Files this job has taken, so one raised and also on its own list is
    /// done once.
    taken: Mutex<HashSet<i64>>,
    /// Files finished, and files that make up the whole (the job's own plus
    /// the raised ones it took).
    finished: AtomicUsize,
    total: AtomicUsize,
    /// How far each thread is through its current file, as f64 bits. One
    /// slot per thread the budget allows, so a thread that joins mid-run
    /// has its own.
    partial: Vec<AtomicU64>,
    /// Set when a thread stops with an error, so the others stop too.
    stopped: AtomicBool,
    /// Threads working for this job, the ones that joined mid-run included.
    workers: AtomicUsize,
    /// How each thread ended, collected as they finish.
    results: Mutex<Vec<Result<(), JobError>>>,
}

/// Threads a job took from the budget after it started
/// (`Fingerprinter::grow`), given back when the run ends, however it ends.
struct Grown<'a> {
    first: &'a FirstUp,
    n: AtomicUsize,
}

impl Drop for Grown<'_> {
    fn drop(&mut self) {
        self.first.give_back(self.n.load(Ordering::SeqCst));
    }
}

impl<V: Volumes + Sync + 'static> JobHandler for Fingerprinter<V> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let only = wanted(job.target())
            .ok_or_else(|| JobError::failed("a fingerprint job's target names no files"))?;
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
        let Some(threads) = self.first.threads(own.len(), self.threads, job) else {
            return Err(JobError::Cancelled);
        };
        let result = self.run_on(job, own, folders, gate, threads.n);
        drop(threads);
        if result.is_err() {
            // Raised files this job didn't get to, and no other job will: a
            // new job takes them, so the screen still gets them first.
            let left = self.first.unattended();
            if !left.is_empty() {
                // If even this fails, the next fingerprint job takes them.
                let _ = job.enqueue(fingerprint_job(Some(left)).priority(Priority::USER));
            }
        }
        result
    }
}

impl<V: Volumes + Sync + 'static> Fingerprinter<V> {
    fn run_on(
        &self,
        job: &JobContext,
        own: Vec<i64>,
        folders: HashMap<MusicFolderId, StoredFolder>,
        gate: ReadGate,
        threads: usize,
    ) -> Result<(), JobError> {
        let run = Run {
            job,
            gate,
            volumes: (self.volumes)(),
            folders,
            total: AtomicUsize::new(own.len()),
            listed: own.iter().copied().collect(),
            taken: Mutex::default(),
            own: Mutex::new(own.into()),
            finished: AtomicUsize::new(0),
            partial: (0..self.threads.max(threads))
                .map(|_| AtomicU64::new(0))
                .collect(),
            stopped: AtomicBool::new(false),
            workers: AtomicUsize::new(threads),
            results: Mutex::default(),
        };
        let grown = Grown {
            first: &self.first,
            n: AtomicUsize::new(0),
        };
        job.progress(0.0)?;
        thread::scope(|s| {
            for n in 0..threads {
                self.spawn_worker(s, &run, &grown, n);
            }
        });
        drop(grown);
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

    /// Starts thread `n` of this job in `s`, and collects how it ends.
    fn spawn_worker<'scope, 'env>(
        &'env self,
        s: &'scope thread::Scope<'scope, 'env>,
        run: &'env Run<'env, V>,
        grown: &'env Grown<'env>,
        n: usize,
    ) {
        s.spawn(move || {
            let result = panic::catch_unwind(AssertUnwindSafe(|| self.work(s, run, grown, n)))
                .unwrap_or_else(|_| Err(JobError::failed("a fingerprint thread panicked")));
            if result.is_err() {
                run.stopped.store(true, Ordering::SeqCst);
            }
            run.results
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(result);
        });
    }

    /// Takes whatever the budget has free now, up to this job's share, and
    /// puts a worker on each. A job that started while a read job held
    /// borrowed threads grows into them once the read ends, instead of
    /// running for hours on what was left (1aC-12). Called between files.
    fn grow<'scope, 'env>(
        &'env self,
        s: &'scope thread::Scope<'scope, 'env>,
        run: &'env Run<'env, V>,
        grown: &'env Grown<'env>,
    ) {
        if run.stopped.load(Ordering::SeqCst) {
            return;
        }
        let want = self
            .threads
            .saturating_sub(run.workers.load(Ordering::SeqCst));
        if want == 0 {
            return;
        }
        let n = self.first.take_free(want, self.threads);
        grown.n.fetch_add(n, Ordering::SeqCst);
        for _ in 0..n {
            let m = run.workers.fetch_add(1, Ordering::SeqCst);
            self.spawn_worker(s, run, grown, m);
        }
    }

    /// One thread: takes files until there are none left.
    fn work<'scope, 'env>(
        &'env self,
        s: &'scope thread::Scope<'scope, 'env>,
        run: &'env Run<'env, V>,
        grown: &'env Grown<'env>,
        n: usize,
    ) -> Result<(), JobError> {
        lower_priority();
        let mut taker = self.first.taker();
        while let Some((id, raised)) = taker.next(&run.own) {
            if run.stopped.load(Ordering::SeqCst) {
                taker.give_back(id, raised);
                return Ok(());
            }
            let first_time = run
                .taken
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id);
            if !first_time {
                taker.release(id);
                continue;
            }
            if raised && !run.listed.contains(&id) {
                run.total.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(hook) = &self.on_file {
                hook(id);
            }
            let result = self.one(run, n, id);
            run.partial[n].store(0f64.to_bits(), Ordering::SeqCst);
            if result.is_err() {
                taker.give_back(id, raised);
                return result;
            }
            taker.release(id);
            run.finished.fetch_add(1, Ordering::SeqCst);
            run.report()?;
            self.grow(s, run, grown);
        }
        Ok(())
    }

    /// Fingerprints file `id`, if it's still due, and records how it went.
    fn one(&self, run: &Run<'_, V>, n: usize, id: i64) -> Result<(), JobError> {
        let job = run.job;
        run.job.check_cancelled()?;
        let Some(due) = job.writer().call(move |c| ledger::due(c, id))? else {
            return Ok(());
        };
        // Only the tags changed (size and mtime moved, the audio didn't):
        // the fingerprint stands. Needs no access to the file (1aC-10).
        let carried = {
            let due = due.clone();
            job.writer().call(move |c| ledger::carry_forward(c, &due))?
        };
        if carried {
            return Ok(());
        }
        let Some(path) = run.path(&due) else {
            // Offline, or a row this build can't place: it stays due.
            return Ok(());
        };
        // OneDrive (ROADMAP 1.1): an online-only file is never opened unless
        // the user opted in, since reading it downloads it.
        if !run.gate.allows(due.online_only) {
            return self.record(job, due, Err(Recorded::Skipped(Outcome::online_only())));
        }
        match run.gate.may_open(&path) {
            Ok(true) => {}
            Ok(false) => {
                return self.record(job, due, Err(Recorded::Skipped(Outcome::online_only())))
            }
            // Couldn't tell: don't open it.
            Err(_) => return self.record(job, due, Err(Recorded::Skipped(Outcome::unreachable()))),
        }
        // Locked, gone, or the drive went away: a skip, so it's tried again
        // next time. Only a file whose content was read and can't be
        // fingerprinted is recorded as failed.
        let Ok(file) = File::open(&path) else {
            return self.record(job, due, Err(Recorded::Skipped(Outcome::unreachable())));
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
        let mut keep_going = |fraction: Option<f64>| {
            if let Some(hook) = &self.on_packet {
                hook(packets);
            }
            packets += 1;
            if let Some(fraction) = fraction {
                run.partial[n].store(fraction.to_bits(), Ordering::SeqCst);
                // Only fails once cancelled, which the check below sees.
                let _ = run.report();
            }
            !job.is_cancelled()
        };
        let printed = panic::catch_unwind(AssertUnwindSafe(|| {
            decode::fingerprint(Box::new(file), &mut keep_going)
        }));
        let result = match printed {
            Ok(Ok(fingerprint)) => Ok(fingerprint.to_blob()),
            Ok(Err(Stopped::Cancelled)) => return Err(JobError::Cancelled),
            // Reading stopped partway (e.g. the drive was unplugged).
            Ok(Err(Stopped::Failed(Unfingerprintable::Unreadable))) => {
                Err(Recorded::Skipped(Outcome::unreachable()))
            }
            Ok(Err(Stopped::Failed(why))) => Err(Recorded::Failed(why)),
            Err(_) => Err(Recorded::Failed(Unfingerprintable::DecoderCrashed)),
        };
        // Rewritten while it was decoding: what was read may be a mix of old
        // and new. Leave it for the next walk.
        if !unchanged(&after, &due) {
            return Ok(());
        }
        self.record(job, due, result)
    }

    fn record(
        &self,
        job: &JobContext,
        due: Due,
        result: Result<Vec<u8>, Recorded>,
    ) -> Result<(), JobError> {
        job.writer().call(move |c| match result {
            Ok(blob) => ledger::done(c, &due, &blob).map(|_| ()),
            Err(Recorded::Failed(why)) => ledger::failed(c, &due, why),
            Err(Recorded::Skipped(outcome)) => ledger::skipped(c, &due, outcome),
        })?;
        Ok(())
    }
}

/// A file that ends without a fingerprint, and why.
enum Recorded {
    Failed(Unfingerprintable),
    Skipped(Outcome),
}

impl<V: Volumes> Run<'_, V> {
    /// Where the file is now, as a `\\?\` path; `None` if its volume is
    /// offline or its row can't be placed.
    fn path(&self, due: &Due) -> Option<PathBuf> {
        let folder = self.folders.get(&due.folder)?;
        let rel = RelPath::parse(&due.rel_path).ok()?;
        folder.stored_path().join(&rel).resolve(&self.volumes).ok()
    }

    /// Reports progress: finished files plus each thread's share of its
    /// current one, over all the files.
    fn report(&self) -> Result<(), JobError> {
        let partial: f64 = self
            .partial
            .iter()
            .map(|p| f64::from_bits(p.load(Ordering::SeqCst)))
            .sum();
        let finished = self.finished.load(Ordering::SeqCst) as f64;
        let total = self.total.load(Ordering::SeqCst).max(1) as f64;
        self.job.progress(((finished + partial) / total).min(1.0))
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

/// Runs this thread below normal priority, so fingerprinting (and the read
/// job's borrowed threads) yields to the app and everything else (ROADMAP
/// 1.4). The thread ends with its job.
#[cfg(windows)]
pub(crate) fn lower_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    // SAFETY: GetCurrentThread returns a pseudo-handle to this thread that
    // needs no closing, and SetThreadPriority only reads it. A failure
    // leaves the thread at normal priority, which is harmless.
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

#[cfg(not(windows))]
pub(crate) fn lower_priority() {}
