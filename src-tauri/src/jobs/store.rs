//! The `job` table. Every function takes the writer's connection, except
//! [`get`], which also works on a read connection.

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::model::{JobId, JobKind, JobRecord, JobStatus, NewJob, Priority};

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// A job a worker has just taken.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Claimed {
    pub id: JobId,
    /// As stored; a kind this build doesn't know fails the job.
    pub kind: String,
    pub target: Option<serde_json::Value>,
    pub priority: Priority,
}

/// How a running job ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Ending {
    Done,
    Failed(String),
    Cancelled,
    /// The app is closing: the job goes back in the queue as if it had
    /// never started.
    Requeued,
}

/// Adds a queued job and returns its id.
pub(super) fn insert(conn: &Connection, job: &NewJob) -> rusqlite::Result<JobId> {
    let target = job.target.as_ref().map(|t| t.to_string());
    conn.execute(
        "INSERT INTO job (kind, target, priority) VALUES (?1, ?2, ?3)",
        params![job.kind.as_str(), target, job.priority.0],
    )?;
    Ok(JobId(conn.last_insert_rowid()))
}

/// Marks the next job running and returns it: the highest priority queued
/// job, oldest first. `None` if nothing is queued.
pub(super) fn claim_next(conn: &Connection) -> rusqlite::Result<Option<Claimed>> {
    let sql = format!(
        "UPDATE job SET status = 'running', started_at = {NOW}, attempts = attempts + 1
         WHERE id = (SELECT id FROM job WHERE status = 'queued'
                     ORDER BY priority DESC, id LIMIT 1)
         RETURNING id, kind, target, priority"
    );
    conn.query_row(&sql, [], |row| {
        Ok(Claimed {
            id: JobId(row.get(0)?),
            kind: row.get(1)?,
            target: json(row, 2)?,
            priority: Priority(row.get(3)?),
        })
    })
    .optional()
}

/// Records a running job's progress, 0 to 1.
pub(super) fn set_progress(conn: &Connection, id: JobId, progress: f64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE job SET progress = ?2 WHERE id = ?1 AND status = 'running'",
        params![id.0, progress.clamp(0.0, 1.0)],
    )?;
    Ok(())
}

/// Records how a running job ended.
pub(super) fn finish(conn: &Connection, id: JobId, ending: &Ending) -> rusqlite::Result<()> {
    let (status, error) = match ending {
        Ending::Done => ("done", None),
        Ending::Failed(e) => ("failed", Some(e.as_str())),
        Ending::Cancelled => ("cancelled", None),
        Ending::Requeued => {
            // Not an attempt: the job goes back as if it had never started.
            conn.execute(
                "UPDATE job SET status = 'queued', started_at = NULL, progress = NULL,
                                attempts = max(attempts - 1, 0)
                 WHERE id = ?1 AND status = 'running'",
                params![id.0],
            )?;
            return Ok(());
        }
    };
    let sql = format!(
        "UPDATE job SET status = ?2, error = ?3, finished_at = {NOW},
                        progress = CASE WHEN ?2 = 'done' THEN 1.0 ELSE progress END
         WHERE id = ?1 AND status = 'running'"
    );
    conn.execute(&sql, params![id.0, status, error])?;
    Ok(())
}

/// Cancels a job that hasn't started. False if it isn't queued (it's
/// running, finished or doesn't exist).
pub(super) fn cancel_queued(conn: &Connection, id: JobId) -> rusqlite::Result<bool> {
    let sql = format!(
        "UPDATE job SET status = 'cancelled', finished_at = {NOW}
         WHERE id = ?1 AND status = 'queued'"
    );
    Ok(conn.execute(&sql, params![id.0])? == 1)
}

/// Every queued job, in the order workers will take them.
pub(super) fn queued(conn: &Connection) -> rusqlite::Result<Vec<Claimed>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, target, priority FROM job WHERE status = 'queued'
         ORDER BY priority DESC, id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Claimed {
            id: JobId(row.get(0)?),
            kind: row.get(1)?,
            target: json(row, 2)?,
            priority: Priority(row.get(3)?),
        })
    })?;
    rows.collect()
}

/// One job, if it exists.
pub fn get(conn: &Connection, id: JobId) -> rusqlite::Result<Option<JobRecord>> {
    conn.query_row(
        "SELECT id, kind, target, priority, status, progress, error, attempts,
                created_at, started_at, finished_at
         FROM job WHERE id = ?1",
        params![id.0],
        |row| {
            let status: String = row.get(4)?;
            Ok(JobRecord {
                id: JobId(row.get(0)?),
                kind: JobKind::parse(&row.get::<_, String>(1)?),
                target: json(row, 2)?,
                priority: Priority(row.get(3)?),
                // The table's CHECK allows only the five statuses.
                status: JobStatus::parse(&status).ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(
                        4,
                        "status".into(),
                        rusqlite::types::Type::Text,
                    )
                })?,
                progress: row.get(5)?,
                error: row.get(6)?,
                attempts: row.get(7)?,
                created_at: row.get(8)?,
                started_at: row.get(9)?,
                finished_at: row.get(10)?,
            })
        },
    )
    .optional()
}

