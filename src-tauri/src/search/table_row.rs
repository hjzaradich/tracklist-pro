//! The track table's rows (`track_row`, migration 0032; ROADMAP 1.11 "The
//! track table"): what the table sorts and filters by, written by the index
//! job in the same transaction as the search index's row for the track.
//!
//! Derived data, like the search index. Only the order and the filters read
//! it: the table shows values worked out again for the rows on screen, so a
//! row is never wrong for being a moment behind, only misplaced.
//!
//! - **The file a track shows** is its Library track's linked file, else its
//!   best file, else its lowest file id. Its name and folder are the sort
//!   keys of the File and Folder columns, its length the length of a track
//!   rekordbox doesn't know, and the filters (missing, format) read it.
//! - **The filters' flags** ([`flag`]) are worked out here too, and kept in
//!   every index, so a filtered page is read from an index alone. What they
//!   follow is noted by the triggers of migration 0032; a rekordbox read or
//!   a relink changes which tracks rekordbox has in a way no trigger can
//!   name, so the `-1` mark refreshes that flag of every row at once
//!   ([`refresh_rekordbox_flags`]). There is no flag for "quality problem":
//!   that is Review's definition (a verdict that is current, no decision, no
//!   better copy ...), which can change without any row changing (a new
//!   rules version makes every verdict stale at once), so the filter reads
//!   it when the query runs (`review::quality_problem_recordings`).
//!   Two flags are Rust's, not the triggers': a Library track's missing tags
//!   and missing art, which read tag data per track. The job asks the
//!   Overview's own function for each page of Library tracks it writes
//!   (`overview::gaps_of`), so a number on the Overview and the rows behind
//!   its chip are one definition (1cC-U1b). They are as current as the rest
//!   of the row: whatever marks a track due (its tags, files, links,
//!   rekordbox's snapshot) refreshes them.
//! - **BPM and key** are `recording_display`'s, the analysis the app keeps
//!   (rekordbox's for a track rekordbox knows). The key is stored as its
//!   position on the Camelot wheel ([`camelot_order`]), so the table sorts
//!   round the wheel in whichever notation it shows.
//! - **A blank value is NULL**, so the table can put blanks last whichever
//!   way it sorts.
//! - **Text is a sort key** ([`crate::library::sort_key`]): capitals and
//!   accents don't matter, as in the Library list.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::library::sort_key;
use crate::send_values::{file_name, ShownFields};

/// The bits of `track_row.flags`.
pub mod flag {
    /// The track is in the Library.
    pub const IN_LIBRARY: i64 = 1;
    /// The track has a file. It is in All music when one of its files isn't
    /// hidden: the table adds that rule when it runs (`crate::hidden`), so
    /// hiding or showing a file changes no row here.
    pub const HAS_FILE: i64 = 2;
    /// The file it shows is not on disk (or the scan marked its Library
    /// track missing, or it has no file at all).
    pub const MISSING: i64 = 4;
    /// rekordbox has the track: one of its files is the trusted match of an
    /// entry.
    pub const IN_REKORDBOX: i64 = 16;
    /// The track is in at least one crate.
    pub const IN_CRATE: i64 = 32;
    /// The Library track lacks a title, an artist or a genre: the
    /// Overview's "missing tags", worked out by `overview::gaps_of` (1cC-U1b).
    pub const MISSING_TAGS: i64 = 64;
    /// The Library track's linked file has tags and no embedded cover: the
    /// Overview's "missing art" (`overview::gaps_of`).
    pub const MISSING_ART: i64 = 128;
}

/// What the table sorts and filters a track by, besides its text.
#[derive(Debug, Clone, Default)]
pub(super) struct Facts {
    /// The bits of [`flag`] except [`flag::HAS_FILE`], which the caller knows.
    pub flags: i64,
    pub format: Option<String>,
    pub file_id: Option<i64>,
    pub file_name: Option<String>,
    pub folder: Option<String>,
    pub duration_ms: Option<i64>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub added_at: Option<String>,
}

/// A track's BPM and key as two scalar subqueries, for a query with the
/// track as `r`: what the `recording_display` view gives, by the same
/// precedence (ROADMAP §2: rekordbox, then the file's tag, then Mixed In
/// Key, then the local estimate). The view itself is not joined, because
/// SQLite builds all of it for every query that joins it (about a second at
/// 100,000 tracks); a test holds the two to the same answers.
pub(crate) const BPM_AND_KEY: &str = "
    (SELECT a.bpm FROM analysis a WHERE a.recording_id = r.id AND a.bpm IS NOT NULL
     ORDER BY CASE a.source WHEN 'rekordbox' THEN 1 WHEN 'tag' THEN 2 WHEN 'mik' THEN 3
                            WHEN 'local' THEN 4 END LIMIT 1),
    (SELECT a.key FROM analysis a WHERE a.recording_id = r.id AND a.key IS NOT NULL
     ORDER BY CASE a.source WHEN 'rekordbox' THEN 1 WHEN 'tag' THEN 2 WHEN 'mik' THEN 3
                            WHEN 'local' THEN 4 END LIMIT 1)";

