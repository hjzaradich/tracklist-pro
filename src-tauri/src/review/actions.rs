//! What the user decides in Review about a Library track's file (1bE-5,
//! 1bE-7; ROADMAP 1.12). Every action is one undoable operation
//! ([`crate::ops`]), and its Undo brings the item back into the queue.
//!
//! # Remembered decisions
//!
//! Three of them write one row of `review_decision` (migration 0027), which
//! the queue's own WHERE reads ([`super::queue`]), so the item leaves as
//! the action returns and is back as its Undo does:
//!
//! - **Dismiss** a "Better file available" item ([`dismiss_better_file`]).
//!   It covers that one better file for that linked file: another better
//!   file later is a new item.
//! - **Keep anyway**, for a quality problem ([`keep_anyway`]). The track
//!   leaves "Quality problems" for good. Nothing about the verdict or the
//!   ranking changes: the file is still flagged there.
//! - **Find a better file**, for a quality problem ([`want_better_file`]):
//!   a remembered mark that the user wants the track in better quality. The
//!   item leaves the queue; nothing else reads the mark yet (the wishlist
//!   does, in Phase 2).
//!
//! A decision is kept **by what the files hold** ([`content_key`]: a file's
//! `audio_hash`, or its `blake3` when it has none), never by a file or track
//! id. So it survives a rescan, a retag, a move, and a music folder removed
//! and added again. A file that is encoded again holds other audio, so it
//! has another key and the decision doesn't cover it: that is a new item. A
//! file with neither hash yet can't be decided about ([`Refused::NoAudioKnown`]).
//!
//! # Switching the linked file
//!
//! Only when the linked file is gone from disk ([`switch_linked_file`]; the
//! "Linked file gone, copy found" item). It writes
//! `library_track.linked_file_id` through the operation log and ranks the
//! track's files again at once, so its "better file" is worked out for the
//! new link. It refuses ([`SwitchRefusal`]) when the linked file is on disk
//! after all, and when the file to switch to is not a file of the track,
//! is itself gone, or is linked by another Library track; and when an entry
//! matched to that file is the entry another Library track is sent as
//! today, as the automatic follow holds there ([`crate::library::follow`]).
//!
//! **What a switch means in rekordbox** ([`SwitchEffect`]) is the send's own
//! answer ([`crate::send_values::sent_as`]): what the track is sent as
//! today, and what it would be sent as linked to the copy. A switch never
//! overrides relink's match: where relink trusts rekordbox's entry on the
//! copy, the track stays that entry. Where it doesn't, the track goes to
//! rekordbox as a new one, and the entry it was sent as is recorded
//! (`replaced_entry`) in the same operation, for the list of tracks to
//! remove in rekordbox ([`crate::after_send`]). The item shows the effect,
//! and the switch works it out again when it is applied: if it is another
//! one by then (a relink ran in between), nothing is switched
//! ([`Refused::Changed`]) and the screen shows the item again.
//!
//! **"Accept all"** ([`accept_clear_best_files`]) makes that same switch for
//! every "Linked file gone, copy found" item, to each item's suggested
//! copy, as one operation, and for nothing else. Which effects it covers is
//! said in one place, [`accept_all_covers`].
//!
//! Nothing here opens, moves or writes a file, and nothing asks the system
//! where a drive is on the database's writer: the commands take that answer
//! beforehand ([`crate::relink::Mounted`]).

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tauri::State;

use super::queue::{better_from, quality_from};
use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::{ErrorKind, IpcError};
use crate::jobs::JobQueue;
use crate::library::LibraryTrackId;
use crate::ops::{self, OpsError, Recorder};
use crate::paths::Volumes;
use crate::relink::Mounted;
use crate::scan::system_volumes;
use crate::send_values::{sent_as, SentAs};

/// The operation kinds, as the operation log stores them.
pub const DISMISS_OPERATION: &str = "dismiss_better_file";
pub const KEEP_OPERATION: &str = "keep_anyway";
pub const WANT_BETTER_OPERATION: &str = "want_better_file";
pub const SWITCH_OPERATION: &str = "switch_linked_file";
pub const ACCEPT_ALL_OPERATION: &str = "accept_clear_best_files";

