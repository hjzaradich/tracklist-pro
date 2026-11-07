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
//! BPM in the ledger, of its current audio (this [`VERSION`], or an older one while it waits to be measured again), has one
//! `'local'` analysis row with that BPM folded into the user's range and its
//! confidence. While the owner's test switch for the key is on
//! ([`Feature::LocalKey`]), the row also holds the file's key, in Camelot;
//! with it off, no `'local'` row holds a key (turning it off takes them
//! out). The row's confidence is the BPM's; the key's stays here. It is
//! updated in place, so its id never changes; a track whose file has
//! neither any more loses it. Never an energy (owner, 2026-11-06: energy
//! comes from the user or tags only). Every other source wins over
//! `'local'` by the precedence the table already has. When the [`VERSION`]
//! is raised, a file's old ledger row waits to be measured again and is
//! shown (and counted by the stale check) as it is, whatever its version,
//! until the new measurement replaces it in place; the key switch applies to
//! it as to any other.
//!
//! The ledger also keeps the energy features of a file's window (loudness,
//! onset density, spectral flux) and the energy the app would suggest from
//! them ([`super::suggest`]); the suggestion is worked out again whenever a
//! file is measured again, so [`record`] clears it.

use rusqlite::{params, Connection};

use super::key::Choice;
use super::window::{Measured, Short};
use super::{bpm, VERSION};
use crate::hash::DEFINITION as HASH_DEFINITION;
use crate::quality::ledger::{Due, HASH_CURRENT};
use crate::quality::Failure;
use crate::settings::{read_feature_enabled, BpmRange, Feature};
use crate::tags::key::{Mode, MusicalKey};

/// How many due files are listed per query.
const PAGE: usize = 1000;

/// The condition that `f` has no estimate of its current audio.
const NOT_MEASURED: &str = "NOT EXISTS (SELECT 1 FROM file_estimate e
                WHERE e.file_id = f.id AND e.audio_hash = f.audio_hash
                  AND e.method_version = ?VERSION)";

/// The file a Library track `lt` shows, as SQL.
pub(super) fn shown() -> String {
    crate::search::shown_file_sql("lt.linked_file_id", "lt.recording_id")
}

pub(super) fn sql(template: &str) -> String {
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
/// folders if given), or a Library track whose `'local'` row doesn't match
/// what the ledger has for the file it shows ([`any_stale`]; the key counts
/// only while [`Feature::LocalKey`] is on).
pub(crate) fn any_due(
    conn: &Connection,
    allows_online_only: bool,
    reachable: Option<&[i64]>,
) -> rusqlite::Result<bool> {
    let reachable = reachable.map(ids_json);
    let local_key = read_feature_enabled(conn, Feature::LocalKey)?;
    conn.prepare_cached(&sql(&any_due_sql(local_key)))?
        .query_row(params![allows_online_only, reachable], |r| r.get(0))
}

fn any_due_sql(local_key: bool) -> String {
    let shown = shown();
    format!(
        "SELECT EXISTS (SELECT 1 FROM file f
                        WHERE f.present = 1 AND (f.online_only = 0 OR ?1)
                          AND (?2 IS NULL OR f.music_folder_id IN (SELECT value FROM json_each(?2)))
                          AND f.id IN (SELECT {shown} FROM library_track lt)
                          AND {HASH_CURRENT} AND {NOT_MEASURED})
             OR {STALE}",
        STALE = stale_sql(local_key)
    )
}

/// Whether a Library track's `'local'` row disagrees with the ledger's
/// entry for the file the track shows now: a BPM row where the file has
/// none, no BPM row where it has one, a BPM row of another measurement, or
/// (while [`Feature::LocalKey`] is on) a key that isn't the file's, and (off)
/// any key at all. A relink, a merge or an undo can leave any of these,
/// since they change the file a track shows without touching its `'local'`
/// row. The BPM row is identified by its confidence, which is stored as the
/// detector gave it and so tells one measurement from another (a change of
/// the BPM range is not seen here: setting the range folds the rows again,
/// [`super::set_bpm_range`]).
pub(crate) fn any_stale(conn: &Connection) -> rusqlite::Result<bool> {
    let local_key = read_feature_enabled(conn, Feature::LocalKey)?;
    conn.prepare_cached(&sql(&format!("SELECT {}", stale_sql(local_key))))?
        .query_row([], |r| r.get(0))
}

