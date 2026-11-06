//! Play history (ROADMAP 3.6): the sets the user played, read from m3u8
//! files exported from rekordbox's History, kept as sessions of plays in
//! order. Not the operation log's history ([`crate::ops`]).
//!
//! - One file is one session, never split. Its date is the one in the
//!   file's name if it has one, otherwise the day it was imported; the user
//!   can correct it.
//! - A play is matched to a track by its path ([`matching`]). A play that
//!   matches nothing (a partner's track in a B2B, a file the user doesn't
//!   have) is kept as a play with what the file said.
//! - **Already imported:** a file whose plays, in order, are the plays of a
//!   session already here is refused. Two plays are the same when both
//!   matched the same track, or their paths have the same key. Stated
//!   limit: the same set exported with one play more or less, or from
//!   another computer whose paths and matches both differ, comes in again.
//! - Every write is one operation: importing a file, correcting a date, and
//!   adding the unmatched plays to a wishlist (only while wishlists are on).
//! - rekordbox's own play counts and last played (1.2) are not touched.
//! - Bad lines never stop an import ([`m3u8`]); they are counted.

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tauri::State;

use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::ops::{self, OpsError};
use crate::relink::rules::path_key;
use crate::relink::Mounted;
use crate::wishlist::{self, WishSkipReason, WishSkippedLine, WishTrack, WishlistOutcome};

pub mod m3u8;
pub mod matching;

#[cfg(test)]
mod tests;

/// The operation kinds, as the operation log stores them.
pub const IMPORT_OPERATION: &str = "history_import";
pub const SET_DATE_OPERATION: &str = "history_set_date";

/// The longest session name kept, in characters.
const MAX_NAME_CHARS: usize = 200;

/// Why nothing was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum HistoryRefusal {
    /// The file couldn't be read.
    FileUnreadable,
    /// The file is over [`m3u8::MAX_BYTES`].
    FileTooBig,
    /// The file names no play.
    NoPlays,
    /// The same plays, in the same order, are already a session.
    AlreadyImported,
    NoSuchSession,
    /// Not a day of the calendar as `YYYY-MM-DD`.
    BadDate,
}

/// What a history command came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum HistoryOutcome {
    /// Done, as one operation (`None` when nothing changed).
    #[serde(rename_all = "camelCase")]
    Done {
        operation_id: Option<i64>,
        session_id: i64,
        /// Plays the session holds, and how many of them matched a track.
        plays: u32,
        matched: u32,
        /// Lines of the file that couldn't be read.
        skipped_lines: u32,
    },
    /// Nothing was written. `session_id` names the session it already is,
    /// for [`HistoryRefusal::AlreadyImported`].
    #[serde(rename_all = "camelCase")]
    Refused {
        reason: HistoryRefusal,
        session_id: Option<i64>,
        skipped_lines: u32,
    },
}

fn refused(reason: HistoryRefusal) -> HistoryOutcome {
    HistoryOutcome::Refused {
        reason,
        session_id: None,
        skipped_lines: 0,
    }
}

/// A session, with its counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: i64,
    /// `m3u8` (later also `stick`).
    pub source: String,
    pub name: String,
    /// `YYYY-MM-DD`.
    pub played_on: String,
    pub imported_at: String,
    pub plays: u32,
    pub matched: u32,
}

/// One play of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Play {
    pub id: i64,
    /// Its place in the session, from 0.
    pub position: u32,
    /// The track it matched, if any.
    pub recording_id: Option<i64>,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub path: String,
}

/// The plays of a session, and whether "Add to a wishlist" is offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaysReply {
    pub plays: Vec<Play>,
    /// Wishlists are on: the unmatched plays can go to one.
    pub wishlist_offer: bool,
}

/// "A was played right before B", `count` times over all sessions. For
/// "what's next" (3.4); only plays that matched a track count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub from: i64,
    pub to: i64,
    pub count: u32,
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn optional(value: &Option<String>) -> Value {
    value.as_deref().map_or(Value::Null, text)
}

