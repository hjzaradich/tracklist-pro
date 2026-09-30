//! Music folders: the folders the user points the app at (1aA-3, ROADMAP §2
//! `music_folder`).
//!
//! A music folder is stored as its volume plus a path from the volume's
//! mount point ([`StoredPath`]), so it survives the drive coming back under
//! another letter. Adding one never touches the disk beyond reading where
//! the folder is. Removing one deletes its index rows (`file`), never a file
//! on disk; a rescan rebuilds them, so there's no undo.
//!
//! Two music folders never overlap: a folder inside an existing one, or one
//! that contains an existing one, is refused. Otherwise the same file would
//! be indexed twice under two folders.

use std::path::Path;

use rusqlite::{ffi, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::{ErrorKind, ErrorParam, IpcError};
use crate::paths::{PathError, RelPath, StoredPath, Volumes};
use crate::volume::{Volume, VolumeId, VolumeKind};

use super::{display_path, system_volumes};

/// A music folder's row id.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct MusicFolderId(pub i64);

/// What a music folder is for. Stored in `music_folder.role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MusicFolderRole {
    /// Scanned into All music (the default).
    #[default]
    Scan,
    /// Watched for new arrivals (Phase 2, ROADMAP 2.1).
    Inbox,
}

impl MusicFolderRole {
    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            MusicFolderRole::Scan => "scan",
            MusicFolderRole::Inbox => "inbox",
        }
    }

    fn parse(name: &str) -> Option<MusicFolderRole> {
        match name {
            "scan" => Some(MusicFolderRole::Scan),
            "inbox" => Some(MusicFolderRole::Inbox),
            _ => None,
        }
    }
}

/// A music folder, as the frontend sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MusicFolder {
    pub id: MusicFolderId,
    pub role: MusicFolderRole,
    /// Whether the folder's watcher is on (1aC-6).
    pub watch: bool,
    /// Where the folder is, e.g. `E:\DJ Music`: under the volume's mount
    /// point now, or where it was last seen if the volume is offline.
    pub path: String,
    /// Whether its volume is mounted right now.
    pub online: bool,
    /// The volume's label, e.g. `GIG USB`. May be empty.
    pub volume_label: String,
    /// When it was added, UTC ISO-8601.
    pub added_at: String,
    /// When a scan last walked the whole folder, UTC ISO-8601. `None` until
    /// one has.
    pub walked_at: Option<String>,
    /// Folders inside it that the last walk couldn't list (1aB-14). `None`
    /// until a walk has finished.
    pub unreadable_folders: Option<u32>,
    /// Audio files in it that the last walk couldn't look at, or whose
    /// names can't be stored (1aB-14). `None` until a walk has finished.
    pub unreadable_files: Option<u32>,
    /// Files in it that are online only (OneDrive placeholders, 1aB-8).
    pub online_only_files: u32,
}

/// Why a music folder command failed. It reaches the frontend as an
/// [`IpcError`] (1aA-12), whose kind names the message in the
/// `musicFolders` namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MusicFolderError {
    /// The path isn't a folder that exists.
    NotAFolder { path: String },
    /// The path can't be stored: not a full path from a drive or share, a
    /// name Windows doesn't allow, or the drive can't be read.
    BadPath { path: String },
    /// That folder is already a music folder.
    AlreadyAdded { path: String },
    /// The folder is inside an existing music folder.
    InsideMusicFolder { path: String, music_folder: String },
    /// The folder contains an existing music folder.
    ContainsMusicFolder { path: String, music_folder: String },
    /// There's no music folder with that id.
    NotFound,
    /// Other data (tracks, Library tracks, relinks) still points at files
    /// in this folder, so its index can't be dropped.
    InUse,
    /// The database failed, shown the way every command shows it (busy,
    /// disk full, …). The detail was logged for developers.
    Database(IpcError),
}

impl From<DbError> for MusicFolderError {
    fn from(e: DbError) -> Self {
        MusicFolderError::Database(IpcError::from(e))
    }
}

