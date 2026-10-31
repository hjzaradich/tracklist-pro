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
//! - **Energy** is the one `recording_display` shows: the user's, else
//!   Mixed In Key's, else the local estimate's, else the audio model's
//!   (ROADMAP §2, `analysis`), counted per level 1 to 10 ([`ENERGY_LEVELS`])
//!   with the tracks that have none. A track with no energy from any source
//!   counts as having none.
//! - **Outside the Library** is the tracks in All music that are not
//!   Library tracks ([`ALL_MUSIC`]).
//!
//! **Left out on purpose**, each with the function a later job should use:
//! - *Unsent changes* ("Send N changes", "send due"): what a send carries
//!   depends on rekordbox's export (a known track goes only inside a sent
//!   crate, a new one always), so a count is only right once `send::review`
//!   has read it; no cheaper signal from the log is exact (ROADMAP 1.13).
//! - *How many need a decision*: the Review screen's read model (1bD-1/2).
//! - **Missing tags** is a Library track with no title, no artist or no
//!   genre ([`missing_tag_ids`]); **missing art** one whose linked file holds
//!   no picture ([`missing_art_sql`]). Both are per-track definitions that
//!   read tag data, so they are not counted here: the index job works each
//!   Library track's two answers out with [`gaps_of`], in the same step as
//!   the rest of its `track_row`, and keeps them as two flag bits
//!   (`search::flag::MISSING_TAGS`, `MISSING_ART`). The Overview counts the
//!   bits, and the Library's two filters read them, so the number and the
//!   rows behind it are one definition (1cC-U1b).
//! - **Low quality** and **energy** go the same way (1cD-R1a): [`door_answers_of`]
//!   holds the two definitions, the index job keeps a Library track's answers
//!   in `search::flag::LOW_QUALITY` and `ENERGY`, and the Overview's numbers
//!   and the Library's doors both read those ([`stored_counts`]).
//! - **Genre** goes the same way (1cD-R1b): [`door_answers_of`] also works out
//!   a Library track's genre ([`genres_of`], the one place that says what it
//!   is), the index job keeps it in `track_row.genre` and `genre_group`
//!   (migration 0044), and the Overview's genre bars ([`genres`]) and the
//!   Library's genre door (`TableFilter::Genre`) both read those.
//!
//! **Cost.** Every number is an aggregate SQL statement on the read pool.
//! The Library's numbers come from one pass over `library_track` that looks
//! the rest up by primary key or index; a test reads each statement's query
//! plan to prove nothing else is scanned. The genres statement groups the
//! tracks by what their tags say and Rust reads the *distinct* groups only
//! ([`genres_of`], for a page of the index job); the Overview itself counts
//! the stored genres, one row per distinct spelling ([`genres`]). The two tag gaps are the index job's work, not the
//! Overview's: [`gaps_of`] hands Rust one small row per Library track of a
//! page of the index, and `shown_tags` is asked only about the tracks those
//! rows can't settle.
//!
//! **Still counting** ([`Counting`]): a flag per source of numbers that can
//! change while background work runs. A flag is true while a job of one of
//! its kinds is queued or running, and only then: a failed or cancelled job
//! holds nothing, and neither does a stale verdict with no job to settle it
//! (a stale verdict is no verdict: it isn't counted as low quality). The
//! exception is `index`: the numbers read from `track_row` are as current
//! as the search index, so it is true while any track is due
//! (`search::pending`), which is what an upgrade leaves behind until the
//! index job has filled the bits.
//!
//! **One place to change** each choice the owner answered (2026-10-12):
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

#[cfg(test)]
mod tests;

/// Whether an operation that was undone still counts as a decision made
/// (owner decision: no, a decision taken back no longer stands).
pub const COUNT_UNDONE_OPERATIONS: bool = false;

/// How far back "this week" reaches: a rolling week ending now, in UTC
/// (owner decision: the last 7 days, not the calendar week). It is an
/// SQLite date modifier.
pub const THIS_WEEK: &str = "-7 days";

/// Whether a track rekordbox knows, but whose genre in rekordbox is blank,
/// takes the genre of its file's tag (owner decision: no, rekordbox's blank
/// is the answer).
pub const BLANK_GENRE_FALLS_BACK: bool = false;

/// The levels of energy, 1 to 10 (the check on `analysis.energy`).
pub const ENERGY_LEVELS: usize = 10;

