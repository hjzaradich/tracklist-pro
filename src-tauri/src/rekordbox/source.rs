//! Where the rekordbox collection comes from: the XML export (1aB-11,
//! ROADMAP 1.2).
//!
//! The user saves File → Export Collection in xml format from rekordbox and
//! picks the file; the app remembers it and reads it as a job
//! ([`read_job`]), which replaces the `rekordbox_track` snapshot
//! ([`super::store`]). With the watch on, the app also looks for a newer
//! export, in the chosen file or in rekordbox's default export folder
//! (Documents, §5.3), and offers to read it. It never reads one unasked.
//!
//! Everything here only reads the export and its folder. The only writes go
//! to the database, through the [`Writer`].
//!
//! Settings (the `setting` table, one JSON value each):
//! - [`XML_PATH`]: the export the user chose, a JSON string. Saved once
//!   it's been read, with the snapshot it made.
//! - [`XML_WATCH`]: whether to look for newer exports, a JSON bool.
//! - [`LAST_READ`]: the last successful read ([`LastRead`]), written in the
//!   same transaction as the snapshot it made.
//! - [`LAST_FAILURE`]: the last failed read ([`ReadFailure`]), cleared by
//!   the next successful one.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::store::{replace_snapshot, SnapshotRows, SnapshotSummary};
use super::{RekordboxXml, XmlError};
use crate::db::{ReadPool, Writer};
use crate::ipc::{ErrorKind, IpcError};
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, JobQueue, NewJob, Priority};

/// The export the user chose (a JSON string).
pub const XML_PATH: &str = "rekordbox_xml_path";
/// Whether to look for newer exports (a JSON bool; off when unset).
pub const XML_WATCH: &str = "rekordbox_xml_watch";
/// The last successful read ([`LastRead`]).
pub const LAST_READ: &str = "rekordbox_xml_last_read";
/// The last failed read ([`ReadFailure`]).
pub const LAST_FAILURE: &str = "rekordbox_xml_last_failure";

/// How much of a file is looked at to tell a rekordbox export from other
/// XML.
const SNIFF_BYTES: usize = 4096;

/// The last export read into the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LastRead {
    pub path: String,
    /// The file's modified time when the read began, in milliseconds since
    /// the Unix epoch.
    pub modified_ms: i64,
    /// When the snapshot was replaced (UTC, ISO 8601).
    pub read_at: String,
    pub summary: SnapshotSummary,
}

/// Why a read failed, in terms the user can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum FailureReason {
    /// The file isn't there any more.
    NotFound,
    /// The file couldn't be opened or read.
    CantRead,
    /// Not well-formed: cut off, or damaged. Export again.
    Damaged,
    /// Well-formed XML, but not a rekordbox collection export.
    NotAnExport,
}

/// The last read that failed. The snapshot is unchanged by it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadFailure {
    pub path: String,
    pub reason: FailureReason,
    /// When it failed (UTC, ISO 8601).
    pub at: String,
}

/// An export file found on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportFile {
    pub path: String,
    /// The file name, for showing.
    pub name: String,
    /// Milliseconds since the Unix epoch.
    pub modified_ms: i64,
}

/// Everything the XML source screen shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct XmlSource {
    /// The export the user chose, if any.
    pub path: Option<String>,
    pub watch: bool,
    pub last_read: Option<LastRead>,
    /// Set when the newest read attempt failed.
    pub last_failure: Option<ReadFailure>,
    /// With the watch on: an export newer than the last read, to offer.
    pub newer_export: Option<ExportFile>,
    /// rekordbox's default export folder (Documents), if Windows has one.
    pub export_folder: Option<String>,
}

/// rekordbox's default export folder (Documents, §5.3), found at startup.
/// Tests point it elsewhere.
/// Also remembers which files in it turned out not to be exports.
pub struct ExportFolder {
    folder: RwLock<Option<PathBuf>>,
    not_exports: NotExports,
}

impl ExportFolder {
    pub fn new(folder: Option<PathBuf>) -> ExportFolder {
        ExportFolder {
            folder: RwLock::new(folder),
            not_exports: NotExports::default(),
        }
    }

