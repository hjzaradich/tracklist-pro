//! The track table's data (1cA-3, 1cA-4; ROADMAP 1.11 "The track table"):
//! one query behind the Library, All music and a crate's tracks. The
//! frontend asks for a window of rows (`offset`, `limit`) of a source, in an
//! order, through filters and a search, and gets the rows of that window
//! and, when it asks, how many rows there are in all. Nothing is ever
//! loaded for the whole library: at 100,000 tracks a window is one indexed
//! query and a fixed number of small ones for its rows.
//!
//! - **Sources:** the Library, All music (every track that has a file that
//!   isn't hidden, `crate::hidden`) and
//!   one crate (in the order the tracks were added, unless sorted).
//! - **The order and the filters** read `track_row`, the derived table the
//!   index job keeps (`search::table_row`): SQL sorts and filters, never
//!   Rust. A blank value always sorts last, whichever way the column is sorted, and ties fall
//!   back to the column's own tie-break (the title, for text a list is
//!   sorted by; then the track's id), so the order is the same every time
//!   and a window never repeats or skips a row. With no sort, the Library
//!   and All music keep the Library list's order (title, artist, id).
//!   There is an index for each ascending sort only; **descending is the
//!   ascending order read backwards** (so tied tracks come in the reverse
//!   of their ascending order), with the blank values moved to the end:
//!   the rows with a value backwards, then the blank rows backwards
//!   ([`Backwards`]), each a scan of the index with nothing to sort. A
//!   search, which finds its rows through the full-text index, and a crate,
//!   which reads its own entries, are not in an index's order; they sort
//!   their (few) rows, at about 50 to 120 ms for a few thousand.
//! - **Search** is the full-text index's (`search::match_expression`): the
//!   rows are those of the tracks the text finds, narrowed by the filters.
//! - **What a row shows** is worked out again for the rows of the window
//!   only, by the code the lists use (`search::fields_of`: rekordbox's
//!   values for a Library track it knows, the file's tags otherwise), so a
//!   row is never wrong for the index being a moment behind; only its place
//!   in the order can be. `behind` says whether it is.
//! - **Filters** (`TableFilter`) all apply together, except the format
//!   filters, which are alternatives: any of the formats named.
//!
//! Read-only: only the database is read, and no audio file is opened.

mod filters;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::crates::{CrateId, CratesError};
use crate::db::{ReadPool, Writer};
use crate::fragile::{self, FragileDirs, FragileReason, Located};
use crate::ipc::IpcError;
use crate::library::{
    self, BetterFile, LibraryTrackId, LinkedFile, StoredFile, FILE_COLUMNS, FILE_JOINS,
};
use crate::paths::Volumes;
use crate::scan::system_volumes;
use crate::search::{self, flag, time_ms};

pub use filters::TableFilter;

/// The most rows one window holds.
pub const MAX_WINDOW: u32 = 500;

/// Which tracks the table lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TableSource {
    /// The Library tracks.
    Library,
    /// Every track that has a file.
    AllMusic,
    /// The Library tracks in one crate, in the order they were added.
    Crate { id: CrateId },
}

/// A column the table can be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SortColumn {
    Title,
    Artist,
    Bpm,
    Key,
    Time,
    Genre,
    Rating,
    DateAdded,
    File,
    Folder,
}

impl SortColumn {
    /// The `track_row` column the order is by, whether it can be blank
    /// (blanks sort last), and what ties fall back to. Each of these is one
    /// of the table's indexes (migration 0032).
    fn column(self) -> (&'static str, bool, &'static [&'static str]) {
        match self {
            SortColumn::Title => ("title_key", false, &["artist_key", "recording_id"]),
            SortColumn::Artist => ("artist_key", true, &["title_key", "recording_id"]),
            SortColumn::Genre => ("genre_key", true, &["title_key", "recording_id"]),
            SortColumn::Bpm => ("bpm", true, &["recording_id"]),
            SortColumn::Key => ("key_order", true, &["recording_id"]),
            SortColumn::Time => ("time_ms", true, &["recording_id"]),
            SortColumn::Rating => ("rating", true, &["recording_id"]),
            SortColumn::DateAdded => ("added_at", true, &["recording_id"]),
            SortColumn::File => ("file_key", true, &["recording_id"]),
            SortColumn::Folder => ("folder_key", true, &["recording_id"]),
        }
    }
}

/// The order the table is sorted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TableSort {
    pub column: SortColumn,
    pub descending: bool,
}

