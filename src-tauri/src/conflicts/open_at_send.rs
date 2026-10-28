//! A field with an **open** conflict when a send goes (1cB-3; ROADMAP 1.9
//! rule 2, 1.10). The owner hasn't decided this yet: this file is the one
//! place that says what happens, so the answer can change here alone.
//!
//! **Today's default (foreman, 2026-10-28):** the send goes ahead, and for
//! that field it carries what it would carry if the app held no value of
//! its own: rekordbox's current value for a track rekordbox has (so a Yes
//! in rekordbox destroys nothing), the file's for a track it doesn't. The
//! base isn't moved ([`bases_kept`]): `sync_base` keeps the value it had
//! (or stays absent), so the conflict is still open after the next read and
//! the app's value waits in Review → Conflicts. Moving the base to
//! rekordbox's value would make the next detection read "app ≠ base,
//! rekordbox = base", only the app changed, and send the app's value over
//! rekordbox's without asking.
//!
//! The send's review says how many fields wait ([`crate::send::Preflight`]
//! `conflicts_waiting`).
//!
//! Other answers the owner may choose, and what each would change:
//! - send the app's value and keep the conflict open: [`value_sent`]
//!   returns the app's value; the base must still be kept;
//! - leave the track out of the send until it's settled: the merge would
//!   report the track as not sendable instead (`super::merge`).

use std::collections::HashSet;

use rusqlite::{params, Connection};

use super::store::json_ids;
use crate::library::LibraryTrackId;

/// The value a send carries for a field with an open conflict, given the
/// app's own value: `None` is "no value of the app's", so the send carries
/// what it carries for a field the app doesn't change (rekordbox's value,
/// or the file's for a track rekordbox doesn't have).
pub(crate) fn value_sent(_app: &str) -> Option<String> {
    None
}

/// The fields of these Library tracks with an open conflict, and every open
/// crate membership conflict whatever its track ([`super::membership`]: a
/// track the send leaves out of a playlist may be in no crate sent), by
/// track then field. The review counts them, and the go records its send
/// only while they are the ones its merge saw: a resolve or an Undo landing
/// between the merge and the record would otherwise record a base the file
/// doesn't match, or write a playlist the decision no longer matches
/// ([`crate::send`]).
///
/// A membership conflict's base is its own (`conflict.base_value`) while it
/// is open, so no send moves it, whatever `sent_entry` records.
pub fn open_fields(
    conn: &Connection,
    tracks: &[LibraryTrackId],
) -> rusqlite::Result<Vec<(LibraryTrackId, String)>> {
    conn.prepare_cached(
        "SELECT library_track_id, field FROM conflict
         WHERE status = 'open'
           AND (library_track_id IN (SELECT value FROM json_each(?1))
                OR substr(field, 1, length(?2)) = ?2)
         ORDER BY library_track_id, field",
    )?
    .query_map(params![json_ids(tracks), super::membership::FIELD_PREFIX], |r| {
        Ok((LibraryTrackId(r.get(0)?), r.get(1)?))
    })?
    .collect()
}

/// The fields of Library track `track` whose base a send doesn't move:
/// those with an open conflict as the send is recorded. Read inside the
/// send's own transaction ([`crate::rekordbox_write::record_send`]).
pub fn bases_kept(conn: &Connection, track: i64) -> rusqlite::Result<HashSet<String>> {
    conn.prepare_cached(
        "SELECT field FROM conflict WHERE library_track_id = ?1 AND status = 'open'",
    )?
    .query_map([track], |r| r.get(0))?
    .collect()
}
