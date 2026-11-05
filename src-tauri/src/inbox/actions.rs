//! What the user does with an Inbox file (ROADMAP 2.1 "Actions"): **Add to
//! Library**, **Not now**, **Hide**, and **Clear** for a file whose track is
//! in the Library already. Each call is one operation ([`crate::ops`]), for
//! one item or many: one Undo takes all of it back, the item's status
//! included, so the item is back in the Inbox as it was.
//!
//! # Refused before anything is written
//!
//! Every item is checked first, from the data (does its track have a
//! Library track, is its file on disk), never from a label. If one item
//! can't be acted on, nothing is written and the answer names that item
//! and why ([`InboxRefusal`]): a call is all or nothing, never half done.
//! A call that would change nothing records no operation.
//!
//! # The actions
//!
//! - [`add`]: the item's own file becomes the linked file of a new Library
//!   track, where it is (managed copies come with 2.6). Only for a track
//!   with no Library track; a low-bitrate file asks first, as the Library's
//!   own add does (`library::warning`). Status `added`.
//! - [`not_now`]: status `not_now`. The file stays in All music.
//! - [`hide`]: the file's `hidden_reason` becomes `user`, and a choice the
//!   user made of it ("Use this file instead") ends in the same operation
//!   (ROADMAP §2, `file_choice`). Status `hidden`. "Show again" in the
//!   Hidden filter brings it back (`hidden::show_again`).
//! - [`clear`]: for a file whose track is in the Library already, the
//!   reason is `inbox_cleared` ("Put back" is `hidden::show_again` too).
//!   Status `hidden`.
//!
//! No music file is opened or written.

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tauri::State;

use crate::db::{DbError, Writer};
use crate::ipc::IpcError;
use crate::jobs::{JobId, NewJob};
use crate::library::{self, warnings_for, AddWarning};
use crate::ops::{self, OpsError, Recorder};
use crate::scan::chain::Reach;

/// The operation kinds, as the operation log stores them.
pub const ADD_OPERATION: &str = "inbox_add";
pub const NOT_NOW_OPERATION: &str = "inbox_not_now";
pub const HIDE_OPERATION: &str = "inbox_hide";
pub const CLEAR_OPERATION: &str = "inbox_clear";

/// Why an item can't be acted on. Nothing was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum InboxRefusal {
    /// There is no such item (its folder was removed).
    ItemGone,
    /// It was added, hidden or cleared already.
    AlreadyDecided,
    /// Its file isn't in a track yet: it is still being identified.
    NotIdentified,
    /// Its track is in the Library already (it has its own actions).
    HasLibraryTrack,
    /// Clear is for a file whose track is in the Library, and this one's
    /// isn't.
    NoLibraryTrack,
    /// The file isn't on disk.
    FileMissing,
    /// rekordbox has this track as another file: linked to this one it
    /// would arrive as a second entry, without the first one's cues.
    InRekordboxAsAnotherFile,
    /// The file is the probable match of a rekordbox entry, and nothing
    /// can confirm it yet (`library::match_not_confirmed`).
    MatchNotConfirmed,
    /// Another item of this add is a file of the same track.
    SameTrackTwice,
}

/// What an Inbox action came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum InboxOutcome {
    /// Done, as one operation (`None` when nothing changed).
    #[serde(rename_all = "camelCase")]
    Done {
        operation_id: Option<i64>,
        /// The items it changed.
        items: Vec<i64>,
    },
    /// Nothing was written: this item can't be acted on.
    #[serde(rename_all = "camelCase")]
    Refused { item_id: i64, reason: InboxRefusal },
    /// Nothing was written: adding this item's file needs the user's
    /// go-ahead first. Ask again with it confirmed.
    #[serde(rename_all = "camelCase")]
    NeedsConfirmation { item_id: i64, warning: AddWarning },
}

