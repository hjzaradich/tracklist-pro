//! The minimal All music list (1aE-4): every track the scan found, with its
//! title, artist and a file, and whether it's in the Library. It's where a
//! Library started fresh gets its tracks ("Add to Library", the Library's
//! `promote_track`). The full track browser is Phase 1c (ROADMAP 1.11).
//!
//! - A track with no files isn't in All music (e.g. one removed from the
//!   Library after it was sent to rekordbox, kept for the send flow).
//! - The search and the order use the Library list's sort key
//!   ([`library::sort_key`]), so letter case and accents don't matter and
//!   the same tracks come in the same order in both lists.
//! - Each file's path is made readable by the Library's helper
//!   ([`library::StoredFile::shown`]).
//!
//! Read-only: only the database is read, and no file is opened.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::library::{self, sort_key, LinkedFile, StoredFile};
use crate::paths::Volumes;
use crate::scan::system_volumes;

#[cfg(test)]
mod tests;

/// The most tracks one list holds. The list isn't a browser: a search
/// narrows it instead of paging.
pub const LIST_LIMIT: usize = 200;

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
    /// otherwise its best file, or its first.
    pub file: Option<LinkedFile>,
    pub in_library: bool,
    /// One of its files is the probable match of a rekordbox track, and
    /// the match can't be confirmed yet: it can't be added to the Library
    /// ([`library::match_not_confirmed`]). The row says so and its "Add
    /// to Library" is greyed out.
    pub match_not_confirmed: bool,
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
    match_not_confirmed: bool,
}

/// A track with what the search and the order are decided on.
struct Candidate {
    recording_id: i64,
    title: Option<String>,
    artist: Option<String>,
    in_library: bool,
    /// Its best file, or else its first: the file whose name stands in for
    /// a missing title.
    file_id: i64,
    /// Every file's path inside its music folder; the first is `file_id`'s.
    paths: Vec<String>,
}

impl Candidate {
    /// The name of the file that stands in for a missing title.
    fn file_name(&self) -> &str {
        self.paths[0].rsplit('/').next().unwrap_or_default()
    }

    /// Whether the title, the artist or a file's path holds `needle` (a
    /// [`sort_key`]).
    fn matches(&self, needle: &str) -> bool {
        let holds = |text: &str| sort_key(text).contains(needle);
        self.title.as_deref().is_some_and(holds)
            || self.artist.as_deref().is_some_and(holds)
            || self.paths.iter().any(|p| holds(p))
    }

    /// The Library list's order: the title shown (the file's name when
    /// there's none), then the artist, then the track's id.
    fn order(&self) -> (String, String, i64) {
        let shown = self.title.as_deref().unwrap_or(self.file_name());
        let artist = self.artist.as_deref().unwrap_or_default();
        (sort_key(shown), sort_key(artist), self.recording_id)
    }
}

/// A value with something in it: blank text says nothing, like none.
fn filled(text: Option<String>) -> Option<String> {
    text.filter(|t| !t.trim().is_empty())
}

/// Every track that has a file.
fn candidates(conn: &Connection) -> rusqlite::Result<Vec<Candidate>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.title, r.artist, f.id, f.rel_path,
                EXISTS (SELECT 1 FROM library_track lt WHERE lt.recording_id = r.id)
         FROM recording r
         JOIN recording_file rf ON rf.recording_id = r.id
         JOIN file f ON f.id = rf.file_id
         ORDER BY r.id, (rf.role = 'best') DESC, f.id",
    )?;
    let mut rows = stmt.query([])?;
    let mut tracks: Vec<Candidate> = Vec::new();
    while let Some(row) = rows.next()? {
        let recording_id: i64 = row.get(0)?;
        let path: String = row.get(4)?;
        match tracks.last_mut() {
            Some(track) if track.recording_id == recording_id => track.paths.push(path),
            _ => tracks.push(Candidate {
                recording_id,
                title: filled(row.get(1)?),
                artist: filled(row.get(2)?),
                in_library: row.get(5)?,
                file_id: row.get(3)?,
                paths: vec![path],
            }),
        }
    }
    Ok(tracks)
}

/// The tracks matching `search` (all of them when it's blank), in the
/// Library list's order, and how many match in all.
pub fn stored(conn: &Connection, search: &str) -> rusqlite::Result<(u32, Vec<StoredTrack>)> {
    let needle = sort_key(search.trim());
    let mut matching = candidates(conn)?;
    if !needle.is_empty() {
        matching.retain(|track| track.matches(&needle));
    }
    let total = u32::try_from(matching.len()).unwrap_or(u32::MAX);
    matching.sort_by_cached_key(Candidate::order);
    matching.truncate(LIST_LIMIT);

    let mut tracks = Vec::with_capacity(matching.len());
    for track in matching {
        // The file adding the track would link, when it has one.
        let file_id = library::linked_file_for(conn, track.recording_id)?
            .map_or(track.file_id, |choice| choice.file_id);
        tracks.push(StoredTrack {
            recording_id: track.recording_id,
            title: track.title,
            artist: track.artist,
            file: library::stored_file(conn, file_id)?,
            match_not_confirmed: !track.in_library
                && library::match_not_confirmed(conn, track.recording_id)?,
            in_library: track.in_library,
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
                match_not_confirmed: t.match_not_confirmed,
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
