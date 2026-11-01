//! Crates: hand-made groups of Library tracks, and the folders they live in
//! (1aG-7 and 1cA-9/10; ROADMAP 1.14, §2 `crate` and `crate_entry`). A
//! crate is static; a folder holds crates and folders, never tracks; the
//! tree is as deep as the XML writer allows. Notes and the summary line
//! are in [`summary`], sorting by fit in [`fit`]; smart crates come later.
//!
//! - **Every change is one recorded operation** ([`crate::ops`]), so
//!   `undo_last_operation` puts the earlier state back exactly, the entries'
//!   rows and `added_at` included. Deleting a crate deletes its entries
//!   first and explicitly: the log doesn't record rows a foreign key
//!   cascades to.
//! - **A name is trimmed**, can't be empty and can't equal another crate's
//!   or folder's in the same folder (the same name in different folders is
//!   fine), compared exactly as the XML writer compares siblings
//!   ([`sibling_key`]: NFC, letter case and trailing whitespace ignored). A
//!   name with a character XML can't carry is refused too, using the
//!   writer's own check. The point: a crate the app lets the user make is
//!   never the reason a send is refused (ROADMAP 1.9, "Crates and
//!   Playlists are always both written").
//! - **Deleting a folder deletes what is in it**, crates, entries and
//!   folders, deepest first and one row at a time (the log doesn't record
//!   rows a foreign key cascades to), so one undo puts back every row,
//!   position and entry as it was. Library tracks are never touched.
//! - **Moving** puts a crate or folder last in another folder, or at the
//!   top. A folder can't go into itself or its own subfolder, and nothing
//!   goes deeper than the writer allows, so a move never makes a send fail.
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

pub mod fit;
pub mod summary;

use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::{ErrorKind, ErrorParam, IpcError};
use crate::library::{self, LibraryTrackId};
use crate::ops::{self, OpsError};
use crate::rekordbox_write::{build, sibling_key, BuildError, Node, SendInput, MAX_DEPTH};

/// The operation kinds, as the operation log stores them.
pub const CREATE_OPERATION: &str = "create_crate";
pub const CREATE_FOLDER_OPERATION: &str = "create_crate_folder";
pub const RENAME_OPERATION: &str = "rename_crate";
pub const RENAME_FOLDER_OPERATION: &str = "rename_crate_folder";
pub const DELETE_OPERATION: &str = "delete_crate";
pub const DELETE_FOLDER_OPERATION: &str = "delete_crate_folder";
pub const MOVE_OPERATION: &str = "move_crate";
pub const ADD_OPERATION: &str = "add_to_crate";
pub const REMOVE_OPERATION: &str = "remove_from_crate";

/// A crate's row id.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct CrateId(pub i64);

/// What a row of the crate tree is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CrateKind {
    /// Holds crates and folders, never tracks.
    Folder,
    /// A hand-made crate of Library tracks.
    Static,
}

/// A crate or a folder, as the Crates screen and the sidebar list it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Crate {
    pub id: CrateId,
    /// The folder it is in; `None` at the top of the tree.
    pub parent_id: Option<CrateId>,
    pub kind: CrateKind,
    pub name: String,
    /// The tracks in a crate; 0 for a folder.
    pub track_count: u32,
}

/// What deleting a folder would delete, below the folder itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderContents {
    pub crates: u32,
    pub folders: u32,
    /// Different Library tracks in those crates. They stay in the Library.
    pub tracks: u32,
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

/// What adding tracks to a crate by their track id did (1cB-13, 1cB-23): the
/// table's rows are tracks. A track outside the Library joins the Library as
/// a linked Library track in the same operation, so one Undo takes both back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddedToCrate {
    /// How many tracks joined the crate, those that joined the Library with
    /// it included.
    pub added: u32,
    /// How many of those weren't in the Library: they were added to it too.
    pub also_added_to_library: u32,
    /// How many were in the crate already.
    pub already_there: u32,
    /// The tracks outside the Library that couldn't join it, so were left out.
    pub left_out: LeftOutOfCrate,
    /// The operation this recorded; `None` when nothing was added.
    pub operation_id: Option<i64>,
}

/// How many tracks outside the Library were left out of an add to a crate,
/// by why: the reasons "Add to Library" gives (1.8), which a crate add must
/// not get around. Each track is counted once, under one reason.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LeftOutOfCrate {
    /// The track has no file.
    pub no_file: u32,
    /// Its file isn't on disk.
    pub file_missing: u32,
    /// Its only rekordbox match is a probable one, not confirmed yet.
    pub match_not_confirmed: u32,
    /// Its file has the low bitrate finding, which "Add to Library" asks the
    /// user to agree to; an add to a crate can't ask, so it is added by hand.
    pub low_bitrate: u32,
}