/// The verdicts that make a track "made from a low-quality file".
const LOW_QUALITY_VERDICTS: &str = "'low_bitrate', 'suspect_transcode', 'truncated', 'broken'";

/// A Library track is in a crate when a static crate holds it. A smart
/// crate's entries are always-include and never-include rules, not
/// membership. Playlists arrive in Phase 3.
pub(crate) const IN_A_CRATE: &str = "EXISTS (SELECT 1 FROM crate_entry ce
                                  WHERE ce.library_track_id = lt.id AND ce.kind = 'member')";

/// The track has a row from rekordbox in `analysis` (see the module docs).
pub(crate) const ANALYZED: &str = "EXISTS (SELECT 1 FROM analysis a
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
    /// A quality job is queued or running: the low-quality count.
    pub quality: bool,
    /// The search index is behind (a track is due): the low-quality, energy,
    /// genre, missing-tags and missing-art counts, which are read from the
    /// rows the index job writes.
    pub index: bool,
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
    /// Library tracks per energy level: entry 0 is energy 1 and entry 9 is
    /// energy 10 ([`ENERGY_LEVELS`] entries), the energy a track shows.
    pub energy: Vec<u32>,
    /// Library tracks with no energy.
    pub no_energy: u32,
    /// Tracks in All music that are not Library tracks: what "Add to
    /// Library" could still bring in.
    pub outside_library: u32,
    /// Library tracks with no title, no artist or no genre, as the lists
    /// show them ([`missing_tag_ids`]), counted from the flag the index job
    /// keeps (`counting.index` says when it is behind).
    pub missing_tags: u32,
    /// Library tracks whose linked file has tags and no embedded cover
    /// ([`missing_art_sql`]), counted from the flag the index job keeps.
    pub missing_art: u32,
    pub counting: Counting,
}

/// A Library track's linked file with its quality rows, as `f`, `q` and `v`,
/// for a query over `library_track lt`. The one text the Overview's numbers
/// and the Library's door filters (`track_table`) both read a track's file
/// through, so a number and the rows behind it can't differ in which file
/// they look at.
pub(crate) const LINKED_FILE: &str = "LEFT JOIN file f ON f.id = lt.linked_file_id
               LEFT JOIN file_quality q ON q.file_id = f.id
               LEFT JOIN file_verdict v ON v.file_id = f.id";

/// `1` for a Library track whose linked file has a current verdict that
/// flags it, else `0`: a missing or stale verdict is no verdict, never `ok`,
/// and isn't counted. SQL over [`LINKED_FILE`]; the one place that says what
/// "made from a low-quality file" means. The index job keeps the answer
/// ([`door_answers_of`]); the Overview's count and the Library's "Low-quality
/// file" filter read that.
pub(crate) fn low_quality_sql() -> String {
    format!(
        "CASE WHEN f.id IS NULL OR {stale} THEN 0
              WHEN v.verdict IN ({LOW_QUALITY_VERDICTS}) THEN 1
              ELSE 0 END",
        stale = stale_condition()
    )
}

/// A condition on a `track_row` `tr`: the row's Library track exists and
/// `condition` holds for it (SQL over `lt` and [`LINKED_FILE`]). How the
/// Library's "analyzed" door filters say what the Overview's numbers say, in
/// the very same SQL text.
pub(crate) fn library_track_where(condition: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM library_track lt {LINKED_FILE}
                 WHERE lt.recording_id = tr.recording_id AND ({condition}))"
    )
}

/// The Library's numbers in one pass over `library_track`. Each other table
/// is looked up by its primary key or an index. `?1` is the start of this
/// week.
fn library_sql() -> String {
    format!(
        "SELECT count(*),
                coalesce(sum(s.duration_ms), 0),
                coalesce(sum(s.in_a_crate), 0),
                coalesce(sum(s.analyzed), 0),
                coalesce(sum(s.added >= ?1), 0)
         FROM (SELECT f.duration_ms AS duration_ms,
                      {IN_A_CRATE} AS in_a_crate,
                      {ANALYZED} AS analyzed,
                      lt.added_at AS added
               FROM library_track lt
               LEFT JOIN file f ON f.id = lt.linked_file_id) s"
    )
}

/// Tracks with a file that isn't hidden, and how many of them are not
/// Library tracks. A track is counted once however many files it has.
const ALL_MUSIC: &str = concat!(
    "SELECT count(*),
            coalesce(sum(NOT EXISTS (SELECT 1 FROM library_track lt
                                     WHERE lt.recording_id = r.id)), 0)
     FROM recording r WHERE ",
    crate::hidden::has_shown_file!("r.id")
);