/// The kinds of `review_decision`, as stored.
pub(crate) const DISMISSED_BETTER: &str = "dismissed_better";
pub(crate) const KEEP_ANYWAY: &str = "keep_anyway";
pub(crate) const WANT_BETTER: &str = "want_better";

/// The audio the file `alias` holds, as SQL: grouping's own name for it
/// ([`crate::grouping::CONTENT_KEY`]), its `audio_hash`, or its `blake3`
/// when it has none. NULL for a file that has neither yet.
pub(crate) fn content_key(alias: &str) -> String {
    crate::grouping::CONTENT_KEY.replace("f.", &format!("{alias}."))
}

/// Why an action changed nothing.
#[derive(Debug)]
pub enum Refused {
    /// The item is no longer in the queue (answered already, or the files
    /// changed under it).
    ItemGone,
    /// A file the decision would be kept by has no hash yet, so there is
    /// nothing to remember it by.
    NoAudioKnown,
    /// The switch can't be made.
    Switch(SwitchRefusal),
    /// What the switch means in rekordbox is not what the item showed.
    Changed,
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

impl From<DbError> for Refused {
    fn from(e: DbError) -> Self {
        Refused::Ops(OpsError::Db(e))
    }
}

impl From<Refused> for IpcError {
    fn from(e: Refused) -> Self {
        match e {
            Refused::ItemGone => IpcError::new(ErrorKind::ReviewItemGone),
            Refused::NoAudioKnown => IpcError::new(ErrorKind::ReviewNoAudioKnown),
            Refused::Switch(refusal) => IpcError::new(refusal.kind()),
            Refused::Changed => IpcError::new(ErrorKind::ReviewSwitchChanged),
            Refused::Ops(e) => IpcError::from(e),
        }
    }
}

/// What an action recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Acted {
    /// The operation to offer Undo for. `None` only if nothing changed.
    pub operation_id: Option<i64>,
}

// ---- remembered decisions -------------------------------------------------

/// Inserts one decision, as one operation of `kind`.
fn remember(
    conn: &mut Connection,
    operation: &str,
    details: serde_json::Value,
    kind: &'static str,
    linked_key: Vec<u8>,
    other_key: Vec<u8>,
) -> Result<Acted, Refused> {
    let recorded = ops::record(conn, operation, &details, |rec| {
        rec.insert(
            "review_decision",
            &[
                ("kind", Value::Text(kind.to_owned())),
                ("linked_key", Value::Blob(linked_key)),
                ("other_key", Value::Blob(other_key)),
            ],
        )
    })?;
    Ok(Acted {
        operation_id: recorded.operation_id,
    })
}