/// What the table is asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TableQuery {
    pub source: TableSource,
    /// Search text (`search::match_expression`); blank filters nothing.
    pub search: Option<String>,
    pub filters: Vec<TableFilter>,
    /// `None` is the source's own order.
    pub sort: Option<TableSort>,
    /// The first row of the window, counting from 0.
    pub offset: u32,
    /// How many rows, at most [`MAX_WINDOW`].
    pub limit: u32,
    /// Also count the rows: the count costs about as much as a window, so
    /// the first window asks for it and the others don't.
    pub with_total: bool,
}

/// A row of the table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TableRow {
    /// The track's id, as "Add to Library" takes it.
    pub recording_id: i64,
    /// Its Library track, if it is in the Library.
    pub library_track_id: Option<LibraryTrackId>,
    /// The title shown: rekordbox's for a Library track it knows, else the
    /// file's tag, else `None` (the file's name stands in for it).
    pub title: Option<String>,
    pub artist: Option<String>,
    pub bpm: Option<f64>,
    /// The key as Camelot (`8A`); the screen shows it in the user's
    /// notation.
    pub key: Option<String>,
    /// The length in milliseconds.
    pub time_ms: Option<i64>,
    pub genre: Option<String>,
    /// Stars, 0 to 5, for a Library track rekordbox knows.
    pub rating: Option<u8>,
    /// When it joined the Library, else when the scan first found it (UTC
    /// ISO-8601).
    pub added_at: Option<String>,
    /// The file the row plays: the Library track's linked file, else the
    /// file adding the track to the Library would link.
    pub file: Option<LinkedFile>,
    pub in_library: bool,
    /// One of its files is the probable match of a rekordbox track and the
    /// match can't be confirmed yet, so it can't be added to the Library.
    pub match_not_confirmed: bool,
    /// Why the linked file's location is fragile (Library tracks only).
    pub fragile: Option<FragileReason>,
    /// The linked file is gone (Library tracks only).
    pub source_missing: bool,
    /// How many conflicts with rekordbox are still open for it.
    pub open_conflicts: u32,
    pub better_file: Option<BetterFile>,
}

/// A window of rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TablePage {
    /// How many rows the query has in all; only when asked for.
    pub total: Option<u32>,
    pub offset: u32,
    pub rows: Vec<TableRow>,
    /// The index job hasn't caught up with some change: a track may be
    /// missing from the rows or out of place.
    pub behind: bool,
}

/// A track's BPM, its key (Camelot) and the date it was added.
type Extras = (Option<f64>, Option<String>, Option<String>);

/// A row before its file's path is made readable and its fragile location
/// judged.
pub struct StoredRow {
    row: TableRow,
    file: Option<StoredFile>,
}

/// The query's `FROM`, `WHERE` and `ORDER BY`, and its parameters.
struct Built {
    from_where: String,
    order_by: String,
    /// How to read a descending sort of a column that can be blank straight
    /// from its ascending index; see [`Backwards`].
    backwards: Option<Backwards>,
    params: Vec<Value>,
}

/// A descending sort is the ascending order read backwards, with the blank
/// values moved to the end (so the ties of a descending sort are those of
/// the ascending one, reversed). The table has an index for the ascending
/// sorts only, which reads backwards just as well, but not across the
/// "is blank" part of its key: that would put the blanks first. So the
/// window is read in two ranges, the rows with a value and then the blank
/// ones, each a backward scan of the index, with no sorting at all.
struct Backwards {
    /// Extra conditions for the rows that have a value.
    filled: String,
    /// Extra conditions for the blank rows.
    blank: String,
    /// The order of both ranges.
    filled_order: String,
    blank_order: String,
}