/// A play before it is stored.
struct NewPlay {
    entry: m3u8::Entry,
    recording: Option<i64>,
}

/// Imports the m3u8 file `bytes`, read from `file` (its path, for the name,
/// the date and relative paths), as one session: one operation.
pub fn import_m3u8(
    conn: &mut Connection,
    mounted: &Mounted,
    file: &str,
    bytes: &[u8],
) -> Result<HistoryOutcome, OpsError> {
    if bytes.len() > m3u8::MAX_BYTES {
        return Ok(refused(HistoryRefusal::FileTooBig));
    }
    let parsed = m3u8::parse(bytes);
    if parsed.entries.is_empty() {
        return Ok(HistoryOutcome::Refused {
            reason: HistoryRefusal::NoPlays,
            session_id: None,
            skipped_lines: parsed.skipped,
        });
    }
    // Split by hand, not with `Path`: the file is a Windows path wherever
    // the code runs.
    let cut = file.rfind(['/', '\\']);
    let base = cut.map(|i| &file[..i]).filter(|b| !b.is_empty());
    let file_name = cut.map_or(file, |i| &file[i + 1..]);
    let stem = match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file_name,
    }
    .trim();
    let played_on = match m3u8::date_in(stem) {
        Some(date) => date,
        None => conn.query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))?,
    };
    let name: String = if stem.is_empty() {
        played_on.clone()
    } else {
        stem.chars().take(MAX_NAME_CHARS).collect()
    };

    let index = matching::PathIndex::load(conn, mounted)?;
    let plays: Vec<NewPlay> = parsed
        .entries
        .into_iter()
        .map(|entry| NewPlay {
            recording: index.recording_for(&entry.path, base),
            entry,
        })
        .collect();
    if let Some(existing) = same_session(conn, &plays)? {
        return Ok(HistoryOutcome::Refused {
            reason: HistoryRefusal::AlreadyImported,
            session_id: Some(existing),
            skipped_lines: parsed.skipped,
        });
    }

    let matched = count(plays.iter().filter(|p| p.recording.is_some()).count());
    let total = count(plays.len());
    let recorded = ops::record(
        conn,
        IMPORT_OPERATION,
        &json!({ "name": name, "plays": total }),
        |rec| {
            let session = rec.insert(
                "session",
                &[
                    ("source", text("m3u8")),
                    ("name", text(&name)),
                    ("played_on", text(&played_on)),
                ],
            )?;
            for (position, play) in (0_i64..).zip(&plays) {
                rec.insert(
                    "play_event",
                    &[
                        ("session_id", Value::Integer(session)),
                        ("position", Value::Integer(position)),
                        (
                            "recording_id",
                            play.recording.map_or(Value::Null, Value::Integer),
                        ),
                        ("raw_path", text(&play.entry.path)),
                        ("raw_artist", optional(&play.entry.artist)),
                        ("raw_title", optional(&play.entry.title)),
                    ],
                )?;
            }
            Ok(session)
        },
    )?;
    Ok(HistoryOutcome::Done {
        operation_id: recorded.operation_id,
        session_id: recorded.value,
        plays: total,
        matched,
        skipped_lines: parsed.skipped,
    })
}

