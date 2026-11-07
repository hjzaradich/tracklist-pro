//! The energy the app would suggest for a track (ROADMAP 3.7; 3C-E1b).
//!
//! **Owner rule (2026-11-06): a track's energy comes only from tags set
//! beforehand (Mixed In Key) or the user.** What is worked out here is a
//! suggestion, nothing more:
//!
//! - It lives in the ledger (`file_estimate.energy_suggestion`) and nowhere
//!   else. Never an `analysis` row, so nothing that reads a track's energy
//!   (the track table, sorting, smart crates, "what's next", stars, the
//!   timeline, "fill the gap": [`crate::overview::energy_sources!`]) ever
//!   sees it. The one way it becomes a track's energy is the user taking it,
//!   as an ordinary, undoable `set_energy` ([`crate::energy`]).
//! - **A percentile, not a formula.** Each feature of a file (loudness,
//!   onset density, spectral flux and the tempo, folded into the user's
//!   range) is ranked among the Library's files that have features; the mean
//!   of the ranks it has is its score, and the suggestion is the tenth its
//!   score's own rank falls in, 1 to 10. So it says how a track compares
//!   with the rest of *your* Library, and about as many tracks sit at each
//!   level whatever the music.
//! - **Why** is codes and numbers, never text ([`Basis`]): "louder than 83%
//!   of your Library" is the code `loudness` and 83. The UI words it. The
//!   batch keeps, with its count and time, a small table of each feature's
//!   percentiles ([`Cuts`], 101 numbers a feature) in the setting [`BATCH`]
//!   (no migration), so a track's basis is worked out from its own ledger
//!   row and that table, never from the whole Library.
//! - **When.** One batch over the whole Library after the first full pass,
//!   and again whenever the number of files with features has changed by
//!   more than [`RECOMPUTE_OVER`] since the last batch (so a first pass that
//!   was only partway, a drive that wasn't plugged in, is revisited as more
//!   files get measured; so is a Library that shrank). Between batches a
//!   file measured since then (it has features and no suggestion) is placed
//!   among the same population, and only it is written. Reads and writes go
//!   a page at a time, so the writer is never held for the whole Library.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::bpm;
use super::ledger::{shown, sql};
use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::IpcError;
use crate::settings::{read_bpm_range, BpmRange};

/// The setting that keeps the last batch: `{"count": <files with features>,
/// "at": <time>, "cuts": <[`Cuts`]>}`.
pub const BATCH: &str = "energy_batch";

/// The share the count of files with features may change by before the batch
/// is worked out again.
pub const RECOMPUTE_OVER: f64 = 0.05;

/// How many files one read or write call takes.
const PAGE: usize = 1000;

/// How many numbers a feature's percentile table holds: 0% to 100%.
const CUTS: usize = 101;

/// Each feature's percentiles at the last batch, 0% to 100% (empty when no
/// file has the feature): enough to say where a value stands in the Library
/// without the Library.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Cuts {
    loudness: Vec<f64>,
    onsets: Vec<f64>,
    flux: Vec<f64>,
    tempo: Vec<f64>,
}

/// `sorted`'s values at 0%, 1% ... 100%, between two by a straight line.
fn cut_points(sorted: &[f64]) -> Vec<f64> {
    let Some(last) = sorted.len().checked_sub(1) else {
        return Vec::new();
    };
    (0..CUTS)
        .map(|k| {
            let at = (k as f64 / (CUTS - 1) as f64) * last as f64;
            let (lo, frac) = (at.floor() as usize, at.fract());
            let hi = (lo + 1).min(last);
            sorted[lo] + (sorted[hi] - sorted[lo]) * frac
        })
        .collect()
}

/// Where `x` stands in `cuts`, 0 to 100: between two cut points by a
/// straight line, in the middle of a run of equal ones.
fn percentile(cuts: &[f64], x: f64) -> u8 {
    let below = cuts.partition_point(|c| *c < x);
    let up_to = cuts.partition_point(|c| *c <= x);
    let p = if below < up_to {
        (below + up_to - 1) as f64 / 2.0
    } else if below == 0 {
        0.0
    } else if below == cuts.len() {
        (cuts.len() - 1) as f64
    } else {
        let (lo, hi) = (cuts[below - 1], cuts[below]);
        (below - 1) as f64 + (x - lo) / (hi - lo)
    };
    p.round().clamp(0.0, 100.0) as u8
}

