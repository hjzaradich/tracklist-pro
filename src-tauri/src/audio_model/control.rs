//! The audio model's switch, its status, and when the embed job is asked for
//! (ROADMAP 3.8; 5A-A2b).
//!
//! - **Off by default** ([`Feature::AudioModel`]). While it is off nothing is
//!   queued, nothing runs and the model is never loaded.
//! - **Turning it on** queues the embed job if any track is due.
//! - **Turning it off** cancels the embed jobs queued or running. Every
//!   result stored is kept: the next job hides them in the screen.
//! - **Asked for** at startup (`jobs::startup`), when the switch is turned
//!   on, and when the analyze job ends (new files have been hashed and
//!   measured by then); each asks [`request`], which queues only if a track
//!   is due and the switch is on.

use std::sync::Arc;

use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Runtime, State};

use super::job::{embed_job, Embedder};
use super::{ledger, AudioModel};
use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::IpcError;
use crate::jobs::{JobHandler, JobId, JobQueue, NewJob};
use crate::scan::chain::{self, Reach};
use crate::scan::folders;
use crate::scan::ReadGate;
use crate::settings::{read_feature_enabled, write_feature_enabled, Feature};

/// The embed job's handler for the app: the model is the bundled one, read
/// once per run.
pub fn embedder<R: Runtime>(app: &AppHandle<R>) -> impl JobHandler {
    let app = app.clone();
    Embedder::new(crate::scan::system_volumes, move || {
        AudioModel::load_bundled(&app).map(Arc::new)
    })
}

/// Whether the embed job has anything to do now, in the `reachable` music
/// folders if given: false while the switch is off.
pub(crate) fn due(conn: &Connection, reachable: Option<&[i64]>) -> rusqlite::Result<bool> {
    if !read_feature_enabled(conn, Feature::AudioModel)? {
        return Ok(false);
    }
    let gate = ReadGate::for_job(conn)?;
    ledger::any_due(conn, gate.allows(true), reachable)
}

/// Queues an embed job if the switch is on and there is anything to do,
/// unless one is already queued (a running one is asked to run once more).
/// Files in the folders `reach` doesn't name (their drive is unplugged)
/// don't count; with no `reach`, every folder counts.
pub(crate) fn request<E: From<DbError>>(
    writer: &Writer,
    reach: Option<&Reach>,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    let reachable = match reach {
        Some(reach) => {
            let folders = writer.call(|c| folders::stored(c))?;
            Some(reach(&folders))
        }
        None => None,
    };
    if writer.call(move |c| due(c, reachable.as_deref()))? {
        chain::queue_once(writer, embed_job(), enqueue)?;
    }
    Ok(())
}

/// Turns the audio model on or off, as [the module](self) says.
pub fn switch(writer: &Writer, jobs: &JobQueue, on: bool) -> Result<(), DbError> {
    writer.call(move |c| write_feature_enabled(c, Feature::AudioModel, on))?;
    if on {
        return request(writer, None, |job| jobs.enqueue(job));
    }
    let active: Vec<i64> = writer.call(|c| {
        c.prepare("SELECT id FROM job WHERE kind = 'embed' AND status IN ('queued', 'running')")?
            .query_map([], |r| r.get(0))?
            .collect()
    })?;
    for id in active {
        jobs.cancel(JobId(id))?;
    }
    Ok(())
}

/// Where the audio model stands, for the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioModelStatus {
    /// The switch.
    pub enabled: bool,
    /// Library tracks' files with a result for their current audio.
    pub fingerprinted: u32,
    /// Library tracks' files, in all.
    pub total: u32,
    /// Files the model couldn't listen to (not tried again until their audio
    /// changes).
    pub failed: u32,
    /// An embed job is queued or running.
    pub running: bool,
}

/// [`AudioModelStatus`] now.
pub fn status_of(conn: &Connection) -> rusqlite::Result<AudioModelStatus> {
    let counts = ledger::counts(conn)?;
    let running = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM job WHERE kind = 'embed' AND status IN ('queued', 'running'))",
        [],
        |r| r.get(0),
    )?;
    let count = |n: i64| u32::try_from(n).unwrap_or(u32::MAX);
    Ok(AudioModelStatus {
        enabled: read_feature_enabled(conn, Feature::AudioModel)?,
        fingerprinted: count(counts.fingerprinted),
        total: count(counts.total),
        failed: count(counts.failed),
        running,
    })
}

/// Where the audio model stands: the switch and how many tracks are done.
#[tauri::command]
#[specta::specta]
pub async fn audio_model_status(reads: State<'_, ReadPool>) -> Result<AudioModelStatus, IpcError> {
    Ok(reads.read(status_of)?)
}

/// Turns the audio model on (queues the work) or off (cancels it, keeps
/// what it made).
#[tauri::command]
#[specta::specta]
pub async fn set_audio_model_enabled(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    on: bool,
) -> Result<AudioModelStatus, IpcError> {
    switch(&writer, &jobs, on)?;
    Ok(writer.call(|c| status_of(c))?)
}
