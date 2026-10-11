//! The send's two jobs: prepare (read rekordbox again, build the
//! preflight) and write (the user's go).

use std::fs;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use tauri::{AppHandle, Manager, Runtime};

use super::file::{send_path, write_and_record, WriteError};
use super::preflight::{may_send_from_this_export, no_file_at_location, review, Reviewed};
use super::{SendFailure, SendFlow, SendStep, Sent};
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::paths::Volumes;
use crate::rekordbox::source::XmlReader;
use crate::rekordbox_write::{record_send, SentPath, SentTrack};
use crate::relink::{self, Mounted};

impl SendFlow {
    /// The prepare job: reads the export at `path` again and builds the
    /// preflight. The user is waiting on it. It only runs in this run of
    /// the app.
    pub fn prepare_job(&self, path: &str) -> NewJob {
        NewJob::new(JobKind::Export)
            .target(serde_json::json!({ "send": "prepare", "run": &*self.run, "path": path }))
            .priority(Priority::USER)
    }

    /// The write job: the user's go for the preflight `token` names, with
    /// the explicit confirm and the answer about imports since the export
    /// (see `write_send`). Both belong to this one job: they're kept only
    /// in its own row, and no later send reads them.
    /// It only runs in this run of the app.
    pub fn write_job(
        &self,
        token: &str,
        confirmed: bool,
        imported_since_export: Option<bool>,
    ) -> NewJob {
        NewJob::new(JobKind::Export)
            .target(serde_json::json!({
                "send": "write", "run": &*self.run, "token": token, "confirmed": confirmed,
                "importedSinceExport": imported_since_export,
            }))
            .priority(Priority::USER)
    }
}

/// Ends every send job an earlier run of the app left queued or running
/// (they're stored, and a job stopped by the app closing goes back in the
/// queue). Called at startup, before the job queue: a send step only ever
/// runs because of a click in this run. An interrupted one is dropped,
/// and the user starts again. Returns how many were dropped.
pub fn drop_unfinished_jobs(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE job SET status = 'cancelled', progress = NULL,
                        finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE kind = ?1 AND status IN ('queued', 'running')
           AND json_extract(target, '$.send') IS NOT NULL",
        [JobKind::Export.as_str()],
    )
}

/// How a send is recorded once its file is in place.
type Record =
    dyn Fn(&mut Connection, &[SentTrack], &[SentPath]) -> rusqlite::Result<()> + Send + Sync;

/// The send jobs' handler.
pub struct Sender<V> {
    flow: SendFlow,
    reader: XmlReader,
    /// The volumes mounted now, asked when a step needs them.
    volumes: Arc<dyn Fn() -> V + Send + Sync>,
    /// [`record_send`]; tests put a failing one here.
    record: Arc<Record>,
    /// Called once prepare's read is stored, before its tracks are
    /// matched, so tests can act there.
    #[cfg(test)]
    after_read: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Called between matching the tracks and building the preflight.
    #[cfg(test)]
    after_relink: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl<V: Volumes + 'static> Sender<V> {
    pub fn new(flow: SendFlow, volumes: impl Fn() -> V + Send + Sync + 'static) -> Sender<V> {
        Sender {
            flow,
            reader: XmlReader::default(),
            volumes: Arc::new(volumes),
            record: Arc::new(record_send),
            #[cfg(test)]
            after_read: None,
            #[cfg(test)]
            after_relink: None,
        }
    }

    /// Calls `hook` between matching the tracks and building the
    /// preflight.
    #[cfg(test)]
    pub(crate) fn after_relink(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.after_relink = Some(Arc::new(hook));
        self
    }

    /// Calls `hook` once prepare's read is stored, before the matching.
    #[cfg(test)]
    pub(crate) fn after_read(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.after_read = Some(Arc::new(hook));
        self
    }