/// Dismisses the "Better file available" item of the Library track of
/// `recording`, for the better file `file`. Remembered for that one better
/// file and the file the track is linked to now; Undo brings the item back.
/// Refused if that item isn't in the queue (it names another file by now,
/// or the linked file is gone: that item is a switch, not a dismissal).
pub fn dismiss_better_file(
    conn: &mut Connection,
    recording: i64,
    file: i64,
) -> Result<Acted, Refused> {
    let keys: Option<(Option<Vec<u8>>, Option<Vec<u8>>)> = conn
        .query_row(
            &format!(
                "SELECT {}, {} {}",
                content_key("lf"),
                content_key("bfile"),
                better_from(false, "AND lt.recording_id = ?1 AND bf.file_id = ?2")
            ),
            [recording, file],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some(keys) = keys else {
        return Err(Refused::ItemGone);
    };
    let (Some(linked_key), Some(other_key)) = keys else {
        return Err(Refused::NoAudioKnown);
    };
    remember(
        conn,
        DISMISS_OPERATION,
        json!({ "recordingId": recording, "fileId": file }),
        DISMISSED_BETTER,
        linked_key,
        other_key,
    )
}

/// The key of the linked file of the quality-problem item of `recording`.
fn quality_item(conn: &Connection, recording: i64) -> Result<Vec<u8>, Refused> {
    let key: Option<Option<Vec<u8>>> = conn
        .query_row(
            &format!(
                "SELECT {} {}",
                content_key("f"),
                quality_from("AND lt.recording_id = ?1")
            ),
            [recording],
            |r| r.get(0),
        )
        .optional()?;
    match key {
        None => Err(Refused::ItemGone),
        Some(None) => Err(Refused::NoAudioKnown),
        Some(Some(key)) => Ok(key),
    }
}

/// "Keep anyway" for the quality problem of the Library track of
/// `recording`: the track leaves "Quality problems" for good. The verdict
/// stays as it is, so the ranking still treats the file as flagged.
pub fn keep_anyway(conn: &mut Connection, recording: i64) -> Result<Acted, Refused> {
    let key = quality_item(conn, recording)?;
    remember(
        conn,
        KEEP_OPERATION,
        json!({ "recordingId": recording }),
        KEEP_ANYWAY,
        key,
        Vec::new(),
    )
}

/// "Find a better file" for the quality problem of the Library track of
/// `recording`: remembers that the user wants the track in better quality,
/// and the item leaves the queue.
pub fn want_better_file(conn: &mut Connection, recording: i64) -> Result<Acted, Refused> {
    let key = quality_item(conn, recording)?;
    remember(
        conn,
        WANT_BETTER_OPERATION,
        json!({ "recordingId": recording }),
        WANT_BETTER,
        key,
        Vec::new(),
    )
}

/// Whether the user marked the track of `recording` as wanted in better
/// quality: a `want_better` decision for the audio its Library track is
/// linked to. What the wishlist will read (Phase 2).
pub fn wants_better(conn: &Connection, recording: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        &format!(
            "SELECT EXISTS (SELECT 1 FROM library_track lt
                            JOIN file f ON f.id = lt.linked_file_id
                            JOIN review_decision d ON d.linked_key = {}
                            WHERE lt.recording_id = ?1 AND d.kind = '{WANT_BETTER}')",
            content_key("f")
        ),
        [recording],
        |r| r.get(0),
    )
}

// ---- the switch -----------------------------------------------------------

/// What switching a Library track's linked file means in rekordbox, by what
/// the track is sent as before and after ([`sent_as`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SwitchEffect {
    /// rekordbox doesn't know the track: it is sent as no entry today.
    /// Nothing of rekordbox's changes.
    NotInRekordbox,
    /// rekordbox's entry stays the track's: relink trusts it on the copy
    /// (or the user confirmed it). rekordbox keeps the track and its cues,
    /// and shows the file as missing until it is relocated there.
    KeepsEntry,
    /// The track is sent as rekordbox's entry today and as a new track on
    /// the copy: rekordbox sees a new track without the old cues, and the
    /// old entry goes on the list of tracks to remove in rekordbox.
    NewTrack,
    /// The track is sent as one entry today, and on the copy as another
    /// entry rekordbox already holds for that file. The old entry goes on
    /// the list of tracks to remove in rekordbox.
    OtherEntry,
}

/// Which switches "Accept all" makes: the clear cases. A switch that arrives
/// in rekordbox as another track than today (a new one without the old
/// cues, or another entry) isn't one; those items stay in the queue for a
/// decision one by one. **The one place that says it** (the owner may decide
/// to include them).
pub fn accept_all_covers(effect: SwitchEffect) -> bool {
    match effect {
        SwitchEffect::NotInRekordbox | SwitchEffect::KeepsEntry => true,
        SwitchEffect::NewTrack | SwitchEffect::OtherEntry => false,
    }
}

/// Why a linked file can't be switched. Nothing was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SwitchRefusal {
    /// The linked file is on disk after all.
    LinkedOnDisk,
    /// The file to switch to isn't a file of the track.
    NotInTrack,
    /// The file to switch to is gone from disk itself.
    TargetGone,
    /// Another Library track is linked to the file to switch to.
    TargetLinked,
    /// A rekordbox entry matched to the file to switch to is the entry
    /// another Library track is sent as today: that track would lose it.
    EntryOfAnotherTrack,
}

