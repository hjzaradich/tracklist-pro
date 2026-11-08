//! A Library track's own genre: "Use" on a genre guess of the audio model
//! (ROADMAP 3.8, owner decisions 2026-11-08; Run 5B, B1). Later "Your
//! genres" guesses use the same command with a genre's name.
//!
//! Setting it is an **ordinary edit of the `Genre` field**: the app's own
//! value in the edit store (`app_value`, ROADMAP §2 and 1.10; 1cB-18,
//! [`app_values`]), the same store that Review → Conflicts' "Edit" and a
//! merge of two Library tracks write. So:
//!
//! - It is one operation ([`SET_GENRE_OPERATION`]); one Undo puts back
//!   exactly what the store held before: the earlier app value, or none.
//!   The store's rules apply as they are: the first edit records what
//!   rekordbox showed as the value it replaced (none for a track rekordbox
//!   doesn't know), and a name equal to rekordbox's current genre clears
//!   the app's own value, so the field follows rekordbox again.
//! - The next send carries it (1.10: only the app changed), and a later
//!   change made in rekordbox is taken in or is a conflict, as for any
//!   field. Nothing here touches the send or the conflicts engine.
//! - **It wins** over rekordbox's genre and the file's tag wherever a
//!   Library track's genre is worked out: what the lists show, search and
//!   sort by (`send_values::shown_fields`), and the
//!   genre the Overview counts, the genre doors and the canonical genre
//!   (`overview::genres_of` → `track_row.genre` → `track_row.genre_node`,
//!   through [`own_genre_sql`]). A name the genre tree doesn't know is a
//!   spelling like any other: unmapped, listed in Review.
//! - Only a Library track has one (the store is per Library track); the
//!   command refuses any other.
//! - Nothing sets it automatically: only the command, on the user's press.
//!
//! # Keeping the index current
//!
//! The index job works a track's genre out again when the track is due
//! (`search_due`). Every operation that changes a row of the store for a listed field
//! (`Genre`, and since 6A also `Name`, `Artist`, `Album`, `Remixer`, `Label`),
//! and the Undo of one, marks its tracks due in its own transaction
//! ([`mark_due`], which `ops` runs for each operation and each Undo), so a
//! resolved conflict and a merge of Library tracks follow too. The merge
//! before a send, which takes rekordbox's value in outside the log, marks
//! the track itself (`app_values::take_in`).

use rusqlite::Connection;
use serde_json::json;
use tauri::State;

use crate::conflicts::app_values::{self, AppValueError};
use crate::db::Writer;
use crate::ipc::{ErrorKind, IpcError};
use crate::jobs::JobQueue;
use crate::library::LibraryTrackId;
use crate::paths::Volumes;

/// The operation setting a track's genre records.
pub const SET_GENRE_OPERATION: &str = "set_genre";

/// The rekordbox `TRACK` attribute that holds the genre: the field of the
/// edit store this module writes and reads.
pub const GENRE_FIELD: &str = "Genre";

/// The most tracks due that the command works out before it returns, as
/// for an energy set by a key press (`energy::set_energy`).
const SETTLE_NOW_LIMIT: usize = 10;

/// Why the genre wasn't set. Nothing was changed.
#[derive(Debug)]
pub enum SetGenreError {
    /// The name is blank.
    NameEmpty,
    /// The edit store refused: no such Library track, a name holding a
    /// character XML can't carry (`BadValue`), or the database.
    Store(AppValueError),
}

impl From<AppValueError> for SetGenreError {
    fn from(e: AppValueError) -> Self {
        SetGenreError::Store(e)
    }
}

impl From<SetGenreError> for IpcError {
    fn from(e: SetGenreError) -> Self {
        match e {
            SetGenreError::NameEmpty => IpcError::new(ErrorKind::GenreNameEmpty),
            SetGenreError::Store(AppValueError::NoTrack(_)) => {
                IpcError::new(ErrorKind::LibraryTrackNotFound)
            }
            SetGenreError::Store(AppValueError::BadValue { .. }) => {
                IpcError::new(ErrorKind::GenreNameNotSendable)
            }
            SetGenreError::Store(AppValueError::Ops(e)) => e.into(),
            SetGenreError::Store(e) => {
                eprintln!("set_genre: {e}");
                IpcError::new(ErrorKind::Internal)
            }
        }
    }
}

