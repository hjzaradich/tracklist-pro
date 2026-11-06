//! The optional reader for rekordbox's own library database (ROADMAP 2.7).
//! Off until the user turns it on ([`Feature::RekordboxDatabase`]); one
//! removable module.
//!
//! The user types the database key in Settings. It's kept in memory for
//! this run of the app only ([`DbKey`]): never written to disk, a log, an
//! error or a job's target, and never shown back. The user picks the
//! database file, and a read is a job, as an XML read is:
//! 1. `master.db`, and its `-wal` file if there is one, are copied into a
//!    staging folder in app data ([`WriteGuard::staging`]).
//!    rekordbox's own files are only ever opened for reading.
//! 2. The copy is decrypted into a plain SQLite file there ([`cipher`]).
//! 3. That file is opened read-only, and its tracks and playlists are read
//!    as an export's would be ([`library`]).
//! 4. What was read joins the core where an XML read enters
//!    ([`DatabaseSource`]), and is stored and recorded as an XML read is,
//!    marked [`ReadSource::Database`].
//!
//! The staging folder is deleted as the read ends, whether it worked or
//! not. A failed read changes nothing else: the snapshot, the XML export
//! and its last failure stay as they were. Sends stay XML (ROADMAP 1.9).
//! OneLibrary's `exportLibrary.db` is never read.

mod cipher;
mod library;

use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::jobs::{JobContext, JobError, JobId, JobKind, JobQueue, NewJob, Priority};
use crate::rekordbox::source::{stored_source, DatabaseSource, LastRead, ReadSource};
use crate::rekordbox::RekordboxXml;
use crate::settings::{read_feature_enabled, Feature};
use crate::write_guard::WriteGuard;

/// The staging folder's name in app data's `staging` folder.
const STAGING: &str = "rekordbox-database";
/// The staged copies' names.
const COPY: &str = "master.db";
const PLAIN: &str = "plain.db";

/// The key the user typed, as typed. Its `Debug` never shows it, and its
/// bytes are wiped when it's dropped.
pub struct DbKey(Vec<u8>);

impl DbKey {
    /// The key typed as `text`; `None` for nothing typed. Takes the text's
    /// own bytes, so no other copy is made here.
    pub fn from_text(text: String) -> Option<DbKey> {
        (!text.is_empty()).then(|| DbKey(text.into_bytes()))
    }

    fn passphrase(&self) -> &[u8] {
        &self.0
    }

    /// A key typed as `x'…'` with 64 hex digits is the raw key itself, not
    /// a passphrase to derive one from.
    fn raw(&self) -> Option<[u8; 32]> {
        let hex = self.0.strip_prefix(b"x'")?.strip_suffix(b"'")?;
        if hex.len() != 64 {
            return None;
        }
        let mut raw = [0u8; 32];
        for (byte, pair) in raw.iter_mut().zip(hex.chunks(2)) {
            let digits = std::str::from_utf8(pair).ok()?;
            *byte = u8::from_str_radix(digits, 16).ok()?;
        }
        Some(raw)
    }
}

impl fmt::Debug for DbKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DbKey(redacted)")
    }
}

impl Drop for DbKey {
    fn drop(&mut self) {
        self.0.fill(0);
        std::hint::black_box(&self.0);
    }
}

/// Why a read didn't happen, or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DbFailure {
    /// The feature is off.
    TurnedOff,
    /// No key has been typed in this run of the app.
    NoKey,
    /// The database file isn't there.
    NotFound,
    /// It couldn't be read, or copied into staging.
    CantRead,
    /// The key isn't this database's.
    WrongKey,
    /// The file is cut off or damaged.
    Damaged,
    /// The file opened, but isn't a rekordbox library.
    NotALibrary,
    /// The read was cancelled.
    Stopped,
}

