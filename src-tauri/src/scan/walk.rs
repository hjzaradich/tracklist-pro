//! Stage 1 of the scan: the walk (1aA-4, ROADMAP 1.1).
//!
//! Lists every audio file in the music folders with what the directory
//! listing already says (size, modified time, whether it's online only)
//! plus its file id, and upserts a `file` row for each, keyed by (music
//! folder, on-disk path). Nothing is read from inside a file; tags, hashes
//! and fingerprints are later stages.
//!
//! - Each music folder is resolved once, to a `\\?\` path under its
//!   volume's mount point now, and each file's path is built from it and
//!   the names in the listing. Win32 never rewrites a name (trailing dots
//!   and spaces survive).
//! - **Links aren't followed:** a directory symlink, junction or mount
//!   point inside a music folder is skipped, and so is a file symlink. A
//!   folder reached that way is indexed where it really is, if that's a
//!   music folder, so nothing is indexed twice and no walk can loop.
//! - A folder that can't be listed (no permission, gone mid-walk) is
//!   skipped and the rest of the walk carries on. So is an audio file that
//!   can't be looked at, or whose name can't be stored. The walk counts
//!   both, and a walk that reaches the end of a music folder stores the
//!   counts on it (1aB-14).
//! - **Online-only files** (OneDrive placeholders) are indexed and marked,
//!   from the listing's attributes; nothing opens them ([`super::online_only`]).
//! - **The unchanged check** (1aC-1, [`super::unchanged`]): before a file's
//!   row is updated, its stored size, mtime and file id are compared with
//!   the listing. A file whose mtime alone moved (rekordbox touches files)
//!   is compared by its partial hash; if the content is the same, no stage
//!   redoes it. The first walk of a file opens nothing.
//! - Rows are written in batches, each in one transaction, and every batch's
//!   new rows go to the frontend as one [`ScannedFiles`] event. Cancelling
//!   stops at the next entry; batches already written stay, whole.
//! - **Rows are never deleted.** A walk that reaches the end of a music
//!   folder marks the files it didn't find `present = 0` (1aB-9), except
//!   under folders it couldn't list: those weren't looked for. A file that
//!   comes back is marked present again, same row.
//! - **A folder whose drive goes away mid-walk** (unplugged, or swapped
//!   for another drive on the same letter) stops there: files found since
//!   the last batch are dropped, not written, and nothing is marked
//!   missing. The folder is offline, not empty. Before each batch is
//!   written the walk checks that the folder still resolves to where the
//!   walk started, and the volumes are looked at again after every device
//!   change ([`crate::paths::devices_changed`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::folders::{self, MusicFolderId, StoredFolder};
use super::is_indexed;
use super::online_only::{attributes, is_online_only, ReadGate};
use super::unchanged::{self, Verdict};
use crate::hash::partial::PartialHash;
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::paths::{RelPath, Volumes};

/// A batch is written once it holds this many files…
pub const BATCH_MAX: usize = 1000;
/// …or once its first file has waited this long, so the first results
/// show up quickly even on a slow drive.
pub const BATCH_WINDOW: Duration = Duration::from_millis(250);

/// A file the walk added to the index, as the frontend hears it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScannedFile {
    /// The `file` row id.
    pub id: i64,
    pub music_folder_id: MusicFolderId,
    /// From the music folder, `/`-separated, in the on-disk spelling.
    pub rel_path: String,
    /// In bytes.
    pub size: i64,
    /// Milliseconds since the Unix epoch (the stored nanoseconds don't fit
    /// a JavaScript number).
    pub modified_ms: i64,
    /// A OneDrive placeholder: its bytes aren't on this PC.
    pub online_only: bool,
}

/// Files the walk just added to the index, one batch at a time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct ScannedFiles(pub Vec<ScannedFile>);

/// One file as the listing described it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub rel: RelPath,
    pub size: i64,
    /// Nanoseconds since the Unix epoch.
    pub mtime_ns: i64,
    pub file_id: Option<String>,
    /// The listing's attributes say it's a placeholder.
    pub online_only: bool,
}