/// The key of a ledger row in Camelot, as SQL (NULL for none or silence),
/// built from [`MusicalKey::to_camelot`] so the two never differ.
fn camelot_sql() -> String {
    let mut sql = String::from("CASE e.key_mode");
    for (mode, name) in [(Mode::Major, "major"), (Mode::Minor, "minor")] {
        sql.push_str(&format!(" WHEN '{name}' THEN CASE e.key_pitch_class"));
        for pitch_class in 0..12 {
            let camelot = MusicalKey { pitch_class, mode }.to_camelot();
            sql.push_str(&format!(" WHEN {pitch_class} THEN '{camelot}'"));
        }
        sql.push_str(" END");
    }
    sql.push_str(" END");
    sql
}

/// The condition of [`any_stale`], as an `EXISTS (…)`.
fn stale_sql(local_key: bool) -> String {
    let wanted_key = if local_key {
        camelot_sql()
    } else {
        "NULL".to_string()
    };
    format!(
        "EXISTS (SELECT 1 FROM library_track lt
                 JOIN file f ON f.id = {shown}
                 LEFT JOIN file_estimate e ON e.file_id = f.id AND e.audio_hash = f.audio_hash
                 LEFT JOIN analysis a ON a.recording_id = lt.recording_id AND a.source = 'local'
                 WHERE CASE WHEN e.bpm IS NULL THEN a.bpm IS NOT NULL
                            ELSE a.bpm IS NULL OR a.confidence IS NOT e.bpm_confidence
                       END
                    OR a.key IS NOT ({wanted_key}))",
        shown = shown()
    )
}

/// What the job found in a file.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Measured(Measured),
    Failed(Failure),
}

/// A row's columns.
#[derive(Default)]
struct Columns {
    analysed_ms: Option<i64>,
    bpm: Option<f64>,
    bpm_confidence: Option<f64>,
    key_pitch_class: Option<u8>,
    key_mode: Option<&'static str>,
    key_confidence: Option<f64>,
    loudness_lufs: Option<f64>,
    onset_density: Option<f64>,
    spectral_flux: Option<f64>,
    skipped: Option<&'static str>,
}

impl Row {
    fn columns(&self) -> Columns {
        let skipped = |why| Columns {
            skipped: Some(why),
            ..Columns::default()
        };
        match self {
            Row::Measured(Measured::Window {
                analysed_ms,
                tempo,
                key,
                features,
            }) => {
                let (key_pitch_class, key_mode, key_confidence) = match key {
                    Some(Choice::Key { key, confidence }) => (
                        Some(key.pitch_class),
                        Some(match key.mode {
                            Mode::Major => "major",
                            Mode::Minor => "minor",
                        }),
                        Some(*confidence),
                    ),
                    Some(Choice::Silence) => (None, Some("silence"), None),
                    None => (None, None, None),
                };
                Columns {
                    analysed_ms: Some(*analysed_ms),
                    bpm: tempo.map(|t| t.bpm),
                    bpm_confidence: tempo.map(|t| t.confidence),
                    key_pitch_class,
                    key_mode,
                    key_confidence,
                    loudness_lufs: features.loudness_lufs,
                    onset_density: Some(features.onset_density),
                    spectral_flux: Some(features.spectral_flux),
                    skipped: None,
                }
            }
            Row::Measured(Measured::Short(Short::TooShort)) => skipped("too_short"),
            Row::Measured(Measured::Short(Short::Silent)) => skipped("silent"),
            Row::Failed(why) => skipped(why.as_str()),
        }
    }
}

