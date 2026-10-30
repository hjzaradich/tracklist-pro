//! Energy as rekordbox's star rating (ROADMAP §7, "How energy reaches
//! rekordbox"; owner decisions 2026-10-28 and 2026-10-30; 1cC-2).
//!
//! By default a send never writes energy. An opt-in setting sends it as
//! each Library track's star rating instead, so a DJ can sort by stars in
//! rekordbox. Everything here sits below the screen: the setting, the
//! mapping, the count that lets the screen ask before turning it on, and the
//! few places the conflict engine asks about it. The setting is off until the
//! user turns it on, and while it's off none of this runs.
//!
//! # The mapping
//!
//! The app's energy stays 1 to 10; rekordbox gets 1 to 5 stars: 1–2 is one
//! star, 3–4 two, 5–6 three, 7–8 four and 9–10 five ([`stars_for`]). The
//! energy is the one the Overview shows ([`crate::overview::shown_energy`]):
//! the user's, then Mixed In Key's, the local estimate's, the audio model's.
//! A track with no energy has no rating of the app's own, so a send keeps
//! rekordbox's stars, as it does for every field the app has no value for.
//!
//! # The app's side while it's on
//!
//! The star rating is derived, never stored: [`derive`] puts a `Rating` of
//! each track's energy into the app's values the merge before a send reads
//! ([`crate::conflicts::app_values::read_for_send`]), in place of an own
//! `Rating` of the track's. So the send carries it like any value the app
//! means to change: the merge decides by the three sides, and
//! `gather_merged` writes it, for a track rekordbox has and for a new one.
//! Because it is derived, an energy change (key press, Undo, a new Mixed In
//! Key reading) moves the stars with no write of its own.
//!
//! - **Never sent, or sent before.** The derived value replaces whatever
//!   rekordbox showed when the merge ran. For a field with a base the base
//!   says what the send before replaced, as for any field; for one with
//!   none, `replaced` is rekordbox's current stars, so the first send after
//!   turning the setting on simply writes them. Turning it on is confirmed
//!   by the user first, with [`would_change`] saying how many tracks that is.
//! - **A star change made in rekordbox is a conflict** (row 8a of the
//!   decision table, [`crate::conflicts::decide_with`]): the energy hasn't
//!   moved, but rekordbox's stars did. Only while the setting is on, and
//!   only for a track with an energy.
//! - **Settling it** ([`adopt`], in the resolve's own operation, so one
//!   Undo takes it all back): keeping the app's sends the energy's stars
//!   again and changes nothing else. Keeping rekordbox's sets the user's
//!   energy to twice the stars (4 stars, energy 8); with 0 stars (unrated)
//!   it takes the user's own energy away instead (owner decision,
//!   2026-10-30), and the track shows whatever the next source gives, or
//!   none. A value given by hand is kept as rekordbox's is. No own `Rating`
//!   is stored while the setting is on, so none comes back when it's turned
//!   off.
//!
//! # Counting
//!
//! Two counts, both of tracks rekordbox has whose stars would change. Before
//! the setting is on, [`would_change`] counts from rekordbox's values as of
//! the newest read (only a send's prepare step reads the export afresh) and
//! says when that read was made, or that there was none. In a send, with the
//! setting on, [`stars_changed_by`] counts exactly, from the send's own fresh
//! read and merge; the preflight carries it (`Preflight::stars_changed`).
//!
//! # Turning it off
//!
//! Leaves the stars as last sent; the app holds no `Rating` of its own any
//! more, so the rating follows rekordbox again, as it does by default. A
//! conflict still open on the stars is withdrawn by the next merge, which
//! finds the sides no longer disagree.
//!
//! # Where it is stored
//!
//! In the `setting` table (migration 0005), as a JSON boolean under
//! [`ENERGY_AS_STARS`], like `read_online_only_files`: it is the user's
//! choice, not Library data, so it isn't in the operation log and Undo
//! doesn't touch it, and "Start fresh" (which starts an empty Library and
//! resets no setting) keeps it. Off when unset, or when the stored value
//! isn't a boolean.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::conflicts::app_values;
use crate::conflicts::resolve::Resolved;
use crate::conflicts::{
    load_sides, same, stars_of, AppValue, AppValues, Merged, RekordboxEntry, Status,
};
use crate::db::{ReadPool, Writer};
use crate::energy;
use crate::ipc::IpcError;
use crate::library::LibraryTrackId;
use crate::ops::{OpsError, Recorder};
use crate::overview::shown_energy;
use crate::paths::Volumes;
use crate::scan::system_volumes;
use crate::send_values::{send_values, Outcome, SendValues};