    pub fn get(&self) -> Option<PathBuf> {
        self.folder
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn not_exports(&self) -> &NotExports {
        &self.not_exports
    }

    #[cfg(test)]
    pub(crate) fn set(&self, folder: Option<PathBuf>) {
        *self.folder.write().unwrap_or_else(|e| e.into_inner()) = folder;
    }
}

/// Files whose first bytes showed they aren't rekordbox exports, by path
/// and modified time, so the watch opens each version of one only once.
/// Kept in memory for this run of the app.
#[derive(Default)]
pub struct NotExports(Mutex<HashSet<(PathBuf, i64)>>);

impl NotExports {
    /// Past this many, the list starts over, so it can't grow without end.
    const MAX: usize = 1024;

    /// Whether `candidate` looks like an export, asking `sniff` only if this
    /// version of the file hasn't already been found not to be one.
    fn check(&self, candidate: &Candidate, sniff: impl Fn(&Path) -> bool) -> bool {
        let key = (candidate.path.clone(), candidate.modified_ms);
        if self.lock().contains(&key) {
            return false;
        }
        let looks = sniff(&candidate.path);
        if !looks {
            let mut known = self.lock();
            if known.len() >= Self::MAX {
                known.clear();
            }
            known.insert(key);
        }
        looks
    }

    fn lock(&self) -> MutexGuard<'_, HashSet<(PathBuf, i64)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// --- settings ---

/// A setting's value, or `None` if it's unset or not what this version
/// expects (e.g. written by a newer one).
fn read_setting<T: DeserializeOwned>(conn: &Connection, key: &str) -> rusqlite::Result<Option<T>> {
    let value: Option<String> = conn
        .query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(value.and_then(|v| serde_json::from_str(&v).ok()))
}

fn write_setting<T: Serialize>(conn: &Connection, key: &str, value: &T) -> rusqlite::Result<()> {
    let json = serde_json::to_string(value)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (key, json),
    )?;
    Ok(())
}

fn now(conn: &Connection) -> rusqlite::Result<String> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
        r.get(0)
    })
}

/// The source as stored, without looking at the disk. One query, so it
/// sees the settings as one read left them, never half of a read's changes.
pub fn stored_source(conn: &Connection) -> rusqlite::Result<XmlSource> {
    let mut values: std::collections::HashMap<String, String> = conn
        .prepare("SELECT key, value FROM setting WHERE key IN (?1, ?2, ?3, ?4)")?
        .query_map([XML_PATH, XML_WATCH, LAST_READ, LAST_FAILURE], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    fn take<T: DeserializeOwned>(
        values: &mut std::collections::HashMap<String, String>,
        key: &str,
    ) -> Option<T> {
        values
            .remove(key)
            .and_then(|v| serde_json::from_str(&v).ok())
    }
    Ok(XmlSource {
        path: take(&mut values, XML_PATH),
        watch: take(&mut values, XML_WATCH).unwrap_or(false),
        last_read: take(&mut values, LAST_READ),
        last_failure: take(&mut values, LAST_FAILURE),
        newer_export: None,
        export_folder: None,
    })
}

// --- finding exports ---

/// Milliseconds since the Unix epoch, or 0 for a time before it.
fn millis(meta: &fs::Metadata) -> io::Result<i64> {
    let modified = meta.modified()?;
    Ok(modified
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)))
}

/// `path` in the form the app opens files through: `\\?\` on Windows, so
/// names are taken literally (ROADMAP §5.6). `None` if it isn't a full path.
/// The plain path is what's stored and shown.
fn open_path(path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    return crate::paths::verbatim_absolute(path).ok();
    #[cfg(not(windows))]
    return path.is_absolute().then(|| path.to_owned());
}

/// Whether the directory listing says the file's data isn't on this PC: a
/// OneDrive (or other cloud) online-only placeholder, which reading would
/// download (§5.5). Tells from the listing alone, never opening the file.
// Lane 4 (1aB-8) builds the shared check; this one is folded into it later.
fn is_online_only(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const OFFLINE: u32 = 0x1000;
        const RECALL_ON_OPEN: u32 = 0x4_0000;
        const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
        meta.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// Whether `path` is a file that starts like a rekordbox export: its first
/// few KiB hold the `DJ_PLAYLISTS` element. Only reads.
pub fn looks_like_export(path: &Path) -> bool {
    let Some(file) = open_path(path).and_then(|p| File::open(p).ok()) else {
        return false;
    };
    let mut head = Vec::with_capacity(SNIFF_BYTES);
    if file
        .take(SNIFF_BYTES as u64)
        .read_to_end(&mut head)
        .is_err()
    {
        return false;
    }
    head.windows(b"<DJ_PLAYLISTS".len())
        .any(|w| w == b"<DJ_PLAYLISTS")
}

/// A file the watch may offer.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// As stored and shown (no `\\?\` prefix).
    pub path: PathBuf,
    pub modified_ms: i64,
    /// Whether its first bytes must be checked before it's offered: a file
    /// found in the export folder must be, the chosen export needn't be.
    pub check_contents: bool,
}

