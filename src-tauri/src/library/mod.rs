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
//! on disk: those tracks wait in the Missing list (ROADMAP 1.3). Adding a
//! track that's already in the Library changes nothing and returns the
//! Library track it has; an existing link is never re-pointed here.
//!
//! - It only touches the database. No audio file is opened, read or written.
//! - The add goes through the operation log ([`crate::ops`]), so it can be
//!   undone.
//! - One Library track per track: the table enforces it.

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{DbError, ReadPool, Writer};
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

/// Why [`check`] refused a track, before any path is made readable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    TrackNotFound,
    NoFile,
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
        Some(choice) if !choice.present => Err(Refusal::FileMissing {
            file_id: choice.file_id,
        }),
        Some(choice) => Ok(Plan::Link(choice)),
    })
}

/// Inserts the linked Library track for `recording`, as one change of the
/// operation `rec` is recording. For callers that add many tracks as one
/// operation (1aE); [`check`] each track first.
pub fn insert_linked(
    rec: &mut Recorder<'_>,
    recording: i64,
    file_id: i64,
) -> Result<LibraryTrackId, OpsError> {
    rec.insert(
        "library_track",
        &[
            ("recording_id", Value::Integer(recording)),
            (
                "kind",
                Value::Text(LibraryTrackKind::Linked.as_str().to_owned()),
            ),
            ("linked_file_id", Value::Integer(file_id)),
        ],
    )
    .map(LibraryTrackId)
}

/// What [`promote_on`] did, before paths are made readable.
type PromotedStored = (StoredTrack, bool);

/// [`promote`], on the writer connection. One writer job, so nothing can
/// change between the check and the insert.
fn promote_on(
    conn: &mut Connection,
    recording: i64,
) -> Result<Result<PromotedStored, (Refusal, Option<StoredFile>)>, OpsError> {
    let (id, added) = match check(conn, recording)? {
        Err(refusal) => {
            let file = match refusal {
                Refusal::FileMissing { file_id } => stored_file(conn, file_id)?,
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
    match writer.call(move |conn| Ok(promote_on(conn, recording)))?? {
        Ok((track, added)) => Ok(Promoted {
            library_track: track.to_library_track(volumes),
            added,
        }),
        Err((Refusal::TrackNotFound, _)) => Err(LibraryError::TrackNotFound),
        Err((Refusal::NoFile, _)) => Err(LibraryError::NoFile),
        Err((Refusal::FileMissing { .. }, file)) => Err(LibraryError::FileMissing {
            path: file.map(|f| f.shown(volumes).path).unwrap_or_default(),
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
    fn shown(&self, volumes: &impl Volumes) -> LinkedFile {
        let stored = StoredPath::new(self.volume.clone(), self.rel.clone());
        let path = match stored.resolve(volumes) {
            Ok(abs) => display_path(&abs),
            // Offline: a Windows path under the last mount point, built by
            // hand so it reads the same on every platform.
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
    pub added_at: String,
}

impl StoredTrack {
    pub fn to_library_track(&self, volumes: &impl Volumes) -> LibraryTrack {
        LibraryTrack {
            id: self.id,
            recording_id: self.recording_id,
            kind: self.kind,
            title: self.title.clone(),
            artist: self.artist.clone(),
            file: self.file.as_ref().map(|f| f.shown(volumes)),
            added_at: self.added_at.clone(),
        }
    }
}

const FILE_COLUMNS: &str =
    "f.id, v.identity, mf.rel_path, f.rel_path, v.last_mount_path, f.present";
const FILE_JOINS: &str = "LEFT JOIN music_folder mf ON mf.id = f.music_folder_id
                          LEFT JOIN volume v ON v.id = mf.volume_id";

/// Reads [`FILE_COLUMNS`], starting at column `at`. `None` if there's no
/// file, or its volume identity or path doesn't read back.
fn file_from(r: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<Option<StoredFile>> {
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

fn stored_file(conn: &Connection, file_id: i64) -> rusqlite::Result<Option<StoredFile>> {
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
        "SELECT lt.id, lt.recording_id, lt.kind, r.title, r.artist, lt.added_at, {FILE_COLUMNS}
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
            file_from(r, 6)?,
        ))
    })?;
    let mut tracks = Vec::new();
    for row in rows {
        let (id, recording_id, kind, title, artist, added_at, file) = row?;
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

/// The Library list: every Library track, sorted by the title it shows
/// (its title, or its file's name when it has none), ignoring case, then
/// by artist, then in the order they were added.
pub fn list(tracks: &[StoredTrack], volumes: &impl Volumes) -> Vec<LibraryTrack> {
    let mut list: Vec<LibraryTrack> = tracks.iter().map(|t| t.to_library_track(volumes)).collect();
    let key = |t: &LibraryTrack| {
        let shown = t
            .title
            .as_deref()
            .or(t.file.as_ref().map(|f| f.name.as_str()))
            .unwrap_or_default()
            .to_lowercase();
        let artist = t.artist.as_deref().unwrap_or_default().to_lowercase();
        (shown, artist, t.id)
    };
    list.sort_by_cached_key(key);
    list
}

/// Every Library track, sorted for the Library list (see [`list`]).
#[tauri::command]
#[specta::specta]
pub async fn library_tracks(reads: State<'_, ReadPool>) -> Result<Vec<LibraryTrack>, IpcError> {
    let tracks = reads.read(stored)?;
    Ok(list(&tracks, &system_volumes()))
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

#[cfg(test)]
mod tests;
