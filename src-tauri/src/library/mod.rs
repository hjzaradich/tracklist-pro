//! Library tracks: the tracks the user added to play with (1aD-6; ROADMAP
//! 1.8, §2 `library_track`).
//!
//! In the MVP every Library track is **linked**: it points at a file that
//! already exists and is never written. Adding a track ("Add to Library",
//! `promote` in code) picks that file ([`linked_file_for`]):
//!
//! 1. the file rekordbox uses for the track, when there's a trusted match
//!    (`rekordbox_track.file_id` with `relink_probable = 0`), so nothing
//!    changes in rekordbox and cues and history stay safe;
//! 2. otherwise the track's best file. A probable match is ignored.
//!
//! The add is refused when the track has no such file, or the file isn't
//! on disk: those tracks wait in the Missing list (ROADMAP 1.3). A file on
//! an unplugged drive counts as on disk (`file.present` stays 1, 1aB-9), so
//! its track can be added and isn't shown as missing. Adding a
//! track that's already in the Library changes nothing and returns the
//! Library track it has; an existing link is never re-pointed here.
//!
//! - It only touches the database. No audio file is opened, read or written.
//! - The add goes through the operation log ([`crate::ops`]), so it can be
//!   undone.
//! - One Library track per track: the table enforces it.

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{DbError, ReadPool, Writer};
use crate::fragile::{self, FragileDirs, FragileReason, Located};
use crate::ipc::{ErrorKind, ErrorParam, IpcError};
use crate::ops::{self, OpsError, Recorder};
use crate::paths::{RelPath, StoredPath, Volumes};
use crate::scan::{display_path, system_volumes};
use crate::volume::VolumeId;

/// The operation kind of an add, as the operation log stores it.
pub const PROMOTE_OPERATION: &str = "promote";

/// A Library track's row id.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct LibraryTrackId(pub i64);

/// What a Library track's audio is. Stored in `library_track.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LibraryTrackKind {
    /// It points at an existing file, which is never written.
    Linked,
    /// A managed copy in the Library folder (Phase 2, ROADMAP 2.6).
    Copy,
}

impl LibraryTrackKind {
    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            LibraryTrackKind::Linked => "linked",
            LibraryTrackKind::Copy => "copy",
        }
    }

    fn parse(name: &str) -> Option<LibraryTrackKind> {
        match name {
            "linked" => Some(LibraryTrackKind::Linked),
            "copy" => Some(LibraryTrackKind::Copy),
            _ => None,
        }
    }
}

/// The file a linked Library track plays, as the frontend sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LinkedFile {
    /// Where the file is, e.g. `E:\DJ Music\a.mp3`: under its volume's
    /// mount point now, or where that was last seen if the volume is
    /// offline.
    pub path: String,
    /// The file's name, e.g. `a.mp3`.
    pub name: String,
    /// Whether the last scan found it on disk (`file.present`).
    pub present: bool,
    /// Whether its drive is connected now. A file on an unplugged drive
    /// stays `present`, and its track isn't missing.
    pub drive_connected: bool,
}

/// A Library track, with what the Library list shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTrack {
    pub id: LibraryTrackId,
    /// The track (`recording`) it was made from.
    pub recording_id: i64,
    pub kind: LibraryTrackKind,
    /// The track's title and artist. `None` when it has none yet; the list
    /// then shows the linked file's name as the title.
    pub title: Option<String>,
    pub artist: Option<String>,
    /// The linked file. `None` only for a Library track with no file link.
    pub file: Option<LinkedFile>,
    /// Why the linked file's location is fragile (Downloads, a temp folder,
    /// an external or network drive), if it is (1aD-7).
    pub fragile: Option<FragileReason>,
    /// Whether the linked file is gone. Kept current by the scan
    /// (`library_track.source_status`), not worked out when listed.
    pub source_missing: bool,
    /// How many conflicts with rekordbox are still open for it. Removing
    /// the track drops them (undo brings them back).
    pub open_conflicts: u32,
    /// When it was added, UTC ISO-8601.
    pub added_at: String,
}