/// What adding tracks to a new crate did: the crate, and the add.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddedToNewCrate {
    pub crate_id: CrateId,
    /// The name as stored (trimmed).
    pub name: String,
    pub added: AddedToCrate,
}

/// Why a crate command was refused. Nothing was changed.
#[derive(Debug)]
pub enum CratesError {
    /// There's no such crate.
    NotFound,
    /// There's no such folder (what [`CratesError::NotFound`] says when the
    /// item acted on is a folder).
    FolderNotFound,
    /// A Library track to add doesn't exist.
    TrackNotFound,
    /// The name is empty once trimmed.
    NameEmpty,
    /// [`CratesError::NameEmpty`] for a folder.
    FolderNameEmpty,
    /// Another crate or folder in the same folder has this name. `existing` is that crate's
    /// name as stored, which may differ in letter case.
    NameTaken { existing: String },
    /// [`CratesError::NameTaken`] for a folder.
    FolderNameTaken { existing: String },
    /// The name holds a character XML can't carry, so a send couldn't write
    /// it.
    NameUnsendable,
    /// A note is longer than a note can be.
    NotesTooLong,
    /// The place to put a crate or folder isn't a folder.
    NotAFolder,
    /// A folder can't go inside itself or one of its own subfolders.
    MoveIntoItself,
    /// The folders would nest deeper than a send can write.
    TooDeep,
    /// The database or the operation log failed.
    Ops(OpsError),
}

impl CratesError {
    /// The refusal as it is worded when the item acted on is `kind`: a
    /// folder's own words for a missing item, an empty name and a taken name.
    /// Any other refusal, and any other kind, is left as it is.
    fn for_kind(self, kind: Option<CrateKind>) -> Self {
        if kind != Some(CrateKind::Folder) {
            return self;
        }
        match self {
            CratesError::NotFound => CratesError::FolderNotFound,
            CratesError::NameEmpty => CratesError::FolderNameEmpty,
            CratesError::NameTaken { existing } => CratesError::FolderNameTaken { existing },
            other => other,
        }
    }
}

/// A command's answer, worded for the kind of item it acted on.
fn worded<T>(done: Done<T>, kind: Option<CrateKind>) -> Done<T> {
    done.map_err(|refusal| refusal.for_kind(kind))
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
            CratesError::FolderNotFound => IpcError::new(ErrorKind::FolderNotFound),
            CratesError::TrackNotFound => IpcError::new(ErrorKind::LibraryTrackNotFound),
            CratesError::NameEmpty => IpcError::new(ErrorKind::CrateNameEmpty),
            CratesError::FolderNameEmpty => IpcError::new(ErrorKind::FolderNameEmpty),
            CratesError::NameTaken { existing } => IpcError::with(
                ErrorKind::CrateNameTaken,
                [("name", ErrorParam::from(existing))],
            ),
            CratesError::FolderNameTaken { existing } => IpcError::with(
                ErrorKind::FolderNameTaken,
                [("name", ErrorParam::from(existing))],
            ),
            CratesError::NameUnsendable => IpcError::new(ErrorKind::CrateNameUnsendable),
            CratesError::NotesTooLong => IpcError::new(ErrorKind::CrateNotesTooLong),
            CratesError::NotAFolder => IpcError::new(ErrorKind::CrateNotAFolder),
            CratesError::MoveIntoItself => IpcError::new(ErrorKind::CrateMoveIntoItself),
            CratesError::TooDeep => IpcError::new(ErrorKind::CrateTooDeep),
            CratesError::Ops(e) => IpcError::from(e),
        }
    }
}

/// A command's answer: the refusal, or what it did.
type Done<T> = Result<T, CratesError>;

/// The name as it will be stored, if a send can carry it next to
/// `siblings` (the other crates' and folders' ids and names in the same
/// folder).
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

