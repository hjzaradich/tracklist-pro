//! A crate's notes and its summary line (1cB-15; ROADMAP 1.14, "Notes and
//! summary"): an optional one-line note, and what the crate's tracks add up
//! to: BPM range, energy range, keys and top genres.
//!
//! - **Notes** are `crate.notes` (migration 0005). Saving one is a recorded
//!   operation ([`NOTES_OPERATION`], naming the crate), so Undo puts the old
//!   note back. The text is one line: line breaks and tabs become spaces,
//!   the ends are trimmed, and an empty note is no note (the column goes
//!   back to `NULL`). Saving the note a crate has records nothing.
//! - **The summary is worked out from the rows the track table sorts by**
//!   (`track_row`: the same BPM, key and genre the table shows, kept by the
//!   index job), plus each track's energy, which the table doesn't hold, by
//!   the rule the Overview's energy chart uses (the user's, then Mixed In
//!   Key's, then the local estimate's, then the audio model's). It is three
//!   scans of the crate's own entries, each track found by its primary key,
//!   and so it grows with the crate and never with the Library: no track's
//!   tags are read. Only the spelling of the (at most three) top genres is
//!   read from a track, the first one in each genre.
//! - **A part with no data is `None` or empty**, so the screen leaves it out.
//!   A track the index hasn't reached yet adds to the track count and to
//!   nothing else; the index job ending asks the screen again.
//! - **Genres** are grouped the way the Genre column sorts them (capitals and
//!   accents don't matter), biggest first, ties by name. **Keys** are every
//!   different key in Camelot order (1A, 1B, 2A ... 12B), as Camelot, for
//!   the screen to write in the user's notation.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use tauri::State;

use super::fit::Camelot;
use super::{static_name, CrateId, CratesError, Done};
use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::library::LibraryTrackId;
use crate::ops::{self, OpsError};
use crate::paths::Volumes;
use crate::scan::system_volumes;

#[cfg(test)]
mod tests;

/// The operation kind a saved note is recorded as.
pub const NOTES_OPERATION: &str = "set_crate_notes";

/// The most characters a note holds.
pub const MAX_NOTES_CHARS: usize = 300;

/// The most genres the summary names.
pub const TOP_GENRES: usize = 3;

/// The lowest and highest BPM in a crate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BpmRange {
    pub min: f64,
    pub max: f64,
}

/// The lowest and highest energy (1 to 10) in a crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnergyRange {
    pub min: u8,
    pub max: u8,
}

/// What the crate view shows under a crate's name.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CrateSummary {
    /// The crate's note, if it has one.
    pub notes: Option<String>,
    /// How many tracks are in the crate.
    pub tracks: u32,
    /// `None` when no track has a BPM.
    pub bpm: Option<BpmRange>,
    /// `None` when no track has an energy.
    pub energy: Option<EnergyRange>,
    /// Every different key, as Camelot, in Camelot order; empty when no
    /// track has a key.
    pub keys: Vec<String>,
    /// The biggest genres, biggest first (at most [`TOP_GENRES`]); empty when
    /// no track has a genre.
    pub genres: Vec<String>,
}

/// What the summary is worked from: a crate's entries, each with its row in
/// `track_row` (if the index has one) and its energy.
const FROM: &str = "FROM crate_entry ce
     JOIN library_track lt ON lt.id = ce.library_track_id
     LEFT JOIN track_row tr ON tr.recording_id = lt.recording_id
     WHERE ce.crate_id = ?1 AND ce.kind = 'member'";

/// A track's energy: the `analysis` row that ranks first among those with an
/// energy, by the Overview's order (`overview::ENERGY`; a test holds the two
/// to the same answers).
pub(crate) const ENERGY_OF_LT: &str = "(SELECT a.energy FROM analysis a
         WHERE a.recording_id = lt.recording_id AND a.energy IS NOT NULL
         ORDER BY CASE a.source
             WHEN 'user' THEN 1 WHEN 'mik' THEN 2 WHEN 'local' THEN 3 WHEN 'ml' THEN 4
         END
         LIMIT 1)";