/// The `setting` key of the opt-in.
pub const ENERGY_AS_STARS: &str = "energy_as_stars";

/// The rekordbox attribute the stars live in.
pub const RATING: &str = "Rating";

/// How many Library tracks [`would_change`] reads at a time.
const TRACKS_AT_A_TIME: usize = crate::conflicts::store::TRACKS_AT_A_TIME;

// ---- The mapping ----------------------------------------------------------

/// The stars an energy is sent as: 1–2 is 1, 3–4 is 2, 5–6 is 3, 7–8 is 4
/// and 9–10 is 5. `None` for a number outside 1 to 10.
pub fn stars_for(energy: i64) -> Option<u8> {
    match energy {
        1..=10 => u8::try_from((energy + 1) / 2).ok(),
        _ => None,
    }
}

/// The `Rating` value rekordbox stores for `stars` (0 to 5): 51 a star.
pub fn rating_of_stars(stars: u8) -> String {
    (u32::from(stars) * 51).to_string()
}

/// The `Rating` an energy is sent as: [`stars_for`] in rekordbox's own
/// number.
pub fn rating_for(energy: i64) -> Option<String> {
    stars_for(energy).map(rating_of_stars)
}

/// The energy that keeping rekordbox's stars gives the user: twice the
/// stars (4 stars, energy 8). `None` for 0 stars, which clears the user's
/// energy instead, and for a number that isn't 1 to 5 stars.
pub fn energy_for_stars(stars: u8) -> Option<i64> {
    (1..=5).contains(&stars).then(|| i64::from(stars) * 2)
}

// ---- The setting ----------------------------------------------------------

/// Whether the user has energy sent as the star rating. Off when unset, or
/// when the stored value isn't a JSON boolean.
pub fn is_on(conn: &Connection) -> rusqlite::Result<bool> {
    let on: Option<Option<bool>> = conn
        .query_row(
            "SELECT CASE json_type(value) WHEN 'true' THEN 1 WHEN 'false' THEN 0 END
             FROM setting WHERE key = ?1",
            [ENERGY_AS_STARS],
            |r| r.get(0),
        )
        .optional()?;
    Ok(on.flatten().unwrap_or(false))
}

/// Stores the setting.
pub fn write(conn: &Connection, on: bool) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, json(?2))
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (ENERGY_AS_STARS, if on { "true" } else { "false" }),
    )?;
    Ok(())
}

/// [`is_on`], inside an operation.
fn is_on_in(rec: &Recorder<'_>) -> Result<bool, OpsError> {
    let on = rec.read_rows(
        "SELECT json_type(value) = 'true' FROM setting WHERE key = ?1",
        [ENERGY_AS_STARS],
        |r| r.get::<_, Option<bool>>(0),
    )?;
    Ok(on.into_iter().next().flatten().unwrap_or(false))
}

// ---- Energy ---------------------------------------------------------------

/// The energy each of these Library tracks shows (the same one as the
/// Overview: [`shown_energy!`]), by Library track id; a track with none is
/// not in the map.
fn shown_energies(
    conn: &Connection,
    ids: &[LibraryTrackId],
) -> rusqlite::Result<HashMap<i64, i64>> {
    let ids: Vec<i64> = ids.iter().map(|id| id.0).collect();
    let ids = serde_json::to_string(&ids).expect("numbers always serialize");
    let mut stmt = conn.prepare(concat!(
        "SELECT lt.id, ",
        shown_energy!("lt.recording_id"),
        " FROM library_track lt
          WHERE lt.id IN (SELECT value FROM json_each(?1))"
    ))?;
    let rows = stmt.query_map([ids], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
    })?;
    let mut energies = HashMap::new();
    for row in rows {
        if let (track, Some(energy)) = row? {
            energies.insert(track, energy);
        }
    }
    Ok(energies)
}