/// The ids and names of everything in `parent` (`None`: the top of the
/// tree), other than `except`: the names a send compares as siblings.
/// Smart crates (Phase 3) count too, as they do for the unique index on
/// `(parent_id, name)` and for [`name_clash`], so a name this accepts is a
/// name the database accepts.
fn siblings(
    rec: &ops::Recorder<'_>,
    parent: Option<i64>,
    except: Option<i64>,
) -> Result<Vec<(i64, String)>, OpsError> {
    rec.read_rows(
        "SELECT id, name FROM crate
         WHERE parent_id IS ?1 AND id <> ?2 ORDER BY id",
        (parent, except.unwrap_or(0)),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// A crate or folder as the commands need it.
struct Found {
    kind: CrateKind,
    parent: Option<i64>,
    name: String,
}

fn kind_from(stored: &str) -> CrateKind {
    if stored == "folder" {
        CrateKind::Folder
    } else {
        CrateKind::Static
    }
}

/// The crate or folder `id`, or `None` if there is none (a smart crate
/// counts as none: nothing here handles it).
fn found(rec: &ops::Recorder<'_>, id: CrateId) -> Result<Option<Found>, OpsError> {
    Ok(rec
        .read_rows(
            "SELECT kind, parent_id, name FROM crate
             WHERE id = ?1 AND kind IN ('folder', 'static')",
            [id.0],
            |r| {
                Ok(Found {
                    kind: kind_from(&r.get::<_, String>(0)?),
                    parent: r.get(1)?,
                    name: r.get(2)?,
                })
            },
        )?
        .pop())
}

/// A static crate's name, or `None` if `id` isn't a static crate.
fn static_name(rec: &ops::Recorder<'_>, id: CrateId) -> Result<Option<String>, OpsError> {
    Ok(found(rec, id)?
        .filter(|f| f.kind == CrateKind::Static)
        .map(|f| f.name))
}

/// The kind of crate `id` is, read before the operation is named.
fn kind_of(conn: &Connection, id: CrateId) -> rusqlite::Result<Option<CrateKind>> {
    let mut stmt = conn
        .prepare_cached("SELECT kind FROM crate WHERE id = ?1 AND kind IN ('folder', 'static')")?;
    let mut rows = stmt.query([id.0])?;
    match rows.next()? {
        Some(row) => Ok(Some(kind_from(&row.get::<_, String>(0)?))),
        None => Ok(None),
    }
}

/// The most folders one inside the other that a send writes: the writer
/// counts the `Crates` folder as the first level of a path.
const MAX_FOLDER_DEPTH: usize = MAX_DEPTH - 1;

/// How many folders deep `folder` sits: 0 for the top of the tree, 1 for a
/// folder at the top, and so on.
fn depth_of(rec: &ops::Recorder<'_>, folder: i64) -> Result<usize, OpsError> {
    let above: i64 = rec
        .read_rows(
            "SELECT count(*) FROM crate_ancestor WHERE crate_id = ?1",
            [folder],
            |r| r.get(0),
        )?
        .pop()
        .unwrap_or(0);
    Ok(above as usize + 1)
}

/// How many levels of folders `id` and what is below it make: 0 for a
/// crate, 1 for a folder with no folder in it, and so on.
fn levels_of(rec: &ops::Recorder<'_>, id: i64) -> Result<usize, OpsError> {
    let levels: i64 = rec
        .read_rows(
            "WITH RECURSIVE down (id, level) AS (
                 SELECT id, 1 FROM crate WHERE id = ?1 AND kind = 'folder'
                 UNION ALL
                 SELECT c.id, down.level + 1 FROM crate c JOIN down ON c.parent_id = down.id
                 WHERE c.kind = 'folder'
             )
             SELECT ifnull(max(level), 0) FROM down",
            [id],
            |r| r.get(0),
        )?
        .pop()
        .unwrap_or(0);
    Ok(levels as usize)
}

/// Checks that `parent` can hold a new or moved node `levels` folder
/// levels tall ([`levels_of`]): it is a folder, and the deepest folder ends
/// up no deeper than the writer writes. Answers the parent's id.
fn room_in(
    rec: &ops::Recorder<'_>,
    parent: Option<CrateId>,
    levels: usize,
) -> Result<Done<Option<i64>>, OpsError> {
    let Some(parent) = parent else {
        return Ok(if levels > MAX_FOLDER_DEPTH {
            Err(CratesError::TooDeep)
        } else {
            Ok(None)
        });
    };
    match found(rec, parent)? {
        None => return Ok(Err(CratesError::NotFound)),
        Some(f) if f.kind != CrateKind::Folder => return Ok(Err(CratesError::NotAFolder)),
        Some(_) => {}
    }
    if depth_of(rec, parent.0)? + levels > MAX_FOLDER_DEPTH {
        return Ok(Err(CratesError::TooDeep));
    }
    Ok(Ok(Some(parent.0)))
}

/// The position after the last of the nodes in `parent`.
fn next_position(rec: &ops::Recorder<'_>, parent: Option<i64>) -> Result<i64, OpsError> {
    Ok(rec
        .read_rows(
            "SELECT ifnull(max(position) + 1, 0) FROM crate WHERE parent_id IS ?1",
            [parent],
            |r| r.get(0),
        )?
        .pop()
        .unwrap_or(0))
}

/// The first of the crates `ids` whose name equals a sibling's by the
/// writer's rule ([`sibling_key`]), with that name. Undo asks before it
/// brings a crate or an earlier name back: such a pair would make a send
/// fail. Ids that name no crate are skipped.
pub fn name_clash(conn: &Connection, ids: &[i64]) -> rusqlite::Result<Option<(i64, String)>> {
    // `main.`, as the log itself names tables: a TEMP table called `crate`
    // can't stand in.
    let mut own = conn.prepare_cached("SELECT parent_id, name FROM main.crate WHERE id = ?1")?;
    let mut siblings =
        conn.prepare_cached("SELECT name FROM main.crate WHERE parent_id IS ?1 AND id <> ?2")?;
    for &id in ids {
        let mut rows = own.query([id])?;
        let Some(row) = rows.next()? else {
            continue;
        };
        let (parent, name): (Option<i64>, String) = (row.get(0)?, row.get(1)?);
        let key = sibling_key(&name);
        let mut others = siblings.query((parent, id))?;
        while let Some(other) = others.next()? {
            if sibling_key(other.get_ref(0)?.as_str()?) == key {
                return Ok(Some((id, name)));
            }
        }
    }
    Ok(None)
}

/// Makes a static crate at the top of the tree, last among its siblings.
/// Recorded as one operation.
pub fn create_on(conn: &mut Connection, name: &str) -> Result<Done<CrateId>, OpsError> {
    make_on(conn, CrateKind::Static, None, name)
}

/// Makes a static crate in `parent` (`None`: the top), last among its
/// siblings. Recorded as one operation.
pub fn create_in(
    conn: &mut Connection,
    parent: Option<CrateId>,
    name: &str,
) -> Result<Done<CrateId>, OpsError> {
    make_on(conn, CrateKind::Static, parent, name)
}

/// Makes an empty folder in `parent` (`None`: the top), last among its
/// siblings. Recorded as one operation.
pub fn create_folder_on(
    conn: &mut Connection,
    parent: Option<CrateId>,
    name: &str,
) -> Result<Done<CrateId>, OpsError> {
    make_on(conn, CrateKind::Folder, parent, name)
}

fn make_on(
    conn: &mut Connection,
    kind: CrateKind,
    parent: Option<CrateId>,
    name: &str,
) -> Result<Done<CrateId>, OpsError> {
    let name = name.trim().to_owned();
    let operation = match kind {
        CrateKind::Static => CREATE_OPERATION,
        CrateKind::Folder => CREATE_FOLDER_OPERATION,
    };
    ops::record(
        conn,
        operation,
        &serde_json::json!({ "name": name, "parentId": parent.map(|p| p.0) }),
        |rec| make_in(rec, kind, parent, &name),
    )
    .map(|recorded| worded(recorded.value, Some(kind)))
}

/// The writes of [`make_on`], inside the operation `rec` is recording. Every
/// refusal comes before the first write.
fn make_in(
    rec: &mut ops::Recorder<'_>,
    kind: CrateKind,
    parent: Option<CrateId>,
    name: &str,
) -> Result<Done<CrateId>, OpsError> {
    let stored_kind = match kind {
        CrateKind::Static => "static",
        CrateKind::Folder => "folder",
    };
    let levels = usize::from(kind == CrateKind::Folder);
    let parent = match room_in(rec, parent, levels)? {
        Ok(parent) => parent,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let name = match checked_name(&siblings(rec, parent, None)?, name) {
        Ok(name) => name,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let position = next_position(rec, parent)?;
    let id = rec.insert(
        "crate",
        &[
            ("parent_id", parent.into()),
            ("kind", stored_kind.to_owned().into()),
            ("name", name.into()),
            ("position", position.into()),
        ],
    )?;
    Ok(Ok(CrateId(id)))
}

/// Renames a crate or folder and returns the operation it recorded.
/// Renaming it to what it's called already changes nothing and records
/// nothing (`None`). A new name that differs only in letter case or trailing
/// space is fine: it's the same crate. The name must differ from the others
/// in the same folder.
pub fn rename_on(
    conn: &mut Connection,
    id: CrateId,
    name: &str,
) -> Result<Done<Option<i64>>, OpsError> {
    let name = name.trim().to_owned();
    let kind = kind_of(conn, id)?;
    let operation = match kind {
        Some(CrateKind::Folder) => RENAME_FOLDER_OPERATION,
        _ => RENAME_OPERATION,
    };
    ops::record(
        conn,
        operation,
        &serde_json::json!({ "crateId": id.0, "name": name }),
        |rec| {
            let Some(from) = found(rec, id)? else {
                return Ok(Err(CratesError::NotFound));
            };
            let name = match checked_name(&siblings(rec, from.parent, Some(id.0))?, &name) {
                Ok(name) => name,
                Err(refusal) => return Ok(Err(refusal)),
            };
            rec.detail("from", from.name);
            rec.set("crate", id.0, "name", name)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| worded(recorded.value, kind).map(|()| recorded.operation_id))
}

/// Deletes a crate, or a folder with everything in it, and the entries
/// (the Library tracks stay). Recorded as one operation; undo brings back
/// every crate, folder and entry as it was, with its position.
pub fn delete_on(conn: &mut Connection, id: CrateId) -> Result<Done<()>, OpsError> {
    let operation = match kind_of(conn, id)? {
        Some(CrateKind::Folder) => DELETE_FOLDER_OPERATION,
        _ => DELETE_OPERATION,
    };
    ops::record(
        conn,
        operation,
        &serde_json::json!({ "crateId": id.0 }),
        |rec| {
            let Some(top) = found(rec, id)? else {
                return Ok(Err(CratesError::NotFound));
            };
            rec.detail("name", top.name);
            // Everything below, deepest first, so a row goes before the
            // folder it is in (`parent_id` is RESTRICT). Undo puts them back
            // in the opposite order: folders before what is in them.
            let doomed: Vec<i64> = rec.read_rows(
                "WITH RECURSIVE down (id, level) AS (
                     SELECT id, 0 FROM crate WHERE id = ?1
                     UNION ALL
                     SELECT c.id, down.level + 1 FROM crate c JOIN down ON c.parent_id = down.id
                 )
                 SELECT id FROM down ORDER BY level DESC, id DESC",
                [id.0],
                |r| r.get(0),
            )?;
            for node in doomed {
                // The entries first, one by one: the log doesn't record rows
                // a foreign key deletes along with the crate.
                let entries: Vec<i64> = rec.read_rows(
                    "SELECT id FROM crate_entry WHERE crate_id = ?1 ORDER BY id",
                    [node],
                    |r| r.get(0),
                )?;
                for entry in entries {
                    rec.delete("crate_entry", entry)?;
                }
                rec.delete("crate", node)?;
            }
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value)
}

/// Moves a crate or folder, with everything in it, into the folder
/// `folder` (`None`: to the top), last among what is there. Moving it
/// where it is already changes nothing and records nothing (`None`).
/// Refused: into itself or a folder inside it, into a crate, a name the
/// folder has already, or deeper than a send can write.
pub fn move_on(
    conn: &mut Connection,
    id: CrateId,
    folder: Option<CrateId>,
) -> Result<Done<Option<i64>>, OpsError> {
    let kind = kind_of(conn, id)?;
    ops::record(
        conn,
        MOVE_OPERATION,
        &serde_json::json!({ "crateId": id.0, "folderId": folder.map(|f| f.0) }),
        |rec| {
            let Some(node) = found(rec, id)? else {
                return Ok(Err(CratesError::NotFound));
            };
            if let Some(target) = folder {
                let inside: bool = rec
                    .read_rows(
                        "SELECT ?1 = ?2 OR EXISTS (SELECT 1 FROM crate_ancestor
                                                   WHERE crate_id = ?2 AND ancestor_id = ?1)",
                        (id.0, target.0),
                        |r| r.get(0),
                    )?
                    .pop()
                    .unwrap_or(false);
                if inside {
                    return Ok(Err(CratesError::MoveIntoItself));
                }
            }
            let levels = levels_of(rec, id.0)?;
            let parent = match room_in(rec, folder, levels)? {
                Ok(parent) => parent,
                Err(refusal) => return Ok(Err(refusal)),
            };
            if parent == node.parent {
                return Ok(Ok(()));
            }
            if let Err(refusal) = checked_name(&siblings(rec, parent, Some(id.0))?, &node.name) {
                return Ok(Err(refusal));
            }
            let position = next_position(rec, parent)?;
            rec.detail("name", node.name);
            rec.set("crate", id.0, "parent_id", parent)?;
            rec.set("crate", id.0, "position", position)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| worded(recorded.value, kind).map(|()| recorded.operation_id))
}

/// What deleting folder `id` would delete below it, or `None` if there is
/// no such folder.
pub fn folder_contents(conn: &Connection, id: CrateId) -> rusqlite::Result<Option<FolderContents>> {
    let (all, folders, crates, tracks) = conn.query_row(
        "WITH RECURSIVE down (id) AS (
             SELECT id FROM crate WHERE id = ?1 AND kind = 'folder'
             UNION ALL
             SELECT c.id FROM crate c JOIN down ON c.parent_id = down.id
         )
         SELECT
             (SELECT count(*) FROM down),
             (SELECT count(*) FROM crate WHERE id IN (SELECT id FROM down) AND id <> ?1
                                           AND kind = 'folder'),
             (SELECT count(*) FROM crate WHERE id IN (SELECT id FROM down) AND kind <> 'folder'),
             (SELECT count(DISTINCT library_track_id) FROM crate_entry
              WHERE crate_id IN (SELECT id FROM down))",
        [id.0],
        |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, u32>(3)?,
            ))
        },
    )?;
    Ok((all > 0).then_some(FolderContents {
        crates,
        folders,
        tracks,
    }))
}

