//! The online look-up as a background job, and the commands around it
//! (ROADMAP 2.2; Run 6A, S2a).
//!
//! The job ([`JobKind::IdentifyLookup`], target `{"file_ids": [...]}`) looks
//! each file up in turn ([`super::lookup`]) for the services that are switched
//! on at that moment, and stores each answer as soon as it has it
//! ([`super::cache`]). It stops between files when cancelled. One file that
//! fails is marked failed and the job goes on; once the internet turns out
//! to be unreachable, the files left are marked failed as offline without
//! another request, so nothing retries in a loop. Every request goes through
//! the gate, which keeps each service's speed limit.
//!
//! **Priority** is [`PRIORITY`]: under everything the user is waiting on,
//! over the whole-collection background stages (-10), because someone asked
//! for it. Its speed is the services' limits (about one file a few seconds),
//! not the computer's.
//!
//! **Not restartable** (`jobs::store::RESTARTABLE`): a close during a run
//! ends the job as interrupted instead of putting it back. A restarted run
//! would start the whole list again, send requests nobody asked for at the
//! next start, and repeat files whose answers are already kept. The answers
//! stored so far stay, and asking again costs the user one click.

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;
use tauri::State;

use super::cache;
use super::lookup::{look_up, Answer, Backend, Input, On, Why};
use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, JobQueue, NewJob, Priority};
use crate::library::LibraryTrackId;
use crate::net::{is_opted_in, Service};

/// Under the work the user waits on, over the background stages.
pub const PRIORITY: Priority = Priority(-5);

/// The job for `file_ids`, in that order.
pub fn lookup_job(file_ids: &[i64]) -> NewJob {
    NewJob::new(JobKind::IdentifyLookup)
        .target(serde_json::json!({ "file_ids": file_ids }))
        .priority(PRIORITY)
}

/// The job's handler.
pub struct Looker {
    backend: Arc<dyn Backend>,
    reads: ReadPool,
}

impl Looker {
    pub fn new(backend: impl Backend, reads: ReadPool) -> Looker {
        Looker {
            backend: Arc::new(backend),
            reads,
        }
    }

    fn switches(&self) -> Result<On, JobError> {
        Ok(On {
            acoustid: is_opted_in(&self.reads, Service::AcoustId)?,
            musicbrainz: is_opted_in(&self.reads, Service::MusicBrainz)?,
        })
    }
}

fn input_of(conn: &Connection, file: i64) -> rusqlite::Result<Option<Input>> {
    conn.query_row(
        "SELECT fingerprint, duration_ms FROM file WHERE id = ?1",
        [file],
        |r| {
            Ok(Input {
                fingerprint: r.get(0)?,
                duration_ms: r.get(1)?,
            })
        },
    )
    .optional()
}

impl JobHandler for Looker {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let ids: Vec<i64> = job
            .target()
            .and_then(|t| t.get("file_ids"))
            .and_then(|ids| serde_json::from_value(ids.clone()).ok())
            .ok_or_else(|| JobError::failed("the look-up job has no list of files"))?;
        let mut offline = false;
        for (done, &file) in ids.iter().enumerate() {
            job.check_cancelled()?;
            let Some(input) = self.reads.read(|conn| input_of(conn, file))? else {
                // The file is gone; there is nothing to keep.
                continue;
            };
            let answer = if offline {
                Answer::failed(Why::Offline)
            } else {
                look_up(&*self.backend, self.switches()?, &input)
            };
            offline = answer.why == Some(Why::Offline);
            job.writer()
                .call(move |conn| cache::save(conn, file, &answer))?;
            job.progress((done + 1) as f64 / ids.len() as f64)?;
        }
        Ok(())
    }
}

/// How a file's look-up stands, for the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LookupState {
    /// Never looked up (or the track has no linked file).
    Never,
    /// In a job that has not started.
    Queued,
    /// In a job that is running.
    Running,
    Found,
    Nothing,
    Failed,
    Skipped,
}

/// A later attempt that did not replace a good answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LookupAttempt {
    /// A short code, e.g. `offline`.
    pub reason: String,
    pub when: String,
}

/// One Library track's look-up status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdentifyLookupStatus {
    pub library_track_id: LibraryTrackId,
    /// While a job is queued or running for the track's file this says so;
    /// the rest still describes the answer kept from before.
    pub state: LookupState,
    /// Why it found nothing, failed or was skipped: a short code, e.g.
    /// `offline`, `key_missing`, `acoustid_off`, `no_match`.
    pub reason: Option<String>,
    /// How many details were found.
    pub found: u32,
    /// When the kept answer was fetched (UTC).
    pub when: Option<String>,
    pub later_attempt: Option<LookupAttempt>,
}