/// Puts a `Rating` derived from the track's energy into `app`, for each of
/// the tracks `sent` that shows an energy, in place of an own `Rating`.
/// Nothing at all while the setting is off.
///
/// `replaced` is what the derived value replaces in rekordbox: its entry's
/// current `Rating` (empty if the entry has none), or not known for a track
/// rekordbox doesn't have. It counts only for a field never sent (the engine
/// takes a sent field's from the send's record), where it makes the first
/// send after turning the setting on write the stars, as the user confirmed.
pub fn derive(conn: &Connection, sent: &[SendValues], app: &mut AppValues) -> rusqlite::Result<()> {
    if !is_on(conn)? {
        return Ok(());
    }
    let ids: Vec<LibraryTrackId> = sent.iter().map(|s| s.library_track).collect();
    let energies = shown_energies(conn, &ids)?;
    for one in sent {
        let Some(rating) = energies
            .get(&one.library_track.0)
            .and_then(|energy| rating_for(*energy))
        else {
            continue;
        };
        let replaced = match &one.outcome {
            Outcome::Ready(values) if values.in_rekordbox => Some(
                values
                    .get(RATING)
                    .map_or_else(String::new, |v| v.value.clone()),
            ),
            _ => None,
        };
        app.set(
            one.library_track,
            AppValue {
                field: RATING.to_owned(),
                value: rating,
                replaced,
            },
        );
    }
    Ok(())
}

/// The Library tracks among `ids` whose `Rating` the app derives from their
/// energy right now: those that show an energy, while the setting is on.
pub fn derived_tracks(conn: &Connection, ids: &[LibraryTrackId]) -> rusqlite::Result<HashSet<i64>> {
    if !is_on(conn)? {
        return Ok(HashSet::new());
    }
    Ok(shown_energies(conn, ids)?.into_keys().collect())
}

/// Settles a conflict's effect on what the app holds, in the resolve's own
/// operation ([`crate::conflicts::resolve`] runs it). For a `Rating` of a
/// track whose stars follow its energy, see the module docs; for everything
/// else it is [`app_values::adopt`].
pub fn adopt(rec: &mut Recorder<'_>, resolved: &Resolved) -> Result<(), OpsError> {
    if resolved.field != RATING || !is_on_in(rec)? {
        return app_values::adopt(rec, resolved);
    }
    let shown = rec.read_rows(
        concat!(
            "SELECT lt.recording_id, ",
            shown_energy!("lt.recording_id"),
            " FROM library_track lt WHERE lt.id = ?1"
        ),
        [resolved.library_track.0],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
    )?;
    let Some((recording, Some(_))) = shown.into_iter().next() else {
        // A track with no energy keeps the stars it has: nothing derived.
        return app_values::adopt(rec, resolved);
    };
    // No own `Rating` is stored while the stars follow energy.
    app_values::forget(rec, resolved.library_track, RATING)?;
    match resolved.status {
        Status::KeptApp | Status::Open => {}
        Status::KeptRekordbox | Status::Edited => match stars_of(&resolved.value) {
            Some(0) => {
                energy::clear_in(rec, recording)?;
            }
            Some(stars) => {
                if let Some(energy) = energy_for_stars(stars) {
                    energy::set_in(rec, &[recording], energy)?;
                }
            }
            None => {}
        },
    }
    Ok(())
}

// ---- Turning it on --------------------------------------------------------

/// The answer to "how many tracks' stars would change", which the screen
/// shows before the user turns the setting on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "result", rename_all = "camelCase")]
pub enum WouldChange {
    /// rekordbox's export was never read, so there's nothing to compare
    /// with. Not the same as no track changing.
    NoRead,
    /// The count, from rekordbox's values as of the newest read. Only a
    /// send's prepare step reads the export afresh, so the screen says how
    /// old this read is.
    Counted {
        tracks: u32,
        /// When that read was made, as the database writes times.
        read_at: String,
    },
}