/// The key a ledger row holds, from its columns.
pub(crate) fn key_of(pitch_class: Option<i64>, mode: Option<&str>) -> Option<MusicalKey> {
    let mode = match mode? {
        "major" => Mode::Major,
        "minor" => Mode::Minor,
        _ => return None,
    };
    let pitch_class = u8::try_from(pitch_class?).ok().filter(|p| *p < 12)?;
    Some(MusicalKey { pitch_class, mode })
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
    let c = row.columns();
    tx.prepare_cached(
        "INSERT INTO file_estimate
             (file_id, audio_hash, method_version, analysed_ms, bpm, bpm_confidence,
              key_pitch_class, key_mode, key_confidence,
              loudness_lufs, onset_density, spectral_flux, skipped)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT (file_id) DO UPDATE SET
             audio_hash = excluded.audio_hash, method_version = excluded.method_version,
             analysed_ms = excluded.analysed_ms, bpm = excluded.bpm,
             bpm_confidence = excluded.bpm_confidence,
             key_pitch_class = excluded.key_pitch_class, key_mode = excluded.key_mode,
             key_confidence = excluded.key_confidence, skipped = excluded.skipped,
             loudness_lufs = excluded.loudness_lufs, onset_density = excluded.onset_density,
             spectral_flux = excluded.spectral_flux,
             -- worked out again across the Library (suggest.rs)
             energy_suggestion = NULL,
             analysed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
    )?
    .execute(params![
        due.id,
        due.audio_hash,
        i64::from(VERSION),
        c.analysed_ms,
        c.bpm,
        c.bpm_confidence,
        c.key_pitch_class,
        c.key_mode,
        c.key_confidence,
        c.loudness_lufs,
        c.onset_density,
        c.spectral_flux,
        c.skipped,
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
                  key_pitch_class, key_mode, key_confidence,
                  loudness_lufs, onset_density, spectral_flux, energy_suggestion, skipped)
             SELECT ?1, e.audio_hash, e.method_version, e.analysed_ms, e.bpm, e.bpm_confidence,
                    e.key_pitch_class, e.key_mode, e.key_confidence,
                    e.loudness_lufs, e.onset_density, e.spectral_flux, e.energy_suggestion,
                    e.skipped
             FROM file_estimate e
             WHERE e.audio_hash = ?2 AND e.method_version = ?VERSION AND e.file_id <> ?1
             LIMIT 1
             ON CONFLICT (file_id) DO UPDATE SET
                 audio_hash = excluded.audio_hash, method_version = excluded.method_version,
                 analysed_ms = excluded.analysed_ms, bpm = excluded.bpm,
                 bpm_confidence = excluded.bpm_confidence,
                 key_pitch_class = excluded.key_pitch_class, key_mode = excluded.key_mode,
                 key_confidence = excluded.key_confidence, loudness_lufs = excluded.loudness_lufs,
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

/// What a Library track's `'local'` row holds.
#[derive(Debug, Clone, PartialEq, Default)]
struct Shown {
    bpm: Option<f64>,
    /// The BPM's confidence.
    confidence: Option<f64>,
    /// In Camelot.
    key: Option<String>,
}

impl Shown {
    fn is_empty(&self) -> bool {
        self.bpm.is_none() && self.key.is_none()
    }
}

/// A Library track's `'local'` row as it is, and as the ledger says it
/// should be.
struct Step {
    recording: i64,
    wanted: Shown,
    /// The row there is: its id, what it holds, and whether it holds an
    /// energy too (never written from here; a guard all the same).
    row: Option<(i64, Shown, bool)>,
}

/// Brings the `'local'` analysis rows of the Library tracks showing file
/// `only` (every Library track for `None`) in step with the ledger, folding
/// each BPM into `range`, with the key only while [`Feature::LocalKey`] is
/// on. Writes only what differs: a row is inserted, updated in place (its
/// id kept) or, when it would hold nothing, deleted. A file's ledger row
/// counts whatever method version it is of, so a file waiting to be measured
/// again keeps showing its own older measurement until the new one replaces
/// it. Returns how many rows it wrote.
pub(crate) fn sync(
    conn: &Connection,
    only: Option<i64>,
    range: BpmRange,
) -> rusqlite::Result<usize> {
    Ok(sync_page(conn, only, 0, -1, range)?.0)
}

/// [`sync`] for the next `limit` Library tracks (a negative `limit`: all of
/// them) after track `after`, lowest id first. Returns how many rows it
/// wrote and the last track it looked at, `None` when there were none.
pub(crate) fn sync_page(
    conn: &Connection,
    only: Option<i64>,
    after: i64,
    limit: i64,
    range: BpmRange,
) -> rusqlite::Result<(usize, Option<i64>)> {
    let local_key = read_feature_enabled(conn, Feature::LocalKey)?;
    let steps: Vec<Step> = {
        let mut stmt = conn.prepare_cached(&sql(&format!(
            "SELECT lt.recording_id, e.bpm, e.bpm_confidence, e.key_pitch_class, e.key_mode,
                    a.id, a.bpm, a.confidence, a.key, a.energy IS NOT NULL
             FROM library_track lt
             JOIN file f ON f.id = {shown}
             LEFT JOIN file_estimate e ON e.file_id = f.id AND e.audio_hash = f.audio_hash
             LEFT JOIN analysis a ON a.recording_id = lt.recording_id AND a.source = 'local'
             WHERE (?1 IS NULL OR f.id = ?1) AND lt.recording_id > ?2
             ORDER BY lt.recording_id LIMIT ?3",
            shown = shown()
        )))?;
        let rows = stmt.query_map(params![only, after, limit], |r| {
            let bpm: Option<f64> = r.get(1)?;
            let confidence: Option<f64> = r.get(2)?;
            let tempo = bpm.zip(confidence).map(|(b, c)| (bpm::fold(b, range), c));
            let key = if local_key {
                key_of(r.get(3)?, r.get::<_, Option<String>>(4)?.as_deref())
                    .map(MusicalKey::to_camelot)
            } else {
                None
            };
            let id: Option<i64> = r.get(5)?;
            Ok(Step {
                recording: r.get(0)?,
                wanted: Shown {
                    bpm: tempo.map(|t| t.0),
                    confidence: tempo.map(|t| t.1),
                    key,
                },
                row: match id {
                    Some(id) => Some((
                        id,
                        Shown {
                            bpm: r.get(6)?,
                            confidence: r.get(7)?,
                            key: r.get(8)?,
                        },
                        r.get::<_, bool>(9)?,
                    )),
                    None => None,
                },
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let last = steps.last().map(|s| s.recording);
    let mut written = 0;
    for step in steps {
        let wanted = step.wanted;
        written += match step.row {
            None if !wanted.is_empty() => conn
                .prepare_cached(
                    "INSERT INTO analysis (recording_id, source, bpm, key, confidence)
                     VALUES (?1, 'local', ?2, ?3, ?4)",
                )?
                .execute(params![
                    step.recording,
                    wanted.bpm,
                    wanted.key,
                    wanted.confidence
                ])?,
            Some((id, _, false)) if wanted.is_empty() => conn
                .prepare_cached("DELETE FROM analysis WHERE id = ?1")?
                .execute([id])?,
            Some((id, had, _)) if had != wanted => conn
                .prepare_cached(
                    "UPDATE analysis SET bpm = ?2, key = ?3, confidence = ?4,
                         analyzed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE id = ?1",
                )?
                .execute(params![id, wanted.bpm, wanted.key, wanted.confidence])?,
            _ => 0,
        };
    }
    Ok((written, last))
}

/// What is stored for file `id`'s current audio, if anything: the BPM (as
/// the detector gave it, unfolded) and its confidence, or the reason there
/// are none.
#[cfg(test)]
pub(crate) fn current(conn: &Connection, id: i64) -> rusqlite::Result<Option<Stored>> {
    use rusqlite::OptionalExtension;
    conn.prepare_cached(&sql(
        "SELECT e.bpm, e.bpm_confidence, e.skipped, e.analysed_ms,
                e.key_pitch_class, e.key_mode, e.key_confidence
         FROM file f JOIN file_estimate e ON e.file_id = f.id
         WHERE f.id = ?1 AND e.audio_hash = f.audio_hash AND e.method_version = ?VERSION",
    ))?
    .query_row([id], |r| {
        Ok(Stored {
            bpm: r.get(0)?,
            confidence: r.get(1)?,
            skipped: r.get(2)?,
            analysed_ms: r.get(3)?,
            key: key_of(r.get(4)?, r.get::<_, Option<String>>(5)?.as_deref()),
            key_mode: r.get(5)?,
            key_confidence: r.get(6)?,
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
    pub key: Option<MusicalKey>,
    /// `'major'`, `'minor'` or `'silence'`.
    pub key_mode: Option<String>,
    pub key_confidence: Option<f64>,
}
