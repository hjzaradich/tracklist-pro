//! Which files are due an estimate, keeping the measurements in
//! `file_estimate` (migration 0053), and the `'local'` analysis rows shown
//! from them.
//!
//! The same rules as the waveforms ([`crate::waveform`]'s ledger), because
//! this is another thing computed from a file's audio:
//!
//! - **Results follow the audio, not the modified time** (ROADMAP 5.1). A
//!   present file is due when the hash stage is current for it and it has
//!   no row, or its row was measured from other audio, or by another
//!   [`VERSION`]. A retag costs nothing.
//! - **Each result is written in one transaction,** with the track's
//!   `'local'` row, and only while the file's row still has the audio
//!   hash, size and modified time the job read. A file that couldn't be
//!   reached gets no row, so it's tried again next time.
//! - **Library tracks only:** the file each Library track shows.
//! - **Due means checkable now:** not on an unplugged drive, and not an
//!   online-only OneDrive file unless the user opted in.
//!
//! **The `'local'` rows** ([`sync`]): a Library track whose shown file has a
//! BPM in the ledger, of its current audio and this [`VERSION`], has one
//! `'local'` analysis row with that BPM folded into the user's range and its
//! confidence. It is updated in place, so its id never changes; a track
//! whose file has no BPM any more loses it. Nothing else is written there:
//! no key (E2 adds it behind a switch), and never an energy (owner,
//! 2026-11-06: energy comes from the user or tags only). Every other
//! source wins over `'local'` by the precedence the table already has.

use rusqlite::{params, Connection};

use super::window::{Measured, Short};
use super::{bpm, VERSION};
use crate::hash::DEFINITION as HASH_DEFINITION;
use crate::quality::ledger::{Due, HASH_CURRENT};
use crate::quality::Failure;
use crate::settings::BpmRange;

/// How many due files are listed per query.
const PAGE: usize = 1000;

/// The condition that `f` has no estimate of its current audio.
const NOT_MEASURED: &str = "NOT EXISTS (SELECT 1 FROM file_estimate e
                WHERE e.file_id = f.id AND e.audio_hash = f.audio_hash
                  AND e.method_version = ?VERSION)";

/// The file a Library track `lt` shows, as SQL.
fn shown() -> String {
    crate::search::shown_file_sql("lt.linked_file_id", "lt.recording_id")
}

fn sql(template: &str) -> String {
    template
        .replace("?HASH", &i64::from(HASH_DEFINITION).to_string())
        .replace("?VERSION", &i64::from(VERSION).to_string())
}

fn ids_json(ids: &[i64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())
}

/// The files due, lowest id first, one page after `after`; only file `only`
/// if given, only those in the `reachable` music folders if given.
fn page(
    conn: &Connection,
    only: Option<i64>,
    reachable: Option<&[i64]>,
    after: i64,
    limit: usize,
) -> rusqlite::Result<Vec<Due>> {
    let reachable = reachable.map(ids_json);
    let mut stmt = conn.prepare_cached(&sql(&format!(
        "SELECT f.id, f.music_folder_id, f.rel_path, f.size, f.mtime, f.online_only, f.audio_hash
         FROM file f
         WHERE f.present = 1 AND f.id > ?1
           AND (?3 IS NULL OR f.music_folder_id IN (SELECT value FROM json_each(?3)))
           AND (?4 IS NULL OR f.id = ?4)
           AND f.id IN (SELECT {shown} FROM library_track lt)
           AND {HASH_CURRENT}
           AND {NOT_MEASURED}
         ORDER BY f.id LIMIT ?2",
        shown = shown()
    )))?;
    let rows = stmt.query_map(params![after, limit as i64, reachable, only], |r| {
        Ok(Due {
            id: r.get(0)?,
            folder: crate::scan::MusicFolderId(r.get(1)?),
            rel_path: r.get(2)?,
            size: r.get(3)?,
            mtime: r.get(4)?,
            online_only: r.get(5)?,
            audio_hash: r.get(6)?,
        })
    })?;
    rows.collect()
}

/// The ids of every Library track's file that's due, lowest first; only
/// files in the `reachable` music folders if given.
pub(crate) fn due_ids(conn: &Connection, reachable: Option<&[i64]>) -> rusqlite::Result<Vec<i64>> {
    let mut ids = Vec::new();
    let mut after = 0;
    loop {
        let files = page(conn, None, reachable, after, PAGE)?;
        let Some(last) = files.last() else {
            return Ok(ids);
        };
        after = last.id;
        ids.extend(files.iter().map(|f| f.id));
    }
}

/// File `id` as it stands now, if it's a Library track's file, present and
/// due.
pub(crate) fn due(conn: &Connection, id: i64) -> rusqlite::Result<Option<Due>> {
    Ok(page(conn, Some(id), None, id - 1, 1)?.into_iter().next())
}