/// The ORDER BY of `sort` for `source`. A blank sorts last whichever way the
/// column goes, and the ties fall back to the column's own tie-break, so the
/// order is total and the same every time. Descending is ascending read
/// backwards (blanks still last), ties included.
///
/// Also returns, when the sort can be served by a backward scan of an index
/// ([`Backwards`]), the two ranges that do it. They give the same order as
/// the plain `ORDER BY`, which a search or a crate (where no index serves
/// the order) uses.
fn order_by(source: &TableSource, sort: Option<TableSort>) -> (String, Option<Backwards>) {
    let Some(TableSort { column, descending }) = sort else {
        return (
            match source {
                TableSource::Crate { .. } => "ce.added_at, ce.library_track_id".to_owned(),
                _ => "tr.title_key, tr.artist_key, tr.recording_id".to_owned(),
            },
            None,
        );
    };
    let (name, nullable, ties) = column.column();
    let direction = if descending { " DESC" } else { "" };
    let tie_order = ties
        .iter()
        .map(|tie| format!("tr.{tie}{direction}"))
        .collect::<Vec<_>>()
        .join(", ");
    let ordered = match nullable {
        true => format!("(tr.{name} IS NULL), tr.{name}{direction}, {tie_order}"),
        false => format!("tr.{name}{direction}, {tie_order}"),
    };
    let backwards = (descending && nullable).then(|| Backwards {
        filled: format!("(tr.{name} IS NULL) = 0 AND tr.{name} IS NOT NULL"),
        blank: format!("(tr.{name} IS NULL) = 1 AND tr.{name} IS NULL"),
        filled_order: format!("tr.{name} DESC, {tie_order}"),
        blank_order: tie_order,
    });
    (ordered, backwards)
}

/// The SQL of a query: where its rows come from, how they are filtered and
/// in what order.
fn build(query: &TableQuery, quality: Option<&str>) -> Built {
    let mut params: Vec<Value> = Vec::new();
    let mut conditions: Vec<String> = Vec::new();
    let from = match &query.source {
        TableSource::Library => {
            conditions.push(format!("(tr.flags & {}) <> 0", flag::IN_LIBRARY));
            "track_row tr"
        }
        TableSource::AllMusic => {
            conditions.push(format!("(tr.flags & {}) <> 0", flag::HAS_FILE));
            // A track whose files are all hidden is not in All music: the
            // rule the Overview's count reads too (`crate::hidden`).
            conditions.push(format!(
                "tr.recording_id NOT IN ({})",
                crate::hidden::TRACKS_WITH_ONLY_HIDDEN_FILES
            ));
            "track_row tr"
        }
        TableSource::Crate { id } => {
            params.push(Value::Integer(id.0));
            conditions.push(format!(
                "ce.crate_id = ?{} AND ce.kind = 'member'",
                params.len()
            ));
            "crate_entry ce
             JOIN library_track lt ON lt.id = ce.library_track_id
             JOIN track_row tr ON tr.recording_id = lt.recording_id"
        }
    };
    let mut searched = false;
    if let Some(expression) = query.search.as_deref().and_then(search::match_expression) {
        searched = true;
        params.push(Value::Text(expression));
        conditions.push(format!(
            "tr.recording_id IN (SELECT rowid FROM search_track WHERE search_track MATCH ?{})",
            params.len()
        ));
    }
    let (order_by, backwards) = order_by(&query.source, query.sort);
    // A search is read by rowid from the index of words, and a crate from
    // its own entries: neither is in the order of a sort index, so there is
    // nothing for a backward scan to read.
    let indexed = !searched && !matches!(query.source, TableSource::Crate { .. });
    conditions.extend(filters::conditions(&query.filters, quality, &mut params));
    Built {
        from_where: format!("FROM {from} WHERE {}", conditions.join(" AND ")),
        order_by,
        backwards: backwards.filter(|_| indexed),
        params,
    }
}

/// Whether the crate exists (and is a static one, the kind that holds
/// tracks).
fn crate_exists(conn: &Connection, id: CrateId) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM crate WHERE id = ?1 AND kind = 'static')",
        [id.0],
        |r| r.get(0),
    )
}

/// The ids a `SELECT` of `from_where` gives, in `order_by`, from `offset`.
fn select_ids(
    conn: &Connection,
    from_where: &str,
    order_by: &str,
    params: &[Value],
    limit: u32,
    offset: u32,
) -> rusqlite::Result<Vec<i64>> {
    let mut params = params.to_vec();
    params.push(Value::Integer(i64::from(limit)));
    params.push(Value::Integer(i64::from(offset)));
    let sql = format!(
        "SELECT tr.recording_id {from_where} ORDER BY {order_by} LIMIT ?{} OFFSET ?{}",
        params.len() - 1,
        params.len()
    );
    conn.prepare_cached(&sql)?
        .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?
        .collect()
}

