//! Full-text search over the tracks (1cA-6; ROADMAP 1.11 "What search
//! covers"). This is the index, how it is kept current, and the query
//! function both lists filter by. (The search box is All music's; under it,
//! "N hidden files match" is [`crate::hidden`]'s, 1cB-6.)
//!
//! - **What is searched** (owner decision, 2026-10-12): title, artist,
//!   album, remixer, label, genre, comment and file name. Capitals and
//!   accents don't matter, and a word matches from its start. Results have
//!   no ranking: the lists keep their own order.
//! - **One row per track** (`recording`), because both lists are per track.
//!   The title and artist are what the lists show (computed by
//!   [`crate::send_values::shown_fields`], the lists' own rule, so rekordbox's
//!   title is found for a Library track it knows); the other five come from
//!   the same source. The file name column holds the names of every file of
//!   the track, hidden ones too, without folder and extension
//!   ([`settle::file_name_text`] is the one place that decides it).
//! - **Hidden files** aren't marked in the index: [`find`] reads which of a
//!   matching track's files are hidden (a file the user hid, a cleared Inbox
//!   file, an extra copy) when it runs, so the mark can't go stale.
//! - **Kept current by a job** (`index`). Database triggers only note
//!   which tracks are due (`search_due`), because the shown title can't be
//!   worked out in SQL; the job works the text out in Rust, a page of
//!   tracks per transaction, and removes a track from `search_due` in the
//!   same transaction that writes its row. After an upgrade every track is
//!   due. [`request`] queues the job when something is due, and [`pending`]
//!   says whether the index is behind, so a screen can say so.
//! - **Derived data:** [`rebuild`] drops every row and works them all out
//!   again; a test pins that it gives the very index the incremental path
//!   does.
//! - **The track table's rows** (`track_row`, `table_row`) are written by
//!   the same job in the same transaction, from the same text: what the
//!   table sorts and filters by (ROADMAP 1.11 "The track table").
//!
//! Limits, on purpose:
//! - A query has at most [`MAX_TERMS`] words of at most [`MAX_TERM_CHARS`]
//!   characters each; more are ignored.
//! - A comment is indexed up to [`MAX_COMMENT_CHARS`] characters.
//! - A track not in the Library never takes its text from rekordbox. A
//!   Library track does, so the rekordbox read marks every Library track due.
//! - The index doesn't see a drive being plugged in or out unless the scan
//!   flips the file's `present`; [`rebuild`] fixes any other drift.
//! - Own `label`, `genre` and `remixers` columns on `recording` (nothing
//!   fills them yet) aren't read; when a phase does, its migration must mark
//!   every track due.

mod job;
pub(crate) mod query;
mod settle;
mod table_row;

#[cfg(test)]
pub(crate) mod tests;

pub use job::{index_job, indexer, indexer_with, request};
pub use query::{find, match_expression, pending, Found, HiddenFile, Why};
pub(crate) use settle::fields_of;
pub use settle::{file_name_text, rebuild, settle, settle_if_small, settle_page};
pub use table_row::{camelot_order, flag, time_ms};
pub(crate) use table_row::{shown_file_sql, BPM_AND_KEY};

/// The most words of a query that count.
pub const MAX_TERMS: usize = 12;
/// The most characters of a word that count.
pub const MAX_TERM_CHARS: usize = 64;
/// The most characters of a comment that are indexed.
pub const MAX_COMMENT_CHARS: usize = 1000;
