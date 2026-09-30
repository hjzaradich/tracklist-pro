//! The hash job (1aB-5, 1aB-6): hashes every due file in its music
//! folders, one at a time, at background priority.
//!
//! - Each music folder is resolved once, to a `\\?\` path under its
//!   volume's mount point now; a folder on an unplugged drive is skipped.
//! - A file is hashed only if the open file still has the size and
//!   modified time the walk stored, before and after reading. Otherwise it
//!   changed since the walk, and the next walk makes it due again.
//! - Due files come from `file_stage` ([`state`]), a page at a time.
//!   Progress counts files, and moves within a big file as it's read;
//!   cancelling is checked before every buffer read.
//! - Results are written in batches, each in one transaction. On cancel,
//!   the files already hashed are written; the file being read isn't.
//! - A file that can't be opened or read is recorded as skipped
//!   (`unreachable`), so the next run tries it again.
//! - Each run ends with a [`Summary`] line in the log, counting what was
//!   hashed and what was left and why, so dogfooding shows it if files are
//!   always "changed since the walk" on some drive.
//!
//! # Why the walk's stat and the open file's stat can be compared exactly
//!
//! The walk stores the size and modified time from the directory listing
//! (`FindNextFileW`); this job reads them from the open handle
//! (`GetFileInformationByHandle`). On NTFS both come from the file's own
//! record, which the directory index copy is refreshed from when a handle
//! that wrote the file closes; on exFAT and FAT32 the directory entry *is*
//! the only copy. So for a file nobody is writing, the two agree exactly
//! (tested on NTFS in `the_walk_and_the_open_file_agree_on_size_and_mtime`).
//!
//! Two cases can disagree for a while, and neither blocks a file for good:
//! a file still being written, and an NTFS hard link, whose directory entry
//! can show a stale size and time until a handle is opened through that
//! link. Opening it here refreshes the entry, so the next walk stores what
//! the handle says and the file is hashed then. Meanwhile it's counted
//! under [`Summary::changed_since_walk`].

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::state::{self, Due, Record};
use super::{hash_reader, open_form, partial, BUFFER};
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::paths::{RelPath, Volumes};
use crate::scan::folders;
use crate::scan::MusicFolderId;
use crate::scan_state;

/// A batch is written once it holds this many files…
pub const BATCH_MAX: usize = 256;
/// …or once its first file has waited this long.
pub const BATCH_WINDOW: Duration = Duration::from_secs(1);
/// Due files are read from the database this many at a time.
const PAGE: usize = 512;

/// A hash job over `ids`, or every music folder if `None`, at background
/// priority: nobody is waiting on it.
pub fn hash_job(ids: Option<Vec<MusicFolderId>>) -> NewJob {
    let job = NewJob::new(JobKind::Hash).priority(Priority::BACKGROUND);
    match ids {
        Some(ids) => job.target(serde_json::json!({ "music_folder_ids": ids })),
        None => job,
    }
}

/// The music folders a hash job's target names: every folder for no
/// target. `None` if the target isn't one [`hash_job`] makes.
fn wanted(target: Option<&serde_json::Value>) -> Option<Option<Vec<MusicFolderId>>> {
    let Some(target) = target else {
        return Some(None);
    };
    let ids = target.get("music_folder_ids")?.as_array()?;
    ids.iter()
        .map(|id| id.as_i64().map(MusicFolderId))
        .collect::<Option<Vec<_>>>()
        .map(Some)
}

/// Called before each buffer read with the file's row id and how many of
/// its bytes were read so far. Tests use it to act mid-file.
type ReadHook = Box<dyn Fn(i64, u64) + Send + Sync>;

/// What one run did with the files it found due.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Hashed, with an audio_hash.
    pub hashed: u64,
    /// Hashed, but with no audio_hash (unknown, unsupported or broken).
    pub no_audio_hash: u64,
    /// Left because the open file's size or modified time isn't the walk's.
    pub changed_since_walk: u64,
    /// Left because it couldn't be opened or read.
    pub unreachable: u64,
    /// Left because it's a OneDrive online-only file.
    pub online_only: u64,
    /// Left because its music folder's drive isn't plugged in.
    pub offline: u64,
}

/// The hash job's handler.
pub struct Hasher<V> {
    /// The volumes mounted now, asked once per job.
    volumes: Box<dyn Fn() -> V + Send + Sync>,
    on_read: Option<ReadHook>,
    on_summary: Option<Box<dyn Fn(Summary) + Send + Sync>>,
    window: Duration,
    buffer: usize,
}