/// The export the user chose, as a candidate if it's a file: offered by
/// its modified time alone, without opening it. Its details come from a
/// listing of its folder, which opens nothing (for some cloud placeholders,
/// even opening the file for its details downloads it). An online-only
/// file isn't offered: reading it would download it.
pub fn chosen_candidate(path: &Path) -> Option<Candidate> {
    let name = path.file_name()?;
    let entries = fs::read_dir(open_path(path.parent()?)?).ok()?;
    let entry = entries.filter_map(Result::ok).find(|e| {
        let listed = e.file_name();
        // Windows ignores letter case in names.
        listed == name
            || (cfg!(windows)
                && listed.to_string_lossy().to_lowercase() == name.to_string_lossy().to_lowercase())
    })?;
    let meta = entry.metadata().ok()?;
    if !meta.is_file() || is_online_only(&meta) {
        return None;
    }
    Some(Candidate {
        path: path.to_owned(),
        modified_ms: millis(&meta).ok()?,
        check_contents: false,
    })
}

/// The `.xml` files directly inside `folder` (not in subfolders), from the
/// directory listing alone. Online-only files are left out, so they're
/// never opened.
pub fn exports_in(folder: &Path) -> Vec<Candidate> {
    let Some(entries) = open_path(folder).and_then(|f| fs::read_dir(f).ok()) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let path = folder.join(&name);
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
            {
                return None;
            }
            // From the listing: on Windows this opens nothing.
            let meta = entry.metadata().ok()?;
            if !meta.is_file() || is_online_only(&meta) {
                return None;
            }
            Some(Candidate {
                path,
                modified_ms: millis(&meta).ok()?,
                check_contents: true,
            })
        })
        .collect()
}

/// The newest of `candidates` modified after the last read (any, if
/// there's been no read) whose contents, where they must be checked, look
/// like a rekordbox export. A file already found not to be one isn't
/// opened again until it changes. Only reads.
///
/// A newer modified time only prompts an offer; the user decides, and the
/// read itself trusts nothing but the file's contents.
pub fn newer_export(
    candidates: impl IntoIterator<Item = Candidate>,
    last: Option<&LastRead>,
    not_exports: &NotExports,
) -> Option<ExportFile> {
    newer_export_by(candidates, last, |c| {
        not_exports.check(c, looks_like_export)
    })
}

/// [`newer_export`], checking contents with `looks_like`.
fn newer_export_by(
    candidates: impl IntoIterator<Item = Candidate>,
    last: Option<&LastRead>,
    looks_like: impl Fn(&Candidate) -> bool,
) -> Option<ExportFile> {
    let mut candidates: Vec<Candidate> = candidates
        .into_iter()
        .filter(|c| last.is_none_or(|l| c.modified_ms > l.modified_ms))
        .collect();
    // Newest first; only as many files are opened as it takes.
    candidates.sort_by_key(|c| std::cmp::Reverse(c.modified_ms));
    let found = candidates
        .into_iter()
        .find(|c| !c.check_contents || looks_like(c))?;
    Some(ExportFile {
        name: found
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: found.path.to_string_lossy().into_owned(),
        modified_ms: found.modified_ms,
    })
}

/// `source` with its export folder filled in and, with the watch on, a
/// newer export to offer: the chosen file or any export in `export_folder`.
/// Only reads the disk.
pub fn offer_newer(
    mut source: XmlSource,
    export_folder: Option<&Path>,
    not_exports: &NotExports,
) -> XmlSource {
    source.export_folder = export_folder.map(|f| f.to_string_lossy().into_owned());
    source.newer_export = None;
    if source.watch {
        let mut candidates: Vec<Candidate> = source
            .path
            .iter()
            .filter_map(|p| chosen_candidate(Path::new(p)))
            .collect();
        if let Some(folder) = export_folder {
            candidates.extend(exports_in(folder));
        }
        source.newer_export = newer_export(candidates, source.last_read.as_ref(), not_exports);
    }
    source
}