/// SQL for "the energy a track shows": the energy of the `analysis` row that
/// ranks first among those with an energy, the user's, then Mixed In Key's,
/// then the local estimate's, then the audio model's (the order of the
/// `recording_display` view, which also works out BPM and key), or `NULL`.
/// `$recording` is a SQL expression for the track's id. The one place that
/// says which energy a track shows: the index job keeps a Library track's
/// answer for the Overview's chart and its doors ([`door_answers_of`]), and a
/// crate's summary line (`crates::summary`) and energy as stars
/// (`energy_stars`) read it here. Each `analysis` row is found by its unique
/// (track, source) index.
macro_rules! shown_energy {
    ($recording:literal) => {
        concat!(
            "(SELECT a.energy FROM analysis a
               WHERE a.recording_id = ",
            $recording,
            " AND a.energy IS NOT NULL
               ORDER BY CASE a.source
                   WHEN 'user' THEN 1 WHEN 'mik' THEN 2 WHEN 'local' THEN 3 WHEN 'ml' THEN 4
               END
               LIMIT 1)"
        )
    };
}
pub(crate) use shown_energy;

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

/// Each Library track's genre source: what rekordbox says (`NULL`
/// for a track rekordbox doesn't know) and, only where the file's tag is
/// needed, the genre frames of the linked file as a `raw_tags` object, one
/// array per block, in the file's own order. Rust then reads each distinct
/// pair with the send's own tag rules ([`crate::send_values::tag_values`]),
/// so a tag means the same here as in a send ([`genres_of`]).
///
/// The frames kept are the ones `send_values::tag_fields` maps to `Genre`:
/// `TCON` (ID3v2), `©gen` (MP4), `GENRE` in any case (Vorbis, APE) and
/// `IGNR` (RIFF INFO). Nothing else in a tag is looked at.
/// The frames of a file's tags that `send_values::tag_fields` maps to
/// `Genre` (and, with `more`, to `Name` and `Artist`), as a condition on a
/// block (`blk.key`) and an item (`i.value`) of `raw_tags`. Keys are the ones
/// the tags layer writes: `TCON` / `TIT2` / `TPE1` (ID3v2), `©gen` /
/// `©nam` / `©ART` (MP4), `genre` / `title` / `artist` in any case
/// (Vorbis, APE), `IGNR` / `INAM` / `IART` (RIFF INFO), and for the title and
/// artist `NAME` and `AUTH` (AIFF) and `title` and `artist` (ID3v1).
fn frame_filter(more: bool) -> String {
    let key = "json_extract(i.value, '$.key')";
    let mut keys = format!(
        "(blk.key = 'id3v2' AND {key} = 'TCON')
         OR (blk.key = 'mp4_ilst' AND {key} = char(169) || 'gen')
         OR (blk.key IN ('vorbis_comments', 'ape') AND lower({key}) = 'genre')
         OR (blk.key = 'riff_info' AND {key} = 'IGNR')"
    );
    if more {
        keys += &format!(
            "
         OR (blk.key = 'id3v2' AND {key} IN ('TIT2', 'TPE1'))
         OR (blk.key = 'mp4_ilst' AND {key} IN (char(169) || 'nam', char(169) || 'ART'))
         OR (blk.key IN ('vorbis_comments', 'ape') AND lower({key}) IN ('title', 'artist'))
         OR (blk.key = 'riff_info' AND {key} IN ('INAM', 'IART'))
         OR (blk.key = 'aiff_text' AND {key} IN ('NAME', 'AUTH'))
         OR (blk.key = 'id3v1' AND {key} IN ('title', 'artist'))"
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
                         AND {pickable}
                       ORDER BY {order}
                       LIMIT 1)";

/// Each Library track's `id`, what rekordbox says (`rb`) and the file's
/// genre frames (`digest`), as [`genres_of`] reads them. `scoped` limits it
/// to the Library tracks of `?1`, a JSON array of their ids (a page of the
/// index job).
fn genre_source_sql(scoped: bool) -> String {
    let scope = if scoped {
        "WHERE lt.id IN (SELECT value FROM json_each(?1))"
    } else {
        ""
    };
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
               FROM library_track lt
               {scope}) g",
        frames = frame_filter(false),
        rekordbox_genre = REKORDBOX_GENRE
            .replace("{order}", crate::attach::PICK_ORDER)
            .replace("{pickable}", crate::attach::PICKABLE),
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

/// The genre of each of the Library tracks `ids`, by Library track id, for
/// those that have one: **the one definition** of a track's genre, as
/// [`gaps_of`] is for the gaps. The index job calls it through
/// [`door_answers_of`] for each page of tracks it writes and keeps the
/// answer in `track_row.genre` and `genre_group`; the Overview's genre bars
/// count those and the Library's genre door lists them.
///
/// The genre is rekordbox's for a track rekordbox knows (blank there means
/// none, [`BLANK_GENRE_FALLS_BACK`]), else the linked file's tag, read with
/// the send's own rules; outer blanks are dropped.
fn genres_of(conn: &Connection, ids: &[LibraryTrackId]) -> rusqlite::Result<HashMap<i64, String>> {
    let json = serde_json::to_string(&ids.iter().map(|id| id.0).collect::<Vec<_>>())
        .expect("numbers always serialize");
    // Most tracks of a page share a source (a few hundred distinct frames at
    // most), and parsing one is the cost, so each is read once.
    let mut known: HashMap<(Option<String>, Option<String>), Option<String>> = HashMap::new();
    let mut stmt = conn.prepare_cached(&genre_source_sql(true))?;
    let mut rows = stmt.query([json])?;
    let mut out = HashMap::new();
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let source: (Option<String>, Option<String>) = (row.get(1)?, row.get(2)?);
        let genre = known
            .entry(source)
            .or_insert_with_key(|(rb, digest)| genre_of(rb.as_deref(), digest.as_deref()));
        if let Some(genre) = genre {
            out.insert(id, genre.clone());
        }
    }
    Ok(out)
}

/// The key a genre is counted and filtered by: capitals don't make a genre
/// of its own. Used for `track_row.genre_group` and by the genre door.
pub(crate) fn genre_group(genre: &str) -> String {
    genre.trim().to_lowercase()
}

/// Every genre with its track count, and the tracks without one, from the
/// genres the index job keeps (`track_row.genre`, `genre_group`): one row per
/// distinct spelling, read from the index. While the index is behind the
/// counts are behind (`counting.index`).
fn genres(conn: &Connection, tracks: u32) -> rusqlite::Result<(Vec<GenreCount>, u32)> {
    let mut stmt = conn.prepare_cached(stored_genres_sql())?;
    let spellings = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, count(r.get::<_, i64>(1)?)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let genres = rank_genres(spellings);
    let with_genre = genres
        .iter()
        .fold(0u32, |sum, g| sum.saturating_add(g.tracks));
    Ok((genres, tracks.saturating_sub(with_genre)))
}

/// How many Library tracks spell their genre each way (`track_row.genre`),
/// from the index on `genre_group` and `genre`. Only a Library track's row
/// has a genre (see [`door_answers_of`]).
fn stored_genres_sql() -> &'static str {
    "SELECT genre, count(*) FROM track_row
     WHERE genre_group IS NOT NULL
     GROUP BY genre_group, genre"
}

