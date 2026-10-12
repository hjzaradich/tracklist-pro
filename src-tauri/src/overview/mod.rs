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

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
use crate::quality::stale_condition;
use crate::scan::system_volumes;

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
    /// Library tracks with no title, no artist or no genre, as the lists
    /// show them ([`missing_tags`]).
    pub missing_tags: u32,
    /// Library tracks whose linked file has tags and no embedded cover
    /// ([`MISSING_ART`]).
    pub missing_art: u32,
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
    format!(
        "SELECT rb, digest, count(*) FROM ({}) GROUP BY rb, digest",
        genre_source_sql()
    )
}

/// The frames of a file's tags that `send_values::tag_fields` maps to
/// `Genre` (and, with `artist`, to `Artist`), as a condition on a block
/// (`blk.key`) and an item (`i.value`) of `raw_tags`. Keys are the ones the
/// tags layer writes: `TCON` / `TPE1` (ID3v2), `©gen` / `©ART` (MP4), `genre`
/// / `artist` in any case (Vorbis, APE), `IGNR` / `IART` (RIFF INFO), and
/// for the artist `AUTH` (AIFF) and `artist` (ID3v1).
fn frame_filter(artist: bool) -> String {
    let key = "json_extract(i.value, '$.key')";
    let mut keys = format!(
        "(blk.key = 'id3v2' AND {key} = 'TCON')
         OR (blk.key = 'mp4_ilst' AND {key} = char(169) || 'gen')
         OR (blk.key IN ('vorbis_comments', 'ape') AND lower({key}) = 'genre')
         OR (blk.key = 'riff_info' AND {key} = 'IGNR')"
    );
    if artist {
        keys += &format!(
            "
         OR (blk.key = 'id3v2' AND {key} = 'TPE1')
         OR (blk.key = 'mp4_ilst' AND {key} = char(169) || 'ART')
         OR (blk.key IN ('vorbis_comments', 'ape') AND lower({key}) = 'artist')
         OR (blk.key = 'riff_info' AND {key} = 'IART')
         OR (blk.key = 'aiff_text' AND {key} = 'AUTH')
         OR (blk.key = 'id3v1' AND {key} = 'artist')"
        );
    }
    keys
}

/// What rekordbox says a Library track's genre is, from the trusted entry
/// attach would pick (`{order}` is [`crate::attach::PICK_ORDER`]): `NULL`
/// for a track rekordbox doesn't know, `''` for a known track whose genre
/// is blank. Needs `lt` (`library_track`).
const REKORDBOX_GENRE: &str =
    "(SELECT coalesce(CAST(json_extract(rt.attributes, '$.Genre') AS TEXT), '')
                       FROM rekordbox_track rt
                       LEFT JOIN recording_file rf
                              ON rf.file_id = rt.file_id AND rf.recording_id = rt.recording_id
                       WHERE rt.recording_id = lt.recording_id AND rt.relink_probable = 0
                       ORDER BY {order}
                       LIMIT 1)";

/// Each Library track's `id`, what rekordbox says (`rb`) and the file's
/// genre frames (`digest`), as [`genres_sql`] describes.
fn genre_source_sql() -> String {
    // `rb` is `''` for a known track whose genre is blank.
    let tag_needed = if BLANK_GENRE_FALLS_BACK {
        "g.rb IS NULL OR trim(g.rb) = ''"
    } else {
        "g.rb IS NULL"
    };
    format!(
        "SELECT g.id AS id, g.rb AS rb,
                CASE WHEN g.fid IS NOT NULL AND ({tag_needed}) THEN (
                    SELECT json_group_object(b.block, json(b.items)) FROM (
                        SELECT blk.key AS block, json_group_array(json(i.value)) AS items
                        FROM file f, json_each(f.raw_tags) blk, json_each(blk.value) i
                        WHERE f.id = g.fid AND json_type(blk.value) = 'array' AND ({frames})
                        GROUP BY blk.key) b) END AS digest
         FROM (SELECT lt.id AS id, lt.linked_file_id AS fid,
                      {rekordbox_genre} AS rb
               FROM library_track lt) g",
        frames = frame_filter(false),
        rekordbox_genre = REKORDBOX_GENRE.replace("{order}", crate::attach::PICK_ORDER),
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
            tracks: spellings
                .values()
                .fold(0u32, |sum, n| sum.saturating_add(*n)),
            name: spellings
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(name, _)| name.clone())
                .unwrap_or_default(),
        })
        .collect();
    genres.sort_by(|a, b| b.tracks.cmp(&a.tracks).then_with(|| a.name.cmp(&b.name)));
    let with_genre = genres
        .iter()
        .fold(0u32, |sum, g| sum.saturating_add(g.tracks));
    Ok((genres, tracks.saturating_sub(with_genre)))
}

