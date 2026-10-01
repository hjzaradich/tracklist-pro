//! The minimal All music list (1aE-4): every track the scan found, with its
//! title, artist and a file, and whether it's in the Library. It's where a
//! Library started fresh gets its tracks ("Add to Library", the Library's
//! `promote_track`). The full track browser is Phase 1c (ROADMAP 1.11).
//!
//! Read-only: only the database is read, and no file is opened.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::library;
use crate::paths::{RelPath, StoredPath, Volumes};
use crate::scan::{display_path, system_volumes};
use crate::volume::VolumeId;

#[cfg(test)]
mod tests;

/// The most tracks one list holds. The list isn't a browser: a search
/// narrows it instead of paging.
pub const LIST_LIMIT: u32 = 200;

/// A file of a track, as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrackFile {
    /// Where the file is, e.g. `E:\DJ Music\a.mp3`: under its volume's
    /// mount point now, or where that was last seen if the volume is
    /// offline.
    pub path: String,
    /// The file's name, e.g. `a.mp3`.
    pub name: String,
    /// Whether the last scan found it on disk.
    pub present: bool,
}

/// A track in All music.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AllMusicTrack {
    /// The track's id, as "Add to Library" takes it.
    pub recording_id: i64,
    /// `None` when the track has none yet; the list then shows the file's
    /// name as the title.
    pub title: Option<String>,
    pub artist: Option<String>,
    /// The file adding it to the Library would link (ROADMAP 1.8), or
    /// otherwise its first file.
    pub file: Option<TrackFile>,
    pub in_library: bool,
}

/// The tracks matching a search, up to [`LIST_LIMIT`] of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AllMusicList {
    /// How many tracks match, shown or not.
    pub total: u32,
    pub tracks: Vec<AllMusicTrack>,
}

/// A track as stored, before its file's path is made readable.
pub struct StoredTrack {
    recording_id: i64,
    title: Option<String>,
    artist: Option<String>,
    file: Option<StoredFile>,
    in_library: bool,
}

struct StoredFile {
    volume: Option<VolumeId>,
    /// From the volume's mount point; `None` if the stored path doesn't
    /// read back.
    rel: Option<RelPath>,
    /// Inside its music folder, as stored.
    rel_path: String,
    last_mount_path: Option<String>,
    present: bool,
}

impl StoredFile {
    fn shown(&self, volumes: &impl Volumes) -> TrackFile {
        let name = self
            .rel_path
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let path = match (&self.volume, &self.rel) {
            (Some(volume), Some(rel)) => {
                match StoredPath::new(volume.clone(), rel.clone()).resolve(volumes) {
                    Ok(abs) => display_path(&abs),
                    // Offline: under the last mount point, built by hand so
                    // it reads the same on every platform.
                    Err(_) => {
                        let mount = self.last_mount_path.as_deref().unwrap_or_default();
                        let mut path = mount.trim_end_matches('\\').to_owned();
                        for part in rel.components() {
                            if !path.is_empty() {
                                path.push('\\');
                            }
                            path.push_str(part);
                        }
                        path
                    }
                }
            }
            _ => self.rel_path.replace('/', "\\"),
        };
        TrackFile {
            path,
            name,
            present: self.present,
        }
    }
}

/// A value with something in it: blank text says nothing, like none.
fn filled(text: Option<String>) -> Option<String> {
    text.filter(|t| !t.trim().is_empty())
}

