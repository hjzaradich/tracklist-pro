//! Quality measurements: stage 4 of the scan (ROADMAP 1.6; 1bA-10,
//! 1bA-11).
//!
//! A background job ([`job`]) decodes each file once and measures:
//!
//! - the **spectral cutoff** ([`cutoff`]): the highest frequency within
//!   50 dB of the 2-8 kHz level, from a 60 s sample, or why it can't be
//!   measured;
//! - the **duration decoded** against the **duration the header claims**,
//!   and **how the stream ended**;
//! - the **decode errors**, counted by kind.
//!
//! These are measurements only. What they mean (`suspect transcode`,
//! `truncated`, `broken`…) is decided later (1bB-6), from the rows in
//! `file_quality`. Nothing here shows text to the user.
//!
//! Results follow the audio, not the modified time (§5.1): a row records
//! the `audio_hash` it was measured from and the [`VERSION`] of the method,
//! and a file is measured again only when its audio changed or the method
//! did ([`ledger`]).
//!
//! **What shows as an error.** Symphonia resynchronizes past a lot of
//! damage without reporting it (a corrupt FLAC frame, a broken MP3 header),
//! so a damaged file often has no decode errors and shows only as audio
//! missing: `decoded_ms` shorter than `header_ms`. Both are stored, and
//! the verdicts read them together. Reported errors are counted by kind
//! (`decode`: the decoder refused a packet; `container`: the container
//! couldn't produce one).
//!
//! **Files with no `audio_hash`** aren't measured: a file whose container
//! the hash stage couldn't find the audio in (a WAV whose data chunk runs
//! past its end, an empty file) has no audio to key a measurement to. (The
//! read stage can already mark some of those truncated or broken.)
//!
//! **Sharing the decode with the fingerprint job (not done).** Both jobs
//! walk a file with [`crate::fingerprint::decode::walk`]. They could be one
//! pass: a single job taking each due file once and handing every decoded
//! block to both the fingerprinter and [`cutoff::Meter`]. That needs one
//! job that knows both ledgers (a file due only one of them still has to be
//! decoded in full, or only the due half is fed) and one result writer per
//! kind, so it's left for when the extra decode shows up as a cost.
//!
//! Everything here only reads audio files. The only writes go to the
//! database, through the [`crate::db::Writer`].

pub mod cutoff;
mod job;
pub mod ledger;
pub mod measure;

pub use cutoff::Gap;
pub use job::{quality_job, Qualifier};
pub use ledger::Stored;
pub use measure::{measure, Ended, Errors, Failure, Measured};

use crate::db::{DbError, Writer};
use crate::jobs::{JobHandler, JobId, NewJob};
use crate::scan::{chain, ReadGate};
use tauri::{AppHandle, Runtime};

/// The version of the method. Bump it when anything that shapes a
/// measurement changes (the sample, the FFT, the threshold, what counts as
/// a decode error), and every file is measured again.
pub const VERSION: u8 = 1;

/// The quality job's handler for the app, on the thread budget the
/// fingerprint jobs share.
pub fn qualifier<R: Runtime>(app: &AppHandle<R>) -> impl JobHandler {
    Qualifier::new(
        crate::scan::system_volumes,
        crate::fingerprint::shared_first(app),
    )
}

/// Queues a quality job if any file is due one, unless one is already
/// queued (a running one is asked to run once more). Called when the
/// fingerprints are in (the scan chain), so the files have audio hashes.
pub(crate) fn request<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    let due = writer.call(|c| {
        let gate = ReadGate::for_job(c)?;
        ledger::any_due(c, gate.allows(true))
    })?;
    if due {
        chain::queue_once(writer, quality_job(None), enqueue)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
