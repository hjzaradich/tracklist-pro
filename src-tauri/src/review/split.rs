//! "These are the same track", "Not the same" and a split, on a possible
//! version mix-up and on "Unclear duplicates" (1bE-2; ROADMAP 1.4 "A decision
//! sticks", 1.12 "Keyboard"). Each is one operation of the log, so Undo takes
//! it back exactly and the item is in the queue again.
//!
//! # These are the same track (Enter, the one filled button)
//!
//! [`same_track`] remembers "same" between every two of the item's tracks
//! ([`crate::grouping::remember`], kept by what the files hold), and the files
//! of the tracks outside the Library move into the Library track, as changes
//! of the same operation ([`super::same::move_file`]). Each track that lost its
//! files is kept, with none, while the decision stands (`track_merge`, kind
//! `same`), so Undo has a track to move them back into. The job's own version
//! links between the tracks go first (they would hold the move back; they are
//! derived, and the grouping pass makes them again after an Undo). A version
//! link the user made or confirmed refuses the action
//! ([`Refused::LinkedVersions`]): versions never merge.
//!
//! **Two Library tracks are never merged here**: with two or more Library
//! tracks in the item, only the decision is remembered, and the grouping then
//! shows them as "Same track twice in your Library", whose Merge is the user's
//! ([`super::same`], 1bE-9).
//!
//! # Not the same (S on an item with two files)
//!
//! [`not_same`] remembers "apart" between the two files' tracks. Nothing moves.
//!
//! # A split (S with three or more files, on the selected row)
//!
//! The file of the selected row becomes a track of its own for good (owner
//! decisions, 2026-10-12): it moves into a new track, and "apart" is
//! remembered between it and every other track of the item, the track it left
//! included, so the grouping never merges it back. The **linked file of a
//! Library track can't be split off**: when it is the selected row, the other
//! files of its track leave instead, together as one new track, and "apart" is
//! remembered between the linked file's track and every other track of the
//! item (foreman's answer, 2026-10-16). A file that already has a track of its
//! own in the item moves nowhere: only the decisions are made.
//!
//! The new track is made the way the grouping job makes tracks, outside the
//! log ([`crate::ops::record_with_derived_first`], in the operation's own
//! transaction): it is derived state, and the log never names it as a row,
//! only as the value a moved file's `recording_id` was set to. So a track id
//! the grouping reuses can never be a row the log takes back.
//!
//! # Undo
//!
//! The moved files go back to the tracks they came from and the decisions are
//! taken back; a grouping pass over the item's files then puts the item back
//! in the queue ([`after_undo`]), and a track a split made, left with nothing,
//! is swept. When the grouping job ran in between: the decisions keep the
//! files where the action put them, so nothing it did stands in the way. A
//! file that changed track since is "changed since" (the log's own check).
//! And Undo is refused, with nothing changed ([`undo_refused`]), when the
//! track a split took files out of may not be that track any more (none of
//! the files that stayed holds what it held there, or the track by that id
//! was made at another time: it may have been swept and its id given to
//! another, ids being reused, so the files would go back into the wrong
//! one), or when a file the action moved is a Library track's file now (two
//! tracks would end up on one file). The track a split made isn't checked:
//! a file still in a track by that id goes back where the user wants it.
//!
//! # What is refused, changing nothing
//!
//! [`Refused::ItemGone`] when the item isn't the one the screen showed;
//! [`Refused::NoAudioKnown`] when a file to decide about isn't hashed yet;
//! [`Refused::SameAudio`] when the file split off holds the very same audio
//! as a file on the other side (files with the very same audio are always one
//! track, ROADMAP 1.4, so the split couldn't stick); and
//! [`Refused::LinkedVersions`] above.

use std::collections::BTreeSet;

use rusqlite::types::Value;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tauri::State;