// --- the read job ---

/// A job that reads the export at `path` into the snapshot, at the
/// priority of work the user is waiting on.
pub fn read_job(path: &Path) -> NewJob {
    NewJob::new(JobKind::ReadRekordbox)
        .target(serde_json::json!({ "path": path.to_string_lossy() }))
        .priority(Priority::USER)
}

/// Reading is this share of the job's progress; storing is the rest.
const READ_SHARE: f64 = 0.9;
/// Progress is reported after about this many bytes.
const PROGRESS_BYTES: u64 = 1 << 20;

/// The read job's handler.
#[derive(Default)]
pub struct XmlReader {
    /// Called as the read passes each [`Stage`], so tests can act there.
    #[cfg(test)]
    pub(crate) hook: Option<std::sync::Arc<dyn Fn(Stage) + Send + Sync>>,
}

/// Points in a read that tests can act at.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Progress was just reported, partway through reading the file.
    Progress,
    /// The file has been parsed; nothing is stored yet.
    Parsed,
}

impl XmlReader {
    #[cfg(test)]
    fn at(&self, stage: Stage) {
        if let Some(hook) = &self.hook {
            hook(stage);
        }
    }
}

impl JobHandler for XmlReader {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let Some(path) = job
            .target()
            .and_then(|t| t.get("path"))
            .and_then(|p| p.as_str())
            .map(PathBuf::from)
        else {
            return Err(JobError::failed("the job names no export"));
        };
        match self.read(job, &path) {
            Ok(()) => Ok(()),
            Err(Stop::Cancelled) => Err(JobError::Cancelled),
            Err(Stop::Db(e)) => Err(e.into()),
            Err(Stop::Failed(reason, detail)) => {
                let failure_path = path.to_string_lossy().into_owned();
                job.writer().call(move |c| {
                    let failure = ReadFailure {
                        path: failure_path,
                        reason,
                        at: now(c)?,
                    };
                    write_setting(c, LAST_FAILURE, &failure)
                })?;
                Err(JobError::failed(detail))
            }
        }
    }
}

enum Stop {
    Cancelled,
    /// The export couldn't be read.
    Failed(FailureReason, String),
    /// The snapshot couldn't be stored.
    Db(crate::db::DbError),
}

impl XmlReader {
    fn read(&self, job: &JobContext, path: &Path) -> Result<(), Stop> {
        let failed = |reason, detail: &dyn std::fmt::Display| {
            Stop::Failed(reason, format!("{}: {detail}", path.display()))
        };
        let io_failed = |e: &io::Error| {
            let reason = if e.kind() == io::ErrorKind::NotFound {
                FailureReason::NotFound
            } else {
                FailureReason::CantRead
            };
            failed(reason, e)
        };
        let Some(open) = open_path(path) else {
            return Err(failed(FailureReason::NotFound, &"not a full path"));
        };
        let file = File::open(open).map_err(|e| io_failed(&e))?;
        let meta = file.metadata().map_err(|e| io_failed(&e))?;
        let modified_ms = millis(&meta).map_err(|e| io_failed(&e))?;
        let counting = Counting {
            inner: file,
            read: 0,
            total: meta.len().max(1),
            next_report: PROGRESS_BYTES,
            job,
            #[cfg(test)]
            reader: self,
        };
        let parsed = RekordboxXml::parse(BufReader::with_capacity(1 << 16, counting));
        #[cfg(test)]
        self.at(Stage::Parsed);
        job.check_cancelled().map_err(|_| Stop::Cancelled)?;
        let xml = parsed.map_err(|e| match &e {
            XmlError::Io(io) => io_failed(io),
            XmlError::Malformed { .. } | XmlError::Truncated | XmlError::UnsupportedEncoding(_) => {
                failed(FailureReason::Damaged, &e)
            }
            XmlError::NotRekordboxXml { .. } | XmlError::NoCollection => {
                failed(FailureReason::NotAnExport, &e)
            }
        })?;
        let rows = SnapshotRows::from_xml(&xml);
        let tree = crate::after_send::RekordboxTree::from_xml(&xml);
        drop(xml);
        job.check_cancelled().map_err(|_| Stop::Cancelled)?;

        let path = path.to_string_lossy().into_owned();
        job.writer()
            .call(move |c| {
                replace_snapshot(c, &rows, |tx, summary, read_at| {
                    // The file read becomes the chosen export only now, with
                    // the snapshot it made.
                    write_setting(tx, XML_PATH, &path)?;
                    let last = LastRead {
                        path,
                        modified_ms,
                        read_at: read_at.to_owned(),
                        summary: *summary,
                    };
                    write_setting(tx, LAST_READ, &last)?;
                    tx.execute("DELETE FROM setting WHERE key = ?1", [LAST_FAILURE])?;
                    crate::after_send::save_tree(tx, &tree)?;
                    Ok(())
                })
            })
            .map_err(Stop::Db)?;
        // Stored: a cancel now changes nothing.
        let _ = job.progress(1.0);
        // Match the fresh snapshot's tracks to files. The read stands if
        // this fails; the next read or scan asks again.
        if let Err(e) = crate::relink::request(job.writer(), |j| job.enqueue(j)) {
            eprintln!("rekordbox read: couldn't queue a relink: {e}");
        }
        Ok(())
    }
}

