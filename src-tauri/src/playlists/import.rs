//! "Make an app playlist from this rekordbox playlist" (3.5): a one-time
//! copy of a rekordbox playlist, in rekordbox's order, from the newest read
//! (`rekordbox_playlist_entry`, which every read replaces, XML and database
//! alike).
//!
//! - **Library tracks only.** Each entry's rekordbox track is matched to a
//!   Library track the way the add from rekordbox finds it: through the
//!   track's trusted match to a file (`rekordbox_track.file_id`, never a
//!   probable one) and that file's track. An entry with no Library track is
//!   counted and left out, never added to the Library or guessed at.
//! - **A copy, made once.** The playlist's origin is rekordbox and its
//!   rekordbox path is kept for display; a later read never changes it.
//!   Copying the same playlist again makes another playlist.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::{insert_entries, make_in, Done, PlaylistId, PlaylistsError, FROM_REKORDBOX_OPERATION};
use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::ops::{self, OpsError};

/// A playlist of the newest rekordbox read, to copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RekordboxPlaylist {
    /// Folder names from below ROOT down to the playlist.
    pub path: Vec<String>,
    /// Its entries, every one (repeats, and tracks outside the Library,
    /// included).
    pub entries: u32,
}

/// What copying a rekordbox playlist did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CopiedFromRekordbox {
    pub playlist_id: PlaylistId,
    /// Entries copied, in rekordbox's order.
    pub added: u32,
    /// Entries left out: their track isn't a Library track (or has no
    /// trusted match to a file).
    pub not_in_library: u32,
    pub operation_id: Option<i64>,
}

/// Every playlist with entries in the newest read, in rekordbox's tree
/// order. Of two with the same path, the first.
pub fn rekordbox_playlists(conn: &Connection) -> rusqlite::Result<Vec<RekordboxPlaylist>> {
    let mut stmt = conn.prepare(
        "SELECT path, count(*) FROM rekordbox_playlist_entry
         WHERE playlist_index IN (SELECT min(playlist_index) FROM rekordbox_playlist_entry
                                  GROUP BY path)
         GROUP BY playlist_index ORDER BY playlist_index",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (path, entries) = row?;
        if let Ok(path) = serde_json::from_str(&path) {
            out.push(RekordboxPlaylist { path, entries });
        }
    }
    Ok(out)
}

/// Copies the rekordbox playlist at `path` (the first with that path) into a
/// new app playlist named `name`, as one operation.
pub fn copy_on(
    conn: &mut Connection,
    path: &[String],
    name: &str,
) -> Result<Done<CopiedFromRekordbox>, OpsError> {
    let key = serde_json::to_string(path).expect("strings always serialize");
    let name = name.trim().to_owned();
    let recorded = ops::record(
        conn,
        FROM_REKORDBOX_OPERATION,
        &serde_json::json!({ "path": path, "name": name }),
        |rec| {
            // Each entry's Library track, or none, in rekordbox's order.
            let entries: Vec<Option<i64>> = rec.read_rows(
                "SELECT lt.id
                 FROM rekordbox_playlist_entry e
                 LEFT JOIN rekordbox_track rt
                        ON rt.track_id = e.track_id AND rt.relink_probable = 0
                 LEFT JOIN recording_file rf ON rf.file_id = rt.file_id
                 LEFT JOIN library_track lt ON lt.recording_id = rf.recording_id
                 WHERE e.playlist_index = (SELECT min(playlist_index)
                                           FROM rekordbox_playlist_entry WHERE path = ?1)
                 ORDER BY e.position",
                [&key],
                |r| r.get(0),
            )?;
            if entries.is_empty() {
                return Ok(Err(PlaylistsError::RekordboxPlaylistNotFound));
            }
            let tracks: Vec<i64> = entries.iter().flatten().copied().collect();
            let id = match make_in(rec, &name, Some(path))? {
                Ok(id) => id,
                Err(refusal) => return Ok(Err(refusal)),
            };
            insert_entries(rec, id, &tracks, 0)?;
            rec.detail("tracks", tracks.len());
            Ok(Ok(CopiedFromRekordbox {
                playlist_id: id,
                added: tracks.len() as u32,
                not_in_library: (entries.len() - tracks.len()) as u32,
                operation_id: None,
            }))
        },
    )?;
    Ok(recorded.value.map(|copied| CopiedFromRekordbox {
        operation_id: recorded.operation_id,
        ..copied
    }))
}

/// The playlists of the newest rekordbox read that can be copied.
#[tauri::command]
#[specta::specta]
pub async fn rekordbox_playlists_to_copy(
    reads: State<'_, ReadPool>,
) -> Result<Vec<RekordboxPlaylist>, IpcError> {
    Ok(reads.read(rekordbox_playlists)?)
}

/// "Make an app playlist from this rekordbox playlist".
#[tauri::command]
#[specta::specta]
pub async fn playlist_from_rekordbox(
    writer: State<'_, Writer>,
    path: Vec<String>,
    name: String,
) -> Result<CopiedFromRekordbox, IpcError> {
    Ok(writer.call(move |c| Ok(copy_on(c, &path, &name)))???)
}
