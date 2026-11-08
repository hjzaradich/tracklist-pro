//! Which files are due the audio model, keeping what it heard in `embedding`
//! and the files it couldn't hear in `embedding_failure` (migration 0055).
//!
//! The same rules as the estimates ([`crate::estimate`]'s ledger), because
//! this is another thing computed from a file's audio:
//!
//! - **Results follow the audio, not the modified time.** A present file is
//!   due when the hash stage is current for it and it has neither a result
//!   nor a failure for its current `audio_hash` and the current
//!   [`model_id`](super::subset::model_id). A retag costs nothing; a file
//!   whose audio changed is listened to again; a new model or label list
//!   listens to every file again.
//! - **A failure is not retried** until the file's audio (or the model)
//!   changes. A file that could not be reached at all (its drive is
//!   unplugged, a OneDrive file that is online only, a file rewritten while
//!   it was read) gets nothing recorded, so it is tried again next time.
//! - **Each result is written in one transaction,** only while the file's
//!   row still has the audio hash, size and modified time the job read, and
//!   replaces the file's older result or failure.
//! - **Library tracks only:** the file each Library track shows.

use rusqlite::{params, Connection, OptionalExtension};

use super::subset::{self, SUBSET_LEN};
use super::EMBEDDING_LEN;
use crate::hash::DEFINITION as HASH_DEFINITION;
use crate::quality::ledger::{Due, HASH_CURRENT};

/// How many due files are listed per query.
const PAGE: usize = 1000;

/// The condition that `f` has neither a result nor a failure for its current
/// audio and the model `?MODEL`.
const NOT_DONE: &str = "NOT EXISTS (SELECT 1 FROM embedding e
                WHERE e.file_id = f.id AND e.audio_hash = f.audio_hash AND e.model_id = ?MODEL)
            AND NOT EXISTS (SELECT 1 FROM embedding_failure x
                WHERE x.file_id = f.id AND x.audio_hash = f.audio_hash AND x.model_id = ?MODEL)";

/// `template` with the hash stage's version and the model's id (a name
/// made of letters, digits and `@ / -`, never quoted text) put in.
fn sql(template: &str) -> String {
    template
        .replace("?HASH", &i64::from(HASH_DEFINITION).to_string())
        .replace("?MODEL", &format!("'{}'", subset::model_id()))
}

fn shown() -> String {
    crate::search::shown_file_sql("lt.linked_file_id", "lt.recording_id")
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
           AND {NOT_DONE}
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

/// Whether any Library track's file is due, one the OneDrive gate allows
/// (`allows_online_only`), in the `reachable` music folders if given.
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
                          AND f.id IN (SELECT {shown} FROM library_track lt)
                          AND {HASH_CURRENT} AND {NOT_DONE})",
        shown = shown()
    )))?
    .query_row(params![allows_online_only, reachable], |r| r.get(0))
}

/// Why a file wasn't listened to. Stored as `embedding_failure.reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    NoAudio,
    /// A format or codec this build can't decode.
    Unsupported,
    Damaged,
    /// The decoder or the model panicked on it.
    DecoderCrashed,
    /// The model failed on it.
    ModelFailed,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::NoAudio => "no_audio",
            Reason::Unsupported => "unsupported",
            Reason::Damaged => "damaged",
            Reason::DecoderCrashed => "decoder_crashed",
            Reason::ModelFailed => "model_failed",
        }
    }
}

/// What the job found in a file.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The fingerprint ([`EMBEDDING_LEN`] numbers) and the kept label
    /// scores ([`SUBSET_LEN`] numbers).
    Heard {
        fingerprint: Vec<f32>,
        scores: Vec<f32>,
    },
    Failed(Reason),
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