impl InboxOutcome {
    fn refused(item_id: i64, reason: InboxRefusal) -> InboxOutcome {
        InboxOutcome::Refused { item_id, reason }
    }
}

/// What the checks read of one item.
#[derive(Debug, Clone)]
struct Item {
    id: i64,
    file: i64,
    status: String,
    present: bool,
    hidden: bool,
    recording: Option<i64>,
    in_library: bool,
}

impl Item {
    /// New, or put off with Not now: still to decide.
    fn open(&self) -> bool {
        self.status == "new" || self.status == "not_now"
    }
}

/// The items `ids`, in that order; `Err` names the first that is gone.
fn load(conn: &Connection, ids: &[i64]) -> rusqlite::Result<Result<Vec<Item>, i64>> {
    let wanted = serde_json::to_string(ids).expect("numbers always serialize");
    let mut stmt = conn.prepare_cached(
        "SELECT i.id, i.file_id, i.status, f.present, f.hidden_reason IS NOT NULL,
                rf.recording_id,
                EXISTS (SELECT 1 FROM library_track lt WHERE lt.recording_id = rf.recording_id)
         FROM inbox_item i
         JOIN file f ON f.id = i.file_id
         LEFT JOIN recording_file rf ON rf.file_id = f.id
         WHERE i.id IN (SELECT value FROM json_each(?1))",
    )?;
    let found: HashMap<i64, Item> = stmt
        .query_map([&wanted], |r| {
            Ok(Item {
                id: r.get(0)?,
                file: r.get(1)?,
                status: r.get(2)?,
                present: r.get(3)?,
                hidden: r.get(4)?,
                recording: r.get(5)?,
                in_library: r.get(6)?,
            })
        })?
        .map(|item| item.map(|item| (item.id, item)))
        .collect::<rusqlite::Result<_>>()?;
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            continue;
        }
        match found.get(id) {
            Some(item) => items.push(item.clone()),
            None => return Ok(Err(*id)),
        }
    }
    Ok(Ok(items))
}

/// Sets an item's status, and when it was decided, as changes of `rec`.
fn decide(rec: &mut Recorder<'_>, item: i64, status: &str) -> Result<(), OpsError> {
    let now: String = rec
        .read_rows("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
            r.get(0)
        })?
        .pop()
        .unwrap_or_default();
    // The status first: a `new` item has no `decided_at` (a CHECK), and
    // Undo takes the changes back in the opposite order.
    rec.set("inbox_item", item, "status", status)?;
    rec.set("inbox_item", item, "decided_at", now)?;
    Ok(())
}

/// Puts an Inbox item whose file was hidden back to `new`, as changes of
/// `rec`: "Show again" and "Put back" (`hidden::show_again`). Nothing for a
/// file with no item, or an item that isn't `hidden`.
pub(crate) fn put_back(rec: &mut Recorder<'_>, file: i64) -> Result<(), OpsError> {
    let item: Option<i64> = rec
        .read_rows(
            "SELECT id FROM inbox_item WHERE file_id = ?1 AND status = 'hidden'",
            [file],
            |r| r.get(0),
        )?
        .pop();
    if let Some(item) = item {
        // `decided_at` first: a `new` item has none (a CHECK).
        rec.set("inbox_item", item, "decided_at", Option::<String>::None)?;
        rec.set("inbox_item", item, "status", "new")?;
    }
    Ok(())
}

/// Whether rekordbox matches a file of `recording` other than `file`
/// (trusted or probable).
fn rekordbox_has_another_file(
    conn: &Connection,
    recording: i64,
    file: i64,
) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM rekordbox_track rt
                        JOIN recording_file rf ON rf.file_id = rt.file_id
                        WHERE rf.recording_id = ?1 AND rt.file_id <> ?2)",
    )?
    .query_row([recording, file], |r| r.get(0))
}

