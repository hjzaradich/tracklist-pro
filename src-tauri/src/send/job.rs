//! The send's two jobs: prepare (read rekordbox again, build the
//! preflight) and write (the user's go).

use std::fs;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use tauri::{AppHandle, Manager, Runtime};

use super::file::{send_path, write_and_record, WriteError};
use super::preflight::{review, Reviewed};
use super::{SendFailure, SendFlow, Sent};
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::paths::Volumes;
use crate::rekordbox::source::XmlReader;
use crate::rekordbox_write::{record_send, SentTrack};
use crate::relink::{self, Mounted};

/// The prepare job: reads the export at `path` again and builds the
/// preflight. The user is waiting on it.
pub fn prepare_job(path: &str) -> NewJob {
    NewJob::new(JobKind::Export)
        .target(serde_json::json!({ "send": "prepare", "path": path }))
        .priority(Priority::USER)
}

/// The write job: the user's go for the preflight `token` names.
pub fn write_job(token: &str, confirmed: bool) -> NewJob {
    NewJob::new(JobKind::Export)
        .target(serde_json::json!({ "send": "write", "token": token, "confirmed": confirmed }))
        .priority(Priority::USER)
}

/// How a send is recorded once its file is in place.
type Record = dyn Fn(&mut Connection, &[SentTrack]) -> rusqlite::Result<()> + Send + Sync;

/// The send jobs' handler.
pub struct Sender<V> {
    flow: SendFlow,
    reader: XmlReader,
    /// The volumes mounted now, asked when a step needs them.
    volumes: Arc<dyn Fn() -> V + Send + Sync>,
    /// [`record_send`]; tests put a failing one here.
    record: Arc<Record>,
}

impl<V: Volumes + 'static> Sender<V> {
    pub fn new(flow: SendFlow, volumes: impl Fn() -> V + Send + Sync + 'static) -> Sender<V> {
        Sender {
            flow,
            reader: XmlReader::default(),
            volumes: Arc::new(volumes),
            record: Arc::new(record_send),
        }
    }

    /// Records sends with `record` instead of [`record_send`].
    #[cfg(test)]
    pub(crate) fn recording_with(
        mut self,
        record: impl Fn(&mut Connection, &[SentTrack]) -> rusqlite::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.record = Arc::new(record);
        self
    }
}

/// The send jobs' handler for the app. The send flow must already be in
/// Tauri's state.
pub fn sender<R: Runtime>(app: &AppHandle<R>) -> impl JobHandler {
    let flow = app.state::<SendFlow>().inner().clone();
    Sender::new(flow, crate::scan::system_volumes)
}

/// Why a step stopped: what the checklist shows, and the detail for the
/// job's own record.
struct Stop {
    failure: SendFailure,
    detail: String,
    /// Whether the preflight is no longer good for a send.
    drop_preflight: bool,
}

impl Stop {
    fn new(failure: SendFailure, detail: impl std::fmt::Display) -> Stop {
        Stop {
            failure,
            detail: detail.to_string(),
            drop_preflight: false,
        }
    }

    fn dropping_preflight(mut self) -> Stop {
        self.drop_preflight = true;
        self
    }

    fn cancelled() -> Stop {
        Stop::new(SendFailure::Cancelled, "cancelled")
    }

    fn internal(detail: impl std::fmt::Display) -> Stop {
        Stop::new(SendFailure::Internal, detail)
    }
}

impl From<crate::db::DbError> for Stop {
    fn from(e: crate::db::DbError) -> Stop {
        Stop::internal(e)
    }
}

impl<V: Volumes + 'static> JobHandler for Sender<V> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let _one_at_a_time = self.flow.running.lock().unwrap_or_else(|e| e.into_inner());
        let step = job
            .target()
            .and_then(|t| t.get("send"))
            .and_then(|s| s.as_str());
        let done = match step {
            Some("prepare") => self.prepare(job),
            Some("write") => self.write(job),
            _ => Err(Stop::internal("the job names no send step")),
        };
        let Err(stop) = done else {
            return Ok(());
        };
        self.flow.end(|steps| {
            steps.failure = Some(stop.failure);
            if stop.drop_preflight {
                steps.review = None;
            }
        });
        Err(match stop.failure {
            SendFailure::Cancelled => JobError::Cancelled,
            _ => JobError::failed(stop.detail),
        })
    }
}