impl DbFailure {
    fn says(self) -> &'static str {
        match self {
            DbFailure::TurnedOff => "reading rekordbox's database is turned off",
            DbFailure::NoKey => "no database key was entered",
            DbFailure::NotFound => "the database isn't there",
            DbFailure::CantRead => "the database couldn't be read",
            DbFailure::WrongKey => "the key isn't this database's",
            DbFailure::Damaged => "the database is cut off or damaged",
            DbFailure::NotALibrary => "the file isn't a rekordbox library",
            DbFailure::Stopped => "the read was stopped",
        }
    }
}

/// A failed read, with its detail for the job (never the key).
struct Failed(DbFailure, String);

impl Failed {
    fn new(reason: DbFailure, detail: impl fmt::Display) -> Failed {
        Failed(reason, format!("{}: {detail}", reason.says()))
    }
}

/// This run of the app's key and read outcomes.
#[derive(Default)]
pub struct Session {
    state: Mutex<SessionState>,
}

#[derive(Default)]
struct SessionState {
    key: Option<Arc<DbKey>>,
    /// How many reads have failed: changes exactly when one does.
    failures: u32,
    /// Why the newest read failed, until one works.
    failure: Option<DbFailure>,
}

impl Session {
    fn lock(&self) -> std::sync::MutexGuard<'_, SessionState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_key(&self, key: Option<DbKey>) {
        self.lock().key = key.map(Arc::new);
    }

    fn key(&self) -> Option<Arc<DbKey>> {
        self.lock().key.clone()
    }

    fn ended(&self, failure: Option<DbFailure>) {
        let mut state = self.lock();
        if failure.is_some() {
            state.failures = state.failures.wrapping_add(1);
        }
        state.failure = failure;
    }
}

/// The app's one session.
fn session() -> Arc<Session> {
    static SESSION: LazyLock<Arc<Session>> = LazyLock::new(Arc::default);
    SESSION.clone()
}

/// The read job for the database at `path`. Its target names the file
/// and the source, never the key.
pub fn read_job(path: &Path) -> NewJob {
    NewJob::new(JobKind::ReadRekordbox)
        .target(serde_json::json!({ "path": path.to_string_lossy(), "source": "database" }))
        .priority(Priority::USER)
}

/// The database read the XML read job asks for ([`DatabaseSource`]).
pub struct DatabaseReader {
    guard: Box<dyn Fn() -> Option<WriteGuard> + Send + Sync>,
    session: Arc<Session>,
}

/// The app's database reader, for the read job's handler.
pub fn reader<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn DatabaseSource> {
    let app = app.clone();
    Arc::new(DatabaseReader {
        guard: Box::new(move || app.try_state::<WriteGuard>().map(|g| g.inner().clone())),
        session: session(),
    })
}

impl DatabaseSource for DatabaseReader {
    fn read(&self, job: &JobContext, path: &Path) -> Result<(RekordboxXml, i64), JobError> {
        let read = self.attempt(job, path);
        self.session.ended(read.as_ref().err().map(|f| f.0));
        read.map_err(|Failed(reason, detail)| match reason {
            DbFailure::Stopped => JobError::Cancelled,
            _ => JobError::failed(detail),
        })
    }
}

impl DatabaseReader {
    fn attempt(&self, job: &JobContext, path: &Path) -> Result<(RekordboxXml, i64), Failed> {
        let on = job
            .writer()
            .call(|c| read_feature_enabled(c, Feature::RekordboxDatabase))
            .map_err(|e| Failed::new(DbFailure::CantRead, e))?;
        if !on {
            return Err(Failed::new(DbFailure::TurnedOff, path.display()));
        }
        let key = self
            .session
            .key()
            .ok_or_else(|| Failed::new(DbFailure::NoKey, path.display()))?;
        let guard =
            (self.guard)().ok_or_else(|| Failed::new(DbFailure::CantRead, "no app data folder"))?;
        read_database(&guard, &key, job, path)
    }
}

