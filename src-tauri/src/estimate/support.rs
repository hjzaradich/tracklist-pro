//! What tests in other modules (the relink, the grouping and the review
//! tests) need to set up and look at the app's own estimates, without
//! decoding any audio.

use rusqlite::Connection;

use super::VERSION;
use crate::db::{DbError, Writer};
use crate::jobs::{JobId, NewJob};

/// File `file`'s ledger row as the job leaves it for its current audio:
/// the detector's `bpm` and `confidence` (or neither).
pub(crate) fn plant(
    conn: &Connection,
    file: i64,
    bpm: Option<f64>,
    confidence: Option<f64>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO file_estimate
             (file_id, audio_hash, method_version, analysed_ms, bpm, bpm_confidence)
         SELECT id, audio_hash, ?2, 1000, ?3, ?4 FROM file WHERE id = ?1",
        rusqlite::params![file, i64::from(VERSION), bpm, confidence],
    )?;
    Ok(())
}

/// A `'local'` analysis row on `track`, as an older sync left it.
pub(crate) fn plant_local(
    conn: &Connection,
    track: i64,
    bpm: f64,
    confidence: f64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO analysis (recording_id, source, bpm, confidence) VALUES (?1, 'local', ?2, ?3)",
        rusqlite::params![track, bpm, confidence],
    )?;
    Ok(())
}

/// The BPM of `track`'s `'local'` row, if it has one.
pub(crate) fn local_bpm(writer: &Writer, track: i64) -> Option<f64> {
    writer
        .call(move |c| {
            c.query_row(
                "SELECT bpm FROM analysis WHERE recording_id = ?1 AND source = 'local'",
                [track],
                |r| r.get(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
        })
        .unwrap()
}

/// Queues by storing the row, as the queue does.
pub(crate) fn store_row(writer: &Writer) -> impl Fn(NewJob) -> Result<JobId, DbError> + '_ {
    |job| {
        writer.call(move |c| {
            c.execute(
                "INSERT INTO job (kind, target, priority) VALUES (?1, ?2, ?3)",
                (
                    job.kind.as_str(),
                    job.target.map(|t| t.to_string()),
                    job.priority.0,
                ),
            )?;
            Ok(JobId(c.last_insert_rowid()))
        })
    }
}

/// How many jobs of `kind` are queued.
pub(crate) fn queued(writer: &Writer, kind: &'static str) -> i64 {
    writer
        .call(move |c| {
            c.query_row(
                "SELECT count(*) FROM job WHERE kind = ?1 AND status = 'queued'",
                [kind],
                |r| r.get(0),
            )
        })
        .unwrap()
}