/// What adding a track to the Library did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Promoted {
    pub library_track: LibraryTrack,
    /// False when the track was already in the Library: nothing changed.
    pub added: bool,
}

/// Why a file was picked as a track's linked file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedFileSource {
    /// rekordbox uses it for this track (a trusted match).
    Rekordbox,
    /// It's the track's best file.
    BestFile,
}

/// The file a track would be linked to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkedFileChoice {
    pub file_id: i64,
    pub source: LinkedFileSource,
    /// Whether the last scan found the file on disk.
    pub present: bool,
}

/// Why a track can't be added to the Library. Nothing was changed.
#[derive(Debug)]
pub enum LibraryError {
    /// There's no track with that id.
    TrackNotFound,
    /// The track has no trusted rekordbox match and no best file.
    NoFile,
    /// The file the track would link to isn't on disk.
    FileMissing { path: String },
    /// One of the track's files is the probable match of a rekordbox
    /// track, and nothing can confirm that match yet.
    MatchNotConfirmed,
    /// The database or the operation log failed.
    Ops(OpsError),
}

impl From<OpsError> for LibraryError {
    fn from(e: OpsError) -> Self {
        LibraryError::Ops(e)
    }
}

impl From<DbError> for LibraryError {
    fn from(e: DbError) -> Self {
        LibraryError::Ops(OpsError::Db(e))
    }
}

impl From<rusqlite::Error> for LibraryError {
    fn from(e: rusqlite::Error) -> Self {
        LibraryError::Ops(OpsError::from(e))
    }
}

impl From<LibraryError> for IpcError {
    fn from(e: LibraryError) -> Self {
        match e {
            LibraryError::TrackNotFound => IpcError::new(ErrorKind::LibraryTrackNotFound),
            LibraryError::NoFile => IpcError::new(ErrorKind::LibraryNoFile),
            LibraryError::FileMissing { path } => IpcError::with(
                ErrorKind::LibraryFileMissing,
                [("path", ErrorParam::from(path))],
            ),
            LibraryError::MatchNotConfirmed => IpcError::new(ErrorKind::LibraryMatchNotConfirmed),
            LibraryError::Ops(e) => IpcError::from(e),
        }
    }
}

/// The file `recording` would be linked to, or `None` if it has neither a
/// trusted rekordbox match nor a best file.
///
/// A trusted match beats the best file; a probable match
/// (`relink_probable = 1`) is ignored. If rekordbox uses several of the
/// track's files (duplicates it holds separately), the pick is the one on
/// disk, then the track's best file, then the lowest file id.
///
/// The match is found through the file's own track row, not
/// `rekordbox_track.recording_id`, which a fresh read clears until
/// grouping runs again.
pub fn linked_file_for(
    conn: &Connection,
    recording: i64,
) -> rusqlite::Result<Option<LinkedFileChoice>> {
    let trusted = conn
        .query_row(
            "SELECT f.id, f.present
             FROM rekordbox_track rt
             JOIN recording_file rf ON rf.file_id = rt.file_id
             JOIN file f ON f.id = rt.file_id
             WHERE rf.recording_id = ?1 AND rt.relink_probable = 0
             ORDER BY f.present DESC, (rf.role = 'best') DESC, f.id
             LIMIT 1",
            [recording],
            |r| {
                Ok(LinkedFileChoice {
                    file_id: r.get(0)?,
                    source: LinkedFileSource::Rekordbox,
                    present: r.get(1)?,
                })
            },
        )
        .optional()?;
    if trusted.is_some() {
        return Ok(trusted);
    }
    conn.query_row(
        "SELECT f.id, f.present
         FROM recording_file rf JOIN file f ON f.id = rf.file_id
         WHERE rf.recording_id = ?1 AND rf.role = 'best'",
        [recording],
        |r| {
            Ok(LinkedFileChoice {
                file_id: r.get(0)?,
                source: LinkedFileSource::BestFile,
                present: r.get(1)?,
            })
        },
    )
    .optional()
}

