//! A playlist's timeline and "fill the gap" (ROADMAP 3.5), backend.
//!
//! - **Timeline.** One call returns every entry in order with the shown
//!   duration, BPM, Camelot key and energy (the track table's own row, so
//!   the numbers are the ones the user sees; energy is only what the table
//!   shows from tags or the user, never an estimate), the time each entry
//!   starts, the playlist's target energy at that entry and the gap to it,
//!   and for each transition (entry i → i + 1) flags for key, BPM and energy.
//! - **Flags are "what's next"'s rules** ([`crate::next::bridge`] reuses its
//!   functions): a key clash is a pair "what's next" gives no credit for
//!   (Camelot relation), a BPM jump one beyond the tolerance with half and
//!   double time read as the closest, an energy jump a step no direction
//!   gives credit for. The tolerance is the user's tight or wide setting. A
//!   track with no value gives `unknown`, never a clash.
//! - **Start times** add up the durations before; a track with no duration
//!   counts as 0 and is counted in `unknownDurations`.
//! - **Target shape.** A small list of points (where in the playlist, 0 to 1,
//!   and the energy, 1 to 10) in the playlist's `target_energy` column,
//!   set or cleared as one Undo each. An entry sits at `i / (n - 1)` of the
//!   way (a lone entry at 0); between points the target is a straight line,
//!   before the first and after the last it stays flat. A shape that doesn't
//!   fit the rules is tidied, not refused: points outside 0 to 1 or 1 to 10
//!   are pulled in, a point with no number is dropped, two points at the same
//!   place keep the last, and at most [`MOST_TARGET_POINTS`] are kept.
//! - **Fill the gap** ranks Library tracks for the place after an entry
//!   ([`crate::next::bridge`]); inserting one is the ordinary add at a
//!   position. Read-only.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::{name_of, Done, Playlist, PlaylistEntryId, PlaylistId, PlaylistsError};
use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::next::bridge::{self, BridgeReason, Gap};
use crate::next::{self, Tolerance};
use crate::ops::{self, OpsError};
use crate::track_table::TableRow;

/// The operation kinds, as the operation log stores them.
pub const SET_TARGET_OPERATION: &str = "set_playlist_target";
pub const CLEAR_TARGET_OPERATION: &str = "clear_playlist_target";

/// The most points a target shape keeps.
pub const MOST_TARGET_POINTS: usize = 32;

/// The most suggestions "fill the gap" returns.
pub const MOST_SUGGESTIONS: u32 = 50;

/// One point of the target energy shape.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TargetPoint {
    /// How far into the playlist, 0 (the first track) to 1 (the last).
    pub at: f64,
    /// The energy wanted there, 1 to 10 (any number is taken and pulled
    /// into that range; one that isn't a number drops the point).
    pub energy: f64,
}

/// The shape as it will be stored: tidied (see the module doc) and in order.
pub fn tidy_target(points: &[TargetPoint]) -> Vec<TargetPoint> {
    let mut tidy: Vec<TargetPoint> = points
        .iter()
        .filter(|p| p.at.is_finite() && p.energy.is_finite())
        .map(|p| TargetPoint {
            at: p.at.clamp(0.0, 1.0),
            energy: p.energy.clamp(1.0, 10.0),
        })
        .collect();
    // Stable, so of two points at the same place the later one given is last.
    tidy.sort_by(|a, b| a.at.total_cmp(&b.at));
    let mut out: Vec<TargetPoint> = Vec::with_capacity(tidy.len());
    for p in tidy {
        match out.last_mut() {
            Some(last) if last.at == p.at => *last = p,
            _ => out.push(p),
        }
    }
    out.truncate(MOST_TARGET_POINTS);
    out
}

/// The target energy `at` of the way through (0 to 1), or `None` with no
/// shape. `points` must be in order, as [`tidy_target`] leaves them.
pub fn target_at(points: &[TargetPoint], at: f64) -> Option<f64> {
    let first = points.first()?;
    let last = points.last()?;
    if at <= first.at {
        return Some(first.energy);
    }
    if at >= last.at {
        return Some(last.energy);
    }
    let after = points.iter().position(|p| p.at >= at)?;
    let (p, q) = (points[after - 1], points[after]);
    let along = (at - p.at) / (q.at - p.at);
    Some(p.energy + (q.energy - p.energy) * along)
}

