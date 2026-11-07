//! The app's own estimates (ROADMAP 3.7 "Local estimates"): BPM now; the
//! energy features (E1b) and key (E2) join here.
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
mod job;
pub(crate) mod ledger;
pub mod window;

#[cfg(test)]
mod tests;

use rusqlite::Connection;
use tauri::{AppHandle, Runtime};

use crate::db::{DbError, Writer};
use crate::jobs::{JobHandler, JobId, NewJob};
use crate::scan::chain::{self, Reach};
use crate::scan::folders;
use crate::scan::ReadGate;

pub use job::{analyze_job, Analyzer, PRIORITY};

/// The version of the method. Bump it when anything that shapes what is
/// measured changes (the window, the detector, a feature added), and every
/// file is measured again; the `'local'` rows are updated in place.
pub const VERSION: u8 = 1;

/// The analyze job's handler for the app, on the thread budget the
/// fingerprint jobs share.
pub fn analyzer<R: Runtime>(app: &AppHandle<R>) -> impl JobHandler {
    Analyzer::new(
        crate::scan::system_volumes,
        crate::fingerprint::shared_first(app),
    )
}

/// Queues an analyze job if there is anything to do, unless one is already
/// queued (a running one is asked to run once more). Files in the folders
/// `reach` doesn't name (their drive is unplugged) don't count; with no
/// `reach`, every folder counts.
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
        chain::queue_once(writer, analyze_job(), enqueue)?;
    }
    Ok(())
}

/// Whether the analyze job has anything to do now, in the `reachable` music
/// folders if given: the question [`request`] asks, for a caller that asks
/// it on a read connection.
pub(crate) fn due(conn: &Connection, reachable: Option<&[i64]>) -> rusqlite::Result<bool> {
    let gate = ReadGate::for_job(conn)?;
    ledger::any_due(conn, gate.allows(true), reachable)
}