/// How many ids one check of "are these Library tracks?" asks about.
const LIBRARY_CHECK_CHUNK: usize = 1000;

/// Whether every one of `tracks` is a Library track. Asks about the ids
/// given, a chunk at a time, not about the whole Library.
fn all_in_library(rec: &ops::Recorder<'_>, tracks: &[LibraryTrackId]) -> Result<bool, OpsError> {
    let mut wanted: Vec<i64> = tracks.iter().map(|t| t.0).collect();
    wanted.sort_unstable();
    wanted.dedup();
    for chunk in wanted.chunks(LIBRARY_CHECK_CHUNK) {
        let ids = serde_json::to_string(chunk).unwrap_or_else(|_| "[]".into());
        let found = rec.read_rows(
            "SELECT count(*) FROM library_track
             WHERE id IN (SELECT value FROM json_each(?1))",
            [ids],
            |r| r.get::<_, i64>(0),
        )?;
        if found != [chunk.len() as i64] {
            return Ok(false);
        }
    }
    Ok(true)
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
            let Some(name) = static_name(rec, id)? else {
                return Ok(Err(CratesError::NotFound));
            };
            if !all_in_library(rec, tracks)? {
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
            rec.detail("name", name);
            rec.detail("tracks", added);
            Ok(Ok(Changed {
                changed: added,
                skipped: tracks.len() as u32 - added,
                operation_id: None,
            }))
        },
    )
    .map(with_operation)
}