/// A key's place on the Camelot wheel: 1A is 2, 1B is 3, 2A is 4, up to 12B
/// at 25. `None` for anything else.
pub fn camelot_order(key: &str) -> Option<i64> {
    let (number, letter) = key.split_at(key.len().checked_sub(1)?);
    let number: i64 = number.parse().ok().filter(|n| (1..=12).contains(n))?;
    match letter {
        "A" => Some(number * 2),
        "B" => Some(number * 2 + 1),
        _ => None,
    }
}

/// A track's length in milliseconds: rekordbox's, when it has one, else the
/// shown file's.
pub fn time_ms(total_time_s: Option<u32>, duration_ms: Option<i64>) -> Option<i64> {
    total_time_s
        .filter(|&s| s > 0)
        .map(|s| i64::from(s) * 1000)
        .or(duration_ms)
        .filter(|&ms| ms > 0)
}

/// Text with something in it, or `None`.
fn filled(text: &str) -> Option<&str> {
    Some(text).filter(|t| !t.trim().is_empty())
}

/// The folder of a file inside its music folder, `Music/House` for
/// `House/a.mp3` in `Music`.
fn folder_of(music_folder: &str, rel_path: &str) -> String {
    let inside = rel_path.rsplit_once('/').map_or("", |(dir, _)| dir);
    match inside.is_empty() {
        true => music_folder.to_owned(),
        false => format!("{music_folder}/{inside}"),
    }
}

/// Whether rekordbox has the track `track` (a SQL name for its id): one of
/// its files is the trusted match of a rekordbox entry.
fn in_rekordbox_sql(track: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM recording_file rf2
                 JOIN rekordbox_track rt ON rt.file_id = rf2.file_id
                 WHERE rf2.recording_id = {track} AND rt.relink_probable = 0)"
    )
}

/// The facts of the tracks `ids`, in one query.
pub(super) fn facts_for(conn: &Connection, ids: &[i64]) -> rusqlite::Result<HashMap<i64, Facts>> {
    let json = serde_json::to_string(ids).expect("numbers always serialize");
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT r.id, lt.id IS NOT NULL, lt.added_at, r.created_at,
                f.id, f.rel_path, f.duration_ms, mf.rel_path,
                {BPM_AND_KEY},
                f.sniffed_format,
                f.id IS NULL OR f.present = 0 OR coalesce(lt.source_status = 'missing', 0),
                {in_rekordbox},
                coalesce({in_a_crate}, 0)
         FROM recording r
         LEFT JOIN library_track lt ON lt.recording_id = r.id
         LEFT JOIN file f ON f.id = COALESCE(
             lt.linked_file_id,
             (SELECT rf.file_id FROM recording_file rf WHERE rf.recording_id = r.id
              ORDER BY (rf.role = 'best') DESC, rf.file_id LIMIT 1))
         LEFT JOIN music_folder mf ON mf.id = f.music_folder_id
         WHERE r.id IN (SELECT value FROM json_each(?1))",
        in_rekordbox = in_rekordbox_sql("r.id"),
        // The Overview's own text for "in a crate", so its count and the
        // table's filter can't mean different things (1cC-4).
        in_a_crate = crate::overview::IN_A_CRATE,
    ))?;
    let rows = stmt.query_map([&json], |r| {
        let rel_path: Option<String> = r.get(5)?;
        let music_folder: Option<String> = r.get(7)?;
        let added_at: Option<String> = r.get(2)?;
        let created_at: String = r.get(3)?;
        let bit = |at: usize, flag: i64| -> rusqlite::Result<i64> {
            Ok(if r.get::<_, bool>(at)? { flag } else { 0 })
        };
        let flags = bit(1, flag::IN_LIBRARY)?
            | bit(11, flag::MISSING)?
            | bit(12, flag::IN_REKORDBOX)?
            | bit(13, flag::IN_CRATE)?;
        Ok((
            r.get::<_, i64>(0)?,
            Facts {
                flags,
                format: r.get(10)?,
                file_id: r.get(4)?,
                file_name: rel_path.as_deref().map(|p| file_name(p).to_owned()),
                folder: rel_path
                    .as_deref()
                    .zip(music_folder.as_deref())
                    .map(|(path, folder)| folder_of(folder, path)),
                duration_ms: r.get(6)?,
                bpm: r.get(8)?,
                key: r.get(9)?,
                added_at: Some(added_at.unwrap_or(created_at)),
            },
        ))
    })?;
    rows.collect()
}