/// A scan of `ids`, or of every music folder if `None`, at the priority of
/// work the user is waiting on.
pub fn scan_job(ids: Option<Vec<MusicFolderId>>) -> NewJob {
    let job = NewJob::new(JobKind::Scan).priority(Priority::USER);
    match ids {
        Some(ids) => job.target(serde_json::json!({ "music_folder_ids": ids })),
        None => job,
    }
}

/// The music folders a scan job's target names: every folder for no
/// target. `None` if the target isn't one [`scan_job`] makes.
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

/// The scan job's handler: walks the folders its target names.
pub struct Walker<V, S> {
    /// The volumes mounted now, asked once per job.
    volumes: Box<dyn Fn() -> V + Send + Sync>,
    /// Where each batch of new rows goes.
    sink: S,
    /// Called before each directory entry is looked at, with how many the
    /// job has looked at so far. Tests use it to act mid-walk.
    on_entry: Option<EntryHook>,
    /// Called with the path of each file the unchanged check is about to
    /// open. Tests use it to see which files are read.
    on_read: Option<ReadHook>,
    /// How long a batch's first file may wait: [`BATCH_WINDOW`].
    window: Duration,
}

/// See [`Walker::on_entry`].
type EntryHook = Box<dyn Fn(u64) + Send + Sync>;

/// See [`Walker::on_read`].
type ReadHook = Box<dyn Fn(&Path) + Send + Sync>;

impl<V, S> Walker<V, S>
where
    V: Volumes + 'static,
    S: Fn(Vec<ScannedFile>) + Send + Sync + 'static,
{
    pub fn new(volumes: impl Fn() -> V + Send + Sync + 'static, sink: S) -> Walker<V, S> {
        Walker {
            volumes: Box::new(volumes),
            sink,
            on_entry: None,
            on_read: None,
            window: BATCH_WINDOW,
        }
    }

    /// Replaces [`BATCH_WINDOW`], so a test decides when batches go out.
    #[cfg(test)]
    pub(crate) fn batch_window(mut self, window: Duration) -> Self {
        self.window = window;
        self
    }

    /// Calls `hook` before each directory entry, with the number of entries
    /// looked at so far, e.g. to cancel at an exact point.
    #[cfg(test)]
    pub(crate) fn on_entry(mut self, hook: impl Fn(u64) + Send + Sync + 'static) -> Self {
        self.on_entry = Some(Box::new(hook));
        self
    }

    /// Calls `hook` with each file the unchanged check is about to open.
    #[cfg(test)]
    pub(crate) fn on_read(mut self, hook: impl Fn(&Path) + Send + Sync + 'static) -> Self {
        self.on_read = Some(Box::new(hook));
        self
    }
}

impl<V, S> JobHandler for Walker<V, S>
where
    V: Volumes + 'static,
    S: Fn(Vec<ScannedFile>) + Send + Sync + 'static,
{
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let ids = wanted(job.target())
            .ok_or_else(|| JobError::failed("a scan's target names no music folders"))?;
        let mut folders = job.writer().call(|c| folders::stored(c))?;
        if let Some(ids) = ids {
            folders.retain(|f| ids.contains(&f.id));
        }
        let volumes = (self.volumes)();
        let count = folders.len().max(1) as f64;
        let mut walk = Walk {
            job,
            sink: &self.sink,
            on_entry: self.on_entry.as_deref(),
            reads: Reads {
                gate: job.writer().call(|c| ReadGate::for_job(c))?,
                on_read: self.on_read.as_deref(),
            },
            window: self.window,
            entries: 0,
        };
        for (n, folder) in folders.iter().enumerate() {
            let range = (n as f64 / count, (n + 1) as f64 / count);
            // An offline folder stays in the index as it is (1aB-9).
            let Ok(root) = folder.stored_path().resolve(&volumes) else {
                job.progress(range.1)?;
                continue;
            };
            // Who the drive says it is, asked of the drive itself: the
            // drive list can lag behind a swap by a moment.
            let identity = || {
                volumes
                    .sighting_for(&root)
                    .ok()
                    .map(|s| (s.volume.id, s.guid))
            };
            let at_start = identity();
            let still_there = || {
                folder.stored_path().resolve(&volumes).ok().as_ref() == Some(&root)
                    && at_start
                        .as_ref()
                        .is_none_or(|start| identity().as_ref() == Some(start))
            };
            walk.folder(folder, &root, range, &still_there)?;
        }
        job.progress(1.0)
    }
}