/// `search` as a LIKE pattern matching it anywhere, taken literally.
fn like_pattern(search: &str) -> String {
    let mut pattern = String::from("%");
    for c in search.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// Tracks that have a file, and match the search (in the title, the artist
/// or a file's path) when there is one.
const MATCHING: &str = "FROM recording r
     WHERE EXISTS (SELECT 1 FROM recording_file rf WHERE rf.recording_id = r.id)
       AND (?1 = '%%'
            OR r.title LIKE ?1 ESCAPE '\\'
            OR r.artist LIKE ?1 ESCAPE '\\'
            OR EXISTS (SELECT 1 FROM recording_file rf JOIN file f ON f.id = rf.file_id
                       WHERE rf.recording_id = r.id AND f.rel_path LIKE ?1 ESCAPE '\\'))";

fn stored_file(conn: &Connection, file_id: i64) -> rusqlite::Result<Option<StoredFile>> {
    conn.query_row(
        "SELECT v.identity, mf.rel_path, f.rel_path, v.last_mount_path, f.present
         FROM file f
         LEFT JOIN music_folder mf ON mf.id = f.music_folder_id
         LEFT JOIN volume v ON v.id = mf.volume_id
         WHERE f.id = ?1",
        [file_id],
        |r| {
            let identity: Option<String> = r.get(0)?;
            let folder: Option<String> = r.get(1)?;
            let rel_path: String = r.get(2)?;
            let volume = identity.and_then(|i| VolumeId::from_stored(i).ok());
            let rel = match (
                folder.map(|f| RelPath::parse(&f)),
                RelPath::parse(&rel_path),
            ) {
                (Some(Ok(folder)), Ok(rel)) => Some(folder.join(&rel)),
                _ => None,
            };
            Ok(StoredFile {
                volume,
                rel,
                rel_path,
                last_mount_path: r.get(3)?,
                present: r.get(4)?,
            })
        },
    )
    .optional()
}

/// The file the list shows for a track: the one adding it would link,
/// otherwise its first file.
fn shown_file(conn: &Connection, recording: i64) -> rusqlite::Result<Option<i64>> {
    if let Some(choice) = library::linked_file_for(conn, recording)? {
        return Ok(Some(choice.file_id));
    }
    conn.query_row(
        "SELECT min(file_id) FROM recording_file WHERE recording_id = ?1",
        [recording],
        |r| r.get(0),
    )
}

/// The tracks matching `search` (all of them when it's blank): titled ones
/// first by title, then artist, letter case ignored.
pub fn stored(conn: &Connection, search: &str) -> rusqlite::Result<(u32, Vec<StoredTrack>)> {
    let pattern = like_pattern(search.trim());
    let total: u32 = conn.query_row(&format!("SELECT count(*) {MATCHING}"), [&pattern], |r| {
        r.get(0)
    })?;
    let mut stmt = conn.prepare(&format!(
        "SELECT r.id, r.title, r.artist,
                EXISTS (SELECT 1 FROM library_track lt WHERE lt.recording_id = r.id)
         {MATCHING}
         ORDER BY coalesce(trim(r.title), '') = '', r.title COLLATE NOCASE,
                  r.artist COLLATE NOCASE, r.id
         LIMIT {LIST_LIMIT}"
    ))?;
    let rows = stmt
        .query_map([&pattern], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut tracks = Vec::with_capacity(rows.len());
    for (recording_id, title, artist, in_library) in rows {
        let file = match shown_file(conn, recording_id)? {
            Some(file_id) => stored_file(conn, file_id)?,
            None => None,
        };
        tracks.push(StoredTrack {
            recording_id,
            title: filled(title),
            artist: filled(artist),
            file,
            in_library,
        });
    }
    Ok((total, tracks))
}

/// The list for the frontend, with each file's path made readable.
pub fn list(total: u32, tracks: &[StoredTrack], volumes: &impl Volumes) -> AllMusicList {
    AllMusicList {
        total,
        tracks: tracks
            .iter()
            .map(|t| AllMusicTrack {
                recording_id: t.recording_id,
                title: t.title.clone(),
                artist: t.artist.clone(),
                file: t.file.as_ref().map(|f| f.shown(volumes)),
                in_library: t.in_library,
            })
            .collect(),
    }
}

/// The tracks in All music whose title, artist or file path holds `search`
/// (every track when it's blank), up to 200, with whether each is in the
/// Library.
#[tauri::command]
#[specta::specta]
pub async fn all_music_tracks(
    reads: State<'_, ReadPool>,
    search: Option<String>,
) -> Result<AllMusicList, IpcError> {
    let search = search.unwrap_or_default();
    let (total, tracks) = reads.read(|conn| stored(conn, &search))?;
    Ok(list(total, &tracks, &system_volumes()))
}
