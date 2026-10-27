//! Hidden files (1cB-5, 1cB-6; ROADMAP 1.11 "Hidden filter" and "Search
//! finds hidden files too"): everything the app isn't showing, each file with
//! its reason and its way back, and how many hidden files a search matches.
//!
//! # The one rule
//!
//! A track is in All music when it has a file that isn't hidden, that is,
//! one with no `hidden_reason` ([`has_shown_file!`]). The Overview's All
//! music count, the track table's All music rows and count, and search's
//! "N hidden files match" (is the track in the results already?) all read
//! that one predicate. Nothing is stored for it: the rule is applied when a
//! query runs, so hiding a file or showing it again (or Undo of either)
//! changes the lists at once and no index job has to follow. (`track_row`'s
//! `HAS_FILE` flag stays "the track has a file"; the table adds this rule.)
//! A Library track whose files are all hidden stays in the Library.
//!
//! # What is listed
//!
//! One row per file the app isn't showing ([`page`]):
//! - a file with a `hidden_reason`: `user` (hidden by the user; way back
//!   "Show again", [`show_again`]) or `inbox_cleared` (a cleared Inbox file;
//!   Phase 2 adds "Put back in Inbox", nothing writes this reason yet);
//! - an extra copy of a track (`recording_file.role = 'extra'`), whose reason
//!   names the track and the file that was chosen as its best. Its way back
//!   would be "Use this file instead", which has to reopen a settled
//!   best-file choice in Review. Review has no such path (the best file is
//!   the ranking's, ROADMAP 1.7, and the Library link only moves when its
//!   file is gone), so it needs a design decision and is not built:
//!   [`WayBack`] has no variant for it yet.
//!
//! Nothing in the app hides a file yet: [`show_again`] is the way back, built
//! and tested against a file whose reason a test sets.

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tauri::State;

use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::library::LibraryTrackId;
use crate::ops;
use crate::paths::Volumes;
use crate::scan::system_volumes;
use crate::search::{self, Why};
use crate::send_values::file_name;
use crate::track_table::{SortColumn, TableSort};

#[cfg(test)]
mod tests;

/// SQL for "the track `$track` (a SQL expression for its id) has a file the
/// app shows": a file with no `hidden_reason`. The one place that says what
/// a shown file is; the aliases are its own, so `$track` may use any.
macro_rules! has_shown_file {
    ($track:literal) => {
        concat!(
            "EXISTS (SELECT 1 FROM recording_file shown_rf
                     JOIN file shown_f ON shown_f.id = shown_rf.file_id
                     WHERE shown_rf.recording_id = ",
            $track,
            " AND shown_f.hidden_reason IS NULL)"
        )
    };
}
pub(crate) use has_shown_file;

/// SQL for the ids of the tracks that have files and show none of them: the
/// tracks All music leaves out. Starts from the few hidden files, so it
/// costs a lookup of those and not a walk over the library.
pub(crate) const TRACKS_WITH_ONLY_HIDDEN_FILES: &str = concat!(
    "SELECT hf.recording_id FROM file hf_file
     JOIN recording_file hf ON hf.file_id = hf_file.id
     WHERE hf_file.hidden_reason IS NOT NULL AND NOT ",
    has_shown_file!("hf.recording_id")
);

/// The operation "Show again" records.
pub const SHOW_AGAIN_OPERATION: &str = "show_file_again";

/// What a row can be offered to bring the file back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WayBack {
    /// Clears the file's `hidden_reason` ("Show again").
    ShowAgain,
}

impl WayBack {
    /// The way back for a file hidden for `why`, if one is built.
    pub fn of(why: Why) -> Option<WayBack> {
        match why {
            Why::UserHid => Some(WayBack::ShowAgain),
            // Phase 2: "Put back in Inbox" (the Inbox doesn't exist yet).
            Why::ClearedInbox => None,
            // "Use this file instead" needs a decision on how a user's
            // pick sits with the ranking (see the module docs).
            Why::ExtraFile => None,
        }
    }
}

/// One file the app isn't showing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HiddenRow {
    pub file_id: i64,
    /// Its track; `None` for a hidden file the grouping hasn't placed yet.
    pub recording_id: Option<i64>,
    pub why: Why,
    pub way_back: Option<WayBack>,
    /// The track as the lists name it: its title, else a file's name.
    pub track: String,
    pub artist: Option<String>,
    /// This file's name, without its folder.
    pub file: String,
    /// For an extra copy, the name of the file chosen as the track's best.
    pub best_file: Option<String>,
}

