//! The Library's size and change stamp (1cC, audit 1cB item A1).
//!
//! The palette, the Overview and the send's "Library changed since the last
//! read" note used to fetch the whole Library list, 100,000 rows with their
//! titles and paths, to read its length and to see whether it had come back
//! different. [`library_summary`] answers both for the price of a count:
//!
//! - **`tracks`** is how many Library tracks there are (`count(*)` over the
//!   narrowest index of `library_track`).
//! - **`change_stamp`** is a number that moves whenever any row the Library
//!   list is made of changes ([`LISTED_TABLES`]): a track added or removed,
//!   retitled (a tag, rekordbox's entry or the user's own value), relinked,
//!   a file marked missing, a conflict opened or closed. The screens
//!   compare it with the one they saw last; they never read what it means.
//!
//! **How it is kept.** The writer is the only connection that changes the
//! database, so it is where every change is seen. [`LibraryStamp::install`]
//! puts SQLite's update, commit and rollback hooks on the writer's
//! connection: each changed row of a listed table marks the open
//! transaction, a commit of a marked transaction moves the stamp once, and a
//! rollback forgets the mark. Nothing is stored (the stamp starts again with
//! the app, as the screens do) and no table, trigger or migration is added.
//! Reading it is an atomic load, so it never waits behind a long write.
//!
//! **What the stamp promises.** Every committed change to a listed table
//! moves it, and it moves before a reader can see the change (the commit
//! hook runs before the commit is visible), so a read never shows a change
//! the stamp has not counted. It can run a moment ahead of what a read
//! shows; asking again settles that, and every change ends in a command or a
//! job that makes the screens ask again. It is coarser than the list: a
//! listed table changing in a way the list does not show (a file's hash)
//! moves it too, because the hook sees rows, not columns. That is the safe
//! side for what it feeds: the note stops being shown when the stamp moves,
//! so a move too many hides a note, and a move too few would leave a stale
//! one up.
//!
//! **One thing the hooks do not see:** a DELETE with no WHERE clause on a
//! table that has no triggers and nothing referring to it, which SQLite
//! clears in one step without visiting the rows. No code does that to a
//! listed table, and a test (`a_delete_with_no_where_clause_is_not_seen_by_the_hooks_so_no_code_makes_one`)
//! scans the source for it; a delete that must empty a table says `WHERE 1`.
//!
//! **Not in the stamp:** the crates (the screens watch the crate list on its
//! own), and the drives being plugged in or out, which change what the list
//! says about a file with no database write (the scan marks the files, and
//! that write moves the stamp).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{DbError, Writer};

/// The tables the Library list is made from: the tracks, what they are
/// called (the track's own title, tags on the files, rekordbox's entry, the
/// user's own value), where their files are and whether those are there,
/// the better file and the open conflicts.
///
/// A test (`the_stamp_watches_every_table_the_library_list_reads`) holds this
/// list to the tables the list's query reads. A table that the
/// list starts to read through some other query must be added here.
///
/// Every table here must be a rowid table: SQLite's update hook does not fire
/// for WITHOUT ROWID tables (a test holds this too). A table the list reads
/// only alongside another listed one (`file_stage` is written with the hash
/// write that also updates `file`) is covered by that one, not named.
pub const LISTED_TABLES: &[&str] = &[
    "library_track",
    "recording",
    "recording_file",
    "file",
    "music_folder",
    "volume",
    "rekordbox_track",
    "app_value",
    "better_file",
    "conflict",
];

/// What the screens need to know about the Library without its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    /// How many Library tracks there are.
    pub tracks: u32,
    /// Moves whenever any row the Library list is made of changes (a track
    /// added, removed, retitled or relinked). Only ever compared with the
    /// last one seen: for equality, never for order.
    pub change_stamp: u32,
}

#[derive(Default)]
struct Shared {
    /// A listed row changed in the transaction that is open.
    marked: AtomicBool,
    /// Commits of marked transactions so far (wraps).
    stamp: AtomicU32,
}

/// The change stamp: a cheap handle on a counter the writer's hooks keep.
/// Clones share it.
#[derive(Clone, Default)]
pub struct LibraryStamp {
    shared: Arc<Shared>,
}

impl LibraryStamp {
    /// Starts watching `writer`'s connection. Call it once, when the writer
    /// is opened: changes made before it are not counted (the stamp is only
    /// ever compared with an earlier one). Replaces any update, commit or
    /// rollback hook the connection had.
    pub fn install(writer: &Writer) -> Result<LibraryStamp, DbError> {
        let stamp = LibraryStamp::default();
        let (on_row, on_commit, on_rollback) = (
            stamp.shared.clone(),
            stamp.shared.clone(),
            stamp.shared.clone(),
        );
        writer.call(move |conn| {
            conn.update_hook(Some(
                move |_action: rusqlite::hooks::Action, _db: &str, table: &str, _row: i64| {
                    if LISTED_TABLES.contains(&table) {
                        on_row.marked.store(true, Ordering::SeqCst);
                    }
                },
            ))?;
            conn.commit_hook(Some(move || {
                if on_commit.marked.swap(false, Ordering::SeqCst) {
                    on_commit.stamp.fetch_add(1, Ordering::SeqCst);
                }
                // Never turns the commit into a rollback.
                false
            }))?;
            conn.rollback_hook(Some(move || {
                on_rollback.marked.store(false, Ordering::SeqCst);
            }))?;
            Ok(())
        })?;
        Ok(stamp)
    }

    /// The stamp now.
    pub fn get(&self) -> u32 {
        self.shared.stamp.load(Ordering::SeqCst)
    }
}

/// The Library's size and stamp. The stamp is taken first: a change that
/// lands between the two moves the stamp the next time, so it is not lost.
pub(super) fn summary(conn: &Connection, stamp: &LibraryStamp) -> rusqlite::Result<LibrarySummary> {
    let change_stamp = stamp.get();
    let tracks = conn.query_row("SELECT count(*) FROM library_track", [], |r| r.get(0))?;
    Ok(LibrarySummary {
        tracks,
        change_stamp,
    })
}
