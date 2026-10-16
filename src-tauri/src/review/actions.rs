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
//!   file, now or later, is a new item. The roles job offers a track the
//!   best-ranked better file that isn't dismissed, and a dismissal made or
//!   taken back makes the track due for it (a trigger), so the next item is
//!   there as the action returns. With every better file dismissed, a
//!   flagged linked file is a single-file quality problem again.
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
//! the "Linked file gone, copy found" items the screen showed, each to the
//! copy it showed, as one operation, and for nothing else: an item that
//! joined the queue since is never switched unseen. Which effects it covers
//! is said in one place, [`accept_all_covers`].
//!
//! **Undo after a send.** A switch that made the track a new one for
//! rekordbox can't be undone once the track was sent since
//! ([`sent_as_new_since`]): the copy's entry is in rekordbox by then, and
//! nothing would list it. A switch that kept the track rekordbox's own
//! entry can be undone whenever.
//!
//! # Relink matches
//!
//! A probable match of a rekordbox entry to a file is confirmed or answered
//! "Not the same" here too (1bE-6): the commands are thin, and what they do
//! is in [`crate::relink::decide`], beside the rules that read it.
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

/// What a file holds, as [`content_key`] reads it: `None` if it has no hash
/// yet.
type Key = Option<Vec<u8>>;

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
    /// Two tracks to merge are linked as versions: versions never merge.
    LinkedVersions,
    /// The file to split off holds the very same audio as a file it would
    /// be split from: such files are always one track (1bE-2).
    SameAudio,
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

impl From<crate::relink::decide::Refused> for Refused {
    fn from(e: crate::relink::decide::Refused) -> Self {
        match e {
            crate::relink::decide::Refused::ItemGone => Refused::ItemGone,
            crate::relink::decide::Refused::NoAudioKnown => Refused::NoAudioKnown,
            crate::relink::decide::Refused::Ops(e) => Refused::Ops(e),
        }
    }
}

