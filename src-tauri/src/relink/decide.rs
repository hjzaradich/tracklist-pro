//! What the user says about a probable match (1bE-6; ROADMAP 1.2, 1.12):
//! confirm it, say the file is not the entry's ("Not the same"), or withdraw
//! a confirmation. Each is one undoable operation ([`crate::ops`]) on the
//! `relink` table (migration 0028), which is the only thing relink carries
//! from one run to the next.
//!
//! - **Confirm** ([`confirm_match`]) stores a `same` row for the entry's
//!   Location key and the audio the file holds, with the file's `audio_hash`
//!   if that is current. It replaces an earlier confirmation of that
//!   Location (confirming again after the audio changed records the audio of
//!   now) and takes away a "not the same" said about that audio.
//! - **Not the same** ([`reject_match`]) stores a `not_same` row for the
//!   Location key and the audio. Relink then never matches that entry to a
//!   file holding that audio again in steps 2 to 5 ([`super::rules`]); it may
//!   propose another file, which is a new item. A confirmation of that very
//!   file or audio goes with it; every other decision about the Location
//!   stays.
//! - **Withdraw** ([`withdraw`]) deletes the confirmation of a Location: the
//!   entry is decided again from scratch by the next relink run. The Details
//!   panel offers it for a Library track whose entry was confirmed to its
//!   file (1cB-17; `send_values::sources`); it is also what Undo of a
//!   confirmation amounts to.
//!
//! **Kept by what the file holds** ([`FILE_KEY`]: its `audio_hash`, or its
//! `blake3` when it has none), as `review_decision` is: a decision survives
//! a move, a rename, a rescan and the file's music folder removed and added
//! again, because the file is a new row then with the same audio. A row
//! also points at the file the user looked at (`file_id`), while that row
//! exists: what a confirmation is applied to, and what tells that the
//! confirmed file holds other audio now (probable again until confirmed
//! again). The pointer never holds the file in place: removing its music
//! folder lets go of it, and a confirmation without its file is applied to
//! a file with its audio. "Not the same" needs known audio: a file that
//! hasn't been hashed yet can't be refused ([`Refused::NoAudioKnown`]).
//!
//! **One action, one transaction.** Relink decides every row again when it
//! runs, and a run is queued after every action here and after its Undo. So
//! that the Review queue is right before that run ends, each action also
//! writes what relink will make of it into the entry's own `rekordbox_track`
//! rows ([`apply_to_rows`]): confirmed → trusted, not the same → unmatched,
//! withdrawn → probable again. Those rows are derived state (a snapshot, not
//! in the operation log), written in the operation's own transaction
//! ([`ops::record_with_derived`]): the decision and the rows land together or
//! not at all. The relink run that follows decides them again from scratch.

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

use super::rules::confidence;
use super::{hash_version, HASH_CURRENT};
use crate::ops::{self, OpsError, Recorder};

/// The operation kinds, as the operation log stores them.
pub const CONFIRM_OPERATION: &str = "confirm_relink";
pub const REJECT_OPERATION: &str = "reject_relink";
pub const WITHDRAW_OPERATION: &str = "withdraw_relink";

/// Whether `kind` is one of the operations here: a relink run follows it,
/// and its Undo.
pub fn is_operation(kind: &str) -> bool {
    [CONFIRM_OPERATION, REJECT_OPERATION, WITHDRAW_OPERATION].contains(&kind)
}

/// What the file `f` holds, as SQL: grouping's name for it
/// ([`crate::grouping::CONTENT_KEY`]): its `audio_hash`, or its `blake3`
/// when it has none. What a decision is kept by (`relink.file_key`).
pub(crate) const FILE_KEY: &str = crate::grouping::CONTENT_KEY;

/// What a `relink` row says, as stored in `verdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The entry at this Location is this file: a confirmation.
    Same,
    /// The entry at this Location is not this audio.
    NotSame,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Same => "same",
            Verdict::NotSame => "not_same",
        }
    }
}

/// Why nothing was decided. Nothing was changed.
#[derive(Debug)]
pub enum Refused {
    /// The entry isn't waiting on that file any more (answered already, or
    /// a relink run matched it otherwise); or there is no confirmation to
    /// withdraw.
    ItemGone,
    /// The file hasn't been hashed yet, so there is no audio to say "not the
    /// same" about.
    NoAudioKnown,
    /// The database or the operation log failed.
    Ops(OpsError),
}

impl From<OpsError> for Refused {
    fn from(e: OpsError) -> Self {
        Refused::Ops(e)
    }
}

impl From<rusqlite::Error> for Refused {
    fn from(e: rusqlite::Error) -> Self {
        Refused::Ops(OpsError::from(e))
    }
}