/// Whether `recording` would be linked to a file rekordbox isn't known to
/// use, while one of its files is the *probable* match of a rekordbox
/// entry (`relink_probable = 1`). Added like that, the track would go to
/// rekordbox as a new one, beside the entry that is probably its own: a
/// second entry, without the cues. So it can't be added until the match
/// can be confirmed (1bE-6). A track that also has a trusted match is
/// linked to that file and isn't held back.
pub fn match_not_confirmed(conn: &Connection, recording: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM rekordbox_track rt
                        JOIN recording_file rf ON rf.file_id = rt.file_id
                        WHERE rf.recording_id = ?1 AND rt.relink_probable = 1)
            AND NOT EXISTS (SELECT 1 FROM rekordbox_track rt
                            JOIN recording_file rf ON rf.file_id = rt.file_id
                            WHERE rf.recording_id = ?1 AND rt.relink_probable = 0)",
        [recording],
        |r| r.get(0),
    )
}

/// Why [`check`] refused a track, before any path is made readable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    TrackNotFound,
    NoFile,
    /// See [`match_not_confirmed`].
    MatchNotConfirmed,
    /// The chosen file isn't on disk.
    FileMissing {
        file_id: i64,
    },
}

/// What adding `recording` would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// It's in the Library already, as this Library track.
    AlreadyThere(LibraryTrackId),
    /// Link it to this file.
    Link(LinkedFileChoice),
}

/// Decides what adding `recording` would do, without changing anything.
pub fn check(conn: &Connection, recording: i64) -> rusqlite::Result<Result<Plan, Refusal>> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM library_track WHERE recording_id = ?1",
            [recording],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(Ok(Plan::AlreadyThere(LibraryTrackId(id))));
    }
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM recording WHERE id = ?1)",
        [recording],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(Err(Refusal::TrackNotFound));
    }
    Ok(match linked_file_for(conn, recording)? {
        None => Err(Refusal::NoFile),
        Some(_) if match_not_confirmed(conn, recording)? => Err(Refusal::MatchNotConfirmed),
        Some(choice) if !choice.present => Err(Refusal::FileMissing {
            file_id: choice.file_id,
        }),
        Some(choice) => Ok(Plan::Link(choice)),
    })
}

/// Inserts the linked Library track for `recording`, as one change of the
/// operation `rec` is recording. For callers that add many tracks as one
/// operation (1aE); [`check`] each track first, in the same writer job, so
/// nothing changes in between.
///
/// Adding a track the user removed earlier clears its removal record in
/// the same operation (so undoing the add brings the record back), and
/// gives the Library track the location the removed one was last sent to.
/// Its sync bases aren't restored: the next send records them again.
pub fn insert_linked(
    rec: &mut Recorder<'_>,
    recording: i64,
    file_id: i64,
) -> Result<LibraryTrackId, OpsError> {
    let removal: Option<(i64, Option<String>, Option<String>)> = rec
        .read_rows(
            "SELECT id, last_sent_location, last_exported_at FROM library_removal
             WHERE recording_id = ?1",
            [recording],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?
        .pop();
    let mut values = vec![
        ("recording_id", Value::Integer(recording)),
        (
            "kind",
            Value::Text(LibraryTrackKind::Linked.as_str().to_owned()),
        ),
        ("linked_file_id", Value::Integer(file_id)),
    ];
    if let Some((removal, location, exported_at)) = removal {
        rec.delete("library_removal", removal)?;
        values.push(("last_sent_location", location.into()));
        values.push(("last_exported_at", exported_at.into()));
    }
    rec.insert("library_track", &values).map(LibraryTrackId)
}

/// The operation kind of a removal, as the operation log stores it.
pub const REMOVE_OPERATION: &str = "remove_from_library";

/// A track the user removed from the Library and hasn't added back (§2
/// `library_removal`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemovedTrack {
    /// The track (`recording`) that was removed.
    pub recording_id: i64,
    /// When it was removed, UTC ISO-8601.
    pub removed_at: String,
    /// Where it was last sent to rekordbox. `None` if it never was.
    pub last_sent_location: Option<String>,
    pub last_exported_at: Option<String>,
}

