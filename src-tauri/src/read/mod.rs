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
//! - Rows are written in batches, each in one transaction with the batch's
//!   `file_stage` rows. Cancelling stops before the next file; batches
//!   already written stay, whole, and the unwritten one is read again next
//!   time.

mod codec;
mod file;

pub use codec::Codec;
pub use file::{FileRead, NotRead, Verdict};

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use tauri::State;

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
/// mounted at the start of each job.
pub fn reader() -> impl JobHandler {
    Reader::new(crate::scan::system_volumes)
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
}

/// See [`Reader::on_file`].
type FileHook = Box<dyn Fn(u64) + Send + Sync>;

impl<V: Volumes + 'static> Reader<V> {
    pub fn new(volumes: impl Fn() -> V + Send + Sync + 'static) -> Reader<V> {
        Reader {
            volumes: Box::new(volumes),
            on_file: None,
            batch_max: BATCH_MAX,
            window: BATCH_WINDOW,
        }
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

impl<V: Volumes + 'static> JobHandler for Reader<V> {
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

        let mut batch = Batch::new(self.batch_max, self.window);
        let mut looked_at = 0u64;
        let mut after = 0i64;
        loop {
            let page_scope = scope.clone();
            let limit = self.batch_max;
            let page = job.writer().call(move |c| {
                scan_state::due(c, Stage::Read, READ_VERSION, &page_scope, after, limit)
            })?;
            let Some(last) = page.last() else { break };
            after = last.id;
            for due in page {
                if let Some(hook) = &self.on_file {
                    hook(looked_at);
                }
                looked_at += 1;
                job.check_cancelled()?;
                if let Some(result) = read_due(&folders, &volumes, gate, &due) {
                    batch.push(result);
                }
                if batch.due() {
                    batch.flush(job)?;
                }
                if total > 0 {
                    job.progress(looked_at as f64 / total as f64)?;
                }
            }
        }
        batch.flush(job)?;
        job.progress(1.0)
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