/// Genres with their tracks, from `(spelling, tracks)` pairs: most tracks
/// first, then by name. Capitals don't make another genre; its name is the
/// spelling most of its tracks use (then the smallest, so the answer never
/// wobbles).
fn rank_genres(spellings: impl IntoIterator<Item = (String, u32)>) -> Vec<GenreCount> {
    let mut by_name: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
    for (genre, n) in spellings {
        let count = by_name
            .entry(genre_group(&genre))
            .or_default()
            .entry(genre)
            .or_default();
        *count = count.saturating_add(n);
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
    genres
}

/// A condition on a Library track's linked file `f`: it has tags and no
/// embedded cover. The tags layer records a picture as a binary frame under
/// the key of its format (`tags::raw`): `APIC` in ID3v2 (a description may
/// follow after a colon), `covr` in MP4, `METADATA_BLOCK_PICTURE` in Vorbis
/// comments and FLAC, and an item named `Cover Art (...)` in APE. A file
/// whose tags were never read (`raw_tags` is NULL) is unknown, not without a
/// cover.
const NO_COVER: &str = "f.raw_tags IS NOT NULL AND NOT EXISTS (
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

/// The ids of the Library tracks whose linked file has tags and no embedded
/// cover ([`NO_COVER`]): the definition of "missing art". A track with no
/// linked file has nothing to look at and isn't listed. `scoped` limits it
/// to the Library tracks of `?1`, a JSON array of their ids.
fn missing_art_sql(scoped: bool) -> String {
    let scope = if scoped {
        "lt.id IN (SELECT value FROM json_each(?1)) AND "
    } else {
        ""
    };
    format!(
        "SELECT lt.id FROM library_track lt
         JOIN file f ON f.id = lt.linked_file_id
         WHERE {scope}{NO_COVER}"
    )
}

/// Each Library track with what its title, artist and genre are made of,
/// for [`missing_tag_ids`]: `(id, rb, own_title, own_artist, linked, present,
/// known_name, known_artist, digest)`.
///
/// `known_name` and `known_artist` are the `Name` and `Artist` of the
/// trusted rekordbox entry for the linked file (`NULL` when there is none,
/// `''` for a blank one). `digest` holds the linked file's genre, title
/// and artist frames, as [`genre_source_sql`] does for the genre alone.
/// `scoped` limits it to the Library tracks of `?1`, a JSON array of their
/// ids (a page of the index job).
fn missing_tags_sql(scoped: bool) -> String {
    let scope = if scoped {
        "WHERE lt.id IN (SELECT value FROM json_each(?1))"
    } else {
        ""
    };
    let tag_needed = if BLANK_GENRE_FALLS_BACK {
        "s.rb IS NULL OR trim(s.rb) = '' OR s.ka IS NULL"
    } else {
        "s.rb IS NULL OR s.ka IS NULL"
    };
    format!(
        "SELECT s.id, s.rb, s.title, s.artist, s.fid, s.present, s.kn, s.ka,
                CASE WHEN s.fid IS NOT NULL AND ({tag_needed}) THEN (
                    SELECT json_group_object(b.block, json(b.items)) FROM (
                        SELECT blk.key AS block, json_group_array(json(i.value)) AS items
                        FROM file f, json_each(f.raw_tags) blk, json_each(blk.value) i
                        WHERE f.id = s.fid AND json_type(blk.value) = 'array' AND ({frames})
                        GROUP BY blk.key) b) END AS digest
         FROM (SELECT lt.id AS id, lt.linked_file_id AS fid, r.title AS title,
                      r.artist AS artist, f.present AS present,
                      {rekordbox_genre} AS rb,
                      (SELECT coalesce(CAST(json_extract(k.attributes, '$.Name') AS TEXT), '')
                       FROM rekordbox_track k
                       WHERE k.file_id = lt.linked_file_id AND k.relink_probable = 0
                       ORDER BY COALESCE(k.play_count, 0) DESC, k.track_id
                       LIMIT 1) AS kn,
                      (SELECT coalesce(CAST(json_extract(k.attributes, '$.Artist') AS TEXT), '')
                       FROM rekordbox_track k
                       WHERE k.file_id = lt.linked_file_id AND k.relink_probable = 0
                       ORDER BY COALESCE(k.play_count, 0) DESC, k.track_id
                       LIMIT 1) AS ka
               FROM library_track lt
               LEFT JOIN recording r ON r.id = lt.recording_id
               LEFT JOIN file f ON f.id = lt.linked_file_id
               {scope}) s",
        frames = frame_filter(true),
        rekordbox_genre = REKORDBOX_GENRE
            .replace("{order}", crate::attach::PICK_ORDER)
            .replace("{pickable}", crate::attach::PICKABLE),
    )
}

/// Which Library tracks lack a title, an artist or a genre, and which have
/// tags and no cover: the two per-track definitions the Overview counts
/// ("tracks with missing tags", "tracks with missing art"). Library track
/// ids.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Gaps {
    pub tags: HashSet<i64>,
    pub art: HashSet<i64>,
}