/// Sets the genre of the Library track `track` to `name` (outer blanks
/// dropped), as the app's own value of its `Genre`: one operation, one
/// Undo. Answers the operation recorded, none when nothing changed (the
/// store already held that name).
pub fn set_genre_on(
    conn: &mut Connection,
    volumes: &impl Volumes,
    track: LibraryTrackId,
    name: &str,
) -> Result<Option<i64>, SetGenreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(SetGenreError::NameEmpty);
    }
    let details = json!({
        "libraryTrackId": track.0,
        "field": GENRE_FIELD,
        // What the top bar's Undo names the step by.
        "name": name,
    });
    let recorded = app_values::set_as(
        conn,
        volumes,
        SET_GENRE_OPERATION,
        &details,
        track,
        GENRE_FIELD,
        name,
    )?;
    Ok(recorded.operation_id)
}

/// Sets a Library track's genre to a genre's name, as the user's own edit
/// (ROADMAP 3.8: "Use" on a genre guess): one operation, one Undo. Answers
/// the operation recorded, none if nothing changed. The track's row is
/// worked out now, so the lists read right after show the genre.
#[tauri::command(async)]
#[specta::specta]
pub fn set_genre(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    library_track: LibraryTrackId,
    name: String,
) -> Result<Option<i64>, IpcError> {
    let volumes = crate::scan::system_volumes();
    let operation =
        writer.call(move |conn| Ok(set_genre_on(conn, &volumes, library_track, &name)))??;
    if operation.is_some() {
        crate::library::settle_now_up_to(&writer, &jobs, SETTLE_NOW_LIMIT);
    }
    Ok(operation)
}

/// The SQL expression for the app's own genre of the Library track whose id
/// is the SQL expression `track`: `NULL` when it has none, `''` when the
/// user wants it empty. The index finds it by the store's unique key.
pub(crate) fn own_genre_sql(track: &str) -> String {
    format!(
        "(SELECT av.value FROM app_value av
          WHERE av.library_track_id = {track} AND av.field = '{GENRE_FIELD}')"
    )
}

/// The fields the lists use the app's own value of, as JSON for SQL.
fn listed_fields() -> String {
    serde_json::to_string(&crate::send_values::OWN_LISTED_FIELDS).expect("strings always serialize")
}

/// Marks due for the index every Library track whose own value of a listed
/// field (`OWN_LISTED_FIELDS`: Name, Artist, Album, Remixer, Label, Genre)
/// the operation `operation` changed: in the operation's own transaction as it
/// is recorded, and again in the Undo's. A row the operation (or its Undo)
/// took away is found by what the log recorded of it; one still there by
/// the row itself. An operation that changed no row of the store (most of
/// them) is done with after one look at its log.
pub(crate) fn mark_due(conn: &Connection, operation: i64) -> rusqlite::Result<()> {
    let touched: bool = conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM change
                            WHERE operation_id = ?1 AND entity = 'app_value')",
        )?
        .query_row([operation], |r| r.get(0))?;
    if !touched {
        return Ok(());
    }
    conn.prepare_cached(
        "INSERT OR IGNORE INTO search_due (recording_id)
         SELECT lt.recording_id FROM library_track lt
         WHERE lt.id IN (
             SELECT CAST(coalesce(t.after, t.before) AS INTEGER)
             FROM change t
             JOIN change f ON f.operation_id = t.operation_id AND f.entity = t.entity
                          AND f.entity_id = t.entity_id AND f.field = 'field'
             WHERE t.operation_id = ?1 AND t.entity = 'app_value'
               AND t.field = 'library_track_id'
               AND coalesce(f.after, f.before) IN (SELECT value FROM json_each(?2))
             UNION
             SELECT a.library_track_id
             FROM change c JOIN app_value a ON a.id = c.entity_id
             WHERE c.operation_id = ?1 AND c.entity = 'app_value' AND a.field IN (SELECT value FROM json_each(?2)))",
    )?
    .execute((operation, listed_fields()))?;
    Ok(())
}

/// Marks one Library track due for the index (its own genre changed outside
/// the log).
pub(crate) fn mark_track_due(conn: &Connection, track: LibraryTrackId) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT OR IGNORE INTO search_due (recording_id)
         SELECT recording_id FROM library_track WHERE id = ?1",
    )?
    .execute([track.0])?;
    Ok(())
}

#[cfg(test)]
mod tests;
