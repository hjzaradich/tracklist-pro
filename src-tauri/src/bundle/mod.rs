//! The portability bundle (ROADMAP 4.1): the Library's database in a
//! `.zip`, to move it to another of your own computers.
//!
//! A bundle is a consistent snapshot of the database ([`create`]) with the
//! rows that are true only on this PC taken out ([`MACHINE_ONLY`]), plus
//! `manifest.json`. Ids travel unchanged, AUTOINCREMENT counters too, and
//! files keep their drive identity and relative paths, so the music is
//! found again by drive and folder. No keys or passwords are in the
//! database (the master.db key lives in memory only).
//!
//! Importing reads the picked file into app data's staging folder and
//! checks it there ([`check`]); a bad file never reaches the live database.
//! A good one is staged as `import/pending.db`; once the user confirms, the
//! app restarts and [`swap`] puts it in place before the database opens,
//! keeping the old Library as a backup that can be restored the same way.
//!
//! This job writes bundles only into app data (`bundles/`); saving one
//! somewhere the user picks comes later.

mod check;
mod folders;
pub mod swap;
#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime, State};

use crate::db::{DbError, ReadPool, DB_FILE_NAME};
use crate::ipc::{ErrorKind, IpcError};
use crate::write_guard::{GuardError, KeepOut, SaveRefusal, WriteGuard};
use folders::{exists, keep_newest, make_folder, remove};
use swap::{is_stamp, stamp, Layout};

/// The manifest's `app`.
const APP: &str = "tracklist-pro";
/// The bundle format this build writes and the newest it reads.
const FORMAT: u32 = 1;
const MANIFEST_ENTRY: &str = "manifest.json";
const DB_ENTRY: &str = "library.db";
/// How many bundles are kept in `bundles/`: just the newest. They are
/// staging for the Save dialog that comes later, not an archive.
const KEEP_BUNDLES: usize = 1;

/// What makes a database true only on this PC. Run on the bundle's copy,
/// never on the Library: the job queue, consent to online services (given
/// per install), settings that name this PC's files or state, where each
/// drive was last mounted, and the files the audio model could not hear (the
/// other PC may decode them). Everything else travels, including the
/// audio model's results (`embedding`): they are computed from the user's
/// own audio and take hours to make again. The genre model learned from them
/// (`genre_model`, `genre_model_node`) travels too: it is checked against the
/// tracks it was learned from, and learned again on the other PC if they
/// differ.
const MACHINE_ONLY: &str = "
    DELETE FROM job;
    DELETE FROM service_optin;
    DELETE FROM embedding_failure;
    DELETE FROM setting
     WHERE key IN ('rekordbox_xml_path', 'rekordbox_xml_watch',
                   'rekordbox_xml_last_failure', 'read_online_only_files',
                   'update_check_asked', 'update_check_result')
        OR key GLOB 'inbox.incompleteWalks.*';
    UPDATE setting SET value = json_set(value, '$.path', '')
     WHERE key = 'rekordbox_xml_last_read' AND json_valid(value);
    UPDATE volume SET last_mount_path = NULL;
";

/// The manifest stored beside the database in a bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub app: String,
    pub format: u32,
    pub app_version: String,
    pub schema_version: u32,
    pub created_ms: i64,
    pub counts: Counts,
}

/// A Library's size in a few numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub tracks: i64,
    pub crates: i64,
    pub playlists: i64,
    pub tags: i64,
    pub music_folders: i64,
}

/// A bundle just made.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreatedBundle {
    pub path: String,
    pub manifest: Manifest,
}

/// What a bundle or backup holds, for the confirm screen. The schema and
/// counts are read from its database, not taken from the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BundleSummary {
    pub created_ms: i64,
    /// None for a backup.
    pub app_version: Option<String>,
    pub schema_version: u32,
    pub counts: Counts,
}

/// A replaced Library kept in `backups/`.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeptLibrary {
    pub id: String,
    pub created_ms: i64,
    /// None when it can't be read.
    pub counts: Option<Counts>,
}

#[derive(Debug)]
pub enum BundleError {
    /// Not a zip with exactly our entries, not SQLite, or not our schema.
    NotOurs,
    /// An entry unpacks to more than its cap.
    TooLarge,
    /// The database is damaged or its links are broken.
    Damaged,
    /// Made by a newer build (format or schema).
    Newer,
    /// Confirm with nothing staged.
    NothingStaged,
    /// No backup with that id.
    NoSuchBackup,
    /// A swap would write over a file; both are kept.
    Blocked,
    /// A crash injected by a test.
    Interrupted,
    Guard(GuardError),
    Io(io::Error),
    Db(DbError),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::Guard(e) => write!(f, "{e}"),
            BundleError::Io(e) => write!(f, "{e}"),
            BundleError::Db(e) => write!(f, "{e}"),
            other => write!(f, "{other:?}"),
        }
    }
}

