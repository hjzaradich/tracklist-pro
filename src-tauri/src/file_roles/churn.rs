//! A rekordbox read makes due only the tracks whose rekordbox link changed
//! (1cD-X4).
//!
//! # What the roles read of rekordbox
//!
//! One thing per file: whether a `rekordbox_track` row names it, 2 if one
//! does as a trusted match, 1 if only as a probable one, 0 if none
//! ([`facts`](super::facts); [`status_of`]). Nothing else of a row is read.
//!
//! # What a read does to it
//!
//! A read deletes every row and inserts them again unmatched, and the relink
//! job that follows matches them again. The triggers of migration 0023 see
//! that as a lost and a made match for every matched file, so every matched
//! track was made due, twice, although a read nearly always leaves each
//! file's link as it was: 66,667 tracks ranked again, about 13 s, to write
//! the roles they already had.
//!
//! # How it is held back
//!
//! The triggers can't change (no migration), so the tracks they add are
//! taken back and the real changes put in their place:
//!
//! 1. **The read** ([`begin`], [`hold`]) notes which tracks were due and
//!    each file's status, deletes and inserts the rows, and takes out of
//!    `best_file_due` the tracks that statement added: they are the only
//!    ones the read can have added, as it all happens in one transaction.
//!    Those tracks (`held`) and the statuses before the read (`before`) are
//!    kept in the `setting` row [`KEY`], in the same transaction.
//! 2. **The relink** ([`take`], [`settle`]), in its one transaction, matches
//!    every row again, and then compares each file's status with the one in
//!    `before`: a track holding a file whose status is not what it was is
//!    made due, and the tracks the relink's own updates added for files
//!    whose status is the same are taken back. A track that was already due
//!    (some other change) stays due; that is never looked at.
//! 3. **Anything else that ranks first** ([`restore`]): a roles run, a user
//!    action that ranks tracks, the start of the app. They put every held
//!    track back in `best_file_due` and delete the row, which is the old
//!    behaviour (every matched track due) exactly. So between the read and
//!    the relink the roles are those of the links before the read; nothing
//!    ranks a track on the links the read cleared without the track being
//!    due again afterwards.
//!
//! When unsure, a track is made due: an unreadable row, or a read that came
//! before another read's relink, puts every track back or compares against
//! the first read's links, never against the later, cleared ones.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// The `setting` row that carries the held tracks from a read to its relink.
const KEY: &str = "file_roles_churn";

/// What a read held back, as stored.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Held {
    /// `(file, status)` of every file with a link, before the read.
    before: Vec<(i64, i64)>,
    /// The tracks the read's own statements made due, and were taken out.
    held: Vec<i64>,
}

/// What a read noted before it changed the snapshot.
pub(crate) struct Before {
    due: HashSet<i64>,
    /// `None` when an earlier read's hold is still waiting for its relink:
    /// that read's statuses stay the ones to compare with.
    statuses: Option<HashMap<i64, i64>>,
}

/// What the rekordbox snapshot says of each file with a link: 2 trusted,
/// 1 probable only. A file with none isn't listed (0). The same as the
/// roles' own reading of it.
pub(crate) fn status_of(conn: &Connection) -> rusqlite::Result<HashMap<i64, i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT file_id, CASE WHEN MIN(relink_probable) = 0 THEN 2 ELSE 1 END
         FROM rekordbox_track WHERE file_id IS NOT NULL GROUP BY file_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

fn due_set(conn: &Connection) -> rusqlite::Result<HashSet<i64>> {
    let mut stmt = conn.prepare_cached("SELECT recording_id FROM best_file_due")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

fn stored(conn: &Connection) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM setting WHERE key = ?1", [KEY], |r| {
        r.get(0)
    })
    .optional()
}

fn write(conn: &Connection, held: &Held) -> rusqlite::Result<()> {
    let json = serde_json::to_string(held)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (KEY, json),
    )?;
    Ok(())
}

fn due_these(conn: &Connection, recordings: &[i64]) -> rusqlite::Result<()> {
    let ids = serde_json::to_string(recordings).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT OR IGNORE INTO best_file_due (recording_id)
         SELECT value FROM json_each(?1)
         WHERE value IN (SELECT id FROM recording)",
        [ids],
    )?;
    Ok(())
}

fn due_every_track(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO best_file_due (recording_id) SELECT id FROM recording",
        [],
    )?;
    Ok(())
}

/// Called by a read just before it changes the snapshot, in its transaction.
pub(crate) fn begin(conn: &Connection) -> rusqlite::Result<Before> {
    let waiting = stored(conn)?.is_some();
    Ok(Before {
        due: due_set(conn)?,
        statuses: if waiting {
            None
        } else {
            Some(status_of(conn)?)
        },
    })
}