impl SwitchRefusal {
    fn kind(self) -> ErrorKind {
        match self {
            SwitchRefusal::LinkedOnDisk => ErrorKind::ReviewSwitchLinkedOnDisk,
            SwitchRefusal::NotInTrack => ErrorKind::ReviewSwitchNotInTrack,
            SwitchRefusal::TargetGone => ErrorKind::ReviewSwitchTargetGone,
            SwitchRefusal::TargetLinked => ErrorKind::ReviewSwitchTargetLinked,
            SwitchRefusal::EntryOfAnotherTrack => ErrorKind::ReviewSwitchEntryTaken,
        }
    }
}

/// The switch a "Linked file gone, copy found" item offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SwitchOffer {
    /// The copy to switch to: the item's suggested file.
    pub file_id: i64,
    /// What the switch means in rekordbox; `None` when it is refused.
    pub effect: Option<SwitchEffect>,
    /// Why the switch can't be made now, if it can't.
    pub refusal: Option<SwitchRefusal>,
}

/// One switch, worked out: the Library track, its linked file, the file to
/// switch to, and either what it means in rekordbox or why it is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Planned {
    pub recording: i64,
    pub library_track: LibraryTrackId,
    pub from: i64,
    pub to: i64,
    pub outcome: Result<Effect, SwitchRefusal>,
}

/// A switch's effect, with the entry it leaves behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Effect {
    pub effect: SwitchEffect,
    /// The `rekordbox_track` row the track is sent as today and wouldn't be
    /// after the switch: what goes on the list to remove in rekordbox.
    pub left_behind: Option<i64>,
}