use super::actions::{rank_again, Acted, Refused};
use super::compare::item_tracks;
use super::same::{ask_for_merge_follow_up, files_of, known_files, move_file, regroup};
use super::{ReviewItemId, ReviewKind};
use crate::db::Writer;
use crate::grouping::{remember, Verdict, CONTENT_KEY};
use crate::ipc::IpcError;
use crate::jobs::JobQueue;
use crate::ops::{self, OpsError, Recorder};

/// The operation kinds, as the operation log stores them.
pub const SAME_TRACK_OPERATION: &str = "same_track";
pub const NOT_SAME_OPERATION: &str = "not_same_track";
pub const SPLIT_OPERATION: &str = "split_off_file";

/// Whether items of `kind` take "these are the same track" (Enter) and S:
/// **the one place that says it**. A possible version mix-up and "Unclear
/// duplicates" for now; the owner is asked about "Clearly best file".
pub fn decides_tracks(kind: ReviewKind) -> bool {
    matches!(kind, ReviewKind::VersionDoubt | ReviewKind::HeldSet)
}

/// Whether `kind` is one of this module's operations.
pub fn is_operation(kind: &str) -> bool {
    [SAME_TRACK_OPERATION, NOT_SAME_OPERATION, SPLIT_OPERATION].contains(&kind)
}

// ---- reading -------------------------------------------------------------------

/// The item's tracks, if it is still the item the screen showed (`shown`,
/// its tracks then): else [`Refused::ItemGone`].
fn still_the_item(conn: &Connection, id: ReviewItemId, shown: &[i64]) -> Result<Vec<i64>, Refused> {
    if !decides_tracks(id.kind) {
        return Err(Refused::ItemGone);
    }
    let mut now = item_tracks(conn, id)?.ok_or(Refused::ItemGone)?;
    now.sort_unstable();
    now.dedup();
    let shown: BTreeSet<i64> = shown.iter().copied().collect();
    if now.len() < 2 || now.iter().copied().collect::<BTreeSet<_>>() != shown {
        return Err(Refused::ItemGone);
    }
    Ok(now)
}

fn track_of(conn: &Connection, file: i64) -> rusqlite::Result<i64> {
    conn.prepare_cached("SELECT recording_id FROM recording_file WHERE file_id = ?1")?
        .query_row([file], |r| r.get(0))
}

/// A Library track points at the file: it never moves.
fn pinned(conn: &Connection, file: i64) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM library_track
                        WHERE linked_file_id = ?1 OR source_file_id = ?1)",
    )?
    .query_row([file], |r| r.get(0))
}

/// The track is in the Library, or one of its files is a Library track's.
fn held_by_library(conn: &Connection, track: i64) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM library_track WHERE recording_id = ?1)
             OR EXISTS (SELECT 1 FROM recording_file rf JOIN library_track lt
                            ON rf.file_id IN (lt.linked_file_id, lt.source_file_id)
                        WHERE rf.recording_id = ?1)",
    )?
    .query_row([track], |r| r.get(0))
}

fn in_library(conn: &Connection, track: i64) -> rusqlite::Result<bool> {
    conn.prepare_cached("SELECT EXISTS (SELECT 1 FROM library_track WHERE recording_id = ?1)")?
        .query_row([track], |r| r.get(0))
}

/// The file's `audio_hash`, if it has one: files with the same one are
/// always one track.
fn audio_hash(conn: &Connection, file: i64) -> rusqlite::Result<Option<Vec<u8>>> {
    let hash: Option<Vec<u8>> = conn
        .prepare_cached("SELECT audio_hash FROM file WHERE id = ?1")?
        .query_row([file], |r| r.get(0))?;
    Ok(hash.filter(|h| !h.is_empty()))
}

/// The audio the file is named by in a decision is known.
fn audio_known(conn: &Connection, file: i64) -> rusqlite::Result<bool> {
    conn.prepare_cached(&format!(
        "SELECT length({CONTENT_KEY}) > 0 FROM file f WHERE f.id = ?1"
    ))?
    .query_row([file], |r| {
        Ok(r.get::<_, Option<bool>>(0)?.unwrap_or(false))
    })
}

