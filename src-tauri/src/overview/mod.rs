//! The numbers the Overview screen shows (ROADMAP 1.13; task 1cA-12): one
//! command, [`overview`], that answers them all at once.
//!
//! **What the numbers mean** is the owner's decision of 2026-10-12
//! (ROADMAP 1.13, "What the numbers mean"):
//! - A track's **genre** is rekordbox's for a track rekordbox knows, else
//!   the file's tag ([`GenreCount`]). "Knows" is what attach means by a
//!   trusted entry: one relink matched to the track and didn't mark
//!   probable, picked among several by the same order as attach's BPM and
//!   key ([`crate::attach::PICK_ORDER`]).
//! - **Analyzed** means rekordbox supplied a BPM or a key: the track has an
//!   `analysis` row from rekordbox, which attach writes only then.
//! - **Decisions made** counts the operations in the undo log.
//! - **Energy** is not here: nothing stores it until task 1cC-1.
//!
//! **Left out on purpose**, each with the function a later job should use:
//! - *Unsent changes* ("Send N changes"): defined with the Send button
//!   (1cB-11), from `send::preflight`.
//! - *How many need a decision*: the Review screen's read model (1bD-1/2).
//! - *Missing tags or art*: nothing says which tags or what counts as art.
//!
//! **Cost.** Every number is an aggregate SQL statement on the read pool;
//! no row is loaded to be counted in Rust. The Library's numbers come from
//! one pass over `library_track` that looks the rest up by primary key or
//! index ([`LIBRARY`]); a test reads each statement's query plan to prove
//! nothing else is scanned. The genres statement groups the tracks by what
//! their tags say and Rust reads the *distinct* groups only ([`genres`]).
//!
//! **Still counting** ([`Counting`]): a flag per source of numbers that can
//! change while background work runs. A flag is true while a job of one of
//! its kinds is queued or running (a failed or cancelled job holds
//! nothing), and for `quality` also while any Library file's verdict is
//! missing or stale, since a stale verdict is no verdict ([`crate::quality`]).
//!
//! **One place to change** each choice still waiting on the owner:
//! [`COUNT_UNDONE_OPERATIONS`], [`THIS_WEEK`] and [`BLANK_GENRE_FALLS_BACK`].
//! The crate count is `crate_entry` rows of kind `member` ([`IN_A_CRATE`]);
//! playlists arrive in Phase 3.
//!
//! Read-only: the whole answer is read in one transaction.

use std::collections::{BTreeMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::quality::stale_condition;

#[cfg(test)]
mod tests;

/// Whether an operation that was undone still counts as a decision made
/// (owner question; the default is no: a decision taken back no longer
/// stands).
pub const COUNT_UNDONE_OPERATIONS: bool = false;

/// How far back "this week" reaches: a rolling week ending now, in UTC
/// (owner question; the default is the last 7 days, not the calendar week).
/// It is an SQLite date modifier.
pub const THIS_WEEK: &str = "-7 days";

/// Whether a track rekordbox knows, but whose genre in rekordbox is blank,
/// takes the genre of its file's tag (owner question; the default is no:
/// rekordbox's blank is the answer).
pub const BLANK_GENRE_FALLS_BACK: bool = false;

/// The verdicts that make a track "made from a low-quality file".
const LOW_QUALITY_VERDICTS: &str = "'low_bitrate', 'suspect_transcode', 'truncated', 'broken'";

/// A Library track is in a crate when a static crate holds it. A smart
/// crate's entries are always-include and never-include rules, not
/// membership. Playlists arrive in Phase 3.
const IN_A_CRATE: &str = "EXISTS (SELECT 1 FROM crate_entry ce
                                  WHERE ce.library_track_id = lt.id AND ce.kind = 'member')";

/// The track has a row from rekordbox in `analysis` (see the module docs).
const ANALYZED: &str = "EXISTS (SELECT 1 FROM analysis a
                                WHERE a.recording_id = lt.recording_id AND a.source = 'rekordbox')";