/// How many rows `from_where` has.
fn count_rows(conn: &Connection, from_where: &str, params: &[Value]) -> rusqlite::Result<u32> {
    let n: i64 = conn
        .prepare_cached(&format!("SELECT count(*) {from_where}"))?
        .query_row(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The ids of the tracks in the window, in order, and the count when asked.
fn window_ids(conn: &Connection, query: &TableQuery) -> rusqlite::Result<(Vec<i64>, Option<u32>)> {
    window_ids_up_to(conn, query, MAX_WINDOW)
}

/// [`window_ids`] with the window at most `most` rows.
fn window_ids_up_to(
    conn: &Connection,
    query: &TableQuery,
    most: u32,
) -> rusqlite::Result<(Vec<i64>, Option<u32>)> {
    // Review's quality problems are worked out once for the count and the window.
    let quality = match query.filters.contains(&TableFilter::QualityProblem) {
        true => Some(filters::quality_problems(conn)?),
        false => None,
    };
    let built = build(query, quality.as_deref());
    let total = match query.with_total {
        true => Some(count_rows(conn, &built.from_where, &built.params)?),
        false => None,
    };
    let limit = query.limit.min(most);
    let offset = query.offset;
    let ids = match &built.backwards {
        None => select_ids(
            conn,
            &built.from_where,
            &built.order_by,
            &built.params,
            limit,
            offset,
        )?,
        Some(b) => {
            // The rows with a value, backwards...
            let filled_where = format!("{} AND {}", built.from_where, b.filled);
            let mut ids = select_ids(
                conn,
                &filled_where,
                &b.filled_order,
                &built.params,
                limit,
                offset,
            )?;
            // ...and, if the window runs past them, the blank ones. Where
            // the blank rows start is known unless the window began beyond
            // the end of the rows with a value.
            if ids.len() < limit as usize {
                let with_value = match (ids.is_empty(), offset) {
                    (true, 0) => 0,
                    (true, _) => count_rows(conn, &filled_where, &built.params)?,
                    (false, _) => offset + ids.len() as u32,
                };
                let blank_where = format!("{} AND {}", built.from_where, b.blank);
                ids.extend(select_ids(
                    conn,
                    &blank_where,
                    &b.blank_order,
                    &built.params,
                    limit - ids.len() as u32,
                    offset.saturating_sub(with_value),
                )?);
            }
            ids
        }
    };
    Ok((ids, total))
}

/// The rows for the tracks `ids`, in that order.
fn rows_for(
    conn: &Connection,
    volumes: &impl Volumes,
    ids: &[i64],
) -> rusqlite::Result<Vec<StoredRow>> {
    let json = serde_json::to_string(ids).expect("numbers always serialize");
    let stored: HashMap<i64, library::StoredTrack> = library::stored_for_recordings(conn, ids)?
        .into_iter()
        .map(|t| (t.recording_id, t))
        .collect();
    let library_ids: HashMap<i64, LibraryTrackId> =
        stored.iter().map(|(&r, t)| (r, t.id)).collect();
    let mut fields = search::fields_of(conn, volumes, ids, &library_ids)?;

    // BPM, key and when it was added, for every track.
    let mut facts: HashMap<i64, Extras> = HashMap::new();
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT r.id, {}, COALESCE(lt.added_at, r.created_at)
         FROM recording r
         LEFT JOIN library_track lt ON lt.recording_id = r.id
         WHERE r.id IN (SELECT value FROM json_each(?1))",
        search::BPM_AND_KEY
    ))?;
    for row in stmt.query_map([&json], |r| {
        Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
    })? {
        let (id, values): (i64, Extras) = row?;
        facts.insert(id, values);
    }

    // The file each track outside the Library shows: the one adding it to
    // the Library would link (a trusted rekordbox match, `linked_file_for`),
    // else its best file, else its lowest file id. Two queries for them all.
    let mut shown_file: HashMap<i64, i64> = HashMap::new();
    let mut stmt = conn.prepare_cached(
        "SELECT rf.recording_id, rf.file_id FROM recording_file rf
         WHERE rf.recording_id IN (SELECT value FROM json_each(?1))
         ORDER BY rf.recording_id, (rf.role = 'best') DESC, rf.file_id",
    )?;
    for row in stmt.query_map([&json], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (recording, file): (i64, i64) = row?;
        shown_file.entry(recording).or_insert(file);
    }
    let mut stmt = conn.prepare_cached(
        "SELECT recording_id, file_id FROM (
             SELECT rf.recording_id, f.id AS file_id,
                    row_number() OVER (
                        PARTITION BY rf.recording_id
                        ORDER BY f.present DESC, (rf.role = 'best') DESC, f.id) AS n
             FROM rekordbox_track rt
             JOIN recording_file rf ON rf.file_id = rt.file_id
             JOIN file f ON f.id = rt.file_id
             WHERE rf.recording_id IN (SELECT value FROM json_each(?1))
               AND rt.relink_probable = 0)
         WHERE n = 1",
    )?;
    for row in stmt.query_map([&json], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (recording, file): (i64, i64) = row?;
        shown_file.insert(recording, file);
    }
    for track in stored.values() {
        if let Some(file) = &track.file {
            shown_file.insert(track.recording_id, file.file_id);
        }
    }

    // Where those files are, and how long they are.
    let file_ids = serde_json::to_string(&shown_file.values().copied().collect::<Vec<_>>())
        .expect("numbers always serialize");
    let mut places: HashMap<i64, StoredFile> = HashMap::new();
    let mut durations: HashMap<i64, i64> = HashMap::new();
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {FILE_COLUMNS}, f.duration_ms FROM file f {FILE_JOINS}
         WHERE f.id IN (SELECT value FROM json_each(?1))"
    ))?;
    for row in stmt.query_map([&file_ids], |r| {
        Ok((
            library::file_from(r, 0)?,
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(6)?,
        ))
    })? {
        let (place, id, duration) = row?;
        places.extend(place.map(|place| (id, place)));
        durations.extend(duration.map(|duration| (id, duration)));
    }

    // The tracks outside the Library whose match to a rekordbox track can't
    // be confirmed yet (`library::match_not_confirmed`, for all at once).
    let mut unconfirmed: HashSet<i64> = HashSet::new();
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT rf.recording_id
         FROM rekordbox_track rt JOIN recording_file rf ON rf.file_id = rt.file_id
         WHERE rf.recording_id IN (SELECT value FROM json_each(?1)) AND rt.relink_probable = 1
           AND NOT EXISTS (SELECT 1 FROM rekordbox_track rt2
                           JOIN recording_file rf2 ON rf2.file_id = rt2.file_id
                           WHERE rf2.recording_id = rf.recording_id AND rt2.relink_probable = 0)",
    )?;
    for row in stmt.query_map([&json], |r| r.get(0))? {
        unconfirmed.insert(row?);
    }

    let mut rows = Vec::with_capacity(ids.len());
    for &id in ids {
        let library_track = stored.get(&id);
        let (file, file_id) = match library_track {
            // A Library track shows its linked file, as the Library list does.
            Some(track) => (track.file.clone(), track.file.as_ref().map(|f| f.file_id)),
            None => {
                let file_id = shown_file.get(&id).copied();
                (file_id.and_then(|f| places.get(&f).cloned()), file_id)
            }
        };
        let duration = file_id.and_then(|file_id| durations.get(&file_id).copied());
        let shown = fields.remove(&id).unwrap_or_default();
        let (bpm, key, added_at) = facts.remove(&id).unwrap_or_default();
        let blank = |text: String| (!text.trim().is_empty()).then_some(text);
        rows.push(StoredRow {
            row: TableRow {
                recording_id: id,
                library_track_id: library_track.map(|t| t.id),
                title: blank(shown.title),
                artist: blank(shown.artist),
                bpm,
                key,
                time_ms: time_ms(shown.total_time_s, duration),
                genre: blank(shown.genre),
                rating: shown.stars,
                added_at,
                file: None,
                in_library: library_track.is_some(),
                match_not_confirmed: library_track.is_none() && unconfirmed.contains(&id),
                fragile: None,
                source_missing: library_track.is_some_and(|t| t.source_missing),
                open_conflicts: library_track.map_or(0, |t| t.open_conflicts),
                better_file: library_track.and_then(|t| t.better_file),
            },
            file,
        });
    }
    Ok(rows)
}

