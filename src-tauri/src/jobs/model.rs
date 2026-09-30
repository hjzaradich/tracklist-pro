//! What a job is: its kind, target, priority and status.

use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;

/// A job's row id in the `job` table.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct JobId(pub i64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The kinds of background work (ROADMAP 0.1). Stored in `job.kind` as
/// [`JobKind::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Walk the music folders for audio files (1.1).
    Scan,
    /// Read the files' tags and audio properties (1.1 stage 2).
    Read,
    /// Hash a file's audio (`audio_hash`).
    Hash,
    /// Acoustic fingerprint (1.4).
    Fingerprint,
    /// Estimate BPM, key and energy (3.7).
    Analyze,
    /// Run the audio model (3.8).
    Embed,
    /// Make a Library copy in another format (2.6).
    Convert,
    /// Write an export, e.g. the rekordbox XML (1.9).
    Export,
    /// Read rekordbox's XML export into the snapshot (1.2).
    ReadRekordbox,
}

impl JobKind {
    /// Every kind, in the order above.
    pub const ALL: [JobKind; 9] = [
        JobKind::Scan,
        JobKind::Read,
        JobKind::Hash,
        JobKind::Fingerprint,
        JobKind::Analyze,
        JobKind::Embed,
        JobKind::Convert,
        JobKind::Export,
        JobKind::ReadRekordbox,
    ];

    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Scan => "scan",
            JobKind::Read => "read",
            JobKind::Hash => "hash",
            JobKind::Fingerprint => "fingerprint",
            JobKind::Analyze => "analyze",
            JobKind::Embed => "embed",
            JobKind::Convert => "convert",
            JobKind::Export => "export",
            JobKind::ReadRekordbox => "read_rekordbox",
        }
    }

    /// The kind stored as `name`, if there is one.
    pub fn parse(name: &str) -> Option<JobKind> {
        JobKind::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

impl fmt::Display for JobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a job is in its life. Stored in `job.status` as
/// [`JobStatus::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Waiting for a worker.
    Queued,
    /// A worker is doing it.
    Running,
    /// Finished its work.
    Done,
    /// Stopped with an error, kept in `job.error`.
    Failed,
    /// Stopped because it was cancelled.
    Cancelled,
}

impl JobStatus {
    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    /// The status stored as `name`, if there is one.
    pub fn parse(name: &str) -> Option<JobStatus> {
        [
            JobStatus::Queued,
            JobStatus::Running,
            JobStatus::Done,
            JobStatus::Failed,
            JobStatus::Cancelled,
        ]
        .into_iter()
        .find(|s| s.as_str() == name)
    }

    /// True once the job has stopped for good: done, failed or cancelled.
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled
        )
    }
}

/// How soon a job should run. Workers always take the highest-priority
/// queued job first, and the oldest among equals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Priority(pub i64);

impl Priority {
    /// Work nobody is waiting on, e.g. fingerprinting the whole collection.
    pub const BACKGROUND: Priority = Priority(-10);
    /// The default.
    pub const NORMAL: Priority = Priority(0);
    /// Work the user just asked for and is waiting on.
    pub const USER: Priority = Priority(10);
}

/// A job to enqueue.
#[derive(Debug, Clone, PartialEq)]
pub struct NewJob {
    pub kind: JobKind,
    /// What it works on, e.g. `{"music_folder_id": 3}`. Stored as JSON.
    pub target: Option<serde_json::Value>,
    pub priority: Priority,
}

impl NewJob {
    /// A job of `kind` with no target, at normal priority.
    pub fn new(kind: JobKind) -> NewJob {
        NewJob {
            kind,
            target: None,
            priority: Priority::NORMAL,
        }
    }

    /// Sets what the job works on.
    pub fn target(mut self, target: serde_json::Value) -> NewJob {
        self.target = Some(target);
        self
    }

    /// Sets the priority.
    pub fn priority(mut self, priority: Priority) -> NewJob {
        self.priority = priority;
        self
    }
}

/// A job as stored in the database.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRecord {
    pub id: JobId,
    /// `None` if the stored kind isn't one this build knows.
    pub kind: Option<JobKind>,
    pub target: Option<serde_json::Value>,
    pub priority: Priority,
    pub status: JobStatus,
    pub progress: Option<f64>,
    pub error: Option<String>,
    pub attempts: i64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_stored_name() {
        for kind in JobKind::ALL {
            assert_eq!(JobKind::parse(kind.as_str()), Some(kind));
            // The stored name and the IPC name are the same word.
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::Value::String(kind.as_str().into())
            );
        }
        assert_eq!(JobKind::parse("Scan"), None);
        assert_eq!(JobKind::parse(""), None);
    }

    #[test]
    fn the_kinds_are_the_roadmap_ones_plus_reading_files_and_rekordbox() {
        let names: Vec<_> = JobKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "scan",
                "read",
                "hash",
                "fingerprint",
                "analyze",
                "embed",
                "convert",
                "export",
                "read_rekordbox"
            ]
        );
    }

    #[test]
    fn every_status_round_trips_and_only_the_last_three_are_finished() {
        use JobStatus::*;
        for (status, finished) in [
            (Queued, false),
            (Running, false),
            (Done, true),
            (Failed, true),
            (Cancelled, true),
        ] {
            assert_eq!(JobStatus::parse(status.as_str()), Some(status));
            assert_eq!(status.is_finished(), finished, "{status:?}");
        }
    }

    #[test]
    fn user_work_outranks_normal_work_which_outranks_background_work() {
        assert!(Priority::USER > Priority::NORMAL);
        assert!(Priority::NORMAL > Priority::BACKGROUND);
        assert_eq!(Priority::default(), Priority::NORMAL);
    }
}