impl From<Refused> for IpcError {
    fn from(e: Refused) -> Self {
        match e {
            Refused::ItemGone => IpcError::new(ErrorKind::ReviewItemGone),
            Refused::NoAudioKnown => IpcError::new(ErrorKind::ReviewNoAudioKnown),
            Refused::Switch(refusal) => IpcError::new(refusal.kind()),
            Refused::Changed => IpcError::new(ErrorKind::ReviewSwitchChanged),
            Refused::LinkedVersions => IpcError::new(ErrorKind::ReviewMergeLinkedVersions),
            Refused::SameAudio => IpcError::new(ErrorKind::ReviewSameAudio),
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
    let keys: Option<(Key, Key)> = conn
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
    let acted = remember(
        conn,
        DISMISS_OPERATION,
        json!({ "recordingId": recording, "fileId": file }),
        DISMISSED_BETTER,
        linked_key,
        other_key,
    )?;
    // The dismissal made the track due (a trigger): ranked again at once,
    // the next better file that isn't dismissed is its item, if it has one.
    rank_again(conn, &[recording]);
    Ok(acted)
}

/// The files of the track `recording` that were dismissed as a better file
/// for the audio the file `linked` holds. What the roles job leaves out when
/// it works out which better file a Library track is offered
/// ([`crate::file_roles`]).
pub(crate) fn dismissed_better_files(
    conn: &Connection,
    recording: i64,
    linked: i64,
) -> rusqlite::Result<HashSet<i64>> {
    conn.prepare_cached(&format!(
        "SELECT bfile.id FROM recording_file rf
         JOIN file bfile ON bfile.id = rf.file_id
         JOIN file lf ON lf.id = ?2
         WHERE rf.recording_id = ?1
           AND EXISTS (SELECT 1 FROM review_decision d
                       WHERE d.kind = '{DISMISSED_BETTER}'
                         AND d.linked_key = {} AND d.other_key = {})",
        content_key("lf"),
        content_key("bfile")
    ))?
    .query_map([recording, linked], |r| r.get(0))?
    .collect()
}

/// The key of the linked file of the quality-problem item of `recording`.
fn quality_item(conn: &Connection, recording: i64) -> Result<Vec<u8>, Refused> {
    let key: Option<Key> = conn
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
/// decision one by one (owner decision, 2026-10-13). **The one place that
/// says it.**
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
    /// Linked to the copy, the track goes to rekordbox as a new track (the
    /// next send adds an entry for the copy).
    pub arrives_new: bool,
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
        let (effect, left_behind) = match (was, after) {
            (SentAs::Entry(old), SentAs::Entry(new)) if old == new => {
                (SwitchEffect::KeepsEntry, None)
            }
            (SentAs::Entry(old), SentAs::Entry(_)) => (SwitchEffect::OtherEntry, Some(old)),
            (SentAs::Entry(old), SentAs::New | SentAs::Nothing) => {
                (SwitchEffect::NewTrack, Some(old))
            }
            (SentAs::New | SentAs::Nothing, _) => (SwitchEffect::NotInRekordbox, None),
        };
        p.outcome = Ok(Effect {
            effect,
            left_behind,
            arrives_new: after == SentAs::New,
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

/// The detail of a switch operation that names the Library tracks it made
/// new tracks for rekordbox: what [`sent_as_new_since`] reads.
const SENT_AS_NEW: &str = "sentAsNew";

/// Whether a Library track that the switch operation `operation` made a new
/// track for rekordbox (a track rekordbox didn't know, or one whose entry
/// isn't trusted on the copy) **has been sent since**. Then the copy's new
/// entry is in rekordbox (or about to be imported), and undoing the switch
/// would leave it there with nothing to list it: the undo is refused, as
/// undoing the add of a track that was sent since is (ROADMAP §2,
/// `operation` / `change`).
///
/// A switch that keeps the track rekordbox's own entry adds nothing to
/// rekordbox, so a send in between doesn't stand in the way of its undo.
pub(crate) fn sent_as_new_since(conn: &Connection, operation: i64) -> rusqlite::Result<bool> {
    conn.prepare_cached(&format!(
        "SELECT EXISTS (
             SELECT 1 FROM operation o, json_each(o.details, '$.{SENT_AS_NEW}') made
             JOIN library_track lt ON lt.id = made.value
             WHERE o.id = ?1 AND o.kind IN ('{SWITCH_OPERATION}', '{ACCEPT_ALL_OPERATION}')
               AND lt.last_exported_at >= o.created_at)"
    ))?
    .query_row([operation], |r| r.get(0))
}

/// What a switch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Switched {
    pub operation_id: Option<i64>,
    pub track: SwitchedTrack,
}

/// Ranks the files of the tracks an action changed again, so their "better
/// file" is right as the action returns: the first [`crate::file_roles::BATCH`]
/// of them (500); beyond that the roles job does it, and the queue is right
/// when that job ends. The action has committed by now: a failure here is
/// logged and left to the roles job, which finds the tracks due.
pub(super) fn rank_again(conn: &mut Connection, recordings: &[i64]) {
    let now = &recordings[..recordings.len().min(crate::file_roles::BATCH)];
    if let Err(e) = crate::file_roles::settle_tracks(conn, now) {
        eprintln!("review: couldn't rank the tracks again: {e}");
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
        if effect.arrives_new {
            rec.detail(SENT_AS_NEW, vec![planned.library_track.0]);
        }
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
    /// The item isn't in the queue any more, or suggests another copy than
    /// the one the screen showed.
    ItemGone,
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

/// One "Linked file gone, copy found" item as "Accept all" names it: the
/// track and the copy the item suggests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AcceptItem {
    pub recording_id: i64,
    pub file_id: i64,
}

/// The items "Accept all" would switch now, oldest first: the N on its
/// button is how many, and a click sends exactly these back.
pub fn accept_all_items(
    conn: &Connection,
    volumes: &impl Volumes,
) -> rusqlite::Result<Vec<AcceptItem>> {
    let conn = conn.unchecked_transaction()?;
    let items = clear_best_file_items(&conn)?;
    let (clear, _) = sort_out(plan(&conn, volumes, &items)?);
    Ok(clear
        .into_iter()
        .map(|(p, _)| AcceptItem {
            recording_id: p.recording,
            file_id: p.to,
        })
        .collect())
}

/// "Accept all" for "Clearly best file": switches the items `shown` (what the
/// screen showed when the button was pressed) that are still clear cases
/// ([`accept_all_covers`]) to the copy each one showed, as ONE operation, so
/// one Undo restores every link. **Only items in `shown` are ever switched**:
/// one that joined the queue since is left for the next look. An item that
/// is gone from the queue, suggests another copy by now, is refused, or
/// isn't a clear case any more is left as it is and returned with why.
/// Nothing in another group is touched.
///
/// A shown track whose files are due to be ranked again is ranked first, so
/// each item is compared with what the ranking gives now.
pub fn accept_clear_best_files(
    conn: &mut Connection,
    volumes: &impl Volumes,
    shown: &[AcceptItem],
) -> Result<AcceptedAll, Refused> {
    let mut shown: Vec<AcceptItem> = shown.to_vec();
    shown.sort_by_key(|item| (item.recording_id, item.file_id));
    shown.dedup();
    let tracks = serde_json::to_string(&shown.iter().map(|i| i.recording_id).collect::<Vec<_>>())
        .expect("numbers always serialize");
    let due: Vec<i64> = conn
        .prepare_cached(
            "SELECT recording_id FROM best_file_due
             WHERE recording_id IN (SELECT value FROM json_each(?1))",
        )?
        .query_map([&tracks], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    crate::file_roles::settle_tracks(conn, &due)?;

    // The shown items that are still items, with the copy they showed.
    let now: HashSet<(i64, i64)> = clear_best_file_items(conn)?.into_iter().collect();
    let (still, gone): (Vec<AcceptItem>, Vec<AcceptItem>) = shown
        .into_iter()
        .partition(|item| now.contains(&(item.recording_id, item.file_id)));
    let wanted: Vec<(i64, i64)> = still.iter().map(|i| (i.recording_id, i.file_id)).collect();
    let (clear, mut skipped) = sort_out(plan(conn, volumes, &wanted)?);
    skipped.extend(gone.into_iter().map(|item| SkippedItem {
        recording_id: item.recording_id,
        reason: SkipReason::ItemGone,
    }));
    skipped.sort_by_key(|item| item.recording_id);
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
        let mut new_tracks = Vec::new();
        for (p, effect) in &clear {
            switched.push(apply(rec, p, *effect)?);
            if effect.arrives_new {
                new_tracks.push(p.library_track.0);
            }
        }
        rec.detail("tracks", switched.len());
        if !new_tracks.is_empty() {
            rec.detail(SENT_AS_NEW, new_tracks);
        }
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

// ---- relink matches ----------------------------------------------------------

/// What a "Confirm relink" item offers to decide: the file its entry is
/// probably matched to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelinkOffer {
    /// The matched file.
    pub file_id: i64,
    /// The user confirmed this very match before, and the file holds other
    /// audio now: it is asked again.
    pub confirmed_before: bool,
}

// ---- commands ---------------------------------------------------------------

/// Where the drives are, asked off the database's writer.
pub(super) fn mounted(reads: &ReadPool) -> Result<Mounted, DbError> {
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

/// Asks for a relink run: a confirmation, a "not the same" or their undo
/// changed what relink carries from run to run, and the run decides every
/// entry again (it asks for the attach and the roles itself as it ends). A
/// failure to ask is not the action's.
pub(crate) fn ask_for_relink(writer: &Writer, jobs: &JobQueue) {
    if let Err(e) = crate::relink::request(writer, |j| jobs.enqueue(j)) {
        eprintln!("review: couldn't ask for a relink: {e:?}");
    }
}

/// One relink decision on the writer, then the relink run it needs.
fn decide_relink(
    writer: &Writer,
    jobs: &JobQueue,
    decide: impl FnOnce(&mut Connection) -> Result<Option<i64>, crate::relink::decide::Refused>
        + Send
        + 'static,
) -> Result<Acted, IpcError> {
    let operation_id = writer.call(move |c| Ok(decide(c).map_err(Refused::from)))??;
    ask_for_relink(writer, jobs);
    Ok(Acted { operation_id })
}

/// Confirms a probable relink match: the rekordbox entry `entry_id` is the
/// file `file_id`.
#[tauri::command(async)]
#[specta::specta]
pub fn review_confirm_relink(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    entry_id: i64,
    file_id: i64,
) -> Result<Acted, IpcError> {
    decide_relink(&writer, &jobs, move |c| {
        crate::relink::decide::confirm_match(c, entry_id, file_id)
    })
}

/// "Not the same": the rekordbox entry `entry_id` is not the file `file_id`.
#[tauri::command(async)]
#[specta::specta]
pub fn review_reject_relink(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    entry_id: i64,
    file_id: i64,
) -> Result<Acted, IpcError> {
    decide_relink(&writer, &jobs, move |c| {
        crate::relink::decide::reject_match(c, entry_id, file_id)
    })
}

/// Withdraws the confirmation of the rekordbox entry `entry_id`'s Location:
/// the entry is decided again from scratch. No screen calls it yet.
#[tauri::command(async)]
#[specta::specta]
pub fn review_withdraw_relink(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    entry_id: i64,
) -> Result<Acted, IpcError> {
    decide_relink(&writer, &jobs, move |c| {
        crate::relink::decide::withdraw(c, entry_id)
    })
}

/// Dismisses a "Better file available" item, for that one better file.
#[tauri::command(async)]
#[specta::specta]
pub fn review_dismiss_better_file(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    recording_id: i64,
    file_id: i64,
) -> Result<Acted, IpcError> {
    let acted = writer.call(move |c| Ok(dismiss_better_file(c, recording_id, file_id)))??;
    ask_for_follow_up_work(&writer, &jobs);
    Ok(acted)
}

/// "Keep anyway" for a quality problem.
#[tauri::command(async)]
#[specta::specta]
pub fn review_keep_anyway(writer: State<'_, Writer>, recording_id: i64) -> Result<Acted, IpcError> {
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
    let switched = writer.call(move |c| {
        Ok(switch_linked_file(
            c,
            &mounted,
            recording_id,
            file_id,
            effect,
        ))
    })??;
    ask_for_follow_up_work(&writer, &jobs);
    Ok(switched)
}

/// The items "Accept all" would switch now: N on its button is how many.
#[tauri::command]
#[specta::specta]
pub async fn review_accept_all_items(
    reads: State<'_, ReadPool>,
) -> Result<Vec<AcceptItem>, IpcError> {
    let volumes = system_volumes();
    Ok(reads.read(|c| accept_all_items(c, &volumes))?)
}

/// "Accept all" for "Clearly best file", as one operation: `items` are the
/// ones the screen showed, and no other item is switched.
#[tauri::command(async)]
#[specta::specta]
pub fn review_accept_clear_best_files(
    writer: State<'_, Writer>,
    reads: State<'_, ReadPool>,
    jobs: State<'_, JobQueue>,
    items: Vec<AcceptItem>,
) -> Result<AcceptedAll, IpcError> {
    let mounted = mounted(&reads)?;
    let accepted = writer.call(move |c| Ok(accept_clear_best_files(c, &mounted, &items)))??;
    ask_for_follow_up_work(&writer, &jobs);
    Ok(accepted)
}
