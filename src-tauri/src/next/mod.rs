//! Harmonic "what's next" (ROADMAP 3.4): for a chosen track, the Library
//! tracks that follow it best, each with its top reasons as data. The scoring
//! is in [`score`] (pure); this module reads the facts, the weights and the
//! played transitions, and hands the best on.
//!
//! - **Signals:** Camelot compatibility (same, one number up or down, relative
//!   major or minor), BPM closeness (half and double time count, as in Sort
//!   by fit, 1.14, whose tempo reading and thresholds this reuses), the
//!   energy step in the direction the user chose, and the transitions the
//!   user played (3.6, [`crate::play_history::transitions`]). Genre and tags
//!   are a hook only for now ([`score::ExtraSignal`]).
//! - **Weights and tolerance** are settings (`next_weights`,
//!   `next_tolerance` in the `setting` table), visible and adjustable. The
//!   defaults are genre-neutral. Changing them is not an operation, like
//!   every setting. A stored value that cannot be read is the default.
//!   *Tight* uses Sort by fit's thresholds (a tempo within 2, 4 or 8 BPM,
//!   only neighbor keys); *wide* also takes bigger tempo gaps, keys two
//!   steps or diagonally away, and bigger energy steps.
//! - **Cost:** one query reads every Library track's facts, which are scored
//!   in memory; only the shown few have their reasons worked out and their
//!   rows read.
//! - **Bad data** in one track (a BPM that is not a number or not positive,
//!   a key or energy out of range) reads as missing for that track alone.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::crates::fit::Camelot;
use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::scan::system_volumes;
use crate::search::flag;
use crate::track_table::{self, TableRow};

pub mod score;
#[cfg(test)]
mod tests;

use score::{Ask, Candidate, ExtraSignal, Facts, Reason};

/// How many results the Details panel shows.
pub const SHOWN: usize = 10;

/// The most a weight can be.
pub const MOST_WEIGHT: u8 = 10;

/// `setting` keys.
const WEIGHTS_KEY: &str = "next_weights";
const TOLERANCE_KEY: &str = "next_tolerance";

/// Which way the energy should go from the chosen track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Up,
    Same,
    Down,
}

/// How far from the chosen track a fit may stray.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Tolerance {
    /// Sort by fit's thresholds: close blending.
    #[default]
    Tight,
    /// Bigger tempo, key and energy jumps still count.
    Wide,
}

/// The signals that carry a weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Signal {
    Key,
    Bpm,
    Energy,
    Played,
}

/// How much each signal counts, 0 (off) to [`MOST_WEIGHT`].
///
/// TODO(3A-G, 3A-T): `genre` and `tags` join these when those features merge
/// (see [`score::ExtraSignal`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[serde(rename = "NextWeights")]
pub struct Weights {
    pub key: u8,
    pub bpm: u8,
    pub energy: u8,
    pub played: u8,
}

/// Genre-neutral defaults (proposed): key and tempo count most, then the
/// energy step and what the user has played.
impl Default for Weights {
    fn default() -> Weights {
        Weights {
            key: 4,
            bpm: 4,
            energy: 2,
            played: 2,
        }
    }
}

/// The weights and the tolerance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[serde(rename = "NextSettings")]
pub struct Settings {
    pub weights: Weights,
    pub tolerance: Tolerance,
}

impl Weights {
    fn clamped(self) -> Weights {
        let c = |w: u8| w.min(MOST_WEIGHT);
        Weights {
            key: c(self.key),
            bpm: c(self.bpm),
            energy: c(self.energy),
            played: c(self.played),
        }
    }
}