/// The status of each of `tracks`, in that order.
pub fn status_of(
    conn: &Connection,
    tracks: &[LibraryTrackId],
) -> rusqlite::Result<Vec<IdentifyLookupStatus>> {
    tracks.iter().map(|&t| one_status(conn, t)).collect()
}

fn one_status(conn: &Connection, track: LibraryTrackId) -> rusqlite::Result<IdentifyLookupStatus> {
    let file: Option<i64> = conn
        .query_row(
            "SELECT linked_file_id FROM library_track WHERE id = ?1",
            [track.0],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let mut status = IdentifyLookupStatus {
        library_track_id: track,
        state: LookupState::Never,
        reason: None,
        found: 0,
        when: None,
        later_attempt: None,
    };
    let Some(file) = file else { return Ok(status) };
    if let Some(kept) = cache::load(conn, file)? {
        use super::lookup::Outcome;
        status.state = match kept.outcome {
            Outcome::Found => LookupState::Found,
            Outcome::Nothing => LookupState::Nothing,
            Outcome::Failed => LookupState::Failed,
            Outcome::Skipped => LookupState::Skipped,
        };
        status.reason = kept.reason;
        status.found = u32::try_from(kept.candidates.len()).unwrap_or(u32::MAX);
        status.when = Some(kept.fetched_at);
        status.later_attempt = kept
            .attempt
            .map(|(reason, when)| LookupAttempt { reason, when });
    }
    let waiting: Option<String> = conn
        .prepare_cached(
            "SELECT j.status FROM job j, json_each(j.target, '$.file_ids') e
             WHERE j.kind = ?1 AND j.status IN ('queued', 'running') AND e.value = ?2
             ORDER BY j.status = 'running' DESC LIMIT 1",
        )?
        .query_row((JobKind::IdentifyLookup.as_str(), file), |r| r.get(0))
        .optional()?;
    match waiting.as_deref() {
        Some("running") => status.state = LookupState::Running,
        Some(_) => status.state = LookupState::Queued,
        None => {}
    }
    Ok(status)
}

/// The linked files of `tracks`, each once, in order; a track with none adds
/// nothing.
pub(super) fn linked_files(
    conn: &Connection,
    tracks: &[LibraryTrackId],
) -> rusqlite::Result<Vec<i64>> {
    let mut files = Vec::new();
    for track in tracks {
        let file: Option<i64> = conn
            .query_row(
                "SELECT linked_file_id FROM library_track WHERE id = ?1",
                [track.0],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(file) = file.filter(|f| !files.contains(f)) {
            files.push(file);
        }
    }
    Ok(files)
}

/// Looks up the linked files of Library tracks online, for the services
/// that are switched on, in the background. Answers the job, or none when no
/// track has a linked file.
#[tauri::command(async)]
#[specta::specta]
pub fn identify_look_up_tracks(
    reads: State<'_, ReadPool>,
    jobs: State<'_, JobQueue>,
    library_track_ids: Vec<LibraryTrackId>,
) -> Result<Option<JobId>, IpcError> {
    let files = reads.read(|conn| linked_files(conn, &library_track_ids))?;
    Ok(enqueue(&jobs, files)?)
}

/// Looks up files online (the Inbox's), as [`identify_look_up_tracks`] does.
#[tauri::command(async)]
#[specta::specta]
pub fn identify_look_up_files(
    jobs: State<'_, JobQueue>,
    file_ids: Vec<i64>,
) -> Result<Option<JobId>, IpcError> {
    let mut files: Vec<i64> = Vec::new();
    for id in file_ids {
        if !files.contains(&id) {
            files.push(id);
        }
    }
    Ok(enqueue(&jobs, files)?)
}

/// Queues the job for `files`; none for an empty list.
pub fn enqueue(jobs: &JobQueue, files: Vec<i64>) -> Result<Option<JobId>, crate::db::DbError> {
    if files.is_empty() {
        return Ok(None);
    }
    jobs.enqueue(lookup_job(&files)).map(Some)
}

/// Where the online look-up of each of these Library tracks stands.
#[tauri::command]
#[specta::specta]
pub async fn identify_lookup_status(
    reads: State<'_, ReadPool>,
    library_track_ids: Vec<LibraryTrackId>,
) -> Result<Vec<IdentifyLookupStatus>, IpcError> {
    Ok(reads.read(|conn| status_of(conn, &library_track_ids))?)
}

/// The job's handler for the app: the real services, through the gate.
pub fn looker<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Looker {
    use tauri::Manager;
    Looker::new(
        super::lookup::Live(app.state::<crate::net::Net>().inner().clone()),
        app.state::<ReadPool>().inner().clone(),
    )
}
