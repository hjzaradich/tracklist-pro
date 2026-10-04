//! Where comparison results are kept: the `fingerprint_match` table
//! (migration 0016), one row per compared pair of files, the lower file id
//! first.
//!
//! A row stands for the two fingerprints it was made from. The database
//! deletes a file's rows when its `fingerprint` bytes change (a trigger),
//! and a row is only written while both files still hold the fingerprints
//! that were compared ([`put`]), so a stored result is never about other
//! audio. A new size or modified time changes nothing here (ROADMAP 5.1).

use std::collections::{HashMap, HashSet};

use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::compare::{Comparison, Segment};
use super::VERSION;

/// A stored result: `comparison`'s A is `file_a` and its B is `file_b`.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredMatch {
    pub file_a: i64,
    pub file_b: i64,
    pub comparison: Comparison,
}

const COLUMNS: &str = "file_a, file_b, items_a, items_b, coverage_a, coverage_b, score, segments";

fn bad_segments(e: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(e))
}

/// `[[offset_a, offset_b, items, score, alignment], …]`.
fn segments_to_json(segments: &[Segment]) -> String {
    let rows: Vec<(usize, usize, usize, f64, u8)> = segments
        .iter()
        .map(|s| (s.offset_a, s.offset_b, s.items, s.score, s.alignment))
        .collect();
    serde_json::to_string(&rows).expect("numbers always serialize")
}

fn segments_from_json(json: &str) -> rusqlite::Result<Vec<Segment>> {
    let rows: Vec<(usize, usize, usize, f64, u8)> =
        serde_json::from_str(json).map_err(bad_segments)?;
    Ok(rows
        .into_iter()
        .map(|(offset_a, offset_b, items, score, alignment)| Segment {
            offset_a,
            offset_b,
            items,
            score,
            alignment,
        })
        .collect())
}

fn stored(row: &Row<'_>) -> rusqlite::Result<StoredMatch> {
    let segments: String = row.get(7)?;
    Ok(StoredMatch {
        file_a: row.get(0)?,
        file_b: row.get(1)?,
        comparison: Comparison {
            items_a: row.get::<_, i64>(2)? as usize,
            items_b: row.get::<_, i64>(3)? as usize,
            coverage_a: row.get::<_, f64>(4)? as f32,
            coverage_b: row.get::<_, f64>(5)? as f32,
            score: row.get(6)?,
            segments: segments_from_json(&segments)?,
        },
    })
}

/// Every stored result of this [`VERSION`], by file ids.
pub fn all(conn: &Connection) -> rusqlite::Result<Vec<StoredMatch>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM fingerprint_match WHERE version = ?1 ORDER BY file_a, file_b"
    ))?;
    let rows = stmt.query_map([VERSION], stored)?;
    rows.collect()
}

/// The stored results file `id` is part of, as stored: `id` may be either
/// side ([`Comparison::swapped`] turns one round).
pub fn of_file(conn: &Connection, id: i64) -> rusqlite::Result<Vec<StoredMatch>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM fingerprint_match
         WHERE version = ?1 AND (file_a = ?2 OR file_b = ?2) ORDER BY file_a, file_b"
    ))?;
    let rows = stmt.query_map(params![VERSION, id], stored)?;
    rows.collect()
}

/// The stored result for files `a` and `b`, if they've been compared, with
/// `a` as the comparison's A whichever id is lower.
pub fn of_pair(conn: &Connection, a: i64, b: i64) -> rusqlite::Result<Option<Comparison>> {
    let (low, high) = (a.min(b), a.max(b));
    let found = conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM fingerprint_match
                 WHERE version = ?1 AND file_a = ?2 AND file_b = ?3"
            ),
            params![VERSION, low, high],
            stored,
        )
        .optional()?;
    Ok(found.map(|m| {
        if m.file_a == a {
            m.comparison
        } else {
            m.comparison.swapped()
        }
    }))
}

/// Every pair with a result of this [`VERSION`], the lower id first.
pub(crate) fn compared_pairs(conn: &Connection) -> rusqlite::Result<HashSet<(i64, i64)>> {
    let mut stmt =
        conn.prepare("SELECT file_a, file_b FROM fingerprint_match WHERE version = ?1")?;
    let rows = stmt.query_map([VERSION], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// Deletes results made by another [`VERSION`] of the comparison. Returns
/// how many went.
pub(crate) fn drop_other_versions(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM fingerprint_match WHERE version <> ?1",
        [VERSION],
    )
}

/// One side of a result to store: the file, and the fingerprint blob that
/// was compared.
pub(crate) struct Side<'a> {
    pub file: i64,
    pub blob: &'a [u8],
}

