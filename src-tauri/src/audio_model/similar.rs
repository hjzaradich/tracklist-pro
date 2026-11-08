//! "Sounds like" (ROADMAP 3.8, 5A-A3): the Library tracks whose fingerprints
//! are nearest a track's, for the "Audio model" panel. Review only: it is the
//! audio model's opinion, shown on its own, and it feeds no other suggestion
//! (not "What's next").
//!
//! - **Similarity.** Every current fingerprint (192 numbers) has the mean
//!   fingerprint of all of them taken off, then is scaled to length 1. The
//!   similarity of two tracks is the cosine of those, which is then just a
//!   dot product. Taking the mean off matters: fingerprints of one library
//!   share a large common part, so without it everything looks alike. The
//!   percent shown is the cosine as a whole percent, 0 when it is negative.
//! - **Which tracks.** Library tracks' shown files, present, with a result
//!   of the current model for their current audio and a whole, finite
//!   fingerprint. The track itself and its other files are never listed.
//!   The 25 best, best first; a tie goes to the lower track id.
//! - **A cache,** so a search is one pass over memory, not one blob read per
//!   track: the vectors, already centred and scaled, in one array
//!   (192 x 4 bytes a track). It is rebuilt on the first search after
//!   [`fingerprints_changed`] has been called, which [`ledger::record`]
//!   does whenever it stores or removes a result. Tracks that left the
//!   Library or changed their file since are checked against the database
//!   for the best few and dropped, so a stale cache can only make the list
//!   shorter, never wrong.
//! - **Never loads the model.**
//!
//! [`ledger::record`]: super::ledger

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use tauri::State;

use super::panel::{fingerprint_of, shown, standing, PanelState, Standing};
use super::subset;
use super::EMBEDDING_LEN;
use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::paths::Volumes;
use crate::track_table::{self, TableRow};

/// How many tracks are listed.
pub const LISTED: usize = 25;

/// How many of the best are checked against the database before the first
/// [`LISTED`] that pass are kept.
const CHECKED: usize = 4 * LISTED;

/// Bumped each time a stored fingerprint is added, replaced or removed.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Says the fingerprints in the database have changed: the next search
/// rebuilds its cache. (A counter shared by every database in the process,
/// which only ever means an extra rebuild.)
pub(crate) fn fingerprints_changed() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// One listed track.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SimilarTrack {
    pub row: TableRow,
    /// 0 to 100.
    pub percent: u8,
}

/// The tracks that sound like a track, or why there are none.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SoundsLike {
    pub state: PanelState,
    pub tracks: Vec<SimilarTrack>,
}

impl SoundsLike {
    fn none(state: PanelState) -> SoundsLike {
        SoundsLike {
            state,
            tracks: Vec::new(),
        }
    }
}

/// Every current fingerprint, centred and scaled.
struct Matrix {
    generation: u64,
    mean: Vec<f32>,
    /// Per row: the file and the track it is the shown file of.
    files: Vec<i64>,
    recordings: Vec<i64>,
    /// `files.len() * EMBEDDING_LEN` numbers.
    unit: Vec<f32>,
}

/// Takes `mean` off `v` and scales it to length 1 (all zeros if nothing is
/// left to point anywhere).
fn centred(v: &[f32], mean: &[f32]) -> Vec<f32> {
    let mut out: Vec<f32> = v.iter().zip(mean).map(|(a, m)| a - m).collect();
    let norm = out
        .iter()
        .map(|x| f64::from(*x).powi(2))
        .sum::<f64>()
        .sqrt();
    if norm > 1e-9 {
        for x in &mut out {
            *x = (f64::from(*x) / norm) as f32;
        }
    } else {
        out.fill(0.0);
    }
    out
}

impl Matrix {
    fn build(conn: &Connection, generation: u64) -> rusqlite::Result<Matrix> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT lt.recording_id, f.id, e.fingerprint
             FROM library_track lt
             JOIN file f ON f.id = {shown} AND f.present = 1
             JOIN embedding e ON e.file_id = f.id AND e.audio_hash = f.audio_hash
                             AND e.model_id = ?1",
            shown = shown()
        ))?;
        let mut files = Vec::new();
        let mut recordings = Vec::new();
        let mut raw: Vec<f32> = Vec::new();
        let mut sum = vec![0f64; EMBEDDING_LEN];
        let mut rows = stmt.query([subset::model_id()])?;
        while let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(2)?;
            let Some(fingerprint) = fingerprint_of(&bytes) else {
                continue;
            };
            recordings.push(row.get(0)?);
            files.push(row.get(1)?);
            for (s, x) in sum.iter_mut().zip(&fingerprint) {
                *s += f64::from(*x);
            }
            raw.extend(fingerprint);
        }
        let n = files.len().max(1) as f64;
        let mean: Vec<f32> = sum.iter().map(|s| (s / n) as f32).collect();
        let mut unit = Vec::with_capacity(raw.len());
        for v in raw.as_chunks::<EMBEDDING_LEN>().0 {
            unit.extend(centred(v, &mean));
        }
        Ok(Matrix {
            generation,
            mean,
            files,
            recordings,
            unit,
        })
    }

    /// The best [`CHECKED`] rows for `query` (centred, scaled), best first,
    /// leaving out track `not`: `(row, cosine)`.
    fn nearest(&self, query: &[f32], not: i64) -> Vec<(usize, f32)> {
        let mut scored: Vec<(usize, f32)> = self
            .unit
            .as_chunks::<EMBEDDING_LEN>()
            .0
            .iter()
            .enumerate()
            .filter(|(i, _)| self.recordings[*i] != not)
            .map(|(i, v)| (i, v.iter().zip(query).map(|(a, b)| a * b).sum::<f32>()))
            .collect();
        let order = |a: &(usize, f32), b: &(usize, f32)| {
            b.1.total_cmp(&a.1)
                .then(self.recordings[a.0].cmp(&self.recordings[b.0]))
        };
        if scored.len() > CHECKED {
            scored.select_nth_unstable_by(CHECKED, order);
            scored.truncate(CHECKED);
        }
        scored.sort_by(order);
        scored
    }
}

