//! Playlists (ROADMAP 3.5, §2 `playlist` and `playlist_entry`): ordered
//! lists of Library tracks, made in the app or copied from rekordbox
//! ([`import`]).
//!
//! - **Ordered.** Each entry has a `position`, dense from 0. New entries go
//!   last, or at a place the caller gives; a reorder moves entries and
//!   rewrites every position that changed, one `set` per row, so its Undo
//!   puts each back exactly. A track may be in a playlist more than once.
//! - **Every change is one recorded operation** ([`crate::ops`]) with its
//!   own kind for the Undo label. Deleting a playlist deletes its entries
//!   first and explicitly: the log doesn't record rows a foreign key
//!   cascades to.
//! - **A name is trimmed**, can't be empty, can't equal another playlist's
//!   or `Tags` (the tag playlists' folder) by the writer's sibling rule
//!   ([`sibling_key`]), and must be one the writer can carry. All of a
//!   send's playlists sit in one folder, so a name this accepts is never the
//!   reason a send is refused. The database holds the rule too
//!   (`playlist.name_key` is unique), so an Undo can't bring a taken name
//!   back.
//! - **The send** writes each playlist under the app's `Playlists` folder
//!   in position order, repeats kept ([`send_nodes`]). The app owns that
//!   folder: edits made to it in rekordbox are never taken in (the
//!   membership merge reads only `Crates`).
//! - **Conversions are copies**: "Save as crate" makes a static crate of
//!   the playlist's tracks, each once, its order dropped; "Start a playlist
//!   from this crate" makes a playlist of the crate's tracks in the order
//!   the caller gives (the view's sort), the rest after in the crate's
//!   order. Neither touches the source.
//!
//! Library tracks only. Nothing here touches a file on disk.

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

pub mod import;
pub mod timeline;

use crate::crates::{CrateId, CrateKind, CratesError};
use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::{ErrorKind, ErrorParam, IpcError};
use crate::library::LibraryTrackId;
use crate::ops::{self, OpsError, Recorder};
use crate::rekordbox_write::{sibling_key, Node};
use crate::track_table::{self, TableRow};

/// The operation kinds, as the operation log stores them.
pub const CREATE_OPERATION: &str = "create_playlist";
pub const RENAME_OPERATION: &str = "rename_playlist";
pub const DELETE_OPERATION: &str = "delete_playlist";
pub const ADD_OPERATION: &str = "add_to_playlist";
pub const REMOVE_OPERATION: &str = "remove_from_playlist";
pub const MOVE_OPERATION: &str = "move_in_playlist";
pub const SAVE_AS_CRATE_OPERATION: &str = "save_playlist_as_crate";
pub const FROM_CRATE_OPERATION: &str = "playlist_from_crate";
pub const FROM_REKORDBOX_OPERATION: &str = "playlist_from_rekordbox";

/// A playlist's row id.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct PlaylistId(pub i64);

/// One entry's row id: a track's place in a playlist (a track that is in it
/// twice has two).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct PlaylistEntryId(pub i64);

/// Where a playlist came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PlaylistOrigin {
    /// Made in the app (a conversion from a crate included).
    App,
    /// Copied once from a rekordbox playlist ([`import`]).
    Rekordbox,
}

/// A playlist, as the sidebar and the playlist screen list it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub origin: PlaylistOrigin,
    /// For a copy of a rekordbox playlist, its path in rekordbox (folder
    /// names down to the playlist), for display. `None` for the app's own.
    pub source_path: Option<Vec<String>>,
    /// How many entries it has, repeats counted.
    pub track_count: u32,
}

/// One entry of a playlist with the track's row as the track table shows
/// it (energy, key, BPM and length included).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItem {
    pub entry_id: PlaylistEntryId,
    /// From 0, dense.
    pub position: u32,
    pub row: TableRow,
}

/// A playlist and every entry, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistView {
    pub playlist: Playlist,
    pub items: Vec<PlaylistItem>,
}