/// What the read hands on: the page's rows and where their files are.
pub struct Read {
    total: Option<u32>,
    rows: Vec<StoredRow>,
    located: Vec<Located>,
    behind: bool,
}

/// Reads a window, or `None` if the crate asked for doesn't exist.
pub fn read(
    conn: &Connection,
    volumes: &impl Volumes,
    query: &TableQuery,
) -> rusqlite::Result<Option<Read>> {
    if let TableSource::Crate { id } = &query.source {
        if !crate_exists(conn, *id)? {
            return Ok(None);
        }
    }
    let (ids, total) = window_ids(conn, query)?;
    let rows = rows_for(conn, volumes, &ids)?;
    let files: Vec<i64> = rows
        .iter()
        .filter(|r| r.row.in_library)
        .filter_map(|r| r.file.as_ref().map(|f| f.file_id))
        .collect();
    Ok(Some(Read {
        total,
        rows,
        located: fragile::locate(conn, &files)?,
        behind: search::pending(conn)?,
    }))
}

/// The most ids one request for ids holds (see [`ids`]).
pub const MAX_IDS: u32 = 1_000_000;

/// The ids of the tracks (`recording` ids, as `TableRow::recording_id` has
/// them) in rows `offset` to `offset + limit` of a query, in the order the
/// table lists them, or `None` if the crate asked for doesn't exist. It is
/// the window's own query, so the ids and the rows can't disagree, but it
/// works out no row: the table asks for it to select a range or all of its
/// tracks (Shift+click, Ctrl+A; 1cB-8), which it can't do from the windows
/// it holds. Holds [`MAX_IDS`] at most.
pub fn ids(conn: &Connection, query: &TableQuery) -> rusqlite::Result<Option<Vec<i64>>> {
    if let TableSource::Crate { id } = &query.source {
        if !crate_exists(conn, *id)? {
            return Ok(None);
        }
    }
    let query = TableQuery {
        with_total: false,
        ..query.clone()
    };
    Ok(Some(window_ids_up_to(conn, &query, MAX_IDS)?.0))
}

