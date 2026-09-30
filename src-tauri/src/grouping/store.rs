//! Where grouping meets the database: loads the files, asks
//! [`super::plan`] where each goes, and applies the answer, all in one
//! transaction. Also the removal side: letting go of a music folder's files.
//!
//! Tracks are `recording` rows, and a file's track is its `recording_file`
//! row (migration 0003). The only tables written are those two, plus
//! `rekordbox_track.recording_id`, which grouping owns: a matched
//! rekordbox track always points at its file's track.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};

use super::plan::{plan, Dest, Member, Move};

/// What one grouping pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Files that had no track and got one.
    pub placed: u64,
    /// Files that changed track.
    pub moved: u64,
    /// Tracks made.
    pub recordings_made: u64,
    /// Tracks left with no files and nothing else pointing at them, deleted.
    pub recordings_removed: u64,
    /// Files that should have moved but sit between linked versions.
    pub left_alone: u64,
}

/// A track is unreferenced when nothing but its files (`recording_file`)
/// points at it. Everything that names a track by id is listed here, so
/// deleting one never trips a foreign key (a test compares this list with
/// the schema's foreign keys).
const UNREFERENCED: &str =
    "NOT EXISTS (SELECT 1 FROM recording_file WHERE recording_id = recording.id)
     AND NOT EXISTS (SELECT 1 FROM library_track WHERE recording_id = recording.id)
     AND NOT EXISTS (SELECT 1 FROM analysis WHERE recording_id = recording.id)
     AND NOT EXISTS (SELECT 1 FROM version_link
                     WHERE recording_a = recording.id OR recording_b = recording.id)
     AND NOT EXISTS (SELECT 1 FROM rekordbox_track WHERE recording_id = recording.id)";

