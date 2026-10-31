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
//! Library track it has; an add never re-points an existing link.
//!
//! **When a link moves** ([`follow`], 1bE-10): only when the linked file is
//! gone from disk and a file with the very same audio is on disk in the
//! same track, and only where that doesn't change which rekordbox entry the
//! track is sent as. Then the link follows the file on its own. Nothing
//! else changes a link on its own: not grouping, not the best-file ranking,
//! not relink, not a send. The user can switch a gone linked file to
//! another copy of the track in Review ([`crate::review::actions`], 1bE-7).
//!
//! A file with the `low bitrate` finding is not added until the user has
//! agreed (1bE-8; see `warning`): the add answers `needsConfirmation` and
//! changes nothing. The rekordbox offer's bulk add is the one named
//! exception.
//!
//! - It only touches the database. No audio file is opened, read or written.
//! - The add goes through the operation log ([`crate::ops`]), so it can be
//!   undone.
//! - One Library track per track: the table enforces it.

pub mod follow;

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

mod stamp;
mod warning;
pub use stamp::{LibraryStamp, LibrarySummary, LISTED_TABLES};
use warning::is_confirmed;
pub use warning::{warning_for, AddWarning, Confirmed, PromoteOutcome};

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
    /// The file's id (`file.id`): what the preview player is given to play
    /// it (1cA-7).
    pub id: i64,
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
    /// The title and artist the list shows: what a send writes for the
    /// track, else (for the title) the file's name. `None` only when the
    /// track has neither.
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
    /// Another file of the track ranks ahead of the linked file in what the
    /// audio is, or in being there at all (ROADMAP 1.7, "Better file
    /// available"). The linked file doesn't change for it: a link only
    /// follows the same audio ([`follow`]).
    pub better_file: Option<BetterFile>,
}

