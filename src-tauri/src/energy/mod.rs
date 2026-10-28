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
//! nothing is sent to rekordbox: energy is not sent by default (ROADMAP §1.1,
//! "Energy"; 1cC-2 adds the opt-in).
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

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::db::Writer;
use crate::ipc::{ErrorKind, IpcError};
use crate::ops::{self, OpsError};

/// The operation setting energy records.
pub const SET_ENERGY_OPERATION: &str = "set_energy";

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
    let wanted = serde_json::to_string(recordings).expect("numbers always serialize");
    let recorded = ops::record(
        conn,
        SET_ENERGY_OPERATION,
        &serde_json::json!({}),
        |rec| {
            // The tracks that exist, with the user's row if they have one.
            let held: HashMap<i64, Held> = rec
                .read_rows(
                    "SELECT r.id, a.id, a.energy
                     FROM recording r
                     LEFT JOIN analysis a ON a.recording_id = r.id AND a.source = 'user'
                     WHERE r.id IN (SELECT value FROM json_each(?1))",
                    [wanted.as_str()],
                    |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))),
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
            rec.detail("tracks", changed);
            Ok(changed)
        },
    )?;
    let asked = recordings.iter().collect::<std::collections::HashSet<_>>().len();
    Ok(EnergySet {
        changed: recorded.value,
        skipped: (asked as u32).saturating_sub(recorded.value),
        operation_id: recorded.operation_id,
    })
}

/// Gives tracks an energy from 1 to 10: one operation, one Undo.
#[tauri::command(async)]
#[specta::specta]
pub fn set_energy(
    writer: State<'_, Writer>,
    recordings: Vec<i64>,
    energy: i64,
) -> Result<EnergySet, IpcError> {
    Ok(writer.call(move |conn| Ok(set_energy_on(conn, &recordings, energy)))??)
}

#[cfg(test)]
mod tests;