/// The page for the frontend: each file's path made readable and the
/// fragile locations judged against `dirs`.
pub fn page(read: Read, offset: u32, volumes: &impl Volumes, dirs: &FragileDirs) -> TablePage {
    let fragile = fragile::judge(&read.located, volumes, dirs);
    TablePage {
        total: read.total,
        offset,
        rows: read
            .rows
            .into_iter()
            .map(|stored| {
                let mut row = stored.row;
                if let Some(file) = &stored.file {
                    row.fragile = row
                        .in_library
                        .then(|| fragile.get(&file.file_id).copied())
                        .flatten();
                    row.file = Some(file.shown(volumes));
                }
                row
            })
            .collect(),
        behind: read.behind,
    }
}

/// A window of the track table: the rows of a source, searched, filtered
/// and sorted, with how many there are in all when asked.
///
/// When the index is behind (`behind`), the screen asks for the index job, as
/// the search module intends ("a screen asks for it when `pending` says the
/// index is behind"); the job ending reloads the lists.
#[tauri::command]
#[specta::specta]
pub async fn track_table_page(
    reads: State<'_, ReadPool>,
    writer: State<'_, Writer>,
    jobs: State<'_, crate::jobs::JobQueue>,
    query: TableQuery,
) -> Result<TablePage, IpcError> {
    let volumes = system_volumes();
    let found = reads.read(|conn| read(conn, &volumes, &query))?;
    let Some(found) = found else {
        return Err(CratesError::NotFound.into());
    };
    if found.behind {
        if let Err(e) = search::request(&writer, |job| jobs.enqueue(job)) {
            eprintln!("track_table_page: couldn't ask for the index: {e:?}");
        }
    }
    Ok(page(found, query.offset, &volumes, &FragileDirs::system()))
}

/// The ids of the tracks in a range of a query's rows, to select them
/// (1cB-8). `with_total` is ignored.
#[tauri::command]
#[specta::specta]
pub async fn track_table_ids(
    reads: State<'_, ReadPool>,
    query: TableQuery,
) -> Result<Vec<i64>, IpcError> {
    match reads.read(|conn| ids(conn, &query))? {
        Some(found) => Ok(found),
        None => Err(CratesError::NotFound.into()),
    }
}