/// The session whose plays are `plays`, in order, if there is one.
fn same_session(conn: &Connection, plays: &[NewPlay]) -> rusqlite::Result<Option<i64>> {
    let wanted: Vec<(Option<i64>, String)> = plays
        .iter()
        .map(|p| (p.recording, play_key(&p.entry.path)))
        .collect();
    let mut stmt = conn.prepare(
        "SELECT s.id FROM session s
         WHERE (SELECT count(*) FROM play_event p WHERE p.session_id = s.id) = ?1
         ORDER BY s.id",
    )?;
    let candidates = stmt
        .query_map([wanted.len() as i64], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut plays_of = conn.prepare(
        "SELECT recording_id, raw_path FROM play_event WHERE session_id = ?1 ORDER BY position",
    )?;
    for session in candidates {
        let held = plays_of
            .query_map([session], |r| {
                Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let same = held
            .iter()
            .zip(&wanted)
            .all(|((rec, path), (want_rec, want_key))| {
                (rec.is_some() && rec == want_rec) || play_key(path) == *want_key
            });
        if same {
            return Ok(Some(session));
        }
    }
    Ok(None)
}

/// A play's path as the already-imported check compares it: `/` for `\`,
/// then the path key.
fn play_key(path: &str) -> String {
    path_key(&path.replace('\\', "/"))
}

fn session_exists(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    conn.query_row("SELECT 1 FROM session WHERE id = ?1", [id], |_| Ok(()))
        .optional()
        .map(|found| found.is_some())
}

/// Corrects the day a session was played: one operation, none when it
/// already has that date.
pub fn set_date(
    conn: &mut Connection,
    session: i64,
    played_on: &str,
) -> Result<HistoryOutcome, OpsError> {
    let played_on = played_on.trim();
    if !m3u8::real_day(played_on) {
        return Ok(refused(HistoryRefusal::BadDate));
    }
    if !session_exists(conn, session)? {
        return Ok(refused(HistoryRefusal::NoSuchSession));
    }
    let recorded = ops::record(
        conn,
        SET_DATE_OPERATION,
        &json!({ "sessionId": session, "playedOn": played_on }),
        |rec| rec.set("session", session, "played_on", played_on),
    )?;
    let (plays, matched) = counts(conn, session)?;
    Ok(HistoryOutcome::Done {
        operation_id: recorded.operation_id,
        session_id: session,
        plays,
        matched,
        skipped_lines: 0,
    })
}

fn counts(conn: &Connection, session: i64) -> rusqlite::Result<(u32, u32)> {
    conn.query_row(
        "SELECT count(*), count(recording_id) FROM play_event WHERE session_id = ?1",
        [session],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// Every session, the latest played first.
pub fn sessions(conn: &Connection) -> rusqlite::Result<Vec<SessionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.source, s.name, s.played_on, s.imported_at,
                count(p.id), count(p.recording_id)
         FROM session s LEFT JOIN play_event p ON p.session_id = s.id
         GROUP BY s.id
         ORDER BY s.played_on DESC, s.id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SessionSummary {
            id: r.get(0)?,
            source: r.get(1)?,
            name: r.get(2)?,
            played_on: r.get(3)?,
            imported_at: r.get(4)?,
            plays: r.get(5)?,
            matched: r.get(6)?,
        })
    })?;
    rows.collect()
}

/// The plays of a session, in order.
pub fn plays(conn: &Connection, session: i64) -> rusqlite::Result<PlaysReply> {
    let mut stmt = conn.prepare(
        "SELECT id, position, recording_id, raw_artist, raw_title, raw_path
         FROM play_event WHERE session_id = ?1 ORDER BY position",
    )?;
    let plays = stmt
        .query_map([session], |r| {
            Ok(Play {
                id: r.get(0)?,
                position: r.get(1)?,
                recording_id: r.get(2)?,
                artist: r.get(3)?,
                title: r.get(4)?,
                path: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(PlaysReply {
        plays,
        wishlist_offer: wishlist::enabled(conn)?,
    })
}

/// Every "A right before B" in the sessions, with how often, the most
/// played first; only those from `from` when given. A track played twice
/// in a row isn't a transition.
pub fn transitions(conn: &Connection, from: Option<i64>) -> rusqlite::Result<Vec<Transition>> {
    let mut stmt = conn.prepare(
        "SELECT a.recording_id, b.recording_id, count(*)
         FROM play_event a
         JOIN play_event b ON b.session_id = a.session_id AND b.position = a.position + 1
         WHERE a.recording_id IS NOT NULL AND b.recording_id IS NOT NULL
           AND a.recording_id <> b.recording_id
           AND (?1 IS NULL OR a.recording_id = ?1)
         GROUP BY a.recording_id, b.recording_id
         ORDER BY count(*) DESC, a.recording_id, b.recording_id",
    )?;
    let rows = stmt.query_map([from], |r| {
        Ok(Transition {
            from: r.get(0)?,
            to: r.get(1)?,
            count: r.get(2)?,
        })
    })?;
    rows.collect()
}

/// Adds a session's unmatched plays to the end of a wishlist, one
/// operation (the wishlist's own "add"). A play without both an artist and
/// a title can't be a wish; it comes back as skipped, by its place in the
/// set (from 1). Refused while wishlists are off.
pub fn add_unmatched_to_wishlist(
    conn: &mut Connection,
    session: i64,
    wishlist_id: i64,
) -> Result<WishlistOutcome, OpsError> {
    let mut tracks = Vec::new();
    let mut skipped = Vec::new();
    let mut skipped_total = 0_u32;
    for play in plays(conn, session)?.plays {
        if play.recording_id.is_some() {
            continue;
        }
        match (play.artist, play.title) {
            (Some(artist), Some(title)) => tracks.push(WishTrack {
                artist,
                title,
                version: None,
                isrc: None,
            }),
            (artist, _) => {
                skipped_total += 1;
                if skipped.len() < wishlist::parse::MAX_SKIPPED_SHOWN {
                    skipped.push(WishSkippedLine {
                        line: play.position + 1,
                        text: play.path.chars().take(200).collect(),
                        reason: if artist.is_none() {
                            WishSkipReason::MissingArtist
                        } else {
                            WishSkipReason::MissingTitle
                        },
                    });
                }
            }
        }
    }
    wishlist::add_tracks(conn, wishlist_id, tracks, skipped, skipped_total)
}

// ---- commands ---------------------------------------------------------------

/// Every session with its counts.
#[tauri::command]
#[specta::specta]
pub async fn history_sessions(reads: State<'_, ReadPool>) -> Result<Vec<SessionSummary>, IpcError> {
    Ok(reads.read(sessions)?)
}

/// The plays of a session, in order.
#[tauri::command]
#[specta::specta]
pub async fn history_plays(
    reads: State<'_, ReadPool>,
    session_id: i64,
) -> Result<PlaysReply, IpcError> {
    Ok(reads.read(move |c| plays(c, session_id))?)
}

/// Imports an m3u8 file at `path` (picked by the user in the file dialog)
/// as a session. The file is only read.
#[tauri::command(async)]
#[specta::specta]
pub fn history_import_m3u8(
    reads: State<'_, ReadPool>,
    writer: State<'_, Writer>,
    path: String,
) -> Result<HistoryOutcome, IpcError> {
    let bytes = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() > m3u8::MAX_BYTES as u64 => {
            return Ok(refused(HistoryRefusal::FileTooBig))
        }
        Ok(_) => match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(refused(HistoryRefusal::FileUnreadable)),
        },
        Err(_) => return Ok(refused(HistoryRefusal::FileUnreadable)),
    };
    // Where the drives are, asked off the writer.
    let known = reads.read(crate::relink::identities)?;
    let mounted = Mounted::ask(&known, &crate::scan::system_volumes());
    Ok(writer.call(move |c| Ok(import_m3u8(c, &mounted, &path, &bytes)))??)
}

/// Corrects the day a session was played.
#[tauri::command(async)]
#[specta::specta]
pub fn history_set_date(
    writer: State<'_, Writer>,
    session_id: i64,
    played_on: String,
) -> Result<HistoryOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(set_date(c, session_id, &played_on)))??)
}

/// Adds a session's unmatched plays to a wishlist.
#[tauri::command(async)]
#[specta::specta]
pub fn history_add_to_wishlist(
    writer: State<'_, Writer>,
    session_id: i64,
    wishlist_id: i64,
) -> Result<WishlistOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(add_unmatched_to_wishlist(c, session_id, wishlist_id)))??)
}