impl From<MusicFolderError> for IpcError {
    fn from(e: MusicFolderError) -> Self {
        let path = |path: String| [("path", ErrorParam::from(path))];
        let folder = |folder: String| [("musicFolder", ErrorParam::from(folder))];
        match e {
            MusicFolderError::NotAFolder { path: p } => {
                IpcError::with(ErrorKind::NotAFolder, path(p))
            }
            MusicFolderError::BadPath { path: p } => IpcError::with(ErrorKind::BadPath, path(p)),
            MusicFolderError::AlreadyAdded { path: p } => {
                IpcError::with(ErrorKind::AlreadyAdded, path(p))
            }
            // The user just picked the folder; the message names the one
            // already there.
            MusicFolderError::InsideMusicFolder { music_folder, .. } => {
                IpcError::with(ErrorKind::InsideMusicFolder, folder(music_folder))
            }
            MusicFolderError::ContainsMusicFolder { music_folder, .. } => {
                IpcError::with(ErrorKind::ContainsMusicFolder, folder(music_folder))
            }
            MusicFolderError::NotFound => IpcError::new(ErrorKind::MusicFolderNotFound),
            MusicFolderError::InUse => IpcError::new(ErrorKind::MusicFolderInUse),
            MusicFolderError::Database(e) => e,
        }
    }
}

/// How a new folder and an existing one overlap, if they do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Overlap {
    Same,
    /// The new folder is inside the existing one.
    Inside,
    /// The new folder contains the existing one.
    Contains,
}

/// Whether `new` and `existing` (on the same volume) overlap, compared the
/// way Windows compares names: letter case ignored, spelling exact.
pub(crate) fn overlap(new: &RelPath, existing: &RelPath) -> Option<Overlap> {
    match (
        new.strip_prefix(existing).is_some(),
        existing.strip_prefix(new).is_some(),
    ) {
        (true, true) => Some(Overlap::Same),
        (true, false) => Some(Overlap::Inside),
        (false, true) => Some(Overlap::Contains),
        (false, false) => None,
    }
}

/// The folder at `path` as a person reads it, under `mount`.
fn under_mount(mount: &Path, rel: &RelPath) -> String {
    let mut path = mount.to_path_buf();
    for part in rel.components() {
        path.push(part);
    }
    display_path(&path)
}

fn kind_name(kind: VolumeKind) -> &'static str {
    match kind {
        VolumeKind::Internal => "internal",
        VolumeKind::External => "external",
        VolumeKind::Network => "network",
    }
}

/// Adds the folder at `path` (absolute) as a music folder.
///
/// Links and junctions in `path` are followed first, so the folder is
/// stored where it really is, and overlap is checked there.
pub fn add(
    writer: &Writer,
    volumes: &impl Volumes,
    path: &Path,
    role: MusicFolderRole,
) -> Result<MusicFolder, MusicFolderError> {
    let asked = path.to_string_lossy().into_owned();
    let stored = StoredPath::from_absolute(path, volumes).map_err(|e| match e {
        PathError::Unreadable { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            MusicFolderError::NotAFolder {
                path: asked.clone(),
            }
        }
        _ => MusicFolderError::BadPath {
            path: asked.clone(),
        },
    })?;
    let abs = stored
        .resolve(volumes)
        .map_err(|_| MusicFolderError::BadPath {
            path: asked.clone(),
        })?;
    if !abs.is_dir() {
        return Err(MusicFolderError::NotAFolder { path: asked });
    }
    let volume = volumes
        .volume_for(&abs)
        .map_err(|_| MusicFolderError::BadPath {
            path: asked.clone(),
        })?;
    let guid = volumes.guid(&volume.id);
    let shown = display_path(&abs);
    let rel = stored.rel().clone();
    writer.call(move |conn| {
        let added = insert(conn, &volume, guid.as_deref(), &rel, role, &shown)?;
        // The clone rule reads the volume rows; one may be new.
        super::volumes::remember(conn)?;
        Ok(added)
    })?
}

