//! Provisional grouping: one file → one track (1aC-2; ROADMAP 1.4, 1.8, §2).
//!
//! Every present file belongs to exactly one track (`recording`). Files
//! that share a non-NULL `audio_hash` (an exact audio match: the same
//! audio frames, whatever their tags say) share a track; every other file
//! has a track of its own. A file with no `audio_hash` is never merged.
//!
//! It's one step you can run again at any time ([`regroup`]): a new file
//! gets a track, a file whose audio changed moves to the track its audio
//! belongs to, and a track left with no files (and nothing else pointing
//! at it) is deleted. Real grouping, with fingerprints and versions,
//! replaces this in 1b (1bC-1); the rule lives in [`plan`], a pure function
//! that step swaps out.
//!
//! - It only touches the database. No audio file is read or written.
//! - It runs as a background job ([`group_job`]), quick enough to run
//!   after every hash stage: `scan::chain` queues it once the hashes are
//!   done (or none were due), and after a walk that found files it has no
//!   read for ([`any_ungrouped`]).
//! - Removing a music folder lets go of its files first
//!   ([`release_folder_files`]), or their track rows would refuse it.

pub mod plan;
mod store;

pub use store::{any_ungrouped, regroup, release_folder_files, Summary};

use tauri::State;

use crate::db::Writer;
use crate::ipc::IpcError;
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, JobQueue, NewJob, Priority};

/// A grouping job, at background priority: nobody is waiting on it.
pub fn group_job() -> NewJob {
    NewJob::new(JobKind::Group).priority(Priority::BACKGROUND)
}

/// The grouping job's handler.
#[derive(Default)]
pub struct Grouper {
    on_summary: Option<Box<dyn Fn(Summary) + Send + Sync>>,
}

impl Grouper {
    /// Hands each run's [`Summary`] to `hook` as well as to the log.
    #[cfg(test)]
    pub(crate) fn on_summary(mut self, hook: impl Fn(Summary) + Send + Sync + 'static) -> Self {
        self.on_summary = Some(Box::new(hook));
        self
    }
}

impl JobHandler for Grouper {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        job.check_cancelled()?;
        // One transaction: the pass is quick, and all or nothing.
        let summary = job.writer().call(regroup)?;
        eprintln!("group job {} done: {summary:?}", job.id());
        if let Some(hook) = &self.on_summary {
            hook(summary);
        }
        job.progress(1.0)
    }
}

/// Groups every file into tracks, in the background. Returns the job's id;
/// if a grouping job is already queued, returns that one instead of
/// starting another. If one is running (it may have loaded the files
/// already), it's asked to run once more when it ends, like the chained
/// stages are ([`crate::scan::chain::queue_once`]).
#[tauri::command(async)]
#[specta::specta]
pub fn group_files(
    jobs: State<'_, JobQueue>,
    writer: State<'_, Writer>,
) -> Result<JobId, IpcError> {
    Ok(start(&jobs, &writer)?)
}

/// [`group_files`], without Tauri.
pub fn start(jobs: &JobQueue, writer: &Writer) -> Result<JobId, crate::db::DbError> {
    crate::scan::chain::queue_once(writer, group_job(), |j| jobs.enqueue(j))
}

#[cfg(test)]
mod tests;