/// The job kinds whose work changes the counts of files and tracks.
const FILE_JOBS: [&str; 3] = ["scan", "read", "group"];
/// The job kinds whose work changes what rekordbox gives the tracks.
const REKORDBOX_JOBS: [&str; 3] = ["read_rekordbox", "relink", "attach"];
/// The job kinds whose work changes the verdicts.
const QUALITY_JOBS: [&str; 1] = ["quality"];

/// One genre and how many Library tracks have it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenreCount {
    /// The genre as most of its tracks spell it, without outer blanks.
    /// Capitals don't make a genre of its own.
    pub name: String,
    pub tracks: u32,
}

/// Which numbers can still change because background work is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Counting {
    /// A scan, a tags read or a grouping run is queued or running: the
    /// tracks, total length, genres (from tags) and All music counts.
    pub files: bool,
    /// Rekordbox's data is being read, matched or attached: analyzed and
    /// genres (from rekordbox).
    pub rekordbox: bool,
    /// A quality job is queued or running, or a Library file's verdict is
    /// missing or stale: the low-quality count.
    pub quality: bool,
}

/// Every Overview number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    /// Library tracks.
    pub tracks: u32,
    /// The Library's length: the sum of its linked files' durations.
    pub total_length_ms: u64,
    /// Library tracks in at least one crate. Crates only until playlists
    /// arrive (Phase 3).
    pub in_crates: u32,
    /// Library tracks in no crate.
    pub in_no_crate: u32,
    /// Library tracks rekordbox supplied a BPM or a key for.
    pub analyzed_by_rekordbox: u32,
    /// Library tracks it didn't.
    pub not_analyzed: u32,
    /// Whether a rekordbox library was ever read. Before that every track
    /// is "not analyzed", which says nothing.
    pub rekordbox_read: bool,
    /// Library tracks whose linked file has a current verdict that is not
    /// `ok`. A file with no current verdict isn't counted.
    pub low_quality: u32,
    /// When the Library was last sent, in milliseconds since the Unix
    /// epoch; `None` if never.
    pub last_sent_ms: Option<u64>,
    /// Tracks in All music: tracks with a file that isn't hidden.
    pub all_music: u32,
    /// Library tracks added in the last 7 days ([`THIS_WEEK`]).
    pub added_this_week: u32,
    /// Operations made in the last 7 days ([`THIS_WEEK`]).
    pub decisions_this_week: u32,
    /// Every genre, most tracks first, then by name.
    pub genres: Vec<GenreCount>,
    /// Library tracks with no genre.
    pub no_genre: u32,
    pub counting: Counting,
}

/// The Library's numbers in one pass over `library_track`. Each other table
/// is looked up by its primary key or an index. `?1` is the start of this
/// week.
///
/// `q` is 0 for a track with no verdict to read, 1 for a flagged file whose
/// verdict is current, and 2 for a file whose verdict is missing or stale.
fn library_sql() -> String {
    format!(
        "SELECT count(*),
                coalesce(sum(s.duration_ms), 0),
                coalesce(sum(s.in_a_crate), 0),
                coalesce(sum(s.analyzed), 0),
                coalesce(sum(s.q = 1), 0),
                coalesce(sum(s.q = 2), 0),
                coalesce(sum(s.added >= ?1), 0)
         FROM (SELECT f.duration_ms AS duration_ms,
                      {IN_A_CRATE} AS in_a_crate,
                      {ANALYZED} AS analyzed,
                      CASE WHEN f.id IS NULL THEN 0
                           WHEN {stale} THEN 2
                           WHEN v.verdict IN ({LOW_QUALITY_VERDICTS}) THEN 1
                           ELSE 0 END AS q,
                      lt.added_at AS added
               FROM library_track lt
               LEFT JOIN file f ON f.id = lt.linked_file_id
               LEFT JOIN file_quality q ON q.file_id = f.id
               LEFT JOIN file_verdict v ON v.file_id = f.id) s",
        stale = stale_condition(),
    )
}

/// Tracks with a file that isn't hidden. A track is counted once however
/// many files it has.
const ALL_MUSIC: &str = "SELECT count(*) FROM recording r
     WHERE EXISTS (SELECT 1 FROM recording_file rf JOIN file f ON f.id = rf.file_id
                   WHERE rf.recording_id = r.id AND f.hidden_reason IS NULL)";

