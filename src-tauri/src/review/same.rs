//! Two Library tracks that are the same recording (1bE-9; ROADMAP 1.4,
//! 1.12): the "Same track twice in your Library" item. The grouping job
//! never merges two Library tracks; it keeps them apart and records the
//! pair (`both_in_library`), and the user decides here: keep both, or merge
//! them into one, choosing which stays. Each is one operation of the log.
//!
//! # Keep both
//!
//! [`keep_both`] remembers "apart" between every two of the item's Library
//! tracks ([`crate::grouping::remember`], kept by what the files hold, as
//! the grouping keeps every decision), so the pair isn't asked about again
//! and is never merged. A copy outside the Library in the same set then
//! joins neither track and isn't asked about either: the grouping pass
//! settles its pairs with the set (owner decision, 2026-10-12).
//!
//! # Merge
//!
//! [`merge`] folds every other Library track of the item into the one the
//! user chose. The track that stays keeps its own cues, grid and play count
//! (the entry it is sent as stays). Where the tracks hold different values
//! of a field and one is the user's (their edits, rekordbox's rating, color
//! and comments), the user is asked field by field, and the staying track's
//! value is chosen by default; a value only one track has carries over
//! (1cB-20, owner decision 2026-10-28, [`super::merge_fields`]). The values
//! taken become the staying track's own. For each track that leaves, in one
//! operation:
//!
//! 1. The rekordbox entry it is sent as today ([`sent_as`]), or for a track
//!    a send wrote that no read has shown yet, the `Location` it was sent
//!    to, is recorded (`replaced_entry`, reason `merge`): it goes on the
//!    list of tracks to remove in rekordbox ([`crate::after_send`]). A track
//!    rekordbox never held leaves nothing there.
//! 2. Its crate entries go to the track that stays, each with its own kind;
//!    where a crate holds both, the staying track's own entry wins and the
//!    other is deleted.
//! 3. Its sync bases, conflicts and the app's own values are deleted, and
//!    its Library track.
//! 4. Its files move into the track that stays, starting over there (their
//!    role is the ranking's, worked out again at once).
//! 5. Its track is kept, with no files, while the merge stands
//!    (`track_merge`), so Undo has a track to move the files back into.
//!
//! Then, outside the log (derived state): the moved files' rekordbox
//! entries point at the track that stays, a grouping pass over the item's
//! own files runs (a copy outside the Library joins the merged track), and
//! the tracks are ranked. So the queue is right as the action returns; the
//! grouping job does the rest of the library.
//!
//! **Undo** puts every row back and runs the grouping pass over the item's
//! files and the ranking again ([`after_undo`]): the two tracks and their
//! crates are back and the pair is an open item again. It is refused, with
//! nothing changed, once the track that stayed is linked to another file
//! ([`merge_link_changed_since`]): it followed its file to one the merge
//! moved, or was switched. It works after a send too, as undoing a
//! removal does: a merge writes nothing a send changes, and the next send
//! sends the track that came back again (foreman's ruling, 2026-10-16).
//!
//! Which track the item suggests keeping is said in one place,
//! [`suggest_keep`].

use std::collections::BTreeSet;

use rusqlite::types::Value;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tauri::State;

use super::actions::{ask_for_follow_up_work, mounted, rank_again, Acted, Refused};
use super::held;
use super::merge_fields::{self, Member, MergeChoice, MergeField};
use crate::conflicts::app_values;
use crate::db::{ReadPool, Writer};
use crate::grouping::{remember, Verdict, CONTENT_KEY};
use crate::ipc::IpcError;
use crate::jobs::JobQueue;
use crate::library::LibraryTrackId;
use crate::ops::{self, OpsError, Recorder};
use crate::paths::Volumes;
use crate::rekordbox::location::decode;
use crate::send_values::{sent_as, SentAs};

/// The operation kinds, as the operation log stores them.
pub const KEEP_BOTH_OPERATION: &str = "keep_both_library_tracks";
pub const MERGE_OPERATION: &str = "merge_library_tracks";