/// `path` in the form files are opened through: `\\?\` on Windows, so
/// names are taken literally. `None` if it isn't a full path.
fn open_path(path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    return crate::paths::verbatim_absolute(path).ok();
    #[cfg(not(windows))]
    return path.is_absolute().then(|| path.to_owned());
}

/// Steps 1 to 3 of a read (see the module's doc).
fn read_database(
    guard: &WriteGuard,
    key: &DbKey,
    job: &JobContext,
    path: &Path,
) -> Result<(RekordboxXml, i64), Failed> {
    let shown = path.display();
    let io_failed = |e: io::Error| {
        let reason = if e.kind() == io::ErrorKind::NotFound {
            DbFailure::NotFound
        } else {
            DbFailure::CantRead
        };
        Failed::new(reason, format_args!("{shown}: {e}"))
    };
    let stopped = |_| Failed::new(DbFailure::Stopped, &shown);
    let staging_failed = |e| Failed::new(DbFailure::CantRead, format_args!("staging: {e}"));
    let open =
        open_path(path).ok_or_else(|| Failed::new(DbFailure::NotFound, "not a full path"))?;
    let mut source = File::open(&open).map_err(io_failed)?;
    let modified_ms = source
        .metadata()
        .and_then(|m| m.modified())
        .map_err(io_failed)?
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));

    // Declared first, so it's dropped (deleting every copy) after the
    // files and the connection opened in it.
    let staging = guard.staging(STAGING).map_err(staging_failed)?;
    let copy = staging.copy_in(COPY, &mut source).map_err(staging_failed)?;
    drop(source);
    // The decryptor folds the `-wal` in itself, so `-shm` is never opened.
    let mut name = open.clone().into_os_string();
    name.push("-wal");
    let wal_copy = match File::open(PathBuf::from(name)) {
        Ok(mut sidecar) => Some(
            staging
                .copy_in(&format!("{COPY}-wal"), &mut sidecar)
                .map_err(staging_failed)?,
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_failed(e)),
    };
    job.progress(0.2).map_err(stopped)?;

    let (plain, out) = staging.create(PLAIN).map_err(staging_failed)?;
    let mut db = File::open(&copy).map_err(io_failed)?;
    let mut wal = wal_copy.map(File::open).transpose().map_err(io_failed)?;
    let decrypted = cipher::decrypt(key, &mut db, wal.as_mut(), &mut BufWriter::new(out));
    drop((db, wal));
    decrypted.map_err(|e| match e {
        cipher::DecryptError::WrongKey => Failed::new(DbFailure::WrongKey, &shown),
        cipher::DecryptError::Damaged(why) => {
            Failed::new(DbFailure::Damaged, format_args!("{shown}: {why}"))
        }
        cipher::DecryptError::NotEncrypted => Failed::new(DbFailure::NotALibrary, &shown),
        cipher::DecryptError::Io(e) => {
            Failed::new(DbFailure::CantRead, format_args!("{shown}: {e}"))
        }
    })?;
    job.progress(0.5).map_err(stopped)?;

    let conn = guard
        .check(&plain)
        .map_err(staging_failed)?
        .open_database_read_only()
        .map_err(|e| Failed::new(DbFailure::Damaged, format_args!("{shown}: {e}")))?;
    let xml = library::read(&conn).map_err(|e| match e {
        library::LibraryError::NotALibrary(e) => {
            Failed::new(DbFailure::NotALibrary, format_args!("{shown}: {e}"))
        }
        library::LibraryError::Xml(e) => {
            Failed::new(DbFailure::Damaged, format_args!("{shown}: {e}"))
        }
    })?;
    drop(conn);
    if let Err(e) = staging.close() {
        eprintln!("rekordbox database: couldn't delete the staging folder: {e}");
    }
    job.progress(0.8).map_err(stopped)?;
    Ok((xml, modified_ms))
}

// --- commands ---