/// The start of this week as stored timestamps are written.
fn week_start() -> String {
    format!("strftime('%Y-%m-%dT%H:%M:%fZ', ?1 / 1000.0, 'unixepoch', '{THIS_WEEK}')")
}

fn decisions_sql() -> String {
    let undone = if COUNT_UNDONE_OPERATIONS {
        ""
    } else {
        "AND undone_at IS NULL"
    };
    format!(
        "SELECT count(*) FROM operation WHERE created_at >= {} {undone}",
        week_start()
    )
}

/// Each Library track's genre source, grouped: what rekordbox says (`NULL`
/// for a track rekordbox doesn't know) and, only where the file's tag is
/// needed, the genre frames of the linked file as a `raw_tags` object, one
/// array per block, in the file's own order. Rust then reads each distinct
/// group with the send's own tag rules ([`crate::send_values::tag_values`]),
/// so a tag means the same here as in a send.
///
/// The frames kept are the ones `send_values::tag_fields` maps to `Genre`:
/// `TCON` (ID3v2), `©gen` (MP4), `GENRE` in any case (Vorbis, APE) and
/// `IGNR` (RIFF INFO). Nothing else in a tag is looked at.
fn genres_sql() -> String {
    // `rb` is `''` for a known track whose genre is blank.
    let tag_needed = if BLANK_GENRE_FALLS_BACK {
        "g.rb IS NULL OR trim(g.rb) = ''"
    } else {
        "g.rb IS NULL"
    };
    format!(
        "SELECT rb, digest, count(*) FROM (
             SELECT g.rb AS rb,
                    CASE WHEN g.fid IS NOT NULL AND ({tag_needed}) THEN (
                        SELECT json_group_object(b.block, json(b.items)) FROM (
                            SELECT blk.key AS block, json_group_array(json(i.value)) AS items
                            FROM file f, json_each(f.raw_tags) blk, json_each(blk.value) i
                            WHERE f.id = g.fid AND json_type(blk.value) = 'array' AND (
                                   (blk.key = 'id3v2' AND json_extract(i.value, '$.key') = 'TCON')
                                OR (blk.key = 'mp4_ilst'
                                    AND json_extract(i.value, '$.key') = char(169) || 'gen')
                                OR (blk.key IN ('vorbis_comments', 'ape')
                                    AND lower(json_extract(i.value, '$.key')) = 'genre')
                                OR (blk.key = 'riff_info'
                                    AND json_extract(i.value, '$.key') = 'IGNR'))
                            GROUP BY blk.key) b) END AS digest
             FROM (SELECT lt.linked_file_id AS fid,
                          (SELECT coalesce(CAST(json_extract(rt.attributes, '$.Genre') AS TEXT), '')
                           FROM rekordbox_track rt
                           LEFT JOIN recording_file rf
                                  ON rf.file_id = rt.file_id AND rf.recording_id = rt.recording_id
                           WHERE rt.recording_id = lt.recording_id AND rt.relink_probable = 0
                           ORDER BY {order}
                           LIMIT 1) AS rb
                   FROM library_track lt) g)
         GROUP BY rb, digest",
        order = crate::attach::PICK_ORDER,
    )
}

/// The genre one group of tracks has, if any.
fn genre_of(rekordbox: Option<&str>, digest: Option<&str>) -> Option<String> {
    let usable = |text: &str| Some(text.trim().to_owned()).filter(|t| !t.is_empty());
    match rekordbox {
        Some(text) if !(BLANK_GENRE_FALLS_BACK && text.trim().is_empty()) => usable(text),
        _ => crate::send_values::tag_values(digest?, &["Genre"])
            .into_iter()
            .find_map(|v| usable(&v.value)),
    }
}

