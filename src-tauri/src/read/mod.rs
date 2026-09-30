//! Stage 2 of the scan: reading files (1aB-2, 1aB-3, 1aB-4; ROADMAP 1.1,
//! §5.5).
//!
//! For each present file that's due (`scan_state`: never read, changed
//! since its last read, or read by an older [`READ_VERSION`]), one pass
//! reads its tags leniently, its format from the bytes, its codec and audio
//! properties from the audio (never from rekordbox, §5.3), and whether
//! it's truncated or broken. Results go to the file's stage-2 columns
//! (`raw_tags`, `sniffed_format`, `codec`, `bitrate`, `sample_rate`,
//! `duration_ms`, and `quality_verdict` for `truncated` / `broken` only).
//!
//! - A broken tag block never stops a file being indexed: the tag reader
//!   reads around it, and the audio properties still come through.
//! - A file that can't be reached or opened (gone, locked, no permission, an
//!   unplugged drive) is recorded as a skip (`unreachable`), so the next
//!   run tries again; its columns are left as they were. Damaged content is
//!   never a failure: it's a `truncated` or `broken` verdict, and the file
//!   counts as read.
//! - A OneDrive online-only file (reading it would download it) is never
//!   opened unless the user opted in ([`crate::scan::ReadGate`]): one the
//!   walk marked is skipped (`online_only`), and each open is preceded by a
//!   fresh attribute check, so one that went online-only since the walk
//!   isn't opened either.
//! - Files are opened read-only through `\\?\` paths (§5.6). Nothing here
//!   writes to a music file; the only writes go to the database.
//! - **Several files at once** (1aC-12): a cold first read is mostly
//!   waiting, on the disk and on the antivirus scan of each file's first
//!   open, and those waits overlap. The job reads on a thread of its own
//!   and borrows up to three more from the thread budget the fingerprint
//!   jobs share ([`crate::fingerprint::FirstUp`], below-normal priority),
//!   if they're free when it starts; it never waits for them, so a long
//!   fingerprint run only makes it read as it did before. Every rule above
//!   holds per file, whichever thread reads it, and every file's result is
//!   written the same, so a read on four threads writes what a read on one
//!   writes; only the order within a batch differs.
//! - Rows are written in batches, each in one transaction with the batch's
//!   `file_stage` rows. Cancelling stops handing out files at once; batches
//!   already written stay, whole, and the unwritten one is read again next
//!   time.

mod codec;
mod file;

pub use codec::Codec;
pub use file::{FileRead, NotRead, Verdict};

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use tauri::State;

use crate::fingerprint::{default_threads, lower_priority, FirstUp};
use crate::ipc::IpcError;
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, JobQueue, NewJob, Priority};
use crate::paths::{RelPath, Volumes};
use crate::scan::folders::{self, MusicFolderId, StoredFolder};
use crate::scan::ReadGate;
use crate::scan_state::{self, DueFile, Outcome, Recorded, Scope, Stage};

/// The version of what this stage reads. Bump it when a change here should
/// reread every file (a new column, a fixed parser).
pub const READ_VERSION: i64 = 1;

/// A batch is written once it holds this many files…
pub const BATCH_MAX: usize = 500;
/// …or once its first file has waited this long, so results show up
/// quickly even on a slow drive.
pub const BATCH_WINDOW: Duration = Duration::from_millis(500);

/// How often a reading thread with nothing to read checks whether the job
/// has stopped.
const IDLE_POLL: Duration = Duration::from_millis(100);

/// A read of the files in `ids`, or in every music folder if `None`.
pub fn read_job(ids: Option<Vec<MusicFolderId>>) -> NewJob {
    let job = NewJob::new(JobKind::Read).priority(Priority::NORMAL);
    match ids {
        Some(ids) => job.target(serde_json::json!({ "music_folder_ids": ids })),
        None => job,
    }
}

/// The files a read job's target names. `None` if the target isn't one
/// [`read_job`] makes.
fn scope(target: Option<&serde_json::Value>) -> Option<Scope> {
    let Some(target) = target else {
        return Some(Scope::All);
    };
    let ids = target.get("music_folder_ids")?.as_array()?;
    ids.iter()
        .map(|id| id.as_i64())
        .collect::<Option<Vec<_>>>()
        .map(Scope::Folders)
}

