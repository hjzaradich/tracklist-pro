//! Crates: hand-made groups of Library tracks (1aG-7; ROADMAP 1.14, §2
//! `crate` and `crate_entry`). Only the basics exist since 1aG: a crate is
//! static and at the top level. Folders, notes, summaries and smart crates
//! come in Phase 1c and 3.
//!
//! - **Every change is one recorded operation** ([`crate::ops`]), so
//!   `undo_last_operation` puts the earlier state back exactly, the entries'
//!   rows and `added_at` included. Deleting a crate deletes its entries
//!   first and explicitly: the log doesn't record rows a foreign key
//!   cascades to.
//! - **A name is trimmed**, can't be empty and can't equal another top-level
//!   crate's, compared exactly as the XML writer compares siblings
//!   ([`sibling_key`]: NFC, letter case and trailing whitespace ignored). A
//!   name with a character XML can't carry is refused too, using the
//!   writer's own check. The point: a crate the app lets the user make is
//!   never the reason a send is refused (ROADMAP 1.9, "Crates and
//!   Playlists are always both written").
//! - **A track is in a crate once.** Adding one that's there is skipped, not
//!   an error.
//! - **A crate's tracks come in the order they were added** (`added_at`,
//!   then the Library track's id), the order a send writes them
//!   ([`crate::send::crate_tree`]).
//!
//! Nothing here touches a file on disk.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{DbError, ReadPool, Writer};
use crate::fragile::{self, FragileDirs, Located};
use crate::ipc::{ErrorKind, ErrorParam, IpcError};
use crate::library::{self, LibraryTrack, LibraryTrackId, StoredTrack};
use crate::ops::{self, OpsError};
use crate::paths::Volumes;
use crate::rekordbox_write::{build, sibling_key, BuildError, Node, SendInput};
use crate::scan::system_volumes;

/// The operation kinds, as the operation log stores them.
pub const CREATE_OPERATION: &str = "create_crate";
pub const RENAME_OPERATION: &str = "rename_crate";
pub const DELETE_OPERATION: &str = "delete_crate";
pub const ADD_OPERATION: &str = "add_to_crate";
pub const REMOVE_OPERATION: &str = "remove_from_crate";

/// A crate's row id.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct CrateId(pub i64);

/// A crate, as the Crates screen lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Crate {
    pub id: CrateId,
    pub name: String,
    pub track_count: u32,
}

/// What adding tracks to a crate, or removing them, did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    /// How many tracks were added or removed.
    pub changed: u32,
    /// How many were left alone: already in the crate when adding, not in it
    /// when removing.
    pub skipped: u32,
    /// The operation this recorded; `None` when nothing changed. Undo is
    /// offered only for a recorded operation: with none, undo would take
    /// back an earlier, unrelated one.
    pub operation_id: Option<i64>,
}

/// Why a crate command was refused. Nothing was changed.
#[derive(Debug)]
pub enum CratesError {
    /// There's no such crate.
    NotFound,
    /// A Library track to add doesn't exist.
    TrackNotFound,
    /// The name is empty once trimmed.
    NameEmpty,
    /// Another top-level crate has this name. `existing` is that crate's
    /// name as stored, which may differ in letter case.
    NameTaken { existing: String },
    /// The name holds a character XML can't carry, so a send couldn't write
    /// it.
    NameUnsendable,
    /// The database or the operation log failed.
    Ops(OpsError),
}

impl From<OpsError> for CratesError {
    fn from(e: OpsError) -> Self {
        CratesError::Ops(e)
    }
}

impl From<DbError> for CratesError {
    fn from(e: DbError) -> Self {
        CratesError::Ops(OpsError::Db(e))
    }
}

impl From<rusqlite::Error> for CratesError {
    fn from(e: rusqlite::Error) -> Self {
        CratesError::Ops(OpsError::from(e))
    }
}

impl From<CratesError> for IpcError {
    fn from(e: CratesError) -> Self {
        match e {
            CratesError::NotFound => IpcError::new(ErrorKind::CrateNotFound),
            CratesError::TrackNotFound => IpcError::new(ErrorKind::LibraryTrackNotFound),
            CratesError::NameEmpty => IpcError::new(ErrorKind::CrateNameEmpty),
            CratesError::NameTaken { existing } => IpcError::with(
                ErrorKind::CrateNameTaken,
                [("name", ErrorParam::from(existing))],
            ),
            CratesError::NameUnsendable => IpcError::new(ErrorKind::CrateNameUnsendable),
            CratesError::Ops(e) => IpcError::from(e),
        }
    }
}