/// One job's walk over its music folders.
struct Walk<'a, S> {
    job: &'a JobContext,
    sink: &'a S,
    on_entry: Option<&'a (dyn Fn(u64) + Send + Sync)>,
    reads: Reads<'a>,
    window: Duration,
    /// Directory entries looked at so far.
    entries: u64,
}

/// What the unchanged check may open.
struct Reads<'a> {
    gate: ReadGate,
    on_read: Option<&'a (dyn Fn(&Path) + Send + Sync)>,
}

/// A folder waiting to be listed, and its share of the job's progress.
struct Dir {
    abs: PathBuf,
    rel: RelPath,
    lo: f64,
    hi: f64,
}

impl<S: Fn(Vec<ScannedFile>)> Walk<'_, S> {
    /// Walks one music folder at `root` (a `\\?\` path), moving the job's
    /// progress from `range.0` to `range.1`.
    ///
    /// Progress without counting first: each folder's share is split evenly
    /// among its subfolders, and folders are listed depth first in name
    /// order, so it only ever goes up.
    ///
    /// `still_there` says whether the folder still resolves to `root`, on
    /// the same drive (serial and GUID) as when the walk began; once it
    /// doesn't, the walk of this folder stops (see the module doc).
    fn folder(
        &mut self,
        folder: &StoredFolder,
        root: &Path,
        range: (f64, f64),
        still_there: &dyn Fn() -> bool,
    ) -> Result<(), JobError> {
        let job = self.job;
        // Every file found from now on is seen at or after this.
        let started: String = job.writer().call(|c| c.query_row(NOW, [], |r| r.get(0)))?;
        let mut batch = Batch::new(folder.id, self.window, started.clone());
        let mut unread = Unread::default();
        let mut stack = vec![Dir {
            abs: root.to_path_buf(),
            rel: RelPath::root(),
            lo: range.0,
            hi: range.1,
        }];
        while let Some(dir) = stack.pop() {
            job.progress(dir.lo)?;
            // Files found a while ago go out even if the rest of the walk
            // turns up no more audio for a long time.
            if batch.due() {
                if !still_there() {
                    return Ok(());
                }
                batch.flush(job, self.sink, &self.reads)?;
            }
            let Ok(entries) = fs::read_dir(&dir.abs) else {
                unread.folder(&dir.rel);
                continue;
            };
            let mut subdirs = Vec::new();
            for entry in entries {
                if let Some(hook) = self.on_entry {
                    hook(self.entries);
                }
                self.entries += 1;
                job.check_cancelled()?;
                // The listing broke off: the rest of this folder is unread.
                let Ok(entry) = entry else {
                    unread.folder(&dir.rel);
                    break;
                };
                let name = entry.file_name();
                // From the listing itself: no file is opened for this.
                let Ok(kind) = entry.file_type() else {
                    unread.file_if_audio(&name);
                    continue;
                };
                // Symlinks, junctions and mount points: never followed.
                if kind.is_symlink() {
                    continue;
                }
                // Folders Windows marks as its own (hidden and system, like
                // System Volume Information and $Recycle.Bin at a drive's
                // root), at any depth: neither walked nor counted.
                if kind.is_dir() && is_windows_own(&entry) {
                    continue;
                }
                // A name that isn't valid Unicode, or that a path can't
                // hold, can't be stored.
                let Some(rel_name) = name.to_str().and_then(|n| RelPath::parse(n).ok()) else {
                    if kind.is_dir() {
                        unread.folders += 1;
                    } else if kind.is_file() {
                        unread.file_if_audio(&name);
                    }
                    continue;
                };
                if kind.is_dir() {
                    subdirs.push(rel_name);
                    continue;
                }
                if !kind.is_file() || !is_indexed(rel_name.as_str()) {
                    continue;
                }
                // Also from the listing: size, times and attributes.
                let Ok(meta) = entry.metadata() else {
                    unread.files += 1;
                    continue;
                };
                let abs = dir.abs.join(rel_name.as_str());
                let found = Found {
                    rel: dir.rel.join(&rel_name),
                    size: i64::try_from(meta.len()).unwrap_or(i64::MAX),
                    mtime_ns: meta.modified().map(nanos).unwrap_or(0),
                    // An access-0 handle: never recalls a placeholder.
                    file_id: file_id(&abs),
                    online_only: is_online_only(attributes(&meta)),
                };
                batch.push(found, abs);
                if batch.due() {
                    if !still_there() {
                        return Ok(());
                    }
                    batch.flush(job, self.sink, &self.reads)?;
                }
            }
            if subdirs.is_empty() {
                job.progress(dir.hi)?;
                continue;
            }
            subdirs.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            let step = (dir.hi - dir.lo) / subdirs.len() as f64;
            // Pushed last-first, so they're listed first-first.
            for (i, name) in subdirs.iter().enumerate().rev() {
                stack.push(Dir {
                    abs: dir.abs.join(name.as_str()),
                    rel: dir.rel.join(name),
                    lo: dir.lo + step * i as f64,
                    hi: dir.lo + step * (i + 1) as f64,
                });
            }
        }
        if !still_there() {
            return Ok(());
        }
        batch.flush(job, self.sink, &self.reads)?;
        let id = folder.id;
        job.writer()
            .call(move |c| finish(c, id, &unread, &started))?;
        Ok(())
    }
}

