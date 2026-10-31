//! Working out the index rows of the tracks that are due.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use super::table_row::{self, Facts};
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

/// A hash of the eight texts a track's search row holds, to tell a row
/// that would be written again as it is (the search table keeps no text to
/// read back). Every text is length-prefixed, so none can run into the next.
fn text_hash(texts: &[&str; 8]) -> i64 {
    let mut hasher = blake3::Hasher::new();
    for text in texts {
        hasher.update(&(text.len() as u64).to_le_bytes());
        hasher.update(text.as_bytes());
    }
    let hash = hasher.finalize();
    i64::from_le_bytes(hash.as_bytes()[..8].try_into().expect("eight bytes"))
}

/// A row of the index, before it is written.
struct Row {
    fields: ShownFields,
    file_names: String,
    /// What the track table sorts and filters by.
    facts: Facts,
    /// The track has a file.
    has_file: bool,
}

/// Works out and writes the rows of up to `limit` due tracks, in one
/// transaction, and returns how many tracks that was. A track's new rows are
/// written in the same transaction that removes it from `search_due`, so a
/// crash never leaves a track with neither. A row that comes out the same as
/// the stored one is not written (neither is its search row), so a track
/// that was marked for nothing costs a read.
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
        // Which tracks rekordbox has is the table's, for every track.
        table_row::refresh_rekordbox_flags(&tx)?;
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
        let Some(row) = rows.get(id) else {
            // No row to write (the track is gone, or has neither a file nor
            // a Library track): whatever it had goes.
            tx.prepare_cached("DELETE FROM search_track WHERE rowid = ?1")?
                .execute([id])?;
            tx.prepare_cached("DELETE FROM track_row WHERE recording_id = ?1")?
                .execute([id])?;
            continue;
        };
        let f = &row.fields;
        let comment: String = f.comment.chars().take(MAX_COMMENT_CHARS).collect();
        // A title that is the file's name is indexed like a file name.
        let title = match f.title_is_file_name {
            true => file_name_text(&f.title),
            false => f.title.clone(),
        };
        let text = [
            title.as_str(),
            &f.artist,
            &f.album,
            &f.remixer,
            &f.label,
            &f.genre,
            &comment,
            &row.file_names,
        ];
        // Only what changed is written: most of the tracks a rekordbox read
        // marks come out the same as they were.
        let written = table_row::write(
            &tx,
            *id,
            &row.fields,
            &row.facts,
            row.has_file,
            text_hash(&text),
        )?;
        if written != (table_row::Written::Changed { text: true }) {
            continue;
        }
        tx.prepare_cached("DELETE FROM search_track WHERE rowid = ?1")?
            .execute([id])?;
        tx.prepare_cached(
            "INSERT INTO search_track
                 (rowid, title, artist, album, remixer, label, genre, comment, file_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?
        .execute(params![
            id, text[0], text[1], text[2], text[3], text[4], text[5], text[6], text[7]
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
    let mut fields = fields_of(conn, volumes, &wanted, &library)?;
    let mut facts = table_row::facts_for(conn, &wanted)?;
    // The Library tracks' two gap flags are the Overview's own definition.
    let library_tracks: Vec<LibraryTrackId> = wanted
        .iter()
        .filter_map(|id| library.get(id).copied())
        .collect();
    let gaps = gaps_or_none(conn, volumes, &library_tracks);
    let mut rows = HashMap::new();
    for id in wanted {
        // A track the lists can't name yet (no row) is skipped; it is due
        // again the moment something about it changes.
        let (Some(fields), Some(facts)) = (fields.remove(&id), facts.remove(&id)) else {
            continue;
        };
        let mut facts = facts;
        if let Some(lt) = library.get(&id) {
            if gaps.tags.contains(&lt.0) {
                facts.flags |= table_row::flag::MISSING_TAGS;
            }
            if gaps.art.contains(&lt.0) {
                facts.flags |= table_row::flag::MISSING_ART;
            }
        }
        let has_file = names.contains_key(&id);
        let file_names = names.remove(&id).unwrap_or_default().join("\u{1f}");
        rows.insert(
            id,
            Row {
                fields,
                file_names,
                facts,
                has_file,
            },
        );
    }
    Ok(rows)
}

/// [`crate::overview::gaps_of`] for a page of Library tracks, which never
/// fails the page: one track whose tags can't be read must not stop the index
/// job (it would stay due, and the Overview would say "still counting" for
/// ever). If the page errors, each track is asked alone; a track that still
/// errors is logged and has no gaps, and its row is written like any other.
fn gaps_or_none(
    conn: &Connection,
    volumes: &impl Volumes,
    tracks: &[LibraryTrackId],
) -> crate::overview::Gaps {
    match crate::overview::gaps_of(conn, volumes, tracks) {
        Ok(gaps) => gaps,
        Err(error) => {
            eprintln!(
                "index: the tag gaps of a page of {} tracks failed: {error}",
                tracks.len()
            );
            let mut all = crate::overview::Gaps::default();
            for track in tracks {
                match crate::overview::gaps_of(conn, volumes, &[*track]) {
                    Ok(gaps) => {
                        all.tags.extend(gaps.tags);
                        all.art.extend(gaps.art);
                    }
                    Err(error) => eprintln!(
                        "index: Library track {} has no tag gaps recorded, its tags can't be read: {error}",
                        track.0
                    ),
                }
            }
            all
        }
    }
}

/// What the lists show of each of the tracks `wanted`, given which of them
/// are Library tracks: a Library track's from [`shown_fields`], another
/// track's from its own tags. The one place the index, the track table's
/// rows and the rows on screen get their text.
pub(crate) fn fields_of(
    conn: &Connection,
    volumes: &impl Volumes,
    wanted: &[i64],
    library: &HashMap<i64, LibraryTrackId>,
) -> rusqlite::Result<HashMap<i64, ShownFields>> {
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
    let mut out = HashMap::new();
    for &id in wanted {
        let fields = match library.get(&id) {
            Some(lt) => library_fields.remove(lt),
            None => other_fields.remove(&id),
        };
        if let Some(fields) = fields {
            out.insert(id, fields);
        }
    }
    Ok(out)
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

/// Settles the tracks that are due now, when there are only a few of them
/// (at most `limit`, and the mark for every Library track isn't among them):
/// for an action that has just changed a track or two, so the lists read
/// right after it show the change. A longer list is left to the index job.
/// Returns how many tracks it settled.
pub fn settle_if_small(
    conn: &mut Connection,
    volumes: &impl Volumes,
    limit: usize,
) -> rusqlite::Result<usize> {
    let (due, everything): (i64, bool) = conn.query_row(
        "SELECT count(*), coalesce(max(recording_id = -1), 0) FROM search_due",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if due == 0 || everything || due as usize > limit {
        return Ok(0);
    }
    settle(conn, volumes, limit)
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
    tx.execute("DELETE FROM track_row", [])?;
    tx.execute("DELETE FROM search_due", [])?;
    tx.execute(
        "INSERT INTO search_due (recording_id) SELECT id FROM recording",
        [],
    )?;
    tx.commit()
}