/// The two gaps of the Library tracks `ids`: **the one definition**. The
/// index job calls it for each page of tracks it writes, and keeps the answer
/// as two flag bits of the track's `track_row`; the Overview counts the bits
/// and the Library's two filters list them. Nothing else says what a missing
/// tag or a missing cover is.
pub(crate) fn gaps_of(
    conn: &Connection,
    volumes: &impl Volumes,
    ids: &[LibraryTrackId],
) -> rusqlite::Result<Gaps> {
    if ids.is_empty() {
        return Ok(Gaps::default());
    }
    Ok(Gaps {
        tags: missing_tag_ids(conn, volumes, false, Some(ids))?
            .into_iter()
            .collect(),
        art: missing_art_ids(conn, ids)?.into_iter().collect(),
    })
}

/// A Library track's doors' answers, for the Library tracks of `?1`, a JSON
/// array of their ids: `(id, low quality, energy)`.
fn door_answers_sql() -> String {
    format!(
        "SELECT lt.id, {low_quality}, {energy}
         FROM library_track lt
         {LINKED_FILE}
         WHERE lt.id IN (SELECT value FROM json_each(?1))",
        low_quality = low_quality_sql(),
        energy = shown_energy!("lt.recording_id"),
    )
}

/// What the Overview's doors ask of one Library track, besides its gaps.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DoorAnswer {
    /// Made from a low-quality file ([`low_quality_sql`]).
    pub low_quality: bool,
    /// The energy it shows, 1 to 10 ([`shown_energy!`]).
    pub energy: Option<u8>,
    /// Its genre, without outer blanks ([`genres_of`]); `None` for none.
    pub genre: Option<String>,
}