/// The note as it will be stored: one line, trimmed. `None` is no note.
fn tidy(notes: &str) -> Option<String> {
    let one_line: String = notes
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = one_line.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Gives a crate a note, or none (an empty `notes`). Answers the operation
/// recorded, or `None` when the crate has that note already (nothing is
/// recorded). Recorded as one operation naming the crate, so one Undo puts
/// the earlier note back. A folder takes no note.
pub fn set_notes_on(
    conn: &mut Connection,
    id: CrateId,
    notes: &str,
) -> Result<Done<Option<i64>>, OpsError> {
    let stored = tidy(notes);
    if stored
        .as_ref()
        .is_some_and(|text| text.chars().count() > MAX_NOTES_CHARS)
    {
        return Ok(Err(CratesError::NotesTooLong));
    }
    let recorded = ops::record(
        conn,
        NOTES_OPERATION,
        &serde_json::json!({ "crateId": id.0 }),
        |rec| {
            let Some(name) = static_name(rec, id)? else {
                return Ok(Err(CratesError::NotFound));
            };
            rec.detail("name", name);
            rec.set("crate", id.0, "notes", stored)?;
            Ok(Ok(()))
        },
    )?;
    Ok(recorded.value.map(|()| recorded.operation_id))
}

/// The crate's note, or `None` for none. `Err` of nothing when `id` is not
/// a crate.
fn notes_of(conn: &Connection, id: CrateId) -> rusqlite::Result<Option<Option<String>>> {
    conn.query_row(
        "SELECT notes FROM crate WHERE id = ?1 AND kind = 'static'",
        [id.0],
        |r| r.get::<_, Option<String>>(0),
    )
    .optional()
}

/// The summary of crate `id`, or `None` if there is no such crate (a folder
/// has none).
pub fn summarize(
    conn: &Connection,
    volumes: &impl Volumes,
    id: CrateId,
) -> rusqlite::Result<Option<CrateSummary>> {
    let Some(notes) = notes_of(conn, id)? else {
        return Ok(None);
    };
    let (tracks, bpm, energy) = conn.query_row(
        &format!(
            "SELECT count(*), min(s.bpm), max(s.bpm), min(s.energy), max(s.energy)
             FROM (SELECT tr.bpm AS bpm, {ENERGY_OF_LT} AS energy
                   {FROM}) s"
        ),
        [id.0],
        |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, Option<f64>>(1)?.zip(r.get::<_, Option<f64>>(2)?),
                r.get::<_, Option<i64>>(3)?.zip(r.get::<_, Option<i64>>(4)?),
            ))
        },
    )?;

    let mut stmt = conn.prepare_cached(&format!(
        "SELECT DISTINCT tr.key_order {FROM} AND tr.key_order IS NOT NULL ORDER BY tr.key_order"
    ))?;
    let mut keys = Vec::new();
    for order in stmt.query_map([id.0], |r| r.get::<_, i64>(0))? {
        keys.extend(Camelot::from_order(order?).map(Camelot::name));
    }

    // The biggest genres, and the first track of each, whose spelling is
    // shown.
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT tr.genre_key, count(*), min(lt.recording_id)
         {FROM} AND tr.genre_key IS NOT NULL
         GROUP BY tr.genre_key ORDER BY count(*) DESC, tr.genre_key LIMIT {TOP_GENRES}"
    ))?;
    let top: Vec<(String, i64)> = stmt
        .query_map([id.0], |r| Ok((r.get(0)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let genres = spelled(conn, volumes, &top)?;

    Ok(Some(CrateSummary {
        notes,
        tracks,
        bpm: bpm.map(|(min, max)| BpmRange { min, max }),
        energy: energy.and_then(|(min, max)| {
            Some(EnergyRange {
                min: u8::try_from(min).ok()?,
                max: u8::try_from(max).ok()?,
            })
        }),
        keys,
        genres,
    }))
}

/// The genres of `top` (a genre's sort key and the first track that has it)
/// as that track spells them, in the same order.
fn spelled(
    conn: &Connection,
    volumes: &impl Volumes,
    top: &[(String, i64)],
) -> rusqlite::Result<Vec<String>> {
    if top.is_empty() {
        return Ok(Vec::new());
    }
    let recordings: Vec<i64> = top.iter().map(|(_, recording)| *recording).collect();
    let mut library: HashMap<i64, LibraryTrackId> = HashMap::new();
    for &recording in &recordings {
        let track: Option<i64> = conn
            .query_row(
                "SELECT id FROM library_track WHERE recording_id = ?1",
                [recording],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(track) = track {
            library.insert(recording, LibraryTrackId(track));
        }
    }
    let mut fields = crate::search::fields_of(conn, volumes, &recordings, &library)?;
    Ok(top
        .iter()
        .map(|(key, recording)| {
            fields
                .remove(recording)
                .map(|shown| shown.genre.trim().to_owned())
                .filter(|genre| !genre.is_empty())
                // The index holds a genre the track no longer shows: say the
                // index's own spelling rather than nothing.
                .unwrap_or_else(|| key.clone())
        })
        .collect())
}

// `async` runs the write off the main thread, like the other crate commands.

/// A crate's notes and summary line.
#[tauri::command]
#[specta::specta]
pub async fn crate_summary(
    reads: State<'_, ReadPool>,
    id: CrateId,
) -> Result<CrateSummary, IpcError> {
    let volumes = system_volumes();
    reads
        .read(|conn| summarize(conn, &volumes, id))?
        .ok_or_else(|| CratesError::NotFound.into())
}

/// Gives a crate a note; an empty note clears it. Answers the operation
/// recorded, or none when the note is the one it has.
#[tauri::command]
#[specta::specta]
pub async fn set_crate_notes(
    writer: State<'_, Writer>,
    id: CrateId,
    notes: String,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(set_notes_on(conn, id, &notes)))???)
}
