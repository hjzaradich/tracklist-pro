//! A music folder that moved: point it at its new place (ROADMAP 4.1).
//!
//! After a bundle is imported on another computer, or after the user moved
//! the music, a music folder's drive or path is gone. The user points it at
//! where it is now. Only a folder that is missing can be moved. Before
//! anything changes, a sample of the folder's known files must be found
//! under the new place, at the same paths and sizes, so a wrong folder is
//! refused. Then the folder's volume and path are re-pointed in one
//! recorded operation (one Undo puts them back) and the folder is scanned
//! again. The files keep their rows and ids, so Library links, history and
//! Undo stay valid.
//!
//! Read-only on the music: it only asks for each sampled file's size.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::db::{DbError, Writer};
use crate::ipc::{ErrorKind, ErrorParam, IpcError};
use crate::jobs::JobQueue;
use crate::ops::{self, OpsError};
use crate::paths::{PathError, RelPath, StoredPath, Volumes};
use crate::volume::Volume;

use super::folders::{
    overlap, stored_one, under_mount, upsert_volume, MusicFolderId, StoredFolder,
};
use super::{display_path, system_volumes};

/// How many known files are looked for under the new place.
pub const SAMPLE_CAP: usize = 20;

/// The operation's kind in the log.
pub const OPERATION: &str = "relocate_music_folder";

/// What a relocation did, for the screen to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Relocated {
    pub id: MusicFolderId,
    /// How many of the checked files were found at the new place.
    pub found: u32,
    /// How many known files were looked for (at most [`SAMPLE_CAP`]).
    pub checked: u32,
    /// The folder's new place as a person reads it.
    pub path: String,
}

/// Why a folder couldn't be moved. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelocateError {
    /// There's no music folder with that id.
    NotFound,
    /// The folder is reachable now: only a missing one can be moved.
    NotMissing,
    /// The new path isn't a folder that exists.
    NotAFolder {
        path: String,
    },
    /// The new path can't be stored.
    BadPath {
        path: String,
    },
    /// The new place is inside, or around, another music folder.
    Overlaps {
        music_folder: String,
    },
    /// The app knows no files in the folder yet, so there is nothing to
    /// recognise it by.
    NoKnownFiles,
    /// Too few of the folder's known files are at the new place.
    TooFewFiles {
        found: u32,
        checked: u32,
    },
    Database(IpcError),
}

impl From<DbError> for RelocateError {
    fn from(e: DbError) -> Self {
        RelocateError::Database(IpcError::from(e))
    }
}

impl From<OpsError> for RelocateError {
    fn from(e: OpsError) -> Self {
        RelocateError::Database(IpcError::from(e))
    }
}

impl From<rusqlite::Error> for RelocateError {
    fn from(e: rusqlite::Error) -> Self {
        RelocateError::from(DbError::from(e))
    }
}

impl From<RelocateError> for IpcError {
    fn from(e: RelocateError) -> Self {
        let count = |n: u32| ErrorParam::from(i32::try_from(n).unwrap_or(i32::MAX));
        match e {
            RelocateError::NotFound => IpcError::new(ErrorKind::MusicFolderNotFound),
            RelocateError::NotMissing => IpcError::new(ErrorKind::RelocateNotMissing),
            RelocateError::NotAFolder { path } => {
                IpcError::with(ErrorKind::NotAFolder, [("path", ErrorParam::from(path))])
            }
            RelocateError::BadPath { path } => {
                IpcError::with(ErrorKind::BadPath, [("path", ErrorParam::from(path))])
            }
            RelocateError::Overlaps { music_folder } => IpcError::with(
                ErrorKind::RelocateOverlaps,
                [("musicFolder", ErrorParam::from(music_folder))],
            ),
            RelocateError::NoKnownFiles => IpcError::new(ErrorKind::RelocateNoKnownFiles),
            RelocateError::TooFewFiles { found, checked } => IpcError::with(
                ErrorKind::RelocateTooFewFiles,
                [("found", count(found)), ("checked", count(checked))],
            ),
            RelocateError::Database(e) => e,
        }
    }
}