/// Every track the user removed from the Library and hasn't added back, in
/// the order they were removed. The offer to add rekordbox's tracks leaves
/// these out, so nothing the user removed comes back uninvited (ROADMAP
/// 1.3).
pub fn removed_tracks(conn: &Connection) -> rusqlite::Result<Vec<RemovedTrack>> {
    let mut stmt = conn.prepare(
        "SELECT recording_id, removed_at, last_sent_location, last_exported_at
         FROM library_removal ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(RemovedTrack {
            recording_id: r.get(0)?,
            removed_at: r.get(1)?,
            last_sent_location: r.get(2)?,
            last_exported_at: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// The removed tracks rekordbox may still hold, because they were sent
/// before: what the user has to remove in rekordbox by hand, since an XML
/// import can't remove a track (ROADMAP 1.9 rule 7).
pub fn remove_in_rekordbox(conn: &Connection) -> rusqlite::Result<Vec<RemovedTrack>> {
    Ok(removed_tracks(conn)?
        .into_iter()
        .filter(|t| t.last_sent_location.is_some())
        .collect())
}

/// Rows of `table` that belong to a Library track, by rowid.
fn rows_of(rec: &Recorder<'_>, table: &str, id: LibraryTrackId) -> Result<Vec<i64>, OpsError> {
    rec.read_rows(
        &format!("SELECT rowid FROM {table} WHERE library_track_id = ?1 ORDER BY rowid"),
        [id.0],
        |r| r.get(0),
    )
}

/// Removes one Library track, as one operation: its crate entries, sync
/// bases and conflicts go with it (they'd otherwise block the delete), and
/// a removal record is made. Undo puts every row back exactly.
fn remove_on(conn: &mut Connection, id: LibraryTrackId) -> Result<bool, OpsError> {
    ops::record(
        conn,
        REMOVE_OPERATION,
        &serde_json::json!({ "libraryTrackId": id.0 }),
        |rec| {
            let row: Option<(i64, Option<String>, Option<String>)> = rec
                .read_rows(
                    "SELECT recording_id, last_sent_location, last_exported_at
                     FROM library_track WHERE id = ?1",
                    [id.0],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?
                .pop();
            let Some((recording, location, exported_at)) = row else {
                return Ok(false);
            };
            for table in ["crate_entry", "sync_base", "conflict"] {
                for rowid in rows_of(rec, table, id)? {
                    rec.delete(table, rowid)?;
                }
            }
            rec.insert(
                "library_removal",
                &[
                    ("recording_id", Value::Integer(recording)),
                    ("last_sent_location", location.into()),
                    ("last_exported_at", exported_at.into()),
                ],
            )?;
            rec.delete("library_track", id.0)?;
            Ok(true)
        },
    )
    .map(|recorded| recorded.value)
}

/// Removes a Library track from the Library ("Remove from Library"). The
/// track itself and its file are untouched: only the Library track goes.
/// Recorded as one operation, so undo restores it, and it leaves a removal
/// record (see [`removed_tracks`]). It's dropped from the next send, but
/// can't be removed from rekordbox through XML.
pub fn remove(writer: &Writer, id: LibraryTrackId) -> Result<(), LibraryError> {
    match writer.call(move |conn| Ok(remove_on(conn, id)))?? {
        true => Ok(()),
        false => Err(LibraryError::TrackNotFound),
    }
}

/// The file a refused add would have linked to.
struct MissingFile {
    /// Where it is, if its volume and path read back.
    file: Option<StoredFile>,
    /// Its path inside its music folder, as stored: never empty.
    rel_path: String,
}

impl MissingFile {
    /// The path to name in the refusal: the full path if it can be made,
    /// otherwise the path inside the music folder. Never empty.
    fn shown(&self, volumes: &impl Volumes) -> String {
        match &self.file {
            Some(file) => file.shown(volumes).path,
            None => self.rel_path.replace('/', "\\"),
        }
    }
}

/// What [`promote_on`] did, before paths are made readable.
type PromotedStored = (StoredTrack, bool);

/// [`promote`], on the writer connection. One writer job, so nothing can
/// change between the check and the insert.
fn promote_on(
    conn: &mut Connection,
    recording: i64,
) -> Result<Result<PromotedStored, (Refusal, Option<MissingFile>)>, OpsError> {
    let (id, added) = match check(conn, recording)? {
        Err(refusal) => {
            let file = match refusal {
                Refusal::FileMissing { file_id } => Some(MissingFile {
                    file: stored_file(conn, file_id)?,
                    rel_path: conn.query_row(
                        "SELECT rel_path FROM file WHERE id = ?1",
                        [file_id],
                        |r| r.get(0),
                    )?,
                }),
                _ => None,
            };
            return Ok(Err((refusal, file)));
        }
        Ok(Plan::AlreadyThere(id)) => (id, false),
        Ok(Plan::Link(choice)) => {
            let recorded = ops::record(
                conn,
                PROMOTE_OPERATION,
                &serde_json::json!({ "recordingId": recording }),
                |rec| insert_linked(rec, recording, choice.file_id),
            )?;
            (recorded.value, true)
        }
    };
    let track = stored_one(conn, id)?.ok_or(OpsError::RowNotFound {
        entity: "library_track".to_owned(),
        id: id.0,
    })?;
    Ok(Ok((track, added)))
}

/// Adds the track `recording` to the Library as a linked Library track
/// ("Add to Library"). Recorded as one operation, so undo removes it again.
/// The file is never touched.
pub fn promote(
    writer: &Writer,
    volumes: &impl Volumes,
    recording: i64,
) -> Result<Promoted, LibraryError> {
    promote_with(writer, volumes, &FragileDirs::system(), recording)
}

/// [`promote`], judging fragile locations against `dirs`.
pub fn promote_with(
    writer: &Writer,
    volumes: &impl Volumes,
    dirs: &FragileDirs,
    recording: i64,
) -> Result<Promoted, LibraryError> {
    let done = writer.call(move |conn| {
        Ok(promote_on(conn, recording).and_then(|done| {
            let located = match &done {
                Ok((track, _)) => match &track.file {
                    Some(file) => fragile::locate(conn, &[file.file_id])?,
                    None => Vec::new(),
                },
                Err(_) => Vec::new(),
            };
            Ok((done, located))
        }))
    })??;
    match done {
        (Ok((track, added)), located) => {
            let fragile = fragile::judge(&located, volumes, dirs);
            Ok(Promoted {
                library_track: track.to_library_track(volumes, &fragile),
                added,
            })
        }
        (Err((Refusal::TrackNotFound, _)), _) => Err(LibraryError::TrackNotFound),
        (Err((Refusal::NoFile, _)), _) => Err(LibraryError::NoFile),
        (Err((Refusal::MatchNotConfirmed, _)), _) => Err(LibraryError::MatchNotConfirmed),
        (Err((Refusal::FileMissing { .. }, file)), _) => Err(LibraryError::FileMissing {
            path: file.map(|f| f.shown(volumes)).unwrap_or_default(),
        }),
    }
}

/// Where a file is, as stored: its volume and the path on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    pub file_id: i64,
    pub volume: VolumeId,
    /// From the volume's mount point.
    pub rel: RelPath,
    /// Where the volume was mounted last, e.g. `E:\`.
    pub last_mount_path: Option<String>,
    pub present: bool,
}

impl StoredFile {
    /// The file for the frontend, placed under its volume's mount point
    /// now, or under the last one if the volume is offline.
    pub fn shown(&self, volumes: &impl Volumes) -> LinkedFile {
        let stored = StoredPath::new(self.volume.clone(), self.rel.clone());
        let resolved = stored.resolve(volumes);
        let drive_connected = resolved.is_ok();
        let path = match resolved {
            Ok(abs) => display_path(&abs),
            // Offline: a Windows path under the last mount point, built by
            // hand so it reads the same on every platform. A volume that was
            // never seen mounted has none, and the path comes out relative.
            Err(_) => {
                let mount = self.last_mount_path.as_deref().unwrap_or_default();
                let mut path = mount.trim_end_matches('\\').to_owned();
                for part in self.rel.components() {
                    if !path.is_empty() {
                        path.push('\\');
                    }
                    path.push_str(part);
                }
                path
            }
        };
        LinkedFile {
            path,
            name: self.rel.components().last().unwrap_or_default().to_owned(),
            present: self.present,
            drive_connected,
        }
    }
}

/// A Library track as stored, before its file's path is made readable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTrack {
    pub id: LibraryTrackId,
    pub recording_id: i64,
    pub kind: LibraryTrackKind,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub file: Option<StoredFile>,
    /// `library_track.source_status` is `missing`.
    pub source_missing: bool,
    pub open_conflicts: u32,
    pub added_at: String,
}

impl StoredTrack {
    pub fn to_library_track(
        &self,
        volumes: &impl Volumes,
        fragile: &HashMap<i64, FragileReason>,
    ) -> LibraryTrack {
        LibraryTrack {
            id: self.id,
            recording_id: self.recording_id,
            kind: self.kind,
            title: self.title.clone(),
            artist: self.artist.clone(),
            file: self.file.as_ref().map(|f| f.shown(volumes)),
            fragile: self
                .file
                .as_ref()
                .and_then(|f| fragile.get(&f.file_id))
                .copied(),
            source_missing: self.source_missing,
            open_conflicts: self.open_conflicts,
            added_at: self.added_at.clone(),
        }
    }
}

pub(crate) const FILE_COLUMNS: &str =
    "f.id, v.identity, mf.rel_path, f.rel_path, v.last_mount_path, f.present";
pub(crate) const FILE_JOINS: &str = "LEFT JOIN music_folder mf ON mf.id = f.music_folder_id
                          LEFT JOIN volume v ON v.id = mf.volume_id";

/// Reads [`FILE_COLUMNS`], starting at column `at`. `None` if there's no
/// file, or its volume identity or path doesn't read back.
pub(crate) fn file_from(r: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<Option<StoredFile>> {
    let Some(file_id) = r.get::<_, Option<i64>>(at)? else {
        return Ok(None);
    };
    let identity: Option<String> = r.get(at + 1)?;
    let folder: Option<String> = r.get(at + 2)?;
    let rel: String = r.get(at + 3)?;
    let (Some(identity), Some(folder)) = (identity, folder) else {
        return Ok(None);
    };
    let (Ok(volume), Ok(folder), Ok(rel)) = (
        VolumeId::from_stored(identity),
        RelPath::parse(&folder),
        RelPath::parse(&rel),
    ) else {
        return Ok(None);
    };
    Ok(Some(StoredFile {
        file_id,
        volume,
        rel: folder.join(&rel),
        last_mount_path: r.get(at + 4)?,
        present: r.get(at + 5)?,
    }))
}

pub fn stored_file(conn: &Connection, file_id: i64) -> rusqlite::Result<Option<StoredFile>> {
    conn.query_row(
        &format!("SELECT {FILE_COLUMNS} FROM file f {FILE_JOINS} WHERE f.id = ?1"),
        [file_id],
        |r| file_from(r, 0),
    )
    .optional()
    .map(Option::flatten)
}

/// A value with something in it: blank text says nothing, like none.
fn filled(text: Option<String>) -> Option<String> {
    text.filter(|t| !t.trim().is_empty())
}

fn stored_where(
    conn: &Connection,
    filter: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<Vec<StoredTrack>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT lt.id, lt.recording_id, lt.kind, r.title, r.artist, lt.added_at, lt.source_status,
                (SELECT count(*) FROM conflict c
                 WHERE c.library_track_id = lt.id AND c.status = 'open'),
                {FILE_COLUMNS}
         FROM library_track lt
         JOIN recording r ON r.id = lt.recording_id
         LEFT JOIN file f ON f.id = lt.linked_file_id
         {FILE_JOINS}
         {filter}
         ORDER BY lt.id"
    ))?;
    let rows = stmt.query_map(params, |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, u32>(7)?,
            file_from(r, 8)?,
        ))
    })?;
    let mut tracks = Vec::new();
    for row in rows {
        let (id, recording_id, kind, title, artist, added_at, status, open_conflicts, file) = row?;
        // A kind this build doesn't know is left out rather than guessed.
        let Some(kind) = LibraryTrackKind::parse(&kind) else {
            continue;
        };
        tracks.push(StoredTrack {
            id: LibraryTrackId(id),
            recording_id,
            kind,
            title: filled(title),
            artist: filled(artist),
            file,
            source_missing: status == "missing",
            open_conflicts,
            added_at,
        });
    }
    Ok(tracks)
}