/// What the Hidden filter is asked for: a window, like the track table's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HiddenQuery {
    /// Search text (`search::match_expression`); blank filters nothing. A
    /// file is listed when its track matches.
    pub search: Option<String>,
    /// Title, Artist or File; any other column, or none, is the title.
    pub sort: Option<TableSort>,
    pub offset: u32,
    /// At most [`crate::track_table::MAX_WINDOW`].
    pub limit: u32,
    pub with_total: bool,
}

/// A window of hidden files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HiddenPage {
    pub total: Option<u32>,
    pub offset: u32,
    pub rows: Vec<HiddenRow>,
    /// The index job hasn't caught up with some change.
    pub behind: bool,
}

/// The files that aren't shown, with their track: a file with a reason, and
/// an extra copy.
const HIDDEN: &str = "WITH hidden(file_id, recording_id) AS (
        SELECT f.id, rf.recording_id FROM file f
        LEFT JOIN recording_file rf ON rf.file_id = f.id
        WHERE f.hidden_reason IS NOT NULL
        UNION
        SELECT file_id, recording_id FROM recording_file WHERE role = 'extra')";

/// The `ORDER BY` of a sort: blanks last, whichever way it goes, then the
/// file's id.
fn order_by(sort: Option<TableSort>) -> String {
    let (column, descending) = match sort {
        Some(TableSort {
            column: SortColumn::Artist,
            descending,
        }) => ("tr.artist_key", descending),
        Some(TableSort {
            column: SortColumn::File,
            descending,
        }) => ("f.rel_path_key", descending),
        Some(TableSort { descending, .. }) => ("tr.title_key", descending),
        None => ("tr.title_key", false),
    };
    let direction = if descending { " DESC" } else { "" };
    format!("({column} IS NULL), {column}{direction}, h.file_id")
}

/// A window of the hidden files, searched and sorted, with how many there
/// are when asked. Read-only.
pub fn page(
    conn: &Connection,
    volumes: &impl Volumes,
    query: &HiddenQuery,
) -> rusqlite::Result<HiddenPage> {
    let mut params: Vec<Value> = Vec::new();
    let mut condition = String::new();
    if let Some(expression) = query.search.as_deref().and_then(search::match_expression) {
        params.push(Value::Text(expression));
        condition = "WHERE h.recording_id IN
                         (SELECT rowid FROM search_track WHERE search_track MATCH ?1)"
            .to_owned();
    }
    let from = format!(
        "FROM hidden h JOIN file f ON f.id = h.file_id
         LEFT JOIN track_row tr ON tr.recording_id = h.recording_id {condition}"
    );
    let total = match query.with_total {
        true => {
            let n: i64 = conn
                .prepare_cached(&format!("{HIDDEN} SELECT count(*) {from}"))?
                .query_row(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
            Some(u32::try_from(n).unwrap_or(u32::MAX))
        }
        false => None,
    };
    params.push(Value::Integer(i64::from(
        query.limit.min(crate::track_table::MAX_WINDOW),
    )));
    params.push(Value::Integer(i64::from(query.offset)));
    let sql = format!(
        "{HIDDEN} SELECT h.file_id, h.recording_id, f.hidden_reason, f.rel_path {from}
         ORDER BY {} LIMIT ?{} OFFSET ?{}",
        order_by(query.sort),
        params.len() - 1,
        params.len()
    );
    type Listed = (i64, Option<i64>, Option<String>, String);
    let listed: Vec<Listed> = conn
        .prepare_cached(&sql)?
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;

    // What each track is called, and its best file.
    let mut tracks: Vec<i64> = listed.iter().filter_map(|l| l.1).collect();
    tracks.sort_unstable();
    tracks.dedup();
    let json = serde_json::to_string(&tracks).expect("numbers always serialize");
    let mut library: HashMap<i64, LibraryTrackId> = HashMap::new();
    let mut stmt = conn.prepare_cached(
        "SELECT recording_id, id FROM library_track
         WHERE recording_id IN (SELECT value FROM json_each(?1))",
    )?;
    for row in stmt.query_map([&json], |r| Ok((r.get(0)?, r.get::<_, i64>(1)?)))? {
        let (recording, id) = row?;
        library.insert(recording, LibraryTrackId(id));
    }
    let fields = search::fields_of(conn, volumes, &tracks, &library)?;
    let mut best: HashMap<i64, String> = HashMap::new();
    let mut stmt = conn.prepare_cached(
        "SELECT rf.recording_id, f.rel_path FROM recording_file rf
         JOIN file f ON f.id = rf.file_id
         WHERE rf.role = 'best' AND rf.recording_id IN (SELECT value FROM json_each(?1))",
    )?;
    for row in stmt.query_map([&json], |r| Ok((r.get(0)?, r.get::<_, String>(1)?)))? {
        let (recording, path) = row?;
        best.insert(recording, file_name(&path).to_owned());
    }

    let rows = listed
        .into_iter()
        .map(|(file_id, recording_id, reason, path)| {
            let why = Why::of(reason.as_deref());
            let shown = recording_id.and_then(|id| fields.get(&id));
            let best_file = recording_id.and_then(|id| best.get(&id).cloned());
            let blank = |text: &str| (!text.trim().is_empty()).then(|| text.to_owned());
            HiddenRow {
                file_id,
                recording_id,
                why,
                way_back: WayBack::of(why),
                track: shown
                    .and_then(|s| blank(&s.title))
                    .or_else(|| best_file.clone())
                    .unwrap_or_else(|| file_name(&path).to_owned()),
                artist: shown.and_then(|s| blank(&s.artist)),
                file: file_name(&path).to_owned(),
                best_file: best_file.filter(|_| why == Why::ExtraFile),
            }
        })
        .collect();
    Ok(HiddenPage {
        total,
        offset: query.offset,
        rows,
        behind: search::pending(conn)?,
    })
}

/// A window of the hidden files (the Hidden filter in All music).
#[tauri::command]
#[specta::specta]
pub async fn hidden_files_page(
    reads: State<'_, ReadPool>,
    query: HiddenQuery,
) -> Result<HiddenPage, IpcError> {
    let volumes = system_volumes();
    Ok(reads.read(|conn| page(conn, &volumes, &query))?)
}

/// What "Show again" did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ShownAgain {
    /// The operation to offer Undo for. `None` if the file wasn't hidden by
    /// the user (shown already, or gone): nothing changed.
    pub operation_id: Option<i64>,
}

