//! The two lists shown after each send (1aF-2; ROADMAP 1.9 rules 6 and 7,
//! §5.2): things rekordbox still holds that an XML import can't remove, so
//! the user removes them by hand.
//!
//! - **Stale playlists** ([`stale_playlists`]): playlists and folders that
//!   the app itself sent before (`sent_playlist`, written by
//!   [`crate::rekordbox_write::record_send`]), that a read has shown in
//!   rekordbox under the top-level `Crates` and `Playlists` folders, that the
//!   latest read still has, and that the app's own tree no longer has
//!   (renamed or deleted in the app). Something the user made in rekordbox
//!   is never listed, even inside those two folders, and nothing outside
//!   them is. Names are compared the way the send writer compares them
//!   (NFC, letter case and trailing whitespace ignored).
//!
//!   **A folder** is listed, once and without its contents, only when
//!   everything rekordbox has inside it (at any depth) was sent by the app
//!   too, so deleting it deletes only what the app sent and its count is
//!   exactly that. A folder that holds anything the user made isn't listed;
//!   the recorded playlists and folders inside it are, one by one, by full
//!   path.
//!
//!   The record of sent paths is cumulative. A path is marked seen once a
//!   read has shown it in rekordbox, and forgotten when a later read no
//!   longer has it (the user deleted it, [`save_tree`]; any read counts,
//!   including one of a different export, which can only list less). A path
//!   never seen stays, since the user may not have imported that send yet,
//!   and only a seen path is ever listed: a read from before a send says
//!   nothing about what the send put there. Sending a path again updates
//!   it.
//! - **Manual removals** ([`manual_removals`]): tracks removed from the
//!   Library that were sent before ([`crate::library::remove_in_rekordbox`])
//!   and that rekordbox still holds. A track leaves the list when a
//!   rekordbox read made after its removal no longer has it; a read from
//!   before the removal says nothing, and an incomplete read can't say a
//!   track is gone (it keeps the tracks it doesn't mention). A track whose
//!   sent `Location` can't be read back is matched by the read's rows matched
//!   to the track instead, shows no path, and leaves the same way.
//!
//! Both are worked out from the database alone, so nothing here depends on
//! a send having just happened, and nothing here writes a file or sends
//! anything. The playlist tree of each read is kept as one setting
//! ([`PLAYLIST_TREE`]), written in the same transaction as the read's
//! snapshot ([`save_tree`]): the snapshot only knows a playlist through
//! its tracks, so an empty playlist or folder would otherwise be invisible.
//!
//! "What the app sends" is the app's whole crate tree, not just what one
//! send carries: a send may carry only the crates that changed (rule 4),
//! and an unchanged crate isn't stale.

pub mod commands;
mod removals;
mod tree;

pub use removals::{manual_removals, ManualRemoval};
pub use tree::{
    app_tree, load_recorded, load_tree, save_tree, stale_playlists, AppTree, Recorded,
    RekordboxTree, StaleKind, StalePlaylist, TreeNode, PLAYLIST_TREE,
};

use rusqlite::Connection;
use serde::Serialize;
use specta::Type;

/// Both lists, for the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AfterSendLists {
    /// Whether rekordbox's playlists are known: false until a read made
    /// since this list existed, so an empty `stale_playlists` isn't read as
    /// "nothing to delete" when nothing was checked.
    pub playlists_checked: bool,
    pub stale_playlists: Vec<StalePlaylist>,
    pub manual_removals: Vec<ManualRemoval>,
}

/// Both lists, from the database as it is now.
pub fn lists(conn: &Connection) -> rusqlite::Result<AfterSendLists> {
    let read = load_tree(conn)?;
    Ok(AfterSendLists {
        playlists_checked: read.is_some(),
        stale_playlists: match &read {
            Some(read) => stale_playlists(read, &app_tree(conn)?, &load_recorded(conn)?),
            None => Vec::new(),
        },
        manual_removals: manual_removals(conn)?,
    })
}

#[cfg(test)]
mod tests;