/// The Library track of `recording` and its linked file, with what stands
/// in the way of switching it to `to`. `None` if the track isn't a linked
/// Library track with a file.
fn check(
    conn: &Connection,
    recording: i64,
    to: i64,
) -> rusqlite::Result<Option<(LibraryTrackId, i64, Option<SwitchRefusal>)>> {
    let track: Option<(i64, i64, bool)> = conn
        .query_row(
            "SELECT lt.id, a.id, a.present FROM library_track lt
             JOIN file a ON a.id = lt.linked_file_id
             WHERE lt.recording_id = ?1 AND lt.kind = 'linked'",
            [recording],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((library_track, from, linked_present)) = track else {
        return Ok(None);
    };
    let target: Option<(bool, bool, bool)> = conn
        .query_row(
            "SELECT n.present,
                    EXISTS (SELECT 1 FROM recording_file rf
                            WHERE rf.file_id = n.id AND rf.recording_id = ?2),
                    EXISTS (SELECT 1 FROM library_track x
                            WHERE x.linked_file_id = n.id AND x.id <> ?3)
             FROM file n WHERE n.id = ?1",
            [to, recording, library_track],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let refusal = if linked_present {
        Some(SwitchRefusal::LinkedOnDisk)
    } else {
        match target {
            None | Some((_, false, _)) => Some(SwitchRefusal::NotInTrack),
            Some((false, _, _)) => Some(SwitchRefusal::TargetGone),
            Some((_, _, true)) => Some(SwitchRefusal::TargetLinked),
            Some((true, true, false)) => None,
        }
    };
    Ok(Some((LibraryTrackId(library_track), from, refusal)))
}

/// Works out each wanted switch (a track and the file to switch it to), in
/// a fixed number of queries. A track that isn't a linked Library track
/// with a file is left out. Changes nothing.
///
/// The effects are right for all of them together, not only one by one: a
/// switch here never takes a file that an entry another Library track is
/// sent as is matched to ([`SwitchRefusal::EntryOfAnotherTrack`]), and a
/// gone file is never matched by relink, so making one of these switches
/// doesn't change what another track is sent as.
pub(crate) fn plan(
    conn: &Connection,
    volumes: &impl Volumes,
    wanted: &[(i64, i64)],
) -> rusqlite::Result<Vec<Planned>> {
    let mut planned: Vec<Planned> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    for &(recording, to) in wanted {
        let Some((library_track, from, refusal)) = check(conn, recording, to)? else {
            continue;
        };
        if refusal.is_none() {
            open.push(planned.len());
        }
        planned.push(Planned {
            recording,
            library_track,
            from,
            to,
            // Filled in below for the switches nothing stands in the way of.
            outcome: Err(refusal.unwrap_or(SwitchRefusal::LinkedOnDisk)),
        });
    }
    if open.is_empty() {
        return Ok(planned);
    }

    // What every Library track with a gone linked file is sent as today, in
    // one send: the entry each of the tracks here is on, and the entries of
    // the others, which a switch mustn't take.
    let gone: Vec<LibraryTrackId> = conn
        .prepare_cached(
            "SELECT lt.id FROM library_track lt JOIN file a ON a.id = lt.linked_file_id
             WHERE lt.kind = 'linked' AND a.present = 0 ORDER BY lt.id",
        )?
        .query_map([], |r| r.get(0).map(LibraryTrackId))?
        .collect::<rusqlite::Result<_>>()?;
    let asked: Vec<(LibraryTrackId, Option<i64>)> = gone.iter().map(|id| (*id, None)).collect();
    let before: HashMap<LibraryTrackId, SentAs> = gone
        .iter()
        .copied()
        .zip(sent_as(conn, volumes, &asked)?)
        .collect();
    let mut entry_of: HashMap<i64, LibraryTrackId> = HashMap::new();
    for (track, sent) in &before {
        if let SentAs::Entry(row) = sent {
            entry_of.insert(*row, *track);
        }
    }

    // The rekordbox rows matched to each file to switch to, whatever the
    // match.
    let targets: HashSet<i64> = open.iter().map(|&i| planned[i].to).collect();
    let json = serde_json::to_string(&targets.into_iter().collect::<Vec<_>>())
        .expect("numbers always serialize");
    let mut rows_on: HashMap<i64, Vec<i64>> = HashMap::new();
    {
        let mut stmt = conn.prepare_cached(
            "SELECT file_id, id FROM rekordbox_track
             WHERE file_id IN (SELECT value FROM json_each(?1))",
        )?;
        for row in stmt.query_map([&json], |r| Ok((r.get(0)?, r.get(1)?)))? {
            let (file, id): (i64, i64) = row?;
            rows_on.entry(file).or_default().push(id);
        }
    }
    open.retain(|&i| {
        let p = &mut planned[i];
        let taken = rows_on.get(&p.to).is_some_and(|rows| {
            rows.iter().any(|row| {
                entry_of
                    .get(row)
                    .is_some_and(|owner| *owner != p.library_track)
            })
        });
        if taken {
            p.outcome = Err(SwitchRefusal::EntryOfAnotherTrack);
        }
        !taken
    });

    // What each track would be sent as on the copy.
    let asked: Vec<(LibraryTrackId, Option<i64>)> = open
        .iter()
        .map(|&i| (planned[i].library_track, Some(planned[i].to)))
        .collect();
    let after = sent_as(conn, volumes, &asked)?;
    for (&i, after) in open.iter().zip(after) {
        let p = &mut planned[i];
        let was = before
            .get(&p.library_track)
            .copied()
            .unwrap_or(SentAs::Nothing);
        p.outcome = Ok(match (was, after) {
            (SentAs::Entry(old), SentAs::Entry(new)) if old == new => Effect {
                effect: SwitchEffect::KeepsEntry,
                left_behind: None,
            },
            (SentAs::Entry(old), SentAs::Entry(_)) => Effect {
                effect: SwitchEffect::OtherEntry,
                left_behind: Some(old),
            },
            (SentAs::Entry(old), SentAs::New | SentAs::Nothing) => Effect {
                effect: SwitchEffect::NewTrack,
                left_behind: Some(old),
            },
            (SentAs::New | SentAs::Nothing, _) => Effect {
                effect: SwitchEffect::NotInRekordbox,
                left_behind: None,
            },
        });
    }
    Ok(planned)
}

/// The switch the "Linked file gone, copy found" item of `recording` offers,
/// to its suggested file `to`. `None` if the track isn't in the Library.
pub(crate) fn offer(
    conn: &Connection,
    volumes: &impl Volumes,
    recording: i64,
    to: i64,
) -> rusqlite::Result<Option<SwitchOffer>> {
    Ok(plan(conn, volumes, &[(recording, to)])?
        .first()
        .map(|p| SwitchOffer {
            file_id: to,
            effect: p.outcome.ok().map(|e| e.effect),
            refusal: p.outcome.err(),
        }))
}

/// One switched Library track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SwitchedTrack {
    pub recording_id: i64,
    pub library_track_id: LibraryTrackId,
    /// The linked file before: gone from disk.
    pub from_file_id: i64,
    /// The linked file now.
    pub to_file_id: i64,
    pub effect: SwitchEffect,
}

/// Makes one planned switch, as changes of the operation `rec` records: the
/// link, and the record of the entry it leaves behind.
fn apply(rec: &mut Recorder<'_>, p: &Planned, effect: Effect) -> Result<SwitchedTrack, OpsError> {
    rec.set("library_track", p.library_track.0, "linked_file_id", p.to)?;
    if let Some(entry) = effect.left_behind {
        let left: Option<(String, String)> = rec
            .read_rows(
                "SELECT location, location_key FROM rekordbox_track WHERE id = ?1
                   AND NOT EXISTS (SELECT 1 FROM replaced_entry r
                                   WHERE r.location_key = rekordbox_track.location_key)",
                [entry],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            .pop();
        if let Some((location, key)) = left {
            rec.insert(
                "replaced_entry",
                &[
                    ("recording_id", Value::Integer(p.recording)),
                    ("location", Value::Text(location)),
                    ("location_key", Value::Text(key)),
                ],
            )?;
        }
    }
    Ok(SwitchedTrack {
        recording_id: p.recording,
        library_track_id: p.library_track,
        from_file_id: p.from,
        to_file_id: p.to,
        effect: effect.effect,
    })
}

/// What a switch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Switched {
    pub operation_id: Option<i64>,
    pub track: SwitchedTrack,
}

/// Ranks the files of the switched tracks again, so their "better file" is
/// of the new link as the action returns. The switch has committed by now:
/// a failure here is logged and left to the roles job, which finds the
/// tracks due.
fn rank_again(conn: &mut Connection, recordings: &[i64]) {
    let now = &recordings[..recordings.len().min(crate::file_roles::BATCH)];
    if let Err(e) = crate::file_roles::settle_tracks(conn, now) {
        eprintln!("review: couldn't rank the switched tracks again: {e}");
    }
}

/// Switches the linked file of the Library track of `recording` to `to`,
/// as one operation. `shown` is the effect the item showed: the switch is
/// made only if that is still what it means in rekordbox.
///
/// `volumes` says where each drive is, for the send's own answer; no file
/// is opened. On the database's writer, hand in an answer taken beforehand
/// ([`Mounted`]).
pub fn switch_linked_file(
    conn: &mut Connection,
    volumes: &impl Volumes,
    recording: i64,
    to: i64,
    shown: SwitchEffect,
) -> Result<Switched, Refused> {
    let Some(planned) = plan(conn, volumes, &[(recording, to)])?.pop() else {
        return Err(Refused::ItemGone);
    };
    let effect = planned.outcome.map_err(Refused::Switch)?;
    if effect.effect != shown {
        return Err(Refused::Changed);
    }
    let details = json!({
        "recordingId": recording,
        "libraryTrackId": planned.library_track.0,
        "from": planned.from,
        "to": to,
    });
    // One writer call: nothing can change between the plan and the write.
    let recorded = ops::record(conn, SWITCH_OPERATION, &details, |rec| {
        apply(rec, &planned, effect)
    })?;
    rank_again(conn, &[recording]);
    Ok(Switched {
        operation_id: recorded.operation_id,
        track: recorded.value,
    })
}

/// Why "Accept all" left an item as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    LinkedOnDisk,
    NotInTrack,
    TargetGone,
    TargetLinked,
    EntryOfAnotherTrack,
    /// The switch would arrive in rekordbox as a new track without the old
    /// cues: not a clear case ([`accept_all_covers`]).
    NewTrack,
    /// The switch would make the track another of rekordbox's entries.
    OtherEntry,
}