/// Reads the files in the music folders `ids`, or in all of them. Returns
/// the job's id; its progress shows in Activity.
#[tauri::command(async)]
#[specta::specta]
pub fn read_files(
    jobs: State<'_, JobQueue>,
    ids: Option<Vec<MusicFolderId>>,
) -> Result<JobId, IpcError> {
    Ok(jobs.enqueue(read_job(ids))?)
}

/// The read job's handler for the app: asks Windows which volumes are
/// mounted at the start of each job, and borrows threads from `first`,
/// the budget the fingerprint jobs share.
pub fn reader(first: FirstUp) -> impl JobHandler {
    Reader::new(crate::scan::system_volumes).sharing(first)
}

/// The read job's handler.
pub struct Reader<V> {
    /// The volumes mounted now, asked once per job.
    volumes: Box<dyn Fn() -> V + Send + Sync>,
    /// Called before each file is read, with how many the job has looked at
    /// so far. Tests use it to act mid-job.
    on_file: Option<FileHook>,
    batch_max: usize,
    window: Duration,
    /// The thread budget shared with the fingerprint jobs.
    first: FirstUp,
    /// At most this many threads read at once, the job's own included.
    threads: usize,
}

/// See [`Reader::on_file`].
type FileHook = Box<dyn Fn(u64) + Send + Sync>;

impl<V: Volumes + Sync + 'static> Reader<V> {
    /// A reader on a budget of its own ([`default_threads`] threads). The
    /// app's shares the fingerprint jobs' instead ([`Reader::sharing`]).
    pub fn new(volumes: impl Fn() -> V + Send + Sync + 'static) -> Reader<V> {
        Reader {
            volumes: Box::new(volumes),
            on_file: None,
            batch_max: BATCH_MAX,
            window: BATCH_WINDOW,
            first: FirstUp::default(),
            threads: default_threads(),
        }
    }

    /// Borrows helper threads from `first`'s budget, which the fingerprint
    /// jobs share, instead of a budget of its own.
    pub fn sharing(mut self, first: FirstUp) -> Self {
        self.first = first;
        self
    }

    /// How many threads may read at once, the job's own included: the
    /// budget. At least 1, which reads one file at a time.
    pub fn threads(mut self, threads: usize) -> Self {
        self.threads = threads.max(1);
        self
    }

    /// Replaces [`BATCH_MAX`] and [`BATCH_WINDOW`], so a test decides when
    /// batches go out.
    #[cfg(test)]
    pub(crate) fn batches(mut self, max: usize, window: Duration) -> Self {
        self.batch_max = max.max(1);
        self.window = window;
        self
    }

    /// Calls `hook` before each file, with the number looked at so far.
    #[cfg(test)]
    pub(crate) fn on_file(mut self, hook: impl Fn(u64) + Send + Sync + 'static) -> Self {
        self.on_file = Some(Box::new(hook));
        self
    }
}

impl<V: Volumes + Sync + 'static> JobHandler for Reader<V> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let scope = scope(job.target())
            .ok_or_else(|| JobError::failed("a read's target names no music folders"))?;
        let folders = job.writer().call(|c| folders::stored(c))?;
        let volumes = (self.volumes)();
        // The OneDrive opt-in, read once for the whole job.
        let gate = job.writer().call(|c| ReadGate::for_job(c))?;
        let count_scope = scope.clone();
        let total = job
            .writer()
            .call(move |c| scan_state::count_due(c, Stage::Read, READ_VERSION, &count_scope))?;
        job.progress(0.0)?;

        // Helpers from the shared budget, if any are free now; never waits.
        let helpers = self
            .first
            .try_threads(self.threads.saturating_sub(1), self.threads);
        let readers = 1 + helpers.as_ref().map_or(0, |t| t.n);
        let run = Run {
            job,
            folders,
            volumes,
            gate,
            readers,
            pending: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            more: AtomicBool::new(true),
            stopped: AtomicBool::new(false),
            taken: AtomicU64::new(0),
        };
        let (results, done) = mpsc::channel();
        let outcome = thread::scope(|s| {
            for n in 0..readers {
                let results = results.clone();
                let run = &run;
                s.spawn(move || {
                    // The job's own thread keeps its priority, as before;
                    // borrowed ones yield like fingerprinting does.
                    if n > 0 {
                        lower_priority();
                    }
                    self.read_files(run, &results);
                });
            }
            drop(results);
            let outcome = self.feed_and_write(&run, &done, &scope, total);
            // Whatever the outcome, the readers stop: nothing more is
            // handed out, and a file they were on is finished, not written.
            run.stop();
            outcome
        });
        drop(helpers);
        outcome
    }
}