/// The stored settings; anything missing or unreadable is the default, one
/// value at a time.
pub fn read_settings(conn: &Connection) -> rusqlite::Result<Settings> {
    let default = Settings::default();
    let weight = |field: &str, fallback: u8| -> rusqlite::Result<u8> {
        let stored: Option<Option<i64>> = conn
            .query_row(
                "SELECT CASE WHEN json_type(value, ?2) IN ('integer', 'real')
                        THEN CAST(value ->> ?2 AS INTEGER) END
                 FROM setting WHERE key = ?1",
                (WEIGHTS_KEY, format!("$.{field}")),
                |r| r.get(0),
            )
            .optional()?;
        Ok(stored
            .flatten()
            .map_or(fallback, |w| w.clamp(0, i64::from(MOST_WEIGHT)) as u8))
    };
    let name: Option<Option<String>> = conn
        .query_row(
            "SELECT CASE WHEN json_type(value) = 'text' THEN value ->> '$' END
             FROM setting WHERE key = ?1",
            [TOLERANCE_KEY],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Settings {
        weights: Weights {
            key: weight("key", default.weights.key)?,
            bpm: weight("bpm", default.weights.bpm)?,
            energy: weight("energy", default.weights.energy)?,
            played: weight("played", default.weights.played)?,
        },
        tolerance: match name.flatten().as_deref() {
            Some("wide") => Tolerance::Wide,
            Some("tight") => Tolerance::Tight,
            _ => default.tolerance,
        },
    })
}

/// Stores the settings (weights above [`MOST_WEIGHT`] are cut to it) and
/// returns what is stored.
pub fn write_settings(conn: &Connection, settings: Settings) -> rusqlite::Result<Settings> {
    let settings = Settings {
        weights: settings.weights.clamped(),
        ..settings
    };
    let weights = serde_json::to_string(&settings.weights).expect("numbers serialize");
    let tolerance = match settings.tolerance {
        Tolerance::Tight => "tight",
        Tolerance::Wide => "wide",
    };
    // The weights are a JSON object; the tolerance a JSON string.
    for (key, value, quote) in [
        (WEIGHTS_KEY, weights.as_str(), false),
        (TOLERANCE_KEY, tolerance, true),
    ] {
        conn.execute(
            if quote {
                "INSERT INTO setting (key, value) VALUES (?1, json_quote(?2))
                 ON CONFLICT (key) DO UPDATE SET
                     value = excluded.value,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
            } else {
                "INSERT INTO setting (key, value) VALUES (?1, json(?2))
                 ON CONFLICT (key) DO UPDATE SET
                     value = excluded.value,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
            },
            (key, value),
        )?;
    }
    Ok(settings)
}

/// A value that is a usable BPM, else none.
fn clean_bpm(bpm: Option<f64>) -> Option<f64> {
    bpm.filter(|b| b.is_finite() && *b > 0.0 && *b < 1000.0)
}

fn facts_of(bpm: Option<f64>, key_order: Option<i64>, flags: i64) -> Facts {
    let energy = flag::energy_of(flags);
    Facts {
        bpm: clean_bpm(bpm),
        key: key_order.and_then(Camelot::from_order),
        energy: (1..=10).contains(&energy).then_some(energy as u8),
    }
}

/// The chosen track's facts; none for every one if the index has no row.
fn target_facts(conn: &Connection, id: i64) -> rusqlite::Result<Facts> {
    let found = conn
        .query_row(
            "SELECT bpm, key_order, flags FROM track_row WHERE recording_id = ?1",
            [id],
            |r| {
                Ok(facts_of(
                    r.get(0).ok().flatten(),
                    r.get(1).ok().flatten(),
                    r.get(2).unwrap_or(0),
                ))
            },
        )
        .optional()?;
    Ok(found.unwrap_or_default())
}