/// Reports how much of the file has been read, and stops the read once
/// the job is cancelled.
struct Counting<'a> {
    inner: File,
    read: u64,
    total: u64,
    next_report: u64,
    job: &'a JobContext,
    #[cfg(test)]
    reader: &'a XmlReader,
}

impl Read for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        if self.read >= self.next_report {
            self.next_report = self.read + PROGRESS_BYTES;
            let fraction = self.read as f64 / self.total as f64;
            self.job
                .progress(fraction.min(1.0) * READ_SHARE)
                .map_err(|_| io::Error::other("cancelled"))?;
            #[cfg(test)]
            self.reader.at(Stage::Progress);
        }
        Ok(n)
    }
}

// --- commands ---

/// Checks that `path` is an export the app can read.
fn check_export(path: &Path) -> Result<(), IpcError> {
    let shown = || path.to_string_lossy().into_owned();
    if !open_path(path).is_some_and(|p| p.is_file()) {
        return Err(IpcError::with(
            ErrorKind::RekordboxXmlNotFound,
            [("path", shown().into())],
        ));
    }
    if !looks_like_export(path) {
        return Err(IpcError::with(
            ErrorKind::NotRekordboxXml,
            [("path", shown().into())],
        ));
    }
    Ok(())
}

/// The rekordbox XML source: the chosen export, the watch, the last read or
/// failure and, with the watch on, a newer export to offer.
#[tauri::command]
#[specta::specta]
pub async fn rekordbox_xml_source(
    reads: State<'_, ReadPool>,
    folder: State<'_, ExportFolder>,
) -> Result<XmlSource, IpcError> {
    let stored = reads.read(stored_source)?;
    Ok(offer_newer(
        stored,
        folder.get().as_deref(),
        folder.not_exports(),
    ))
}

/// Reads an export into the snapshot, as a job. With a `path` (the file
/// the user just picked, or a newer export offered), that file becomes the
/// chosen export once it's read; without one, the chosen export is read
/// again. Returns the job's id.
#[tauri::command]
#[specta::specta]
pub async fn read_rekordbox_xml(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    path: Option<String>,
) -> Result<JobId, IpcError> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => {
            let stored: Option<String> = writer.call(|c| read_setting(c, XML_PATH))?;
            PathBuf::from(stored.ok_or_else(|| IpcError::new(ErrorKind::NoRekordboxXml))?)
        }
    };
    check_export(&path)?;
    Ok(jobs.enqueue(read_job(&path))?)
}

/// Turns the watch for newer exports on or off.
#[tauri::command]
#[specta::specta]
pub async fn set_rekordbox_xml_watch(
    writer: State<'_, Writer>,
    watch: bool,
) -> Result<(), IpcError> {
    Ok(writer.call(move |c| write_setting(c, XML_WATCH, &watch))?)
}

#[cfg(test)]
mod tests;