/// Shows a file the user hid again: clears its `hidden_reason`, as one
/// operation, so Undo hides it again exactly as it was. Only a file hidden
/// by the user is touched; a cleared Inbox file goes back through the Inbox
/// (Phase 2).
pub fn show_again(conn: &mut Connection, file: i64) -> Result<ShownAgain, ops::OpsError> {
    let recorded = ops::record(
        conn,
        SHOW_AGAIN_OPERATION,
        &json!({ "fileId": file }),
        |rec| {
            let reason = rec
                .read_rows(
                    "SELECT hidden_reason FROM file WHERE id = ?1",
                    [file],
                    |r| r.get::<_, Option<String>>(0),
                )?
                .pop()
                .flatten();
            if reason.as_deref() == Some("user") {
                rec.set("file", file, "hidden_reason", Option::<String>::None)?;
            }
            Ok(())
        },
    )?;
    Ok(ShownAgain {
        operation_id: recorded.operation_id,
    })
}

/// "Show again" for a file the user hid.
#[tauri::command(async)]
#[specta::specta]
pub fn show_file_again(writer: State<'_, Writer>, file_id: i64) -> Result<ShownAgain, IpcError> {
    Ok(writer.call(move |conn| Ok(show_again(conn, file_id)))??)
}

/// How many hidden files a search matches that its results don't show
/// already (ROADMAP 1.11, owner decision 2026-10-12): the hidden files of
/// matching tracks that have no shown file, so are not in the results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HiddenMatches {
    pub count: u32,
    /// The index is behind, so the count may be off for a moment.
    pub behind: bool,
}

/// [`HiddenMatches`] for the search `text`. Text with no word to look for
/// filters nothing, so it matches no hidden file.
pub fn matches(conn: &Connection, text: &str) -> rusqlite::Result<HiddenMatches> {
    let Some(found) = search::find(conn, text)? else {
        return Ok(HiddenMatches {
            count: 0,
            behind: false,
        });
    };
    let count = found
        .hidden
        .iter()
        .filter(|h| !h.track_has_shown_file)
        .count();
    Ok(HiddenMatches {
        count: u32::try_from(count).unwrap_or(u32::MAX),
        behind: found.behind,
    })
}

/// How many hidden files `text` matches that the results don't show.
#[tauri::command]
#[specta::specta]
pub async fn hidden_files_matching(
    reads: State<'_, ReadPool>,
    text: String,
) -> Result<HiddenMatches, IpcError> {
    Ok(reads.read(|conn| matches(conn, &text))?)
}