/// What rekordbox holds for a Library track: the entry it is sent as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EntryFacts {
    /// How many cues (hot cues and memory cues).
    pub cues: u32,
    /// Stars, 0 to 5.
    pub stars: u32,
    pub plays: u32,
}

/// One Library track of a "Same track twice" item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SameRecordingTrack {
    pub recording_id: i64,
    pub library_track_id: LibraryTrackId,
    pub linked_file_id: Option<i64>,
    /// `None`: rekordbox doesn't know the track (it is sent as a new one).
    pub rekordbox: Option<EntryFacts>,
    /// How many crates hold it.
    pub crates: u32,
}

/// What a "Same track twice" item offers: its Library tracks, the one the
/// app suggests keeping first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SameRecordingOffer {
    pub tracks: Vec<SameRecordingTrack>,
    /// The track the app suggests keeping in a merge ([`suggest_keep`]).
    pub keep: Option<i64>,
    /// Two of the tracks are linked as versions: versions never merge, so
    /// the item offers "Keep both" only, and says why.
    pub linked_versions: bool,
}

// ---- which track to keep ------------------------------------------------------

/// What [`suggest_keep`] weighs about one Library track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepCandidate {
    pub recording: i64,
    /// The cues of the entry it is sent as (0 for a track rekordbox
    /// doesn't know).
    pub cues: u32,
    pub plays: u32,
    /// Where its best-ranked file stands in the ranking of the item's
    /// files (0 is first).
    pub rank: usize,
}

/// Why a track is suggested for keeping, against the runner-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepWhy {
    MoreCues {
        cues: u32,
        other: u32,
    },
    MorePlays {
        plays: u32,
        other: u32,
    },
    /// The same cues and plays; its best file ranks ahead of the runner-up's
    /// (`other`).
    BetterFile {
        other: i64,
    },
    /// Nothing tells them apart: the older track.
    Tied,
}

/// The track a merge should keep, and why: **the one place that says it**
/// (owner decision, 2026-10-16). The most cues
/// first, because the track that stays keeps its own cues and the other's
/// are lost; then the most plays; then the one holding the best-ranked file;
/// then the older track. `None` for fewer than two tracks.
pub fn suggest_keep(candidates: &[KeepCandidate]) -> Option<(i64, KeepWhy)> {
    if candidates.len() < 2 {
        return None;
    }
    let mut sorted = candidates.to_vec();
    sorted.sort_by_key(|c| {
        (
            std::cmp::Reverse(c.cues),
            std::cmp::Reverse(c.plays),
            c.rank,
            c.recording,
        )
    });
    let (top, next) = (sorted[0], sorted[1]);
    let why = if top.cues != next.cues {
        KeepWhy::MoreCues {
            cues: top.cues,
            other: next.cues,
        }
    } else if top.plays != next.plays {
        KeepWhy::MorePlays {
            plays: top.plays,
            other: next.plays,
        }
    } else if top.rank != next.rank {
        KeepWhy::BetterFile {
            other: next.recording,
        }
    } else {
        KeepWhy::Tied
    };
    Some((top.recording, why))
}

// ---- the item -----------------------------------------------------------------