/// Library tracks whose linked file has tags and no embedded cover. The
/// tags layer records a picture as a binary frame under the key of its
/// format (`tags::raw`): `APIC` in ID3v2 (a description may follow after a
/// colon), `covr` in MP4, `METADATA_BLOCK_PICTURE` in Vorbis comments
/// and FLAC, and an item named `Cover Art (...)` in APE. A file whose tags
/// were never read (`raw_tags` is NULL) is unknown, not without a cover,
/// and a track with no linked file has nothing to look at: neither counts.
const MISSING_ART: &str = "SELECT count(*) FROM library_track lt
     JOIN file f ON f.id = lt.linked_file_id
     WHERE f.raw_tags IS NOT NULL AND NOT EXISTS (
         SELECT 1 FROM json_each(f.raw_tags) blk, json_each(blk.value) i
         WHERE json_type(blk.value) = 'array'
           AND json_extract(i.value, '$.value.type') = 'binary'
           AND json_extract(i.value, '$.value.len') > 0
           AND ((blk.key = 'id3v2' AND (json_extract(i.value, '$.key') = 'APIC'
                    OR substr(json_extract(i.value, '$.key'), 1, 5) = 'APIC:'))
             OR (blk.key = 'mp4_ilst' AND json_extract(i.value, '$.key') = 'covr')
             OR (blk.key = 'vorbis_comments'
                 AND (json_extract(i.value, '$.key') = 'METADATA_BLOCK_PICTURE'
                      OR substr(json_extract(i.value, '$.key'), 1, 23)
                         = 'METADATA_BLOCK_PICTURE:'))
             OR (blk.key = 'ape'
                 AND lower(substr(json_extract(i.value, '$.key'), 1, 9)) = 'cover art')))";

/// Each Library track with what its shown title and artist (and its genre)
/// are made of, for [`missing_tags`]:
/// `(id, rb, title, linked, present, name_ok, known_artist, digest)`.
///
/// `digest` holds the linked file's genre and artist frames, as
/// [`genre_source_sql`] does for the genre alone.
fn missing_tags_sql() -> String {
    let tag_needed = if BLANK_GENRE_FALLS_BACK {
        "s.rb IS NULL OR trim(s.rb) = '' OR s.ka IS NULL"
    } else {
        "s.rb IS NULL OR s.ka IS NULL"
    };
    format!(
        "SELECT s.id, s.rb, s.title, s.fid, s.present, s.name_ok, s.ka,
                CASE WHEN s.fid IS NOT NULL AND ({tag_needed}) THEN (
                    SELECT json_group_object(b.block, json(b.items)) FROM (
                        SELECT blk.key AS block, json_group_array(json(i.value)) AS items
                        FROM file f, json_each(f.raw_tags) blk, json_each(blk.value) i
                        WHERE f.id = s.fid AND json_type(blk.value) = 'array' AND ({frames})
                        GROUP BY blk.key) b) END AS digest
         FROM (SELECT lt.id AS id, lt.linked_file_id AS fid, r.title AS title,
                      f.present AS present,
                      f.rel_path IS NOT NULL AND substr(f.rel_path, -1) <> '/' AS name_ok,
                      {rekordbox_genre} AS rb,
                      (SELECT coalesce(CAST(json_extract(k.attributes, '$.Artist') AS TEXT), '')
                       FROM rekordbox_track k
                       WHERE k.file_id = lt.linked_file_id AND k.relink_probable = 0
                       ORDER BY COALESCE(k.play_count, 0) DESC, k.track_id
                       LIMIT 1) AS ka
               FROM library_track lt
               LEFT JOIN recording r ON r.id = lt.recording_id
               LEFT JOIN file f ON f.id = lt.linked_file_id) s",
        frames = frame_filter(true),
        rekordbox_genre = REKORDBOX_GENRE.replace("{order}", crate::attach::PICK_ORDER),
    )
}