impl Cuts {
    /// `e`'s place in each feature it has.
    fn basis(&self, e: &Entry) -> Vec<Basis> {
        let features = [
            (BasisCode::Loudness, e.loudness, &self.loudness),
            (BasisCode::Onsets, Some(e.onsets), &self.onsets),
            (BasisCode::Flux, Some(e.flux), &self.flux),
            (BasisCode::Tempo, e.tempo, &self.tempo),
        ];
        features
            .into_iter()
            .filter_map(|(code, x, cuts)| {
                let x = x.filter(|_| !cuts.is_empty())?;
                Some(Basis {
                    code,
                    percentile: percentile(cuts, x),
                })
            })
            .collect()
    }
}

/// What a part of the basis is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BasisCode {
    /// Integrated loudness: the percentile is the share of the Library's
    /// files this one is louder than.
    Loudness,
    /// Onsets a second: denser than that share.
    Onsets,
    /// Spectral flux: moves more than that share.
    Flux,
    /// Tempo (folded into the user's range): faster than that share.
    Tempo,
}

/// One feature's place in the Library at the last batch: `percentile` is 0
/// to 100, about the share of the Library's files this one is above in
/// `code`'s feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Basis {
    pub code: BasisCode,
    pub percentile: u8,
}

/// The energy the app suggests for a track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnergySuggestion {
    /// 1 to 10.
    pub value: u8,
    pub basis: Vec<Basis>,
}

/// A file with features, as the ledger has it.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    file: i64,
    /// Folded into the user's range.
    tempo: Option<f64>,
    loudness: Option<f64>,
    onsets: f64,
    flux: f64,
    suggested: Option<u8>,
}

/// The files of Library tracks (their shown file, of its current audio and
/// this method) that have features, as SQL from `FROM` through `WHERE`.
fn featured_files() -> String {
    format!(
        "file_estimate e JOIN file f ON f.id = e.file_id AND f.audio_hash = e.audio_hash
         WHERE e.method_version = ?VERSION AND e.onset_density IS NOT NULL
           AND e.file_id IN (SELECT {} FROM library_track lt)",
        shown()
    )
}

/// The next `limit` featured files after file `after`.
fn page(
    conn: &Connection,
    after: i64,
    limit: i64,
    range: BpmRange,
) -> rusqlite::Result<Vec<Entry>> {
    let mut stmt = conn.prepare_cached(&sql(&format!(
        "SELECT e.file_id, e.bpm, e.loudness_lufs, e.onset_density, e.spectral_flux,
                e.energy_suggestion
         FROM {} AND e.file_id > ?1 ORDER BY e.file_id LIMIT ?2",
        featured_files()
    )))?;
    let rows = stmt.query_map(params![after, limit], |r| entry_of(r, range))?;
    rows.collect()
}

/// File `file`, if it has features (of its current audio and this method).
fn entry(conn: &Connection, file: i64, range: BpmRange) -> rusqlite::Result<Option<Entry>> {
    conn.prepare_cached(&sql(
        "SELECT e.file_id, e.bpm, e.loudness_lufs, e.onset_density, e.spectral_flux,
                e.energy_suggestion
         FROM file_estimate e JOIN file f ON f.id = e.file_id AND f.audio_hash = e.audio_hash
         WHERE e.file_id = ?1 AND e.method_version = ?VERSION AND e.onset_density IS NOT NULL",
    ))?
    .query_row([file], |r| entry_of(r, range))
    .optional()
}

fn entry_of(r: &rusqlite::Row<'_>, range: BpmRange) -> rusqlite::Result<Entry> {
    let bpm: Option<f64> = r.get(1)?;
    Ok(Entry {
        file: r.get(0)?,
        tempo: bpm.map(|b| bpm::fold(b, range)),
        loudness: r.get(2)?,
        onsets: r.get(3)?,
        flux: r.get(4)?,
        suggested: r.get(5)?,
    })
}

/// Every featured file, a page at a time through `read`.
fn all<E>(mut read: impl FnMut(i64) -> Result<Vec<Entry>, E>) -> Result<Vec<Entry>, E> {
    let (mut entries, mut after) = (Vec::new(), 0);
    loop {
        let rows = read(after)?;
        let Some(last) = rows.last() else {
            return Ok(entries);
        };
        after = last.file;
        entries.extend(rows);
    }
}