/// Called by a read right after it has deleted and inserted the rows, in the
/// same transaction and before anything else is written: takes the tracks
/// those statements made due out of `best_file_due` and keeps them for the
/// relink ([`settle`]).
pub(crate) fn hold(conn: &Connection, before: Before) -> rusqlite::Result<()> {
    let added: Vec<i64> = due_set(conn)?.difference(&before.due).copied().collect();
    if added.is_empty() {
        return Ok(());
    }
    let ids = serde_json::to_string(&added).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "DELETE FROM best_file_due WHERE recording_id IN (SELECT value FROM json_each(?1))",
        [ids],
    )?;
    let mut held = match (before.statuses, stored(conn)?) {
        (Some(statuses), _) => Held {
            before: statuses.into_iter().collect(),
            held: Vec::new(),
        },
        // An unreadable row can't be compared with: every track is due.
        (None, Some(json)) => match serde_json::from_str(&json) {
            Ok(held) => held,
            Err(_) => {
                // Every track is due; forget the row so later reads don't
                // make every track due again.
                due_every_track(conn)?;
                conn.execute("DELETE FROM setting WHERE key = ?1", [KEY])?;
                return Ok(());
            }
        },
        (None, None) => Held::default(),
    };
    held.held.extend(added);
    held.held.sort_unstable();
    held.held.dedup();
    write(conn, &held)
}

/// Puts every held track back in `best_file_due` and forgets the hold: the
/// tracks are due as the triggers would have left them. Does nothing when
/// nothing is held. Called before anything ranks tracks, and at startup.
pub(crate) fn restore(conn: &Connection) -> rusqlite::Result<()> {
    let Some(json) = stored(conn)? else {
        return Ok(());
    };
    match serde_json::from_str::<Held>(&json) {
        Ok(held) => due_these(conn, &held.held)?,
        Err(_) => due_every_track(conn)?,
    }
    conn.execute("DELETE FROM setting WHERE key = ?1", [KEY])?;
    Ok(())
}

/// Whether a read is waiting for its relink. For a read connection.
pub(crate) fn is_held(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(stored(conn)?.is_some())
}

/// What the relink notes before it writes the matches.
pub(crate) struct Taken {
    held: Held,
    due: HashSet<i64>,
}

/// Called by the relink at the start of its transaction: the hold of a read
/// waiting for it, if any, and which tracks are due now.
pub(crate) fn take(conn: &Connection) -> rusqlite::Result<Option<Taken>> {
    let Some(json) = stored(conn)? else {
        return Ok(None);
    };
    match serde_json::from_str(&json) {
        Ok(held) => Ok(Some(Taken {
            held,
            due: due_set(conn)?,
        })),
        Err(_) => {
            restore(conn)?;
            Ok(None)
        }
    }
}

/// Called by the relink right after it wrote the matches, in the same
/// transaction and before anything else is written: makes due the tracks
/// that hold a file whose status changed since before the read, and takes
/// back the tracks the relink's own updates added for the files whose status
/// did not.
pub(crate) fn settle(conn: &Connection, taken: Option<Taken>) -> rusqlite::Result<()> {
    let Some(Taken { held, due }) = taken else {
        return Ok(());
    };
    let now = status_of(conn)?;
    let before: HashMap<i64, i64> = held.before.into_iter().collect();
    let changed: Vec<i64> = before
        .keys()
        .chain(now.keys())
        .copied()
        .collect::<HashSet<i64>>()
        .into_iter()
        .filter(|file| {
            before.get(file).copied().unwrap_or(0) != now.get(file).copied().unwrap_or(0)
        })
        .collect();
    let files = serde_json::to_string(&changed).unwrap_or_else(|_| "[]".into());
    // The tracks that hold a changed file are due; the others the relink's
    // updates added are taken back. One that was due before is not touched.
    let keep: HashSet<i64> = {
        let mut stmt = conn.prepare_cached(
            "SELECT recording_id FROM recording_file
             WHERE file_id IN (SELECT value FROM json_each(?1))",
        )?;
        let rows = stmt.query_map([&files], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    due_these(conn, &keep.iter().copied().collect::<Vec<_>>())?;
    let taken_back: Vec<i64> = due_set(conn)?
        .into_iter()
        .filter(|r| !due.contains(r) && !keep.contains(r))
        .collect();
    let ids = serde_json::to_string(&taken_back).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "DELETE FROM best_file_due WHERE recording_id IN (SELECT value FROM json_each(?1))",
        [ids],
    )?;
    conn.execute("DELETE FROM setting WHERE key = ?1", [KEY])?;
    Ok(())
}
