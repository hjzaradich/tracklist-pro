//! The app's own estimates (ROADMAP 3.7 "Local estimates"): BPM and key;
//! the energy features (E1b) join here.
//!
//! - **Measured once, keyed by the audio.** The analyze job ([`job`])
//!   decodes a window from the middle of each Library track's file
//!   ([`window`]) and keeps what it measured in `file_estimate` (migration
//!   0053; [`ledger`]) with the `audio_hash` and the [`VERSION`] it came
//!   from. A file is decoded again only when its audio or the method
//!   changed; a file that can't be measured (it doesn't decode, it's too
//!   short, it's silent) gets a row saying why.
//! - **BPM** ([`bpm`]): the spike's detector, folded into the user's BPM
//!   range (the setting `bpm_range`, rekordbox's 98 to 195 by default).
//! - **Key** ([`key`]): libKeyFinder's method, translated into Rust, from
//!   the same decoded window. Always measured and kept in the ledger, but
//!   shown (a key in the `'local'` row) only while the owner's test switch
//!   is on ([`crate::settings::Feature::LocalKey`], off by default), until
//!   it reaches the bar in 3.7. With it off, nothing reads the local key.
//!   [`compare`] counts how it agrees with rekordbox, for that test.
//! - **Shown as the weakest source.** Each Library track whose file has a
//!   BPM gets one `'local'` analysis row, updated in place; rekordbox, tags,
//!   Mixed In Key and the user all win over it by the precedence the track
//!   table and `recording_display` already have, so it shows only where
//!   nothing better exists.
//! - **Never energy into `analysis`** (owner, 2026-11-06): energy comes only
//!   from sources set beforehand (Mixed In Key tags) or the user. The energy
//!   features and the app's suggestion stay in the ledger (E1b).
//! - **Never sent.** A send never writes a BPM or key into the XML (ROADMAP
//!   1.9 rule 3, [`crate::rekordbox_write`]), so an estimate never reaches
//!   rekordbox, for a track it has or a new one.
//! - **Behind everything else**, in the background: asked for wherever the
//!   waveform job is (the scan chain once the fingerprints are in, a change
//!   to which files the Library has, startup), and run at the same lowest
//!   priority, on a thread of the fingerprint jobs' budget.
//!
//! Everything here only reads audio files. The only writes go to the
//! database, through the [`crate::db::Writer`].

pub mod bpm;
pub mod compare;
pub mod features;
mod job;
pub mod key;
pub(crate) mod ledger;
pub mod suggest;
pub mod window;

#[cfg(test)]
pub(crate) mod support;
#[cfg(test)]
mod tests;

use rusqlite::Connection;
use tauri::{AppHandle, Runtime};

use crate::db::{DbError, ReadPool, Writer};
use crate::jobs::{JobHandler, JobId, NewJob};
use crate::scan::chain::{self, Reach};
use crate::scan::folders;
use crate::scan::ReadGate;
use crate::settings::{read_bpm_range, write_bpm_range, BpmRange};

pub use job::{analyze_job, Analyzer, PRIORITY};

/// The version of the method. Bump it when anything that shapes what is
/// measured changes (the window, the detector, a feature added), and every
/// file is measured again; the `'local'` rows are kept until the new
/// measurement replaces them in place. 2: the key (3C-E2). 3: the energy
/// features (3C-E1b).
pub const VERSION: u8 = 3;

/// The analyze job's handler for the app, on the thread budget the
/// fingerprint jobs share.
pub fn analyzer<R: Runtime>(app: &AppHandle<R>) -> impl JobHandler {
    Analyzer::new(
        crate::scan::system_volumes,
        crate::fingerprint::shared_first(app),
    )
}

/// How many Library tracks one writer call brings in step with the ledger.
const SYNC_PAGE: i64 = 1000;

/// Queues an analyze job if there is anything to do, unless one is already
/// queued (a running one is asked to run once more). Files in the folders
/// `reach` doesn't name (their drive is unplugged) don't count; with no
/// `reach`, every folder counts.
///
/// First, a track whose `'local'` row shows another file's BPM than the
/// file it shows now (a relink, a merge or an undo put it there) is put
/// right at once, without a decode ([`resync`]).
pub(crate) fn request<E: From<DbError> + std::fmt::Debug>(
    writer: &Writer,
    reach: Option<&Reach>,
    mut enqueue: impl FnMut(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    // A pure read: it goes through a read connection of its own, not the
    // writer's queue (the caller's writes have committed by now).
    let reads = ReadPool::with_size(writer.guarded_path(), 1)?;
    if reads.read(ledger::any_stale)? {
        resync(writer, &mut enqueue)?;
    }
    let reachable = match reach {
        Some(reach) => {
            let folders = writer.call(|c| folders::stored(c))?;
            Some(reach(&folders))
        }
        None => None,
    };
    if writer.call(move |c| due(c, reachable.as_deref()))? {
        chain::queue_once(writer, analyze_job(), enqueue)?;
    }
    Ok(())
}

/// Brings every Library track's `'local'` row in step with the ledger and
/// the user's BPM range, a page of tracks per writer call, and asks for the
/// search index when any changed (the track table reads the index). No
/// decode. A failure to ask for the index is not the resync's: the next
/// job to end, or the next launch, asks again.
pub(crate) fn resync<E: From<DbError> + std::fmt::Debug>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<usize, E> {
    let range = writer.call(|c| read_bpm_range(c))?;
    let written = sync_all(writer, range)?;
    if written > 0 {
        if let Err(e) = crate::search::request(writer, enqueue) {
            eprintln!("estimate: couldn't ask for the search index: {e:?}");
        }
    }
    Ok(written)
}

/// [`ledger::sync`] over every Library track in `range`, a page per writer
/// call so nothing else waits for the whole Library. Returns how many rows
/// it wrote.
pub(crate) fn sync_all(writer: &Writer, range: BpmRange) -> Result<usize, DbError> {
    let (mut after, mut written) = (0, 0);
    loop {
        let (wrote, last) = writer.call(move |c| {
            let tx = c.transaction()?;
            let page = ledger::sync_page(&tx, None, after, SYNC_PAGE, range)?;
            tx.commit()?;
            Ok(page)
        })?;
        written += wrote;
        match last {
            Some(last) => after = last,
            None => return Ok(written),
        }
    }
}

/// Sets the user's BPM range and folds every track's estimate into it again
/// (the ledger keeps the detector's own answer, so nothing is decoded).
pub fn set_bpm_range<E: From<DbError> + std::fmt::Debug>(
    writer: &Writer,
    range: BpmRange,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    writer.call(move |c| write_bpm_range(c, range))?;
    resync(writer, enqueue)?;
    Ok(())
}

/// Whether the analyze job has anything to do now, in the `reachable` music
/// folders if given: the question [`request`] asks, for a caller that asks
/// it on a read connection.
pub(crate) fn due(conn: &Connection, reachable: Option<&[i64]>) -> rusqlite::Result<bool> {
    let gate = ReadGate::for_job(conn)?;
    Ok(ledger::any_due(conn, gate.allows(true), reachable)? || suggest::needed(conn)?)
}