/// The Library's features, sorted, to rank a file in.
struct Population {
    loudness: Vec<f64>,
    onsets: Vec<f64>,
    flux: Vec<f64>,
    tempo: Vec<f64>,
    scores: Vec<f64>,
}

fn sorted(values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut v: Vec<f64> = values.collect();
    v.sort_by(f64::total_cmp);
    v
}

/// The share of `sorted` that `x` is above, ties counting half: 0 to 1.
fn rank(sorted: &[f64], x: f64) -> f64 {
    let below = sorted.partition_point(|v| *v < x);
    let up_to = sorted.partition_point(|v| *v <= x);
    (below + up_to) as f64 / 2.0 / sorted.len() as f64
}

impl Population {
    fn of(entries: &[Entry]) -> Population {
        let mut p = Population {
            loudness: sorted(entries.iter().filter_map(|e| e.loudness)),
            onsets: sorted(entries.iter().map(|e| e.onsets)),
            flux: sorted(entries.iter().map(|e| e.flux)),
            tempo: sorted(entries.iter().filter_map(|e| e.tempo)),
            scores: Vec::new(),
        };
        let scores: Vec<f64> = entries.iter().map(|e| score(&p.ranks(e))).collect();
        p.scores = sorted(scores.into_iter());
        p
    }

    /// `e`'s rank in each feature it has, 0 to 1.
    fn ranks(&self, e: &Entry) -> Vec<(BasisCode, f64)> {
        let mut ranks = Vec::with_capacity(4);
        if let Some(x) = e.loudness {
            ranks.push((BasisCode::Loudness, rank(&self.loudness, x)));
        }
        ranks.push((BasisCode::Onsets, rank(&self.onsets, e.onsets)));
        ranks.push((BasisCode::Flux, rank(&self.flux, e.flux)));
        if let Some(x) = e.tempo {
            ranks.push((BasisCode::Tempo, rank(&self.tempo, x)));
        }
        ranks
    }

    /// The energy to suggest for `e`, 1 to 10.
    fn judge(&self, e: &Entry) -> u8 {
        let tenth = (rank(&self.scores, score(&self.ranks(e))) * 10.0).floor() as i64 + 1;
        tenth.clamp(1, 10) as u8
    }

    /// The percentile tables of the features, for the basis.
    fn cuts(&self) -> Cuts {
        Cuts {
            loudness: cut_points(&self.loudness),
            onsets: cut_points(&self.onsets),
            flux: cut_points(&self.flux),
            tempo: cut_points(&self.tempo),
        }
    }
}

/// The mean of the ranks.
fn score(ranks: &[(BasisCode, f64)]) -> f64 {
    ranks.iter().map(|r| r.1).sum::<f64>() / ranks.len() as f64
}

/// The last batch's count of files with features, if there was one.
fn last_count(conn: &Connection) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT json_extract(value, '$.count') FROM setting WHERE key = ?1",
        [BATCH],
        |r| r.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// What a refresh has to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Work {
    /// The whole Library: the first batch, or the files with features have
    /// changed in number by a lot since the last.
    Batch,
    /// Only the files measured since.
    Newcomers,
}

/// How many Library tracks show a file that has features: the count the
/// batch is kept for. (Tracks, not files: a relink that points two tracks at
/// one file changes which files are shown, not how many tracks have
/// features.)
fn featured_tracks(conn: &Connection) -> rusqlite::Result<i64> {
    conn.prepare_cached(&sql(&format!(
        "SELECT count(*) FROM library_track lt
         JOIN file f ON f.id = {shown}
         JOIN file_estimate e ON e.file_id = f.id AND e.audio_hash = f.audio_hash
         WHERE e.method_version = ?VERSION AND e.onset_density IS NOT NULL",
        shown = shown()
    )))?
    .query_row([], |r| r.get(0))
}

/// What is due now, if anything. Nothing while no file has features.
fn work(conn: &Connection) -> rusqlite::Result<Option<Work>> {
    let featured = featured_tracks(conn)?;
    if featured == 0 {
        return Ok(None);
    }
    let pending: i64 = conn.query_row(
        &sql(&format!(
            "SELECT count(*) FROM {} AND e.energy_suggestion IS NULL",
            featured_files()
        )),
        [],
        |r| r.get(0),
    )?;
    Ok(match last_count(conn)? {
        Some(last)
            if last > 0 && (featured - last).abs() as f64 / last as f64 <= RECOMPUTE_OVER =>
        {
            (pending > 0).then_some(Work::Newcomers)
        }
        _ => Some(Work::Batch),
    })
}