/// Every Library track, in the order they were added.
pub fn stored(conn: &Connection) -> rusqlite::Result<Vec<StoredTrack>> {
    stored_where(conn, "", [])
}

/// One Library track, if there is one with that id.
pub fn stored_one(conn: &Connection, id: LibraryTrackId) -> rusqlite::Result<Option<StoredTrack>> {
    Ok(stored_where(conn, "WHERE lt.id = ?1", [id.0])?
        .into_iter()
        .next())
}

/// Text as it's compared for sorting: accents and other combining marks
/// dropped, compatibility forms folded (NFKD), then lowercased, so an
/// accented letter sorts with its base letter. Not locale-aware: other
/// scripts follow Latin in code-point order.
pub fn sort_key(text: &str) -> String {
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;
    text.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// The Library list: every Library track, sorted by the title it shows
/// (its title, or its file's name when it has none), then by artist, then
/// in the order they were added. Text is compared by [`sort_key`], so case
/// and accents don't matter. The one place the list's order is decided.
pub fn list(
    tracks: &[StoredTrack],
    volumes: &impl Volumes,
    fragile: &HashMap<i64, FragileReason>,
) -> Vec<LibraryTrack> {
    let mut list: Vec<LibraryTrack> = tracks
        .iter()
        .map(|t| t.to_library_track(volumes, fragile))
        .collect();
    let key = |t: &LibraryTrack| {
        let shown = t
            .title
            .as_deref()
            .or(t.file.as_ref().map(|f| f.name.as_str()))
            .unwrap_or_default();
        let artist = t.artist.as_deref().unwrap_or_default();
        (sort_key(shown), sort_key(artist), t.id)
    };
    list.sort_by_cached_key(key);
    list
}

/// Every Library track and where their files are, in one database read.
/// The disk-facing part ([`list_with_fragile`]) follows with no connection
/// held.
pub fn stored_with_locations(
    conn: &Connection,
) -> rusqlite::Result<(Vec<StoredTrack>, Vec<Located>)> {
    let tracks = stored(conn)?;
    let files: Vec<i64> = tracks
        .iter()
        .filter_map(|t| t.file.as_ref().map(|f| f.file_id))
        .collect();
    let located = fragile::locate(conn, &files)?;
    Ok((tracks, located))
}

/// The Library list with each row's fragile reason (1aD-7): `located` is
/// judged against `dirs`, then the tracks are listed as in [`list`].
pub fn list_with_fragile(
    tracks: &[StoredTrack],
    located: &[Located],
    volumes: &impl Volumes,
    dirs: &FragileDirs,
) -> Vec<LibraryTrack> {
    list(tracks, volumes, &fragile::judge(located, volumes, dirs))
}

/// Every Library track, sorted for the Library list (see [`list`]).
#[tauri::command]
#[specta::specta]
pub async fn library_tracks(reads: State<'_, ReadPool>) -> Result<Vec<LibraryTrack>, IpcError> {
    let (tracks, located) = reads.read(stored_with_locations)?;
    Ok(list_with_fragile(
        &tracks,
        &located,
        &system_volumes(),
        &FragileDirs::system(),
    ))
}

/// Adds a track to the Library as a linked Library track. Its file is never
/// written. Adding a track that's already there changes nothing.
#[tauri::command]
#[specta::specta]
pub async fn promote_track(
    writer: State<'_, Writer>,
    recording_id: i64,
) -> Result<Promoted, IpcError> {
    Ok(promote(&writer, &system_volumes(), recording_id)?)
}

/// Removes a track from the Library. Only the Library track goes: the
/// file on disk is never touched, and undo brings it back.
#[tauri::command]
#[specta::specta]
pub async fn remove_library_track(
    writer: State<'_, Writer>,
    id: LibraryTrackId,
) -> Result<(), IpcError> {
    Ok(remove(&writer, id)?)
}

#[cfg(test)]
mod tests;