/// Why `item` can't be added on its own, if it can't.
fn add_refusal(conn: &Connection, item: &Item) -> rusqlite::Result<Option<InboxRefusal>> {
    if !item.open() {
        return Ok(Some(InboxRefusal::AlreadyDecided));
    }
    let Some(recording) = item.recording else {
        return Ok(Some(InboxRefusal::NotIdentified));
    };
    Ok(if item.in_library {
        Some(InboxRefusal::HasLibraryTrack)
    } else if !item.present {
        Some(InboxRefusal::FileMissing)
    } else if rekordbox_has_another_file(conn, recording, item.file)? {
        Some(InboxRefusal::InRekordboxAsAnotherFile)
    } else if library::match_not_confirmed(conn, recording)? {
        Some(InboxRefusal::MatchNotConfirmed)
    } else {
        None
    })
}

/// Adds the files of `items` to the Library, each as a new linked Library
/// track of its own track, as one operation. A file with the `low bitrate`
/// finding is added only if `confirm_low_bitrate` says the user agreed.
pub fn add(
    conn: &mut Connection,
    items: &[i64],
    confirm_low_bitrate: bool,
) -> Result<InboxOutcome, OpsError> {
    let items = match load(conn, items)? {
        Ok(items) => items,
        Err(gone) => return Ok(InboxOutcome::refused(gone, InboxRefusal::ItemGone)),
    };
    let mut tracks = HashSet::new();
    for item in &items {
        if let Some(reason) = add_refusal(conn, item)? {
            return Ok(InboxOutcome::refused(item.id, reason));
        }
        if !tracks.insert(item.recording) {
            return Ok(InboxOutcome::refused(item.id, InboxRefusal::SameTrackTwice));
        }
    }
    if !confirm_low_bitrate {
        let files: Vec<i64> = items.iter().map(|i| i.file).collect();
        let warnings = warnings_for(conn, &files)?;
        if let Some(item) = items.iter().find(|i| warnings.contains_key(&i.file)) {
            return Ok(InboxOutcome::NeedsConfirmation {
                item_id: item.id,
                warning: warnings[&item.file],
            });
        }
    }
    let ids: Vec<i64> = items.iter().map(|i| i.id).collect();
    let recorded = ops::record(
        conn,
        ADD_OPERATION,
        &json!({ "items": ids, "tracks": items.len() }),
        |rec| {
            for item in &items {
                let recording = item.recording.expect("checked above");
                library::insert_linked(rec, recording, item.file)?;
                decide(rec, item.id, "added")?;
            }
            Ok(())
        },
    )?;
    Ok(done(recorded.operation_id, ids))
}

/// The items "Add all" adds for `batch`: its new items whose track has no
/// Library track (those have their own actions), one per track (the
/// track's best file if it is one of them, else the first that arrived).
/// Anything else that stands in the way is [`add`]'s to refuse.
pub fn batch_to_add(conn: &Connection, batch: i64) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT i.id, rf.recording_id FROM inbox_item i
         LEFT JOIN recording_file rf ON rf.file_id = i.file_id
         WHERE i.batch_id = ?1 AND i.status = 'new'
           AND NOT EXISTS (SELECT 1 FROM library_track lt
                           WHERE lt.recording_id = rf.recording_id)
         ORDER BY rf.role = 'best' DESC, i.id",
    )?;
    let rows = stmt.query_map([batch], |r| Ok((r.get::<_, i64>(0)?, r.get(1)?)))?;
    let mut tracks: HashSet<i64> = HashSet::new();
    let mut items = Vec::new();
    for row in rows {
        let (item, recording): (i64, Option<i64>) = row?;
        if recording.is_none_or(|r| tracks.insert(r)) {
            items.push(item);
        }
    }
    items.sort_unstable();
    Ok(items)
}

fn done(operation_id: Option<i64>, items: Vec<i64>) -> InboxOutcome {
    let items = if operation_id.is_some() {
        items
    } else {
        Vec::new()
    };
    InboxOutcome::Done {
        operation_id,
        items,
    }
}

