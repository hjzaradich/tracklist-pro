//! The query function: what a search text finds.

use std::collections::HashMap;

use rusqlite::Connection;

use super::{MAX_TERMS, MAX_TERM_CHARS};

/// Why a file is hidden (ROADMAP 1.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The user hid it.
    UserHid,
    /// It was in the Inbox and the Inbox was cleared.
    ClearedInbox,
    /// An extra copy of a track whose best file is another file.
    ExtraFile,
}

/// A hidden file of a track that matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenFile {
    pub file_id: i64,
    pub recording_id: i64,
    pub why: Why,
    /// The track has a file that isn't hidden, so the track is in the
    /// results already. Owner decision, 2026-10-12: "N hidden files match"
    /// counts only the hidden files for which this is `false`.
    pub track_has_shown_file: bool,
}

/// What a search text found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Every matching track (`recording.id`), hidden files or not, in
    /// ascending order. Both lists keep their own order and filter by it;
    /// the Library list through `library_track.recording_id`.
    pub tracks: Vec<i64>,
    /// The hidden files of the matching tracks, by file id.
    pub hidden: Vec<HiddenFile>,
    /// The index is behind: some track changed and its row isn't worked
    /// out yet ([`pending`]), so this may miss or wrongly include it.
    pub behind: bool,
}

/// The FTS5 MATCH expression for a search text, or `None` when the text has
/// no word to look for (nothing filters then).
///
/// The text is never passed to MATCH as it is. It is split at spaces; a
/// word with no letter or digit is dropped; every other word becomes a
/// quoted phrase (`"` doubled) followed by `*`: so operators, column
/// filters, quotes, stars, colons, minus signs and `AND`/`OR`/`NEAR` are
/// only text, and the word matches from its start. The words are all
/// required. At most [`MAX_TERMS`] words of [`MAX_TERM_CHARS`] characters
/// count.
pub fn match_expression(text: &str) -> Option<String> {
    // A control character (a NUL, say) is no part of a word.
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let terms: Vec<String> = text
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .take(MAX_TERMS)
        .map(|word| {
            let word: String = word.chars().take(MAX_TERM_CHARS).collect();
            format!("\"{}\"*", word.replace('"', "\"\""))
        })
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Whether some track's row is missing or out of date.
pub fn pending(conn: &Connection) -> rusqlite::Result<bool> {
    conn.prepare_cached("SELECT EXISTS (SELECT 1 FROM search_due)")?
        .query_row([], |r| r.get(0))
}

/// The tracks `text` finds, and the hidden files among their files. `None`
/// when the text has no word to look for: the list isn't filtered.
///
/// Cheap at 100,000 tracks: one index lookup, then a lookup of hidden files
/// by track.
pub fn find(conn: &Connection, text: &str) -> rusqlite::Result<Option<Found>> {
    let Some(expression) = match_expression(text) else {
        return Ok(None);
    };
    let mut tracks: Vec<i64> = conn
        .prepare_cached("SELECT rowid FROM search_track WHERE search_track MATCH ?1")?
        .query_map([&expression], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    tracks.sort_unstable();

    let hidden = hidden_files(conn, &tracks, tracks.len() <= BY_TRACK_UP_TO)?;
    Ok(Some(Found {
        tracks,
        hidden,
        behind: pending(conn)?,
    }))
}

/// Up to this many matching tracks, their hidden files are looked up track
/// by track; for more, the (few) hidden files of the whole library are
/// walked and those of matching tracks kept.
const BY_TRACK_UP_TO: usize = 300;

/// The hidden files of the tracks `tracks` (sorted), by file id: looked up
/// track by track (`by_track`), or by walking every hidden file.
pub(crate) fn hidden_files(
    conn: &Connection,
    tracks: &[i64],
    by_track: bool,
) -> rusqlite::Result<Vec<HiddenFile>> {
    if tracks.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(i64, i64, String, Option<String>)> = if by_track {
        let ids = serde_json::to_string(tracks).expect("numbers always serialize");
        conn.prepare_cached(
            "SELECT rf.recording_id, rf.file_id, rf.role, f.hidden_reason
             FROM recording_file rf JOIN file f ON f.id = rf.file_id
             WHERE rf.recording_id IN (SELECT value FROM json_each(?1))
               AND (rf.role = 'extra' OR f.hidden_reason IS NOT NULL)
             ORDER BY rf.file_id",
        )?
        .query_map([&ids], row)?
        .collect::<rusqlite::Result<_>>()?
    } else {
        conn.prepare_cached(
            "SELECT rf.recording_id, rf.file_id, rf.role, f.hidden_reason
             FROM recording_file rf JOIN file f ON f.id = rf.file_id
             WHERE rf.role = 'extra'
             UNION
             SELECT rf.recording_id, rf.file_id, rf.role, f.hidden_reason
             FROM file f JOIN recording_file rf ON rf.file_id = f.id
             WHERE f.hidden_reason IS NOT NULL
             ORDER BY 2",
        )?
        .query_map([], row)?
        .collect::<rusqlite::Result<_>>()?
    };
    let mut hidden = Vec::new();
    let mut shown: HashMap<i64, bool> = HashMap::new();
    for (recording_id, file_id, role, reason) in rows {
        if tracks.binary_search(&recording_id).is_err() {
            continue;
        }
        let why = match reason.as_deref() {
            Some("user") => Why::UserHid,
            Some(_) => Why::ClearedInbox,
            None => {
                debug_assert_eq!(role, "extra");
                Why::ExtraFile
            }
        };
        let track_has_shown_file = match shown.get(&recording_id) {
            Some(&known) => known,
            None => {
                let known = has_shown_file(conn, recording_id)?;
                shown.insert(recording_id, known);
                known
            }
        };
        hidden.push(HiddenFile {
            file_id,
            recording_id,
            why,
            track_has_shown_file,
        });
    }
    Ok(hidden)
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64, String, Option<String>)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
}

/// Whether the track has a file that isn't hidden.
fn has_shown_file(conn: &Connection, recording_id: i64) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (
             SELECT 1 FROM recording_file rf JOIN file f ON f.id = rf.file_id
             WHERE rf.recording_id = ?1 AND rf.role <> 'extra' AND f.hidden_reason IS NULL)",
    )?
    .query_row([recording_id], |r| r.get(0))
}