/// The doors' answers of the Library tracks `ids`, by Library track id:
/// **the one definition**, as [`gaps_of`] is for the gaps. The index job
/// calls it for each page of tracks it writes and keeps the answers in the
/// flags (and the genre columns) of the track's `track_row`; the Overview
/// counts them and the Library's doors list them. Another definition that
/// costs too much per row to run in a table filter belongs here, in the same
/// way.
pub(crate) fn door_answers_of(
    conn: &Connection,
    ids: &[LibraryTrackId],
) -> rusqlite::Result<HashMap<i64, DoorAnswer>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let json = serde_json::to_string(&ids.iter().map(|id| id.0).collect::<Vec<_>>())
        .expect("numbers always serialize");
    let mut answers: HashMap<i64, DoorAnswer> = conn
        .prepare_cached(&door_answers_sql())?
        .query_map([json], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                DoorAnswer {
                    low_quality: r.get(1)?,
                    energy: r.get(2)?,
                    genre: None,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (id, genre) in genres_of(conn, ids)? {
        if let Some(answer) = answers.get_mut(&id) {
            answer.genre = Some(genre);
        }
    }
    Ok(answers)
}

/// The ids (among `ids`) of the Library tracks with a linked file that has tags
/// and no cover ([`missing_art_sql`]).
fn missing_art_ids(conn: &Connection, ids: &[LibraryTrackId]) -> rusqlite::Result<Vec<i64>> {
    let json = serde_json::to_string(&ids.iter().map(|id| id.0).collect::<Vec<_>>())
        .expect("numbers always serialize");
    conn.prepare_cached(&missing_art_sql(true))?
        .query_map([json], |r| r.get(0))?
        .collect()
}

/// The Library tracks that lack a title, an artist or a genre (owner
/// decision, ROADMAP 1.13).
///
/// - **Title and artist** come from the same rules the lists use
///   ([`crate::send_values::shown`]): the track's own, else rekordbox's
///   `Name` and `Artist` for a track rekordbox knows, else the Title and
///   Artist tags of its files. The one difference: **the file's name does not
///   stand in for a missing title**. A track rekordbox doesn't know whose
///   files have no Title tag has no title, and so does a known track whose
///   `Name` in rekordbox is blank.
/// - **Genre** is the one of [`genres`].
///
/// One SQL pass hands over, per track, the few values those rules read
/// (rekordbox's name and artist for the linked file, the linked file's
/// genre, title and artist frames), and Rust applies the same rules to them,
/// with the tag rules of the send. Only a track those values can't settle is
/// asked of `shown_tags` itself, in chunks: one with no linked file or a
/// gone one (rekordbox may stand in by location), or whose linked file's
/// tags lack a title or an artist (another file of the track may have it).
///
/// The ids are Library track ids. `only` limits the answer to those tracks
/// (the index job's page); `ask_all` sends every track to `shown_tags`
/// instead of settling some by the values at hand; the tests use it to prove
/// the short way agrees.
pub(crate) fn missing_tag_ids(
    conn: &Connection,
    volumes: &impl Volumes,
    ask_all: bool,
    only: Option<&[LibraryTrackId]>,
) -> rusqlite::Result<Vec<i64>> {
    let mut genre_known: HashMap<(Option<String>, Option<String>), bool> = HashMap::new();
    // What the linked file's frames give: (a title, an artist).
    let mut frames_give: HashMap<String, (bool, bool)> = HashMap::new();
    let blank = |text: &str| text.trim().is_empty();
    let filled = |text: &Option<String>| text.as_deref().is_some_and(|t| !blank(t));
    let mut missing: Vec<i64> = Vec::new();
    let mut ask: Vec<i64> = Vec::new();
    let mut stmt = conn.prepare(&missing_tags_sql(only.is_some()))?;
    let mut rows = match only {
        Some(ids) => {
            stmt.query([
                serde_json::to_string(&ids.iter().map(|id| id.0).collect::<Vec<_>>())
                    .expect("numbers always serialize"),
            ])?
        }
        None => stmt.query([])?,
    };
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let rb: Option<String> = row.get(1)?;
        let digest: Option<String> = row.get(8)?;
        let has_genre =
            *genre_known
                .entry((rb, digest.clone()))
                .or_insert_with_key(|(rb, digest)| {
                    genre_of(rb.as_deref(), digest.as_deref()).is_some()
                });
        if !has_genre {
            missing.push(id);
            continue;
        }
        let own_title: Option<String> = row.get(2)?;
        let own_artist: Option<String> = row.get(3)?;
        let linked: Option<i64> = row.get(4)?;
        let present: Option<bool> = row.get(5)?;
        let known_name: Option<String> = row.get(6)?;
        let known_artist: Option<String> = row.get(7)?;
        // For one of title and artist: `Some(has one)` when the values at
        // hand settle it, `None` when `shown_tags` has to.
        let mut settle = |own: &Option<String>,
                          known: &Option<String>,
                          from_tags: &dyn Fn(&(bool, bool)) -> bool| {
            if ask_all || linked.is_none() {
                None
            } else if filled(own) {
                Some(true)
            } else if known.is_some() {
                // rekordbox's entry for the linked file: its value, blank or not.
                Some(filled(known))
            } else if present == Some(true) {
                // A linked file on disk: its tags, if they give one. If not,
                // another file of the track may.
                let given = digest.as_ref().is_some_and(|digest| {
                    from_tags(
                        frames_give
                            .entry(digest.clone())
                            .or_insert_with_key(|digest| {
                                let give = |name: &str| {
                                    crate::send_values::tag_values(digest, &[name])
                                        .iter()
                                        .any(|v| !blank(&v.value))
                                };
                                (give("Name"), give("Artist"))
                            }),
                    )
                });
                given.then_some(true)
            } else {
                None
            }
        };
        let title = settle(&own_title, &known_name, &|g| g.0);
        let artist = settle(&own_artist, &known_artist, &|g| g.1);
        match (title, artist) {
            (Some(true), Some(true)) => {}
            (Some(false), _) | (_, Some(false)) => missing.push(id),
            _ => ask.push(id),
        }
    }
    for chunk in ask.chunks(SHOWN_CHUNK) {
        let wanted: Vec<LibraryTrackId> = chunk.iter().map(|id| LibraryTrackId(*id)).collect();
        let mut shown = crate::send_values::shown_tags(conn, volumes, &wanted)?;
        for id in chunk {
            let (title, artist) = shown
                .remove(&LibraryTrackId(*id))
                .map_or((None, None), crate::send_values::Shown::into_options);
            if title.is_none() || artist.is_none() {
                missing.push(*id);
            }
        }
    }
    Ok(missing)
}

