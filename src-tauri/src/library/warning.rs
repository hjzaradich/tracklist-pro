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
//!   decision, 2026-10-12). It is the only other caller of
//!   [`insert_linked`](super::insert_linked); a test fails if a third
//!   appears.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::Promoted;
use crate::quality::verdict::Finding;
use crate::quality::{settle_files, stored_verdict};

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
    Ok(found.findings.iter().find_map(|finding| match finding {
        Finding::LowBitrate { kbps } => Some(AddWarning::LowBitrate { kbps: *kbps }),
        _ => None,
    }))
}

/// Whether `warning` has been agreed to.
pub(super) fn is_confirmed(warning: AddWarning, confirmed: Confirmed) -> bool {
    match warning {
        AddWarning::LowBitrate { .. } => confirmed.low_bitrate,
    }
}