impl std::error::Error for BundleError {}

impl From<GuardError> for BundleError {
    fn from(e: GuardError) -> Self {
        BundleError::Guard(e)
    }
}
impl From<io::Error> for BundleError {
    fn from(e: io::Error) -> Self {
        BundleError::Io(e)
    }
}
impl From<zip::result::ZipError> for BundleError {
    fn from(e: zip::result::ZipError) -> Self {
        BundleError::Io(e.into())
    }
}
impl From<DbError> for BundleError {
    fn from(e: DbError) -> Self {
        BundleError::Db(e)
    }
}
impl From<rusqlite::Error> for BundleError {
    fn from(e: rusqlite::Error) -> Self {
        BundleError::Db(e.into())
    }
}

impl From<BundleError> for IpcError {
    fn from(e: BundleError) -> Self {
        match e {
            BundleError::NotOurs => IpcError::new(ErrorKind::BundleNotOurs),
            BundleError::TooLarge => IpcError::new(ErrorKind::BundleTooLarge),
            BundleError::Damaged => IpcError::new(ErrorKind::BundleDamaged),
            BundleError::Newer => IpcError::new(ErrorKind::BundleNewer),
            BundleError::NothingStaged => IpcError::new(ErrorKind::BundleNothingStaged),
            BundleError::NoSuchBackup => IpcError::new(ErrorKind::BundleBackupNotFound),
            BundleError::Guard(GuardError::SaveRefused { why, .. }) => IpcError::new(match why {
                SaveRefusal::SourceNotABundle => ErrorKind::SaveNotABundle,
                SaveRefusal::NotZip => ErrorKind::SaveNotZip,
                SaveRefusal::BadName => ErrorKind::SaveBadName,
                SaveRefusal::Exists => ErrorKind::SaveExists,
                SaveRefusal::FolderMissing => ErrorKind::SaveFolderMissing,
                SaveRefusal::FolderNotPlain => ErrorKind::SaveFolderNotPlain,
                SaveRefusal::InMusicFolder => ErrorKind::SaveInMusicFolder,
                SaveRefusal::InAppData => ErrorKind::SaveInAppData,
                SaveRefusal::GigStick => ErrorKind::SaveGigStick,
                SaveRefusal::InRekordboxFolder => ErrorKind::SaveInRekordboxFolder,
            }),
            BundleError::Db(e) => IpcError::from(e),
            other => {
                eprintln!("bundle: {other}");
                IpcError::new(ErrorKind::CannotWrite)
            }
        }
    }
}

/// Makes a bundle of the Library in `bundles/` and keeps the newest
/// [`KEEP_BUNDLES`].
pub(crate) fn create(
    guard: &WriteGuard,
    reads: &ReadPool,
    now_ms: i64,
) -> Result<CreatedBundle, BundleError> {
    let staging = guard.staging("bundle-export")?;
    let copy = guard.check(&staging.path().join(DB_ENTRY))?;
    let conn = reads.read(|live| copy.copy_database_from(live))?;
    conn.execute_batch(MACHINE_ONLY)?;
    // One file, and VACUUM so nothing taken out is left in free pages.
    conn.pragma_update(None, "journal_mode", "DELETE")?;
    conn.execute_batch("VACUUM")?;
    let manifest = Manifest {
        app: APP.to_owned(),
        format: FORMAT,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_version: crate::db::migrations::applied(&conn)?
            .last()
            .map_or(0, |a| a.version),
        created_ms: now_ms,
        counts: check::counts(&conn)?,
    };
    drop(conn);

    let (zip_path, file) = staging.create("bundle.zip")?;
    write_zip(file, &manifest, copy.as_path())?;
    let folder = make_folder(guard, &guard.app_data_dir().join("bundles"))?;
    let name = format!("tracklist-pro-{}.zip", stamp(now_ms));
    let path = folder.join(&name);
    guard.rename(&zip_path, &path)?;
    keep_newest(guard, &folder, KEEP_BUNDLES, is_bundle, Some(&name));
    Ok(CreatedBundle {
        path: path.to_string_lossy().into_owned(),
        manifest,
    })
}

fn is_bundle(name: &str) -> bool {
    name.strip_prefix("tracklist-pro-")
        .and_then(|rest| rest.strip_suffix(".zip"))
        .is_some_and(is_stamp)
}