/// A JSON column, parsed. The table's CHECK keeps it valid JSON.
fn json(row: &Row, index: usize) -> rusqlite::Result<Option<serde_json::Value>> {
    let text: Option<String> = row.get(index)?;
    text.map(|t| {
        serde_json::from_str(&t).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Writer;
    use serde_json::json;

    fn open() -> (tempfile::TempDir, Writer) {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(dir.path(), "t.db")).unwrap();
        (dir, writer)
    }

    #[test]
    fn an_inserted_job_is_stored_queued_with_its_kind_target_and_priority() {
        let (_dir, w) = open();
        let job = NewJob::new(JobKind::Fingerprint)
            .target(json!({"file_id": 42}))
            .priority(Priority::USER);
        let stored = w
            .call(move |c| {
                let id = insert(c, &job)?;
                get(c, id)
            })
            .unwrap()
            .unwrap();
        assert_eq!(stored.kind, Some(JobKind::Fingerprint));
        assert_eq!(stored.target, Some(json!({"file_id": 42})));
        assert_eq!(stored.priority, Priority::USER);
        assert_eq!(stored.status, JobStatus::Queued);
        assert_eq!(stored.attempts, 0);
        assert_eq!(stored.progress, None);
        assert!(stored.started_at.is_none() && stored.finished_at.is_none());
    }

    #[test]
    fn claiming_takes_the_highest_priority_then_the_oldest() {
        let (_dir, w) = open();
        let order = w
            .call(|c| {
                let low = insert(
                    c,
                    &NewJob::new(JobKind::Hash).priority(Priority::BACKGROUND),
                )?;
                let normal_a = insert(c, &NewJob::new(JobKind::Hash))?;
                let high = insert(c, &NewJob::new(JobKind::Scan).priority(Priority::USER))?;
                let normal_b = insert(c, &NewJob::new(JobKind::Hash))?;
                let mut order = Vec::new();
                while let Some(job) = claim_next(c)? {
                    order.push(job.id);
                }
                Ok((order, vec![high, normal_a, normal_b, low]))
            })
            .unwrap();
        assert_eq!(order.0, order.1);
    }

    #[test]
    fn a_claimed_job_is_running_with_a_start_time_and_one_attempt() {
        let (_dir, w) = open();
        let job = w
            .call(|c| {
                let id = insert(c, &NewJob::new(JobKind::Analyze))?;
                assert_eq!(claim_next(c)?.map(|j| j.id), Some(id));
                assert_eq!(claim_next(c)?, None, "a running job is not claimed twice");
                get(c, id)
            })
            .unwrap()
            .unwrap();
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.attempts, 1);
        assert!(job.started_at.is_some());
    }

    #[test]
    fn each_ending_is_stored_and_satisfies_the_table_checks() {
        let (_dir, w) = open();
        let endings = [
            Ending::Done,
            Ending::Failed("disk full".into()),
            Ending::Cancelled,
            Ending::Requeued,
        ];
        let stored = w
            .call(move |c| {
                let mut stored = Vec::new();
                for ending in endings {
                    let id = insert(c, &NewJob::new(JobKind::Export))?;
                    claim_next(c)?;
                    set_progress(c, id, 0.5)?;
                    finish(c, id, &ending)?;
                    stored.push(get(c, id)?.unwrap());
                }
                Ok(stored)
            })
            .unwrap();
        let summary: Vec<_> = stored
            .iter()
            .map(|j| {
                (
                    j.status,
                    j.progress,
                    j.error.as_deref(),
                    j.finished_at.is_some(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (JobStatus::Done, Some(1.0), None, true),
                (JobStatus::Failed, Some(0.5), Some("disk full"), true),
                (JobStatus::Cancelled, Some(0.5), None, true),
                (JobStatus::Queued, None, None, false),
            ]
        );
        assert!(stored[3].started_at.is_none());
        assert_eq!(stored[3].attempts, 0, "a requeue isn't an attempt");
    }

    #[test]
    fn only_a_queued_job_can_be_cancelled_directly() {
        let (_dir, w) = open();
        w.call(|c| {
            let queued = insert(c, &NewJob::new(JobKind::Convert))?;
            let running = insert(c, &NewJob::new(JobKind::Convert))?;
            // Takes `queued` first (older), so start `running` by hand.
            c.execute(
                "UPDATE job SET status = 'running', started_at = 'x' WHERE id = ?1",
                [running.0],
            )?;
            assert!(!cancel_queued(c, running)?);
            assert!(cancel_queued(c, queued)?);
            assert!(!cancel_queued(c, queued)?, "already cancelled");
            assert!(!cancel_queued(c, JobId(9999))?);
            assert_eq!(get(c, queued)?.unwrap().status, JobStatus::Cancelled);
            assert_eq!(get(c, running)?.unwrap().status, JobStatus::Running);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn progress_is_clamped_to_between_zero_and_one() {
        let (_dir, w) = open();
        let seen = w
            .call(|c| {
                let id = insert(c, &NewJob::new(JobKind::Scan))?;
                claim_next(c)?;
                set_progress(c, id, 1.7)?;
                let high = get(c, id)?.unwrap().progress;
                set_progress(c, id, -3.0)?;
                Ok((high, get(c, id)?.unwrap().progress))
            })
            .unwrap();
        assert_eq!(seen, (Some(1.0), Some(0.0)));
    }

    #[test]
    fn a_kind_this_build_does_not_know_reads_back_as_none() {
        let (_dir, w) = open();
        let job = w
            .call(|c| {
                c.execute("INSERT INTO job (kind) VALUES ('teleport')", [])?;
                get(c, JobId(c.last_insert_rowid()))
            })
            .unwrap()
            .unwrap();
        assert_eq!(job.kind, None);
    }
}