/// A file of a Library track's that is ahead of its linked file, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BetterFile {
    /// The better file: the track's best-ranked one that the user hasn't
    /// dismissed in Review.
    pub file_id: i64,
    /// The step of the ranking that put it ahead: `onDisk`, `verdict`,
    /// `playable`, `cutoff`, `decodeErrors` or `format`. Never tags, art, a
    /// rekordbox link or a place: those don't make a file better.
    pub step: crate::file_roles::BetterStep,
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
    // `prepare_cached`: a crate add asks these once per track.
    let trusted = conn
        .prepare_cached(
            "SELECT f.id, f.present
             FROM rekordbox_track rt
             JOIN recording_file rf ON rf.file_id = rt.file_id
             JOIN file f ON f.id = rt.file_id
             WHERE rf.recording_id = ?1 AND rt.relink_probable = 0
             ORDER BY f.present DESC, (rf.role = 'best') DESC, f.id
             LIMIT 1",
        )?
        .query_row([recording], |r| {
            Ok(LinkedFileChoice {
                file_id: r.get(0)?,
                source: LinkedFileSource::Rekordbox,
                present: r.get(1)?,
            })
        })
        .optional()?;
    if trusted.is_some() {
        return Ok(trusted);
    }
    conn.prepare_cached(
        "SELECT f.id, f.present
         FROM recording_file rf JOIN file f ON f.id = rf.file_id
         WHERE rf.recording_id = ?1 AND rf.role = 'best'",
    )?
    .query_row([recording], |r| {
        Ok(LinkedFileChoice {
            file_id: r.get(0)?,
            source: LinkedFileSource::BestFile,
            present: r.get(1)?,
        })
    })
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
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM rekordbox_track rt
                        JOIN recording_file rf ON rf.file_id = rt.file_id
                        WHERE rf.recording_id = ?1 AND rt.relink_probable = 1)
            AND NOT EXISTS (SELECT 1 FROM rekordbox_track rt
                            JOIN recording_file rf ON rf.file_id = rt.file_id
                            WHERE rf.recording_id = ?1 AND rt.relink_probable = 0)",
    )?
    .query_row([recording], |r| r.get(0))
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
        .prepare_cached("SELECT id FROM library_track WHERE recording_id = ?1")?
        .query_row([recording], |r| r.get(0))
        .optional()?;
    if let Some(id) = existing {
        return Ok(Ok(Plan::AlreadyThere(LibraryTrackId(id))));
    }
    let exists: bool = conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM recording WHERE id = ?1)")?
        .query_row([recording], |r| r.get(0))?;
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
/// bases, sent playlist entries, conflicts and the app's own values go with
/// it (they'd otherwise block the delete), and a removal record is made.
/// Undo puts every row back exactly.
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
            for table in [
                "crate_entry",
                "sync_base",
                "sent_entry",
                "conflict",
                "app_value",
            ] {
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
enum Promotion {
    /// The track is in the Library; `true` if this call added it.
    Done(StoredTrack, bool),
    /// The add needs the user's go-ahead first. Nothing was recorded.
    Warned(AddWarning),
    /// The add was refused. Nothing was changed.
    Refused(Refusal, Option<MissingFile>),
}

/// [`promote`], on the writer connection. One writer job, so nothing can
/// change between the check and the insert.
///
/// The warning (1bE-8) is decided here, between [`check`] and the insert:
/// the file's verdict is settled, and a file with the `low bitrate` finding
/// is not added unless `confirmed` says the user agreed.
fn promote_on(
    conn: &mut Connection,
    recording: i64,
    confirmed: Confirmed,
) -> Result<Promotion, OpsError> {
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
            return Ok(Promotion::Refused(refusal, file));
        }
        Ok(Plan::AlreadyThere(id)) => (id, false),
        Ok(Plan::Link(choice)) => {
            if let Some(warning) = warning_for(conn, choice.file_id)? {
                if !is_confirmed(warning, confirmed) {
                    return Ok(Promotion::Warned(warning));
                }
            }
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
    Ok(Promotion::Done(track, added))
}

/// Adds the track `recording` to the Library as a linked Library track
/// ("Add to Library"). Recorded as one operation, so undo removes it again.
/// The file is never touched.
///
/// A file with the `low bitrate` finding is not added unless `confirmed`
/// says the user agreed: the answer is then
/// [`PromoteOutcome::NeedsConfirmation`] and nothing has changed.
pub fn promote(
    writer: &Writer,
    volumes: &impl Volumes,
    recording: i64,
    confirmed: Confirmed,
) -> Result<PromoteOutcome, LibraryError> {
    promote_with(
        writer,
        volumes,
        &FragileDirs::system(),
        recording,
        confirmed,
    )
}

/// [`promote`], judging fragile locations against `dirs`.
pub fn promote_with(
    writer: &Writer,
    volumes: &impl Volumes,
    dirs: &FragileDirs,
    recording: i64,
    confirmed: Confirmed,
) -> Result<PromoteOutcome, LibraryError> {
    let done = writer.call(move |conn| {
        Ok(promote_on(conn, recording, confirmed).and_then(|done| {
            let located = match &done {
                Promotion::Done(track, _) => match &track.file {
                    Some(file) => fragile::locate(conn, &[file.file_id])?,
                    None => Vec::new(),
                },
                _ => Vec::new(),
            };
            Ok((done, located))
        }))
    })??;
    match done {
        (Promotion::Done(track, added), located) => {
            let fragile = fragile::judge(&located, volumes, dirs);
            Ok(PromoteOutcome::Done(Promoted {
                library_track: track.to_library_track(volumes, &fragile),
                added,
            }))
        }
        (Promotion::Warned(warning), _) => Ok(PromoteOutcome::NeedsConfirmation { warning }),
        (Promotion::Refused(Refusal::TrackNotFound, _), _) => Err(LibraryError::TrackNotFound),
        (Promotion::Refused(Refusal::NoFile, _), _) => Err(LibraryError::NoFile),
        (Promotion::Refused(Refusal::MatchNotConfirmed, _), _) => {
            Err(LibraryError::MatchNotConfirmed)
        }
        (Promotion::Refused(Refusal::FileMissing { .. }, file), _) => {
            Err(LibraryError::FileMissing {
                path: file.map(|f| f.shown(volumes)).unwrap_or_default(),
            })
        }
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
            id: self.file_id,
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
    pub better_file: Option<BetterFile>,
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
            better_file: self.better_file,
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

/// The query behind [`stored`]: one row per Library track, `filter` narrowing
/// it. Its tables are the ones [`LISTED_TABLES`] names (a test holds them to it).
fn stored_sql(filter: &str) -> String {
    format!(
        "SELECT lt.id, lt.recording_id, lt.kind, r.title, r.artist, lt.added_at, lt.source_status,
                (SELECT count(*) FROM conflict c
                 WHERE c.library_track_id = lt.id AND c.status = 'open'),
                {FILE_COLUMNS}, bf.file_id, bf.step
         FROM library_track lt
         JOIN recording r ON r.id = lt.recording_id
         LEFT JOIN better_file bf ON bf.recording_id = lt.recording_id
         LEFT JOIN file f ON f.id = lt.linked_file_id
         {FILE_JOINS}
         {filter}
         ORDER BY lt.id"
    )
}

fn stored_where(
    conn: &Connection,
    filter: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<Vec<StoredTrack>> {
    let mut stmt = conn.prepare(&stored_sql(filter))?;
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
            r.get::<_, Option<i64>>(14)?,
            r.get::<_, Option<String>>(15)?,
        ))
    })?;
    let mut tracks = Vec::new();
    for row in rows {
        let (
            id,
            recording_id,
            kind,
            title,
            artist,
            added_at,
            status,
            open_conflicts,
            file,
            better_file,
            better_step,
        ) = row?;
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
            // A step this build doesn't know is left out, like a kind.
            better_file: better_file
                .zip(
                    better_step
                        .as_deref()
                        .and_then(crate::file_roles::BetterStep::parse),
                )
                .map(|(file_id, step)| BetterFile { file_id, step }),
        });
    }
    Ok(tracks)
}