/// Whether [`refresh`] has anything to do.
pub(crate) fn needed(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(work(conn)?.is_some())
}

/// Works the suggestions out: the whole Library when it's time for a batch,
/// otherwise only the files that have none yet. Returns how many it wrote.
/// Each page of reads and of writes is its own writer call.
pub(crate) fn refresh(writer: &Writer) -> Result<usize, DbError> {
    refresh_paged(writer, PAGE)
}

/// [`refresh`] with `size` files to a call.
pub(crate) fn refresh_paged(writer: &Writer, size: usize) -> Result<usize, DbError> {
    let Some(work) = writer.call(|c| work(c))? else {
        return Ok(0);
    };
    let range = writer.call(|c| read_bpm_range(c))?;
    let entries = all(|after| writer.call(move |c| page(c, after, size as i64, range)))?;
    let population = Population::of(&entries);
    // Each with the features it was ranked on, so a file measured again
    // meanwhile (new features, no suggestion) is left for the next refresh.
    let due: Vec<(i64, u8, f64, f64)> = entries
        .iter()
        .filter(|e| work != Work::Newcomers || e.suggested.is_none())
        .map(|e| (e.file, population.judge(e), e.onsets, e.flux))
        .collect();
    let mut written = 0;
    for chunk in due.chunks(size) {
        let chunk = chunk.to_vec();
        written += writer.call(move |c| {
            let tx = c.transaction()?;
            let mut n = 0;
            {
                let mut set =
                    tx.prepare_cached(&sql("UPDATE file_estimate SET energy_suggestion = ?2
                     WHERE file_id = ?1 AND method_version = ?VERSION
                       AND onset_density IS ?3 AND spectral_flux IS ?4
                       AND energy_suggestion IS NOT ?2"))?;
                for (file, value, onsets, flux) in chunk {
                    n += set.execute(params![file, value, onsets, flux])?;
                }
            }
            tx.commit()?;
            Ok(n)
        })?;
    }
    if work == Work::Batch {
        let cuts = population.cuts();
        writer.call(move |c| {
            let batch = serde_json::json!({ "count": featured_tracks(c)?, "cuts": cuts });
            c.execute(
                "INSERT INTO setting (key, value)
                 VALUES (?1, json_set(?2, '$.at', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))
                 ON CONFLICT (key) DO UPDATE SET
                     value = excluded.value,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![BATCH, batch.to_string()],
            )
        })?;
    }
    Ok(written)
}

/// The suggestion for the Library track `recording`: what the ledger holds
/// for the file it shows, with its basis from the last batch's percentile
/// tables. One ledger row and one setting are read, whatever the size of the
/// Library. `None` when the track isn't in the Library, its file has no
/// features (not analysed yet, can't be, or silent) or no suggestion was
/// worked out yet.
pub(crate) fn suggestion_for(
    conn: &Connection,
    recording: i64,
) -> rusqlite::Result<Option<EnergySuggestion>> {
    let file: Option<i64> = conn
        .query_row(
            &format!(
                "SELECT {} FROM library_track lt WHERE lt.recording_id = ?1",
                shown()
            ),
            [recording],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(file) = file else {
        return Ok(None);
    };
    let range = read_bpm_range(conn)?;
    let Some(entry) = entry(conn, file, range)? else {
        return Ok(None);
    };
    let Some(value) = entry.suggested else {
        return Ok(None);
    };
    let cuts: Option<String> = conn
        .query_row(
            "SELECT json_extract(value, '$.cuts') FROM setting WHERE key = ?1",
            [BATCH],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let cuts = cuts
        .and_then(|c| serde_json::from_str::<Cuts>(&c).ok())
        .unwrap_or_default();
    Ok(Some(EnergySuggestion {
        value,
        basis: cuts.basis(&entry),
    }))
}

/// The energy the app suggests for a track, with why, or nothing. Only a
/// suggestion (ROADMAP 3.7): the track has no energy from it. To use it, set
/// the energy as the user would (`set_energy`), one undoable edit.
#[tauri::command]
#[specta::specta]
pub async fn energy_suggestion(
    reads: State<'_, ReadPool>,
    recording_id: i64,
) -> Result<Option<EnergySuggestion>, IpcError> {
    Ok(reads.read(move |conn| suggestion_for(conn, recording_id))?)
}
