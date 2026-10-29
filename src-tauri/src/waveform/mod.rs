//! Waveforms: a short array of peaks per file, computed once in the job
//! queue and cached in the app database (ROADMAP 1.11 "Audio"; 1cB-9).
//!
//! - **What is stored** ([`peaks`]): for each of at most [`BUCKETS`] buckets
//!   across the track, two bytes: the loudest sample and the RMS level, each
//!   0 to 255 for silence to full scale. That is at most 2 KiB a track, 200
//!   MB for 100,000 tracks, and enough to draw the whole track across a
//!   wide window and to zoom into a few seconds of it. A track shorter than
//!   a few seconds gets fewer buckets.
//! - **Computed once, keyed by the audio.** The `waveform` job ([`job`])
//!   decodes a file once, whatever its format (everything the quality and
//!   fingerprint jobs decode, AIFF and ALAC included), and the row in
//!   `file_waveform` (migration 0040) records the `audio_hash` and the
//!   [`VERSION`] it came from ([`ledger`]). A file is decoded again only when
//!   its audio changed or the method did; a retag or a touched mtime costs
//!   nothing. A file that can't be decoded gets a row saying why.
//! - **Behind everything else.** The job asks for itself where the quality
//!   job does ([`request`]; the scan chain and the startup step) but runs at
//!   [`PRIORITY`], the lowest there is, one file at a time, on a thread of the
//!   fingerprint jobs' budget at below-normal priority, and makes way for any
//!   job that is waiting. Startup asks whether any file is due, like every
//!   other stage (`jobs::startup`, 1cA-13), so a run cut short is picked up
//!   again: a job put back does only what is still due.
//! - **Only in the app database.** Nothing is written next to the music; the
//!   audio is opened read-only.
//! - **The loaded file first.** The player asks for the file it has loaded
//!   ([`waveform_first`]); a job for that one file, at user priority, draws
//!   it without waiting for the budget or for other jobs.
//! - **Same audio, drawn once.** Before a file is decoded, the peaks of
//!   another file with the very same `audio_hash` are copied (an index on
//!   it makes that one lookup): duplicates, a copy in two folders and a file
//!   that moved cost no decode.
//! - **The command** [`waveform_peaks`] answers for a file id (the file a
//!   track shows): the peaks, or [`Waveform::NotReady`] while the job hasn't
//!   reached it, or [`Waveform::Unavailable`] for a file that can't be
//!   decoded (no waveform will come). It reads the database and never
//!   decodes.
//!
//! Everything here only reads audio files. The only writes go to the
//! database, through the [`crate::db::Writer`].

mod job;
mod ledger;
pub mod peaks;

#[cfg(test)]
mod tests;

use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Runtime, State};

use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::IpcError;
use crate::jobs::{JobHandler, JobId, JobQueue, NewJob};
use crate::scan::chain::{self, Reach};
use crate::scan::folders;
use crate::scan::ReadGate;

pub use job::{waveform_job, waveform_job_for, Waveformer, PRIORITY};
pub use peaks::BUCKETS;

/// The waveform job's handler for the app, on the thread budget the
/// fingerprint jobs share.
pub fn waveformer<R: Runtime>(app: &AppHandle<R>) -> impl JobHandler {
    Waveformer::new(
        crate::scan::system_volumes,
        crate::fingerprint::shared_first(app),
    )
}

/// The version of the method. Bump it when anything that shapes the peaks
/// changes (the bucket count, what a byte means, how the buckets are cut),
/// and every file's waveform is computed again.
pub const VERSION: u8 = 1;

/// What the app knows of a file's waveform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Waveform {
    /// The peaks: `peaks[i]` is the loudest sample of bucket `i` and
    /// `levels[i]` its RMS level, each 0 to 255.
    #[serde(rename_all = "camelCase")]
    Ready {
        peaks: Vec<u8>,
        levels: Vec<u8>,
        /// The audio decoded, in milliseconds.
        duration_ms: u32,
    },
    /// The job hasn't reached the file (or its audio just changed).
    NotReady,
    /// The file can't be decoded, so there will be no waveform.
    Unavailable,
}

/// The waveform of file `file_id`, as the database has it now.
pub fn waveform_of(conn: &Connection, file_id: i64) -> rusqlite::Result<Waveform> {
    let Some(stored) = ledger::current(conn, file_id)? else {
        return Ok(Waveform::NotReady);
    };
    let (Some(bytes), Some(duration_ms)) = (stored.peaks, stored.duration_ms) else {
        return Ok(Waveform::Unavailable);
    };
    let (peaks, levels) = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[peak, level]| (peak, level))
        .unzip();
    Ok(Waveform::Ready {
        peaks,
        levels,
        duration_ms: u32::try_from(duration_ms).unwrap_or(u32::MAX),
    })
}

/// The waveform of a file, for the player: the peaks, "not ready yet" while
/// the waveform job hasn't reached it, or "unavailable" for a file that
/// can't be decoded. Reads the database; never decodes.
#[tauri::command(async)]
#[specta::specta]
pub fn waveform_peaks(reads: State<'_, ReadPool>, file_id: i64) -> Result<Waveform, IpcError> {
    Ok(reads.read(move |c| waveform_of(c, file_id))?)
}

/// Asks for file `file_id`'s waveform before the rest: the file the player
/// has loaded. Queues a job for that file alone, at user priority, unless it
/// is not due (already drawn, or no audio hash yet) or the same job is
/// already waiting. The job takes no thread from the budget and waits for no
/// other job: it is one file, and the user is looking at it.
pub(crate) fn request_first<E: From<DbError>>(
    writer: &Writer,
    file_id: i64,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    if writer.call(move |c| ledger::due(c, file_id))?.is_some() {
        chain::queue_once(writer, waveform_job_for(vec![file_id]), enqueue)?;
    }
    Ok(())
}

/// Draws file `file_id`'s waveform before the rest (the file the player has
/// loaded). Nothing happens if it is drawn already; the answer to
/// [`waveform_peaks`] changes when it is.
#[tauri::command(async)]
#[specta::specta]
pub fn waveform_first(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    file_id: i64,
) -> Result<(), IpcError> {
    Ok(request_first(&writer, file_id, |job| jobs.enqueue(job))?)
}

/// Queues a waveform job if any file is due, unless one is already queued
/// (a running one is asked to run once more). Called when the scan chain
/// has its hashes in, like the quality job.
///
/// A file is due only if it can be done now: files in the folders `reach`
/// doesn't name (their drive is unplugged) don't count, so no job is queued
/// with nothing to do. With no `reach`, every folder counts.
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
    let due = writer.call(move |c| due(c, reachable.as_deref()))?;
    if due {
        chain::queue_once(writer, waveform_job(), enqueue)?;
    }
    Ok(())
}

/// Whether any file is due a waveform that can be computed now, in the
/// `reachable` music folders if given: the question [`request`] asks, for a
/// caller that asks it on a read connection.
pub(crate) fn due(conn: &Connection, reachable: Option<&[i64]>) -> rusqlite::Result<bool> {
    let gate = ReadGate::for_job(conn)?;
    ledger::any_due(conn, gate.allows(true), reachable)
}