/// Every Library track but the chosen one, with its facts: the one query.
/// A row that cannot be read as a track is skipped, and a fact that cannot be
/// read is missing; neither stops the rest.
pub fn candidates(conn: &Connection, except: i64) -> rusqlite::Result<Vec<Candidate>> {
    let mut stmt = conn.prepare_cached(
        "SELECT recording_id, bpm, key_order, flags FROM track_row
         WHERE flags & ?2 <> 0 AND recording_id <> ?1",
    )?;
    let rows = stmt.query_map((except, flag::IN_LIBRARY), |r| {
        let Ok(id) = r.get::<_, i64>(0) else {
            return Ok(None);
        };
        Ok(Some(Candidate {
            id,
            facts: facts_of(
                r.get(1).ok().flatten(),
                r.get(2).ok().flatten(),
                r.get(3).unwrap_or(0),
            ),
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.extend(row?);
    }
    Ok(out)
}

/// How often each track was played right after `target`.
fn played_after(conn: &Connection, target: i64) -> rusqlite::Result<HashMap<i64, u32>> {
    Ok(crate::play_history::transitions(conn, Some(target))?
        .into_iter()
        .map(|t| (t.to, t.count))
        .collect())
}

/// One suggestion.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NextResult {
    /// The track, as the track table shows it.
    pub row: TableRow,
    /// 0 to 100.
    pub score: u8,
    /// The top reasons, strongest first, for the screen to word.
    pub reasons: Vec<Reason>,
}

/// The best next tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NextReply {
    pub results: Vec<NextResult>,
    /// How many Library tracks were weighed.
    pub considered: u32,
    /// Signals that count but could not be used, because the chosen track
    /// has nothing to compare.
    pub ignored: Vec<Signal>,
    pub settings: Settings,
}

/// What ranking found, before the rows are read.
pub struct Found {
    pub ranked: Vec<score::Scored>,
    pub considered: u32,
    pub ignored: Vec<Signal>,
    pub settings: Settings,
}

/// Ranks the Library for the track `target`.
pub fn rank_for(
    conn: &Connection,
    target: i64,
    direction: Direction,
    extras: &[&dyn ExtraSignal],
) -> rusqlite::Result<Found> {
    let settings = read_settings(conn)?;
    let candidates = candidates(conn, target)?;
    let played = played_after(conn, target)?;
    let ask = Ask {
        target_id: target,
        target: target_facts(conn, target)?,
        direction,
        settings: &settings,
        played_after: &played,
        extras,
    };
    let ranking = score::rank(&ask, &candidates, SHOWN);
    Ok(Found {
        ranked: ranking.results,
        considered: u32::try_from(candidates.len()).unwrap_or(u32::MAX),
        ignored: ranking.ignored,
        settings,
    })
}

/// The reply: `found` with each result's row.
pub fn reply(
    conn: &Connection,
    volumes: &impl crate::paths::Volumes,
    found: Found,
) -> rusqlite::Result<NextReply> {
    let ids: Vec<i64> = found.ranked.iter().map(|s| s.id).collect();
    let mut rows: HashMap<i64, TableRow> = track_table::rows_of(conn, volumes, &ids)?
        .into_iter()
        .map(|row| (row.recording_id, row))
        .collect();
    let results = found
        .ranked
        .into_iter()
        .filter_map(|s| {
            rows.remove(&s.id).map(|row| NextResult {
                row,
                score: s.score,
                reasons: s.reasons,
            })
        })
        .collect();
    Ok(NextReply {
        results,
        considered: found.considered,
        ignored: found.ignored,
        settings: found.settings,
    })
}

/// The tracks that best follow `recording_id` going `direction`, with why.
// Async, so the database call runs off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn next_for(
    reads: State<'_, ReadPool>,
    recording_id: i64,
    direction: Direction,
) -> Result<NextReply, IpcError> {
    let volumes = system_volumes();
    Ok(reads.read(move |c| {
        let found = rank_for(c, recording_id, direction, &[])?;
        reply(c, &volumes, found)
    })?)
}

/// The weights and the tolerance.
#[tauri::command]
#[specta::specta]
pub async fn next_settings(reads: State<'_, ReadPool>) -> Result<Settings, IpcError> {
    Ok(reads.read(read_settings)?)
}

/// Stores the weights and the tolerance; returns what is stored.
#[tauri::command]
#[specta::specta]
pub async fn next_set_settings(
    writer: State<'_, Writer>,
    settings: Settings,
) -> Result<Settings, IpcError> {
    Ok(writer.call(move |c| write_settings(c, settings))?)
}