/// Where one track is in a query's rows (1cA-5): the same source, search,
/// filters and sort as a [`TableQuery`], and the track to find.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TablePositionQuery {
    pub source: TableSource,
    pub search: Option<String>,
    pub filters: Vec<TableFilter>,
    pub sort: Option<TableSort>,
    /// The track to find (`TableRow::recording_id`).
    pub recording_id: i64,
}

/// The row index (counting from 0) of the track in the rows the query
/// lists, or `None` if it isn't one of them; the outer `None` is a crate
/// that doesn't exist.
///
/// The query is [`build`]'s, the one a window is read with: the same
/// `FROM`, `WHERE` (search and filters) and `ORDER BY`, with ties and blanks
/// ordered as they are there, so this index and a window's rows can't
/// disagree. The ids are read in that order and counted up to the track,
/// which is a scan of the index for a sort (nothing to sort) and stops at
/// the track; it is not a window, so no row is worked out.
pub fn position(
    conn: &Connection,
    asked: &TablePositionQuery,
) -> rusqlite::Result<Option<Option<u32>>> {
    if let TableSource::Crate { id } = &asked.source {
        if !crate_exists(conn, *id)? {
            return Ok(None);
        }
    }
    let query = TableQuery {
        source: asked.source.clone(),
        search: asked.search.clone(),
        filters: asked.filters.clone(),
        sort: asked.sort,
        offset: 0,
        limit: 0,
        with_total: false,
    };
    let quality = match query.filters.contains(&TableFilter::QualityProblem) {
        true => Some(filters::quality_problems(conn)?),
        false => None,
    };
    let built = build(&query, quality.as_deref());
    // The rows in order: one range, or, for a descending sort of a column
    // that can be blank, the rows with a value and then the blank ones, as a
    // window reads them.
    let ranges = match &built.backwards {
        None => vec![(built.from_where.clone(), built.order_by.clone())],
        Some(b) => vec![
            (
                format!("{} AND {}", built.from_where, b.filled),
                b.filled_order.clone(),
            ),
            (
                format!("{} AND {}", built.from_where, b.blank),
                b.blank_order.clone(),
            ),
        ],
    };
    let mut before: u32 = 0;
    for (from_where, order) in ranges {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT tr.recording_id {from_where} ORDER BY {order}"
        ))?;
        let mut rows = stmt.query(rusqlite::params_from_iter(built.params.iter()))?;
        while let Some(row) = rows.next()? {
            if row.get::<_, i64>(0)? == asked.recording_id {
                return Ok(Some(Some(before)));
            }
            before += 1;
        }
    }
    Ok(Some(None))
}

/// Where a track is in the rows a query lists, for the table to keep it
/// selected, and in view, when the order or the filters change; `None` if
/// the track isn't one of them.
#[tauri::command]
#[specta::specta]
pub async fn track_table_position(
    reads: State<'_, ReadPool>,
    query: TablePositionQuery,
) -> Result<Option<u32>, IpcError> {
    match reads.read(|conn| position(conn, &query))? {
        Some(found) => Ok(found),
        None => Err(CratesError::NotFound.into()),
    }
}

/// The file formats (`file.sniffed_format`, e.g. `flac`) the source's tracks
/// are in, for the Format filter: the formats the filter can offer.
pub fn formats(conn: &Connection, source: &TableSource) -> rusqlite::Result<Option<Vec<String>>> {
    let query = TableQuery {
        source: source.clone(),
        search: None,
        filters: Vec::new(),
        sort: None,
        offset: 0,
        limit: 0,
        with_total: false,
    };
    if let TableSource::Crate { id } = source {
        if !crate_exists(conn, *id)? {
            return Ok(None);
        }
    }
    let built = build(&query, None);
    let sql = format!(
        "SELECT DISTINCT tr.format {} AND tr.format IS NOT NULL ORDER BY 1",
        built.from_where
    );
    let formats = conn
        .prepare_cached(&sql)?
        .query_map(rusqlite::params_from_iter(built.params.iter()), |r| {
            r.get(0)
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(formats))
}

/// The formats the Format filter can offer for a source.
#[tauri::command]
#[specta::specta]
pub async fn track_table_formats(
    reads: State<'_, ReadPool>,
    source: TableSource,
) -> Result<Vec<String>, IpcError> {
    match reads.read(|conn| formats(conn, &source))? {
        Some(formats) => Ok(formats),
        None => Err(CratesError::NotFound.into()),
    }
}