/// The Library tracks of the "Same track twice" item named by `anchor`, in
/// track order, or `None` if it isn't in the queue.
pub(crate) fn library_tracks_of(
    conn: &Connection,
    anchor: i64,
) -> rusqlite::Result<Option<Vec<i64>>> {
    let Some(set) = held::components(conn)?
        .into_iter()
        .find(|c| c.anchor == anchor && c.same_recording)
    else {
        return Ok(None);
    };
    let json = serde_json::to_string(&set.tracks).expect("numbers always serialize");
    let mut tracks: Vec<i64> = conn
        .prepare_cached(
            "SELECT recording_id FROM library_track
             WHERE recording_id IN (SELECT value FROM json_each(?1)) ORDER BY recording_id",
        )?
        .query_map([json], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    tracks.dedup();
    Ok(Some(tracks))
}

/// The item's Library tracks, if it is still the item the screen showed
/// (`shown`, its Library tracks then): else [`Refused::ItemGone`].
fn still_the_item(conn: &Connection, anchor: i64, shown: &[i64]) -> Result<Vec<i64>, Refused> {
    let now = library_tracks_of(conn, anchor)?.ok_or(Refused::ItemGone)?;
    let shown: BTreeSet<i64> = shown.iter().copied().collect();
    if now.len() < 2 || now.iter().copied().collect::<BTreeSet<_>>() != shown {
        return Err(Refused::ItemGone);
    }
    Ok(now)
}

/// A Library track as the item and the merge need it.
struct Track {
    recording: i64,
    library_track: LibraryTrackId,
    linked_file: Option<i64>,
    last_sent_location: Option<String>,
    sent: SentAs,
}

/// Each of `recordings`' Library track, with what it is sent as today.
fn tracks(
    conn: &Connection,
    volumes: &impl Volumes,
    recordings: &[i64],
) -> rusqlite::Result<Vec<Track>> {
    let mut found = Vec::new();
    for &recording in recordings {
        let (id, linked, last_sent): (i64, Option<i64>, Option<String>) = conn
            .prepare_cached(
                "SELECT id, linked_file_id, last_sent_location FROM library_track
                 WHERE recording_id = ?1",
            )?
            .query_row([recording], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        found.push(Track {
            recording,
            library_track: LibraryTrackId(id),
            linked_file: linked,
            last_sent_location: last_sent,
            sent: SentAs::Nothing,
        });
    }
    let asked: Vec<(LibraryTrackId, Option<i64>)> =
        found.iter().map(|t| (t.library_track, None)).collect();
    for (track, sent) in found.iter_mut().zip(sent_as(conn, volumes, &asked)?) {
        track.sent = sent;
    }
    Ok(found)
}

/// What the entry `row` holds: its cues, stars and plays.
fn entry_facts(conn: &Connection, row: i64) -> rusqlite::Result<EntryFacts> {
    conn.prepare_cached(
        "SELECT json_array_length(position_marks), coalesce(rating, 0), coalesce(play_count, 0)
         FROM rekordbox_track WHERE id = ?1",
    )?
    .query_row([row], |r| {
        let rating: i64 = r.get(1)?;
        Ok(EntryFacts {
            cues: r.get::<_, i64>(0)?.clamp(0, i64::from(u32::MAX)) as u32,
            // rekordbox writes 0, 51, 102, 153, 204 or 255.
            stars: ((rating.clamp(0, 255) + 25) / 51) as u32,
            plays: r.get::<_, i64>(2)?.clamp(0, i64::from(u32::MAX)) as u32,
        })
    })
}

/// The Library tracks of the "Same track twice" item named by `anchor`,
/// with what rekordbox holds for each and how many crates hold it. The
/// suggestion is the comparison's ([`super::compare`]), which ranks files.
pub(crate) fn offer_tracks(
    conn: &Connection,
    volumes: &impl Volumes,
    anchor: i64,
) -> rusqlite::Result<Vec<SameRecordingTrack>> {
    let Some(recordings) = library_tracks_of(conn, anchor)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for track in tracks(conn, volumes, &recordings)? {
        let rekordbox = match track.sent {
            SentAs::Entry(row) => Some(entry_facts(conn, row)?),
            SentAs::New | SentAs::Nothing => None,
        };
        let crates: i64 = conn
            .prepare_cached(
                "SELECT count(*) FROM crate_entry
                 WHERE library_track_id = ?1 AND kind <> 'never_include'",
            )?
            .query_row([track.library_track.0], |r| r.get(0))?;
        out.push(SameRecordingTrack {
            recording_id: track.recording,
            library_track_id: track.library_track,
            linked_file_id: track.linked_file,
            rekordbox,
            crates: crates as u32,
        });
    }
    Ok(out)
}

// ---- keep both ------------------------------------------------------------------

/// A file of each track whose audio is known, the linked file first: what
/// the decision is remembered by. [`Refused::NoAudioKnown`] if a track has
/// none.
pub(super) fn known_files(conn: &Connection, recordings: &[i64]) -> Result<Vec<i64>, Refused> {
    let mut files = Vec::new();
    for &recording in recordings {
        let file: Option<i64> = conn
            .prepare_cached(&format!(
                "SELECT f.id FROM recording_file rf JOIN file f ON f.id = rf.file_id
                 LEFT JOIN library_track lt ON lt.linked_file_id = f.id AND lt.recording_id = ?1
                 WHERE rf.recording_id = ?1 AND length({CONTENT_KEY}) > 0
                 ORDER BY lt.id IS NULL, f.id LIMIT 1"
            ))?
            .query_map([recording], |r| r.get(0))?
            .next()
            .transpose()?;
        files.push(file.ok_or(Refused::NoAudioKnown)?);
    }
    Ok(files)
}

/// "Keep both" for the "Same track twice" item named by `anchor`, whose
/// Library tracks the screen showed as `shown`: every two of them are
/// remembered as kept apart, as one operation, and the grouping pass runs
/// so the item leaves the queue as the action returns. Undo takes the
/// decision back and the item returns ([`after_undo`]).
pub fn keep_both(conn: &mut Connection, anchor: i64, shown: &[i64]) -> Result<Acted, Refused> {
    let recordings = still_the_item(conn, anchor, shown)?;
    let files = known_files(conn, &recordings)?;
    let details = json!({ "recordingIds": recordings, "tracks": recordings.len() });
    let recorded = ops::record(conn, KEEP_BOTH_OPERATION, &details, |rec| {
        for (i, &a) in files.iter().enumerate() {
            for &b in &files[i + 1..] {
                // Every file has known audio and each is in a track of its
                // own, so `remember` has nothing to refuse.
                let _ = remember(rec, a, b, Verdict::Apart)?;
            }
        }
        Ok(())
    })?;
    regroup(conn, &recordings);
    Ok(Acted {
        operation_id: recorded.operation_id,
    })
}

// ---- merge ----------------------------------------------------------------------

/// What a merge did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Merged {
    pub operation_id: Option<i64>,
    /// The track that stays.
    pub kept: i64,
    /// The tracks merged into it.
    pub merged: Vec<i64>,
}

/// The rekordbox entry a leaving track leaves behind: its `Location` as
/// rekordbox writes it, and its match key.
fn left_behind(conn: &Connection, track: &Track) -> rusqlite::Result<Option<(String, String)>> {
    let entry: Option<(String, String)> = match track.sent {
        SentAs::Entry(row) => conn
            .prepare_cached("SELECT location, location_key FROM rekordbox_track WHERE id = ?1")?
            .query_map([row], |r| Ok((r.get(0)?, r.get(1)?)))?
            .next()
            .transpose()?,
        // Sent by the app, and no read holds it (yet): rekordbox may hold
        // it at the Location it was sent to.
        SentAs::New | SentAs::Nothing => track.last_sent_location.as_ref().and_then(|location| {
            decode(location)
                .ok()
                .map(|decoded| (location.clone(), decoded.match_key()))
        }),
    };
    Ok(entry.filter(|(location, key)| !location.is_empty() && !key.is_empty()))
}

/// Writes one leaving track's part of the merge into `keep`, as changes of
/// the operation `rec` records.
fn fold(
    rec: &mut Recorder<'_>,
    keep: &Track,
    leaving: &Track,
    entry: Option<(String, String)>,
) -> Result<(), OpsError> {
    // 1. The entry it leaves behind in rekordbox, once per Location.
    if let Some((location, key)) = entry {
        let recorded = !rec
            .read_rows(
                "SELECT 1 FROM replaced_entry WHERE location_key = ?1",
                [&key],
                |r| r.get::<_, i64>(0),
            )?
            .is_empty();
        if !recorded {
            rec.insert(
                "replaced_entry",
                &[
                    ("recording_id", Value::Integer(leaving.recording)),
                    ("location", Value::Text(location)),
                    ("location_key", Value::Text(key)),
                    ("reason", Value::Text("merge".to_owned())),
                ],
            )?;
        }
    }
    // 2. Its crate entries: to the track that stays, unless the crate holds
    //    that one already (its own entry wins).
    let entries: Vec<(i64, bool)> = rec.read_rows(
        "SELECT e.id, EXISTS (SELECT 1 FROM crate_entry k
                              WHERE k.crate_id = e.crate_id AND k.library_track_id = ?2)
         FROM crate_entry e WHERE e.library_track_id = ?1 ORDER BY e.id",
        [leaving.library_track.0, keep.library_track.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    for (entry, crate_holds_keep) in entries {
        if crate_holds_keep {
            rec.delete("crate_entry", entry)?;
        } else {
            rec.set(
                "crate_entry",
                entry,
                "library_track_id",
                keep.library_track.0,
            )?;
        }
    }
    // 3. Its sync bases, sent playlist entries, conflicts, the app's own
    //    values and Library track.
    for table in ["sync_base", "sent_entry", "conflict", "app_value"] {
        let rows: Vec<i64> = rec.read_rows(
            &format!("SELECT id FROM {table} WHERE library_track_id = ?1 ORDER BY id"),
            [leaving.library_track.0],
            |r| r.get(0),
        )?;
        for row in rows {
            rec.delete(table, row)?;
        }
    }
    rec.delete("library_track", leaving.library_track.0)?;
    // 4. Its files, into the track that stays. The role first: a track has
    //    one best file, and the ranking works it out again there.
    let files: Vec<i64> = rec.read_rows(
        "SELECT file_id FROM recording_file WHERE recording_id = ?1 ORDER BY file_id",
        [leaving.recording],
        |r| r.get(0),
    )?;
    for file in files {
        move_file(rec, file, keep.recording)?;
    }
    // 5. The track, kept with no files while the merge stands.
    rec.insert(
        "track_merge",
        &[
            ("recording_id", Value::Integer(leaving.recording)),
            ("into_id", Value::Integer(keep.recording)),
        ],
    )?;
    Ok(())
}

/// Moves `file` into the track `to`, as changes of the operation `rec`
/// records: **the one place an action moves a file between tracks** (a
/// merge here; "these are the same track" and a split,
/// [`super::split`]). The file starts over in its new track: its role and
/// match confidence are the ranking's (derived columns, `ops::DERIVED_FIELDS`),
/// set first because a track has one best file, and worked out again once
/// the operation, or its Undo, is made. The caller checks what the grouping's
/// own move checks (`grouping::store::relocate`: no file a Library track
/// points at, no move between linked versions; the database's triggers
/// refuse the second too).
pub(super) fn move_file(rec: &mut Recorder<'_>, file: i64, to: i64) -> Result<(), OpsError> {
    rec.set("recording_file", file, "role", "undecided")?;
    rec.set(
        "recording_file",
        file,
        "match_confidence",
        Option::<f64>::None,
    )?;
    rec.set("recording_file", file, "recording_id", to)?;
    Ok(())
}

/// [`merge_choosing`] with no choice made: every field takes its default,
/// the staying track's value ([`super::merge_fields`]).
pub fn merge(
    conn: &mut Connection,
    volumes: &impl Volumes,
    anchor: i64,
    shown: &[i64],
    keep: i64,
) -> Result<Merged, Refused> {
    merge_choosing(conn, volumes, anchor, shown, keep, &[])
}

/// The Library tracks of the item to merge into `keep`, the staying one
/// first, as the item still is.
fn members(
    conn: &Connection,
    volumes: &impl Volumes,
    anchor: i64,
    shown: &[i64],
    keep: i64,
) -> Result<(Vec<Track>, Vec<Member>), Refused> {
    let recordings = still_the_item(conn, anchor, shown)?;
    if !recordings.contains(&keep) {
        return Err(Refused::ItemGone);
    }
    if linked_versions(conn, &recordings)? {
        return Err(Refused::LinkedVersions);
    }
    let mut all = tracks(conn, volumes, &recordings)?;
    let at = all
        .iter()
        .position(|t| t.recording == keep)
        .ok_or(Refused::ItemGone)?;
    let staying = all.remove(at);
    all.insert(0, staying);
    let members = all
        .iter()
        .map(|t| Member {
            recording: t.recording,
            library_track: t.library_track,
        })
        .collect();
    Ok((all, members))
}

/// What a merge into `keep` asks the user (1cB-20): one row per field where
/// the item's tracks hold different values and at least one is the user's
/// ([`super::merge_fields`]). Refused as [`merge`] is when the item isn't the
/// one the screen showed.
pub fn merge_questions(
    conn: &Connection,
    volumes: &impl Volumes,
    anchor: i64,
    shown: &[i64],
    keep: i64,
) -> Result<Vec<MergeField>, Refused> {
    let (_, members) = members(conn, volumes, anchor, shown, keep)?;
    Ok(merge_fields::plan(conn, volumes, &members)?.fields())
}

/// Merges the "Same track twice" item named by `anchor` (whose Library
/// tracks the screen showed as `shown`) into the track `keep`, one of them,
/// as one operation: the confirmed, undoable action CLAUDE.md asks of
/// deleting a Library track. See the module docs for what it writes.
///
/// `choices` are the values the user picked where the tracks differ
/// ([`merge_questions`]); a field not named takes the staying track's value.
/// The values the staying track takes become its own values, in the same
/// operation, so Undo restores both tracks and their values exactly.
///
/// Refused, changing nothing, when the item isn't the one the screen showed
/// or `keep` isn't one of its Library tracks ([`Refused::ItemGone`]), when a
/// choice isn't of a question the screen showed (also
/// [`Refused::ItemGone`]), and when two of its tracks are linked as versions
/// (versions never merge; [`Refused::LinkedVersions`]).
///
/// `volumes` says where each drive is, for what each track is sent as; no
/// file is opened. On the database's writer, hand in an answer taken
/// beforehand ([`crate::relink::Mounted`]).
pub fn merge_choosing(
    conn: &mut Connection,
    volumes: &impl Volumes,
    anchor: i64,
    shown: &[i64],
    keep: i64,
    choices: &[MergeChoice],
) -> Result<Merged, Refused> {
    let (mut all, members) = members(conn, volumes, anchor, shown, keep)?;
    let recordings: Vec<i64> = all.iter().map(|t| t.recording).collect();
    let writes = merge_fields::plan(conn, volumes, &members)?.writes(choices)?;
    let staying = all.remove(0);
    let mut leaving = Vec::with_capacity(all.len());
    for track in all {
        let entry = left_behind(conn, &track)?;
        leaving.push((track, entry));
    }
    let merged: Vec<i64> = leaving.iter().map(|(t, _)| t.recording).collect();
    // What the staying track is linked to now: its Undo is refused once that
    // changed ([`merge_link_changed_since`]).
    let details = json!({
        "keep": keep,
        "recordingIds": merged,
        "tracks": merged.len(),
        "keepLibraryTrack": staying.library_track.0,
        "keepLinked": staying.linked_file,
        "valuesTaken": writes.len(),
    });
    // One writer call: nothing changes between the plan and the writes.
    let recorded = ops::record_with_derived(
        conn,
        MERGE_OPERATION,
        &details,
        |rec| {
            for (track, entry) in leaving {
                fold(rec, &staying, &track, entry)?;
            }
            // The values the user chose from the tracks that leave: the
            // staying track's own from now on (the engine's store, 1.10),
            // chosen against what rekordbox shows now.
            for write in &writes {
                app_values::set_chosen(
                    rec,
                    staying.library_track,
                    &write.field,
                    &write.value,
                    write.rekordbox.clone(),
                )?;
            }
            Ok(())
        },
        // The moved files' rekordbox entries follow their track (grouping
        // owns that column; the pass below would do it too).
        crate::grouping::sync_rekordbox,
    )?;
    regroup(conn, &recordings);
    rank_again(conn, &[keep]);
    Ok(Merged {
        operation_id: recorded.operation_id,
        kept: keep,
        merged,
    })
}

// ---- after the action, after its undo ----------------------------------------

/// Whether two of `recordings` are linked as versions: versions never
/// merge, so such an item offers "Keep both" only.
pub(crate) fn linked_versions(conn: &Connection, recordings: &[i64]) -> rusqlite::Result<bool> {
    let json = serde_json::to_string(recordings).expect("numbers always serialize");
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM version_link
                        WHERE recording_a IN (SELECT value FROM json_each(?1))
                          AND recording_b IN (SELECT value FROM json_each(?1)))",
    )?
    .query_row([&json], |r| r.get(0))
}