/// Every Library track, in the order they were added, with the title and
/// artist a list shows for each ([`crate::send_values::shown`]): what a
/// send would write, and the file's name when there is no title.
pub fn stored(conn: &Connection, volumes: &impl Volumes) -> rusqlite::Result<Vec<StoredTrack>> {
    let mut tracks = stored_where(conn, "", [])?;
    fill_shown(conn, volumes, &mut tracks)?;
    Ok(tracks)
}

/// Sets each track's title and artist to the ones a list shows. Worked out
/// now, from what the scan stored: nothing is written.
fn fill_shown(
    conn: &Connection,
    volumes: &impl Volumes,
    tracks: &mut [StoredTrack],
) -> rusqlite::Result<()> {
    let ids: Vec<LibraryTrackId> = tracks.iter().map(|t| t.id).collect();
    let mut shown = crate::send_values::shown(conn, volumes, &ids)?;
    for track in tracks {
        if let Some(shown) = shown.remove(&track.id) {
            (track.title, track.artist) = shown.into_options();
        }
    }
    Ok(())
}

/// One Library track, if there is one with that id. Its title and artist are
/// the recording's own (none yet in Phase 1a): the lists use [`stored`].
pub fn stored_one(conn: &Connection, id: LibraryTrackId) -> rusqlite::Result<Option<StoredTrack>> {
    Ok(stored_where(conn, "WHERE lt.id = ?1", [id.0])?
        .into_iter()
        .next())
}

/// The Library tracks made from the tracks `recordings`, in the order of
/// their ids. Their title and artist are the stored ones: the track table
/// works out what it shows itself.
pub(crate) fn stored_for_recordings(
    conn: &Connection,
    recordings: &[i64],
) -> rusqlite::Result<Vec<StoredTrack>> {
    let ids = serde_json::to_string(recordings).expect("numbers always serialize");
    stored_where(
        conn,
        "WHERE lt.recording_id IN (SELECT value FROM json_each(?1))",
        [ids],
    )
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
    volumes: &impl Volumes,
) -> rusqlite::Result<(Vec<StoredTrack>, Vec<Located>)> {
    let tracks = stored(conn, volumes)?;
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

/// How many Library tracks there are, and a stamp that moves when the list
/// of them would come back different. Cheap at any size: no list is read.
#[tauri::command]
#[specta::specta]
pub async fn library_summary(
    reads: State<'_, ReadPool>,
    stamp: State<'_, LibraryStamp>,
) -> Result<LibrarySummary, IpcError> {
    Ok(reads.read(|conn| stamp::summary(conn, &stamp))?)
}

/// Adds a track to the Library as a linked Library track. Its file is never
/// written. Adding a track that's already there changes nothing.
///
/// A file with the `low bitrate` finding is not added unless
/// `confirm_low_bitrate` is true (the user saw the warning and chose to add
/// anyway); the answer is `needsConfirmation` and nothing has changed
/// (1bE-8).
#[tauri::command]
#[specta::specta]
pub async fn promote_track(
    writer: State<'_, Writer>,
    jobs: State<'_, crate::jobs::JobQueue>,
    recording_id: i64,
    confirm_low_bitrate: bool,
) -> Result<PromoteOutcome, IpcError> {
    let confirmed = Confirmed {
        low_bitrate: confirm_low_bitrate,
    };
    let outcome = promote(&writer, &system_volumes(), recording_id, confirmed)?;
    if matches!(&outcome, PromoteOutcome::Done(p) if p.added) {
        settle_now(&writer, &jobs);
        // Its "better file" is worked out in the background. A failure to
        // ask is not the add's: the next job to end asks again.
        if let Err(e) = crate::file_roles::request(&writer, |j| jobs.enqueue(j)) {
            eprintln!("promote_track: couldn't ask for the roles: {e:?}");
        }
    }
    Ok(outcome)
}

/// Removes a track from the Library. Only the Library track goes: the
/// file on disk is never touched, and undo brings it back.
#[tauri::command]
#[specta::specta]
pub async fn remove_library_track(
    writer: State<'_, Writer>,
    jobs: State<'_, crate::jobs::JobQueue>,
    id: LibraryTrackId,
) -> Result<(), IpcError> {
    remove(&writer, id)?;
    settle_now(&writer, &jobs);
    Ok(())
}

/// Works out the track table's row of the track an add or a removal has just
/// changed, so the lists read right after it show the change and don't wait
/// for the index job. A failure is not the change's: the job does it later.
fn settle_now(writer: &Writer, jobs: &crate::jobs::JobQueue) {
    let volumes = system_volumes();
    let settled = writer.call(move |conn| crate::search::settle_if_small(conn, &volumes, 50));
    if let Err(e) = settled {
        eprintln!("library: couldn't update the track table's rows: {e:?}");
    }
    // Anything left over (a long list was due) is the index job's.
    if let Err(e) = crate::search::request(writer, |job| jobs.enqueue(job)) {
        eprintln!("library: couldn't ask for the index: {e:?}");
    }
}

#[cfg(test)]
mod tests;
