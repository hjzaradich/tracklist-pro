//! What the frontend hears about jobs: an event per change, and a snapshot
//! to start from (0E-3).

use serde::{Deserialize, Serialize};
use specta::Type;

use super::model::{JobId, JobKind, JobStatus};

/// A job changed: it was queued, started, made progress or finished.
///
/// Every update carries `seq`, which rises by one with each change the app
/// makes to any job. A listener that starts from an [`ActivitySnapshot`]
/// applies only the updates newer than the snapshot's `seq`, and for each
/// job only updates newer than the last it applied, so a job never goes
/// back to an older state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JobUpdate {
    pub seq: u64,
    pub id: JobId,
    pub kind: JobKind,
    pub status: JobStatus,
    /// 0 to 1, once the job has reported any.
    pub progress: Option<f64>,
    /// Higher runs first.
    pub priority: i64,
}

/// Job updates, sent together: the newest update for each job that
/// changed since the last batch, in `seq` order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct JobUpdates(pub Vec<JobUpdate>);

/// Every queued and running job, and the `seq` of the last update already
/// reflected in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySnapshot {
    pub seq: u64,
    /// Running jobs first, then queued; each in the order workers take them.
    pub jobs: Vec<JobUpdate>,
}

/// What asking to cancel a job did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CancelOutcome {
    /// It hadn't started, and now never will.
    Cancelled,
    /// It's running and will stop at its next check.
    Stopping,
    /// It had already finished, or there's no such job.
    NotActive,
}