/// A grouping pass over the item's files only
/// ([`crate::grouping::regroup_files`]: the files of `recordings` and every
/// file that could join a set with them), so the pairs stand where the
/// decision or the merge puts them, and a copy outside the Library joins a
/// merged track, as the action returns. Nothing wider: the grouping job
/// does that ([`ask_for_merge_follow_up`]). Derived state: if it fails, the
/// action stands and the grouping job does it.
pub(super) fn regroup(conn: &mut Connection, recordings: &[i64]) {
    let files = files_of(conn, recordings);
    let done = files.and_then(|files| crate::grouping::regroup_files(conn, &files));
    if let Err(e) = done {
        eprintln!("review: couldn't run the grouping pass: {e}");
    }
}

/// The files of `recordings`.
pub(super) fn files_of(conn: &Connection, recordings: &[i64]) -> rusqlite::Result<Vec<i64>> {
    let json = serde_json::to_string(recordings).expect("numbers always serialize");
    conn.prepare_cached(
        "SELECT file_id FROM recording_file
         WHERE recording_id IN (SELECT value FROM json_each(?1)) ORDER BY file_id",
    )?
    .query_map([&json], |r| r.get(0))?
    .collect()
}

/// Whether the Undo of the merge `operation` must be refused because the
/// track that stayed is linked to another file since: it followed its file
/// to one the merge moved, or was switched (foreman's ruling, 2026-10-16).
/// Taking the merge back then would put the moved files back in the other
/// track while the staying one plays one of them: two Library tracks on one
/// file. So: refused when a file the merge moved is the linked file of a
/// Library track now, or when the staying track's linked file isn't the one
/// it had at the merge (or its Library track is gone). Asked by the log
/// before anything is unwound ([`crate::ops`]); `false` for any other
/// operation.
pub(crate) fn merge_link_changed_since(
    conn: &Connection,
    operation: i64,
) -> rusqlite::Result<bool> {
    conn.prepare_cached(&format!(
        "SELECT EXISTS (
             SELECT 1 FROM operation o
             WHERE o.id = ?1 AND o.kind = '{MERGE_OPERATION}'
               AND (EXISTS (SELECT 1 FROM change c
                            JOIN library_track lt ON lt.linked_file_id = c.entity_id
                            WHERE c.operation_id = o.id AND c.entity = 'recording_file'
                              AND c.field = 'recording_id')
                    OR NOT EXISTS (SELECT 1 FROM library_track k
                                   WHERE k.id = json_extract(o.details, '$.keepLibraryTrack')
                                     AND k.linked_file_id
                                         IS json_extract(o.details, '$.keepLinked'))))"
    ))?
    .query_row([operation], |r| r.get(0))
}