impl<V: Volumes + Sync + 'static> Reader<V> {
    /// One reading thread: takes files until there are none left, and
    /// sends each result to the writer.
    fn read_files(&self, run: &Run<'_, V>, results: &Sender<Option<Done>>) {
        while let Some(due) = run.next() {
            if let Some(hook) = &self.on_file {
                hook(run.taken.fetch_add(1, Ordering::SeqCst));
            }
            let result = read_due(&run.folders, &run.volumes, run.gate, &due);
            // The writer is gone only once the job is over.
            if results.send(result).is_err() {
                return;
            }
        }
    }

    /// The job's own thread, while the readers read: keeps them fed a page
    /// at a time, and writes what comes back in batches.
    fn feed_and_write(
        &self,
        run: &Run<'_, V>,
        done: &Receiver<Option<Done>>,
        scope: &Scope,
        total: u64,
    ) -> Result<(), JobError> {
        let job = run.job;
        let mut batch = Batch::new(self.batch_max, self.window);
        let mut looked_at = 0u64;
        let mut after = 0i64;
        // Files handed out whose result hasn't come back.
        let mut outstanding = 0usize;
        let mut exhausted = false;
        loop {
            job.check_cancelled()?;
            // Another page once the readers have nearly caught up, so they
            // never wait on the database.
            if !exhausted && run.pending().len() <= run.readers_worth() {
                let page_scope = scope.clone();
                let limit = self.batch_max;
                let page = job.writer().call(move |c| {
                    scan_state::due(c, Stage::Read, READ_VERSION, &page_scope, after, limit)
                })?;
                match page.last() {
                    None => {
                        exhausted = true;
                        run.more.store(false, Ordering::SeqCst);
                    }
                    Some(last) => {
                        after = last.id;
                        outstanding += page.len();
                        run.pending().extend(page);
                    }
                }
                run.wake.notify_all();
            }
            if exhausted && outstanding == 0 {
                break;
            }
            // Wait for a result, but never past the batch window.
            let timeout = if batch.rows.is_empty() {
                self.window
            } else {
                self.window
                    .saturating_sub(batch.since.elapsed())
                    .max(Duration::from_millis(1))
            };
            match done.recv_timeout(timeout) {
                Ok(result) => {
                    outstanding -= 1;
                    looked_at += 1;
                    if let Some(done) = result {
                        batch.push(done);
                    }
                    if total > 0 {
                        job.progress(looked_at as f64 / total as f64)?;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(JobError::failed("the reading threads stopped early"));
                }
            }
            if batch.due() {
                batch.flush(job)?;
            }
        }
        batch.flush(job)?;
        job.progress(1.0)
    }
}

/// One job's run, shared by its reading threads.
struct Run<'a, V> {
    job: &'a JobContext,
    folders: Vec<StoredFolder>,
    volumes: V,
    gate: ReadGate,
    /// Threads reading, the job's own included.
    readers: usize,
    /// Files handed out to read, in id order, next one at the front.
    pending: Mutex<VecDeque<DueFile>>,
    /// Signalled when files are added, or the job stops.
    wake: Condvar,
    /// False once the last page has been added.
    more: AtomicBool,
    /// Set when the job ends early: readers take nothing more.
    stopped: AtomicBool,
    /// Files taken by a reader so far.
    taken: AtomicU64,
}