/// Stores `volume` (or refreshes what's known about it) and adds the
/// folder `rel` on it, unless it overlaps an existing music folder. One
/// transaction, so two adds can't both slip past the overlap check.
///
/// `volume` comes named by the clone rule. Its `guid` is remembered only
/// if none is yet (see [`super::volumes::note_mounted`]).
fn insert(
    conn: &mut Connection,
    volume: &Volume,
    guid: Option<&str>,
    rel: &RelPath,
    role: MusicFolderRole,
    shown: &str,
) -> rusqlite::Result<Result<MusicFolder, MusicFolderError>> {
    let tx = conn.transaction()?;
    let volume_id: i64 = tx.query_row(
        "INSERT INTO volume (identity, label, guid, kind, last_mount_path, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT (identity) DO UPDATE SET
             label = excluded.label,
             guid = coalesce(guid, excluded.guid),
             kind = excluded.kind,
             last_mount_path = excluded.last_mount_path,
             last_seen_at = excluded.last_seen_at
         RETURNING id",
        (
            volume.id.as_str(),
            &volume.label,
            guid,
            kind_name(volume.kind),
            display_path(&volume.mount_path),
        ),
        |r| r.get(0),
    )?;

    let existing: Vec<String> = {
        let mut stmt = tx.prepare("SELECT rel_path FROM music_folder WHERE volume_id = ?1")?;
        let rows = stmt.query_map([volume_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for other in existing {
        // A row that doesn't parse can't overlap anything we can open.
        let Ok(other) = RelPath::parse(&other) else {
            continue;
        };
        let path = shown.to_owned();
        let music_folder = under_mount(&volume.mount_path, &other);
        let refused = match overlap(rel, &other) {
            None => continue,
            Some(Overlap::Same) => MusicFolderError::AlreadyAdded { path },
            Some(Overlap::Inside) => MusicFolderError::InsideMusicFolder { path, music_folder },
            Some(Overlap::Contains) => MusicFolderError::ContainsMusicFolder { path, music_folder },
        };
        return Ok(Err(refused));
    }

    let (id, watch, added_at): (i64, bool, String) = tx.query_row(
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key, role)
         VALUES (?1, ?2, ?3, ?4)
         RETURNING id, watch, added_at",
        (volume_id, rel.as_str(), rel.match_key(), role.as_str()),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    tx.commit()?;
    Ok(Ok(MusicFolder {
        id: MusicFolderId(id),
        role,
        watch,
        path: shown.to_owned(),
        online: true,
        volume_label: volume.label.clone(),
        added_at,
        walked_at: None,
        unreadable_folders: None,
        unreadable_files: None,
        online_only_files: 0,
    }))
}

/// A music folder as stored: where it is and what it's for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFolder {
    pub id: MusicFolderId,
    pub role: MusicFolderRole,
    pub watch: bool,
    pub volume: VolumeId,
    pub rel: RelPath,
    pub volume_label: String,
    /// Where the volume was mounted last, e.g. `E:\`.
    pub last_mount_path: Option<String>,
    pub added_at: String,
    pub walked_at: Option<String>,
    pub unreadable_folders: Option<u32>,
    pub unreadable_files: Option<u32>,
    pub online_only_files: u32,
}

impl StoredFolder {
    /// The folder's stored path: its volume and the path on it.
    pub fn stored_path(&self) -> StoredPath {
        StoredPath::new(self.volume.clone(), self.rel.clone())
    }

    /// The folder for the frontend, placed under its volume's mount point
    /// now, or under the last one if the volume is offline.
    pub fn to_music_folder(&self, volumes: &impl Volumes) -> MusicFolder {
        let (path, online) = match self.stored_path().resolve(volumes) {
            Ok(abs) => (display_path(&abs), true),
            Err(_) => {
                let mount = self.last_mount_path.clone().unwrap_or_default();
                (under_mount(Path::new(&mount), &self.rel), false)
            }
        };
        MusicFolder {
            id: self.id,
            role: self.role,
            watch: self.watch,
            path,
            online,
            volume_label: self.volume_label.clone(),
            added_at: self.added_at.clone(),
            walked_at: self.walked_at.clone(),
            unreadable_folders: self.unreadable_folders,
            unreadable_files: self.unreadable_files,
            online_only_files: self.online_only_files,
        }
    }
}

/// Every music folder, oldest first. Rows whose volume identity, path or
/// role don't read back are left out rather than trusted.
pub fn stored(conn: &Connection) -> rusqlite::Result<Vec<StoredFolder>> {
    let mut stmt = conn.prepare(
        "SELECT mf.id, mf.role, mf.watch, v.identity, mf.rel_path, v.label,
                v.last_mount_path, mf.added_at, mf.walked_at, mf.unreadable_folders,
                mf.unreadable_files,
                (SELECT count(*) FROM file f
                 WHERE f.music_folder_id = mf.id AND f.online_only = 1 AND f.present = 1)
         FROM music_folder mf JOIN volume v ON v.id = mf.volume_id
         ORDER BY mf.id",
    )?;
    let rows = stmt.query_map([], |r| {
        let found = Found {
            walked_at: r.get(8)?,
            unreadable_folders: r.get(9)?,
            unreadable_files: r.get(10)?,
            online_only_files: r.get(11)?,
        };
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, bool>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, String>(7)?,
            found,
        ))
    })?;
    let mut folders = Vec::new();
    for row in rows {
        let (id, role, watch, identity, rel, volume_label, last_mount_path, added_at, found) = row?;
        let (Some(role), Ok(volume), Ok(rel)) = (
            MusicFolderRole::parse(&role),
            VolumeId::from_stored(identity),
            RelPath::parse(&rel),
        ) else {
            continue;
        };
        folders.push(StoredFolder {
            id: MusicFolderId(id),
            role,
            watch,
            volume,
            rel,
            volume_label,
            last_mount_path,
            added_at,
            walked_at: found.walked_at,
            unreadable_folders: found.unreadable_folders,
            unreadable_files: found.unreadable_files,
            online_only_files: found.online_only_files,
        });
    }
    Ok(folders)
}

