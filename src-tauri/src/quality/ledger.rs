//! Which files are due a quality measurement, and keeping the results in
//! `file_quality` (migration 0016).
//!
//! **Results follow the audio, not the modified time** (ROADMAP 5.1,
//! 1aC-10). A present file is due when the hash stage is current for it
//! (so its `audio_hash` can be vouched for) and it has no row, or its row
//! was measured from other audio, or by another [`VERSION`] of the method.
//! A tag rewrite changes a file's size and mtime and nothing here: the
//! audio hash stays, so the file isn't opened.
//!
//! Each result is written in one transaction, and only while the file's
//! row still has the audio hash, size and modified time the job read: if a
//! walk or a hash changed it meanwhile, nothing is written and the file
//! stays due. A file that couldn't be reached gets no row at all, so it's
//! tried again next time.

use rusqlite::{params, Connection, OptionalExtension};

use super::cutoff::Gap;
use super::measure::{Errors, Failure, Measured};
use super::VERSION;
use crate::hash::DEFINITION as HASH_DEFINITION;
use crate::scan::MusicFolderId;

/// How many due files are listed per query.
const PAGE: usize = 1000;

/// A file due a measurement, as its row says now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Due {
    pub id: i64,
    pub folder: MusicFolderId,
    pub rel_path: String,
    pub size: Option<i64>,
    /// Nanoseconds since the Unix epoch.
    pub mtime: Option<i64>,
    pub online_only: bool,
    /// The audio the measurement will be made from.
    pub audio_hash: Vec<u8>,
}

/// The condition that the hash stage is current for file `f`: it has an
/// `audio_hash`, and a done `hash` row at the current version, size and
/// mtime. `?HASH` is the hash stage's version.
const HASH_CURRENT: &str = "f.audio_hash IS NOT NULL
    AND EXISTS (SELECT 1 FROM file_stage h
                WHERE h.file_id = f.id AND h.stage = 'hash' AND h.status = 'done'
                  AND h.version = ?HASH AND h.size IS f.size AND h.mtime IS f.mtime)";

/// The condition that `f` has no measurement of its current audio.
const NOT_MEASURED: &str = "NOT EXISTS (SELECT 1 FROM file_quality q
                WHERE q.file_id = f.id AND q.audio_hash = f.audio_hash
                  AND q.method_version = ?VERSION)";

fn sql(template: &str) -> String {
    template
        .replace("?HASH", &i64::from(HASH_DEFINITION).to_string())
        .replace("?VERSION", &i64::from(VERSION).to_string())
}

/// The files due, lowest id first, one page after `after`; among `only`
/// if given.
fn page(
    conn: &Connection,
    only: Option<&[i64]>,
    after: i64,
    limit: usize,
) -> rusqlite::Result<Vec<Due>> {
    let only = only.map(|ids| serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()));
    let mut stmt = conn.prepare_cached(&sql(&format!(
        "SELECT f.id, f.music_folder_id, f.rel_path, f.size, f.mtime, f.online_only, f.audio_hash
         FROM file f
         WHERE f.present = 1 AND f.id > ?1
           AND (?2 IS NULL OR f.id IN (SELECT value FROM json_each(?2)))
           AND {HASH_CURRENT}
           AND {NOT_MEASURED}
         ORDER BY f.id LIMIT ?3"
    )))?;
    let rows = stmt.query_map(params![after, only, limit as i64], |r| {
        Ok(Due {
            id: r.get(0)?,
            folder: MusicFolderId(r.get(1)?),
            rel_path: r.get(2)?,
            size: r.get(3)?,
            mtime: r.get(4)?,
            online_only: r.get(5)?,
            audio_hash: r.get(6)?,
        })
    })?;
    rows.collect()
}

/// The ids of every file that's due, or of those among `only`, lowest
/// first.
pub(crate) fn due_ids(conn: &Connection, only: Option<&[i64]>) -> rusqlite::Result<Vec<i64>> {
    let mut ids = Vec::new();
    let mut after = 0;
    loop {
        let files = page(conn, only, after, PAGE)?;
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
    Ok(page(conn, Some(&[id]), id - 1, 1)?.into_iter().next())
}

/// Whether any file is due a measurement that `allows` may read (the
/// OneDrive gate: online-only files count only if the user opted in), for
/// the chain to judge whether a job is worth queuing.
pub(crate) fn any_due(conn: &Connection, allows_online_only: bool) -> rusqlite::Result<bool> {
    conn.prepare_cached(&sql(&format!(
        "SELECT EXISTS (SELECT 1 FROM file f
                        WHERE f.present = 1 AND (f.online_only = 0 OR ?1)
                          AND {HASH_CURRENT} AND {NOT_MEASURED})"
    )))?
    .query_row([allows_online_only], |r| r.get(0))
}

