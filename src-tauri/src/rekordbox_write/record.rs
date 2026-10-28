//! Recording a send (ROADMAP 1.9 rule 8): `sync_base` for every field
//! sent, what each base replaced, each track's playlist entries, and the
//! mark on each Library track, in one transaction.
//!
//! `sync_base` holds, per Library track and field, the value last sent to
//! rekordbox, so the next read can tell "the app changed it", "rekordbox
//! changed it" and "both did" apart (1.10). A field's name is its
//! rekordbox XML attribute name, and its value is the text written,
//! unescaped: `Location` in the encoded form sent. `TrackID` isn't a
//! field (rekordbox reassigns it on import) and isn't recorded. Nor are
//! the analysis fields, whatever the caller hands in: they're rekordbox's,
//! the app never edits them, and they can never conflict (1.10), so a
//! base for one has no use, even when Phase 2 sends them (rule 3, case B).
//!
//! # What a base replaced (`sync_base.replaced`, 1cB-19)
//!
//! With each base goes the value it replaced in rekordbox: what rekordbox's
//! entry showed for the field when the send was recorded. The entry is the
//! one at the `Location` sent, in the newest read, and only for a track the
//! send knew rekordbox has (a trusted entry); with none, or with two rows
//! of that read at the `Location`, nothing is recorded. Nothing is recorded
//! either when the base is the value rekordbox showed, by the field's kind
//! ([`crate::conflicts::same`]): the send replaced nothing. Empty: the entry
//! had no such attribute. The conflict engine reads it as what the app's
//! sent value replaced (1.10, decision table row 7: a send never imported
//! isn't "rekordbox changed it back").
//!
//! [`forget_taken_up`] forgets a track's records once a read shows the send
//! taken up, so that a later change back in rekordbox is rekordbox's own.
//! Every read runs it ([`crate::rekordbox::store::replace_snapshot`]).
//!
//! # Playlist entries (`sent_entry`, 1cB-19)
//!
//! For every playlist the send wrote, which Library tracks it put in it
//! (one row per track and playlist, however often it's named): the base of
//! playlist membership. A playlist's rows are those of the last send that
//! wrote it: a send replaces the rows of each playlist it writes, and
//! leaves those of a playlist it doesn't write as they were. [`sent_entries`]
//! reads them.
//!
//! The folders and playlists the send wrote are kept too, in the same
//! transaction (`sent_playlist`, ROADMAP 1.9 rule 6): the path of each, so
//! a later read can tell what the app itself put in rekordbox.
//!
//! A send isn't an operation in the undo log. Its new `sync_base` and
//! `sent_entry` rows never take the row id of a deleted one (migrations
//! 0018 and 0036), so undoing the removal (or merge) of a track still works
//! after a send: the track comes back with the rows it had. Of those, a
//! playlist entry whose playlist a later send wrote again no longer counts
//! ([`sent_entries`]): that send left the track out. Undoing the add of a
//! track that was sent since is refused (`ops`).

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};

use super::{SentPath, SentTrack, ANALYSIS_ATTRIBUTES};
use crate::conflicts::same;
use crate::library::LibraryTrackId;
use crate::rekordbox::location::decode;

/// rekordbox's analysis elements, named as `conflict.field` names them.
const ANALYSIS_ELEMENTS: [&str; 2] = ["TEMPO", "POSITION_MARK"];

/// Whether a field is rekordbox's analysis, which never gets a base.
fn is_analysis(field: &str) -> bool {
    ANALYSIS_ATTRIBUTES.contains(&field) || ANALYSIS_ELEMENTS.contains(&field)
}

/// The attributes of rekordbox's entry at `location` in the read made at
/// `read_at`, when that read has exactly one row there: by name, values as
/// read. `None` when the `Location` can't be read or the read has no row,
/// or two, there.
fn entry_at(
    conn: &Connection,
    location: &str,
    read_at: &str,
) -> rusqlite::Result<Option<HashMap<String, String>>> {
    let Ok(location) = decode(location) else {
        return Ok(None);
    };
    let rows: Vec<i64> = conn
        .prepare_cached(
            "SELECT id FROM rekordbox_track WHERE location_key = ?1 AND read_at = ?2 LIMIT 2",
        )?
        .query_map(params![location.match_key(), read_at], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let [row] = rows[..] else {
        return Ok(None);
    };
    let mut attributes = conn.prepare_cached(
        "SELECT j.key, CAST(j.atom AS TEXT) FROM rekordbox_track rt, json_each(rt.attributes) j
         WHERE rt.id = ?1",
    )?;
    let attributes = attributes
        .query_map([row], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?)))?
        .filter_map(|row| match row {
            Ok((name, Some(value))) => Some(Ok((name, value))),
            Ok((_, None)) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(attributes))
}