/// The time now, as the `file` and `music_folder` rows store it.
const NOW: &str = "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// A folder the listing marks both hidden and system: Windows' own, like
/// `System Volume Information` and `$Recycle.Bin` at a drive's root. It's
/// skipped wherever it is, not only at a root. From the listing; nothing
/// is opened. A folder that's only hidden is the user's and is walked.
fn is_windows_own(entry: &fs::DirEntry) -> bool {
    const HIDDEN_AND_SYSTEM: u32 = 0x2 | 0x4; // FILE_ATTRIBUTE_HIDDEN | _SYSTEM
    entry
        .metadata()
        .is_ok_and(|m| attributes(&m) & HIDDEN_AND_SYSTEM == HIDDEN_AND_SYSTEM)
}

/// What a walk of one music folder couldn't read (1aB-14).
#[derive(Debug, Default)]
pub(crate) struct Unread {
    /// Folders it couldn't list, or finish listing.
    pub folders: u32,
    /// Audio files it couldn't look at, or whose names can't be stored.
    pub files: u32,
    /// The folders counted in `folders` that have a stored path, from the
    /// music folder: rows under them weren't looked for.
    pub unlisted: Vec<RelPath>,
}

impl Unread {
    fn folder(&mut self, rel: &RelPath) {
        self.folders += 1;
        self.unlisted.push(rel.clone());
    }

    /// Counts a file the walk couldn't read, if its name says it's audio.
    fn file_if_audio(&mut self, name: &std::ffi::OsStr) {
        if is_indexed(&name.to_string_lossy()) {
            self.files += 1;
        }
    }
}

/// After a walk of `folder` that started at `started` reached the end, in
/// one transaction: marks the files it didn't see missing (`present = 0`),
/// except under folders it couldn't list, and stores what it couldn't
/// read. Returns how many files it marked missing.
pub(crate) fn finish(
    conn: &mut Connection,
    folder: MusicFolderId,
    unread: &Unread,
    started: &str,
) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let unseen: Vec<(i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT id, rel_path FROM file
             WHERE music_folder_id = ?1 AND present = 1
               AND (last_seen_at IS NULL OR last_seen_at < ?2)",
        )?;
        let rows = stmt.query_map((folder.0, started), |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut missing = 0;
    {
        let mut mark = tx.prepare_cached("UPDATE file SET present = 0 WHERE id = ?1")?;
        for (id, rel) in unseen {
            // A row whose path doesn't parse can't be shown to be outside
            // an unlisted folder, so it's left as it is.
            let Ok(rel) = RelPath::parse(&rel) else {
                continue;
            };
            if unread
                .unlisted
                .iter()
                .any(|dir| rel.strip_prefix(dir).is_some())
            {
                continue;
            }
            missing += mark.execute([id])?;
        }
    }
    tx.execute(
        "UPDATE music_folder SET
             walked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             unreadable_folders = ?2,
             unreadable_files = ?3
         WHERE id = ?1",
        (folder.0, unread.folders, unread.files),
    )?;
    tx.commit()?;
    Ok(missing)
}