/// An item "Accept all" left in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkippedItem {
    pub recording_id: i64,
    pub reason: SkipReason,
}

/// What "Accept all" did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedAll {
    /// One operation for every switch; `None` if nothing was switched.
    pub operation_id: Option<i64>,
    /// Every Library track that was switched, with its old and new file.
    pub switched: Vec<SwitchedTrack>,
    /// The items left in the queue, each with why.
    pub skipped: Vec<SkippedItem>,
}

/// Every "Linked file gone, copy found" item: the track and its suggested
/// copy, oldest first.
fn clear_best_file_items(conn: &Connection) -> rusqlite::Result<Vec<(i64, i64)>> {
    conn.prepare_cached(&format!(
        "SELECT lt.recording_id, bf.file_id {} ORDER BY 1",
        better_from(true, "")
    ))?
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
    .collect()
}

/// Sorts planned switches into the ones "Accept all" makes and the ones it
/// skips, each with its reason.
fn sort_out(planned: Vec<Planned>) -> (Vec<(Planned, Effect)>, Vec<SkippedItem>) {
    let mut clear = Vec::new();
    let mut skipped = Vec::new();
    for p in planned {
        let reason = match p.outcome {
            Ok(effect) if accept_all_covers(effect.effect) => {
                clear.push((p, effect));
                continue;
            }
            Ok(effect) => match effect.effect {
                SwitchEffect::OtherEntry => SkipReason::OtherEntry,
                _ => SkipReason::NewTrack,
            },
            Err(SwitchRefusal::LinkedOnDisk) => SkipReason::LinkedOnDisk,
            Err(SwitchRefusal::NotInTrack) => SkipReason::NotInTrack,
            Err(SwitchRefusal::TargetGone) => SkipReason::TargetGone,
            Err(SwitchRefusal::TargetLinked) => SkipReason::TargetLinked,
            Err(SwitchRefusal::EntryOfAnotherTrack) => SkipReason::EntryOfAnotherTrack,
        };
        skipped.push(SkippedItem {
            recording_id: p.recording,
            reason,
        });
    }
    (clear, skipped)
}