/// Stores `outcome` as file `due.id`'s result, in place of an older result
/// or failure, in one transaction; unless the file's row changed since `due`
/// was read. Returns whether it was stored.
pub(crate) fn record(
    conn: &mut Connection,
    due: &Due,
    outcome: &Outcome,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    if !unchanged(&tx, due)? {
        return Ok(false);
    }
    let model = subset::model_id();
    match outcome {
        Outcome::Heard {
            fingerprint,
            scores,
        } => {
            debug_assert!(fingerprint.len() == EMBEDDING_LEN && scores.len() == SUBSET_LEN);
            tx.execute("DELETE FROM embedding_failure WHERE file_id = ?1", [due.id])?;
            tx.execute(
                "INSERT INTO embedding (file_id, audio_hash, model_id, fingerprint, label_scores)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (file_id) DO UPDATE SET
                     audio_hash = excluded.audio_hash, model_id = excluded.model_id,
                     fingerprint = excluded.fingerprint, label_scores = excluded.label_scores,
                     embedded_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![
                    due.id,
                    due.audio_hash,
                    model,
                    subset::to_blob(fingerprint),
                    subset::to_blob(scores)
                ],
            )?;
        }
        Outcome::Failed(reason) => {
            // A result of the audio before it changed would be shown for
            // the new audio: it goes.
            tx.execute("DELETE FROM embedding WHERE file_id = ?1", [due.id])?;
            tx.execute(
                "INSERT INTO embedding_failure (file_id, audio_hash, model_id, reason)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (file_id) DO UPDATE SET
                     audio_hash = excluded.audio_hash, model_id = excluded.model_id,
                     reason = excluded.reason,
                     failed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![due.id, due.audio_hash, model, reason.as_str()],
            )?;
        }
    }
    tx.commit()?;
    Ok(true)
}

/// A stored result: its fingerprint and the kept label scores.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub model_id: String,
    pub fingerprint: Vec<f32>,
    pub scores: Vec<f32>,
}

/// File `file`'s result, if it has one for its current audio and the current
/// model, of the right sizes.
pub fn current(conn: &Connection, file: i64) -> rusqlite::Result<Option<Stored>> {
    type Row = (String, Vec<u8>, Vec<u8>);
    let row: Option<Row> = conn
        .query_row(
            "SELECT e.model_id, e.fingerprint, e.label_scores
             FROM embedding e JOIN file f ON f.id = e.file_id AND f.audio_hash = e.audio_hash
             WHERE e.file_id = ?1 AND e.model_id = ?2",
            params![file, subset::model_id()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    // A row of the wrong size is not a result: treated as not done.
    Ok(row.and_then(|(model_id, fingerprint, scores)| {
        let (fingerprint, scores) = (
            subset::from_blob(&fingerprint)?,
            subset::from_blob(&scores)?,
        );
        (fingerprint.len() == EMBEDDING_LEN && scores.len() == SUBSET_LEN).then_some(Stored {
            model_id,
            fingerprint,
            scores,
        })
    }))
}

/// How far the work has got, over the Library tracks' files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    /// Library tracks' present files.
    pub total: i64,
    /// Of those, with a result for their current audio and model.
    pub fingerprinted: i64,
    /// Of those, with a failure for their current audio and model.
    pub failed: i64,
}

/// [`Counts`] now.
pub fn counts(conn: &Connection) -> rusqlite::Result<Counts> {
    conn.prepare_cached(&sql(&format!(
        "SELECT count(*), count(e.file_id), count(x.file_id)
         FROM file f
         LEFT JOIN embedding e ON e.file_id = f.id AND e.audio_hash IS f.audio_hash
                              AND e.model_id = ?MODEL
         LEFT JOIN embedding_failure x ON x.file_id = f.id AND x.audio_hash IS f.audio_hash
                              AND x.model_id = ?MODEL
         WHERE f.present = 1 AND f.id IN (SELECT {shown} FROM library_track lt)",
        shown = shown()
    )))?
    .query_row([], |r| {
        Ok(Counts {
            total: r.get(0)?,
            fingerprinted: r.get(1)?,
            failed: r.get(2)?,
        })
    })
}