/// How many tracks' shown values are worked out in one go.
const SHOWN_CHUNK: usize = 5_000;

/// The Overview's numbers that are kept in the Library tracks' `track_row`
/// flags ([`gaps_of`] and [`door_answers_of`] say what they mean).
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredCounts {
    low_quality: u32,
    /// Library tracks per energy level, 1 to 10.
    energy: Vec<u32>,
    no_energy: u32,
    missing_tags: u32,
    missing_art: u32,
}

/// How many Library tracks have each mix of the flags [`StoredCounts`] reads.
fn stored_counts_sql() -> String {
    use crate::search::flag::{ENERGY, IN_LIBRARY, LOW_QUALITY, MISSING_ART, MISSING_TAGS};
    format!(
        "SELECT flags & {}, count(*) FROM track_row
         WHERE (flags & {IN_LIBRARY}) <> 0 GROUP BY 1",
        LOW_QUALITY | ENERGY | MISSING_TAGS | MISSING_ART,
    )
}

/// Counts what [`StoredCounts`] holds, in one pass over the narrow `flags`
/// index, with a row per mix of flags: the very bits the Library's doors
/// list, so a number and the rows behind it always agree. While the index is
/// behind they are behind too, which `counting.index` says.
fn stored_counts(conn: &Connection) -> rusqlite::Result<StoredCounts> {
    use crate::search::flag::{self, LOW_QUALITY, MISSING_ART, MISSING_TAGS};
    let mut counts = StoredCounts {
        low_quality: 0,
        energy: vec![0; ENERGY_LEVELS],
        no_energy: 0,
        missing_tags: 0,
        missing_art: 0,
    };
    let mut stmt = conn.prepare_cached(&stored_counts_sql())?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let (flags, tracks) = (row.get::<_, i64>(0)?, count(row.get::<_, i64>(1)?));
        for (bit, number) in [
            (LOW_QUALITY, &mut counts.low_quality),
            (MISSING_TAGS, &mut counts.missing_tags),
            (MISSING_ART, &mut counts.missing_art),
        ] {
            if flags & bit != 0 {
                *number = number.saturating_add(tracks);
            }
        }
        let level = flag::energy_of(flags);
        let number = match usize::try_from(level - 1) {
            Ok(index) => counts.energy.get_mut(index),
            Err(_) => Some(&mut counts.no_energy),
        };
        if let Some(number) = number {
            *number = number.saturating_add(tracks);
        }
    }
    Ok(counts)
}