/// One track of an add by track id, once it is known what it will be.
enum Item {
    /// A Library track (its id).
    InLibrary(i64),
    /// A track outside the Library, to be linked to this file.
    Link { recording: i64, file: i64 },
}

/// What an add by track id will do, decided before anything is written.
struct Prepared {
    /// The tracks that will join, each once, in the order given.
    items: Vec<Item>,
    left_out: LeftOutOfCrate,
}

/// Sorts `recordings` the way "Add to Library" would judge them (1.8): a
/// Library track joins as it is; a track outside it joins the Library too,
/// unless the Library add refuses it or asks the user to agree to a warning
/// first (then it is left out, counted by reason). A track that doesn't exist
/// refuses the whole call. Changes nothing the log records.
fn prepare(conn: &mut Connection, recordings: &[i64]) -> Result<Done<Prepared>, OpsError> {
    let mut seen = HashSet::new();
    let mut prepared = Prepared {
        items: Vec::new(),
        left_out: LeftOutOfCrate::default(),
    };
    for &recording in recordings {
        if !seen.insert(recording) {
            continue;
        }
        match library::check(conn, recording)? {
            Ok(library::Plan::AlreadyThere(track)) => {
                prepared.items.push(Item::InLibrary(track.0));
            }
            Ok(library::Plan::Link(choice)) => {
                // Whether it carries the low-bitrate warning is asked for
                // all of them at once, below.
                prepared.items.push(Item::Link {
                    recording,
                    file: choice.file_id,
                });
            }
            Err(library::Refusal::TrackNotFound) => return Ok(Err(CratesError::TrackNotFound)),
            Err(library::Refusal::NoFile) => prepared.left_out.no_file += 1,
            Err(library::Refusal::FileMissing { .. }) => prepared.left_out.file_missing += 1,
            Err(library::Refusal::MatchNotConfirmed) => prepared.left_out.match_not_confirmed += 1,
        }
    }
    // One settle and one read of the findings for the whole list, not one of
    // each per track. A track whose file has the warning is left out.
    let files: Vec<i64> = prepared
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Link { file, .. } => Some(*file),
            Item::InLibrary(_) => None,
        })
        .collect();
    let warned = library::warnings_for(conn, &files)?;
    if !warned.is_empty() {
        let before = prepared.items.len();
        prepared.items.retain(|item| match item {
            Item::Link { file, .. } => !warned.contains_key(file),
            Item::InLibrary(_) => true,
        });
        prepared.left_out.low_bitrate += (before - prepared.items.len()) as u32;
    }
    Ok(Ok(prepared))
}