/// What adding, removing or moving entries did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistChanged {
    /// Entries added, removed or moved.
    pub changed: u32,
    /// Entries asked about that aren't in the playlist, so were left alone.
    pub skipped: u32,
    /// The operation this recorded; `None` when nothing changed.
    pub operation_id: Option<i64>,
}

/// Why a playlist command was refused. Nothing was changed.
#[derive(Debug)]
pub enum PlaylistsError {
    NotFound,
    /// The entry to move the others before isn't in the playlist.
    EntryNotFound,
    /// A Library track to add doesn't exist.
    TrackNotFound,
    /// The name is empty once trimmed.
    NameEmpty,
    /// Another playlist (or the `Tags` folder) has this name; `existing` is
    /// that one's name as stored.
    NameTaken {
        existing: String,
    },
    /// The name holds a character XML can't carry.
    NameUnsendable,
    /// The newest rekordbox read has no playlist with entries at that path.
    RekordboxPlaylistNotFound,
    /// A conversion's crate side refused (no such crate, its name taken).
    Crate(CratesError),
    Ops(OpsError),
}

impl From<OpsError> for PlaylistsError {
    fn from(e: OpsError) -> Self {
        PlaylistsError::Ops(e)
    }
}

impl From<rusqlite::Error> for PlaylistsError {
    fn from(e: rusqlite::Error) -> Self {
        PlaylistsError::Ops(OpsError::from(e))
    }
}

impl From<DbError> for PlaylistsError {
    fn from(e: DbError) -> Self {
        PlaylistsError::Ops(OpsError::Db(e))
    }
}

impl From<PlaylistsError> for IpcError {
    fn from(e: PlaylistsError) -> Self {
        match e {
            PlaylistsError::NotFound => IpcError::new(ErrorKind::PlaylistNotFound),
            PlaylistsError::EntryNotFound => IpcError::new(ErrorKind::PlaylistEntryNotFound),
            PlaylistsError::TrackNotFound => IpcError::new(ErrorKind::LibraryTrackNotFound),
            PlaylistsError::NameEmpty => IpcError::new(ErrorKind::PlaylistNameEmpty),
            PlaylistsError::NameTaken { existing } => IpcError::with(
                ErrorKind::PlaylistNameTaken,
                [("name", ErrorParam::from(existing))],
            ),
            PlaylistsError::NameUnsendable => IpcError::new(ErrorKind::PlaylistNameUnsendable),
            PlaylistsError::RekordboxPlaylistNotFound => {
                IpcError::new(ErrorKind::RekordboxPlaylistNotFound)
            }
            PlaylistsError::Crate(e) => IpcError::from(e),
            PlaylistsError::Ops(e) => IpcError::from(e),
        }
    }
}

/// A command's answer: the refusal, or what it did.
pub type Done<T> = Result<T, PlaylistsError>;

/// The name and its sibling key as they will be stored, if no other
/// playlist (other than `except`) has it and a send can carry it.
fn checked_name(
    rec: &Recorder<'_>,
    name: &str,
    except: Option<PlaylistId>,
) -> Result<Done<(String, String)>, OpsError> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(Err(PlaylistsError::NameEmpty));
    }
    let key = sibling_key(name);
    let tags = crate::send::TAGS_FOLDER;
    if key == sibling_key(tags) {
        return Ok(Err(PlaylistsError::NameTaken {
            existing: tags.to_owned(),
        }));
    }
    if let Some(existing) = rec
        .read_rows(
            "SELECT name FROM playlist WHERE name_key = ?1 AND id <> ?2",
            (&key, except.map_or(0, |p| p.0)),
            |r| r.get::<_, String>(0),
        )?
        .pop()
    {
        return Ok(Err(PlaylistsError::NameTaken { existing }));
    }
    if !crate::vocab::sendable(name) {
        return Ok(Err(PlaylistsError::NameUnsendable));
    }
    Ok(Ok((name.to_owned(), key)))
}