/// A command's answer: the refusal, or what it did.
type Done<T> = Result<T, CratesError>;

/// The name as it will be stored, if a send can carry it next to
/// `siblings` (the other top-level crates' ids and names).
///
/// Trimmed; refused if empty, if it equals a sibling's by the writer's
/// rule, or if the writer can't write it. The last check is the writer's
/// own, not a copy.
fn checked_name(siblings: &[(i64, String)], name: &str) -> Done<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CratesError::NameEmpty);
    }
    let key = sibling_key(name);
    if let Some((_, existing)) = siblings.iter().find(|(_, other)| sibling_key(other) == key) {
        return Err(CratesError::NameTaken {
            existing: existing.clone(),
        });
    }
    let alone = SendInput {
        crates: vec![Node::Playlist {
            name: name.to_owned(),
            entries: Vec::new(),
        }],
        ..SendInput::default()
    };
    match build(&alone) {
        Ok(_) => Ok(name.to_owned()),
        Err(BuildError::EmptyName { .. }) => Err(CratesError::NameEmpty),
        // Nothing else can go wrong with one empty playlist, so anything
        // else the writer says is about the name.
        Err(_) => Err(CratesError::NameUnsendable),
    }
}

/// The ids and names of the top-level crates and folders, other than
/// `except`: the names a send compares as siblings.
fn top_level(rec: &ops::Recorder<'_>, except: Option<i64>) -> Result<Vec<(i64, String)>, OpsError> {
    rec.read_rows(
        "SELECT id, name FROM crate WHERE parent_id IS NULL AND id <> ?1 ORDER BY id",
        [except.unwrap_or(0)],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// Whether `id` is a static crate.
fn is_static(rec: &ops::Recorder<'_>, id: CrateId) -> Result<bool, OpsError> {
    Ok(!rec
        .read_rows(
            "SELECT 1 FROM crate WHERE id = ?1 AND kind = 'static'",
            [id.0],
            |_| Ok(()),
        )?
        .is_empty())
}

/// Makes a crate: static, at the top level, last among its siblings.
/// Recorded as one operation.
pub fn create_on(conn: &mut Connection, name: &str) -> Result<Done<CrateId>, OpsError> {
    let name = name.trim().to_owned();
    ops::record(
        conn,
        CREATE_OPERATION,
        &serde_json::json!({ "name": name }),
        |rec| {
            let name = match checked_name(&top_level(rec, None)?, &name) {
                Ok(name) => name,
                Err(refusal) => return Ok(Err(refusal)),
            };
            let position: i64 = rec
                .read_rows(
                    "SELECT ifnull(max(position) + 1, 0) FROM crate WHERE parent_id IS NULL",
                    [],
                    |r| r.get(0),
                )?
                .pop()
                .unwrap_or(0);
            let id = rec.insert(
                "crate",
                &[
                    ("kind", "static".to_owned().into()),
                    ("name", name.into()),
                    ("position", position.into()),
                ],
            )?;
            Ok(Ok(CrateId(id)))
        },
    )
    .map(|recorded| recorded.value)
}

/// Renames a crate and returns the operation it recorded. Renaming it to
/// what it's called already changes nothing and records nothing (`None`). A
/// new name that differs only in letter case or trailing space is fine:
/// it's the same crate.
pub fn rename_on(
    conn: &mut Connection,
    id: CrateId,
    name: &str,
) -> Result<Done<Option<i64>>, OpsError> {
    let name = name.trim().to_owned();
    ops::record(
        conn,
        RENAME_OPERATION,
        &serde_json::json!({ "crateId": id.0, "name": name }),
        |rec| {
            if !is_static(rec, id)? {
                return Ok(Err(CratesError::NotFound));
            }
            let name = match checked_name(&top_level(rec, Some(id.0))?, &name) {
                Ok(name) => name,
                Err(refusal) => return Ok(Err(refusal)),
            };
            rec.set("crate", id.0, "name", name)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// Deletes a crate and its entries (the Library tracks stay). Recorded as
/// one operation; undo brings back the crate and every entry as they were.
pub fn delete_on(conn: &mut Connection, id: CrateId) -> Result<Done<()>, OpsError> {
    ops::record(
        conn,
        DELETE_OPERATION,
        &serde_json::json!({ "crateId": id.0 }),
        |rec| {
            if !is_static(rec, id)? {
                return Ok(Err(CratesError::NotFound));
            }
            // The entries first, one by one: the log doesn't record rows a
            // foreign key deletes along with the crate.
            let entries: Vec<i64> = rec.read_rows(
                "SELECT id FROM crate_entry WHERE crate_id = ?1 ORDER BY id",
                [id.0],
                |r| r.get(0),
            )?;
            for entry in entries {
                rec.delete("crate_entry", entry)?;
            }
            rec.delete("crate", id.0)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value)
}

/// Adds Library tracks to a crate. A track already in it is skipped (and a
/// track listed twice counts once). The entries get the time of the add, and
/// ties (a batch in one millisecond) are ordered by Library track id, as a
/// send orders them, not by the order given. Recorded as one
/// operation; if nothing was added, nothing is recorded.
pub fn add_on(
    conn: &mut Connection,
    id: CrateId,
    tracks: &[LibraryTrackId],
) -> Result<Done<Changed>, OpsError> {
    ops::record(
        conn,
        ADD_OPERATION,
        &serde_json::json!({ "crateId": id.0 }),
        |rec| {
            if !is_static(rec, id)? {
                return Ok(Err(CratesError::NotFound));
            }
            let library: HashSet<i64> = rec
                .read_rows("SELECT id FROM library_track", [], |r| r.get(0))?
                .into_iter()
                .collect();
            if tracks.iter().any(|t| !library.contains(&t.0)) {
                return Ok(Err(CratesError::TrackNotFound));
            }
            let mut present: HashSet<i64> = rec
                .read_rows(
                    "SELECT library_track_id FROM crate_entry WHERE crate_id = ?1",
                    [id.0],
                    |r| r.get(0),
                )?
                .into_iter()
                .collect();
            let mut added = 0;
            for track in tracks {
                if present.insert(track.0) {
                    rec.insert(
                        "crate_entry",
                        &[
                            ("crate_id", id.0.into()),
                            ("library_track_id", track.0.into()),
                        ],
                    )?;
                    added += 1;
                }
            }
            Ok(Ok(Changed {
                changed: added,
                skipped: tracks.len() as u32 - added,
                operation_id: None,
            }))
        },
    )
    .map(with_operation)
}

/// Takes tracks out of a crate; the Library tracks stay. A track that isn't
/// in the crate is skipped. Recorded as one operation; if nothing was
/// removed, nothing is recorded.
pub fn remove_on(
    conn: &mut Connection,
    id: CrateId,
    tracks: &[LibraryTrackId],
) -> Result<Done<Changed>, OpsError> {
    ops::record(
        conn,
        REMOVE_OPERATION,
        &serde_json::json!({ "crateId": id.0 }),
        |rec| {
            if !is_static(rec, id)? {
                return Ok(Err(CratesError::NotFound));
            }
            let entries: HashMap<i64, i64> = rec
                .read_rows(
                    "SELECT library_track_id, id FROM crate_entry WHERE crate_id = ?1",
                    [id.0],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?
                .into_iter()
                .collect();
            let mut removed = HashSet::new();
            for track in tracks {
                if let Some(&entry) = entries.get(&track.0) {
                    if removed.insert(track.0) {
                        rec.delete("crate_entry", entry)?;
                    }
                }
            }
            let removed = removed.len() as u32;
            Ok(Ok(Changed {
                changed: removed,
                skipped: tracks.len() as u32 - removed,
                operation_id: None,
            }))
        },
    )
    .map(with_operation)
}

/// Puts the operation a write recorded (if it recorded one) into its answer.
fn with_operation(recorded: ops::Recorded<Done<Changed>>) -> Done<Changed> {
    recorded.value.map(|changed| Changed {
        operation_id: recorded.operation_id,
        ..changed
    })
}

/// Every crate with its track count, in the order a send writes them.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<Crate>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name,
                (SELECT count(*) FROM crate_entry e
                 WHERE e.crate_id = c.id AND e.kind = 'member')
         FROM crate c WHERE c.kind = 'static' ORDER BY c.position, c.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Crate {
            id: CrateId(r.get(0)?),
            name: r.get(1)?,
            track_count: r.get(2)?,
        })
    })?;
    rows.collect()
}

/// The Library tracks in a crate, in the order they were added, or `None`
/// if there's no such crate. Ids only: [`ordered_tracks`] makes them rows.
pub fn track_ids(conn: &Connection, id: CrateId) -> rusqlite::Result<Option<Vec<LibraryTrackId>>> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM crate WHERE id = ?1 AND kind = 'static')",
        [id.0],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    // The order a send writes them in (`send::crate_tree`).
    let mut stmt = conn.prepare(
        "SELECT library_track_id FROM crate_entry
         WHERE crate_id = ?1 AND kind = 'member' ORDER BY added_at, library_track_id",
    )?;
    let ids = stmt
        .query_map([id.0], |r| Ok(LibraryTrackId(r.get(0)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(ids))
}

/// `ids` as the Library list shows its rows, in the order of `ids`.
pub fn ordered_tracks(
    ids: &[LibraryTrackId],
    stored: &[StoredTrack],
    located: &[Located],
    volumes: &impl Volumes,
    dirs: &FragileDirs,
) -> Vec<LibraryTrack> {
    let fragile = fragile::judge(located, volumes, dirs);
    let by_id: HashMap<LibraryTrackId, &StoredTrack> = stored.iter().map(|t| (t.id, t)).collect();
    ids.iter()
        .filter_map(|id| by_id.get(id))
        .map(|track| track.to_library_track(volumes, &fragile))
        .collect()
}

// `async` runs the writes off the main thread, like the Library's commands.

/// Makes a crate. The name is trimmed.
#[tauri::command]
#[specta::specta]
pub async fn create_crate(writer: State<'_, Writer>, name: String) -> Result<CrateId, IpcError> {
    Ok(writer.call(move |conn| Ok(create_on(conn, &name)))???)
}

/// Renames a crate. Answers the operation recorded, or none when the name
/// is the one it has.
#[tauri::command]
#[specta::specta]
pub async fn rename_crate(
    writer: State<'_, Writer>,
    id: CrateId,
    name: String,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(rename_on(conn, id, &name)))???)
}

/// Deletes a crate. Its Library tracks stay in the Library.
#[tauri::command]
#[specta::specta]
pub async fn delete_crate(writer: State<'_, Writer>, id: CrateId) -> Result<(), IpcError> {
    Ok(writer.call(move |conn| Ok(delete_on(conn, id)))???)
}

/// Adds Library tracks to a crate. A track that's in it already is skipped.
#[tauri::command]
#[specta::specta]
pub async fn add_tracks_to_crate(
    writer: State<'_, Writer>,
    id: CrateId,
    tracks: Vec<LibraryTrackId>,
) -> Result<Changed, IpcError> {
    Ok(writer.call(move |conn| Ok(add_on(conn, id, &tracks)))???)
}

/// Takes tracks out of a crate. They stay in the Library.
#[tauri::command]
#[specta::specta]
pub async fn remove_tracks_from_crate(
    writer: State<'_, Writer>,
    id: CrateId,
    tracks: Vec<LibraryTrackId>,
) -> Result<Changed, IpcError> {
    Ok(writer.call(move |conn| Ok(remove_on(conn, id, &tracks)))???)
}

/// Every crate with its track count.
#[tauri::command]
#[specta::specta]
pub async fn list_crates(reads: State<'_, ReadPool>) -> Result<Vec<Crate>, IpcError> {
    Ok(reads.read(list)?)
}

/// A crate's tracks, in the order they were added.
#[tauri::command]
#[specta::specta]
pub async fn crate_tracks(
    reads: State<'_, ReadPool>,
    id: CrateId,
) -> Result<Vec<LibraryTrack>, IpcError> {
    let volumes = system_volumes();
    let found = reads.read(|conn| {
        let Some(ids) = track_ids(conn, id)? else {
            return Ok(None);
        };
        let (stored, located) = library::stored_with_locations(conn, &volumes)?;
        Ok(Some((ids, stored, located)))
    })?;
    let Some((ids, stored, located)) = found else {
        return Err(CratesError::NotFound.into());
    };
    Ok(ordered_tracks(
        &ids,
        &stored,
        &located,
        &volumes,
        &FragileDirs::system(),
    ))
}

#[cfg(test)]
mod tests;