fn write_zip(file: File, manifest: &Manifest, db: &Path) -> Result<(), BundleError> {
    use zip::write::SimpleFileOptions;
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(MANIFEST_ENTRY, options)?;
    zip.write_all(&serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?)?;
    zip.start_file(DB_ENTRY, options)?;
    io::copy(&mut File::open(db)?, &mut zip)?;
    zip.finish()?.sync_all()?;
    Ok(())
}

/// Reads and checks the bundle at `path` without staging it.
pub(crate) fn inspect(guard: &WriteGuard, path: &Path) -> Result<BundleSummary, BundleError> {
    let staging = guard.staging("bundle-inspect")?;
    let (manifest, db) = check::unpack(path, &staging, &check::CAPS)?;
    summary(guard, &db, Some(manifest), false).map(|(s, _)| s)
}

/// Checks the bundle at `path`, brings it up to this build's schema and
/// stages it for [`confirm_staged`].
pub(crate) fn stage_bundle(guard: &WriteGuard, path: &Path) -> Result<BundleSummary, BundleError> {
    stage_bundle_capped(guard, path, &check::CAPS)
}

fn stage_bundle_capped(
    guard: &WriteGuard,
    path: &Path,
    caps: &check::Caps,
) -> Result<BundleSummary, BundleError> {
    let staging = guard.staging("bundle-import")?;
    let (manifest, db) = check::unpack(path, &staging, caps)?;
    let (summary, db) = summary(guard, &db, Some(manifest), true)?;
    stage(guard, &db)?;
    Ok(summary)
}

/// Checks a staged database (migrating it with `migrate`).
fn summary(
    guard: &WriteGuard,
    db: &Path,
    manifest: Option<Manifest>,
    migrate: bool,
) -> Result<(BundleSummary, PathBuf), BundleError> {
    let checked = check::database(&guard.check(db)?, migrate)?;
    let summary = BundleSummary {
        created_ms: manifest.as_ref().map_or(0, |m| m.created_ms),
        app_version: manifest.map(|m| m.app_version),
        schema_version: checked.schema_version,
        counts: checked.counts,
    };
    Ok((summary, db.to_path_buf()))
}

/// Moves a checked database to `import/pending.db`, replacing an earlier
/// one; an earlier confirm is withdrawn, so only [`confirm_staged`] swaps it in.
fn stage(guard: &WriteGuard, db: &Path) -> Result<(), BundleError> {
    let layout = Layout::of(guard);
    make_folder(guard, &layout.import)?;
    if exists(&layout.marker) {
        remove(guard, &layout.marker)?;
    }
    guard.rename(db, &layout.pending)?;
    Ok(())
}

/// The user confirmed: the staged Library replaces this one at the next
/// start ([`swap::on_startup`]).
pub(crate) fn confirm_staged(guard: &WriteGuard) -> Result<(), BundleError> {
    let layout = Layout::of(guard);
    if !exists(&layout.pending) {
        return Err(BundleError::NothingStaged);
    }
    guard.write_then_rename(&layout.marker, b"confirmed")?;
    Ok(())
}

/// The kept Libraries, newest first.
pub(crate) fn list_kept(guard: &WriteGuard) -> Vec<KeptLibrary> {
    let layout = Layout::of(guard);
    let mut kept: Vec<KeptLibrary> = folders::ours(&layout.backups, is_stamp)
        .into_iter()
        .map(|id| {
            let counts = guard
                .check(&layout.backups.join(&id).join(DB_FILE_NAME))
                .ok()
                .and_then(|p| p.open_database_read_only().ok())
                .and_then(|c| check::counts(&c).ok());
            KeptLibrary {
                created_ms: id.parse().unwrap_or(0),
                id,
                counts,
            }
        })
        .collect();
    kept.reverse();
    kept
}

/// Stages the kept Library `id` for [`confirm_staged`], like an import.
pub(crate) fn stage_kept(guard: &WriteGuard, id: &str) -> Result<BundleSummary, BundleError> {
    let source = Layout::of(guard).backups.join(id).join(DB_FILE_NAME);
    if !is_stamp(id) || !exists(&source) {
        return Err(BundleError::NoSuchBackup);
    }
    let staging = guard.staging("bundle-restore")?;
    let copy = guard.check(&staging.path().join(DB_ENTRY))?;
    let from = guard.check(&source)?.open_database_read_only()?;
    drop(copy.copy_database_from(&from)?);
    drop(from);
    let (mut summary, db) = summary(guard, copy.as_path(), None, true)?;
    summary.created_ms = id.parse().unwrap_or(0);
    stage(guard, &db)?;
    Ok(summary)
}