impl<V: Volumes + 'static> Sender<V> {
    /// Reads the export again, matches rekordbox's tracks to files, and
    /// builds the preflight. Nothing made before this step survives it.
    fn prepare(&self, job: &JobContext) -> Result<(), Stop> {
        self.flow.start_over();
        // The read job itself, run here: it reads the file the job's
        // target names, and records a failure for the rekordbox source.
        self.reader.run(job).map_err(|e| match e {
            JobError::Cancelled => Stop::cancelled(),
            JobError::Failed(detail) => Stop::new(SendFailure::ReadFailed, detail),
        })?;
        // The read replaced every rekordbox row, unmatched. Match them to
        // files now: a track rekordbox has must never go out as new.
        let known = job.writer().call(|c| relink::identities(c))?;
        let mounted = Mounted::ask(&known, &(self.volumes)());
        if job.is_cancelled() {
            return Err(Stop::cancelled());
        }
        job.writer().call(move |c| relink::relink(c, &mounted))?;
        let reviewed = self
            .review(job)?
            .ok_or_else(|| Stop::internal("the read left no record"))?;
        self.flow
            .end(|steps| steps.review = Some(reviewed.preflight));
        Ok(())
    }

    fn review(&self, job: &JobContext) -> Result<Option<Reviewed>, Stop> {
        let volumes = self.volumes.clone();
        Ok(job.writer().call(move |c| review(c, &volumes()))?)
    }

    /// The go: builds the send again, checks it's the reviewed one, writes
    /// the file and records the send.
    fn write(&self, job: &JobContext) -> Result<(), Stop> {
        self.flow.start_write();
        let target = job.target();
        let token = target
            .and_then(|t| t.get("token"))
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        let confirmed = target
            .and_then(|t| t.get("confirmed"))
            .and_then(|c| c.as_bool())
            .unwrap_or(false);

        let Some(reviewed) = self.flow.preflight().filter(|p| p.token == token) else {
            return Err(Stop::new(
                SendFailure::NoPreflight,
                "no preflight with that token",
            ));
        };
        if !reviewed.can_send {
            return Err(Stop::new(
                SendFailure::NotSendable,
                "the preflight can't be sent",
            ));
        }
        if reviewed.needs_confirm && !confirmed {
            return Err(Stop::new(
                SendFailure::NotConfirmed,
                "the send needs a confirm",
            ));
        }
        // rekordbox saved the export again since it was read: the read is
        // old now.
        if modified_ms(&reviewed.export.path) != Some(reviewed.export.modified_ms) {
            return Err(
                Stop::new(SendFailure::ExportChanged, "the export changed on disk")
                    .dropping_preflight(),
            );
        }
        let now = self.review(job)?;
        let Some(now) = now.filter(|n| n.preflight.export.read_at == reviewed.export.read_at)
        else {
            return Err(
                Stop::new(SendFailure::ExportChanged, "rekordbox was read again")
                    .dropping_preflight(),
            );
        };
        if now.preflight.token != reviewed.token {
            return Err(Stop::new(
                SendFailure::LibraryChanged,
                "the send differs from the preflight",
            )
            .dropping_preflight());
        }
        let Some(outgoing) = now.outgoing else {
            return Err(Stop::internal("a sendable preflight has no file"));
        };
        if job.is_cancelled() {
            return Err(Stop::cancelled());
        }

        let outgoing = Arc::new(outgoing);
        let dest = send_path(self.flow.guard());
        let at = write_and_record(self.flow.guard(), &dest, &outgoing, || {
            let record = self.record.clone();
            let outgoing = outgoing.clone();
            job.writer().call(move |c| {
                record(c, &outgoing.sent)?;
                c.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
                    r.get::<_, String>(0)
                })
            })
        })
        .map_err(|e| match e {
            WriteError::Write(e) => Stop::new(SendFailure::CantWrite, e),
            WriteError::Record { error, .. } => Stop::new(SendFailure::CantRecord, error),
        })?;
        // One preflight, one send: the next send starts with a new read.
        self.flow.end(|steps| {
            steps.review = None;
            steps.sent = Some(Sent {
                at,
                new_tracks: reviewed.new_tracks,
                known_tracks: reviewed.known_tracks,
            });
        });
        let _ = job.progress(1.0);
        Ok(())
    }
}

/// The export's modified time now, as the read records it (milliseconds
/// since the Unix epoch). `None` if the file can't be found.
fn modified_ms(path: &str) -> Option<i64> {
    let path = std::path::Path::new(path);
    #[cfg(windows)]
    let path = crate::paths::verbatim_absolute(path).ok()?;
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),
    )
}