/// Whether the job has anything to do now: a Library track's file due a
/// measurement (one the OneDrive gate allows, in the `reachable` music
/// folders if given), or a Library track whose file has a BPM measured and
/// no `'local'` row to show it (a relink or a merge put an analysed file
/// under it).
pub(crate) fn any_due(
    conn: &Connection,
    allows_online_only: bool,
    reachable: Option<&[i64]>,
) -> rusqlite::Result<bool> {
    let reachable = reachable.map(ids_json);
    conn.prepare_cached(&sql(&any_due_sql()))?
        .query_row(params![allows_online_only, reachable], |r| r.get(0))
}

fn any_due_sql() -> String {
    let shown = shown();
    format!(
        "SELECT EXISTS (SELECT 1 FROM file f
                        WHERE f.present = 1 AND (f.online_only = 0 OR ?1)
                          AND (?2 IS NULL OR f.music_folder_id IN (SELECT value FROM json_each(?2)))
                          AND f.id IN (SELECT {shown} FROM library_track lt)
                          AND {HASH_CURRENT} AND {NOT_MEASURED})
             OR EXISTS (SELECT 1 FROM library_track lt
                        JOIN file f ON f.id = {shown}
                        JOIN file_estimate e ON e.file_id = f.id AND e.audio_hash = f.audio_hash
                                            AND e.method_version = ?VERSION
                        WHERE e.bpm IS NOT NULL
                          AND NOT EXISTS (SELECT 1 FROM analysis a
                                          WHERE a.recording_id = lt.recording_id
                                            AND a.source = 'local' AND a.bpm IS NOT NULL))"
    )
}

/// What the job found in a file.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Measured(Measured),
    Failed(Failure),
}

impl Row {
    /// The columns: `(analysed_ms, bpm, bpm_confidence, skipped)`.
    fn columns(&self) -> (Option<i64>, Option<f64>, Option<f64>, Option<&'static str>) {
        match self {
            Row::Measured(Measured::Window { analysed_ms, tempo }) => (
                Some(*analysed_ms),
                tempo.map(|t| t.bpm),
                tempo.map(|t| t.confidence),
                None,
            ),
            Row::Measured(Measured::Short(Short::TooShort)) => {
                (None, None, None, Some("too_short"))
            }
            Row::Measured(Measured::Short(Short::Silent)) => (None, None, None, Some("silent")),
            Row::Failed(why) => (None, None, None, Some(why.as_str())),
        }
    }
}

/// Whether file `due.id`'s row is still the one `due` was read from.
fn unchanged(tx: &Connection, due: &Due) -> rusqlite::Result<bool> {
    tx.prepare_cached(&sql(&format!(
        "SELECT EXISTS (SELECT 1 FROM file f
                        WHERE f.id = ?1 AND f.present = 1 AND f.size IS ?2 AND f.mtime IS ?3
                          AND f.audio_hash = ?4 AND {HASH_CURRENT})"
    )))?
    .query_row(params![due.id, due.size, due.mtime, due.audio_hash], |r| {
        r.get(0)
    })
}

/// Stores `row` as file `due.id`'s estimate, in place of an older one, and
/// brings the `'local'` rows of the tracks showing it in step, in one
/// transaction; unless the file's row changed since `due` was read.
/// Returns whether it was stored.
pub(crate) fn record(
    conn: &mut Connection,
    due: &Due,
    row: &Row,
    range: BpmRange,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    if !unchanged(&tx, due)? {
        return Ok(false);
    }
    let (analysed_ms, bpm, confidence, skipped) = row.columns();
    tx.prepare_cached(
        "INSERT INTO file_estimate
             (file_id, audio_hash, method_version, analysed_ms, bpm, bpm_confidence, skipped)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (file_id) DO UPDATE SET
             audio_hash = excluded.audio_hash, method_version = excluded.method_version,
             analysed_ms = excluded.analysed_ms, bpm = excluded.bpm,
             bpm_confidence = excluded.bpm_confidence, skipped = excluded.skipped,
             loudness_lufs = NULL, onset_density = NULL, spectral_flux = NULL,
             energy_suggestion = NULL,
             analysed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
    )?
    .execute(params![
        due.id,
        due.audio_hash,
        i64::from(VERSION),
        analysed_ms,
        bpm,
        confidence,
        skipped,
    ])?;
    sync(&tx, Some(due.id), range)?;
    tx.commit()?;
    Ok(true)
}