/// How many Library tracks' stars in rekordbox a send would change if the
/// setting were on: those rekordbox has (a trusted entry in its newest
/// read) that show an energy whose stars differ from rekordbox's current
/// stars, with the time of that read ([`WouldChange::NoRead`] before any
/// read). A new track has none to change and a track with no energy keeps
/// its stars, so neither counts. The count doesn't depend on the setting,
/// so the screen can ask before it's turned on.
pub fn would_change(conn: &Connection, volumes: &impl Volumes) -> rusqlite::Result<WouldChange> {
    let read_at: Option<String> =
        conn.query_row("SELECT max(read_at) FROM rekordbox_track", [], |r| r.get(0))?;
    let Some(read_at) = read_at else {
        return Ok(WouldChange::NoRead);
    };
    let ids: Vec<LibraryTrackId> = conn
        .prepare("SELECT id FROM library_track ORDER BY id")?
        .query_map([], |r| Ok(LibraryTrackId(r.get(0)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut changing = 0u32;
    for some in ids.chunks(TRACKS_AT_A_TIME) {
        let sent = send_values(conn, volumes, some)?;
        let energies = shown_energies(conn, some)?;
        for track in load_sides(conn, &sent, &AppValues::default())? {
            let Some(rating) = energies
                .get(&track.library_track.0)
                .and_then(|energy| rating_for(*energy))
            else {
                continue;
            };
            if track.entry != RekordboxEntry::Current {
                continue;
            }
            let there = track
                .fields
                .iter()
                .find(|f| f.field == RATING)
                .and_then(|f| f.rekordbox.as_deref());
            if !same(RATING, there, Some(&rating)) {
                changing += 1;
            }
        }
    }
    Ok(WouldChange::Counted {
        tracks: changing,
        read_at,
    })
}

/// How many tracks' stars in rekordbox the send `merged` changes: tracks
/// rekordbox has for which the merge chose a `Rating` that differs (by
/// stars) from rekordbox's own. `None` while the setting is off. Exact: it
/// is counted from the send's own fresh read and merge, so the send's
/// preflight shows what the Yes in rekordbox will do.
pub fn stars_changed_by(conn: &Connection, merged: &Merged) -> rusqlite::Result<Option<u32>> {
    if !is_on(conn)? {
        return Ok(None);
    }
    let mut changing = 0u32;
    for one in &merged.values {
        let Outcome::Ready(values) = &one.outcome else {
            continue;
        };
        let chosen = merged
            .chosen
            .get(&one.library_track)
            .and_then(|fields| fields.iter().find(|(field, _)| field == RATING));
        let (true, Some((_, chosen))) = (values.in_rekordbox, chosen) else {
            continue;
        };
        let there = values.get(RATING).map(|v| v.value.as_str());
        if !same(RATING, there, Some(chosen)) {
            changing += 1;
        }
    }
    Ok(Some(changing))
}

// ---- Commands -------------------------------------------------------------

/// Whether energy is sent as rekordbox's star rating. Off until the user
/// turns it on.
#[tauri::command]
#[specta::specta]
pub async fn read_energy_as_stars(reads: State<'_, ReadPool>) -> Result<bool, IpcError> {
    Ok(reads.read(is_on)?)
}

/// How many Library tracks' stars in rekordbox turning energy-as-stars on
/// would change (see [`would_change`]) and when the read it counts from was
/// made, or that there's been no read yet, for the screen to ask the user to
/// confirm before it calls [`set_energy_as_stars`].
#[tauri::command]
#[specta::specta]
pub async fn energy_as_stars_would_change(
    reads: State<'_, ReadPool>,
) -> Result<WouldChange, IpcError> {
    let volumes = system_volumes();
    Ok(reads.read(|conn| would_change(conn, &volumes))?)
}

/// Turns sending energy as rekordbox's star rating on or off. Turning it
/// off leaves the stars as last sent, and the rating follows rekordbox
/// again. Not an operation: it is the user's choice, not Library data.
#[tauri::command]
#[specta::specta]
pub async fn set_energy_as_stars(writer: State<'_, Writer>, on: bool) -> Result<(), IpcError> {
    Ok(writer.call(move |c| write(c, on))?)
}

#[cfg(test)]
mod tests;