/// What a base replaced, given what rekordbox's entry showed: `None` when
/// it's the same value by the field's kind.
fn replaced(field: &str, base: &str, showing: &HashMap<String, String>) -> Option<String> {
    let rekordbox = showing.get(field).map_or("", String::as_str);
    (!same(field, Some(rekordbox), Some(base))).then(|| rekordbox.to_owned())
}

/// Records one send: for each track in `sent`, its `sync_base` becomes
/// exactly the fields sent (a field sent before and not this time is
/// dropped; no value is kept twice), each with what it replaced in
/// rekordbox (module docs), and the track's `last_sent_location` and
/// `last_exported_at` are set. Each playlist in `paths` gets exactly its
/// entries' tracks as its `sent_entry` rows. All of it or none: any
/// failure, a track that isn't in the Library included, leaves the
/// database as it was. Tracks that weren't sent aren't touched. An
/// analysis field is never recorded, even if `sent` carries one.
///
/// Call it once per send, after the file is in place. Calling it again
/// with the same send is safe (it replaces, never duplicates), so if it
/// fails once the file is in place, call it again: until it succeeds the
/// bases are the previous send's, and what this send changed would look
/// like rekordbox's edits.
pub fn record_send(
    conn: &mut Connection,
    sent: &[SentTrack],
    paths: &[SentPath],
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let now: String = tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
        r.get(0)
    })?;
    // The newest read: what rekordbox shows as the send is recorded.
    let newest: Option<String> =
        tx.query_row("SELECT max(read_at) FROM rekordbox_track", [], |r| r.get(0))?;
    {
        let mut mark = tx.prepare(
            "UPDATE library_track SET last_sent_location = ?2, last_exported_at = ?3
             WHERE id = ?1",
        )?;
        // A field sent again keeps its row, so nothing that refers to the
        // row by id is left pointing at nothing.
        let mut base = tx.prepare(
            "INSERT INTO sync_base (library_track_id, field, value, synced_at, replaced)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (library_track_id, field)
             DO UPDATE SET value = excluded.value, synced_at = excluded.synced_at,
                           replaced = excluded.replaced",
        )?;
        let mut drop_unsent = tx.prepare(
            "DELETE FROM sync_base
             WHERE library_track_id = ?1
               AND field NOT IN (SELECT value FROM json_each(?2))",
        )?;
        // Every folder and playlist this send wrote, kept across sends: a
        // path already there is updated, never duplicated, and keeps its
        // `seen` mark. Done first: a track that fails below rolls them back too.
        let mut path = tx.prepare(
            "INSERT INTO sent_playlist (path, path_key, kind, sent_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (path_key, kind)
             DO UPDATE SET path = excluded.path, sent_at = excluded.sent_at",
        )?;
        for sent_path in paths {
            path.execute(params![
                serde_json::to_string(&sent_path.path).expect("strings always serialize"),
                serde_json::to_string(&sent_path.key()).expect("strings always serialize"),
                if sent_path.folder {
                    "folder"
                } else {
                    "playlist"
                },
                now,
            ])?;
        }
        for track in sent {
            let id = track.library_track.0;
            let marked = mark.execute(params![id, track.location(), now])?;
            if marked != 1 {
                // Not a Library track. Dropping the transaction rolls
                // everything back.
                return Err(rusqlite::Error::StatementChangedRows(marked));
            }
            // rekordbox's entry, for a track the send knew it has.
            let showing = match &newest {
                Some(read) if track.in_rekordbox => entry_at(&tx, track.location(), read)?,
                _ => None,
            };
            let fields: Vec<(&str, &str)> = track
                .fields()
                .filter(|(field, _)| !is_analysis(field))
                .collect();
            for (field, value) in &fields {
                let replaced = showing
                    .as_ref()
                    .and_then(|showing| replaced(field, value, showing));
                base.execute(params![id, field, value, now, replaced])?;
            }
            let fields: Vec<&str> = fields.iter().map(|(field, _)| *field).collect();
            let fields = serde_json::to_string(&fields).expect("strings always serialize");
            drop_unsent.execute(params![id, fields])?;
        }
        record_entries(&tx, paths, &now)?;
    }
    tx.commit()
}