/// Gives file `due.id` the estimate another file already has of the very
/// same audio (`audio_hash`, this `VERSION`), with its tracks' `'local'`
/// rows. Nothing is opened or decoded. Returns whether there was one to
/// copy.
pub(crate) fn reuse(conn: &mut Connection, due: &Due, range: BpmRange) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    if !unchanged(&tx, due)? {
        return Ok(false);
    }
    let copied = tx
        .prepare_cached(&sql("INSERT INTO file_estimate
                 (file_id, audio_hash, method_version, analysed_ms, bpm, bpm_confidence,
                  loudness_lufs, onset_density, spectral_flux, energy_suggestion, skipped)
             SELECT ?1, e.audio_hash, e.method_version, e.analysed_ms, e.bpm, e.bpm_confidence,
                    e.loudness_lufs, e.onset_density, e.spectral_flux, e.energy_suggestion,
                    e.skipped
             FROM file_estimate e
             WHERE e.audio_hash = ?2 AND e.method_version = ?VERSION AND e.file_id <> ?1
             LIMIT 1
             ON CONFLICT (file_id) DO UPDATE SET
                 audio_hash = excluded.audio_hash, method_version = excluded.method_version,
                 analysed_ms = excluded.analysed_ms, bpm = excluded.bpm,
                 bpm_confidence = excluded.bpm_confidence, loudness_lufs = excluded.loudness_lufs,
                 onset_density = excluded.onset_density, spectral_flux = excluded.spectral_flux,
                 energy_suggestion = excluded.energy_suggestion, skipped = excluded.skipped,
                 analysed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"))?
        .execute(params![due.id, due.audio_hash])?;
    if copied > 0 {
        sync(&tx, Some(due.id), range)?;
    }
    tx.commit()?;
    Ok(copied > 0)
}

/// A Library track's `'local'` row as it is, and as the ledger says it
/// should be.
struct Step {
    recording: i64,
    /// The folded BPM and confidence the ledger gives, if any.
    wanted: Option<(f64, f64)>,
    /// The row there is: its id, BPM and confidence, and whether it holds
    /// anything but a BPM (a key, from E2).
    row: Option<(i64, Option<f64>, Option<f64>, bool)>,
}

/// Brings the `'local'` analysis rows of the Library tracks showing file
/// `only` (every Library track for `None`) in step with the ledger, folding
/// each BPM into `range`. Writes only what differs: a row is inserted,
/// updated in place (its id kept) or, when its BPM goes and it holds
/// nothing else, deleted. Returns how many rows it wrote.
pub(crate) fn sync(
    conn: &Connection,
    only: Option<i64>,
    range: BpmRange,
) -> rusqlite::Result<usize> {
    let steps: Vec<Step> = {
        let mut stmt = conn.prepare_cached(&sql(&format!(
            "SELECT lt.recording_id, e.bpm, e.bpm_confidence,
                    a.id, a.bpm, a.confidence, a.key IS NOT NULL OR a.energy IS NOT NULL
             FROM library_track lt
             JOIN file f ON f.id = {shown}
             LEFT JOIN file_estimate e ON e.file_id = f.id AND e.audio_hash = f.audio_hash
                                      AND e.method_version = ?VERSION
             LEFT JOIN analysis a ON a.recording_id = lt.recording_id AND a.source = 'local'
             WHERE (?1 IS NULL OR f.id = ?1)",
            shown = shown()
        )))?;
        let rows = stmt.query_map([only], |r| {
            let bpm: Option<f64> = r.get(1)?;
            let confidence: Option<f64> = r.get(2)?;
            let id: Option<i64> = r.get(3)?;
            Ok(Step {
                recording: r.get(0)?,
                wanted: bpm.zip(confidence).map(|(b, c)| (bpm::fold(b, range), c)),
                row: match id {
                    Some(id) => Some((id, r.get(4)?, r.get(5)?, r.get::<_, bool>(6)?)),
                    None => None,
                },
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut written = 0;
    for step in steps {
        written += match (step.wanted, step.row) {
            (Some((bpm, confidence)), None) => conn
                .prepare_cached(
                    "INSERT INTO analysis (recording_id, source, bpm, confidence)
                     VALUES (?1, 'local', ?2, ?3)",
                )?
                .execute(params![step.recording, bpm, confidence])?,
            (Some((bpm, confidence)), Some((id, had, had_confidence, _)))
                if had != Some(bpm) || had_confidence != Some(confidence) =>
            {
                conn.prepare_cached(
                    "UPDATE analysis SET bpm = ?2, confidence = ?3,
                         analyzed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE id = ?1",
                )?
                .execute(params![id, bpm, confidence])?
            }
            (None, Some((id, Some(_), _, false))) => conn
                .prepare_cached("DELETE FROM analysis WHERE id = ?1")?
                .execute([id])?,
            (None, Some((id, Some(_), _, true))) => conn
                .prepare_cached("UPDATE analysis SET bpm = NULL WHERE id = ?1")?
                .execute([id])?,
            _ => 0,
        };
    }
    Ok(written)
}

/// What is stored for file `id`'s current audio, if anything: the BPM (as
/// the detector gave it, unfolded) and its confidence, or the reason there
/// are none.
#[cfg(test)]
pub(crate) fn current(conn: &Connection, id: i64) -> rusqlite::Result<Option<Stored>> {
    use rusqlite::OptionalExtension;
    conn.prepare_cached(&sql(
        "SELECT e.bpm, e.bpm_confidence, e.skipped, e.analysed_ms
         FROM file f JOIN file_estimate e ON e.file_id = f.id
         WHERE f.id = ?1 AND e.audio_hash = f.audio_hash AND e.method_version = ?VERSION",
    ))?
    .query_row([id], |r| {
        Ok(Stored {
            bpm: r.get(0)?,
            confidence: r.get(1)?,
            skipped: r.get(2)?,
            analysed_ms: r.get(3)?,
        })
    })
    .optional()
}

/// A stored row.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Stored {
    pub bpm: Option<f64>,
    pub confidence: Option<f64>,
    pub skipped: Option<String>,
    pub analysed_ms: Option<i64>,
}
