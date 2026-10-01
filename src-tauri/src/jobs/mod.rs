//! Background jobs: Activity in the UI (ROADMAP 0.1, §1.2).
//!
//! Every long operation goes through here (CLAUDE.md). A job has a
//! [`JobKind`], an optional JSON target and a [`Priority`]. It's stored in
//! the `job` table, so it outlives the process; [`JobQueue::enqueue`]
//! returns its id. A pool of worker threads runs the highest-priority
//! queued job first, off the DB writer thread. A job that fails or panics
//! is marked failed with its error, and the pool carries on.
//!
//! Every change is sent to the frontend as a [`JobUpdate`], batched into
//! [`JobUpdates`] events, and [`JobQueue::activity`] gives a snapshot to
//! start from. A queued or running job can be cancelled.
//!
//! Keep jobs coarse: one per folder or per batch of files, not one per
//! file.
//!
//! Crash resume (jobs left `running` by a crash) is 1cA-13.

pub mod commands;
mod dispatch;
mod events;
mod model;
mod queue;
pub mod store;

use tauri::{AppHandle, Runtime};
use tauri_specta::Event;

use crate::db::{DbError, Writer};

pub use dispatch::{coalesce, EventSink, BATCH_WINDOW};
pub use events::{ActivitySnapshot, CancelOutcome, JobUpdate, JobUpdates};
pub use model::{JobId, JobKind, JobRecord, JobStatus, NewJob, Priority};
pub use queue::{
    default_workers, JobContext, JobError, JobHandler, JobQueue, JobQueueBuilder, PROGRESS_STEP,
    SHUTDOWN_TIMEOUT,
};

/// Starts the app's job queue, sending every update to the webview.
///
/// The handlers for each kind are registered here as the features that
/// need them arrive (scan in 1a, fingerprint in 1b, …).
pub fn start<R: Runtime>(app: &AppHandle<R>, writer: Writer) -> Result<JobQueue, DbError> {
    JobQueue::builder(writer)
        .on_updates(emitter(app))
        // The scan stages are chained (scan::chain): a finished walk
        // queues the read of its folders, a finished read their hashes,
        // and finished hashes the fingerprints; a stage asked for again
        // while it ran runs once more.
        .handler(
            JobKind::Scan,
            crate::scan::chain::after_walk(crate::scan::walker(app)),
        )
        .handler(
            JobKind::ReadRekordbox,
            crate::rekordbox::source::XmlReader::default(),
        )
        .handler(
            JobKind::Hash,
            crate::scan::chain::after_hash(crate::hash::hasher()),
        )
        .handler(
            JobKind::Read,
            crate::scan::chain::after_read(crate::read::reader(crate::fingerprint::shared_first(
                app,
            ))),
        )
        .handler(
            JobKind::Fingerprint,
            crate::scan::chain::after_fingerprint(crate::fingerprint::fingerprinter(app)),
        )
        .handler(JobKind::Relink, crate::relink::relinker())
        .handler(JobKind::Attach, crate::attach::attacher())
        .handler(
            JobKind::Group,
            crate::scan::chain::after_group(crate::grouping::Grouper::default()),
        )
        .start()
}

/// Sends each batch of job updates to the webview as a typed
/// [`JobUpdates`] event.
pub fn emitter<R: Runtime>(app: &AppHandle<R>) -> impl Fn(&[JobUpdate]) + Send + 'static {
    let app = app.clone();
    move |updates| {
        // Only fails if the app is closing; the snapshot command covers
        // anything a webview missed.
        let _ = JobUpdates(updates.to_vec()).emit(&app);
    }
}

#[cfg(test)]
mod tests;
