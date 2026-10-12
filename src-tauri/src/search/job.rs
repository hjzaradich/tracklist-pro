//! The `index` job: keeps the search index current.

use std::sync::Arc;

use super::settle::{forget_everything, settle_page};
use super::pending;
use crate::db::{DbError, Writer};
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, NewJob, Priority};
use crate::paths::Volumes;

/// Tracks worked out per transaction.
const PAGE: usize = 500;

/// The target key that asks a job to throw the index away and work every
/// row out again first.
const REBUILD_KEY: &str = "rebuild";

/// An index job: settles every track that is due. `rebuild` first empties
/// the index and makes every track due.
pub fn index_job(rebuild: bool) -> NewJob {
    let job = NewJob::new(JobKind::Index).priority(Priority::BACKGROUND);
    match rebuild {
        true => job.target(serde_json::json!({ REBUILD_KEY: true })),
        false => job,
    }
}

/// Queues an index job if any track is due, unless one is already queued (a
/// running one is asked to run once more, as the scan stages are). Called by
/// the scan chain, the rekordbox read, the relink job and the startup check;
/// a screen asks for it when [`pending`] says the index is behind.
pub fn request<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    if writer.call(|c| pending(c))? {
        crate::scan::chain::queue_once(writer, index_job(false), enqueue)?;
    }
    Ok(())
}

/// Whether a job of a higher priority than the index job's is waiting.
fn higher_priority_waiting(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM job WHERE status = 'queued' AND priority > ?1)",
    )?
    .query_row([Priority::BACKGROUND.0], |r| r.get(0))
}

/// The handler of the `index` job.
pub struct Indexer<V, F> {
    volumes: F,
    page: usize,
    _volumes: std::marker::PhantomData<fn() -> V>,
}

/// The handler with the system's volumes, as the app registers it.
pub fn indexer() -> impl JobHandler {
    indexer_with(crate::scan::system_volumes, PAGE)
}

/// The handler with `volumes` (asked once per job) and `page` tracks per
/// transaction.
pub fn indexer_with<V, F>(volumes: F, page: usize) -> impl JobHandler
where
    V: Volumes + Send + Sync + 'static,
    F: Fn() -> V + Send + Sync + 'static,
{
    Indexer {
        volumes,
        page,
        _volumes: std::marker::PhantomData,
    }
}

impl<V, F> JobHandler for Indexer<V, F>
where
    V: Volumes + Send + Sync + 'static,
    F: Fn() -> V + Send + Sync + 'static,
{
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let writer = job.writer();
        let rebuild = job
            .target()
            .and_then(|t| t.get(REBUILD_KEY))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if rebuild {
            writer.call(forget_everything)?;
        }
        let volumes = Arc::new((self.volumes)());
        let page = self.page;
        let (mut done, mut left) = (0usize, writer.call(|c| due(c))?);
        while left > 0 {
            job.check_cancelled()?;
            if writer.call(|c| higher_priority_waiting(c))? {
                // What was written stays; one more run, after the waiting
                // job, does the rest.
                crate::scan::chain::queue_once(writer, index_job(false), |j| job.enqueue(j))?;
                return Ok(());
            }
            let volumes = Arc::clone(&volumes);
            let settled = writer.call(move |c| settle_page(c, &*volumes, page))?;
            done += settled;
            left = writer.call(|c| due(c))?;
            if settled == 0 && left > 0 {
                // Only rows the page step turned into tracks: look again.
                continue;
            }
            job.progress(done as f64 / (done + left).max(1) as f64)?;
        }
        Ok(())
    }
}

/// How many tracks are due (the rekordbox mark counts as one).
fn due(conn: &rusqlite::Connection) -> rusqlite::Result<usize> {
    conn.prepare_cached("SELECT count(*) FROM search_due")?
        .query_row([], |r| r.get::<_, i64>(0))
        .map(|n| n as usize)
}