/// Whether the folder can't be reached now: its drive isn't mounted, or the
/// folder isn't on it.
fn is_missing(folder: &StoredFolder, volumes: &impl Volumes) -> bool {
    match folder.stored_path().resolve(volumes) {
        Ok(abs) => !abs.is_dir(),
        Err(_) => true,
    }
}

/// A folder's known files to look for: up to [`SAMPLE_CAP`] of the files
/// last seen present, spread evenly over the folder's file list.
fn sample(conn: &rusqlite::Connection, id: MusicFolderId) -> rusqlite::Result<Vec<(String, i64)>> {
    let total: i64 = conn.query_row(
        "SELECT count(*) FROM file
         WHERE music_folder_id = ?1 AND present = 1 AND size IS NOT NULL",
        [id.0],
        |r| r.get(0),
    )?;
    let wanted = total.min(SAMPLE_CAP as i64);
    let mut picked = Vec::new();
    for i in 0..wanted {
        picked.push(conn.query_row(
            "SELECT rel_path, size FROM file
             WHERE music_folder_id = ?1 AND present = 1 AND size IS NOT NULL
             ORDER BY rel_path_key LIMIT 1 OFFSET ?2",
            (id.0, i * total / wanted),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?);
    }
    Ok(picked)
}

/// How many of `files` are under `base` with the same size.
fn count_found(base: &Path, files: &[(String, i64)]) -> u32 {
    let found = files.iter().filter(|(rel, size)| {
        let mut path = base.to_path_buf();
        path.extend(rel.split('/'));
        std::fs::metadata(&path)
            .map(|m| m.is_file() && i64::try_from(m.len()).ok() == Some(*size))
            .unwrap_or(false)
    });
    u32::try_from(found.count()).unwrap_or(u32::MAX)
}

/// What the checks learned, ready to be applied in one transaction.
pub(crate) struct Plan {
    id: MusicFolderId,
    volume: Volume,
    guid: Option<String>,
    rel: RelPath,
    /// The old and new place as a person reads them, for the log.
    from: String,
    to: String,
}

/// Points the missing music folder `id` at `new_path`. The caller queues the
/// folder's scan.
pub fn relocate<V: Volumes + Send + Sync + 'static>(
    writer: &Writer,
    volumes: &Arc<V>,
    id: MusicFolderId,
    new_path: &Path,
) -> Result<Relocated, RelocateError> {
    let (old, files) = writer
        .call(move |conn| {
            let Some(old) = stored_one(conn, id)? else {
                return Ok(None);
            };
            let files = sample(conn, id)?;
            Ok(Some((old, files)))
        })?
        .ok_or(RelocateError::NotFound)?;
    if !is_missing(&old, &**volumes) {
        return Err(RelocateError::NotMissing);
    }

    let asked = new_path.to_string_lossy().into_owned();
    let bad = || RelocateError::BadPath {
        path: asked.clone(),
    };
    let new = StoredPath::from_absolute(new_path, &**volumes).map_err(|e| match e {
        PathError::Unreadable { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            RelocateError::NotAFolder {
                path: asked.clone(),
            }
        }
        _ => bad(),
    })?;
    let abs = new.resolve(&**volumes).map_err(|_| bad())?;
    if !abs.is_dir() {
        return Err(RelocateError::NotAFolder { path: asked });
    }
    let volume = volumes.volume_for(&abs).map_err(|_| bad())?;
    let guid = volumes.guid(&volume.id);

    if files.is_empty() {
        return Err(RelocateError::NoKnownFiles);
    }
    let checked = u32::try_from(files.len()).unwrap_or(u32::MAX);
    let found = count_found(&abs, &files);
    // Four in five: a few files may have been renamed or left behind, but a
    // folder that is mostly not there is not this one.
    if u64::from(found) * 5 < u64::from(checked) * 4 {
        return Err(RelocateError::TooFewFiles { found, checked });
    }

    let shown = display_path(&abs);
    let plan = Plan {
        id,
        volume,
        guid,
        rel: new.rel().clone(),
        from: under_mount(
            Path::new(&old.last_mount_path.clone().unwrap_or_default()),
            &old.rel,
        ),
        to: shown.clone(),
    };
    apply(writer, volumes, plan)?;
    Ok(Relocated {
        id,
        found,
        checked,
        path: shown,
    })
}