/// What the Settings row shows. With the feature off, nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RekordboxDatabase {
    /// Whether a key was typed in this run of the app.
    pub key_set: bool,
    /// How many reads have failed in this run: changes exactly when one
    /// does, so a read has ended when this or `last_read` changes.
    pub failures: u32,
    /// Why the newest read failed, until one works.
    pub failure: Option<DbFailure>,
    /// The newest read, when it was of the database.
    pub last_read: Option<LastRead>,
    /// rekordbox's usual folder, where the picker opens, if it's there.
    pub usual_folder: Option<String>,
}

/// rekordbox's usual folder inside `base` (the user's roaming app data),
/// if it's there. Only looked at, never opened.
fn usual_folder_in(base: &Path) -> Option<PathBuf> {
    let folder = base.join("Pioneer").join("rekordbox"); // ci-reads: not a frontend read
    folder.is_dir().then_some(folder)
}

fn feature_on(reads: &ReadPool) -> Result<bool, IpcError> {
    Ok(reads.read(|c| read_feature_enabled(c, Feature::RekordboxDatabase))?)
}

/// The database reader's state for Settings.
#[tauri::command]
#[specta::specta]
pub async fn rekordbox_database(reads: State<'_, ReadPool>) -> Result<RekordboxDatabase, IpcError> {
    status(&reads, &session(), dirs::config_dir().as_deref())
}

fn status(
    reads: &ReadPool,
    session: &Session,
    base: Option<&Path>,
) -> Result<RekordboxDatabase, IpcError> {
    if !feature_on(reads)? {
        return Ok(RekordboxDatabase::default());
    }
    let last_read = reads
        .read(stored_source)?
        .last_read
        .filter(|r| r.source == ReadSource::Database);
    let state = session.lock();
    Ok(RekordboxDatabase {
        key_set: state.key.is_some(),
        failures: state.failures,
        failure: state.failure,
        last_read,
        usual_folder: base
            .and_then(usual_folder_in)
            .map(|f| f.to_string_lossy().into_owned()),
    })
}

/// Keeps the typed key for this run of the app; an empty one forgets it.
/// With the feature off, nothing is kept. Returns whether a key is kept.
#[tauri::command]
#[specta::specta]
pub async fn set_rekordbox_database_key(
    reads: State<'_, ReadPool>,
    key: String,
) -> Result<bool, IpcError> {
    set_key(&reads, &session(), key)
}

fn set_key(reads: &ReadPool, session: &Session, key: String) -> Result<bool, IpcError> {
    let key = if feature_on(reads)? {
        DbKey::from_text(key)
    } else {
        None
    };
    let kept = key.is_some();
    session.set_key(key);
    Ok(kept)
}

/// Whether a read began.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DatabaseReadStart {
    Started { job: JobId },
    Refused { reason: DbFailure },
}

/// Reads the database at `path` into the snapshot, as a job.
#[tauri::command]
#[specta::specta]
pub async fn read_rekordbox_database(
    reads: State<'_, ReadPool>,
    jobs: State<'_, JobQueue>,
    path: String,
) -> Result<DatabaseReadStart, IpcError> {
    start_read(&reads, &jobs, &session(), Path::new(&path))
}

fn start_read(
    reads: &ReadPool,
    jobs: &JobQueue,
    session: &Session,
    path: &Path,
) -> Result<DatabaseReadStart, IpcError> {
    let refused = |reason| Ok(DatabaseReadStart::Refused { reason });
    if !feature_on(reads)? {
        return refused(DbFailure::TurnedOff);
    }
    if session.key().is_none() {
        return refused(DbFailure::NoKey);
    }
    if !open_path(path).is_some_and(|p| p.is_file()) {
        return refused(DbFailure::NotFound);
    }
    Ok(DatabaseReadStart::Started {
        job: jobs.enqueue(read_job(path))?,
    })
}

#[cfg(test)]
mod tests;