/// The playlist's stored shape; none if it has none or what is stored can't
/// be read as a shape.
pub fn read_target(conn: &Connection, id: PlaylistId) -> rusqlite::Result<Vec<TargetPoint>> {
    let stored: Option<Option<String>> = conn
        .query_row(
            "SELECT target_energy FROM playlist WHERE id = ?1",
            [id.0],
            |r| r.get(0),
        )
        .optional()?;
    Ok(stored
        .flatten()
        .and_then(|json| serde_json::from_str::<Vec<TargetPoint>>(&json).ok())
        .map_or_else(Vec::new, |points| tidy_target(&points)))
}

fn write_target(
    conn: &mut Connection,
    kind: &'static str,
    id: PlaylistId,
    points: &[TargetPoint],
) -> Result<Done<Option<i64>>, OpsError> {
    let json: Option<String> =
        (!points.is_empty()).then(|| serde_json::to_string(points).expect("numbers serialize"));
    ops::record(
        conn,
        kind,
        &serde_json::json!({ "playlistId": id.0 }),
        |rec| {
            let Some(name) = name_of(rec, id)? else {
                return Ok(Err(PlaylistsError::NotFound));
            };
            rec.set("playlist", id.0, "target_energy", json.clone())?;
            rec.detail("name", name);
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// Sets the playlist's target energy shape (tidied, see the module doc) as
/// one operation. Returns the operation, `None` when it's the shape already
/// stored. A shape with no usable point clears it.
pub fn set_target_on(
    conn: &mut Connection,
    id: PlaylistId,
    points: &[TargetPoint],
) -> Result<Done<Option<i64>>, OpsError> {
    let points = tidy_target(points);
    let kind = if points.is_empty() {
        CLEAR_TARGET_OPERATION
    } else {
        SET_TARGET_OPERATION
    };
    write_target(conn, kind, id, &points)
}

/// Clears the playlist's target energy shape, as one operation.
pub fn clear_target_on(
    conn: &mut Connection,
    id: PlaylistId,
) -> Result<Done<Option<i64>>, OpsError> {
    write_target(conn, CLEAR_TARGET_OPERATION, id, &[])
}

// ---- the timeline -----------------------------------------------------------

/// How a transition fares on one signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Flow {
    Fine,
    /// A clash or jump, by the tolerance.
    Clash,
    /// One of the two tracks has no value for it.
    Unknown,
}

fn flow(fit: Option<f64>) -> Flow {
    match fit {
        None => Flow::Unknown,
        Some(f) if f > 0.0 => Flow::Fine,
        Some(_) => Flow::Clash,
    }
}

/// One entry of the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    pub entry_id: PlaylistEntryId,
    /// From 0, dense.
    pub position: u32,
    pub recording_id: i64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub duration_ms: Option<i64>,
    pub bpm: Option<f64>,
    /// Camelot (`8A`).
    pub key: Option<String>,
    /// The energy the track shows (tags or the user's), 1 to 10.
    pub energy: Option<u8>,
    /// When it starts, from the start of the playlist, in milliseconds.
    pub start_ms: i64,
    /// The target energy at this entry; none with no target.
    pub target: Option<f64>,
    /// The entry's energy minus the target; none unless both are known.
    pub gap: Option<f64>,
}

/// The step from entry i to entry i + 1.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TimelineTransition {
    pub from_entry: PlaylistEntryId,
    pub to_entry: PlaylistEntryId,
    pub key: Flow,
    pub bpm: Flow,
    /// The tempo difference, next minus this, read as half, as is or double
    /// whichever is smallest (the "what's next" reading).
    pub bpm_delta: Option<f64>,
    pub energy: Flow,
    /// The next energy minus this.
    pub energy_step: Option<i32>,
}

/// A playlist's whole timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub playlist: Playlist,
    pub entries: Vec<TimelineEntry>,
    /// One fewer than the entries (none for fewer than two).
    pub transitions: Vec<TimelineTransition>,
    /// The sum of the known durations.
    pub total_ms: i64,
    /// How many entries have no duration, which `total_ms` leaves out.
    pub unknown_durations: u32,
    /// The target shape in order; empty with none.
    pub target: Vec<TargetPoint>,
    /// The tolerance the flags were judged by (the "what's next" setting).
    pub tolerance: Tolerance,
}

