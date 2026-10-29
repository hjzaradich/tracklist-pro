//! Which files are due a waveform, and keeping the results in
//! `file_waveform` (migration 0040).
//!
//! The same rules as the quality measurements ([`crate::quality::ledger`]),
//! because the waveform is another thing computed from a file's audio:
//!
//! - **Results follow the audio, not the modified time** (ROADMAP 5.1). A
//!   present file is due when the hash stage is current for it (so its
//!   `audio_hash` can be vouched for) and it has no row, or its row was
//!   computed from other audio, or by another [`VERSION`]. A tag rewrite
//!   changes a file's size and mtime and nothing here: the audio hash stays,
//!   so the file isn't opened.
//! - **Each result is written in one transaction,** and only while the
//!   file's row still has the audio hash, size and modified time the job
//!   read. A file that couldn't be reached gets no row, so it's tried again
//!   next time.
//! - **Due means checkable now:** a file in a music folder whose volume
//!   isn't mounted isn't due, and neither is an online-only OneDrive file
//!   unless the user opted in to reading them.

use rusqlite::{params, Connection, OptionalExtension};

use super::VERSION;
use crate::hash::DEFINITION as HASH_DEFINITION;
use crate::quality::ledger::{Due, HASH_CURRENT};
use crate::quality::Failure;

/// How many due files are listed per query.
const PAGE: usize = 1000;

/// The condition that `f` has no waveform of its current audio.
const NOT_COMPUTED: &str = "NOT EXISTS (SELECT 1 FROM file_waveform w
                WHERE w.file_id = f.id AND w.audio_hash = f.audio_hash
                  AND w.method_version = ?VERSION)";

fn sql(template: &str) -> String {
    template
        .replace("?HASH", &i64::from(HASH_DEFINITION).to_string())
        .replace("?VERSION", &i64::from(VERSION).to_string())
}

/// `ids` as the JSON array the queries read with `json_each`.
fn ids_json(ids: &[i64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())
}

/// The files due, lowest id first, one page after `after`; only file `only`
/// if given, and only those in the `reachable` music folders if given.
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
           AND {HASH_CURRENT}
           AND {NOT_COMPUTED}
         ORDER BY f.id LIMIT ?2"
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

/// The ids of every file that's due, lowest first; only files in the
/// `reachable` music folders if given.
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

/// File `id` as it stands now, if it's present and due.
pub(crate) fn due(conn: &Connection, id: i64) -> rusqlite::Result<Option<Due>> {
    // `id - 1`: the page starts after it, so only `id` itself can come back.
    Ok(page(conn, Some(id), None, id - 1, 1)?.into_iter().next())
}

/// Whether any file is due a waveform that can be computed now: one the
/// OneDrive gate allows, in the `reachable` music folders if given.
pub(crate) fn any_due(
    conn: &Connection,
    allows_online_only: bool,
    reachable: Option<&[i64]>,
) -> rusqlite::Result<bool> {
    let reachable = reachable.map(ids_json);
    conn.prepare_cached(&sql(&format!(
        "SELECT EXISTS (SELECT 1 FROM file f
                        WHERE f.present = 1 AND (f.online_only = 0 OR ?1)
                          AND (?2 IS NULL OR f.music_folder_id IN (SELECT value FROM json_each(?2)))
                          AND {HASH_CURRENT} AND {NOT_COMPUTED})"
    )))?
    .query_row(params![allows_online_only, reachable], |r| r.get(0))
}

/// What the job found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Peaks { peaks: Vec<u8>, duration_ms: i64 },
    Failed(Failure),
}

/// Whether file `due.id`'s row is still the one `due` was read from: present,
/// with the same size, modified time and audio hash, and the hash stage
/// current for it.
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

/// Stores `row` as file `due.id`'s waveform, in place of an older one,
/// unless its row changed since `due` was read. Returns whether it was
/// stored.
pub(crate) fn record(conn: &mut Connection, due: &Due, row: &Row) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    if !unchanged(&tx, due)? {
        return Ok(false);
    }
    let (peaks, duration_ms, failure) = match row {
        Row::Peaks { peaks, duration_ms } => (Some(peaks.as_slice()), Some(*duration_ms), None),
        Row::Failed(why) => (None, None, Some(why.as_str())),
    };
    tx.prepare_cached(
        "INSERT INTO file_waveform
             (file_id, audio_hash, method_version, duration_ms, peaks, failure)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (file_id) DO UPDATE SET
             audio_hash = excluded.audio_hash, method_version = excluded.method_version,
             duration_ms = excluded.duration_ms, peaks = excluded.peaks,
             failure = excluded.failure,
             computed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
    )?
    .execute(params![
        due.id,
        due.audio_hash,
        i64::from(VERSION),
        duration_ms,
        peaks,
        failure,
    ])?;
    tx.commit()?;
    Ok(true)
}

/// Gives file `due.id` the waveform another file already has of the very
/// same audio (`audio_hash`, this `VERSION`): its peaks, or the reason there
/// are none. Nothing is opened or decoded. Returns whether there was one to
/// copy (and the file's row was still the one `due` was read from).
pub(crate) fn reuse(conn: &mut Connection, due: &Due) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    if !unchanged(&tx, due)? {
        return Ok(false);
    }
    let copied = tx
        .prepare_cached(&sql("INSERT INTO file_waveform
                 (file_id, audio_hash, method_version, duration_ms, peaks, failure)
             SELECT ?1, w.audio_hash, w.method_version, w.duration_ms, w.peaks, w.failure
             FROM file_waveform w
             WHERE w.audio_hash = ?2 AND w.method_version = ?VERSION AND w.file_id <> ?1
             LIMIT 1
             ON CONFLICT (file_id) DO UPDATE SET
                 audio_hash = excluded.audio_hash, method_version = excluded.method_version,
                 duration_ms = excluded.duration_ms, peaks = excluded.peaks,
                 failure = excluded.failure,
                 computed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"))?
        .execute(params![due.id, due.audio_hash])?;
    tx.commit()?;
    Ok(copied > 0)
}

/// What is stored for a file whose audio is the one the file has now, if
/// anything: `Ok(None)` when the job hasn't reached it (or it was computed
/// by another version, or from other audio).
pub(crate) fn current(conn: &Connection, id: i64) -> rusqlite::Result<Option<Stored>> {
    conn.prepare_cached(&sql(
        "SELECT w.duration_ms, w.peaks, w.failure
         FROM file f JOIN file_waveform w ON w.file_id = f.id
         WHERE f.id = ?1 AND w.audio_hash = f.audio_hash AND w.method_version = ?VERSION",
    ))?
    .query_row([id], |r| {
        Ok(Stored {
            duration_ms: r.get(0)?,
            peaks: r.get(1)?,
            failure: r.get(2)?,
        })
    })
    .optional()
}

/// A stored row: its peaks and length, or the reason it has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stored {
    pub duration_ms: Option<i64>,
    pub peaks: Option<Vec<u8>>,
    pub failure: Option<String>,
}
