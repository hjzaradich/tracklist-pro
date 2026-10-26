//! The track table's data (1cA-3; ROADMAP 1.11 "The track table"):
//! one query behind the Library, All music and a crate's tracks. The
//! frontend asks for a window of rows (`offset`, `limit`) of a source, in an
//! order, through a search, and gets the rows of that window
//! and, when it asks, how many rows there are in all. Nothing is ever
//! loaded for the whole library: at 100,000 tracks a window is one indexed
//! query and a fixed number of small ones for its rows.
//!
//! - **Sources:** the Library, All music (every track that has a file) and
//!   one crate (in the order the tracks were added, unless sorted).
//! - **The order** reads `track_row`, the derived table the index job
//!   keeps (`search::table_row`): SQL sorts, never Rust, and every sort is
//!   read from an index alone. A
//!   blank value always sorts last, whichever way the column is sorted, and
//!   ties fall back to the column's own tie-break (the title, for text a
//!   list is sorted by; then the track's id), so the order is the same
//!   every time and a window never repeats or skips a row. With no sort,
//!   the Library and All music keep the Library list's order (title,
//!   artist, id).
//! - **Search** is the full-text index's (`search::match_expression`): the
//!   rows are those of the tracks the text finds.
//! - **What a row shows** is worked out again for the rows of the window
//!   only, by the code the lists use (`search::fields_of`: rekordbox's
//!   values for a Library track it knows, the file's tags otherwise), so a
//!   row is never wrong for the index being a moment behind; only its place
//!   in the order can be. `behind` says whether it is.
//!
//! Read-only: only the database is read, and no audio file is opened.

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
    /// of the table's indexes, in both directions (migration 0032).
    fn column(self) -> (&'static str, bool, &'static str) {
        match self {
            SortColumn::Title => ("title_key", false, "tr.artist_key, tr.recording_id"),
            SortColumn::Artist => ("artist_key", true, "tr.title_key, tr.recording_id"),
            SortColumn::Genre => ("genre_key", true, "tr.title_key, tr.recording_id"),
            SortColumn::Bpm => ("bpm", true, "tr.recording_id"),
            SortColumn::Key => ("key_order", true, "tr.recording_id"),
            SortColumn::Time => ("time_ms", true, "tr.recording_id"),
            SortColumn::Rating => ("rating", true, "tr.recording_id"),
            SortColumn::DateAdded => ("added_at", true, "tr.recording_id"),
            SortColumn::File => ("file_key", true, "tr.recording_id"),
            SortColumn::Folder => ("folder_key", true, "tr.recording_id"),
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
    params: Vec<Value>,
}

/// The ORDER BY of `sort` for `source`. A blank sorts last whichever way the
/// column goes, and the ties fall back to the column's own tie-break, so the
/// order is total and the same every time.
fn order_by(source: &TableSource, sort: Option<TableSort>) -> String {
    let Some(TableSort { column, descending }) = sort else {
        return match source {
            TableSource::Crate { .. } => "ce.added_at, ce.library_track_id".to_owned(),
            _ => "tr.title_key, tr.artist_key, tr.recording_id".to_owned(),
        };
    };
    let (name, nullable, ties) = column.column();
    let direction = if descending { " DESC" } else { "" };
    match nullable {
        true => format!("(tr.{name} IS NULL), tr.{name}{direction}, {ties}"),
        false => format!("tr.{name}{direction}, {ties}"),
    }
}

/// The SQL of a query: where its rows come from, how they are filtered and
/// in what order.
fn build(query: &TableQuery) -> Built {
    let mut params: Vec<Value> = Vec::new();
    let mut conditions: Vec<String> = Vec::new();
    let from = match &query.source {
        TableSource::Library => {
            conditions.push(format!("(tr.flags & {}) <> 0", flag::IN_LIBRARY));
            "track_row tr"
        }
        TableSource::AllMusic => {
            conditions.push(format!("(tr.flags & {}) <> 0", flag::HAS_FILE));
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
    if let Some(expression) = query.search.as_deref().and_then(search::match_expression) {
        params.push(Value::Text(expression));
        conditions.push(format!(
            "tr.recording_id IN (SELECT rowid FROM search_track WHERE search_track MATCH ?{})",
            params.len()
        ));
    }
    Built {
        from_where: format!("FROM {from} WHERE {}", conditions.join(" AND ")),
        order_by: order_by(&query.source, query.sort),
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

/// The ids of the tracks in the window, in order, and the count when asked.
fn window_ids(conn: &Connection, query: &TableQuery) -> rusqlite::Result<(Vec<i64>, Option<u32>)> {
    let built = build(query);
    let total = match query.with_total {
        true => {
            let n: i64 = conn
                .prepare_cached(&format!("SELECT count(*) {}", built.from_where))?
                .query_row(rusqlite::params_from_iter(built.params.iter()), |r| {
                    r.get(0)
                })?;
            Some(u32::try_from(n).unwrap_or(u32::MAX))
        }
        false => None,
    };
    let limit = query.limit.min(MAX_WINDOW);
    let mut params = built.params.clone();
    params.push(Value::Integer(i64::from(limit)));
    params.push(Value::Integer(i64::from(query.offset)));
    let sql = format!(
        "SELECT tr.recording_id {} ORDER BY {} LIMIT ?{} OFFSET ?{}",
        built.from_where,
        built.order_by,
        params.len() - 1,
        params.len()
    );
    let ids = conn
        .prepare_cached(&sql)?
        .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
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
    let mut stmt = conn.prepare(&format!(
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
    let mut stmt = conn.prepare(
        "SELECT rf.recording_id, rf.file_id FROM recording_file rf
         WHERE rf.recording_id IN (SELECT value FROM json_each(?1))
         ORDER BY rf.recording_id, (rf.role = 'best') DESC, rf.file_id",
    )?;
    for row in stmt.query_map([&json], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (recording, file): (i64, i64) = row?;
        shown_file.entry(recording).or_insert(file);
    }
    let mut stmt = conn.prepare(
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
    let mut stmt = conn.prepare(&format!(
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
    let mut stmt = conn.prepare(
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