/// What the scan has found out about a music folder so far.
struct Found {
    walked_at: Option<String>,
    unreadable_folders: Option<u32>,
    unreadable_files: Option<u32>,
    online_only_files: u32,
}

/// One music folder, if there is one with that id.
pub fn stored_one(conn: &Connection, id: MusicFolderId) -> rusqlite::Result<Option<StoredFolder>> {
    // Few folders; reading them all keeps one parsing path.
    Ok(stored(conn)?.into_iter().find(|f| f.id == id))
}

/// Removes a music folder and its index rows (`file`). Never touches the
/// disk. Refused with [`MusicFolderError::InUse`] if other rows still point
/// at its files: they're never deleted along with it.
pub fn remove(
    conn: &mut Connection,
    id: MusicFolderId,
) -> rusqlite::Result<Result<(), MusicFolderError>> {
    let tx = conn.transaction()?;
    let exists = tx
        .query_row("SELECT 1 FROM music_folder WHERE id = ?1", [id.0], |_| {
            Ok(())
        })
        .optional()?
        .is_some();
    if !exists {
        return Ok(Err(MusicFolderError::NotFound));
    }
    let deleted = tx
        .execute("DELETE FROM file WHERE music_folder_id = ?1", [id.0])
        .and_then(|_| tx.execute("DELETE FROM music_folder WHERE id = ?1", [id.0]));
    match deleted {
        Ok(_) => {
            tx.commit()?;
            Ok(Ok(()))
        }
        // Dropping `tx` rolls the file deletes back.
        Err(e) if is_foreign_key(&e) => Ok(Err(MusicFolderError::InUse)),
        Err(e) => Err(e),
    }
}

/// A foreign key refused the change. SQLite reports an `ON DELETE
/// RESTRICT` refusal as a trigger constraint with the foreign-key message,
/// and other refusals as a foreign-key constraint.
fn is_foreign_key(e: &rusqlite::Error) -> bool {
    match e {
        rusqlite::Error::SqliteFailure(f, message) => {
            f.extended_code == ffi::SQLITE_CONSTRAINT_FOREIGNKEY
                || (f.extended_code == ffi::SQLITE_CONSTRAINT_TRIGGER
                    && message.as_deref() == Some("FOREIGN KEY constraint failed"))
        }
        _ => false,
    }
}

/// Every music folder, oldest first.
#[tauri::command]
#[specta::specta]
pub async fn music_folders(reads: State<'_, ReadPool>) -> Result<Vec<MusicFolder>, IpcError> {
    let folders = reads.read(stored)?;
    let volumes = system_volumes();
    Ok(folders
        .iter()
        .map(|f| f.to_music_folder(&volumes))
        .collect())
}

/// Adds a music folder. `role` defaults to scan.
#[tauri::command]
#[specta::specta]
pub async fn add_music_folder(
    writer: State<'_, Writer>,
    path: String,
    role: Option<MusicFolderRole>,
) -> Result<MusicFolder, IpcError> {
    Ok(add(
        &writer,
        &system_volumes(),
        Path::new(&path),
        role.unwrap_or_default(),
    )?)
}

/// Removes a music folder from the app. Its files stay on disk untouched.
#[tauri::command]
#[specta::specta]
pub async fn remove_music_folder(
    writer: State<'_, Writer>,
    id: MusicFolderId,
) -> Result<(), IpcError> {
    Ok(writer.call(move |conn| remove(conn, id))??)
}
