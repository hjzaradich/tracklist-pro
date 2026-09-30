//! Where grouping meets the database: loads the files, asks
//! [`super::plan`] where each goes, and applies the answer, all in one
//! transaction. Also the removal side: letting go of a music folder's files.
//!
//! Tracks are `recording` rows, and a file's track is its `recording_file`
//! row (migration 0003). The only tables written are those two, plus
//! `rekordbox_track.recording_id` for a file that moved, so a matched
//! rekordbox track keeps pointing at its file's track.

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
/// deleting one never trips a foreign key.
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

/// Groups every file: each present file ends up in exactly one track, and
/// files with the same non-NULL `audio_hash` share it (see [`super::plan`]).
/// Safe to run again at any time; a run with nothing to change writes
/// nothing. All or nothing.
pub fn regroup(conn: &mut Connection) -> rusqlite::Result<Summary> {
    let tx = conn.transaction()?;
    let members = members(&tx)?;
    let versions = version_pairs(&tx)?;
    let plan = plan(&members, &versions);
    let mut summary = Summary {
        left_alone: plan.left_alone as u64,
        ..Summary::default()
    };
    if plan.moves.is_empty() && plan.new_recordings == 0 {
        ensure_best(&tx)?;
        tx.commit()?;
        return Ok(summary);
    }

    let mut made: HashMap<usize, i64> = HashMap::new();
    let mut emptied: HashSet<i64> = HashSet::new();
    for Move { file, from, to } in &plan.moves {
        let to = match *to {
            Dest::Recording(id) => id,
            Dest::New(n) => match made.get(&n) {
                Some(&id) => id,
                None => {
                    tx.execute("INSERT INTO recording DEFAULT VALUES", [])?;
                    let id = tx.last_insert_rowid();
                    made.insert(n, id);
                    summary.recordings_made += 1;
                    id
                }
            },
        };
        match from {
            None => {
                tx.execute(
                    "INSERT INTO recording_file (recording_id, file_id) VALUES (?1, ?2)",
                    params![to, file],
                )?;
                summary.placed += 1;
            }
            Some(from) => {
                // A move starts over: the file's role in its old track
                // means nothing in the new one.
                tx.execute(
                    "UPDATE recording_file SET recording_id = ?1, role = 'undecided',
                            match_confidence = NULL
                     WHERE file_id = ?2",
                    params![to, file],
                )?;
                tx.execute(
                    "UPDATE rekordbox_track SET recording_id = ?1
                     WHERE file_id = ?2 AND recording_id IS NOT ?1",
                    params![to, file],
                )?;
                emptied.insert(*from);
                summary.moved += 1;
            }
        }
    }
    summary.recordings_removed = remove_unreferenced(&tx, emptied)?;
    ensure_best(&tx)?;
    tx.commit()?;
    Ok(summary)
}

/// Deletes each of `candidates` that has no files and nothing else
/// pointing at it. Returns how many were deleted.
fn remove_unreferenced(
    conn: &Connection,
    candidates: impl IntoIterator<Item = i64>,
) -> rusqlite::Result<u64> {
    let mut delete = conn.prepare(&format!(
        "DELETE FROM recording WHERE id = ?1 AND {UNREFERENCED}"
    ))?;
    let mut removed = 0;
    for id in candidates {
        removed += delete.execute([id])? as u64;
    }
    Ok(removed)
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
/// file's row in its track, whatever its role, then deletes any track it
/// leaves with no files and nothing else pointing at it. A track that still
/// has files in another folder stays with them.
///
/// Runs inside the caller's transaction. A track left without a best file
/// gets one on the next [`regroup`]. Rows pointing straight at a file (a
/// Library track's `linked_file_id`) aren't touched: they still refuse the
/// removal.
pub fn release_folder_files(conn: &Connection, folder: i64) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT rf.recording_id
         FROM recording_file rf JOIN file f ON f.id = rf.file_id
         WHERE f.music_folder_id = ?1",
    )?;
    let touched: Vec<i64> = stmt
        .query_map([folder], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    conn.execute(
        "DELETE FROM recording_file
         WHERE file_id IN (SELECT id FROM file WHERE music_folder_id = ?1)",
        [folder],
    )?;
    remove_unreferenced(conn, touched)?;
    Ok(())
}
