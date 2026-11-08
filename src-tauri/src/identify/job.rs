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

/// The job for Inbox files that just arrived: as [`lookup_job`], marked
/// automatic. An automatic job asks nothing for the files if, when it runs,
/// neither AcoustID nor MusicBrainz is switched on (the owner's decision: the
/// only automatic look-ups are while those two are on; Discogs alone doesn't
/// start one).
pub fn automatic_lookup_job(file_ids: &[i64]) -> NewJob {
    NewJob::new(JobKind::IdentifyLookup)
        .target(serde_json::json!({ "file_ids": file_ids, "automatic": true }))
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
            discogs: is_opted_in(&self.reads, Service::Discogs)?,
        })
    }
}

fn input_of(conn: &Connection, file: i64) -> rusqlite::Result<Option<Input>> {
    conn.query_row(
        "SELECT fingerprint, duration_ms, raw_tags, rel_path, fingerprint_rev FROM file WHERE id = ?1",
        [file],
        |r| {
            let raw_tags: Option<String> = r.get(2)?;
            let rel_path: String = r.get(3)?;
            Ok(Input {
                fingerprint: r.get(0)?,
                duration_ms: r.get(1)?,
                fingerprint_rev: r.get(4)?,
                names: super::names::names_of(raw_tags.as_deref(), &rel_path),
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
        let automatic = job
            .target()
            .and_then(|t| t.get("automatic"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        self.backend.new_batch();
        let mut offline = false;
        for (done, &file) in ids.iter().enumerate() {
            job.check_cancelled()?;
            let Some(input) = self.reads.read(|conn| input_of(conn, file))? else {
                // The file is gone; there is nothing to keep.
                continue;
            };
            let on = self.switches()?;
            let answer = if offline {
                Answer::failed(Why::Offline)
            } else if automatic && !on.acoustid && !on.musicbrainz {
                // Switched off since the file arrived: nothing is asked.
                Answer::skipped(Why::ServicesOff)
            } else {
                look_up(&*self.backend, on, &input)
            };
            offline = answer.why == Some(Why::Offline);
            let rev = input.fingerprint_rev;
            job.writer()
                .call(move |conn| cache::save_at(conn, file, &answer, rev))?;
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
    let of_file = file_status(conn, file)?;
    status.state = of_file.state;
    status.reason = of_file.reason;
    status.found = of_file.found;
    status.when = of_file.when;
    status.later_attempt = of_file.later_attempt;
    Ok(status)
}

/// One Inbox file's look-up status: the states are those of a track's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InboxLookupStatus {
    pub file_id: i64,
    pub state: LookupState,
    pub reason: Option<String>,
    pub found: u32,
    pub when: Option<String>,
    pub later_attempt: Option<LookupAttempt>,
    /// The best values the file's kept result found for its artist, title
    /// and Mix, with the services that gave them; none when the file has no
    /// current result that found any.
    pub identified: Option<Identified>,
}

/// What the online look-up says a file is: for each of artist, title and Mix
/// the value most services agree on (then the first found), and the
/// services behind those values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Identified {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub mix: Option<String>,
    pub sources: Vec<super::IdentifySource>,
}

/// The status of each of `files`, in that order.
pub fn file_statuses(conn: &Connection, files: &[i64]) -> rusqlite::Result<Vec<InboxLookupStatus>> {
    files
        .iter()
        .map(|&file_id| {
            let s = file_status(conn, file_id)?;
            Ok(InboxLookupStatus {
                file_id,
                state: s.state,
                reason: s.reason,
                found: s.found,
                when: s.when,
                later_attempt: s.later_attempt,
                identified: identified(conn, file_id)?,
            })
        })
        .collect()
}

/// Where the online look-up of Inbox files stands: the same states as for
/// a track.
#[tauri::command]
#[specta::specta]
pub async fn identify_inbox_status(
    reads: State<'_, ReadPool>,
    file_ids: Vec<i64>,
) -> Result<Vec<InboxLookupStatus>, IpcError> {
    Ok(reads.read(|conn| file_statuses(conn, &file_ids))?)
}

struct FileStatus {
    state: LookupState,
    reason: Option<String>,
    found: u32,
    when: Option<String>,
    later_attempt: Option<LookupAttempt>,
}

fn file_status(conn: &Connection, file: i64) -> rusqlite::Result<FileStatus> {
    let mut status = FileStatus {
        state: LookupState::Never,
        reason: None,
        found: 0,
        when: None,
        later_attempt: None,
    };
    // An answer for audio that has changed since is out of date: the file
    // counts as never looked up.
    if let Some(kept) = cache::load(conn, file)?.filter(|k| !k.stale) {
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

/// The linked files of the Library tracks of `recordings` (a table row's
/// `recordingId`), each once, in order; a recording with no Library track,
/// or none with a linked file, adds nothing.
pub(super) fn linked_files_of_recordings(
    conn: &Connection,
    recordings: &[i64],
) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT linked_file_id FROM library_track
         WHERE recording_id = ?1 AND linked_file_id IS NOT NULL ORDER BY id",
    )?;
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    for recording in recordings {
        let found = stmt.query_map([recording], |r| r.get::<_, i64>(0))?;
        for file in found {
            let file = file?;
            if seen.insert(file) {
                files.push(file);
            }
        }
    }
    Ok(files)
}

/// [`identify_look_up_tracks`] for track ids (a table row's `recordingId`):
/// the Library tracks and their files are found here, so a selection of any
/// size, rows not loaded included, needs nothing mapped by the screen.
#[tauri::command(async)]
#[specta::specta]
pub fn identify_look_up_recordings(
    reads: State<'_, ReadPool>,
    jobs: State<'_, JobQueue>,
    recordings: Vec<i64>,
) -> Result<Option<JobId>, IpcError> {
    let files = reads.read(|conn| linked_files_of_recordings(conn, &recordings))?;
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
        super::lookup::Live::new(
            app.state::<crate::net::Net>().inner().clone(),
            app.state::<crate::credentials::Credentials>()
                .inner()
                .clone(),
        ),
        app.state::<ReadPool>().inner().clone(),
    )
}

/// The best candidate values of `file`'s current kept result, if it found
/// any artist or title.
pub fn identified(conn: &Connection, file: i64) -> rusqlite::Result<Option<Identified>> {
    let Some(kept) =
        cache::load(conn, file)?.filter(|k| k.outcome == super::lookup::Outcome::Found && !k.stale)
    else {
        return Ok(None);
    };
    let mut sources: Vec<super::IdentifySource> = Vec::new();
    let mut best = |field: super::IdentifyField| -> Option<String> {
        // The value with the most distinct sources; the first found wins a tie.
        let mut values: Vec<(&str, Vec<&super::IdentifySource>)> = Vec::new();
        for c in kept.candidates.iter().filter(|c| c.field == field) {
            match values.iter_mut().find(|(v, _)| *v == c.value) {
                Some((_, from)) => {
                    if !from.contains(&&c.source) {
                        from.push(&c.source);
                    }
                }
                None => values.push((&c.value, vec![&c.source])),
            }
        }
        let (value, from) = values.into_iter().reduce(|best, next| {
            if next.1.len() > best.1.len() {
                next
            } else {
                best
            }
        })?;
        for source in from {
            if !sources.contains(source) {
                sources.push(source.clone());
            }
        }
        Some(value.to_owned())
    };
    let artist = best(super::IdentifyField::Artist);
    let title = best(super::IdentifyField::Name);
    let mix = best(super::IdentifyField::Mix);
    if artist.is_none() && title.is_none() {
        return Ok(None);
    }
    Ok(Some(Identified {
        artist,
        title,
        mix,
        sources,
    }))
}