/// Every file with a track, and every present file without one.
fn members(conn: &Connection) -> rusqlite::Result<Vec<Member>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, rf.recording_id, f.audio_hash,
                EXISTS (SELECT 1 FROM library_track lt
                        WHERE lt.linked_file_id = f.id OR lt.source_file_id = f.id)
         FROM file f LEFT JOIN recording_file rf ON rf.file_id = f.id
         WHERE f.present = 1 OR rf.file_id IS NOT NULL
         ORDER BY f.id",
    )?;
    let rows = stmt.query_map([], |r| {
        let key: Option<Vec<u8>> = r.get(2)?;
        Ok(Member {
            file: r.get(0)?,
            recording: r.get(1)?,
            // An empty hash says nothing, like none.
            key: key.filter(|k| !k.is_empty()),
            pinned: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// The pairs of tracks that are linked versions, smaller id first.
fn version_pairs(conn: &Connection) -> rusqlite::Result<HashSet<(i64, i64)>> {
    let mut stmt = conn.prepare("SELECT recording_a, recording_b FROM version_link")?;
    let rows = stmt.query_map([], |r| {
        let (a, b): (i64, i64) = (r.get(0)?, r.get(1)?);
        Ok((a.min(b), a.max(b)))
    })?;
    rows.collect()
}

/// The tracks something other than their files points at: a Library track,
/// analysis, or a matched rekordbox track.
fn referenced(conn: &Connection) -> rusqlite::Result<HashSet<i64>> {
    let mut stmt = conn.prepare(
        "SELECT recording_id FROM library_track
         UNION SELECT recording_id FROM analysis
         UNION SELECT recording_id FROM rekordbox_track WHERE recording_id IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// Groups every file: each present file ends up in exactly one track, and
/// files with the same non-NULL `audio_hash` share it (see [`super::plan`]).
/// Then, whether or not any file moved: every matched rekordbox track
/// points at its file's track, tracks left with nothing are deleted, and
/// every track with files has a best. Safe to run again at any time; a run
/// with nothing to change writes nothing. All or nothing.
pub fn regroup(conn: &mut Connection) -> rusqlite::Result<Summary> {
    let tx = conn.transaction()?;
    let members = members(&tx)?;
    let versions = version_pairs(&tx)?;
    let referenced = referenced(&tx)?;
    let plan = plan(&members, &versions, &referenced);
    let mut summary = Summary {
        left_alone: plan.left_alone as u64,
        ..Summary::default()
    };

    let mut made: HashMap<usize, i64> = HashMap::new();
    {
        let mut make = tx.prepare_cached("INSERT INTO recording DEFAULT VALUES")?;
        let mut place = tx
            .prepare_cached("INSERT INTO recording_file (recording_id, file_id) VALUES (?1, ?2)")?;
        // A move starts over: the file's role in its old track means
        // nothing in the new one.
        let mut relocate = tx.prepare_cached(
            "UPDATE recording_file SET recording_id = ?1, role = 'undecided',
                    match_confidence = NULL
             WHERE file_id = ?2",
        )?;
        for Move { file, from, to } in &plan.moves {
            let to = match *to {
                Dest::Recording(id) => id,
                Dest::New(n) => match made.get(&n) {
                    Some(&id) => id,
                    None => {
                        make.execute([])?;
                        let id = tx.last_insert_rowid();
                        made.insert(n, id);
                        summary.recordings_made += 1;
                        id
                    }
                },
            };
            match from {
                None => {
                    place.execute(params![to, file])?;
                    summary.placed += 1;
                }
                Some(_) => {
                    relocate.execute(params![to, file])?;
                    summary.moved += 1;
                }
            }
        }
    }
    sync_rekordbox(&tx)?;
    summary.recordings_removed = sweep(&tx)?;
    ensure_best(&tx)?;
    tx.commit()?;
    Ok(summary)
}

/// Points each matched rekordbox track at its file's track. Grouping owns
/// `rekordbox_track.recording_id`: a match (relink) sets `file_id`, a fresh
/// read of the snapshot clears `recording_id`, and this puts it right
/// again after placements, moves and both of those. A row with no matched
/// file has no track either.
fn sync_rekordbox(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE rekordbox_track
         SET recording_id = (SELECT rf.recording_id FROM recording_file rf
                             WHERE rf.file_id = rekordbox_track.file_id)
         WHERE recording_id IS NOT (SELECT rf.recording_id FROM recording_file rf
                                    WHERE rf.file_id = rekordbox_track.file_id)",
        [],
    )?;
    Ok(())
}

/// Deletes every track that has no files and nothing else pointing at it,
/// wherever it came from (a move emptied it, or the last thing pointing at
/// it went). Returns how many were deleted.
fn sweep(conn: &Connection) -> rusqlite::Result<u64> {
    let deleted = conn.execute(&format!("DELETE FROM recording WHERE {UNREFERENCED}"), [])?;
    Ok(deleted as u64)
}

/// Gives every track with files but no best file one: the lowest-id file
/// still on disk (any file, if none is), skipping files marked extra. A
/// best that's already there is never changed; ranking files is 1bC-2's.
fn ensure_best(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE recording_file SET role = 'best'
         WHERE file_id IN (
             SELECT (SELECT rf.file_id
                     FROM recording_file rf JOIN file f ON f.id = rf.file_id
                     WHERE rf.recording_id = t.recording_id AND rf.role = 'undecided'
                     ORDER BY f.present DESC, rf.file_id
                     LIMIT 1)
             FROM (SELECT DISTINCT recording_id FROM recording_file) t
             WHERE NOT EXISTS (SELECT 1 FROM recording_file b
                               WHERE b.recording_id = t.recording_id AND b.role = 'best'))",
        [],
    )?;
    Ok(())
}

/// Lets go of the files of a music folder that's being removed, so their
/// `recording_file` rows don't hold the `file` rows back: deletes each
/// file's row in its track, whatever its role, then deletes any track left
/// with no files and nothing else pointing at it, and gives each track that
/// lost its best file a new one. A track that still has files in another
/// folder stays with them.
///
/// Runs inside the caller's transaction, so it rolls back with it. Rows
/// pointing straight at a file (a Library track's `linked_file_id`) aren't
/// touched: they still refuse the removal.
pub fn release_folder_files(conn: &Connection, folder: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM recording_file
         WHERE file_id IN (SELECT id FROM file WHERE music_folder_id = ?1)",
        [folder],
    )?;
    sweep(conn)?;
    ensure_best(conn)
}