/// What a row is compared by: its BPM, key and energy as the table shows
/// them. The energy is only what the table shows, which is only a tag's or
/// the user's ([`crate::overview::energy_sources!`], ROADMAP 3.7).
fn facts_of_row(row: &TableRow) -> next::score::Facts {
    next::facts_of_shown(row.bpm, row.key.as_deref(), row.energy)
}

/// How far through the playlist the entry at `index` of `count` is.
fn fraction(index: usize, count: usize) -> f64 {
    if count <= 1 {
        0.0
    } else {
        index as f64 / (count - 1) as f64
    }
}

/// The playlist's timeline in one read; `None` if there's no such playlist.
pub fn timeline(
    conn: &Connection,
    volumes: &impl crate::paths::Volumes,
    id: PlaylistId,
) -> rusqlite::Result<Option<Timeline>> {
    let Some(view) = super::view(conn, volumes, id)? else {
        return Ok(None);
    };
    let tolerance = next::read_settings(conn)?.tolerance;
    let target = read_target(conn, id)?;
    let count = view.items.len();
    let facts: Vec<_> = view.items.iter().map(|i| facts_of_row(&i.row)).collect();

    let mut start_ms = 0_i64;
    let mut unknown_durations = 0_u32;
    let mut entries = Vec::with_capacity(count);
    for (i, item) in view.items.iter().enumerate() {
        let f = &facts[i];
        let at = target_at(&target, fraction(i, count));
        entries.push(TimelineEntry {
            entry_id: item.entry_id,
            position: item.position,
            recording_id: item.row.recording_id,
            title: item.row.title.clone(),
            artist: item.row.artist.clone(),
            duration_ms: item.row.time_ms,
            bpm: f.bpm,
            key: f.key.map(|k| k.name()),
            energy: f.energy,
            start_ms,
            target: at,
            gap: f.energy.zip(at).map(|(e, t)| f64::from(e) - t),
        });
        match item.row.time_ms {
            Some(ms) => start_ms += ms,
            None => unknown_durations += 1,
        }
    }

    let transitions = view
        .items
        .windows(2)
        .zip(facts.windows(2))
        .map(|(items, pair)| {
            let (a, b) = (&pair[0], &pair[1]);
            let energy_step = a
                .energy
                .zip(b.energy)
                .map(|(a, b)| i32::from(b) - i32::from(a));
            TimelineTransition {
                from_entry: items[0].entry_id,
                to_entry: items[1].entry_id,
                key: flow(bridge::key_fit(a.key, b.key, tolerance)),
                bpm: flow(bridge::bpm_fit(a.bpm, b.bpm, tolerance)),
                bpm_delta: a
                    .bpm
                    .zip(b.bpm)
                    .map(|(a, b)| crate::crates::fit::tempo_gap(a, b).0),
                energy: match energy_step {
                    None => Flow::Unknown,
                    Some(s) if bridge::energy_jump(s, tolerance) => Flow::Clash,
                    Some(_) => Flow::Fine,
                },
                energy_step,
            }
        })
        .collect();

    Ok(Some(Timeline {
        playlist: view.playlist,
        entries,
        transitions,
        total_ms: start_ms,
        unknown_durations,
        target,
        tolerance,
    }))
}

// ---- fill the gap ----------------------------------------------------------

/// One suggested track and why.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GapSuggestion {
    /// The track, as the track table shows it.
    pub row: TableRow,
    /// 0 to 100.
    pub score: u8,
    /// What gap it fills, strongest first; never empty.
    pub reasons: Vec<BridgeReason>,
}

/// The best tracks for a place in a playlist.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FillGapReply {
    pub suggestions: Vec<GapSuggestion>,
    /// How many Library tracks were weighed.
    pub considered: u32,
    /// The playlist's target energy at this place, if it has a target.
    pub target: Option<f64>,
}