/// The cache of fingerprints "Sounds like" searches, kept by the app.
#[derive(Clone, Default)]
pub struct Neighbours(Arc<Mutex<Option<Matrix>>>);

impl Neighbours {
    fn lock(&self) -> MutexGuard<'_, Option<Matrix>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lets go of the cache's memory. The next search rebuilds it.
    pub fn release(&self) {
        *self.lock() = None;
    }

    /// Whether the cache is built (for tests).
    #[cfg(test)]
    pub(crate) fn built(&self) -> bool {
        self.lock().is_some()
    }
}

/// Of the rows `top` (row, cosine), those still true in the database: the
/// track is still a Library track showing that file, present, with a result
/// for its current audio. In `top`'s order.
fn still_true(
    conn: &Connection,
    matrix: &Matrix,
    top: &[(usize, f32)],
) -> rusqlite::Result<Vec<(usize, f32)>> {
    let ids: Vec<i64> = top.iter().map(|&(i, _)| matrix.recordings[i]).collect();
    let json = serde_json::to_string(&ids).expect("numbers always serialize");
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT lt.recording_id, f.id
         FROM library_track lt
         JOIN file f ON f.id = {shown} AND f.present = 1
         WHERE lt.recording_id IN (SELECT value FROM json_each(?1))
           AND EXISTS (SELECT 1 FROM embedding e WHERE e.file_id = f.id
                       AND e.audio_hash = f.audio_hash AND e.model_id = ?2)",
        shown = shown()
    ))?;
    let current: std::collections::HashSet<(i64, i64)> = stmt
        .query_map((json, subset::model_id()), |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(top
        .iter()
        .copied()
        .filter(|&(i, _)| current.contains(&(matrix.recordings[i], matrix.files[i])))
        .collect())
}

/// The percent shown for a cosine.
fn percent_of(cosine: f32) -> u8 {
    (cosine.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// The tracks that sound like track `recording`, using `cache`.
pub(crate) fn sounds_like_of(
    conn: &Connection,
    cache: &Neighbours,
    volumes: &impl Volumes,
    recording: i64,
) -> rusqlite::Result<SoundsLike> {
    let heard = match standing(conn, recording)? {
        Standing::Not(state) => {
            if state == PanelState::Off {
                cache.release();
            }
            return Ok(SoundsLike::none(state));
        }
        Standing::Heard(heard) => heard,
    };
    let Some(fingerprint) = heard.fingerprint else {
        return Ok(SoundsLike::none(PanelState::CouldntBeRead));
    };
    let ranked: Vec<(i64, f32)> = {
        let mut guard = cache.lock();
        let generation = GENERATION.load(Ordering::SeqCst);
        if guard.as_ref().is_none_or(|m| m.generation != generation) {
            *guard = Some(Matrix::build(conn, generation)?);
        }
        let matrix = guard.as_ref().expect("built above");
        let query = centred(&fingerprint, &matrix.mean);
        if query.iter().all(|x| *x == 0.0) {
            // Exactly the average sound: nothing is nearer than anything else.
            return Ok(SoundsLike::none(PanelState::Ready));
        }
        let top = matrix.nearest(&query, recording);
        still_true(conn, matrix, &top)?
            .into_iter()
            .take(LISTED)
            .map(|(i, cosine)| (matrix.recordings[i], cosine))
            .collect()
    };
    if ranked.is_empty() {
        return Ok(SoundsLike::none(PanelState::Ready));
    }
    let ids: Vec<i64> = ranked.iter().map(|&(id, _)| id).collect();
    let mut rows: std::collections::HashMap<i64, TableRow> =
        track_table::rows_of(conn, volumes, &ids)?
            .into_iter()
            .map(|row| (row.recording_id, row))
            .collect();
    let tracks = ranked
        .into_iter()
        .filter_map(|(id, cosine)| {
            rows.remove(&id).map(|row| SimilarTrack {
                row,
                percent: percent_of(cosine),
            })
        })
        .collect();
    Ok(SoundsLike {
        state: PanelState::Ready,
        tracks,
    })
}

/// The Library tracks that sound most like a track, by the audio model's
/// fingerprints. Reads only; never loads the model.
#[tauri::command]
#[specta::specta]
pub async fn sounds_like(
    reads: State<'_, ReadPool>,
    cache: State<'_, Neighbours>,
    recording_id: i64,
) -> Result<SoundsLike, IpcError> {
    let volumes = crate::scan::system_volumes();
    let cache = cache.inner().clone();
    Ok(reads.read(move |c| sounds_like_of(c, &cache, &volumes, recording_id))?)
}
