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
//!   rekordbox doesn't know, and the filters (missing, format, quality)
//!   read it.
//! - **The filters' flags** ([`flag`]) are worked out here too, and kept in
//!   every index, so a filtered page is read from an index alone. What they
//!   follow is noted by the triggers of migration 0032; a rekordbox read or
//!   a relink changes which tracks rekordbox has in a way no trigger can
//!   name, so the `-1` mark refreshes that flag of every row at once
//!   ([`refresh_rekordbox_flags`]).
//! - **BPM and key** are `recording_display`'s, the analysis the app keeps
//!   (rekordbox's for a track rekordbox knows). The key is stored as its
//!   position on the Camelot wheel ([`camelot_order`]), so the table sorts
//!   round the wheel in whichever notation it shows.
//! - **A blank value is NULL**, so the table can put blanks last whichever
//!   way it sorts.
//! - **Text is a sort key** ([`crate::library::sort_key`]): capitals and
//!   accents don't matter, as in the Library list.

use std::collections::HashMap;

use rusqlite::{params, Connection, Transaction};

use crate::library::sort_key;
use crate::review::FLAGGED;
use crate::send_values::{file_name, ShownFields};

/// The bits of `track_row.flags`.
pub mod flag {
    /// The track is in the Library.
    pub const IN_LIBRARY: i64 = 1;
    /// The track has a file: it is in All music.
    pub const HAS_FILE: i64 = 2;
    /// The file it shows is not on disk (or the scan marked its Library
    /// track missing, or it has no file at all).
    pub const MISSING: i64 = 4;
    /// The file it shows has a quality finding (a stored verdict that flags
    /// it, `review::FLAGGED`).
    pub const QUALITY_PROBLEM: i64 = 8;
    /// rekordbox has the track: one of its files is the trusted match of an
    /// entry.
    pub const IN_REKORDBOX: i64 = 16;
    /// The track is in at least one crate.
    pub const IN_CRATE: i64 = 32;
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
                EXISTS (SELECT 1 FROM file_verdict v WHERE v.file_id = f.id AND v.{FLAGGED}),
                {in_rekordbox},
                coalesce(EXISTS (SELECT 1 FROM crate_entry ce
                                 WHERE ce.library_track_id = lt.id AND ce.kind = 'member'), 0)
         FROM recording r
         LEFT JOIN library_track lt ON lt.recording_id = r.id
         LEFT JOIN file f ON f.id = COALESCE(
             lt.linked_file_id,
             (SELECT rf.file_id FROM recording_file rf WHERE rf.recording_id = r.id
              ORDER BY (rf.role = 'best') DESC, rf.file_id LIMIT 1))
         LEFT JOIN music_folder mf ON mf.id = f.music_folder_id
         WHERE r.id IN (SELECT value FROM json_each(?1))",
        in_rekordbox = in_rekordbox_sql("r.id"),
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
            | bit(12, flag::QUALITY_PROBLEM)?
            | bit(13, flag::IN_REKORDBOX)?
            | bit(14, flag::IN_CRATE)?;
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

/// Writes the row of track `id`.
pub(super) fn write(
    tx: &Transaction<'_>,
    id: i64,
    fields: &ShownFields,
    facts: &Facts,
    has_file: bool,
) -> rusqlite::Result<()> {
    let key_of = |text: &str| filled(text).map(sort_key);
    let flags = facts.flags | if has_file { flag::HAS_FILE } else { 0 };
    tx.prepare_cached(
        "INSERT INTO track_row
             (recording_id, flags, format, file_id, title_key, artist_key, genre_key,
              file_key, folder_key, bpm, key_order, time_ms, rating, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )?
    .execute(params![
        id,
        flags,
        facts.format,
        facts.file_id,
        sort_key(&fields.title),
        key_of(&fields.artist),
        key_of(&fields.genre),
        facts.file_name.as_deref().and_then(key_of),
        facts.folder.as_deref().and_then(key_of),
        facts.bpm,
        facts.key.as_deref().and_then(camelot_order),
        time_ms(fields.total_time_s, facts.duration_ms),
        fields.stars,
        facts.added_at,
    ])?;
    Ok(())
}