impl<V> Run<'_, V> {
    fn pending(&self) -> MutexGuard<'_, VecDeque<DueFile>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How low the pending list may run before the next page is fetched:
    /// a few files per reader, so they keep reading while it's fetched.
    fn readers_worth(&self) -> usize {
        4 * self.readers
    }

    /// The next file to read. `None` once there are no more, or the job
    /// stopped.
    fn next(&self) -> Option<DueFile> {
        let mut pending = self.pending();
        loop {
            if self.stopped.load(Ordering::SeqCst) {
                return None;
            }
            if let Some(due) = pending.pop_front() {
                return Some(due);
            }
            if !self.more.load(Ordering::SeqCst) {
                return None;
            }
            pending = self
                .wake
                .wait_timeout(pending, IDLE_POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Ends the run: readers take nothing more.
    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.more.store(false, Ordering::SeqCst);
        self.wake.notify_all();
    }
}

/// One file's result, to write.
#[derive(Debug)]
struct Done {
    due: DueFile,
    outcome: Result<FileRead, Outcome>,
}

/// Reads one due file. `None` if it can't be reached now (its music
/// folder's volume is offline, or its row doesn't read back): nothing is
/// recorded, so it's due again when it's back.
fn read_due(
    folders: &[StoredFolder],
    volumes: &impl Volumes,
    gate: ReadGate,
    due: &DueFile,
) -> Option<Done> {
    // The walk marked it online-only and the user hasn't opted in: it's
    // never opened (reading it would download it).
    if !gate.allows(due.online_only) {
        return Some(Done {
            due: due.clone(),
            outcome: Err(Outcome::online_only()),
        });
    }
    let path = path_of(folders, volumes, due)?;
    let outcome = file::read(&path, |p| gate.may_open(p)).map_err(|e| match e {
        NotRead::OnlineOnly => Outcome::online_only(),
        NotRead::Unreachable(_) => Outcome::unreachable(),
    });
    Some(Done {
        due: due.clone(),
        outcome,
    })
}

/// Where `due` is now: a `\\?\` path under its volume's mount point.
fn path_of(folders: &[StoredFolder], volumes: &impl Volumes, due: &DueFile) -> Option<PathBuf> {
    let folder = folders
        .iter()
        .find(|f| f.id == MusicFolderId(due.music_folder_id))?;
    let rel = RelPath::parse(&due.rel_path).ok()?;
    folder.stored_path().join(&rel).resolve(volumes).ok()
}

/// Files read but not yet written.
struct Batch {
    rows: Vec<Done>,
    since: Instant,
    max: usize,
    window: Duration,
}

impl Batch {
    fn new(max: usize, window: Duration) -> Batch {
        Batch {
            rows: Vec::new(),
            since: Instant::now(),
            max,
            window,
        }
    }

    fn push(&mut self, done: Done) {
        if self.rows.is_empty() {
            self.since = Instant::now();
        }
        self.rows.push(done);
    }

    /// Whether it holds files and is full, or its first file has waited
    /// long enough.
    fn due(&self) -> bool {
        !self.rows.is_empty()
            && (self.rows.len() >= self.max || self.since.elapsed() >= self.window)
    }

    /// Writes the batch in one transaction.
    fn flush(&mut self, job: &JobContext) -> Result<(), JobError> {
        if self.rows.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.rows);
        job.writer().call(move |c| write(c, &rows))?;
        Ok(())
    }
}

/// Writes each read file's stage-2 columns and every file's `file_stage`
/// row, in one transaction.
fn write(conn: &mut Connection, rows: &[Done]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    {
        let mut update = tx.prepare_cached(
            "UPDATE file SET
                 sniffed_format = ?2, codec = ?3, bitrate = ?4, sample_rate = ?5,
                 duration_ms = ?6, raw_tags = ?7,
                 quality_verdict = CASE
                     WHEN ?8 IS NOT NULL THEN ?8
                     WHEN quality_verdict IN ('truncated', 'broken') THEN NULL
                     ELSE quality_verdict
                 END
             WHERE id = ?1",
        )?;
        for done in rows {
            if let Ok(read) = &done.outcome {
                update.execute((
                    done.due.id,
                    read.sniffed_format.as_str(),
                    read.codec.map(Codec::as_str),
                    read.bitrate_kbps,
                    read.sample_rate,
                    read.duration_ms,
                    read.raw_tags.as_deref(),
                    read.verdict.map(Verdict::as_str),
                ))?;
            }
        }
    }
    let recorded: Vec<_> = rows
        .iter()
        .map(|done| {
            let outcome = match &done.outcome {
                Ok(_) => Outcome::Done,
                Err(outcome) => outcome.clone(),
            };
            Recorded::of(&done.due, outcome)
        })
        .collect();
    scan_state::record(&tx, Stage::Read, READ_VERSION, &recorded)?;
    tx.commit()
}

#[cfg(test)]
mod tests;