/// Not now: the items stay in All music, decided later. An item that is
/// `not_now` already is left as it is.
pub fn not_now(conn: &mut Connection, items: &[i64]) -> Result<InboxOutcome, OpsError> {
    let items = match load(conn, items)? {
        Ok(items) => items,
        Err(gone) => return Ok(InboxOutcome::refused(gone, InboxRefusal::ItemGone)),
    };
    if let Some(item) = items.iter().find(|i| !i.open()) {
        return Ok(InboxOutcome::refused(item.id, InboxRefusal::AlreadyDecided));
    }
    let ids: Vec<i64> = items
        .iter()
        .filter(|i| i.status == "new")
        .map(|i| i.id)
        .collect();
    let recorded = ops::record(conn, NOT_NOW_OPERATION, &json!({ "items": ids }), |rec| {
        for id in &ids {
            decide(rec, *id, "not_now")?;
        }
        Ok(())
    })?;
    Ok(done(recorded.operation_id, ids))
}

/// Hide: the files' `hidden_reason` becomes `user`.
pub fn hide(conn: &mut Connection, items: &[i64]) -> Result<InboxOutcome, OpsError> {
    hide_as(conn, items, HIDE_OPERATION, "user", false)
}

/// Clear: for files whose track is in the Library already, the
/// `hidden_reason` becomes `inbox_cleared`.
pub fn clear(conn: &mut Connection, items: &[i64]) -> Result<InboxOutcome, OpsError> {
    hide_as(conn, items, CLEAR_OPERATION, "inbox_cleared", true)
}

fn hide_as(
    conn: &mut Connection,
    items: &[i64],
    kind: &str,
    reason: &'static str,
    in_library_only: bool,
) -> Result<InboxOutcome, OpsError> {
    let items = match load(conn, items)? {
        Ok(items) => items,
        Err(gone) => return Ok(InboxOutcome::refused(gone, InboxRefusal::ItemGone)),
    };
    for item in &items {
        let refusal = if !item.open() || item.hidden {
            Some(InboxRefusal::AlreadyDecided)
        } else if in_library_only && item.recording.is_none() {
            Some(InboxRefusal::NotIdentified)
        } else if in_library_only && !item.in_library {
            Some(InboxRefusal::NoLibraryTrack)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            return Ok(InboxOutcome::refused(item.id, refusal));
        }
    }
    let ids: Vec<i64> = items.iter().map(|i| i.id).collect();
    let recorded = ops::record(conn, kind, &json!({ "items": ids }), |rec| {
        let mut ended = HashSet::new();
        for item in &items {
            rec.set("file", item.file, "hidden_reason", reason)?;
            for choice in choices_of(rec, item.file)? {
                if ended.insert(choice) {
                    rec.delete("file_choice", choice)?;
                }
            }
            decide(rec, item.id, "hidden")?;
        }
        Ok(())
    })?;
    // An ended choice made its track due (a trigger): ranked again at once.
    // The operation has committed: a failure is left to the roles job.
    let recordings: Vec<i64> = items.iter().filter_map(|i| i.recording).collect();
    if recorded.operation_id.is_some() {
        if let Err(e) = crate::file_roles::settle_tracks(conn, &recordings) {
            eprintln!("inbox: couldn't rank the tracks again: {e}");
        }
    }
    Ok(done(recorded.operation_id, ids))
}

