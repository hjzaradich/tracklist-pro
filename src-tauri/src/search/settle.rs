//! Working out the index rows of the tracks that are due.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use super::MAX_COMMENT_CHARS;
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
use crate::send_values::{file_name, shown_fields, shown_recording_fields, ShownFields};

/// The text a file's name adds to the `file_name` column: the name without
/// its folder and without its extension (owner decision, 2026-10-12: folder
/// names aren't searched, and neither is the extension). The one place that
/// decides it; changing it needs a [`rebuild`].
///
/// An extension is a dot followed by two to five letters or digits, at least
/// one a letter, after a name that isn't empty (so `v1.5` keeps its `.5`).
pub fn file_name_text(rel_path: &str) -> String {
    let name = file_name(rel_path);
    if let Some((stem, ext)) = name.rsplit_once('.') {
        let ext_like = (2..=5).contains(&ext.chars().count())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic());
        if ext_like && !stem.is_empty() {
            return stem.to_owned();
        }
    }
    name.to_owned()
}

/// The ids as a JSON array, for `json_each`.
fn json(ids: &[i64]) -> String {
    serde_json::to_string(ids).expect("numbers always serialize")
}

/// A row of the index, before it is written.
struct Row {
    fields: ShownFields,
    file_names: String,
}

/// Works out and writes the rows of up to `limit` due tracks, in one
/// transaction, and returns how many tracks that was. A track's old row is
/// deleted and its new one written in the same transaction that removes it
/// from `search_due`, so a crash never leaves a track with neither.
///
/// `-1` in `search_due` stands for every Library track (a rekordbox read):
/// it is replaced by them first.
pub fn settle_page(
    conn: &mut Connection,
    volumes: &impl Volumes,
    limit: usize,
) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    if tx.execute("DELETE FROM search_due WHERE recording_id = -1", [])? > 0 {
        tx.execute(
            "INSERT OR IGNORE INTO search_due (recording_id)
             SELECT recording_id FROM library_track",
            [],
        )?;
    }
    // Nothing writes any other id that isn't a track's.
    tx.execute(
        "DELETE FROM search_due WHERE recording_id < -1 OR recording_id = 0",
        [],
    )?;
    let page: Vec<i64> = tx
        .prepare("SELECT recording_id FROM search_due ORDER BY recording_id LIMIT ?1")?
        .query_map([limit.max(1) as i64], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if page.is_empty() {
        tx.commit()?;
        return Ok(0);
    }
    let rows = rows_for(&tx, volumes, &page)?;
    for id in &page {
        tx.prepare_cached("DELETE FROM search_track WHERE rowid = ?1")?
            .execute([id])?;
        let Some(row) = rows.get(id) else { continue };
        let f = &row.fields;
        let comment: String = f.comment.chars().take(MAX_COMMENT_CHARS).collect();
        // A title that is the file's name is indexed like a file name.
        let title = match f.title_is_file_name {
            true => file_name_text(&f.title),
            false => f.title.clone(),
        };
        tx.prepare_cached(
            "INSERT INTO search_track
                 (rowid, title, artist, album, remixer, label, genre, comment, file_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?
        .execute(params![
            id,
            title,
            f.artist,
            f.album,
            f.remixer,
            f.label,
            f.genre,
            comment,
            row.file_names
        ])?;
    }
    tx.execute(
        "DELETE FROM search_due WHERE recording_id IN (SELECT value FROM json_each(?1))",
        [json(&page)],
    )?;
    tx.commit()?;
    Ok(page.len())
}

/// The rows to write for the `page` of tracks. A track that doesn't exist,
/// or has neither a file nor a Library track, has none.
fn rows_for(
    conn: &Connection,
    volumes: &impl Volumes,
    page: &[i64],
) -> rusqlite::Result<HashMap<i64, Row>> {
    let ids = json(page);
    // The files' names, best file first.
    let mut names: HashMap<i64, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT rf.recording_id, f.rel_path
         FROM recording_file rf JOIN file f ON f.id = rf.file_id
         WHERE rf.recording_id IN (SELECT value FROM json_each(?1))
         ORDER BY rf.recording_id, (rf.role = 'best') DESC, f.id",
    )?;
    for row in stmt.query_map([&ids], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })? {
        let (recording, rel_path) = row?;
        names
            .entry(recording)
            .or_default()
            .push(file_name_text(&rel_path));
    }
    // The Library tracks among them.
    let mut library: HashMap<i64, LibraryTrackId> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT recording_id, id FROM library_track
         WHERE recording_id IN (SELECT value FROM json_each(?1))",
    )?;
    for row in stmt.query_map([&ids], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (recording, id) = row?;
        library.insert(recording, LibraryTrackId(id));
    }
    let wanted: Vec<i64> = page
        .iter()
        .copied()
        .filter(|id| names.contains_key(id) || library.contains_key(id))
        .collect();
    let in_library: Vec<LibraryTrackId> = wanted
        .iter()
        .filter_map(|id| library.get(id).copied())
        .collect();
    let elsewhere: Vec<i64> = wanted
        .iter()
        .copied()
        .filter(|id| !library.contains_key(id))
        .collect();
    let mut library_fields = shown_fields(conn, volumes, &in_library)?;
    let mut other_fields = shown_recording_fields(conn, &elsewhere)?;
    let mut rows = HashMap::new();
    for id in wanted {
        let fields = match library.get(&id) {
            Some(lt) => library_fields.remove(lt),
            None => other_fields.remove(&id),
        };
        // A track the lists can't name yet (no row) is skipped; it is due
        // again the moment something about it changes.
        let Some(fields) = fields else { continue };
        let file_names = names.remove(&id).unwrap_or_default().join("\u{1f}");
        rows.insert(id, Row { fields, file_names });
    }
    Ok(rows)
}

/// Settles every track that is due, `page` tracks per transaction, and
/// returns how many tracks that was.
pub fn settle(
    conn: &mut Connection,
    volumes: &impl Volumes,
    page: usize,
) -> rusqlite::Result<usize> {
    let mut settled = 0;
    while super::pending(conn)? {
        settled += settle_page(conn, volumes, page)?;
    }
    Ok(settled)
}

/// Throws every row away and works them all out again: the index is derived
/// data, and this is the way back from any drift.
pub fn rebuild(
    conn: &mut Connection,
    volumes: &impl Volumes,
    page: usize,
) -> rusqlite::Result<usize> {
    forget_everything(conn)?;
    settle(conn, volumes, page)
}

/// Empties the index and makes every track due.
pub(super) fn forget_everything(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO search_track (search_track) VALUES ('delete-all')",
        [],
    )?;
    tx.execute("DELETE FROM search_due", [])?;
    tx.execute(
        "INSERT INTO search_due (recording_id) SELECT id FROM recording",
        [],
    )?;
    tx.commit()
}