    /// Records sends with `record` instead of [`record_send`].
    #[cfg(test)]
    pub(crate) fn recording_with(
        mut self,
        record: impl Fn(&mut Connection, &[SentTrack], &[SentPath]) -> rusqlite::Result<()>
            + Send
            + Sync
            + 'static,
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
        // A job another run of the app queued: that run's click, not this
        // one's. It does nothing here, whenever it turns up.
        let run = job
            .target()
            .and_then(|t| t.get("run"))
            .and_then(|r| r.as_str());
        if run != Some(&*self.flow.run) {
            return Err(JobError::Cancelled);
        }
        let _one_at_a_time = self.flow.running.lock().unwrap_or_else(|e| e.into_inner());
        let step = job
            .target()
            .and_then(|t| t.get("send"))
            .and_then(|s| s.as_str());
        let (step, done) = match step {
            Some("prepare") => (SendStep::Prepare, self.prepare(job)),
            Some("write") => (SendStep::Write, self.write(job)),
            _ => (
                SendStep::Write,
                Err(Stop::internal("the job names no send step")),
            ),
        };
        let Err(stop) = done else {
            return Ok(());
        };
        self.flow.end(step, |steps| {
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
        let path = job
            .target()
            .and_then(|t| t.get("path"))
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_owned();
        // The read job itself, run here: it reads the file the job's
        // target names, and records a failure for the rekordbox source.
        self.reader.run(job).map_err(|e| match e {
            JobError::Cancelled => Stop::cancelled(),
            JobError::Failed(detail) => Stop::new(SendFailure::ReadFailed, detail),
        })?;
        #[cfg(test)]
        if let Some(hook) = &self.after_read {
            hook();
        }
        let known = job.writer().call(|c| relink::identities(c))?;
        let mounted = Mounted::ask(&known, &(self.volumes)());
        if job.is_cancelled() {
            return Err(Stop::cancelled());
        }
        // The read replaced every rekordbox row, unmatched, and any other
        // read does the same. So the matching and the preflight are one
        // writer job: no read can land between them, and a track
        // rekordbox has never goes out as new.
        let volumes = self.volumes.clone();
        #[cfg(test)]
        let after_relink = self.after_relink.clone();
        let reviewed = job.writer().call(move |c| {
            relink::relink(c, &mounted)?;
            #[cfg(test)]
            if let Some(hook) = &after_relink {
                hook();
            }
            review(c, &volumes())
        })?;
        let reviewed = reviewed.ok_or_else(|| Stop::internal("the read left no record"))?;
        // Another read, of another export, got in after this one: the
        // preflight would be of a file the user didn't ask for.
        if reviewed.preflight.export.path != path {
            return Err(Stop::new(
                SendFailure::ExportChanged,
                "another export was read meanwhile",
            ));
        }
        // The writer is free again: now ask the disk about the tracks
        // sent at another path than their file's. Only here; the go never
        // asks again, and the token doesn't cover the answer.
        let mut preflight = reviewed.preflight;
        preflight.no_file_at_location = no_file_at_location(&reviewed.sent_elsewhere);
        self.flow.end(SendStep::Prepare, |steps| {
            steps.review = Some(preflight);
        });
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
        // Only a JSON `false` is a "no": anything else there, or nothing,
        // is no answer.
        let imported_since_export = target
            .and_then(|t| t.get("importedSinceExport"))
            .and_then(|a| a.as_bool());
        if !may_send_from_this_export(&reviewed, imported_since_export) {
            return Err(Stop::new(
                SendFailure::ExportOlderThanLastSend,
                "the export is older than the last send, and no send was said not to have been imported since",
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
                // The time first: nothing may fail once the send is
                // recorded.
                let at: String =
                    c.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
                        r.get(0)
                    })?;
                record(c, &outgoing.sent, outgoing.paths())?;
                Ok(at)
            })
        })
        .map_err(|e| match e {
            WriteError::Write(e) => Stop::new(SendFailure::CantWrite, e),
            WriteError::Record { error, .. } => Stop::new(SendFailure::CantRecord, error),
        })?;
        // One preflight, one send: the next send starts with a new read.
        self.flow.end(SendStep::Write, |steps| {
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
