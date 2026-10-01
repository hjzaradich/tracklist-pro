//! The attach job: runs [`super::attach`] in the background.
//!
//! Asked for after every relink run and every grouping run, always through
//! [`request`], which never queues a second one while one is waiting: one
//! run sees every change made before it starts. If one is already running,
//! another is queued, so changes made while it runs are seen too (the same
//! rule as [`crate::relink::request`]).

use std::sync::Mutex;

use super::{any_to_attach, attach, Summary};
use crate::db::{DbError, Writer};
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, NewJob, Priority};

/// An attach job. Background priority: it reads only the database and
/// finishes quickly, but nobody is waiting on it.
pub fn attach_job() -> NewJob {
    NewJob::new(JobKind::Attach).priority(Priority::BACKGROUND)
}

/// Held while checking for a waiting attach job and queuing one, so two
/// callers can't both find none and both queue.
static QUEUING: Mutex<()> = Mutex::new(());

/// Queues an attach job through `enqueue`, unless one is already waiting;
/// then returns that one's id. A running one doesn't count: it may have
/// read the database before the caller's changes. Returns `None` without
/// queuing when there is no rekordbox data to attach or to take back
/// ([`any_to_attach`]), so a library that doesn't use rekordbox never
/// queues one.
pub fn request<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<Option<JobId>, E> {
    let _one_at_a_time = QUEUING.lock().unwrap_or_else(|e| e.into_inner());
    let (needed, waiting) = writer.call(|c| {
        use rusqlite::OptionalExtension;
        let waiting = c
            .query_row(
                "SELECT id FROM job WHERE kind = ?1 AND status = 'queued' ORDER BY id LIMIT 1",
                [JobKind::Attach.as_str()],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        Ok((any_to_attach(c)?, waiting))
    })?;
    match waiting {
        Some(id) => Ok(Some(JobId(id))),
        None if needed => enqueue(attach_job()).map(Some),
        None => Ok(None),
    }
}

/// The attach job's handler.
#[derive(Default)]
pub struct Attacher {
    on_summary: Option<Box<dyn Fn(Summary) + Send + Sync>>,
}

impl Attacher {
    /// Hands each run's [`Summary`] to `hook` as well as to the log.
    #[cfg(test)]
    pub(crate) fn on_summary(mut self, hook: impl Fn(Summary) + Send + Sync + 'static) -> Self {
        self.on_summary = Some(Box::new(hook));
        self
    }
}

/// The attach job's handler for the app.
pub fn attacher() -> impl JobHandler {
    Attacher::default()
}

impl JobHandler for Attacher {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        job.check_cancelled()?;
        // One transaction: all or nothing.
        let summary = job.writer().call(attach)?;
        eprintln!("attach job {} done: {summary:?}", job.id());
        if let Some(hook) = &self.on_summary {
            hook(summary);
        }
        job.progress(1.0)
    }
}