#[tauri::command]
#[specta::specta]
pub async fn create_bundle(
    guard: State<'_, WriteGuard>,
    reads: State<'_, ReadPool>,
) -> Result<CreatedBundle, IpcError> {
    let (guard, reads) = (guard.inner().clone(), reads.inner().clone());
    blocking(move || create(&guard, &reads, crate::diagnostics::now_ms())).await
}

/// Runs `work` on a thread meant for blocking: copying, checking and
/// packing a whole database can take minutes, too long to hold one of the
/// async runtime's few threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, BundleError> + Send + 'static,
) -> Result<T, IpcError> {
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(done) => Ok(done?),
        Err(e) => {
            eprintln!("bundle: the work stopped: {e}");
            Err(IpcError::new(ErrorKind::Internal))
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn inspect_bundle(
    guard: State<'_, WriteGuard>,
    path: String,
) -> Result<BundleSummary, IpcError> {
    let guard = guard.inner().clone();
    blocking(move || inspect(&guard, Path::new(&path))).await
}

#[tauri::command]
#[specta::specta]
pub async fn stage_import(
    guard: State<'_, WriteGuard>,
    path: String,
) -> Result<BundleSummary, IpcError> {
    let guard = guard.inner().clone();
    blocking(move || stage_bundle(&guard, Path::new(&path))).await
}

/// Confirms the staged import or restore and restarts the app. The exit
/// stops the watchers and the job queue and folds the database's log in
/// (`on_run_event`) before the restart swaps the file.
#[tauri::command]
#[specta::specta]
pub async fn confirm_import(
    guard: State<'_, WriteGuard>,
    restart: State<'_, Restart>,
) -> Result<(), IpcError> {
    confirm_staged(&guard)?;
    (restart.0)();
    Ok(())
}

/// Restarts the app (Tauri's `request_restart`), held in the state so the
/// command needn't know the runtime.
pub struct Restart(Box<dyn Fn() + Send + Sync>);

impl Restart {
    pub fn new<R: Runtime>(app: AppHandle<R>) -> Restart {
        Restart(Box::new(move || app.request_restart()))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_backups(guard: State<'_, WriteGuard>) -> Result<Vec<KeptLibrary>, IpcError> {
    Ok(list_kept(&guard))
}

#[tauri::command]
#[specta::specta]
pub async fn stage_restore(
    guard: State<'_, WriteGuard>,
    id: String,
) -> Result<BundleSummary, IpcError> {
    let guard = guard.inner().clone();
    blocking(move || stage_kept(&guard, &id)).await
}

/// Saves a bundle made by [`create_bundle`] where the user picked in the
/// Save dialog: a new `.zip` file, through the write guard's one narrow
/// exception ([`WriteGuard::save_new_bundle_outside`], which lists every
/// condition). `name` is the bundle's file name in app data's `bundles/`.
#[tauri::command]
#[specta::specta]
pub async fn save_bundle_as(
    guard: State<'_, WriteGuard>,
    reads: State<'_, ReadPool>,
    name: String,
    dest_path: String,
) -> Result<(), IpcError> {
    let (guard, reads) = (guard.inner().clone(), reads.inner().clone());
    blocking(move || {
        let keep_out = keep_out(&reads, &[dirs::config_dir(), dirs::data_local_dir()])?;
        save_as(&guard, &name, Path::new(&dest_path), &keep_out)
    })
    .await
}

/// The folders a saved bundle must stay out of: every music folder (where
/// it is now, or was last seen) and rekordbox's `Pioneer` folder in each
/// config folder (roaming and local).
fn keep_out(reads: &ReadPool, configs: &[Option<PathBuf>]) -> Result<KeepOut, BundleError> {
    let volumes = crate::scan::system_volumes();
    let folders = reads.read(crate::scan::folders::stored)?;
    Ok(KeepOut {
        music_folders: folders
            .iter()
            .map(|f| PathBuf::from(f.to_music_folder(&volumes).path))
            .collect(),
        // `Pioneer` holds rekordbox's data; refuse the whole of it.
        rekordbox_data: configs
            .iter()
            .flatten()
            .map(|c| c.join("Pioneer"))
            .collect(),
    })
}

pub(crate) fn save_as(
    guard: &WriteGuard,
    name: &str,
    dest: &Path,
    keep_out: &KeepOut,
) -> Result<(), BundleError> {
    if !is_bundle(name) {
        return Err(BundleError::Guard(GuardError::SaveRefused {
            why: SaveRefusal::SourceNotABundle,
            path: name.into(),
        }));
    }
    let source = guard.app_data_dir().join("bundles").join(name);
    Ok(guard.save_new_bundle_outside(&source, dest, keep_out)?)
}
