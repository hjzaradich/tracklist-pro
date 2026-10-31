//! The send's two jobs: prepare (read rekordbox again, build the
//! preflight) and write (the user's go).

use std::fs;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use tauri::{AppHandle, Manager, Runtime};

use super::file::{send_path, write_and_record, WriteError};
use super::preflight::{
    changed_on_disk, describe, labels_of, may_send_from_this_export, merge, merge_read_only,
    no_file_at_location, Describing, Merging, Reviewed,
};
use super::{SendFailure, SendFlow, SendStep, SendWork, Sent};
use crate::conflicts::open_at_send;
use crate::db::Moment;
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
use crate::rekordbox::source::{stored_source, XmlReader};
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

/// How far along each job is when each of its parts starts, as far as a
/// measured send of 100,000 tracks says each part takes (1cD-S1). The part
/// of the prepare step up to `MATCHING` is the export's read.
mod progress {
    pub const MATCHING: f64 = 0.35;
    pub const MERGING: f64 = 0.65;
    pub const BUILDING: f64 = 0.78;
    pub const LOOKING_AT_FILES: f64 = 0.98;

    pub const GO_MERGING: f64 = 0.02;
    pub const GO_BUILDING: f64 = 0.25;
    pub const WRITING: f64 = 0.5;
    pub const RECORDING: f64 = 0.55;
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
    /// Called between matching the tracks and building the preflight, in
    /// the writer job that matches them.
    #[cfg(test)]
    after_relink: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Called as the merge of a review starts, off the writer.
    #[cfg(test)]
    before_merge: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Called once the merge is done, before the send is built.
    #[cfg(test)]
    after_merge: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Called at the go, between building the send again and writing it.
    #[cfg(test)]
    before_write: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl<V: Volumes + 'static> Sender<V> {
    pub fn new(flow: SendFlow, volumes: impl Fn() -> V + Send + Sync + 'static) -> Sender<V> {
        Sender {
            flow,
            reader: XmlReader::up_to(progress::MATCHING),
            volumes: Arc::new(volumes),
            record: Arc::new(record_send),
            #[cfg(test)]
            after_read: None,
            #[cfg(test)]
            after_relink: None,
            #[cfg(test)]
            before_merge: None,
            #[cfg(test)]
            after_merge: None,
            #[cfg(test)]
            before_write: None,
        }
    }

    /// Calls `hook` at the go, between building the send again and writing
    /// it.
    #[cfg(test)]
    pub(crate) fn before_write(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.before_write = Some(Arc::new(hook));
        self
    }

    /// Calls `hook` between matching the tracks and building the
    /// preflight.
    #[cfg(test)]
    pub(crate) fn after_relink(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.after_relink = Some(Arc::new(hook));
        self
    }

    /// Calls `hook` as the merge of a review starts, off the writer.
    #[cfg(test)]
    pub(crate) fn before_merge(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.before_merge = Some(Arc::new(hook));
        self
    }