/// The writes of an add by track id, inside the operation `rec` is
/// recording: links the tracks outside the Library (the file is only linked,
/// never written), then puts every track not in the crate yet into it.
fn put_in(
    rec: &mut ops::Recorder<'_>,
    id: CrateId,
    name: String,
    prepared: &Prepared,
) -> Result<AddedToCrate, OpsError> {
    let mut present: HashSet<i64> = rec
        .read_rows(
            "SELECT library_track_id FROM crate_entry WHERE crate_id = ?1",
            [id.0],
            |r| r.get(0),
        )?
        .into_iter()
        .collect();
    let (mut added, mut also_added_to_library, mut already_there) = (0, 0, 0);
    for item in &prepared.items {
        let track = match item {
            Item::InLibrary(track) => *track,
            Item::Link { recording, file } => {
                also_added_to_library += 1;
                library::insert_linked(rec, *recording, *file)?.0
            }
        };
        if present.insert(track) {
            rec.insert(
                "crate_entry",
                &[
                    ("crate_id", id.0.into()),
                    ("library_track_id", track.into()),
                ],
            )?;
            added += 1;
        } else {
            already_there += 1;
        }
    }
    rec.detail("name", name);
    rec.detail("tracks", added);
    // What the Undo button names (1cB-23): how many came into the Library.
    rec.detail("newToLibrary", also_added_to_library);
    Ok(AddedToCrate {
        added,
        also_added_to_library,
        already_there,
        left_out: prepared.left_out,
        operation_id: None,
    })
}