/// The `file_choice` rows that choose `file` ("Use this file instead",
/// `file_roles::choice` says what a choice covers): those of its audio
/// that kept its bytes, or kept bytes no other file of its track on disk
/// has (they cover every file of that audio).
fn choices_of(rec: &Recorder<'_>, file: i64) -> Result<Vec<i64>, OpsError> {
    rec.read_rows(
        "SELECT c.id FROM file_choice c JOIN file f ON f.id = ?1
         WHERE c.content_key = coalesce(nullif(f.audio_hash, x''), f.blake3)
           AND (c.blake3 = f.blake3
                OR NOT EXISTS (
                    SELECT 1 FROM recording_file mine
                    JOIN recording_file orf ON orf.recording_id = mine.recording_id
                    JOIN file o ON o.id = orf.file_id
                    WHERE mine.file_id = f.id AND o.id <> f.id AND o.present = 1
                      AND length(c.blake3) > 0 AND o.blake3 = c.blake3
                      AND coalesce(nullif(o.audio_hash, x''), o.blake3) = c.content_key))
         ORDER BY c.id",
        [file],
        |r| r.get(0),
    )
}

/// The background work an add asks for, as the Library's own add does
/// (`library::promote_track`): the files' roles, and their waveforms.
/// A failure to ask is not the add's: the next job to end asks again.
pub(crate) fn ask_after_add<E: From<DbError> + Debug>(
    writer: &Writer,
    reach: Option<&Reach>,
    mut enqueue: impl FnMut(NewJob) -> Result<JobId, E>,
) {
    if let Err(e) = crate::file_roles::request(writer, &mut enqueue) {
        eprintln!("inbox: couldn't ask for the roles: {e:?}");
    }
    crate::waveform::ask_after_library_change_with(writer, reach, enqueue);
}

/// Whether `outcome` added anything.
fn added(outcome: &InboxOutcome) -> bool {
    matches!(
        outcome,
        InboxOutcome::Done {
            operation_id: Some(_),
            ..
        }
    )
}

fn add_and_follow_up(
    writer: &Writer,
    jobs: &crate::jobs::JobQueue,
    outcome: InboxOutcome,
) -> Result<InboxOutcome, IpcError> {
    if added(&outcome) {
        library::settle_now(writer, jobs);
        let reach = crate::scan::chain::reach_by(crate::scan::system_volumes);
        ask_after_add(writer, Some(&reach), |j| jobs.enqueue(j));
    }
    Ok(outcome)
}

/// Add to Library, for Inbox items.
#[tauri::command]
#[specta::specta]
pub async fn inbox_add(
    writer: State<'_, Writer>,
    jobs: State<'_, crate::jobs::JobQueue>,
    item_ids: Vec<i64>,
    confirm_low_bitrate: bool,
) -> Result<InboxOutcome, IpcError> {
    let outcome = writer.call(move |c| Ok(add(c, &item_ids, confirm_low_bitrate)))??;
    add_and_follow_up(&writer, &jobs, outcome)
}

/// "Add all" for an Inbox batch ([`batch_to_add`]).
#[tauri::command]
#[specta::specta]
pub async fn inbox_add_batch(
    writer: State<'_, Writer>,
    jobs: State<'_, crate::jobs::JobQueue>,
    batch_id: i64,
    confirm_low_bitrate: bool,
) -> Result<InboxOutcome, IpcError> {
    let outcome = writer.call(move |c| {
        let items = batch_to_add(c, batch_id)?;
        Ok(add(c, &items, confirm_low_bitrate))
    })??;
    add_and_follow_up(&writer, &jobs, outcome)
}

/// Not now, for Inbox items.
#[tauri::command(async)]
#[specta::specta]
pub fn inbox_not_now(
    writer: State<'_, Writer>,
    item_ids: Vec<i64>,
) -> Result<InboxOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(not_now(c, &item_ids)))??)
}

/// Hide, for Inbox items.
#[tauri::command(async)]
#[specta::specta]
pub fn inbox_hide(writer: State<'_, Writer>, item_ids: Vec<i64>) -> Result<InboxOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(hide(c, &item_ids)))??)
}

/// Clear, for Inbox items whose track is in the Library already.
#[tauri::command(async)]
#[specta::specta]
pub fn inbox_clear(
    writer: State<'_, Writer>,
    item_ids: Vec<i64>,
) -> Result<InboxOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(clear(c, &item_ids)))??)
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
