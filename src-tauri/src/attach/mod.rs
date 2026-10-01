//! Attaching rekordbox data to tracks (1aD-4; ROADMAP 1.2, §2).
//!
//! Relink matches each rekordbox entry to a file, and grouping keeps
//! `rekordbox_track.recording_id` pointing at that file's track. Attach is
//! the step after both: it writes what rekordbox knows about a track's
//! BPM and key into `analysis` with `source = 'rekordbox'`, so the
//! track's displayed values can say where they came from (§2 precedence).
//! Everything else rekordbox holds (grid, cues, play count, last played,
//! rating, colour, comments, My Tags, playlists) stays in
//! `rekordbox_track` and is read per track through [`for_recording`],
//! never copied.
//!
//! - **Trusted matches only.** An entry with `relink_probable = 1`, or
//!   with no track, contributes nothing: a wrong match would put one
//!   track's values on another's file. When a row turns probable, goes
//!   unmatched or lands on another track, the next run removes what it
//!   contributed. Every run decides every track again.
//! - **One rekordbox row per track.** Several entries can point at one
//!   track (a track imported twice into rekordbox). The row comes from
//!   the entry matched to the track's best file if there is one, else the
//!   entry with the highest play count, else the lowest `TrackID` (ties
//!   at each step fall to the next). The other entries stay in
//!   `rekordbox_track` for Review (1b).
//! - **Running again with nothing changed changes nothing:** a row that
//!   already holds the values is left alone, `analyzed_at` included.
//! - Only the database is touched, and only `analysis` rows from
//!   rekordbox. It runs as a background job ([`Attacher`]) asked for
//!   through [`request`] after every relink and every grouping run.

mod data;
mod job;

pub use data::{for_recording, RekordboxEntry};
pub use job::{attach_job, attacher, request, Attacher};

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::tags::key;

/// How a track's entries are ranked, the pick first: matched to the
/// track's best file, then most played, then lowest `TrackID`. Needs `rt`
/// (`rekordbox_track`) and `rf` (its file's `recording_file`).
const PICK_ORDER: &str = "COALESCE(rf.role = 'best', 0) DESC,
                          COALESCE(rt.play_count, 0) DESC,
                          rt.track_id";

/// What one attach run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Tracks that got a rekordbox row.
    pub added: u64,
    /// Rows whose values changed.
    pub updated: u64,
    /// Rows removed: their track no longer has a trusted entry (or the
    /// entry has no BPM or key).
    pub removed: u64,
}

/// The values one rekordbox row gives a track.
#[derive(Debug, Clone, PartialEq)]
struct Values {
    bpm: Option<f64>,
    key: Option<String>,
}

/// One rekordbox entry attached to a track, as far as picking goes.
struct Candidate {
    recording: i64,
    values: Values,
}

/// The BPM rekordbox gives, if it's one: unanalysed tracks have `0`.
fn bpm_value(raw: Option<f64>) -> Option<f64> {
    raw.filter(|b| b.is_finite() && *b > 0.0 && *b < 1000.0)
}

/// The Camelot key for rekordbox's `Tonality`, through the key module, so
/// the spellings are the app's one set (`F#` and `Gb` are 2B, `Bb` is 6B).
fn key_value(tonality: Option<&str>) -> Option<String> {
    key::to_camelot(tonality?)
}

/// The entry each track takes its rekordbox row from (see the module
/// docs), with the values it gives. Tracks whose entry has neither a BPM
/// nor a key are left out: a row must say something.
fn picks(conn: &Connection) -> rusqlite::Result<Vec<Candidate>> {
    // The best candidate first within each track: on the track's best
    // file, then most played, then lowest TrackID.
    let mut stmt = conn.prepare(&format!(
        "SELECT rt.recording_id, rt.bpm, rt.tonality
         FROM rekordbox_track rt
         LEFT JOIN recording_file rf
                ON rf.file_id = rt.file_id AND rf.recording_id = rt.recording_id
         WHERE rt.recording_id IS NOT NULL AND rt.relink_probable = 0
         ORDER BY rt.recording_id, {PICK_ORDER}"
    ))?;
    let mut rows = stmt.query([])?;
    let mut picked = Vec::new();
    let mut last = None;
    while let Some(r) = rows.next()? {
        let recording: i64 = r.get(0)?;
        if last == Some(recording) {
            continue;
        }
        last = Some(recording);
        let bpm: Option<f64> = r.get(1)?;
        let tonality: Option<String> = r.get(2)?;
        let values = Values {
            bpm: bpm_value(bpm),
            key: key_value(tonality.as_deref()),
        };
        if values.bpm.is_some() || values.key.is_some() {
            picked.push(Candidate { recording, values });
        }
    }
    Ok(picked)
}

/// Brings every track's rekordbox `analysis` row in line with the
/// snapshot, in one transaction. Safe to run again at any time.
pub fn attach(conn: &mut Connection) -> rusqlite::Result<Summary> {
    let tx = conn.transaction()?;
    let mut summary = Summary::default();

    let existing: HashMap<i64, (i64, Values)> = {
        let mut stmt = tx.prepare(
            "SELECT id, recording_id, bpm, key FROM analysis WHERE source = 'rekordbox'",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(1)?,
                (
                    r.get::<_, i64>(0)?,
                    Values {
                        bpm: r.get(2)?,
                        key: r.get(3)?,
                    },
                ),
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    let wanted = picks(&tx)?;
    {
        let mut insert = tx.prepare_cached(
            "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, 'rekordbox', ?2, ?3)",
        )?;
        let mut update = tx.prepare_cached(
            "UPDATE analysis SET bpm = ?2, key = ?3,
                    analyzed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
        )?;
        for c in &wanted {
            match existing.get(&c.recording) {
                None => {
                    insert.execute(params![c.recording, c.values.bpm, &c.values.key])?;
                    summary.added += 1;
                }
                Some((id, have)) if *have != c.values => {
                    update.execute(params![id, c.values.bpm, &c.values.key])?;
                    summary.updated += 1;
                }
                Some(_) => {}
            }
        }
    }

    let keep: std::collections::HashSet<i64> = wanted.iter().map(|c| c.recording).collect();
    {
        let mut remove = tx.prepare_cached("DELETE FROM analysis WHERE id = ?1")?;
        for (recording, (id, _)) in &existing {
            if !keep.contains(recording) {
                remove.execute([id])?;
                summary.removed += 1;
            }
        }
    }
    tx.commit()?;
    Ok(summary)
}

/// Whether there is anything for [`attach`] to do or undo: a rekordbox
/// entry, or a rekordbox row on a track.
pub fn any_to_attach(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM rekordbox_track)
             OR EXISTS (SELECT 1 FROM analysis WHERE source = 'rekordbox')",
        [],
        |r| r.get(0),
    )
}

#[cfg(test)]
mod tests;