/// A match as a `rekordbox_track` row holds it, or as a `relink` row
/// remembers it: the file, how it was found and how sure that was.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Proposal {
    pub file: i64,
    pub method: String,
    pub confidence: Option<f64>,
}

/// The probable match of the entry `entry` (a `rekordbox_track` row) to
/// `file`, with the entry's Location key. `None` if the entry isn't waiting
/// on that file.
fn waiting(
    conn: &Connection,
    entry: i64,
    file: i64,
) -> rusqlite::Result<Option<(String, Proposal)>> {
    conn.query_row(
        "SELECT location_key, relink_method, relink_confidence FROM rekordbox_track
         WHERE id = ?1 AND file_id = ?2 AND relink_probable = 1",
        [entry, file],
        |r| {
            Ok((
                r.get(0)?,
                Proposal {
                    file,
                    method: r.get(1)?,
                    confidence: r.get(2)?,
                },
            ))
        },
    )
    .optional()
}

/// What the file `file` holds ([`FILE_KEY`]), if it has been hashed.
fn file_key(conn: &Connection, file: i64) -> rusqlite::Result<Option<Vec<u8>>> {
    Ok(conn
        .query_row(
            &format!("SELECT {FILE_KEY} FROM file f WHERE f.id = ?1"),
            [file],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()?
        .flatten())
}

/// Stores what the user said about the entry at `key` and the file of
/// `about`, as changes of the operation `rec` records. The rows it replaces
/// go in the same operation, and no others: for a confirmation, the
/// Location's earlier confirmation and a "not the same" about this audio;
/// for a "not the same", whatever was said about this audio and a
/// confirmation of this very file. Every other decision about the Location
/// stays.
pub(crate) fn store(
    rec: &mut Recorder<'_>,
    key: &str,
    verdict: Verdict,
    about: &Proposal,
) -> Result<i64, OpsError> {
    let held: Option<Vec<u8>> = rec
        .read_rows(
            &format!("SELECT {FILE_KEY} FROM file f WHERE f.id = ?1"),
            [about.file],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        )?
        .pop()
        .flatten();
    let replaced: Vec<i64> = rec.read_rows(
        match verdict {
            // (`?3` is the file, which only a "not the same" asks about.)
            Verdict::Same => {
                "SELECT id FROM relink WHERE location_key = ?1 AND ?3 IS NOT NULL
                   AND (verdict = 'same' OR (verdict = 'not_same' AND file_key = ?2))
                 ORDER BY id"
            }
            Verdict::NotSame => {
                "SELECT id FROM relink WHERE location_key = ?1
                   AND (file_key = ?2 OR (verdict = 'same' AND file_id = ?3))
                 ORDER BY id"
            }
        },
        params![key, held, about.file],
        |r| r.get(0),
    )?;
    for id in replaced {
        rec.delete("relink", id)?;
    }
    // The audio the file holds now, only if that is known for sure.
    let audio: Option<Vec<u8>> = rec
        .read_rows(
            &format!(
                "SELECT f.audio_hash FROM file f
                 WHERE f.id = ?2 AND length(f.audio_hash) > 0 AND {HASH_CURRENT}"
            ),
            params![hash_version(), about.file],
            |r| r.get(0),
        )?
        .pop();
    rec.insert(
        "relink",
        &[
            ("location_key", Value::Text(key.to_owned())),
            ("file_key", held.map_or(Value::Null, Value::Blob)),
            ("file_id", Value::Integer(about.file)),
            ("verdict", Value::Text(verdict.as_str().to_owned())),
            ("method", Value::Text(about.method.clone())),
            (
                "confidence",
                about.confidence.map_or(Value::Null, Value::Real),
            ),
            ("audio_hash", audio.map_or(Value::Null, Value::Blob)),
        ],
    )
}

/// Records one decision about the entry at `key` and its rows of the
/// snapshot, in one transaction.
fn decide(
    conn: &mut Connection,
    kind: &str,
    key: &str,
    verdict: Verdict,
    about: &Proposal,
) -> Result<Option<i64>, Refused> {
    let recorded = ops::record_with_derived(
        conn,
        kind,
        &json!({}),
        |rec| store(rec, key, verdict, about),
        |conn| apply_to_rows(conn, key, None),
    )?;
    Ok(recorded.operation_id)
}

/// Confirms that the rekordbox entry `entry` is `file`, the file its
/// probable match names. One operation; the entry's rows are trusted in the
/// same transaction, and the next relink run applies the confirmation like
/// any other (step 0). Refused if the entry isn't waiting on that file.
pub fn confirm_match(conn: &mut Connection, entry: i64, file: i64) -> Result<Option<i64>, Refused> {
    let Some((key, proposal)) = waiting(conn, entry, file)? else {
        return Err(Refused::ItemGone);
    };
    decide(conn, CONFIRM_OPERATION, &key, Verdict::Same, &proposal)
}

/// Says the rekordbox entry `entry` is not `file`, the file its probable
/// match names. One operation; the entry's rows lose the match in the same
/// transaction, and relink never matches the entry to that audio again (it
/// may propose another file). Refused if the entry isn't waiting on that
/// file, or the file hasn't been hashed yet.
pub fn reject_match(conn: &mut Connection, entry: i64, file: i64) -> Result<Option<i64>, Refused> {
    let Some((key, proposal)) = waiting(conn, entry, file)? else {
        return Err(Refused::ItemGone);
    };
    if file_key(conn, file)?.is_none() {
        return Err(Refused::NoAudioKnown);
    }
    decide(conn, REJECT_OPERATION, &key, Verdict::NotSame, &proposal)
}

/// Withdraws the confirmation of the Location of the rekordbox entry
/// `entry`. One operation; the entry's rows are probable again in the same
/// transaction, and the next relink run decides them from scratch, as if the
/// user had never confirmed. Refused if that Location has no confirmation.
pub fn withdraw(conn: &mut Connection, entry: i64) -> Result<Option<i64>, Refused> {
    type Confirmation = (String, i64, Option<i64>, String, Option<f64>);
    let confirmed: Option<Confirmation> = conn
        .query_row(
            "SELECT r.location_key, r.id, r.file_id, r.method, r.confidence
             FROM rekordbox_track t
             JOIN relink r ON r.location_key = t.location_key AND r.verdict = 'same'
             WHERE t.id = ?1",
            [entry],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((key, row, file, method, confidence)) = confirmed else {
        return Err(Refused::ItemGone);
    };
    let was = file.map(|file| Proposal {
        file,
        method,
        confidence,
    });
    let recorded = ops::record_with_derived(
        conn,
        WITHDRAW_OPERATION,
        &json!({}),
        |rec| rec.delete("relink", row),
        |conn| apply_to_rows(conn, &key, was.as_ref()),
    )?;
    Ok(recorded.operation_id)
}

/// Writes into the `rekordbox_track` rows at `key` what the `relink` table
/// says about them now, as the next relink run will (see the module docs).
/// It reads only the table, so it is right after an action and after its
/// Undo alike.
///
/// - **A confirmation whose file is on disk** (the file it points at, or,
///   when that row is gone, a file holding the audio it was made for): the
///   rows are matched to it as step 0 matches them, trusted, or probable if
///   the file is known to hold other audio than the user confirmed.
/// - **Otherwise, row by row:** a row matched to a file holding audio the
///   user said is not the entry's loses the match. A row trusted on `was`'s
///   file, with no confirmation left, was confirmed and isn't any more: it
///   is probable again, as it was found. A row with no match takes `was`
///   back as a probable match (a "not the same" that was undone), if that
///   file is on disk, nothing was said against its audio and no other entry
///   holds it.
///
/// `was` is the match an action or an undone operation was about, when it
/// took a decision away.
pub(crate) fn apply_to_rows(
    conn: &Connection,
    key: &str,
    was: Option<&Proposal>,
) -> rusqlite::Result<()> {
    // The file's audio now, if that is known for sure, and its track.
    let audio_now =
        format!("CASE WHEN length(f.audio_hash) > 0 AND {HASH_CURRENT} THEN f.audio_hash END");
    let track_of = "(SELECT recording_id FROM recording_file WHERE file_id = f.id)";
    type Confirmed = (i64, String, Option<Vec<u8>>, Option<Vec<u8>>, Option<i64>);
    let confirmed: Option<Confirmed> = conn
        .query_row(
            &format!(
                "SELECT f.id, r.method, r.audio_hash, {audio_now}, {track_of}
                 FROM relink r JOIN file f
                   ON f.id = r.file_id
                   OR (r.file_id IS NULL AND (f.audio_hash = r.file_key OR f.blake3 = r.file_key))
                 WHERE r.location_key = ?2 AND r.verdict = 'same' AND f.present = 1
                 ORDER BY f.id LIMIT 1"
            ),
            params![hash_version(), key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let mut set = conn.prepare_cached(
        "UPDATE rekordbox_track
         SET file_id = ?2, relink_method = ?3, relink_confidence = ?4, relink_probable = ?5,
             recording_id = ?6, relink_audio_hash = ?7
         WHERE id = ?1",
    )?;
    let rows: Vec<(i64, Option<i64>, bool)> = conn
        .prepare_cached(
            "SELECT id, file_id, relink_probable FROM rekordbox_track
             WHERE location_key = ?1 ORDER BY id",
        )?
        .query_map([key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;

    if let Some((file, method, confirmed_audio, now, recording)) = confirmed {
        let changed = matches!((&confirmed_audio, &now), (Some(then), Some(now)) if then != now);
        let sure = if changed {
            confidence::CONFIRMED_AUDIO_CHANGED
        } else {
            confidence::CONFIRMED
        };
        let evidence = confirmed_audio.or(now);
        for (row, _, _) in rows {
            set.execute(params![
                row, file, method, sure, changed, recording, evidence
            ])?;
        }
        return Ok(());
    }

    // Whether the user said the entry is not the audio `file` holds.
    let mut rejected = conn.prepare_cached(&format!(
        "SELECT EXISTS (SELECT 1 FROM relink r, file f
                        WHERE f.id = ?2 AND r.location_key = ?1 AND r.verdict = 'not_same'
                          AND r.file_key = {FILE_KEY})"
    ))?;
    let mut is_rejected = |file: i64| -> rusqlite::Result<bool> {
        rejected.query_row(params![key, file], |r| r.get(0))
    };
    let mut free = conn.prepare_cached(&format!(
        "SELECT {audio_now}, {track_of} FROM file f
         WHERE f.id = ?2 AND f.present = 1
           AND NOT EXISTS (SELECT 1 FROM rekordbox_track o
                           WHERE o.file_id = f.id AND o.location_key <> ?3)"
    ))?;
    for (row, file, probable) in rows {
        match (file, was) {
            (Some(file), _) if is_rejected(file)? => {
                let none: Option<i64> = None;
                set.execute(params![row, none, none, none, false, none, none])?;
            }
            // Confirmed until now: found again, to be confirmed again.
            (Some(file), Some(was)) if !probable && was.file == file => {
                conn.execute(
                    "UPDATE rekordbox_track
                     SET relink_method = ?2, relink_confidence = ?3, relink_probable = 1
                     WHERE id = ?1",
                    params![
                        row,
                        was.method,
                        was.confidence
                            .unwrap_or(confidence::CONFIRMED_AUDIO_CHANGED)
                    ],
                )?;
            }
            (None, Some(was)) if !is_rejected(was.file)? => {
                type Free = (Option<Vec<u8>>, Option<i64>);
                let found: Option<Free> = free
                    .query_row(params![hash_version(), was.file, key], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .optional()?;
                if let Some((audio, recording)) = found {
                    set.execute(params![
                        row,
                        was.file,
                        was.method,
                        was.confidence,
                        true,
                        recording,
                        audio
                    ])?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// After an operation on `relink` was undone: the rows of the snapshot at
/// the Locations it was about follow what the table says again
/// ([`apply_to_rows`]), so the Review queue is right as the undo returns.
/// The match the undone operation stored (the confirmation or the "not the
/// same" it inserted) is what its rows go back to, while the file it pointed
/// at is still there. Returns whether the operation was about `relink` at
/// all: a relink run is then due.
pub fn after_undo(conn: &Connection, operation: i64) -> rusqlite::Result<bool> {
    // Each `relink` row the operation inserted or deleted, by its fields.
    let mut rows: HashMap<i64, (bool, HashMap<String, Option<String>>)> = HashMap::new();
    {
        let mut stmt = conn.prepare_cached(
            "SELECT entity_id, action, field, before, after FROM change
             WHERE operation_id = ?1 AND entity = 'relink' ORDER BY id",
        )?;
        let changes = stmt.query_map([operation], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        for change in changes {
            let (id, action, field, before, after) = change?;
            let inserted = action == "insert";
            let row = rows.entry(id).or_insert_with(|| (inserted, HashMap::new()));
            row.1.insert(field, if inserted { after } else { before });
        }
    }
    if rows.is_empty() {
        return Ok(false);
    }
    // By Location: the match the operation stored, if it stored one.
    let mut by_key: HashMap<String, Option<Proposal>> = HashMap::new();
    for (inserted, fields) in rows.into_values() {
        let text = |name: &str| fields.get(name).cloned().flatten();
        let Some(key) = text("location_key") else {
            continue;
        };
        let stored = by_key.entry(key).or_default();
        if !inserted {
            continue;
        }
        if let (Some(file), Some(method)) =
            (text("file_id").and_then(|f| f.parse().ok()), text("method"))
        {
            *stored = Some(Proposal {
                file,
                method,
                confidence: text("confidence").and_then(|c| c.parse().ok()),
            });
        }
    }
    for (key, was) in &by_key {
        apply_to_rows(conn, key, was.as_ref())?;
    }
    Ok(true)
}
