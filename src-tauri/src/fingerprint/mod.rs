//! Acoustic fingerprints: stage 3 of the scan (ROADMAP 1.1, 1.4; 1aB-7).
//!
//! Every present file gets a full-track `rusty-chromaprint` fingerprint in
//! `file.fingerprint`, computed in the background ([`job`]), visible
//! tracks first ([`FirstUp`]). Duplicate and cut decisions (1b) compare
//! these full-track fingerprints; a short window may only ever find
//! candidates (1.4).
//!
//! - [`decode`] decodes with Symphonia (MP3, AAC and ALAC in MP4/M4A,
//!   FLAC, WAV, AIFF, Ogg Vorbis, CAF) and fingerprints as it goes, so
//!   memory stays flat on a two-hour mix. Opus isn't among Symphonia's
//!   codecs: those files get `unsupported_codec`.
//! - [`stored`] is the blob format, with a version marker, and the
//!   comparison, which refuses fingerprints of different versions.
//! - [`ledger`] decides which files are due and keeps why a file has no
//!   fingerprint.
//!
//! Everything here only reads audio files. The only writes go to the
//! database, through the [`crate::db::Writer`].

pub mod decode;
mod job;
pub mod ledger;
pub mod stored;

pub use decode::Unfingerprintable;
pub(crate) use job::lower_priority;
pub use job::{default_threads, fingerprint_job, Fingerprinter, FirstUp};
pub use ledger::Outcome;
pub use stored::{compare, BlobError, CompareError, Comparison, Fingerprint, VERSION};

use tauri::{AppHandle, Manager, Runtime, State};

use crate::ipc::IpcError;
use crate::jobs::{JobHandler, JobId, JobKind, JobQueue, JobStatus, NewJob, Priority};

/// The fingerprint job's handler for the app, on the shared [`FirstUp`]
/// line ([`shared_first`]).
pub fn fingerprinter<R: Runtime>(app: &AppHandle<R>) -> impl JobHandler {
    Fingerprinter::new(crate::scan::system_volumes, shared_first(app))
}

/// The [`FirstUp`] line the app's jobs share, kept in Tauri's state for
/// [`fingerprint_first`]: made on the first call, the same one after. Its
/// thread budget is shared too: the read job borrows from it (1aC-12).
pub fn shared_first<R: Runtime>(app: &AppHandle<R>) -> FirstUp {
    if let Some(first) = app.try_state::<FirstUp>() {
        return first.inner().clone();
    }
    let first = FirstUp::default();
    app.manage(first.clone());
    first
}

/// Fingerprints every file that's due, in the background. Returns the
/// job's id; if a fingerprint job over every file is already queued or
/// running, returns that one instead of starting another.
#[tauri::command(async)]
#[specta::specta]
pub fn fingerprint_files(jobs: State<'_, JobQueue>) -> Result<JobId, IpcError> {
    Ok(start(&jobs, fingerprint_job(None))?)
}

/// Fingerprints `file_ids` before anything else: the tracks on screen.
/// A running fingerprint job takes them next; if none is running, a job
/// for just these starts at once, and its id is returned.
#[tauri::command(async)]
#[specta::specta]
pub fn fingerprint_first(
    jobs: State<'_, JobQueue>,
    first: State<'_, FirstUp>,
    file_ids: Vec<i64>,
) -> Result<Option<JobId>, IpcError> {
    Ok(raise(&jobs, &first, file_ids)?)
}

/// [`fingerprint_files`], without Tauri.
pub fn start(jobs: &JobQueue, job: NewJob) -> Result<JobId, crate::db::DbError> {
    if job.target.is_none() {
        let active = jobs.activity()?.jobs.into_iter().find(|j| {
            j.kind == JobKind::Fingerprint
                && matches!(j.status, JobStatus::Queued | JobStatus::Running)
                && j.priority == Priority::BACKGROUND.0
        });
        if let Some(active) = active {
            return Ok(active.id);
        }
    }
    jobs.enqueue(job)
}

/// [`fingerprint_first`], without Tauri.
pub fn raise(
    jobs: &JobQueue,
    first: &FirstUp,
    file_ids: Vec<i64>,
) -> Result<Option<JobId>, crate::db::DbError> {
    if file_ids.is_empty() || first.raise(&file_ids) {
        return Ok(None);
    }
    let job = fingerprint_job(Some(file_ids)).priority(Priority::USER);
    Ok(Some(jobs.enqueue(job)?))
}

#[cfg(test)]
mod tests;