/// The playlist's name, or `None` if there's no such playlist.
fn name_of(rec: &Recorder<'_>, id: PlaylistId) -> Result<Option<String>, OpsError> {
    Ok(rec
        .read_rows("SELECT name FROM playlist WHERE id = ?1", [id.0], |r| {
            r.get(0)
        })?
        .pop())
}

/// One entry as the writes need it.
#[derive(Debug, Clone, Copy)]
struct Entry {
    id: i64,
    track: i64,
    position: i64,
}

/// The playlist's entries in order.
fn entries_of(rec: &Recorder<'_>, id: PlaylistId) -> Result<Vec<Entry>, OpsError> {
    rec.read_rows(
        "SELECT id, library_track_id, position FROM playlist_entry
         WHERE playlist_id = ?1 ORDER BY position, id",
        [id.0],
        |r| {
            Ok(Entry {
                id: r.get(0)?,
                track: r.get(1)?,
                position: r.get(2)?,
            })
        },
    )
}

/// Gives `order` the positions 0, 1, 2… recording only the rows whose
/// position changes.
fn renumber(rec: &mut Recorder<'_>, order: &[Entry]) -> Result<(), OpsError> {
    for (at, entry) in order.iter().enumerate() {
        let at = at as i64;
        if entry.position != at {
            rec.set("playlist_entry", entry.id, "position", at)?;
        }
    }
    Ok(())
}

/// Closes the gaps in `playlists` left by deleted entries, as changes of
/// `rec`: what removing a track from the Library calls after deleting its
/// entries.
pub(crate) fn close_gaps(rec: &mut Recorder<'_>, playlists: &[i64]) -> Result<(), OpsError> {
    for &playlist in playlists {
        let order = entries_of(rec, PlaylistId(playlist))?;
        renumber(rec, &order)?;
    }
    Ok(())
}

/// Inserts `tracks` as entries of `id` from position `from`, as changes of
/// `rec`.
fn insert_entries(
    rec: &mut Recorder<'_>,
    id: PlaylistId,
    tracks: &[i64],
    from: usize,
) -> Result<(), OpsError> {
    for (at, &track) in tracks.iter().enumerate() {
        rec.insert(
            "playlist_entry",
            &[
                ("playlist_id", Value::Integer(id.0)),
                ("library_track_id", Value::Integer(track)),
                ("position", Value::Integer((from + at) as i64)),
            ],
        )?;
    }
    Ok(())
}