impl<V: Volumes + 'static> Hasher<V> {
    pub fn new(volumes: impl Fn() -> V + Send + Sync + 'static) -> Hasher<V> {
        Hasher {
            volumes: Box::new(volumes),
            on_read: None,
            on_summary: None,
            window: BATCH_WINDOW,
            buffer: BUFFER,
        }
    }

    /// Replaces [`BATCH_WINDOW`], so a test decides when batches go out.
    #[cfg(test)]
    pub(crate) fn batch_window(mut self, window: Duration) -> Self {
        self.window = window;
        self
    }

    /// Replaces [`BUFFER`], so a test can make a small file take many reads.
    #[cfg(test)]
    pub(crate) fn buffer(mut self, bytes: usize) -> Self {
        self.buffer = bytes.max(1);
        self
    }

    /// Hands each run's [`Summary`] to `hook` as well as to the log.
    #[cfg(test)]
    pub(crate) fn on_summary(mut self, hook: impl Fn(Summary) + Send + Sync + 'static) -> Self {
        self.on_summary = Some(Box::new(hook));
        self
    }

    /// Calls `hook` before each buffer read (see [`ReadHook`]).
    #[cfg(test)]
    pub(crate) fn on_read(mut self, hook: impl Fn(i64, u64) + Send + Sync + 'static) -> Self {
        self.on_read = Some(Box::new(hook));
        self
    }
}

impl<V: Volumes + 'static> JobHandler for Hasher<V> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let mut summary = Summary::default();
        let result = self.hash_due(job, &mut summary);
        let ending = match &result {
            Ok(()) => "done",
            Err(JobError::Cancelled) => "stopped",
            Err(JobError::Failed(_)) => "failed",
        };
        eprintln!("hash job {} {ending}: {summary:?}", job.id());
        if let Some(hook) = &self.on_summary {
            hook(summary);
        }
        result
    }
}

impl<V: Volumes + 'static> Hasher<V> {
    fn hash_due(&self, job: &JobContext, summary: &mut Summary) -> Result<(), JobError> {
        let ids = wanted(job.target())
            .ok_or_else(|| JobError::failed("a hash job's target names no music folders"))?;
        let mut folders = job.writer().call(|c| folders::stored(c))?;
        if let Some(ids) = &ids {
            folders.retain(|f| ids.contains(&f.id));
        }
        let volumes = (self.volumes)();
        let roots: HashMap<MusicFolderId, PathBuf> = folders
            .iter()
            .filter_map(|f| Some((f.id, f.stored_path().resolve(&volumes).ok()?)))
            .collect();

        let gate = job.writer().call(|c| state::ReadGate::for_job(c))?;
        let counted = ids.clone();
        let total = job
            .writer()
            .call(move |c| state::count_due(c, counted.as_deref()))?
            .max(1) as f64;
        let mut done = 0u64;
        let mut after = 0i64;
        let mut buf = vec![0u8; self.buffer];
        let mut batch = Batch::new(self.window);
        loop {
            let wanted = ids.clone();
            let page = job
                .writer()
                .call(move |c| state::due(c, &gate, wanted.as_deref(), after, PAGE))?;
            let Some(last) = page.last() else { break };
            // Never back to 0 within a run: skipped files stay due.
            after = last.id;
            for file in &page {
                if let Err(e) = job.check_cancelled() {
                    batch.flush(job)?;
                    return Err(e);
                }
                // A file's share of progress, split by how much was read.
                let size = file.size.unwrap_or(0).max(1) as f64;
                let at = |offset: u64| {
                    ((done as f64 + (offset as f64 / size).min(1.0)) / total).min(0.999)
                };
                let outcome = match roots.get(&file.folder).and_then(|r| file_path(r, file)) {
                    // An offline folder's files wait for its drive (1aB-9).
                    None => One::Offline,
                    Some(path) => {
                        let mut stopped = None;
                        let mut stop = |offset: u64| {
                            if let Some(hook) = &self.on_read {
                                hook(file.id, offset);
                            }
                            match job.progress(at(offset)) {
                                Ok(()) => false,
                                Err(e) => {
                                    stopped = Some(e);
                                    true
                                }
                            }
                        };
                        let one = hash_one(&gate, &path, file, &mut buf, &mut stop);
                        if let Ok(One::Stopped) = one {
                            batch.flush(job)?;
                            return Err(stopped.unwrap_or(JobError::Cancelled));
                        }
                        // Gone, locked, or unreadable just now.
                        one.unwrap_or(One::Unreachable)
                    }
                };
                match outcome {
                    One::Hashed(record) => {
                        match &record.result {
                            state::Result::Hashed { audio: Ok(_), .. } => summary.hashed += 1,
                            _ => summary.no_audio_hash += 1,
                        }
                        batch.push(record);
                    }
                    One::Unreachable => {
                        summary.unreachable += 1;
                        let skipped = state::Result::Skipped(scan_state::UNREACHABLE);
                        batch.push(Record::of(file, skipped));
                    }
                    One::OnlineOnly => {
                        summary.online_only += 1;
                        let skipped = state::Result::Skipped(scan_state::ONLINE_ONLY);
                        batch.push(Record::of(file, skipped));
                    }
                    // Left for the next walk or run, unrecorded.
                    One::Changed => summary.changed_since_walk += 1,
                    One::Offline => summary.offline += 1,
                    One::Stopped => unreachable!("handled above"),
                }
                done += 1;
                if batch.due() {
                    batch.flush(job)?;
                }
            }
        }
        batch.flush(job)?;
        job.progress(1.0)
    }
}