/// Suggestions for the place after `after` (`None`: the very start). With
/// the place at the end, only A → X counts; at the start, only X → B. Tracks
/// already in the playlist are left out unless `include_playlist_tracks`
/// (the two neighbors never are). Three reads whatever the Library's size:
/// the playlist, every Library track's facts, the rows of the few kept.
pub fn fill_gap_on(
    conn: &Connection,
    volumes: &impl crate::paths::Volumes,
    id: PlaylistId,
    after: Option<PlaylistEntryId>,
    limit: u32,
    include_playlist_tracks: bool,
) -> rusqlite::Result<Done<FillGapReply>> {
    let Some(view) = super::view(conn, volumes, id)? else {
        return Ok(Err(PlaylistsError::NotFound));
    };
    let items = &view.items;
    let (a, b) = match after {
        None => (None, (!items.is_empty()).then_some(0)),
        Some(entry) => match items.iter().position(|i| i.entry_id == entry) {
            None => return Ok(Err(PlaylistsError::EntryNotFound)),
            Some(i) => (Some(i), (i + 1 < items.len()).then_some(i + 1)),
        },
    };
    let settings = next::read_settings(conn)?;
    let points = read_target(conn, id)?;
    let at = match (a, b) {
        (Some(a), Some(b)) => (fraction(a, items.len()) + fraction(b, items.len())) / 2.0,
        (Some(_), None) => 1.0,
        (None, _) => 0.0,
    };
    let target = target_at(&points, at);
    let mut exclude: HashSet<i64> = a
        .into_iter()
        .chain(b)
        .map(|i| items[i].row.recording_id)
        .collect();
    if !include_playlist_tracks {
        exclude.extend(items.iter().map(|i| i.row.recording_id));
    }
    let gap = Gap {
        before: a.map(|i| facts_of_row(&items[i].row)),
        after: b.map(|i| facts_of_row(&items[i].row)),
        target,
        settings: &settings,
        exclude: &exclude,
    };
    let candidates = next::candidates(conn, 0, false)?;
    let ranked = bridge::bridge(&gap, &candidates, limit.clamp(1, MOST_SUGGESTIONS) as usize);
    let ids: Vec<i64> = ranked.iter().map(|s| s.id).collect();
    let mut rows: std::collections::HashMap<i64, TableRow> =
        crate::track_table::rows_of(conn, volumes, &ids)?
            .into_iter()
            .map(|row| (row.recording_id, row))
            .collect();
    let suggestions = ranked
        .into_iter()
        .filter_map(|s| {
            rows.remove(&s.id).map(|row| GapSuggestion {
                row,
                score: s.score,
                reasons: s.reasons,
            })
        })
        .collect();
    Ok(Ok(FillGapReply {
        suggestions,
        considered: u32::try_from(candidates.len()).unwrap_or(u32::MAX),
        target,
    }))
}

// `async` runs the work off the main thread, like the other playlist commands.

/// One playlist's timeline: every entry in order with its facts, the flags
/// for each transition, and the target shape with the gap to it.
#[tauri::command]
#[specta::specta]
pub async fn playlist_timeline(
    reads: State<'_, ReadPool>,
    id: PlaylistId,
) -> Result<Timeline, IpcError> {
    let volumes = crate::scan::system_volumes();
    reads
        .read(move |c| timeline(c, &volumes, id))?
        .ok_or_else(|| PlaylistsError::NotFound.into())
}

/// Sets the playlist's target energy shape; the operation, or none when it's
/// the shape already stored.
#[tauri::command]
#[specta::specta]
pub async fn set_playlist_target(
    writer: State<'_, Writer>,
    id: PlaylistId,
    points: Vec<TargetPoint>,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |c| Ok(set_target_on(c, id, &points)))???)
}

/// Clears the playlist's target energy shape; the operation, or none when it
/// had none.
#[tauri::command]
#[specta::specta]
pub async fn clear_playlist_target(
    writer: State<'_, Writer>,
    id: PlaylistId,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |c| Ok(clear_target_on(c, id)))???)
}

/// The best Library tracks for the place after entry `after_entry_id` (none:
/// the start), each with what gap it fills. To insert one, add it at the
/// position after that entry.
#[tauri::command]
#[specta::specta]
pub async fn fill_gap(
    reads: State<'_, ReadPool>,
    id: PlaylistId,
    after_entry_id: Option<PlaylistEntryId>,
    limit: u32,
    include_playlist_tracks: bool,
) -> Result<FillGapReply, IpcError> {
    let volumes = crate::scan::system_volumes();
    Ok(reads.read(move |c| {
        fill_gap_on(
            c,
            &volumes,
            id,
            after_entry_id,
            limit,
            include_playlist_tracks,
        )
    })??)
}