/// Every genre with its track count, and the tracks without one.
fn genres(conn: &Connection, tracks: u32) -> rusqlite::Result<(Vec<GenreCount>, u32)> {
    let mut stmt = conn.prepare(&genres_sql())?;
    // Capitals don't make another genre; its name is the spelling most of
    // its tracks use (then the smallest, so the answer never wobbles).
    let mut by_name: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let (rekordbox, digest): (Option<String>, Option<String>) = (row.get(0)?, row.get(1)?);
        let n = u32::try_from(row.get::<_, i64>(2)?).unwrap_or(u32::MAX);
        if let Some(genre) = genre_of(rekordbox.as_deref(), digest.as_deref()) {
            let spellings = by_name.entry(genre.to_lowercase()).or_default();
            let count = spellings.entry(genre).or_default();
            *count = count.saturating_add(n);
        }
    }
    let mut genres: Vec<GenreCount> = by_name
        .into_values()
        .map(|spellings| GenreCount {
            tracks: spellings.values().fold(0u32, |sum, n| sum.saturating_add(*n)),
            name: spellings
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(name, _)| name.clone())
                .unwrap_or_default(),
        })
        .collect();
    genres.sort_by(|a, b| b.tracks.cmp(&a.tracks).then_with(|| a.name.cmp(&b.name)));
    let with_genre = genres.iter().fold(0u32, |sum, g| sum.saturating_add(g.tracks));
    Ok((genres, tracks.saturating_sub(with_genre)))
}

/// Which sources have background work going, from the queued and running
/// jobs.
fn counting(conn: &Connection, verdicts_unsettled: bool) -> rusqlite::Result<Counting> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT kind FROM job WHERE status IN ('queued', 'running')")?;
    let kinds: HashSet<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let any = |names: &[&str]| names.iter().any(|n| kinds.contains(*n));
    Ok(Counting {
        files: any(&FILE_JOBS),
        rekordbox: any(&REKORDBOX_JOBS),
        quality: verdicts_unsettled || any(&QUALITY_JOBS),
    })
}

fn count(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Every Overview number, as of `now_ms` (milliseconds since the Unix
/// epoch; it sets where "this week" starts). Reads in one transaction, so
/// the numbers agree with each other.
pub fn overview_at(conn: &Connection, now_ms: i64) -> rusqlite::Result<Overview> {
    let tx = conn.unchecked_transaction()?;
    let conn = &*tx;

    let since: String = conn.query_row(&format!("SELECT {}", week_start()), [now_ms], |r| r.get(0))?;
    let (tracks, length, in_crates, analyzed, flagged, unsettled, added) =
        conn.query_row(&library_sql(), params![since], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })?;
    let tracks = count(tracks);
    let (in_crates, analyzed) = (count(in_crates), count(analyzed));
    let all_music = conn.query_row(ALL_MUSIC, [], |r| r.get::<_, i64>(0))?;
    let decisions = conn.query_row(&decisions_sql(), [now_ms], |r| r.get::<_, i64>(0))?;
    let last_sent = crate::send::last_send_ms(conn)?;
    let rekordbox_read = conn.query_row("SELECT EXISTS (SELECT 1 FROM rekordbox_track)", [], |r| {
        r.get::<_, bool>(0)
    })?;
    let (genres, no_genre) = genres(conn, tracks)?;
    let counting = counting(conn, unsettled > 0)?;
    drop(tx);

    Ok(Overview {
        tracks,
        total_length_ms: u64::try_from(length).unwrap_or(0),
        in_crates,
        in_no_crate: tracks.saturating_sub(in_crates),
        analyzed_by_rekordbox: analyzed,
        not_analyzed: tracks.saturating_sub(analyzed),
        rekordbox_read,
        low_quality: count(flagged),
        last_sent_ms: last_sent.and_then(|ms| u64::try_from(ms).ok()),
        all_music: count(all_music),
        added_this_week: count(added),
        decisions_this_week: count(decisions),
        genres,
        no_genre,
        counting,
    })
}

/// Every number the Overview screen shows, as of now.
#[tauri::command]
#[specta::specta]
pub async fn overview(reads: State<'_, ReadPool>) -> Result<Overview, IpcError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Ok(reads.read(|conn| overview_at(conn, now_ms))?)
}