/// The file's absolute path under its music folder's `root`, if its stored
/// path still parses.
fn file_path(root: &Path, file: &Due) -> Option<PathBuf> {
    let rel = RelPath::parse(&file.rel_path).ok()?;
    let mut path = root.to_path_buf();
    for part in rel.components() {
        path.push(part);
    }
    Some(path)
}

enum One {
    Hashed(Record),
    /// The file isn't what the walk saw: another size or modified time.
    Changed,
    /// A OneDrive online-only file: reading it would download it.
    OnlineOnly,
    /// It couldn't be opened or read.
    Unreachable,
    /// Its music folder's drive isn't plugged in.
    Offline,
    /// The job was cancelled mid-file.
    Stopped,
}

/// Hashes the file at `path` if it's still the one `due` describes.
fn hash_one(
    gate: &state::ReadGate,
    path: &Path,
    due: &Due,
    buf: &mut [u8],
    stop: &mut dyn FnMut(u64) -> bool,
) -> std::io::Result<One> {
    // An error telling whether it's online-only: unreachable, not opened.
    if !due.may_read || !gate.may_open(path)? {
        return Ok(One::OnlineOnly);
    }
    // `File::open` asks for read access only.
    let mut file = File::open(open_form(path)?)?;
    let before = stat(&file)?;
    if (Some(before.0), Some(before.1)) != (due.size, due.mtime) {
        return Ok(One::Changed);
    }
    let Some(hashes) = hash_reader(&mut file, buf, stop)? else {
        return Ok(One::Stopped);
    };
    // The partial hash (1aC-1) comes from the same open file, just read:
    // its metadata and audio edges are in the cache. It's a shortcut for
    // later walks, so a file it can't be worked out for is still hashed.
    let partial = partial::partial_hash(&mut file, hashes.len).unwrap_or(None);
    if stat(&file)? != before || i64::try_from(hashes.len).ok() != Some(before.0) {
        return Ok(One::Changed);
    }
    let result = state::Result::Hashed {
        blake3: hashes.blake3,
        audio: hashes.audio.map(|a| a.to_bytes()),
        partial,
    };
    Ok(One::Hashed(Record::of(due, result)))
}

/// The open file's size and modified time (nanoseconds since the Unix
/// epoch), as the walk stores them.
fn stat(file: &File) -> std::io::Result<(i64, i64)> {
    let meta = file.metadata()?;
    let size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
    Ok((size, meta.modified().map(nanos).unwrap_or(0)))
}

/// Nanoseconds since the Unix epoch; negative before it (as the walk).
fn nanos(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_nanos()).map_or(i64::MIN, |n| -n),
    }
}

/// Results not yet written.
struct Batch {
    rows: Vec<Record>,
    since: Instant,
    window: Duration,
}

impl Batch {
    fn new(window: Duration) -> Batch {
        Batch {
            rows: Vec::new(),
            since: Instant::now(),
            window,
        }
    }

    fn push(&mut self, record: Record) {
        if self.rows.is_empty() {
            self.since = Instant::now();
        }
        self.rows.push(record);
    }

    fn due(&self) -> bool {
        !self.rows.is_empty()
            && (self.rows.len() >= BATCH_MAX || self.since.elapsed() >= self.window)
    }

    fn flush(&mut self, job: &JobContext) -> Result<(), JobError> {
        if self.rows.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.rows);
        job.writer().call(move |c| state::record(c, &rows))?;
        Ok(())
    }
}
