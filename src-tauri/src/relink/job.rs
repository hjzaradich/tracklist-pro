//! The relink job: runs [`super::relink`] in the background.
//!
//! Queued after every rekordbox read, and wherever else files may have
//! turned up (after scans, 1aC-8), always through [`request`], which never
//! queues a second one while one is waiting: one run sees every change
//! made before it starts. If one is already running, another is queued, so
//! changes made while it runs are seen too.

use std::sync::Mutex;

use tauri::State;

use super::{identities, relink, Mounted, Summary};
use crate::db::{DbError, Writer};
use crate::ipc::IpcError;
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, JobQueue, NewJob, Priority};
use crate::paths::Volumes;

/// A relink job. Background priority: it reads only the database and
/// finishes quickly, but nobody is waiting on it.
pub fn relink_job() -> NewJob {
    NewJob::new(JobKind::Relink).priority(Priority::BACKGROUND)
}

/// Held while checking for a waiting relink job and queuing one, so two
/// callers can't both find none and both queue.
static QUEUING: Mutex<()> = Mutex::new(());

/// Queues a relink job through `enqueue`, unless one is already waiting;
/// then returns that one's id. A running one doesn't count: it may have
/// read the database before the caller's changes.
pub fn request<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<JobId, E> {
    let _one_at_a_time = QUEUING.lock().unwrap_or_else(|e| e.into_inner());
    let waiting = writer.call(|c| {
        use rusqlite::OptionalExtension;
        c.query_row(
            "SELECT id FROM job WHERE kind = ?1 AND status = 'queued' ORDER BY id LIMIT 1",
            [JobKind::Relink.as_str()],
            |r| r.get::<_, i64>(0),
        )
        .optional()
    })?;
    match waiting {
        Some(id) => Ok(JobId(id)),
        None => enqueue(relink_job()),
    }
}

/// Matches rekordbox tracks to files again, in the background. Returns the
/// job's id; if a relink is already waiting to run, returns that one.
#[tauri::command(async)]
#[specta::specta]
pub fn relink_rekordbox_tracks(
    jobs: State<'_, JobQueue>,
    writer: State<'_, Writer>,
) -> Result<JobId, IpcError> {
    Ok(request(&writer, |job| jobs.enqueue(job))?)
}

/// The relink job's handler.
pub struct Relinker<V> {
    /// The volumes mounted now, asked once per job.
    volumes: Box<dyn Fn() -> V + Send + Sync>,
    on_summary: Option<Box<dyn Fn(Summary) + Send + Sync>>,
}

impl<V: Volumes + 'static> Relinker<V> {
    pub fn new(volumes: impl Fn() -> V + Send + Sync + 'static) -> Relinker<V> {
        Relinker {
            volumes: Box::new(volumes),
            on_summary: None,
        }
    }

    /// Hands each run's [`Summary`] to `hook` as well as to the log.
    #[cfg(test)]
    pub(crate) fn on_summary(mut self, hook: impl Fn(Summary) + Send + Sync + 'static) -> Self {
        self.on_summary = Some(Box::new(hook));
        self
    }
}

/// The relink job's handler for the app: asks Windows which volumes are
/// mounted at the start of each job.
pub fn relinker() -> impl JobHandler {
    Relinker::new(crate::scan::system_volumes)
}

impl<V: Volumes + 'static> JobHandler for Relinker<V> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let known = job.writer().call(|c| identities(c))?;
        let mounted = Mounted::ask(&known, &(self.volumes)());
        job.check_cancelled()?;
        // One transaction: a cancel from here on changes nothing.
        let summary = job.writer().call(move |c| relink(c, &mounted))?;
        let _ = job.progress(1.0);
        eprintln!("relink: {summary:?}");
        if let Some(hook) = &self.on_summary {
            hook(summary);
        }
        Ok(())
    }
}
