//! The user's energy for tracks (ROADMAP §2 `analysis`, 1.11 "Keyboard";
//! 1cB-8): keys 1 to 0 give every selected track an energy from 1 to 10.
//!
//! # What is stored
//!
//! An `analysis` row with source `user` (energy only; the table's checks
//! allow no BPM or key for it). The display already ranks it first for
//! energy (`recording_display`), above Mixed In Key's, the local estimate
//! and the audio model's, so the user's word wins and taking it away (Undo)
//! brings the next source's value back. Nothing is written to a file, and
//! nothing is sent to rekordbox unless the user opted in to energy as the star
//! rating (ROADMAP §7, "How energy reaches rekordbox"; `crate::energy_stars`).
//! A Library track's row in the track table keeps the energy it shows, for
//! the Overview and its doors: a trigger marks the track for the index job
//! whatever path changed the `analysis` row (migration 0043).
//!
//! # The operation
//!
//! [`set_energy_on`] is one operation ([`SET_ENERGY_OPERATION`]) for all the
//! tracks given, however many: it inserts the user's row where a track has
//! none and changes it where it has one, so one Undo puts every track back
//! as it was. A track that already has that energy is skipped, and when
//! nothing changes nothing is recorded. A track that is gone is skipped too
//! (it was merged away or removed since the screen listed it); it is not an
//! error, because the rest were asked for.
//!
//! # Clearing
//!
//! [`clear_energy_on`] is one operation ([`CLEAR_ENERGY_OPERATION`]) that
//! takes the user's own row away from every track given that has one
//! ([`clear_in`]). Only the user's word goes: another source's energy
//! (Mixed In Key, the estimate, the audio model) then shows, as the ranking
//! says, and a track with no energy of the user's is left alone. It is
//! undoable as one step. It does not touch `stars_held`: a track that shows
//! no energy afterwards (no other source's shows) has no rating of the app's own, so a send keeps
//! rekordbox's stars, as for any track with no energy
//! (`crate::energy_stars`).

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::db::{ReadPool, Writer};
use crate::ipc::{ErrorKind, IpcError};
use crate::jobs::JobQueue;
use crate::ops::{self, OpsError, Recorder};

/// The operation setting energy records.
pub const SET_ENERGY_OPERATION: &str = "set_energy";

/// The operation clearing the user's energy records.
pub const CLEAR_ENERGY_OPERATION: &str = "clear_energy";

/// The lowest and highest energy (ROADMAP §1.1, "Energy").
pub const LOWEST: i64 = 1;
pub const HIGHEST: i64 = 10;

/// What setting energy did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnergySet {
    /// Tracks whose energy is now the one asked for and was not.
    pub changed: u32,
    /// Tracks left as they were: they had that energy, or are gone.
    pub skipped: u32,
    /// The operation Undo takes back; none if nothing changed.
    pub operation_id: Option<i64>,
}

/// Why energy was not set. Nothing was changed.
#[derive(Debug)]
pub enum Refused {
    /// Not a whole number from [`LOWEST`] to [`HIGHEST`]: a bug in the caller.
    NotAnEnergy(i64),
    /// The database or the operation log failed.
    Ops(OpsError),
}

impl From<OpsError> for Refused {
    fn from(e: OpsError) -> Self {
        Refused::Ops(e)
    }
}

impl From<rusqlite::Error> for Refused {
    fn from(e: rusqlite::Error) -> Self {
        Refused::Ops(OpsError::from(e))
    }
}

impl From<Refused> for IpcError {
    fn from(e: Refused) -> Self {
        match e {
            Refused::NotAnEnergy(energy) => {
                eprintln!("set_energy: {energy} is not an energy");
                IpcError::new(ErrorKind::Internal)
            }
            Refused::Ops(e) => e.into(),
        }
    }
}

/// The `library_track` column that marks a track whose stars stay
/// rekordbox's (`energy_stars`, migration 0041). Setting an energy on the
/// track ends it.
pub(crate) const STARS_HELD: &str = "stars_held";

/// What a track holds now: the id of its `user` row and the energy in it.
type Held = (Option<i64>, Option<i64>);

/// Gives the tracks `recordings` (`recording` ids) the energy `energy`, as
/// one operation. A track listed twice counts once.
pub fn set_energy_on(
    conn: &mut Connection,
    recordings: &[i64],
    energy: i64,
) -> Result<EnergySet, Refused> {
    if !(LOWEST..=HIGHEST).contains(&energy) {
        return Err(Refused::NotAnEnergy(energy));
    }
    let recorded = ops::record(conn, SET_ENERGY_OPERATION, &serde_json::json!({}), |rec| {
        let changed = set_in(rec, recordings, energy)?;
        rec.detail("tracks", changed);
        Ok(changed)
    })?;
    let asked = recordings
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    Ok(EnergySet {
        changed: recorded.value,
        skipped: (asked as u32).saturating_sub(recorded.value),
        operation_id: recorded.operation_id,
    })
}