/// Sets the "rekordbox has it" flag of every row to what the snapshot says
/// now. A rekordbox read or a relink can change it for any track, so the
/// `-1` mark does this in one statement; only the rows that change are
/// written.
pub(super) fn refresh_rekordbox_flags(tx: &Transaction<'_>) -> rusqlite::Result<usize> {
    let has = in_rekordbox_sql("track_row.recording_id");
    tx.execute(
        &format!(
            "UPDATE track_row
             SET flags = (flags & ~{bit}) | CASE WHEN {has} THEN {bit} ELSE 0 END
             WHERE ((flags & {bit}) <> 0) IS NOT {has}",
            bit = flag::IN_REKORDBOX,
        ),
        [],
    )
}

/// The values of a `track_row` row, to compare the row the job has worked
/// out with the one already stored.
#[derive(Debug, PartialEq)]
struct Values {
    flags: i64,
    format: Option<String>,
    file_id: Option<i64>,
    title_key: String,
    artist_key: Option<String>,
    genre_key: Option<String>,
    file_key: Option<String>,
    folder_key: Option<String>,
    bpm: Option<f64>,
    key_order: Option<i64>,
    time_ms: Option<i64>,
    rating: Option<i64>,
    added_at: Option<String>,
    text_hash: i64,
}

/// What [`write`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Written {
    /// The stored row was the same: nothing was written.
    Unchanged,
    /// The row changed or was new. `text` says whether what the search
    /// index holds for the track changed (or was never written) too.
    Changed { text: bool },
}

/// Writes the row of track `id`, but only if it differs from the stored
/// one: a rekordbox read marks every Library track due, and most of them
/// come out the same, which costs a read each instead of a write to the
/// row and to all the indexes. `text_hash` stands for the text the search
/// index holds (see [`Written`]).
pub(super) fn write(
    tx: &Transaction<'_>,
    id: i64,
    fields: &ShownFields,
    facts: &Facts,
    has_file: bool,
    text_hash: i64,
) -> rusqlite::Result<Written> {
    let key_of = |text: &str| filled(text).map(sort_key);
    let new = Values {
        flags: facts.flags | if has_file { flag::HAS_FILE } else { 0 },
        format: facts.format.clone(),
        file_id: facts.file_id,
        title_key: sort_key(&fields.title),
        artist_key: key_of(&fields.artist),
        genre_key: key_of(&fields.genre),
        file_key: facts.file_name.as_deref().and_then(key_of),
        folder_key: facts.folder.as_deref().and_then(key_of),
        bpm: facts.bpm,
        key_order: facts.key.as_deref().and_then(camelot_order),
        time_ms: time_ms(fields.total_time_s, facts.duration_ms),
        rating: fields.stars.map(i64::from),
        added_at: facts.added_at.clone(),
        text_hash,
    };
    let stored = tx
        .prepare_cached(
            "SELECT flags, format, file_id, title_key, artist_key, genre_key, file_key,
                    folder_key, bpm, key_order, time_ms, rating, added_at, text_hash
             FROM track_row WHERE recording_id = ?1",
        )?
        .query_row([id], |r| {
            Ok(Values {
                flags: r.get(0)?,
                format: r.get(1)?,
                file_id: r.get(2)?,
                title_key: r.get(3)?,
                artist_key: r.get(4)?,
                genre_key: r.get(5)?,
                file_key: r.get(6)?,
                folder_key: r.get(7)?,
                bpm: r.get(8)?,
                key_order: r.get(9)?,
                time_ms: r.get(10)?,
                rating: r.get(11)?,
                added_at: r.get(12)?,
                text_hash: r.get(13)?,
            })
        })
        .optional()?;
    if stored.as_ref() == Some(&new) {
        return Ok(Written::Unchanged);
    }
    tx.prepare_cached(
        "INSERT OR REPLACE INTO track_row
             (recording_id, flags, format, file_id, title_key, artist_key, genre_key,
              file_key, folder_key, bpm, key_order, time_ms, rating, added_at, text_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )?
    .execute(params![
        id,
        new.flags,
        new.format,
        new.file_id,
        new.title_key,
        new.artist_key,
        new.genre_key,
        new.file_key,
        new.folder_key,
        new.bpm,
        new.key_order,
        new.time_ms,
        new.rating,
        new.added_at,
        new.text_hash,
    ])?;
    Ok(Written::Changed {
        text: stored.is_none_or(|old| old.text_hash != new.text_hash),
    })
}
