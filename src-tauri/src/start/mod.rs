//! The offer to add rekordbox tracks to the Library (1aE-2, 1aE-3; ROADMAP
//! 1.3, "Adding rekordbox tracks is always offered, never automatic").
//!
//! There is one mechanism, and nothing here runs on its own: after a
//! rekordbox read the app says how many rekordbox tracks aren't in the
//! Library ([`offer`]), and adds them only when the user asks
//! ([`add_offered`]). "Start from rekordbox" is the first use of the offer;
//! later reads use the same one, however the Library was started. Nothing
//! records how it was started, and nothing remembers a playlist pick.
//!
//! **Offered:** each track with a trusted rekordbox match
//! (`relink_probable = 0`) that isn't in the Library and whose file is on
//! disk. Several rekordbox entries for one track are one offered track.
//!
//! **Left out, and counted for the summary:**
//! - entries whose match is still probable (confirmed in Review, Phase 1b);
//! - entries with no file: they stay in Review's Missing list, and enter
//!   the offer once relink finds their file. A trusted match whose file has
//!   since gone from disk counts here too.
//!
//! **Left out, not counted:** streaming entries (never linked, ROADMAP
//! 1.2), tracks the user removed from the Library in the app
//! ([`removed_by_user`]), and matched files that grouping hasn't given a
//! track yet (they're offered once it has).
//!
//! Narrowed to chosen playlists (1aE-3), the same rules apply to the
//! entries in those playlists only. Only tracks are added: the playlists
//! themselves aren't imported (ROADMAP 3.5).
//!
//! Adding is one operation in the operation log, so one undo removes the
//! whole batch. Which file a track links to, and whether it can be added,
//! is the Library's call ([`library::check`]). Only the database is
//! touched; no audio file is opened.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::library::{self, Plan, Refusal};
use crate::ops::{self, OpsError, UndoOutcome};
use crate::rekordbox::location::{decode, Location};

#[cfg(test)]
mod tests;

/// The operation kind of an add, as the operation log stores it.
pub const ADD_OPERATION: &str = "add_rekordbox_tracks";

/// A rekordbox playlist, named by its folders from below ROOT down to the
/// playlist itself (as `rekordbox_track.playlists` stores it).
pub type PlaylistPath = Vec<String>;

/// What the offer holds now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    /// Tracks the add would put in the Library.
    pub to_add: u32,
    /// Tracks with a trusted match that are in the Library already.
    pub already_in_library: u32,
    /// rekordbox entries with no file: waiting in Review's Missing list.
    pub waiting_in_missing: u32,
    /// rekordbox entries whose match is still probable.
    pub waiting_for_confirmation: u32,
}

/// What an add did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddSummary {
    /// Library tracks made.
    pub added: u32,
    pub already_in_library: u32,
    pub waiting_in_missing: u32,
    pub waiting_for_confirmation: u32,
    /// The operation that added them, for [`undo_add`]. `None` when nothing
    /// was added.
    pub operation_id: Option<i64>,
}

/// A rekordbox playlist to pick, with how many of its entries are tracks
/// with a file or a last known path (streaming entries aren't counted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistChoice {
    pub path: PlaylistPath,
    pub tracks: u32,
}

/// The rekordbox entries sorted into what the offer does with each.
struct Sorted {
    /// Each track to add, with the file it links to.
    to_add: Vec<(i64, i64)>,
    already_in_library: u32,
    waiting_in_missing: u32,
    waiting_for_confirmation: u32,
}