#[cfg(windows)]
fn file_id(path: &Path) -> Option<String> {
    super::file_id::file_id(path).ok()
}

#[cfg(not(windows))]
fn file_id(_path: &Path) -> Option<String> {
    None
}

/// Nanoseconds since the Unix epoch; negative before it. The fingerprint
/// job checks a file is still the one its row describes with it too.
pub(crate) fn nanos(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_nanos()).map_or(i64::MIN, |n| -n),
    }
}

/// Files found but not yet written.
struct Batch {
    folder: MusicFolderId,
    /// When the walk started: every row it writes was seen at or after it.
    started: String,
    rows: Vec<Found>,
    /// Where each of `rows` is on disk, for the unchanged check.
    paths: Vec<PathBuf>,
    since: Instant,
    window: Duration,
}

impl Batch {
    fn new(folder: MusicFolderId, window: Duration, started: String) -> Batch {
        Batch {
            folder,
            started,
            rows: Vec::new(),
            paths: Vec::new(),
            since: Instant::now(),
            window,
        }
    }

    fn push(&mut self, found: Found, path: PathBuf) {
        if self.rows.is_empty() {
            self.since = Instant::now();
        }
        self.rows.push(found);
        self.paths.push(path);
    }

    /// Whether it holds files and is full, or its first file has waited
    /// long enough.
    fn due(&self) -> bool {
        !self.rows.is_empty()
            && (self.rows.len() >= BATCH_MAX || self.since.elapsed() >= self.window)
    }

    /// Writes the batch in one transaction, then sends its new rows. Files
    /// whose mtime alone moved are compared by content first (1aC-1), off
    /// the writer.
    fn flush(
        &mut self,
        job: &JobContext,
        sink: &impl Fn(Vec<ScannedFile>),
        reads: &Reads,
    ) -> Result<(), JobError> {
        if self.rows.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.rows);
        let paths = std::mem::take(&mut self.paths);
        let (folder, started) = (self.folder, self.started.clone());
        let rels: Vec<String> = rows.iter().map(|f| f.rel.as_str().to_owned()).collect();
        let before = job
            .writer()
            .call(move |c| unchanged::stored(c, folder, &rels))?;
        let mut now: Vec<Option<PartialHash>> = Vec::with_capacity(rows.len());
        for ((found, path), stored) in rows.iter().zip(&paths).zip(&before) {
            let wanted = stored
                .as_ref()
                .is_some_and(|s| unchanged::wants_read(unchanged::compare(s, found), s));
            if !wanted {
                now.push(None);
                continue;
            }
            job.check_cancelled()?;
            now.push(reads.partial_hash(path, found.online_only));
        }
        let new = job
            .writer()
            .call(move |c| upsert_with(c, folder, &rows, &started, &now))?;
        if !new.is_empty() {
            sink(new);
        }
        Ok(())
    }
}

impl Reads<'_> {
    /// The partial hash of the file at `path` now, if the walk may open it
    /// ([`unchanged::may_read`]) and it has one.
    fn partial_hash(&self, path: &Path, online_only: bool) -> Option<PartialHash> {
        if !unchanged::may_read(&self.gate, path, online_only) {
            return None;
        }
        if let Some(hook) = self.on_read {
            hook(path);
        }
        unchanged::read_now(path)
    }
}

/// [`upsert_with`] with no partial hashes read: a file whose mtime alone
/// moved counts as changed.
#[cfg(test)]
pub(crate) fn upsert(
    conn: &mut Connection,
    folder: MusicFolderId,
    rows: &[Found],
    started: &str,
) -> rusqlite::Result<Vec<ScannedFile>> {
    upsert_with(conn, folder, rows, started, &[])
}

