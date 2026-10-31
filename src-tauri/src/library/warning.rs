//! The warning before a low-bitrate file joins the Library (1bE-8; ROADMAP
//! 1.6, owner decisions 2026-10-10 and 2026-10-12).
//!
//! **The rule lives here, not on the screen.** [`promote`](super::promote)
//! refuses to add a file that has the `low bitrate` finding unless the
//! caller says the user confirmed it, and answers with
//! [`PromoteOutcome::NeedsConfirmation`] (a code and its numbers, never
//! text). The frontend turns that into the warning.
//!
//! - **It is the finding that counts, not the headline verdict.** A file
//!   keeps one verdict (the first that applies), so a cut-off file that is
//!   also below the low line shows `truncated`; it still has the
//!   `low bitrate` finding, and still warns.
//! - **The finding is current when it's checked.** A stored verdict that is
//!   stale (or missing) is settled for that file first, from the stored
//!   measurements and the file's headers; no audio is opened. A file the
//!   read stage hasn't given a bitrate has no finding, so it adds without a
//!   warning.
//! - **One named exception: the rekordbox offer.** `start::add_offered`
//!   adds the tracks rekordbox already knows without a warning per track;
//!   they show under "Quality problems" in Review afterwards (owner
//!   decision, 2026-10-12).
//! - **And a crate add that can't ask** (1cB-23). Adding tracks outside the
//!   Library to a crate (`crates::add_recordings_on`) links them in the same
//!   operation, but it leaves out any track that has this warning, because
//!   there is nobody to agree to it; the track is counted, with its reason,
//!   and is added by hand. These are the only other callers of
//!   [`insert_linked`](super::insert_linked); a test fails if a fourth
//!   appears.

use std::collections::HashMap;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::Promoted;
use crate::quality::verdict::Finding;
use crate::quality::store::CHUNK;
use crate::quality::{settle_files, stored_verdict, stored_verdicts};

/// Why an add needs the user's go-ahead first. A code and its numbers; the
/// words come with the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum AddWarning {
    /// The file has the `low bitrate` finding.
    #[serde(rename_all = "camelCase")]
    LowBitrate { kbps: u32 },
}

/// What asking to add a track to the Library came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum PromoteOutcome {
    /// The track is in the Library now (or was already: `added` says).
    Done(Promoted),
    /// Nothing was added or recorded. Ask again with the matching
    /// confirmation to add it anyway.
    NeedsConfirmation { warning: AddWarning },
}

#[cfg(test)]
impl PromoteOutcome {
    /// The add's result, for a test that expects no warning.
    pub(crate) fn done(self) -> Promoted {
        match self {
            PromoteOutcome::Done(promoted) => promoted,
            other => panic!("expected the track to be added, got {other:?}"),
        }
    }
}

/// What the user has already agreed to for this add. Nothing is agreed to
/// unless the caller says so.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Confirmed {
    /// The user saw the low-bitrate warning and chose to add anyway.
    pub low_bitrate: bool,
}

/// The warning adding `file_id` calls for, if any. Settles the file's
/// verdict first, so the finding is current; opens no audio.
pub fn warning_for(conn: &mut Connection, file_id: i64) -> rusqlite::Result<Option<AddWarning>> {
    settle_files(conn, &[file_id])?;
    let found = stored_verdict(conn, file_id)?;
    Ok(low_bitrate_in(&found.findings))
}

/// [`warning_for`] for many files at once: the warning each of `file_ids`
/// calls for, among those that call for one. Settles the stale verdicts a
/// page of files at a time and reads the findings of the whole page in one
/// query, instead of a settle and a read per file; the answers are the same.
pub fn warnings_for(
    conn: &mut Connection,
    file_ids: &[i64],
) -> rusqlite::Result<HashMap<i64, AddWarning>> {
    let mut asked: Vec<i64> = file_ids.to_vec();
    asked.sort_unstable();
    asked.dedup();
    let mut warnings = HashMap::new();
    for page in asked.chunks(CHUNK) {
        settle_files(conn, page)?;
        for (file_id, found) in stored_verdicts(conn, page)? {
            if let Some(warning) = low_bitrate_in(&found.findings) {
                warnings.insert(file_id, warning);
            }
        }
    }
    Ok(warnings)
}

/// The warning a file's findings call for, if any.
fn low_bitrate_in(findings: &[Finding]) -> Option<AddWarning> {
    findings.iter().find_map(|finding| match finding {
        Finding::LowBitrate { kbps } => Some(AddWarning::LowBitrate { kbps: *kbps }),
        _ => None,
    })
}

/// Whether `warning` has been agreed to.
pub(super) fn is_confirmed(warning: AddWarning, confirmed: Confirmed) -> bool {
    match warning {
        AddWarning::LowBitrate { .. } => confirmed.low_bitrate,
    }
}