/// Which sources have background work going, from the queued and running
/// jobs.
fn counting(conn: &Connection) -> rusqlite::Result<Counting> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT kind FROM job WHERE status IN ('queued', 'running')")?;
    let kinds: HashSet<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let any = |names: &[&str]| names.iter().any(|n| kinds.contains(*n));
    Ok(Counting {
        files: any(&FILE_JOBS),
        rekordbox: any(&REKORDBOX_JOBS),
        quality: any(&QUALITY_JOBS),
        index: crate::search::pending(conn)?,
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

    let since: String =
        conn.query_row(&format!("SELECT {}", week_start()), [now_ms], |r| r.get(0))?;
    let (tracks, length, in_crates, analyzed, added) =
        conn.query_row(&library_sql(), params![since], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
    let tracks = count(tracks);
    let (in_crates, analyzed) = (count(in_crates), count(analyzed));
    let (all_music, outside_library) = conn.query_row(ALL_MUSIC, [], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    })?;
    let decisions = conn.query_row(&decisions_sql(), [now_ms], |r| r.get::<_, i64>(0))?;
    let last_sent = crate::send::last_send_ms(conn)?;
    let rekordbox_read =
        conn.query_row("SELECT EXISTS (SELECT 1 FROM rekordbox_track)", [], |r| {
            r.get::<_, bool>(0)
        })?;
    let (genres, no_genre) = genres(conn, tracks)?;
    let stored = stored_counts(conn)?;
    let counting = counting(conn)?;
    drop(tx);

    Ok(Overview {
        tracks,
        total_length_ms: u64::try_from(length).unwrap_or(0),
        in_crates,
        in_no_crate: tracks.saturating_sub(in_crates),
        analyzed_by_rekordbox: analyzed,
        not_analyzed: tracks.saturating_sub(analyzed),
        rekordbox_read,
        low_quality: stored.low_quality,
        last_sent_ms: last_sent.and_then(|ms| u64::try_from(ms).ok()),
        all_music: count(all_music),
        added_this_week: count(added),
        decisions_this_week: count(decisions),
        genres,
        no_genre,
        energy: stored.energy,
        no_energy: stored.no_energy,
        outside_library: count(outside_library),
        missing_tags: stored.missing_tags,
        missing_art: stored.missing_art,
        counting,
    })
}

/// Every number the Overview screen shows, as of now. When the index is
/// behind (`counting.index`), it asks for the index job, as the track table
/// does: the low-quality, energy, genre, missing-tags and missing-art counts
/// come from the rows that job writes, and the screen asks again while it runs.
#[tauri::command]
#[specta::specta]
pub async fn overview(
    reads: State<'_, ReadPool>,
    writer: State<'_, crate::db::Writer>,
    jobs: State<'_, crate::jobs::JobQueue>,
) -> Result<Overview, IpcError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let found = reads.read(|conn| overview_at(conn, now_ms))?;
    if found.counting.index {
        if let Err(e) = crate::search::request(&writer, |job| jobs.enqueue(job)) {
            eprintln!("overview: couldn't ask for the index: {e:?}");
        }
    }
    Ok(found)
}