/// Upserts `rows` into `folder` in one transaction, keyed by the on-disk
/// path (an NFC name and its NFD twin are two files, §0.3). A file seen
/// again gets its stat, file id and online-only mark refreshed and is
/// marked present. Each row is marked seen now, and never earlier than
/// `started` (the walk's start), even if the clock went back since.
///
/// The unchanged check (1aC-1, [`unchanged`]) decides what else a file seen
/// again needs, from its stored row and `now[i]`, the partial hash of
/// `rows[i]` as it is now (`None`, or missing, if it wasn't read):
/// - unchanged: nothing more;
/// - only its mtime moved, and `now[i]` equals the stored partial hash:
///   the `file_stage` rows recorded at the old size and mtime move to the
///   new mtime, so no stage redoes it;
/// - only its file id moved (size and mtime match), and `now[i]` equals the
///   stored partial hash: the row takes the new id and the stages stand;
/// - changed: the stored partial hash is cleared (it's the old content's),
///   and if it's another file (its file id differs) its `file_stage` rows
///   are dropped, so every stage redoes it. A changed size or mtime
///   already makes the stages due.
///
/// Returns the rows that are new.
pub(crate) fn upsert_with(
    conn: &mut Connection,
    folder: MusicFolderId,
    rows: &[Found],
    started: &str,
    now: &[Option<PartialHash>],
) -> rusqlite::Result<Vec<ScannedFile>> {
    let tx = conn.transaction()?;
    let mut new = Vec::new();
    {
        let mut insert = tx.prepare_cached(
            "INSERT INTO file
                 (music_folder_id, rel_path, rel_path_key, size, mtime, file_id, online_only,
                  last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                     max(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?8))
             ON CONFLICT (music_folder_id, rel_path) DO NOTHING",
        )?;
        // ?9: the content changed, so the stored partial hash goes.
        let mut update = tx.prepare_cached(
            "UPDATE file SET
                 rel_path_key = ?3, size = ?4, mtime = ?5, file_id = ?6, online_only = ?7,
                 present = 1,
                 last_seen_at = max(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?8),
                 partial_hash = CASE WHEN ?9 THEN NULL ELSE partial_hash END
             WHERE music_folder_id = ?1 AND rel_path = ?2",
        )?;
        let mut carry_stages = tx.prepare_cached(
            "UPDATE file_stage SET mtime = ?2
             WHERE file_id = ?1 AND size IS ?3 AND mtime IS ?4",
        )?;
        let mut drop_stages = tx.prepare_cached("DELETE FROM file_stage WHERE file_id = ?1")?;
        for (i, f) in rows.iter().enumerate() {
            let row = (
                folder.0,
                f.rel.as_str(),
                f.rel.match_key(),
                f.size,
                f.mtime_ns,
                f.file_id.as_deref(),
                f.online_only,
                started,
            );
            let Some((id, stored)) = unchanged::stored_one(&tx, folder, f.rel.as_str())? else {
                insert.execute(row)?;
                new.push(ScannedFile {
                    id: tx.last_insert_rowid(),
                    music_folder_id: folder,
                    rel_path: f.rel.as_str().to_owned(),
                    size: f.size,
                    modified_ms: f.mtime_ns.div_euclid(1_000_000),
                    online_only: f.online_only,
                });
                continue;
            };
            let change = unchanged::compare(&stored, f);
            let verdict = unchanged::verdict(change, &stored, now.get(i).and_then(Option::as_ref));
            let changed = matches!(verdict, Verdict::Changed { .. });
            update.execute((
                row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, changed,
            ))?;
            match verdict {
                Verdict::Unchanged | Verdict::NewIdOnly | Verdict::Changed { replaced: false } => {}
                Verdict::TouchedOnly => {
                    carry_stages.execute((id, f.mtime_ns, stored.size, stored.mtime))?;
                }
                Verdict::Changed { replaced: true } => {
                    drop_stages.execute([id])?;
                }
            }
        }
    }
    tx.commit()?;
    Ok(new)
}