/// Whether `kind` is one of this module's operations.
pub fn is_operation(kind: &str) -> bool {
    kind == KEEP_BOTH_OPERATION || kind == MERGE_OPERATION
}

/// After the Undo of a "Keep both" or a merge (`operation`, of `kind`): the
/// grouping pass over the item's files, so the pair is an open item again
/// as the undo returns (the ranking follows in
/// [`crate::ops::undo_for_user`]). Nothing for any other operation.
pub(crate) fn after_undo(conn: &mut Connection, operation: i64, kind: &str) {
    if !is_operation(kind) {
        return;
    }
    let recordings: rusqlite::Result<Vec<i64>> = conn
        .prepare_cached(
            "SELECT value FROM operation o, json_each(o.details, '$.recordingIds')
             WHERE o.id = ?1
             UNION SELECT json_extract(o.details, '$.keep') FROM operation o
             WHERE o.id = ?1 AND json_extract(o.details, '$.keep') IS NOT NULL",
        )
        .and_then(|mut stmt| {
            stmt.query_map([operation], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()
        });
    match recordings {
        Ok(recordings) => regroup(conn, &recordings),
        Err(e) => eprintln!("review: couldn't read what the undone operation was about: {e}"),
    }
}

/// Asks for what "Keep both", a merge or their undo leave to the
/// background: a whole grouping pass, the roles, the search index and the
/// rekordbox analysis (attach).
pub(crate) fn ask_for_merge_follow_up(writer: &Writer, jobs: &JobQueue) {
    ask_for_follow_up_work(writer, jobs);
    // The grouping pass for the rest of the library: the action only
    // settled the item's own files.
    if let Err(e) = crate::grouping::start(jobs, writer) {
        eprintln!("review: couldn't ask for the grouping job: {e:?}");
    }
    if let Err(e) = crate::attach::request(writer, |j| jobs.enqueue(j)) {
        eprintln!("review: couldn't ask for attach: {e:?}");
    }
}

// ---- commands -------------------------------------------------------------------

/// "Keep both" for a "Same track twice" item: `recording_ids` are the
/// Library tracks the screen showed.
#[tauri::command(async)]
#[specta::specta]
pub fn review_keep_both(
    writer: State<'_, Writer>,
    jobs: State<'_, JobQueue>,
    anchor: i64,
    recording_ids: Vec<i64>,
) -> Result<Acted, IpcError> {
    let acted = writer.call(move |c| Ok(keep_both(c, anchor, &recording_ids)))??;
    ask_for_merge_follow_up(&writer, &jobs);
    Ok(acted)
}

/// The fields a merge of a "Same track twice" item into `keep` asks about
/// (1cB-20): `recording_ids` are the Library tracks the screen showed.
#[tauri::command(async)]
#[specta::specta]
pub fn review_merge_questions(
    reads: State<'_, ReadPool>,
    anchor: i64,
    recording_ids: Vec<i64>,
    keep: i64,
) -> Result<Vec<MergeField>, IpcError> {
    let mounted = mounted(&reads)?;
    reads
        .read(|c| Ok(merge_questions(c, &mounted, anchor, &recording_ids, keep)))?
        .map_err(IpcError::from)
}

/// Merges a "Same track twice" item into `keep`: `recording_ids` are the
/// Library tracks the screen showed, `choices` the values picked where the
/// tracks differ.
#[tauri::command(async)]
#[specta::specta]
pub fn review_merge(
    writer: State<'_, Writer>,
    reads: State<'_, ReadPool>,
    jobs: State<'_, JobQueue>,
    anchor: i64,
    recording_ids: Vec<i64>,
    keep: i64,
    choices: Vec<MergeChoice>,
) -> Result<Merged, IpcError> {
    let mounted = mounted(&reads)?;
    let merged = writer.call(move |c| {
        Ok(merge_choosing(
            c,
            &mounted,
            anchor,
            &recording_ids,
            keep,
            &choices,
        ))
    })??;
    ask_for_merge_follow_up(&writer, &jobs);
    Ok(merged)
}