/// Each playlist's `sent_entry` rows become exactly the tracks this send
/// put in it. A playlist this send didn't write keeps its rows.
fn record_entries(conn: &Connection, paths: &[SentPath], now: &str) -> rusqlite::Result<()> {
    let mut drop_left = conn.prepare(
        "DELETE FROM sent_entry
         WHERE path_key = ?1 AND library_track_id NOT IN (SELECT value FROM json_each(?2))",
    )?;
    // A track put in again keeps its row.
    let mut entry = conn.prepare(
        "INSERT INTO sent_entry (library_track_id, path, path_key, sent_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (library_track_id, path_key)
         DO UPDATE SET path = excluded.path, sent_at = excluded.sent_at",
    )?;
    for playlist in paths.iter().filter(|p| !p.folder) {
        let path = serde_json::to_string(&playlist.path).expect("strings always serialize");
        let key = serde_json::to_string(&playlist.key()).expect("strings always serialize");
        let mut seen = HashSet::new();
        let tracks: Vec<i64> = playlist
            .entries
            .iter()
            .map(|t| t.0)
            .filter(|t| seen.insert(*t))
            .collect();
        let ids = serde_json::to_string(&tracks).expect("numbers always serialize");
        drop_left.execute(params![key, ids])?;
        for track in tracks {
            entry.execute(params![track, path, key, now])?;
        }
    }
    Ok(())
}

/// Forgets the record of what a send replaced (`sync_base.replaced`) for
/// every track whose send the read made at `read_at` shows taken up: its
/// entry at the `Location` sent, a row of that read, shows the base of a
/// field the send changed (one with a record), by the field's kind. An
/// import overwrites a whole entry, so then every record of the track goes;
/// a field rekordbox shows otherwise since is rekordbox's change.
///
/// A row an incomplete read kept from an earlier one isn't of this read,
/// and forgets nothing (§5.3); nor does a `Location` with no row of this
/// read, or two. Runs inside the read's transaction, after its rows are in.
/// Not an operation: the records are derived from sends and reads, like
/// the sends that write them.
pub fn forget_taken_up(conn: &Connection, read_at: &str) -> rusqlite::Result<()> {
    // Every track with a record, its records, and the Location it was sent to.
    let mut stmt = conn.prepare(
        "SELECT b.library_track_id, b.field, b.value, l.value
         FROM sync_base b
         JOIN sync_base l ON l.library_track_id = b.library_track_id AND l.field = 'Location'
         WHERE b.replaced IS NOT NULL
         ORDER BY b.library_track_id, b.id",
    )?;
    let mut tracks: Vec<(i64, String, Vec<(String, String)>)> = Vec::new();
    for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))? {
        let (track, field, base, location): (i64, String, String, String) = row?;
        match tracks.last_mut() {
            Some((last, _, fields)) if *last == track => fields.push((field, base)),
            _ => tracks.push((track, location, vec![(field, base)])),
        }
    }
    let mut forget = conn.prepare(
        "UPDATE sync_base SET replaced = NULL
         WHERE library_track_id = ?1 AND replaced IS NOT NULL",
    )?;
    for (track, location, fields) in tracks {
        let Some(showing) = entry_at(conn, &location, read_at)? else {
            continue;
        };
        let taken_up = fields.iter().any(|(field, base)| {
            same(
                field,
                Some(showing.get(field).map_or("", String::as_str)),
                Some(base),
            )
        });
        if taken_up {
            forget.execute([track])?;
        }
    }
    Ok(())
}

/// A playlist a send put a Library track in ([`sent_entries`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentEntry {
    pub library_track: LibraryTrackId,
    /// The names from the top-level folder (`Crates` or `Playlists`) down
    /// to the playlist, as the send wrote them.
    pub path: Vec<String>,
    /// The same names normalized as the writer compares siblings
    /// ([`SentPath::key`]).
    pub key: Vec<String>,
}

/// The playlists the last sends put these Library tracks in: per playlist,
/// the last send that wrote it. Ordered by track, then by the normalized
/// path. A track no send put in a playlist has none.
///
/// A row an Undo put back (of a removal or a merge) whose playlist a later
/// send wrote again isn't that send's, so it isn't one: that send left the
/// track out of the playlist. Such a row goes the next time the playlist
/// is written.
pub fn sent_entries(
    conn: &Connection,
    tracks: &[LibraryTrackId],
) -> rusqlite::Result<Vec<SentEntry>> {
    let ids = serde_json::to_string(&tracks.iter().map(|t| t.0).collect::<Vec<_>>())
        .expect("numbers always serialize");
    let parse = |text: String| -> rusqlite::Result<Vec<String>> {
        serde_json::from_str(&text).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
    };
    let rows: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT e.library_track_id, e.path, e.path_key FROM sent_entry e
             WHERE e.library_track_id IN (SELECT value FROM json_each(?1))
               AND NOT EXISTS (SELECT 1 FROM sent_playlist p
                               WHERE p.path_key = e.path_key AND p.kind = 'playlist'
                                 AND p.sent_at > e.sent_at)
             ORDER BY e.library_track_id, e.path_key",
        )?
        .query_map([ids], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(track, path, key)| {
            Ok(SentEntry {
                library_track: LibraryTrackId(track),
                path: parse(path)?,
                key: parse(key)?,
            })
        })
        .collect()
}