/// Two of `tracks` are linked as versions by the user (made or confirmed):
/// the job's own links give way to "same", the user's never do.
fn linked_by_user(conn: &Connection, tracks: &[i64]) -> rusqlite::Result<bool> {
    let json = serde_json::to_string(tracks).expect("numbers always serialize");
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM version_link
                        WHERE (source = 'user' OR confirmed = 1)
                          AND recording_a IN (SELECT value FROM json_each(?1))
                          AND recording_b IN (SELECT value FROM json_each(?1)))",
    )?
    .query_row([&json], |r| r.get(0))
}

/// Deletes the job's own version links between two of `tracks`, never one
/// the user made or confirmed. Derived state: the grouping makes them from
/// its pairs, and makes them again after an Undo.
fn drop_job_links(conn: &Connection, tracks: &[i64]) -> rusqlite::Result<()> {
    let json = serde_json::to_string(tracks).expect("numbers always serialize");
    conn.prepare_cached(
        "DELETE FROM version_link
         WHERE source <> 'user' AND confirmed = 0
           AND recording_a IN (SELECT value FROM json_each(?1))
           AND recording_b IN (SELECT value FROM json_each(?1))",
    )?
    .execute([&json])?;
    Ok(())
}

/// Remembers `verdict` between the track of `a` and the track of `b`, as
/// changes of the operation `rec` records. Both files' audio is known (the
/// callers check), so `remember` has nothing to refuse.
fn decide(rec: &mut Recorder<'_>, a: i64, b: i64, verdict: Verdict) -> Result<(), OpsError> {
    let _ = remember(rec, a, b, verdict)?;
    Ok(())
}

// ---- these are the same track -----------------------------------------------------

/// "These are the same track" for the item `id`, whose tracks the screen
/// showed as `shown`. See the module docs.
pub fn same_track(
    conn: &mut Connection,
    id: ReviewItemId,
    shown: &[i64],
) -> Result<Acted, Refused> {
    let tracks = still_the_item(conn, id, shown)?;
    if linked_by_user(conn, &tracks)? {
        return Err(Refused::LinkedVersions);
    }
    let known = known_files(conn, &tracks)?;
    let mut library = Vec::new();
    for &track in &tracks {
        if held_by_library(conn, track)? {
            library.push(track);
        }
    }
    // The track the others join: the Library track. With two Library tracks
    // nothing moves (never merged here); with none, the oldest track.
    let into = match library.as_slice() {
        [one] => Some(*one),
        [] => tracks.first().copied(),
        _ => None,
    };
    let mut moving: Vec<(i64, Vec<i64>)> = Vec::new();
    if let Some(into) = into {
        for &track in tracks.iter().filter(|&&t| t != into) {
            moving.push((track, files_of(conn, &[track])?));
        }
    }
    let details = json!({
        "recordingIds": tracks,
        "tracks": tracks.len(),
        "into": into,
    });
    let moves = !moving.is_empty();
    let link_ends = tracks.clone();
    let recorded = ops::record_with_derived_first(
        conn,
        SAME_TRACK_OPERATION,
        &details,
        // The job's own links between them would hold the moves back.
        |c| match moves {
            true => drop_job_links(c, &link_ends),
            false => Ok(()),
        },
        |rec, ()| {
            // The decision first, while every track still holds its own files:
            // it covers every pair of audio between them.
            for (i, &a) in known.iter().enumerate() {
                for &b in &known[i + 1..] {
                    decide(rec, a, b, Verdict::Same)?;
                }
            }
            if let Some(into) = into {
                for (track, files) in moving {
                    for file in files {
                        move_file(rec, file, into)?;
                    }
                    rec.insert(
                        "track_merge",
                        &[
                            ("recording_id", Value::Integer(track)),
                            ("into_id", Value::Integer(into)),
                            ("kind", Value::Text("same".to_owned())),
                        ],
                    )?;
                }
            }
            Ok(())
        },
        // The moved files' rekordbox entries follow their track.
        crate::grouping::sync_rekordbox,
    )?;
    regroup(conn, &tracks);
    rank_again(conn, &tracks);
    Ok(Acted {
        operation_id: recorded.operation_id,
    })
}