/// Adds tracks to a crate by their track id (`TableRow.recordingId`, 1cB-13).
/// A track outside the Library is added to the Library as well (1cB-23): a
/// linked Library track, as "Add to Library" makes it, no file touched. One
/// that "Add to Library" refuses is left out and counted by reason
/// ([`LeftOutOfCrate`]). A track already in the crate is skipped, and one listed
/// twice counts once. A track that doesn't exist refuses the whole call
/// before anything is added. One operation for everything added, so one Undo
/// takes the tracks out of the crate and out of the Library again; nothing
/// added, nothing recorded.
pub fn add_recordings_on(
    conn: &mut Connection,
    id: CrateId,
    recordings: &[i64],
) -> Result<Done<AddedToCrate>, OpsError> {
    if kind_of(conn, id)? != Some(CrateKind::Static) {
        return Ok(Err(CratesError::NotFound));
    }
    let prepared = match prepare(conn, recordings)? {
        Ok(prepared) => prepared,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let recorded = ops::record(
        conn,
        ADD_OPERATION,
        &serde_json::json!({ "crateId": id.0 }),
        |rec| {
            let Some(name) = static_name(rec, id)? else {
                return Ok(Err(CratesError::NotFound));
            };
            put_in(rec, id, name, &prepared).map(Ok)
        },
    )?;
    Ok(recorded.value.map(|done| AddedToCrate {
        operation_id: recorded.operation_id,
        ..done
    }))
}

/// Makes a static crate at the top of the tree and adds tracks to it, as
/// [`add_recordings_on`] does, in ONE operation: one Undo takes the tracks,
/// the Library tracks that came with them and the crate away. A refusal (the
/// name, a track that doesn't exist) comes before anything is written, so it
/// leaves no crate behind.
pub fn add_recordings_to_new_crate_on(
    conn: &mut Connection,
    name: &str,
    recordings: &[i64],
) -> Result<Done<AddedToNewCrate>, OpsError> {
    let prepared = match prepare(conn, recordings)? {
        Ok(prepared) => prepared,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let name = name.trim().to_owned();
    let recorded = ops::record(
        conn,
        ADD_OPERATION,
        &serde_json::json!({ "newCrate": true }),
        |rec| {
            let id = match make_in(rec, CrateKind::Static, None, &name)? {
                Ok(id) => id,
                Err(refusal) => return Ok(Err(refusal)),
            };
            let stored = static_name(rec, id)?.unwrap_or_else(|| name.clone());
            let added = put_in(rec, id, stored.clone(), &prepared)?;
            Ok(Ok(AddedToNewCrate {
                crate_id: id,
                name: stored,
                added,
            }))
        },
    )?;
    Ok(recorded.value.map(|mut done| {
        done.added.operation_id = recorded.operation_id;
        done
    }))
}

/// The crates (not folders) a track is in, by id: the answer to "which
/// crates is this track in?" for the Details panel (1cB-14). `recording` is
/// the track's id (`TableRow.recordingId`); a track outside the Library is in
/// no crate. The caller puts them in tree order.
pub fn crates_of(conn: &Connection, recording: i64) -> rusqlite::Result<Vec<CrateId>> {
    let mut stmt = conn.prepare_cached(
        "SELECT c.id FROM crate_entry e
         JOIN library_track l ON l.id = e.library_track_id
         JOIN crate c ON c.id = e.crate_id
         WHERE l.recording_id = ?1 AND e.kind = 'member' AND c.kind = 'static'
         ORDER BY c.id",
    )?;
    let ids = stmt
        .query_map([recording], |r| Ok(CrateId(r.get(0)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
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
            let Some(name) = static_name(rec, id)? else {
                return Ok(Err(CratesError::NotFound));
            };
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
            rec.detail("name", name);
            rec.detail("tracks", removed);
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

/// Every folder and crate with its track count. Each folder's own are in
/// the order a send writes them (`position`, then id); the caller builds the
/// tree from `parent_id`.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<Crate>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.parent_id, c.kind, c.name,
                (SELECT count(*) FROM crate_entry e
                 WHERE e.crate_id = c.id AND e.kind = 'member')
         FROM crate c WHERE c.kind IN ('folder', 'static')
         ORDER BY ifnull(c.parent_id, 0), c.position, c.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Crate {
            id: CrateId(r.get(0)?),
            parent_id: r.get::<_, Option<i64>>(1)?.map(CrateId),
            kind: kind_from(&r.get::<_, String>(2)?),
            name: r.get(3)?,
            track_count: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// The Library tracks in a crate, in the order they were added, or `None`
/// if there's no such crate.
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

// `async` runs the writes off the main thread, like the Library's commands.

/// Makes a crate, in `parent` or at the top. The name is trimmed.
#[tauri::command]
#[specta::specta]
pub async fn create_crate(
    writer: State<'_, Writer>,
    name: String,
    parent: Option<CrateId>,
) -> Result<CrateId, IpcError> {
    Ok(writer.call(move |conn| Ok(create_in(conn, parent, &name)))???)
}

/// Makes an empty folder, in `parent` or at the top. The name is trimmed.
#[tauri::command]
#[specta::specta]
pub async fn create_crate_folder(
    writer: State<'_, Writer>,
    name: String,
    parent: Option<CrateId>,
) -> Result<CrateId, IpcError> {
    Ok(writer.call(move |conn| Ok(create_folder_on(conn, parent, &name)))???)
}

/// Moves a crate or folder into a folder, or to the top (`folder` none).
/// Answers the operation recorded, or none when it is there already.
#[tauri::command]
#[specta::specta]
pub async fn move_crate(
    writer: State<'_, Writer>,
    id: CrateId,
    folder: Option<CrateId>,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(move_on(conn, id, folder)))???)
}

/// What deleting a folder would delete: the crates and folders in it and
/// the different tracks in those crates.
#[tauri::command]
#[specta::specta]
pub async fn crate_folder_contents(
    reads: State<'_, ReadPool>,
    id: CrateId,
) -> Result<FolderContents, IpcError> {
    reads
        .read(|conn| folder_contents(conn, id))?
        .ok_or_else(|| CratesError::NotFound.into())
}

/// Renames a crate or folder. Answers the operation recorded, or none when
/// the name is the one it has.
#[tauri::command]
#[specta::specta]
pub async fn rename_crate(
    writer: State<'_, Writer>,
    id: CrateId,
    name: String,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(rename_on(conn, id, &name)))???)
}

/// Deletes a crate, or a folder with everything in it. The Library tracks
/// stay in the Library.
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

/// Adds tracks to a crate by their track id (a table row's `recordingId`).
/// A track outside the Library joins the Library too; one the Library add
/// refuses is left out and counted by reason; one in the crate already is
/// skipped. One operation for all of it.
#[tauri::command]
#[specta::specta]
pub async fn add_recordings_to_crate(
    writer: State<'_, Writer>,
    jobs: State<'_, crate::jobs::JobQueue>,
    id: CrateId,
    recordings: Vec<i64>,
) -> Result<AddedToCrate, IpcError> {
    let added = writer.call(move |conn| Ok(add_recordings_on(conn, id, &recordings)))???;
    if added.also_added_to_library > 0 {
        crate::waveform::ask_after_library_change(&writer, |j| jobs.enqueue(j));
    }
    Ok(added)
}

/// Makes a crate at the top of the tree and adds tracks to it, as
/// `add_recordings_to_crate` does, as one operation (one Undo).
#[tauri::command]
#[specta::specta]
pub async fn add_recordings_to_new_crate(
    writer: State<'_, Writer>,
    jobs: State<'_, crate::jobs::JobQueue>,
    name: String,
    recordings: Vec<i64>,
) -> Result<AddedToNewCrate, IpcError> {
    let made = writer
        .call(move |conn| Ok(add_recordings_to_new_crate_on(conn, &name, &recordings)))???;
    if made.added.also_added_to_library > 0 {
        crate::waveform::ask_after_library_change(&writer, |j| jobs.enqueue(j));
    }
    Ok(made)
}

/// The crates a track is in, for the Details panel.
#[tauri::command]
#[specta::specta]
pub async fn crates_of_track(
    reads: State<'_, ReadPool>,
    recording_id: i64,
) -> Result<Vec<CrateId>, IpcError> {
    Ok(reads.read(|conn| crates_of(conn, recording_id))?)
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

/// Every folder and crate with its track count.
#[tauri::command]
#[specta::specta]
pub async fn list_crates(reads: State<'_, ReadPool>) -> Result<Vec<Crate>, IpcError> {
    Ok(reads.read(list)?)
}

#[cfg(test)]
mod tests;
