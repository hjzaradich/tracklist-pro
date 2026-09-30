//! The job commands the frontend can call. Registered in `ipc.rs`.
//!
//! They're async, so Tauri runs them off the main thread, like the thread
//! that sends job events; an event sent from the main thread would jump
//! ahead of events already waiting to go out.

use tauri::State;

use crate::ipc::IpcError;

use super::events::{ActivitySnapshot, CancelOutcome};
use super::model::JobId;
use super::queue::JobQueue;

/// Every queued and running job, for the Activity status to start from.
#[tauri::command(async)]
#[specta::specta]
pub fn activity(jobs: State<'_, JobQueue>) -> Result<ActivitySnapshot, IpcError> {
    Ok(jobs.activity()?)
}

/// Cancels a queued or running job.
#[tauri::command(async)]
#[specta::specta]
pub fn cancel_job(jobs: State<'_, JobQueue>, id: JobId) -> Result<CancelOutcome, IpcError> {
    Ok(jobs.cancel(id)?)
}