/// Makes a playlist named `name`, as changes of `rec`; `source_path` for a
/// copy of a rekordbox playlist. Every refusal comes before the write.
fn make_in(
    rec: &mut Recorder<'_>,
    name: &str,
    source_path: Option<&[String]>,
) -> Result<Done<PlaylistId>, OpsError> {
    let (name, key) = match checked_name(rec, name, None)? {
        Ok(checked) => checked,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let (origin, path) = match source_path {
        Some(path) => (
            "rekordbox",
            Value::Text(serde_json::to_string(path).expect("strings always serialize")),
        ),
        None => ("app", Value::Null),
    };
    let id = rec.insert(
        "playlist",
        &[
            ("name", Value::Text(name.clone())),
            ("name_key", Value::Text(key)),
            ("origin", Value::Text(origin.to_owned())),
            ("source_path", path),
        ],
    )?;
    rec.detail("name", name);
    Ok(Ok(PlaylistId(id)))
}

/// Makes an empty playlist. Recorded as one operation.
pub fn create_on(conn: &mut Connection, name: &str) -> Result<Done<PlaylistId>, OpsError> {
    let name = name.trim().to_owned();
    ops::record(
        conn,
        CREATE_OPERATION,
        &serde_json::json!({ "name": name }),
        |rec| make_in(rec, &name, None),
    )
    .map(|recorded| recorded.value)
}

/// Renames a playlist and returns the operation it recorded; `None` when it
/// has that name already.
pub fn rename_on(
    conn: &mut Connection,
    id: PlaylistId,
    name: &str,
) -> Result<Done<Option<i64>>, OpsError> {
    let name = name.trim().to_owned();
    ops::record(
        conn,
        RENAME_OPERATION,
        &serde_json::json!({ "playlistId": id.0, "name": name }),
        |rec| {
            let Some(from) = name_of(rec, id)? else {
                return Ok(Err(PlaylistsError::NotFound));
            };
            let (name, key) = match checked_name(rec, &name, Some(id))? {
                Ok(checked) => checked,
                Err(refusal) => return Ok(Err(refusal)),
            };
            rec.detail("from", from);
            rec.set("playlist", id.0, "name", name)?;
            rec.set("playlist", id.0, "name_key", key)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// Deletes a playlist and its entries, as one operation (Library tracks
/// stay). Returns the operation.
pub fn delete_on(conn: &mut Connection, id: PlaylistId) -> Result<Done<Option<i64>>, OpsError> {
    ops::record(
        conn,
        DELETE_OPERATION,
        &serde_json::json!({ "playlistId": id.0 }),
        |rec| {
            let Some(name) = name_of(rec, id)? else {
                return Ok(Err(PlaylistsError::NotFound));
            };
            let entries = entries_of(rec, id)?;
            for entry in &entries {
                rec.delete("playlist_entry", entry.id)?;
            }
            rec.delete("playlist", id.0)?;
            rec.detail("name", name);
            rec.detail("tracks", entries.len());
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// Whether every one of `tracks` is a Library track.
fn all_in_library(rec: &Recorder<'_>, tracks: &[i64]) -> Result<bool, OpsError> {
    let unique: HashSet<i64> = tracks.iter().copied().collect();
    let json = serde_json::to_string(&unique).expect("numbers always serialize");
    let found: i64 = rec
        .read_rows(
            "SELECT count(*) FROM library_track WHERE id IN (SELECT value FROM json_each(?1))",
            [json],
            |r| r.get(0),
        )?
        .pop()
        .unwrap_or(0);
    Ok(found as usize == unique.len())
}

/// Puts the operation a write recorded (if it recorded one) into its answer.
fn with_operation(recorded: ops::Recorded<Done<PlaylistChanged>>) -> Done<PlaylistChanged> {
    recorded.value.map(|changed| PlaylistChanged {
        operation_id: recorded.operation_id,
        ..changed
    })
}

/// Adds Library `tracks` to a playlist, in the order given, as one
/// operation: last, or from position `at` (past the end: last), the entries
/// from there on moving down. A track already in it is added again.
pub fn add_on(
    conn: &mut Connection,
    id: PlaylistId,
    tracks: &[LibraryTrackId],
    at: Option<u32>,
) -> Result<Done<PlaylistChanged>, OpsError> {
    let tracks: Vec<i64> = tracks.iter().map(|t| t.0).collect();
    ops::record(
        conn,
        ADD_OPERATION,
        &serde_json::json!({ "playlistId": id.0 }),
        |rec| {
            let Some(name) = name_of(rec, id)? else {
                return Ok(Err(PlaylistsError::NotFound));
            };
            if !all_in_library(rec, &tracks)? {
                return Ok(Err(PlaylistsError::TrackNotFound));
            }
            let entries = entries_of(rec, id)?;
            let at = at.map_or(entries.len(), |at| (at as usize).min(entries.len()));
            // Those before stay (gaps closed), those after move down.
            renumber(rec, &entries[..at])?;
            for (i, entry) in entries[at..].iter().enumerate() {
                let to = (at + tracks.len() + i) as i64;
                if entry.position != to {
                    rec.set("playlist_entry", entry.id, "position", to)?;
                }
            }
            insert_entries(rec, id, &tracks, at)?;
            rec.detail("name", name);
            rec.detail("tracks", tracks.len());
            Ok(Ok(PlaylistChanged {
                changed: tracks.len() as u32,
                skipped: 0,
                operation_id: None,
            }))
        },
    )
    .map(with_operation)
}

/// Adds the Library tracks of `recordings` (what a track table's selection
/// holds) to a playlist as [`add_on`] does, last or from `at`, in the order
/// given and each track once. A recording that isn't a Library track is
/// left out and counted in `skipped`.
pub fn add_recordings_on(
    conn: &mut Connection,
    id: PlaylistId,
    recordings: &[i64],
    at: Option<u32>,
) -> Result<Done<PlaylistChanged>, OpsError> {
    let mut seen = HashSet::new();
    let mut tracks = Vec::new();
    let mut left_out = 0;
    for recording in recordings.iter().filter(|r| seen.insert(**r)) {
        let found = conn
            .query_row(
                "SELECT id FROM library_track WHERE recording_id = ?1",
                [recording],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        match found {
            Some(track) => tracks.push(LibraryTrackId(track)),
            None => left_out += 1,
        }
    }
    add_on(conn, id, &tracks, at).map(|done| {
        done.map(|changed| PlaylistChanged {
            skipped: left_out,
            ..changed
        })
    })
}

/// Removes entries from a playlist, as one operation; the rest close up.
/// Entries that aren't in it are skipped.
pub fn remove_on(
    conn: &mut Connection,
    id: PlaylistId,
    entries: &[PlaylistEntryId],
) -> Result<Done<PlaylistChanged>, OpsError> {
    let asked: HashSet<i64> = entries.iter().map(|e| e.0).collect();
    ops::record(
        conn,
        REMOVE_OPERATION,
        &serde_json::json!({ "playlistId": id.0 }),
        |rec| {
            let Some(name) = name_of(rec, id)? else {
                return Ok(Err(PlaylistsError::NotFound));
            };
            let (gone, kept): (Vec<Entry>, Vec<Entry>) = entries_of(rec, id)?
                .into_iter()
                .partition(|e| asked.contains(&e.id));
            for entry in &gone {
                rec.delete("playlist_entry", entry.id)?;
            }
            renumber(rec, &kept)?;
            rec.detail("name", name);
            rec.detail("tracks", gone.len());
            Ok(Ok(PlaylistChanged {
                changed: gone.len() as u32,
                skipped: (asked.len() - gone.len()) as u32,
                operation_id: None,
            }))
        },
    )
    .map(with_operation)
}

/// Moves entries of a playlist, keeping their order among themselves, to
/// just before entry `before` (`None`: to the end), as one operation that
/// rewrites every position that changes. When `before` is one of the moved
/// entries they go before the first entry after it that isn't moved.
/// Entries that aren't in the playlist are skipped; a `before` that isn't
/// in it is refused.
pub fn move_on(
    conn: &mut Connection,
    id: PlaylistId,
    entries: &[PlaylistEntryId],
    before: Option<PlaylistEntryId>,
) -> Result<Done<PlaylistChanged>, OpsError> {
    let asked: HashSet<i64> = entries.iter().map(|e| e.0).collect();
    ops::record(
        conn,
        MOVE_OPERATION,
        &serde_json::json!({ "playlistId": id.0 }),
        |rec| {
            let Some(name) = name_of(rec, id)? else {
                return Ok(Err(PlaylistsError::NotFound));
            };
            let current = entries_of(rec, id)?;
            let target = match before {
                None => None,
                Some(b) => match current.iter().position(|e| e.id == b.0) {
                    None => return Ok(Err(PlaylistsError::EntryNotFound)),
                    // The first entry from there on that stays put.
                    Some(at) => current[at..].iter().find(|e| !asked.contains(&e.id)),
                },
            };
            let (moving, rest): (Vec<Entry>, Vec<Entry>) =
                current.iter().partition(|e| asked.contains(&e.id));
            let at = target.map_or(rest.len(), |t| {
                rest.iter().position(|e| e.id == t.id).unwrap_or(rest.len())
            });
            let mut order = rest[..at].to_vec();
            order.extend(moving.iter().copied());
            order.extend(rest[at..].iter().copied());
            renumber(rec, &order)?;
            rec.detail("name", name);
            rec.detail("tracks", moving.len());
            Ok(Ok(PlaylistChanged {
                changed: moving.len() as u32,
                skipped: (asked.len() - moving.len()) as u32,
                operation_id: None,
            }))
        },
    )
    .map(with_operation)
}

/// "Save as crate": a new static crate at the top of the crate tree named
/// `name`, holding the playlist's tracks once each, as one operation. The
/// playlist stays as it is.
pub fn save_as_crate_on(
    conn: &mut Connection,
    id: PlaylistId,
    name: &str,
) -> Result<Done<CrateId>, OpsError> {
    let name = name.trim().to_owned();
    ops::record(
        conn,
        SAVE_AS_CRATE_OPERATION,
        &serde_json::json!({ "playlistId": id.0, "name": name }),
        |rec| {
            let Some(from) = name_of(rec, id)? else {
                return Ok(Err(PlaylistsError::NotFound));
            };
            let mut seen = HashSet::new();
            let tracks: Vec<i64> = entries_of(rec, id)?
                .into_iter()
                .map(|e| e.track)
                .filter(|t| seen.insert(*t))
                .collect();
            let made = crate::crates::make_in(rec, CrateKind::Static, None, &name, None)?;
            let crate_id = match made {
                Ok(crate_id) => crate_id,
                Err(refusal) => return Ok(Err(PlaylistsError::Crate(refusal))),
            };
            for track in &tracks {
                rec.insert(
                    "crate_entry",
                    &[
                        ("crate_id", Value::Integer(crate_id.0)),
                        ("library_track_id", Value::Integer(*track)),
                    ],
                )?;
            }
            rec.detail("playlist", from);
            rec.detail("name", name.clone());
            rec.detail("tracks", tracks.len());
            Ok(Ok(crate_id))
        },
    )
    .map(|recorded| recorded.value)
}

/// The tracks crate `id` holds now, if it is a static or smart crate: a
/// static crate's in the order they were added, a smart crate's evaluated.
fn crate_tracks(conn: &Connection, id: CrateId) -> rusqlite::Result<Option<Vec<LibraryTrackId>>> {
    if let Some(tracks) = crate::crates::track_ids(conn, id)? {
        return Ok(Some(tracks));
    }
    let smart: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM crate WHERE id = ?1 AND kind = 'smart')",
        [id.0],
        |r| r.get(0),
    )?;
    if smart {
        return crate::crates::smart::membership::evaluate(conn, id).map(Some);
    }
    Ok(None)
}

/// "Start a playlist from this crate": a new playlist named `name` holding
/// the crate's tracks, as one operation. Its order is `order` (the crate
/// view's sort), then any of the crate's tracks `order` doesn't name, in the
/// crate's order; ids in `order` that aren't the crate's are ignored. A
/// smart crate gives the tracks its rule holds now.
pub fn from_crate_on(
    conn: &mut Connection,
    crate_id: CrateId,
    name: &str,
    order: &[LibraryTrackId],
) -> Result<Done<PlaylistId>, OpsError> {
    let Some(members) = crate_tracks(conn, crate_id)? else {
        return Ok(Err(PlaylistsError::Crate(CratesError::NotFound)));
    };
    let held: HashSet<i64> = members.iter().map(|t| t.0).collect();
    let mut seen = HashSet::new();
    let tracks: Vec<i64> = order
        .iter()
        .chain(members.iter())
        .map(|t| t.0)
        .filter(|t| held.contains(t) && seen.insert(*t))
        .collect();
    let name = name.trim().to_owned();
    ops::record(
        conn,
        FROM_CRATE_OPERATION,
        &serde_json::json!({ "crateId": crate_id.0, "name": name }),
        |rec| {
            let id = match make_in(rec, &name, None)? {
                Ok(id) => id,
                Err(refusal) => return Ok(Err(refusal)),
            };
            insert_entries(rec, id, &tracks, 0)?;
            rec.detail("tracks", tracks.len());
            Ok(Ok(id))
        },
    )
    .map(|recorded| recorded.value)
}

fn origin_from(stored: &str) -> PlaylistOrigin {
    match stored {
        "rekordbox" => PlaylistOrigin::Rekordbox,
        _ => PlaylistOrigin::App,
    }
}

/// The playlists as listed: `WHERE` part and its parameter.
fn playlists_where(
    conn: &Connection,
    filter: &str,
    param: Option<i64>,
) -> rusqlite::Result<Vec<Playlist>> {
    let sql = format!(
        "SELECT p.id, p.name, p.origin, p.source_path,
                (SELECT count(*) FROM playlist_entry e WHERE e.playlist_id = p.id)
         FROM playlist p {filter} ORDER BY p.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(param), |r| {
        let path: Option<String> = r.get(3)?;
        Ok(Playlist {
            id: PlaylistId(r.get(0)?),
            name: r.get(1)?,
            origin: origin_from(&r.get::<_, String>(2)?),
            source_path: path.and_then(|p| serde_json::from_str(&p).ok()),
            track_count: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// Every playlist, oldest first: the order the sidebar lists them and a
/// send writes them.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<Playlist>> {
    playlists_where(conn, "", None)
}

/// Each playlist's Library tracks in position order, by playlist id.
fn tracks_by_playlist(conn: &Connection) -> rusqlite::Result<HashMap<i64, Vec<LibraryTrackId>>> {
    let mut out: HashMap<i64, Vec<LibraryTrackId>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT playlist_id, library_track_id FROM playlist_entry
         ORDER BY playlist_id, position, id",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (playlist, track) = row?;
        out.entry(playlist).or_default().push(LibraryTrackId(track));
    }
    Ok(out)
}

/// The playlists a send writes under the `Playlists` folder: every
/// playlist, oldest first, its tracks in position order, repeats kept, an
/// empty one written empty. A track the send leaves out is reported with
/// each of its entries, as a crate's are.
pub fn send_nodes(conn: &Connection) -> rusqlite::Result<Vec<Node>> {
    let mut tracks = tracks_by_playlist(conn)?;
    Ok(list(conn)?
        .into_iter()
        .map(|p| Node::Playlist {
            entries: tracks.remove(&p.id.0).unwrap_or_default(),
            name: p.name,
        })
        .collect())
}

/// One playlist with every entry's row, in order; `None` if there's no such
/// playlist.
pub fn view(
    conn: &Connection,
    volumes: &impl crate::paths::Volumes,
    id: PlaylistId,
) -> rusqlite::Result<Option<PlaylistView>> {
    let Some(playlist) = playlists_where(conn, "WHERE p.id = ?1", Some(id.0))?.pop() else {
        return Ok(None);
    };
    let entries: Vec<(i64, u32, i64)> = conn
        .prepare(
            "SELECT e.id, e.position, lt.recording_id
             FROM playlist_entry e JOIN library_track lt ON lt.id = e.library_track_id
             WHERE e.playlist_id = ?1 ORDER BY e.position, e.id",
        )?
        .query_map([id.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut seen = HashSet::new();
    let recordings: Vec<i64> = entries
        .iter()
        .map(|e| e.2)
        .filter(|r| seen.insert(*r))
        .collect();
    let rows: HashMap<i64, TableRow> = track_table::rows_of(conn, volumes, &recordings)?
        .into_iter()
        .map(|row| (row.recording_id, row))
        .collect();
    let items = entries
        .into_iter()
        .filter_map(|(entry, position, recording)| {
            rows.get(&recording).map(|row| PlaylistItem {
                entry_id: PlaylistEntryId(entry),
                position,
                row: row.clone(),
            })
        })
        .collect();
    Ok(Some(PlaylistView { playlist, items }))
}

// `async` runs the work off the main thread, like the crates' commands.

/// Every playlist, oldest first.
#[tauri::command]
#[specta::specta]
pub async fn list_playlists(reads: State<'_, ReadPool>) -> Result<Vec<Playlist>, IpcError> {
    Ok(reads.read(list)?)
}

/// One playlist and its entries in order, each with the track's row.
#[tauri::command]
#[specta::specta]
pub async fn playlist_view(
    reads: State<'_, ReadPool>,
    id: PlaylistId,
) -> Result<PlaylistView, IpcError> {
    let volumes = crate::scan::system_volumes();
    reads
        .read(move |c| view(c, &volumes, id))?
        .ok_or_else(|| PlaylistsError::NotFound.into())
}

/// Makes an empty playlist. The name is trimmed.
#[tauri::command]
#[specta::specta]
pub async fn create_playlist(
    writer: State<'_, Writer>,
    name: String,
) -> Result<PlaylistId, IpcError> {
    Ok(writer.call(move |c| Ok(create_on(c, &name)))???)
}

/// Renames a playlist; the operation, or none when the name is the same.
#[tauri::command]
#[specta::specta]
pub async fn rename_playlist(
    writer: State<'_, Writer>,
    id: PlaylistId,
    name: String,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |c| Ok(rename_on(c, id, &name)))???)
}

/// Deletes a playlist (its tracks stay in the Library); the operation.
#[tauri::command]
#[specta::specta]
pub async fn delete_playlist(
    writer: State<'_, Writer>,
    id: PlaylistId,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |c| Ok(delete_on(c, id)))???)
}

/// Adds Library tracks, last or from position `at`.
#[tauri::command]
#[specta::specta]
pub async fn add_tracks_to_playlist(
    writer: State<'_, Writer>,
    id: PlaylistId,
    tracks: Vec<LibraryTrackId>,
    at: Option<u32>,
) -> Result<PlaylistChanged, IpcError> {
    Ok(writer.call(move |c| Ok(add_on(c, id, &tracks, at)))???)
}

/// Adds the Library tracks among `recordings` (a table's selection), last
/// or from position `at`; `skipped` counts the ones outside the Library.
#[tauri::command]
#[specta::specta]
pub async fn add_recordings_to_playlist(
    writer: State<'_, Writer>,
    id: PlaylistId,
    recordings: Vec<i64>,
    at: Option<u32>,
) -> Result<PlaylistChanged, IpcError> {
    Ok(writer.call(move |c| Ok(add_recordings_on(c, id, &recordings, at)))???)
}

/// Removes entries.
#[tauri::command]
#[specta::specta]
pub async fn remove_from_playlist(
    writer: State<'_, Writer>,
    id: PlaylistId,
    entries: Vec<PlaylistEntryId>,
) -> Result<PlaylistChanged, IpcError> {
    Ok(writer.call(move |c| Ok(remove_on(c, id, &entries)))???)
}

/// Moves entries before entry `before`, or to the end.
#[tauri::command]
#[specta::specta]
pub async fn move_in_playlist(
    writer: State<'_, Writer>,
    id: PlaylistId,
    entries: Vec<PlaylistEntryId>,
    before: Option<PlaylistEntryId>,
) -> Result<PlaylistChanged, IpcError> {
    Ok(writer.call(move |c| Ok(move_on(c, id, &entries, before)))???)
}

/// "Save as crate": a new crate at the top of the tree.
#[tauri::command]
#[specta::specta]
pub async fn save_playlist_as_crate(
    writer: State<'_, Writer>,
    id: PlaylistId,
    name: String,
) -> Result<CrateId, IpcError> {
    Ok(writer.call(move |c| Ok(save_as_crate_on(c, id, &name)))???)
}

/// "Start a playlist from this crate", in `order` (the view's sort).
#[tauri::command]
#[specta::specta]
pub async fn playlist_from_crate(
    writer: State<'_, Writer>,
    crate_id: CrateId,
    name: String,
    order: Vec<LibraryTrackId>,
) -> Result<PlaylistId, IpcError> {
    Ok(writer.call(move |c| Ok(from_crate_on(c, crate_id, &name, &order)))???)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod timeline_tests;