/// Stores `comparison` (A is `a`, B is `b`) for the pair, replacing an
/// older result, but only if both files still hold exactly the fingerprints
/// that were compared. Returns whether it was stored.
pub(crate) fn put(
    conn: &Connection,
    a: Side<'_>,
    b: Side<'_>,
    comparison: &Comparison,
) -> rusqlite::Result<bool> {
    if a.file == b.file {
        return Ok(false);
    }
    let swapped;
    let (low, high, comparison) = if a.file < b.file {
        (a, b, comparison)
    } else {
        swapped = comparison.swapped();
        (b, a, &swapped)
    };
    let changed = conn
        .prepare_cached(
            "INSERT OR REPLACE INTO fingerprint_match
                 (file_a, file_b, version, items_a, items_b,
                  coverage_a, coverage_b, score, segments)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
             WHERE (SELECT fingerprint FROM file WHERE id = ?1) = ?10
               AND (SELECT fingerprint FROM file WHERE id = ?2) = ?11",
        )?
        .execute(params![
            low.file,
            high.file,
            VERSION,
            comparison.items_a as i64,
            comparison.items_b as i64,
            f64::from(comparison.coverage_a),
            f64::from(comparison.coverage_b),
            comparison.score,
            segments_to_json(&comparison.segments),
            low.blob,
            high.blob,
        ])?;
    Ok(changed == 1)
}

/// The next `limit` present files with a fingerprint, by id after `after`:
/// each file's id and fingerprint blob.
pub(crate) fn fingerprints_after(
    conn: &Connection,
    after: i64,
    limit: usize,
) -> rusqlite::Result<Vec<(i64, Vec<u8>)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, fingerprint FROM file
         WHERE id > ?1 AND present = 1 AND fingerprint IS NOT NULL
         ORDER BY id LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![after, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// File `id`'s fingerprint blob, if it has one.
pub(crate) fn fingerprint_of(conn: &Connection, id: i64) -> rusqlite::Result<Option<Vec<u8>>> {
    let blob: Option<Option<Vec<u8>>> = conn
        .prepare_cached("SELECT fingerprint FROM file WHERE id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?;
    Ok(blob.flatten())
}

/// Whether a matching pass has anything to do: a present file has a
/// fingerprint and no `fingerprint_matched` row of this [`VERSION`], or its
/// row names a file that no longer stands for it (gone, missing, or
/// holding another fingerprint now).
pub fn any_due(conn: &Connection) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (
             SELECT 1 FROM file f
             LEFT JOIN fingerprint_matched m ON m.file_id = f.id AND m.version = ?1
             WHERE f.present = 1 AND f.fingerprint IS NOT NULL
               AND (m.file_id IS NULL
                    OR (m.stands <> f.id AND NOT EXISTS (
                            SELECT 1 FROM file s
                            WHERE s.id = m.stands AND s.present = 1
                              AND s.fingerprint = f.fingerprint))))",
    )?
    .query_row([VERSION], |r| r.get(0))
}

/// Every file that has been through a pass of this [`VERSION`] with its
/// current fingerprint, and the file whose results stand for it.
pub(crate) fn matched(conn: &Connection) -> rusqlite::Result<HashMap<i64, i64>> {
    let mut stmt =
        conn.prepare("SELECT file_id, stands FROM fingerprint_matched WHERE version = ?1")?;
    let rows = stmt.query_map([VERSION], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// Records that `file` is through matching with the fingerprint it was
/// read with, its results standing under `stands`; only if it still holds
/// exactly that fingerprint. Returns whether it was recorded.
pub(crate) fn mark_matched(
    conn: &Connection,
    file: Side<'_>,
    stands: i64,
) -> rusqlite::Result<bool> {
    let changed = conn
        .prepare_cached(
            "INSERT OR REPLACE INTO fingerprint_matched (file_id, version, stands)
             SELECT ?1, ?2, ?3
             WHERE (SELECT fingerprint FROM file WHERE id = ?1) = ?4
               AND EXISTS (SELECT 1 FROM file WHERE id = ?3)",
        )?
        .execute(params![file.file, VERSION, stands, file.blob])?;
    Ok(changed == 1)
}