// ---- not the same, and a split ----------------------------------------------------

/// What S did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotSame {
    pub operation_id: Option<i64>,
    /// The item is still in the queue: a split left the question between its
    /// other tracks open. The screen then stays on it (foreman's answer,
    /// 2026-10-16); otherwise it goes on to the next item.
    pub item_stays: bool,
}

/// S on the item `id`, whose tracks the screen showed as `shown`: with two
/// files, "not the same" between them; with three or more, the file `file`
/// (the selected row) is split off. See the module docs.
pub fn not_same(
    conn: &mut Connection,
    id: ReviewItemId,
    shown: &[i64],
    file: Option<i64>,
) -> Result<NotSame, Refused> {
    let tracks = still_the_item(conn, id, shown)?;
    let files = files_of(conn, &tracks)?;
    let acted = match files.len() {
        2 => apart(conn, &tracks, files[0], files[1])?,
        _ => {
            let file = file
                .filter(|f| files.contains(f))
                .ok_or(Refused::ItemGone)?;
            split(conn, &tracks, file)?
        }
    };
    Ok(NotSame {
        operation_id: acted.operation_id,
        item_stays: item_tracks(conn, id)?.is_some(),
    })
}

/// Whether a file of `side` holds the very same audio as a file of
/// `others`, where the two tracks can't be told apart by being in the
/// Library: such files are always one track, so "apart" couldn't stick.
fn same_audio(
    conn: &Connection,
    side: &[i64],
    side_in_library: bool,
    others: &[(i64, bool)],
) -> rusqlite::Result<bool> {
    for &a in side {
        let Some(mine) = audio_hash(conn, a)? else {
            continue;
        };
        for &(b, in_library) in others {
            if side_in_library && in_library {
                continue;
            }
            if audio_hash(conn, b)?.as_ref() == Some(&mine) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// "Not the same" between the two files `a` and `b` of an item, each in a
/// track of its own.
fn apart(conn: &mut Connection, tracks: &[i64], a: i64, b: i64) -> Result<Acted, Refused> {
    if !(audio_known(conn, a)? && audio_known(conn, b)?) {
        return Err(Refused::NoAudioKnown);
    }
    let (ta, tb) = (track_of(conn, a)?, track_of(conn, b)?);
    if same_audio(
        conn,
        &[a],
        in_library(conn, ta)?,
        &[(b, in_library(conn, tb)?)],
    )? {
        return Err(Refused::SameAudio);
    }
    let details = json!({ "recordingIds": tracks, "tracks": tracks.len() });
    let recorded = ops::record(conn, NOT_SAME_OPERATION, &details, |rec| {
        decide(rec, a, b, Verdict::Apart)
    })?;
    regroup(conn, tracks);
    Ok(Acted {
        operation_id: recorded.operation_id,
    })
}

/// The first of `files` whose audio is known, a Library track's file first.
fn known_among(conn: &Connection, files: &[i64]) -> rusqlite::Result<Option<i64>> {
    let mut found = None;
    for &file in files {
        if audio_known(conn, file)? {
            if pinned(conn, file)? {
                return Ok(Some(file));
            }
            found = found.or(Some(file));
        }
    }
    Ok(found)
}

/// Splits `file`, one of its files, off the item `tracks`.
fn split(conn: &mut Connection, tracks: &[i64], file: i64) -> Result<Acted, Refused> {
    let from = track_of(conn, file)?;
    let in_track = files_of(conn, &[from])?;
    let linked = pinned(conn, file)?;
    // What leaves the track: the file, or, for a Library track's linked
    // file, every other file of its track (that no Library track points at).
    let mut moving = Vec::new();
    if linked {
        for &other in in_track.iter().filter(|&&f| f != file) {
            if !pinned(conn, other)? {
                moving.push(other);
            }
        }
    } else if in_track.len() > 1 {
        moving.push(file);
    }
    let stayed: Vec<i64> = in_track
        .iter()
        .copied()
        .filter(|f| !moving.contains(f))
        .collect();
    // The side split off: the file, or the linked file's track as it will be.
    let side: Vec<i64> = match linked {
        true => stayed.clone(),
        false => vec![file],
    };
    let side_in_library = match linked || moving.is_empty() {
        true => in_library(conn, from)?,
        // A new track, outside the Library.
        false => false,
    };
    // Every other track of the item as it will be, by its files.
    let mut others: Vec<Vec<i64>> = Vec::new();
    match linked {
        true if !moving.is_empty() => others.push(moving.clone()),
        false if !moving.is_empty() => others.push(stayed.clone()),
        _ => {}
    }
    for &track in tracks.iter().filter(|&&t| t != from) {
        others.push(files_of(conn, &[track])?);
    }
    // What each side's decision is kept by: the file itself, and a file of
    // each other track whose audio is known.
    if !audio_known(conn, file)? {
        return Err(Refused::NoAudioKnown);
    }
    let mut other_known = Vec::new();
    for group in &others {
        other_known.push(known_among(conn, group)?.ok_or(Refused::NoAudioKnown)?);
    }
    // Files with the very same audio are always one track.
    let mut other_files: Vec<(i64, bool)> = Vec::new();
    for group in &others {
        for &f in group {
            let in_lib = !moving.contains(&f) && in_library(conn, track_of(conn, f)?)?;
            other_files.push((f, in_lib));
        }
    }
    if same_audio(conn, &side, side_in_library, &other_files)? {
        return Err(Refused::SameAudio);
    }

    // What tells the track the files left from any track that takes its id
    // later: when it was made, and the files that stayed with what they hold.
    let from_made: String = conn
        .prepare_cached("SELECT created_at FROM recording WHERE id = ?1")?
        .query_row([from], |r| r.get(0))?;
    let mut stayed_with = Vec::new();
    for &f in &stayed {
        let key: String = conn
            .prepare_cached(&format!(
                "SELECT hex(coalesce({CONTENT_KEY}, x'')) FROM file f WHERE f.id = ?1"
            ))?
            .query_row([f], |r| r.get(0))?;
        stayed_with.push(json!({ "file": f, "key": key }));
    }
    let mut all_tracks = tracks.to_vec();
    let details = json!({
        "file": file,
        "from": from,
        "fromMade": from_made,
        "moved": moving,
        "stayed": stayed_with,
        "recordingIds": tracks,
        "tracks": tracks.len(),
    });
    let new_track = !moving.is_empty();
    let to_move = moving.clone();
    let recorded = ops::record_with_derived_first(
        conn,
        SPLIT_OPERATION,
        &details,
        // The track the files leave for, made as the grouping makes tracks.
        |c| -> rusqlite::Result<Option<i64>> {
            if !new_track {
                return Ok(None);
            }
            c.prepare_cached("INSERT INTO recording DEFAULT VALUES")?
                .execute([])?;
            Ok(Some(c.last_insert_rowid()))
        },
        |rec, into| {
            if let Some(into) = into {
                for &moved in &to_move {
                    move_file(rec, moved, into)?;
                }
                rec.detail("into", into);
            }
            // "Apart", now that every file is where it ends up: it covers
            // every pair of audio between the two tracks.
            for &other in &other_known {
                decide(rec, file, other, Verdict::Apart)?;
            }
            Ok(into)
        },
        crate::grouping::sync_rekordbox,
    )?;
    if let Some(into) = recorded.value {
        all_tracks.push(into);
    }
    regroup(conn, &all_tracks);
    rank_again(conn, &all_tracks);
    Ok(Acted {
        operation_id: recorded.operation_id,
    })
}

// ---- undo -------------------------------------------------------------------------

/// Whether the Undo of `operation` must be refused (see the module docs):
/// a file it moved is a Library track's file now; or it split files out of
/// a track that may not be that track any more: none of the files that
/// stayed there is in it holding what it held then, or the track by that id
/// was made at another time. Track ids and file ids are reused once their
/// rows are gone, so an id alone proves nothing: the track may have been
/// swept and its id given to another. Asked by the
/// log before anything is unwound ([`crate::ops`]); `false` for any other
/// operation.
pub(crate) fn undo_refused(conn: &Connection, operation: i64) -> rusqlite::Result<bool> {
    conn.prepare_cached(&format!(
        "SELECT EXISTS (
             SELECT 1 FROM operation o
             WHERE o.id = ?1 AND o.kind IN ('{SAME_TRACK_OPERATION}', '{SPLIT_OPERATION}')
               AND (EXISTS (SELECT 1 FROM change c
                            JOIN library_track lt
                              ON c.entity_id IN (lt.linked_file_id, lt.source_file_id)
                            WHERE c.operation_id = o.id AND c.entity = 'recording_file'
                              AND c.field = 'recording_id')
                    OR (o.kind = '{SPLIT_OPERATION}'
                        AND json_array_length(o.details, '$.moved') > 0
                        AND (NOT EXISTS (
                                 SELECT 1 FROM recording r
                                 WHERE r.id = json_extract(o.details, '$.from')
                                   AND r.created_at = json_extract(o.details, '$.fromMade'))
                             OR NOT EXISTS (
                                 SELECT 1 FROM json_each(o.details, '$.stayed') s
                                 JOIN recording_file rf
                                   ON rf.file_id = json_extract(s.value, '$.file')
                                 JOIN file f ON f.id = rf.file_id
                                 WHERE rf.recording_id = json_extract(o.details, '$.from')
                                   AND hex(coalesce({CONTENT_KEY}, x''))
                                       = json_extract(s.value, '$.key'))))))"
    ))?
    .query_row([operation], |r| r.get(0))
}

/// After the Undo of one of this module's operations (`operation`, of
/// `kind`): the grouping pass over the item's files, so the item is in the
/// queue again as the undo returns, and a track a split made, left with
/// nothing, goes (the pass sweeps it). Nothing for any other operation.
pub(crate) fn after_undo(conn: &mut Connection, operation: i64, kind: &str) {
    if !is_operation(kind) {
        return;
    }
    let recordings: rusqlite::Result<Vec<i64>> = conn
        .prepare_cached(
            "SELECT value FROM operation o, json_each(o.details, '$.recordingIds')
             WHERE o.id = ?1",
        )
        .and_then(|mut stmt| {
            stmt.query_map([operation], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()
        });
    match recordings {
        Ok(recordings) => {
            regroup(conn, &recordings);
            rank_again(conn, &recordings);
        }
        Err(e) => eprintln!("review: couldn't read what the undone operation was about: {e}"),
    }
}

// ---- commands -----------------------------------------------------------------------

/// "These are the same track" for a possible version mix-up or "Unclear
/// duplicates": `recording_ids` are the tracks the screen showed.
#[tauri::command(async)]
#[specta::specta]
pub fn review_same_track(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    id: ReviewItemId,
    recording_ids: Vec<i64>,
) -> Result<Acted, IpcError> {
    let acted = writer.call(move |c| Ok(same_track(c, id, &recording_ids)))??;
    ask_for_merge_follow_up(&writer, &jobs);
    Ok(acted)
}

/// S: "not the same" (two files), or the file `file_id` split off (three or
/// more): `recording_ids` are the tracks the screen showed.
#[tauri::command(async)]
#[specta::specta]
pub fn review_not_same(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    id: ReviewItemId,
    recording_ids: Vec<i64>,
    file_id: Option<i64>,
) -> Result<NotSame, IpcError> {
    let acted = writer.call(move |c| Ok(not_same(c, id, &recording_ids, file_id)))??;
    ask_for_merge_follow_up(&writer, &jobs);
    Ok(acted)
}