    /// Calls `hook` once the merge is done, before the send is built.
    #[cfg(test)]
    pub(crate) fn after_merge(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.after_merge = Some(Arc::new(hook));
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

impl From<rusqlite::Error> for Stop {
    fn from(e: rusqlite::Error) -> Stop {
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
        self.flow.working_on(SendWork::Reading);
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
        self.part(job, SendWork::Matching, progress::MATCHING)?;
        let known = job.writer().call(|c| relink::identities(c))?;
        let mounted = Mounted::ask(&known, &(self.volumes)());
        if job.is_cancelled() {
            return Err(Stop::cancelled());
        }
        // The read replaced every rekordbox row, unmatched, and any other
        // read does the same. So the review reads the moment the matching
        // ended, held in the matching's own writer job: no read can land
        // between them, and a track rekordbox has never goes out as new.
        let moment = Moment::open(job.writer().guarded_path())?;
        #[cfg(test)]
        let after_relink = self.after_relink.clone();
        let (moment, mounted) = job.writer().call(move |c| {
            relink::relink(c, &mounted)?;
            #[cfg(test)]
            if let Some(hook) = &after_relink {
                hook();
            }
            moment.hold()?;
            Ok((moment, mounted))
        })?;
        self.part(job, SendWork::Merging, progress::MERGING)?;
        let volumes = (self.volumes)();
        let (merging, moment) = self.merged(job, &volumes, moment, Some(mounted))?;
        let merging = merging.ok_or_else(|| Stop::internal("the read left no record"))?;
        self.part(job, SendWork::Building, progress::BUILDING)?;
        let reviewed = describe(moment.conn(), &volumes, merging, Describing::All)?;
        // Another read, of another export, got in after this one: the
        // preflight would be of a file the user didn't ask for.
        if reviewed.preflight.export.path != path {
            return Err(Stop::new(
                SendFailure::ExportChanged,
                "another export was read meanwhile",
            ));
        }
        // Now ask the disk about the tracks sent at another path than their
        // file's, and about files changed after the export that no scan has
        // seen yet (1.10, "Second channel"). Only here; the go never asks
        // again, and the token doesn't cover the answers.
        self.part(job, SendWork::LookingAtFiles, progress::LOOKING_AT_FILES)?;
        let mut preflight = reviewed.preflight;
        let no_file = no_file_at_location(&reviewed.sent_elsewhere);
        preflight.no_file_at_location =
            labels_of(moment.conn(), &volumes, &no_file, &reviewed.gone)?;
        let changed = changed_on_disk(&reviewed.watched, preflight.export.modified_ms);
        preflight.changed_after_export =
            labels_of(moment.conn(), &volumes, &changed, &reviewed.gone)?;
        self.flow.end(SendStep::Prepare, |steps| {
            steps.review = Some(preflight);
        });
        Ok(())
    }

    /// A part of a step begins, `done` of the way through its job. A report
    /// that fails stops nothing; a cancelled job stops here.
    fn part(&self, job: &JobContext, work: SendWork, done: f64) -> Result<(), Stop> {
        self.flow.working_on(work);
        if job.is_cancelled() {
            return Err(Stop::cancelled());
        }
        let _ = job.progress(done);
        Ok(())
    }

    /// The merge at `moment`, and the moment the rest of the review reads.
    /// With nothing to write it is all done on `moment`, off the writer. With
    /// something to write it is done again on the writer, whatever else
    /// landed meanwhile, and the rest reads the moment that job ends: what
    /// the merge wrote is in it. `None` if rekordbox was never read.
    ///
    /// `matched`, for the prepare step: a read landing since the matching
    /// left every row unmatched, so this job matches them first, and no
    /// track rekordbox has goes out as new.
    fn merged(
        &self,
        job: &JobContext,
        volumes: &V,
        moment: Moment,
        matched: Option<Mounted>,
    ) -> Result<(Option<Merging>, Moment), Stop> {
        #[cfg(test)]
        if let Some(hook) = &self.before_merge {
            hook();
        }
        let merged = self.merge_on(job, volumes, moment, matched)?;
        #[cfg(test)]
        if let Some(hook) = &self.after_merge {
            hook();
        }
        Ok(merged)
    }

    fn merge_on(
        &self,
        job: &JobContext,
        volumes: &V,
        moment: Moment,
        matched: Option<Mounted>,
    ) -> Result<(Option<Merging>, Moment), Stop> {
        if let Ok(merging) = merge_read_only(moment.conn(), volumes)? {
            return Ok((merging, moment));
        }
        fn read_at(conn: &Connection) -> rusqlite::Result<Option<String>> {
            Ok(stored_source(conn)?.last_read.map(|read| read.read_at))
        }
        let seen = read_at(moment.conn())?;
        drop(moment);
        let moment = Moment::open(job.writer().guarded_path())?;
        let volumes = self.volumes.clone();
        Ok(job.writer().call(move |c| {
            if let Some(mounted) = matched {
                if read_at(c)? != seen {
                    relink::relink(c, &mounted)?;
                }
            }
            let merging = merge(c, &volumes())?;
            moment.hold()?;
            Ok((merging, moment))
        })?)
    }

    /// The review the go checks: the database as it is now, which holds the
    /// writer only if the merge has something to write.
    fn review(&self, job: &JobContext) -> Result<Option<Reviewed>, Stop> {
        let moment = Moment::open(job.writer().guarded_path())?;
        moment.hold()?;
        let volumes = (self.volumes)();
        let (merging, moment) = self.merged(job, &volumes, moment, None)?;
        let Some(merging) = merging else {
            return Ok(None);
        };
        self.part(job, SendWork::Building, progress::GO_BUILDING)?;
        Ok(Some(describe(
            moment.conn(),
            &volumes,
            merging,
            Describing::ForTheGo,
        )?))
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
        self.part(job, SendWork::Merging, progress::GO_MERGING)?;
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
        #[cfg(test)]
        if let Some(hook) = &self.before_write {
            hook();
        }

        self.part(job, SendWork::Writing, progress::WRITING)?;
        let outgoing = Arc::new(outgoing);
        let open_seen = now.open;
        let dest = send_path(self.flow.guard());
        let at = write_and_record(self.flow.guard(), &dest, &outgoing, || {
            // The file is in place: a cancel changes nothing any more.
            self.flow.working_on(SendWork::Recording);
            let _ = job.progress(progress::RECORDING);
            let record = self.record.clone();
            let outgoing = outgoing.clone();
            job.writer()
                .call(move |c| {
                    // The conflicts the merge chose from must be the ones
                    // now: a resolve or an Undo landing since would make the
                    // record keep or move a base the file doesn't match.
                    let sent: Vec<LibraryTrackId> =
                        outgoing.sent.iter().map(|s| s.library_track).collect();
                    if open_at_send::open_fields(c, &sent)? != open_seen {
                        return Ok(None);
                    }
                    // The time first: nothing may fail once the send is
                    // recorded.
                    let at: String =
                        c.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
                            r.get(0)
                        })?;
                    record(c, &outgoing.sent, outgoing.paths())?;
                    Ok(Some(at))
                })
                .map_err(RecordError::Db)?
                .ok_or(RecordError::ConflictsChanged)
        })
        .map_err(|e| match e {
            WriteError::Write(e) => Stop::new(SendFailure::CantWrite, e),
            WriteError::Record {
                error: RecordError::ConflictsChanged,
                ..
            } => Stop::new(
                SendFailure::LibraryChanged,
                "a conflict was settled or reopened during the send",
            )
            .dropping_preflight(),
            WriteError::Record {
                error: RecordError::Db(error),
                ..
            } => Stop::new(SendFailure::CantRecord, error),
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

/// Why the go's record step didn't record.
enum RecordError {
    /// The database refused.
    Db(crate::db::DbError),
    /// The open conflicts of the tracks sent aren't the ones the send's
    /// merge saw: nothing was recorded, and the file is put back.
    ConflictsChanged,
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