impl Sorted {
    fn offer(&self) -> Offer {
        Offer {
            to_add: count(self.to_add.len()),
            already_in_library: self.already_in_library,
            waiting_in_missing: self.waiting_in_missing,
            waiting_for_confirmation: self.waiting_for_confirmation,
        }
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The tracks the user removed from the Library in the app. The offer
/// leaves them out; they can still be added back by hand (ROADMAP 1.3).
///
/// This is the one place the offer consults the Library's removal records
/// ([`library::removed_tracks`]). Adding a track back by hand clears its
/// record, so from then on it's an ordinary Library track here.
fn removed_by_user(conn: &Connection) -> rusqlite::Result<HashSet<i64>> {
    Ok(library::removed_tracks(conn)?
        .into_iter()
        .map(|removed| removed.recording_id)
        .collect())
}

/// Whether an entry with these playlists (the stored JSON array of paths)
/// is in one of the chosen ones.
fn in_chosen(playlists: &str, chosen: &HashSet<&PlaylistPath>) -> bool {
    serde_json::from_str::<Vec<PlaylistPath>>(playlists)
        .map(|paths| paths.iter().any(|p| chosen.contains(p)))
        .unwrap_or(false)
}

/// Whether an entry with no file is a streaming entry, which never has one.
fn is_streaming(location: &str) -> bool {
    matches!(decode(location), Ok(Location::Streaming(_)))
}

/// Sorts every rekordbox entry (or, with `playlists`, the entries in those
/// playlists). Changes nothing.
fn sort(conn: &Connection, playlists: Option<&[PlaylistPath]>) -> rusqlite::Result<Sorted> {
    let chosen: Option<HashSet<&PlaylistPath>> = playlists.map(|p| p.iter().collect());
    // The track is found through the file's own track row, not
    // `rekordbox_track.recording_id`, which a fresh read clears until
    // grouping runs again (as `library::linked_file_for` does).
    let mut stmt = conn.prepare(
        "SELECT rt.location, rt.file_id, rt.relink_probable, rt.playlists, rf.recording_id
         FROM rekordbox_track rt
         LEFT JOIN recording_file rf ON rf.file_id = rt.file_id
         ORDER BY rt.id",
    )?;
    let mut rows = stmt.query([])?;
    let mut trusted = BTreeSet::new();
    let mut sorted = Sorted {
        to_add: Vec::new(),
        already_in_library: 0,
        waiting_in_missing: 0,
        waiting_for_confirmation: 0,
    };
    while let Some(row) = rows.next()? {
        if let Some(chosen) = &chosen {
            if !in_chosen(row.get_ref(3)?.as_str()?, chosen) {
                continue;
            }
        }
        let file: Option<i64> = row.get(1)?;
        let probable: bool = row.get(2)?;
        let recording: Option<i64> = row.get(4)?;
        if file.is_none() {
            if !is_streaming(row.get_ref(0)?.as_str()?) {
                sorted.waiting_in_missing += 1;
            }
        } else if probable {
            sorted.waiting_for_confirmation += 1;
        } else if let Some(recording) = recording {
            trusted.insert(recording);
        }
    }

    let removed = removed_by_user(conn)?;
    for recording in trusted {
        if removed.contains(&recording) {
            continue;
        }
        match library::check(conn, recording)? {
            Ok(Plan::AlreadyThere(_)) => sorted.already_in_library += 1,
            Ok(Plan::Link(choice)) => sorted.to_add.push((recording, choice.file_id)),
            // The matched file has gone from disk since it was matched.
            Err(Refusal::FileMissing { .. }) => sorted.waiting_in_missing += 1,
            // A trusted match is never held back for a probable one.
            Err(Refusal::NoFile | Refusal::TrackNotFound | Refusal::MatchNotConfirmed) => {}
        }
    }
    Ok(sorted)
}

/// What the offer holds now: for the whole collection, or narrowed to the
/// entries in `playlists`. Changes nothing.
pub fn offer(conn: &Connection, playlists: Option<&[PlaylistPath]>) -> rusqlite::Result<Offer> {
    Ok(sort(conn, playlists)?.offer())
}

/// Adds every offered track to the Library as a linked Library track, as
/// one operation: one undo removes them all. Run it on the writer
/// connection, so nothing changes between the check and the inserts.
/// Repeating it adds nothing and logs nothing.
pub fn add_offered(
    conn: &mut Connection,
    playlists: Option<&[PlaylistPath]>,
) -> Result<AddSummary, OpsError> {
    let sorted = sort(conn, playlists)?;
    let details = serde_json::json!({ "tracks": sorted.to_add.len() });
    let recorded = ops::record(conn, ADD_OPERATION, &details, |rec| {
        for &(recording, file) in &sorted.to_add {
            library::insert_linked(rec, recording, file)?;
        }
        Ok(())
    })?;
    Ok(AddSummary {
        added: count(sorted.to_add.len()),
        already_in_library: sorted.already_in_library,
        waiting_in_missing: sorted.waiting_in_missing,
        waiting_for_confirmation: sorted.waiting_for_confirmation,
        operation_id: recorded.operation_id,
    })
}

/// Undoes the add recorded as `operation`, if it's the next operation to
/// undo. If something else was done since (or the add was undone already),
/// nothing is undone ([`UndoOutcome::NothingToUndo`]): the summary's undo
/// never undoes a different action.
pub fn undo_add(conn: &mut Connection, operation: i64) -> Result<UndoOutcome, OpsError> {
    ops::undo_only(conn, operation)
}

/// How playlist names are ordered in the pick: letter case ignored.
fn sort_key(path: &PlaylistPath) -> Vec<String> {
    path.iter().map(|name| name.to_lowercase()).collect()
}

/// Every rekordbox playlist that holds a track, with its track count, by
/// folder and name. The snapshot keeps each entry's playlists, not the tree
/// itself, so empty playlists and rekordbox's own order aren't here.
pub fn playlists(conn: &Connection) -> rusqlite::Result<Vec<PlaylistChoice>> {
    let mut stmt = conn.prepare(
        "SELECT location, file_id, playlists FROM rekordbox_track WHERE playlists <> '[]'",
    )?;
    let mut rows = stmt.query([])?;
    let mut counts: BTreeMap<(Vec<String>, PlaylistPath), u32> = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let file: Option<i64> = row.get(1)?;
        if file.is_none() && is_streaming(row.get_ref(0)?.as_str()?) {
            continue;
        }
        let paths: Vec<PlaylistPath> =
            serde_json::from_str(row.get_ref(2)?.as_str()?).unwrap_or_default();
        for path in paths {
            *counts.entry((sort_key(&path), path)).or_default() += 1;
        }
    }
    Ok(counts
        .into_iter()
        .map(|((_, path), tracks)| PlaylistChoice { path, tracks })
        .collect())
}

/// How many rekordbox tracks could be added to the Library, and how many
/// are left out and why. With `playlists`, only the tracks in those
/// rekordbox playlists. Changes nothing.
#[tauri::command]
#[specta::specta]
pub async fn rekordbox_offer(
    reads: State<'_, ReadPool>,
    playlists: Option<Vec<PlaylistPath>>,
) -> Result<Offer, IpcError> {
    Ok(reads.read(|conn| offer(conn, playlists.as_deref()))?)
}

/// Adds the offered rekordbox tracks to the Library as linked Library
/// tracks, as one undoable operation. No file is written. With `playlists`,
/// only the tracks in those rekordbox playlists; the playlists themselves
/// aren't imported and the pick isn't remembered.
#[tauri::command]
#[specta::specta]
pub async fn add_rekordbox_tracks(
    writer: State<'_, Writer>,
    playlists: Option<Vec<PlaylistPath>>,
) -> Result<AddSummary, IpcError> {
    Ok(writer.call(move |conn| Ok(add_offered(conn, playlists.as_deref())))??)
}

/// Undoes an add of rekordbox tracks, if nothing else was done since.
#[tauri::command]
#[specta::specta]
pub async fn undo_add_rekordbox_tracks(
    writer: State<'_, Writer>,
    operation_id: i64,
) -> Result<UndoOutcome, IpcError> {
    Ok(writer.call(move |conn| Ok(undo_add(conn, operation_id)))??)
}

/// The rekordbox playlists that hold tracks, for narrowing the offer.
#[tauri::command]
#[specta::specta]
pub async fn rekordbox_playlists(
    reads: State<'_, ReadPool>,
) -> Result<Vec<PlaylistChoice>, IpcError> {
    Ok(reads.read(playlists)?)
}