/// [`set_energy_on`]'s writes, inside an operation the caller records: the
/// user's energy of each track in `recordings` becomes `energy`. Answers
/// how many changed. `energy` must be from [`LOWEST`] to [`HIGHEST`] (the
/// table refuses any other). The conflict engine uses it to settle a star
/// conflict in the resolve's own operation (`energy_stars::adopt`).
pub(crate) fn set_in(
    rec: &mut Recorder<'_>,
    recordings: &[i64],
    energy: i64,
) -> Result<u32, OpsError> {
    let wanted = serde_json::to_string(recordings).expect("numbers always serialize");
    // The tracks that exist, with the user's row if they have one.
    let held: HashMap<i64, Held> = rec
        .read_rows(
            "SELECT r.id, a.id, a.energy
                 FROM recording r
                 LEFT JOIN analysis a ON a.recording_id = r.id AND a.source = 'user'
                 WHERE r.id IN (SELECT value FROM json_each(?1))",
            [wanted.as_str()],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    (r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?),
                ))
            },
        )?
        .into_iter()
        .collect();
    let mut done = std::collections::HashSet::new();
    let mut changed = 0u32;
    for &recording in recordings {
        if !done.insert(recording) {
            continue;
        }
        match held.get(&recording) {
            None => {}
            Some((_, Some(now))) if *now == energy => {}
            Some((Some(row), _)) => {
                rec.set("analysis", *row, "energy", energy)?;
                changed += 1;
            }
            Some((None, _)) => {
                rec.insert(
                    "analysis",
                    &[
                        ("recording_id", recording.into()),
                        ("source", Value::Text("user".to_owned())),
                        ("energy", energy.into()),
                    ],
                )?;
                changed += 1;
            }
        }
    }
    // Setting an energy ends a track's "stars stay rekordbox's" marker, in
    // the same operation, so Undo puts it back with the energy.
    let marked = rec.read_rows(
        "SELECT id FROM library_track
         WHERE stars_held = 1 AND recording_id IN (SELECT value FROM json_each(?1))",
        [wanted.as_str()],
        |r| r.get::<_, i64>(0),
    )?;
    for track in marked {
        rec.set("library_track", track, STARS_HELD, 0)?;
    }
    Ok(changed)
}

/// Takes the user's own energy away from the track `recording`, inside an
/// operation the caller records: it then shows the next source's energy
/// (Mixed In Key's, the local estimate's, the audio model's), or none.
/// Answers whether there was one to take away.
pub(crate) fn clear_in(rec: &mut Recorder<'_>, recording: i64) -> Result<bool, OpsError> {
    let row = rec
        .read_rows(
            "SELECT id FROM analysis WHERE recording_id = ?1 AND source = 'user'",
            [recording],
            |r| r.get::<_, i64>(0),
        )?
        .pop();
    match row {
        Some(row) => {
            rec.delete("analysis", row)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Takes the user's own energy away from the tracks `recordings`
/// (`recording` ids), as one operation. A track with none, or that is gone,
/// is left alone (counted in `skipped`); when nothing changes nothing is
/// recorded.
pub fn clear_energy_on(conn: &mut Connection, recordings: &[i64]) -> Result<EnergySet, OpsError> {
    let mut asked = std::collections::HashSet::new();
    let recorded = ops::record(
        conn,
        CLEAR_ENERGY_OPERATION,
        &serde_json::json!({}),
        |rec| {
            let mut cleared = 0u32;
            for &recording in recordings {
                if asked.insert(recording) && clear_in(rec, recording)? {
                    cleared += 1;
                }
            }
            rec.detail("tracks", cleared);
            Ok(cleared)
        },
    )?;
    Ok(EnergySet {
        changed: recorded.value,
        skipped: (asked.len() as u32).saturating_sub(recorded.value),
        operation_id: recorded.operation_id,
    })
}

/// How many of the tracks `recordings` have an energy of the user's own, for
/// the screen to offer "Clear" only when there is something to clear.
pub fn user_energy_among(conn: &Connection, recordings: &[i64]) -> rusqlite::Result<u32> {
    let wanted = serde_json::to_string(recordings).expect("numbers always serialize");
    conn.query_row(
        "SELECT count(DISTINCT recording_id) FROM analysis
         WHERE source = 'user' AND recording_id IN (SELECT value FROM json_each(?1))",
        [wanted.as_str()],
        |r| r.get(0),
    )
}

/// The most tracks due that a key press works out before it returns (PR 156
/// review, nit 3). An add may wait for 50, but this runs on a key press, and
/// working out a Library track's row reads its tags; a longer list is the
/// index job's, whose end reloads the lists (`useSetEnergy`).
const SETTLE_NOW_LIMIT: usize = 10;

/// Gives tracks an energy from 1 to 10: one operation, one Undo. The
/// Overview and its doors read the energy from the track table's rows, so a
/// few tracks' rows (at most [`SETTLE_NOW_LIMIT`] due) are worked out now,
/// for the lists read right after it.
#[tauri::command(async)]
#[specta::specta]
pub fn set_energy(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    recordings: Vec<i64>,
    energy: i64,
) -> Result<EnergySet, IpcError> {
    let set = writer.call(move |conn| Ok(set_energy_on(conn, &recordings, energy)))??;
    if set.changed > 0 {
        crate::library::settle_now_up_to(&writer, &jobs, SETTLE_NOW_LIMIT);
    }
    Ok(set)
}

/// Takes the user's own energy away from tracks: one operation, one Undo.
/// Another source's energy then shows (the ranking in `recording_display`).
/// The track table's rows are worked out again as for [`set_energy`].
#[tauri::command(async)]
#[specta::specta]
pub fn clear_energy(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    recordings: Vec<i64>,
) -> Result<EnergySet, IpcError> {
    let cleared = writer.call(move |conn| Ok(clear_energy_on(conn, &recordings)))??;
    if cleared.changed > 0 {
        crate::library::settle_now_up_to(&writer, &jobs, SETTLE_NOW_LIMIT);
    }
    Ok(cleared)
}

/// How many of these tracks have an energy of the user's own (what "Clear"
/// would take away).
#[tauri::command]
#[specta::specta]
pub async fn user_energy_count(
    reads: State<'_, ReadPool>,
    recordings: Vec<i64>,
) -> Result<u32, IpcError> {
    Ok(reads.read(move |conn| user_energy_among(conn, &recordings))?)
}

#[cfg(test)]
mod tests;