/// How many items "Accept all" would switch now: the N on its button.
pub fn accept_all_count(conn: &Connection, volumes: &impl Volumes) -> rusqlite::Result<u32> {
    let conn = conn.unchecked_transaction()?;
    let items = clear_best_file_items(&conn)?;
    let (clear, _) = sort_out(plan(&conn, volumes, &items)?);
    Ok(u32::try_from(clear.len()).unwrap_or(u32::MAX))
}

/// "Accept all" for "Clear best file": switches every "Linked file gone,
/// copy found" item that is a clear case ([`accept_all_covers`]) to its
/// suggested copy, as ONE operation, so one Undo restores every link.
/// Nothing in another group is touched. Returns what it switched and what
/// it left, with why.
///
/// A track of the group whose files are due to be ranked again (an item in
/// the queue, or a Library track whose linked file is gone) is ranked first,
/// so each item and its suggested copy are what the ranking gives now.
pub fn accept_clear_best_files(
    conn: &mut Connection,
    volumes: &impl Volumes,
) -> Result<AcceptedAll, Refused> {
    let due: Vec<i64> = conn
        .prepare_cached(
            "SELECT d.recording_id FROM best_file_due d
             WHERE d.recording_id IN (SELECT recording_id FROM better_file WHERE step = 'onDisk')
                OR d.recording_id IN (SELECT lt.recording_id FROM library_track lt
                                      JOIN file a ON a.id = lt.linked_file_id
                                      WHERE lt.kind = 'linked' AND a.present = 0)",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    crate::file_roles::settle_tracks(conn, &due)?;

    let items = clear_best_file_items(conn)?;
    let (clear, skipped) = sort_out(plan(conn, volumes, &items)?);
    if clear.is_empty() {
        return Ok(AcceptedAll {
            operation_id: None,
            switched: Vec::new(),
            skipped,
        });
    }
    // One writer call: nothing can change between the plan and the writes.
    let recorded = ops::record(conn, ACCEPT_ALL_OPERATION, &json!({}), |rec| {
        let mut switched = Vec::with_capacity(clear.len());
        for (p, effect) in &clear {
            switched.push(apply(rec, p, *effect)?);
        }
        rec.detail("tracks", switched.len());
        Ok(switched)
    })?;
    let recordings: Vec<i64> = recorded.value.iter().map(|s| s.recording_id).collect();
    rank_again(conn, &recordings);
    Ok(AcceptedAll {
        operation_id: recorded.operation_id,
        switched: recorded.value,
        skipped,
    })
}

// ---- commands ---------------------------------------------------------------

/// Where the drives are, asked off the database's writer.
fn mounted(reads: &ReadPool) -> Result<Mounted, DbError> {
    let known = reads.read(crate::relink::identities)?;
    Ok(Mounted::ask(&known, &system_volumes()))
}

/// Asks for the background work a changed link needs: the roles of every
/// track that is due, and the search index (a Library track's shown names
/// follow its link). A failure to ask is not the action's: the next job to
/// end asks again.
pub(crate) fn ask_for_follow_up_work(writer: &Writer, jobs: &JobQueue) {
    if let Err(e) = crate::file_roles::request(writer, |j| jobs.enqueue(j)) {
        eprintln!("review: couldn't ask for the roles: {e:?}");
    }
    if let Err(e) = crate::search::request(writer, |j| jobs.enqueue(j)) {
        eprintln!("review: couldn't ask for the search index: {e:?}");
    }
}

/// Dismisses a "Better file available" item, for that one better file.
#[tauri::command(async)]
#[specta::specta]
pub fn review_dismiss_better_file(
    writer: State<'_, Writer>,
    recording_id: i64,
    file_id: i64,
) -> Result<Acted, IpcError> {
    Ok(writer.call(move |c| Ok(dismiss_better_file(c, recording_id, file_id)))??)
}

/// "Keep anyway" for a quality problem.
#[tauri::command(async)]
#[specta::specta]
pub fn review_keep_anyway(
    writer: State<'_, Writer>,
    recording_id: i64,
) -> Result<Acted, IpcError> {
    Ok(writer.call(move |c| Ok(keep_anyway(c, recording_id)))??)
}

/// "Find a better file" for a quality problem: remembers the wish.
#[tauri::command(async)]
#[specta::specta]
pub fn review_want_better_file(
    writer: State<'_, Writer>,
    recording_id: i64,
) -> Result<Acted, IpcError> {
    Ok(writer.call(move |c| Ok(want_better_file(c, recording_id)))??)
}

/// Switches a Library track's linked file, which is gone, to `file_id`.
/// `effect` is what the item showed the switch means in rekordbox.
#[tauri::command(async)]
#[specta::specta]
pub fn review_switch_linked_file(
    writer: State<'_, Writer>,
    reads: State<'_, ReadPool>,
    jobs: State<'_, JobQueue>,
    recording_id: i64,
    file_id: i64,
    effect: SwitchEffect,
) -> Result<Switched, IpcError> {
    let mounted = mounted(&reads)?;
    let switched = writer
        .call(move |c| Ok(switch_linked_file(c, &mounted, recording_id, file_id, effect)))??;
    ask_for_follow_up_work(&writer, &jobs);
    Ok(switched)
}

/// How many items "Accept all" would switch now.
#[tauri::command]
#[specta::specta]
pub async fn review_accept_all_count(reads: State<'_, ReadPool>) -> Result<u32, IpcError> {
    let volumes = system_volumes();
    Ok(reads.read(|c| accept_all_count(c, &volumes))?)
}

/// "Accept all" for "Clear best file", as one operation.
#[tauri::command(async)]
#[specta::specta]
pub fn review_accept_clear_best_files(
    writer: State<'_, Writer>,
    reads: State<'_, ReadPool>,
    jobs: State<'_, JobQueue>,
) -> Result<AcceptedAll, IpcError> {
    let mounted = mounted(&reads)?;
    let accepted = writer.call(move |c| Ok(accept_clear_best_files(c, &mounted)))??;
    ask_for_follow_up_work(&writer, &jobs);
    Ok(accepted)
}
