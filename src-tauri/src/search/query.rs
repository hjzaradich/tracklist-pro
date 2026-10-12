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

    // Hidden files are few: walk them, and keep those of matching tracks.
    let mut hidden = Vec::new();
    if !tracks.is_empty() {
        let mut stmt = conn.prepare_cached(
            "SELECT rf.recording_id, rf.file_id, rf.role, f.hidden_reason
             FROM recording_file rf JOIN file f ON f.id = rf.file_id
             WHERE rf.role = 'extra'
             UNION
             SELECT rf.recording_id, rf.file_id, rf.role, f.hidden_reason
             FROM file f JOIN recording_file rf ON rf.file_id = f.id
             WHERE f.hidden_reason IS NOT NULL
             ORDER BY 2",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
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
    }
    Ok(Some(Found {
        tracks,
        hidden,
        behind: pending(conn)?,
    }))
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