/// What a measurement says about a file, as one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Measured(Measured),
    Failed(Failure),
}

/// Stores `row` as file `due.id`'s measurement, in place of an older one,
/// unless its row changed since `due` was read. Returns whether it was
/// stored.
pub(crate) fn record(conn: &mut Connection, due: &Due, row: &Row) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    let unchanged: bool = tx
        .prepare_cached(&sql(&format!(
            "SELECT EXISTS (SELECT 1 FROM file f
                            WHERE f.id = ?1 AND f.present = 1 AND f.size IS ?2 AND f.mtime IS ?3
                              AND f.audio_hash = ?4 AND {HASH_CURRENT})"
        )))?
        .query_row(params![due.id, due.size, due.mtime, due.audio_hash], |r| {
            r.get(0)
        })?;
    if !unchanged {
        return Ok(false);
    }
    let (cutoff_hz, cutoff_gap, decoded_ms, header_ms, errors, ended, failure) = match row {
        Row::Measured(m) => (
            m.cutoff.as_ref().ok().copied(),
            m.cutoff.as_ref().err().map(|gap: &Gap| gap.as_str()),
            Some(m.decoded_ms),
            m.header_ms,
            m.errors,
            Some(m.ended.as_str()),
            None,
        ),
        Row::Failed(why) => (
            None,
            None,
            None,
            None,
            Errors::default(),
            None,
            Some(why.as_str()),
        ),
    };
    tx.prepare_cached(
        "INSERT INTO file_quality
             (file_id, audio_hash, method_version, cutoff_hz, cutoff_gap, decoded_ms,
              header_ms, decode_errors, error_kinds, ended, failure)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (file_id) DO UPDATE SET
             audio_hash = excluded.audio_hash, method_version = excluded.method_version,
             cutoff_hz = excluded.cutoff_hz, cutoff_gap = excluded.cutoff_gap,
             decoded_ms = excluded.decoded_ms, header_ms = excluded.header_ms,
             decode_errors = excluded.decode_errors, error_kinds = excluded.error_kinds,
             ended = excluded.ended, failure = excluded.failure,
             measured_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
    )?
    .execute(params![
        due.id,
        due.audio_hash,
        i64::from(VERSION),
        cutoff_hz,
        cutoff_gap,
        decoded_ms,
        header_ms,
        errors.total(),
        errors.kinds_json(),
        ended,
        failure,
    ])?;
    tx.commit()?;
    Ok(true)
}

/// What's stored for file `id`, as [`record`] wrote it. For tests and for
/// the verdicts built on these measurements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub audio_hash: Vec<u8>,
    pub method_version: i64,
    pub cutoff_hz: Option<i64>,
    pub cutoff_gap: Option<String>,
    pub decoded_ms: Option<i64>,
    pub header_ms: Option<i64>,
    pub decode_errors: i64,
    pub error_kinds: Option<String>,
    pub ended: Option<String>,
    pub failure: Option<String>,
}

/// The measurement stored for file `id`, if there is one.
pub fn stored(conn: &Connection, id: i64) -> rusqlite::Result<Option<Stored>> {
    conn.query_row(
        "SELECT audio_hash, method_version, cutoff_hz, cutoff_gap, decoded_ms, header_ms,
                decode_errors, error_kinds, ended, failure
         FROM file_quality WHERE file_id = ?1",
        [id],
        |r| {
            Ok(Stored {
                audio_hash: r.get(0)?,
                method_version: r.get(1)?,
                cutoff_hz: r.get(2)?,
                cutoff_gap: r.get(3)?,
                decoded_ms: r.get(4)?,
                header_ms: r.get(5)?,
                decode_errors: r.get(6)?,
                error_kinds: r.get(7)?,
                ended: r.get(8)?,
                failure: r.get(9)?,
            })
        },
    )
    .optional()
}