#[cfg(test)]
impl Plan {
    /// A plan to `rel` on `volume`, for tests that skip the checks.
    pub(crate) fn for_test(id: MusicFolderId, volume: Volume, rel: &str) -> Plan {
        Plan {
            id,
            volume,
            guid: None,
            rel: RelPath::parse(rel).unwrap(),
            from: String::new(),
            to: rel.to_owned(),
        }
    }
}

/// Makes the change in one transaction. The folder is checked once more
/// there, since the drive may have come back since the first look, and the
/// volume is stored there too, so a refused call leaves nothing behind.
pub(crate) fn apply<V: Volumes + Send + Sync + 'static>(
    writer: &Writer,
    volumes: &Arc<V>,
    plan: Plan,
) -> Result<(), RelocateError> {
    let volumes = Arc::clone(volumes);
    let refused = writer.call(move |conn| {
        let Plan {
            id,
            volume,
            guid,
            rel,
            from,
            to,
        } = plan;
        let details = serde_json::json!({ "name": to, "from": from });
        let done = ops::record_with_derived_first(
            conn,
            OPERATION,
            &details,
            |tx| {
                let still_missing = match stored_one(tx, id)? {
                    None => return Ok(Err(RelocateError::NotFound)),
                    Some(old) => is_missing(&old, &*volumes),
                };
                if !still_missing {
                    return Ok(Err(RelocateError::NotMissing));
                }
                Ok(Ok(upsert_volume(tx, &volume, guid.as_deref())?))
            },
            |rec, volume_id| {
                let volume_id = match volume_id {
                    Ok(volume_id) => volume_id,
                    Err(refusal) => return Ok(Some(refusal)),
                };
                let others: Vec<(String, bool)> = rec.read_rows(
                    "SELECT rel_path, volume_id = ?1 FROM music_folder WHERE id <> ?2",
                    (volume_id, id.0),
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                for (other, same_volume) in others {
                    let Ok(other) = RelPath::parse(&other) else {
                        continue;
                    };
                    if same_volume && overlap(&rel, &other).is_some() {
                        return Ok(Some(RelocateError::Overlaps {
                            music_folder: under_mount(&volume.mount_path, &other),
                        }));
                    }
                }
                rec.set("music_folder", id.0, "volume_id", volume_id)?;
                rec.set("music_folder", id.0, "rel_path", rel.as_str())?;
                rec.set("music_folder", id.0, "rel_path_key", rel.match_key())?;
                Ok(None)
            },
            // The clone rule reads the volume rows, and one may be new.
            super::volumes::remember,
        );
        Ok(done)
    })?;
    // A refusal wrote nothing, so the operation (and the volume row) were
    // rolled back with it.
    match refused?.value {
        Some(refusal) => Err(refusal),
        None => Ok(()),
    }
}

/// Points a missing music folder at its new place and scans it again, as
/// something the user asked for. The folder's files keep their ids.
#[tauri::command]
#[specta::specta]
pub async fn relocate_music_folder(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    watchers: State<'_, super::Watchers>,
    folder_id: MusicFolderId,
    new_path: String,
) -> Result<Relocated, IpcError> {
    let moved = relocate(
        &writer,
        &Arc::new(system_volumes()),
        folder_id,
        Path::new(&new_path),
    )?;
    super::chain::queue_once(&writer, super::scan_job(Some(vec![folder_id])), |job| {
        jobs.enqueue(job)
    })?;
    // The watchers look at the folder in its new place.
    watchers.refresh();
    Ok(moved)
}