/// How many Library tracks lack a title, an artist or a genre.
///
/// The title and the artist are the ones the lists show
/// ([`crate::send_values::shown`]: rekordbox's for a track rekordbox knows,
/// else the files' tags, the file's name standing in for a missing title),
/// so a title that is only the file's name counts as a title, since it is
/// what the list shows, and a track with a file always has one. The genre is
/// the one of [`genres`].
///
/// One SQL pass hands over, per track, the few values those rules read
/// (rekordbox's artist for the linked file, the linked file's genre and
/// artist frames), and Rust applies the same rules to them, with the tag
/// rules of the send. Only a track those values can't settle is asked of
/// `shown` itself, in chunks: one with no linked file or a gone one
/// (rekordbox may stand in by location), or whose linked file's tags give
/// no artist (another file of the track may). `ask_all` sends every track
/// there instead; the tests use it to prove the short way agrees.
pub fn missing_tags(conn: &Connection, volumes: &impl Volumes) -> rusqlite::Result<u32> {
    missing_tags_by(conn, volumes, false)
}

fn missing_tags_by(
    conn: &Connection,
    volumes: &impl Volumes,
    ask_all: bool,
) -> rusqlite::Result<u32> {
    let mut genre_known: HashMap<(Option<String>, Option<String>), bool> = HashMap::new();
    let mut artist_known: HashMap<String, bool> = HashMap::new();
    let blank = |text: &str| text.trim().is_empty();
    let mut missing = 0u32;
    let mut ask: Vec<i64> = Vec::new();
    let mut stmt = conn.prepare(&missing_tags_sql())?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let rb: Option<String> = row.get(1)?;
        let digest: Option<String> = row.get(7)?;
        let has_genre =
            *genre_known
                .entry((rb, digest.clone()))
                .or_insert_with_key(|(rb, digest)| {
                    genre_of(rb.as_deref(), digest.as_deref()).is_some()
                });
        if !has_genre {
            missing = missing.saturating_add(1);
            continue;
        }
        let title: Option<String> = row.get(2)?;
        let linked: Option<i64> = row.get(3)?;
        let present: Option<bool> = row.get(4)?;
        let name_ok: bool = row.get(5)?;
        let known_artist: Option<String> = row.get(6)?;
        let title_ok = title.as_deref().is_some_and(|t| !blank(t)) || (linked.is_some() && name_ok);
        // Settled by the values at hand: Some(has an artist).
        let artist = match (&known_artist, present, &digest) {
            _ if ask_all || !title_ok => None,
            // rekordbox's entry for the linked file: its artist, blank or not.
            (Some(artist), _, _) => Some(!blank(artist)),
            // A linked file on disk: its tags, if they give one.
            (None, Some(true), Some(digest)) => {
                let given = *artist_known
                    .entry(digest.clone())
                    .or_insert_with_key(|digest| {
                        crate::send_values::tag_values(digest, &["Artist"])
                            .iter()
                            .any(|v| !blank(&v.value))
                    });
                given.then_some(true)
            }
            _ => None,
        };
        match artist {
            Some(true) => {}
            Some(false) => missing = missing.saturating_add(1),
            None => ask.push(id),
        }
    }
    for chunk in ask.chunks(SHOWN_CHUNK) {
        let wanted: Vec<LibraryTrackId> = chunk.iter().map(|id| LibraryTrackId(*id)).collect();
        let mut shown = crate::send_values::shown(conn, volumes, &wanted)?;
        for id in chunk {
            let (title, artist) = shown
                .remove(&LibraryTrackId(*id))
                .map_or((None, None), crate::send_values::Shown::into_options);
            if title.is_none() || artist.is_none() {
                missing = missing.saturating_add(1);
            }
        }
    }
    Ok(missing)
}

/// How many tracks' shown values are worked out in one go.
const SHOWN_CHUNK: usize = 5_000;

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
pub fn overview_at(
    conn: &Connection,
    volumes: &impl Volumes,
    now_ms: i64,
) -> rusqlite::Result<Overview> {
    let tx = conn.unchecked_transaction()?;
    let conn = &*tx;

    let since: String =
        conn.query_row(&format!("SELECT {}", week_start()), [now_ms], |r| r.get(0))?;
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
    let rekordbox_read =
        conn.query_row("SELECT EXISTS (SELECT 1 FROM rekordbox_track)", [], |r| {
            r.get::<_, bool>(0)
        })?;
    let (genres, no_genre) = genres(conn, tracks)?;
    let missing_tags = missing_tags(conn, volumes)?;
    let missing_art = conn.query_row(MISSING_ART, [], |r| r.get::<_, i64>(0))?;
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
        missing_tags,
        missing_art: count(missing_art),
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
    Ok(reads.read(|conn| overview_at(conn, &system_volumes(), now_ms))?)
}
