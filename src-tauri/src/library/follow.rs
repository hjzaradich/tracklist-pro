//! A Library track follows its file (1bE-10; owner decisions 2026-10-12;
//! ROADMAP 1.7, 1.8).
//!
//! A Library track is linked to one file. When that file is gone from disk
//! and a file with **the very same audio** is on disk, the link moves to
//! that file on its own: the user moved or renamed the file, and nothing
//! about the track changed. This is the one place in the app that changes
//! an existing `library_track.linked_file_id` (a test scans the source for
//! any other).
//!
//! # When a link moves
//!
//! All of this must hold ([`follow_files`]):
//!
//! - **The linked file is gone**: `file.present = 0`, which only a walk that
//!   listed the file's folder and didn't find it sets. A drive that isn't
//!   plugged in is never walked, so its files stay present and are never
//!   taken for gone (unplugged isn't missing). While the linked file is on
//!   disk nothing moves, so a file that comes back later moves nothing
//!   back.
//! - **The same audio, known for sure**: the new file's `audio_hash` equals
//!   the gone file's, the new file's hash is current (the hash stage ran at
//!   its size and mtime now), and the gone file's was current when it went.
//!   Another encode never follows, even one of the same length and title:
//!   that is a "Clear best file" item in Review (`better_file`, step
//!   `onDisk`; 1bE-7), and the user's choice.
//! - **The new file is on disk** (`present = 1`), in any folder the app
//!   scans, on any drive.
//! - **It is a file of the track itself**: grouping has put it in the
//!   Library track's own track (files with the same `audio_hash` are one
//!   track). So no file changes track when a link moves, and everything
//!   grouping holds about the linked file stays true: the track's linked
//!   file is one of its files, before and after. A file grouping keeps in
//!   another track isn't followed.
//! - **One track, one file**: no Library track is linked to the new file,
//!   and no other Library track's linked file holds that audio (two Library
//!   tracks that are one recording are the user's to merge or keep, 1bE-9).
//!   Where two tracks could claim one file, nobody follows.
//! - **The automatic follow never changes which rekordbox entry a track is
//!   sent as.** [`crate::send_values::sent_as`] is asked what the track is
//!   sent as now and what it would be sent as linked to the new file. A
//!   track sent as rekordbox's own entry follows only to a file on which it
//!   is still sent as that same entry (relink matched the entry to it, as
//!   the same audio). So when rekordbox's entry is unmatched, sits on
//!   another file, or rekordbox already holds a second entry for the new
//!   file, nothing moves: the track keeps its old link, is still sent as
//!   rekordbox's own entry, and the choice is the user's in Review. A track
//!   rekordbox doesn't know follows on the app's own evidence, and is then
//!   sent as a new track at the new place; not onto a track whose only
//!   rekordbox match is probable, which an add refuses too
//!   ([`super::match_not_confirmed`]). And a follow never takes a file that
//!   another track's entry is matched to, which would leave that track out
//!   of the send.
//!
//! Of several files that qualify, the one the track is sent as rekordbox's
//! entry on, else the track's best file, else the lowest file id.
//!
//! # How it runs
//!
//! As the second step of the relink job ([`crate::relink::job`]), after
//! relink has committed: that job runs after the hash stage, after every
//! grouping run and after every read of rekordbox, which are the moments
//! the evidence above becomes current. Never on the UI path, and never
//! inside relink's transaction. It reads only the database: no file is
//! opened, moved or written.
//!
//! Each follow is one automatic operation in the log
//! ([`crate::ops::AutomaticKind::FollowFile`]) with its own transaction.
//! Undo skips it; undoing an earlier action on the same Library track (its
//! add) takes the follow back with it. Running it again changes nothing: a
//! followed track's linked file is on disk.
//!
//! Cost: one pass over the Library tracks whose linked file is gone, found
//! by primary key from `library_track`, and the files of their own tracks.
//! With no linked file gone it is one query. The send is only asked about
//! the tracks that have a file to follow to.
//!
//! The tests are in `relink/tests/follow*.rs`, beside the job it runs in.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde_json::json;

use super::LibraryTrackId;
use crate::ops::{self, AutomaticKind, OpsError};
use crate::paths::Volumes;
use crate::send_values::{sent_as, SentAs};

/// One link that moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Followed {
    pub library_track: LibraryTrackId,
    /// The linked file before: gone from disk.
    pub from: i64,
    /// The linked file now: the same audio, on disk.
    pub to: i64,
}

/// What a run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// The links that moved, by Library track id.
    pub followed: Vec<Followed>,
    /// Library tracks whose linked file is gone and that have a file of the
    /// same audio in their track, but didn't follow: it would change which
    /// rekordbox entry the track (or another track) is sent as, or the
    /// track's only rekordbox match is probable.
    pub held: u32,
}

/// A file a Library track could follow to.
struct Candidate {
    file: i64,
    /// It's the track's best file.
    best: bool,
}

/// A Library track whose linked file is gone, with the files of the same
/// audio in its own track.
struct Lead {
    library_track: LibraryTrackId,
    recording: i64,
    from: i64,
    candidates: Vec<Candidate>,
}

/// Moves the linked file of every Library track whose file is gone to the
/// same audio at a new place (see the module docs for the rule). Each move
/// is recorded as its own automatic operation; a run with nothing to follow
/// writes nothing. `volumes` only places files for the send's own answer;
/// no file is opened.
///
/// If a write fails, the moves made before it stay (each is whole), the
/// error is returned, and the next run goes on from there.
pub fn follow_files(conn: &mut Connection, volumes: &impl Volumes) -> Result<Summary, OpsError> {
    let leads = leads(conn)?;
    if leads.is_empty() {
        return Ok(Summary::default());
    }

    // What every Library track with a gone linked file is sent as now: the
    // tracks that could follow, and the tracks whose entry a follow mustn't
    // take.
    let gone = gone_tracks(conn)?;
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
    // The rekordbox rows matched to each candidate file, trusted or not.
    let rows_on = rows_matched_to(conn, &leads)?;
    // A candidate another track's entry is matched to is never taken: that
    // entry stands in for the other track's gone file only while no Library
    // track is linked to the file it's matched to.
    let free = |lead: &Lead, file: i64| -> bool {
        rows_on.get(&file).into_iter().flatten().all(|row| {
            entry_of
                .get(row)
                .is_none_or(|owner| *owner == lead.library_track)
        })
    };

    // What each track would be sent as on each candidate, a round of
    // candidates at a time (a fixed number of queries a round).
    let rounds = leads.iter().map(|l| l.candidates.len()).max().unwrap_or(0);
    let mut after: HashMap<(LibraryTrackId, i64), SentAs> = HashMap::new();
    for round in 0..rounds {
        let asked: Vec<(LibraryTrackId, Option<i64>)> = leads
            .iter()
            .filter_map(|lead| {
                let candidate = lead.candidates.get(round)?;
                free(lead, candidate.file).then_some((lead.library_track, Some(candidate.file)))
            })
            .collect();
        if asked.is_empty() {
            continue;
        }
        for ((track, file), sent) in asked.iter().zip(sent_as(conn, volumes, &asked)?) {
            if let Some(file) = file {
                after.insert((*track, *file), sent);
            }
        }
    }

    let mut summary = Summary::default();
    for lead in &leads {
        let was = before
            .get(&lead.library_track)
            .copied()
            .unwrap_or(SentAs::Nothing);
        let Some(to) = target(conn, lead, was, &after)? else {
            summary.held += 1;
            continue;
        };
        if record(conn, lead, to)? {
            summary.followed.push(Followed {
                library_track: lead.library_track,
                from: lead.from,
                to,
            });
        }
    }
    Ok(summary)
}

/// The file `lead`'s track follows to, if any: see the module docs.
fn target(
    conn: &Connection,
    lead: &Lead,
    was: SentAs,
    after: &HashMap<(LibraryTrackId, i64), SentAs>,
) -> rusqlite::Result<Option<i64>> {
    // In order: the track's best file, then the lowest file id.
    let mut order: Vec<&Candidate> = lead.candidates.iter().collect();
    order.sort_by_key(|c| (!c.best, c.file));
    let on = |file: i64| after.get(&(lead.library_track, file)).copied();
    if let SentAs::Entry(entry) = was {
        // Sent as rekordbox's own entry: only to a file on which it still
        // is that entry.
        return Ok(order
            .iter()
            .map(|c| c.file)
            .find(|&file| on(file) == Some(SentAs::Entry(entry))));
    }
    // rekordbox doesn't know the track (or its entry can't be told): a file
    // rekordbox holds an entry for first.
    if let Some(file) = order
        .iter()
        .map(|c| c.file)
        .find(|&file| matches!(on(file), Some(SentAs::Entry(_))))
    {
        return Ok(Some(file));
    }
    let Some(file) = order
        .iter()
        .map(|c| c.file)
        .find(|&file| on(file).is_some())
    else {
        return Ok(None);
    };
    // It would go to rekordbox as a new track. Not beside an entry that is
    // probably its own: that waits for the match to be confirmed, as an add
    // does (ROADMAP 1.8).
    if on(file) == Some(SentAs::New) && super::match_not_confirmed(conn, lead.recording)? {
        return Ok(None);
    }
    Ok(Some(file))
}

/// Moves one link, as one automatic operation. The evidence is checked
/// again inside the operation's own transaction, so the write is only made
/// on what the database holds then. Returns whether the link moved.
fn record(conn: &mut Connection, lead: &Lead, to: i64) -> Result<bool, OpsError> {
    let (track, from) = (lead.library_track.0, lead.from);
    let details = json!({ "libraryTrackId": track, "from": from, "to": to });
    let recorded = ops::record_automatic(conn, AutomaticKind::FollowFile, &details, |rec| {
        let still: Vec<i64> = rec.read_rows(
            "SELECT lt.id FROM library_track lt
             JOIN file a ON a.id = lt.linked_file_id
             JOIN recording_file rf ON rf.recording_id = lt.recording_id AND rf.file_id = ?3
             JOIN file n ON n.id = rf.file_id
             WHERE lt.id = ?1 AND lt.linked_file_id = ?2
               AND a.present = 0 AND n.present = 1 AND n.audio_hash = a.audio_hash
               AND NOT EXISTS (SELECT 1 FROM library_track x
                               WHERE x.linked_file_id = n.id OR x.source_file_id = n.id)",
            [track, from, to],
            |r| r.get(0),
        )?;
        if still.is_empty() {
            return Ok(false);
        }
        rec.set("library_track", track, "linked_file_id", to)
    })?;
    Ok(recorded.value)
}

/// Every linked Library track whose linked file is gone.
fn gone_tracks(conn: &Connection) -> rusqlite::Result<Vec<LibraryTrackId>> {
    conn.prepare_cached(
        "SELECT lt.id FROM library_track lt JOIN file a ON a.id = lt.linked_file_id
         WHERE lt.kind = 'linked' AND a.present = 0
         ORDER BY lt.id",
    )?
    .query_map([], |r| r.get(0).map(LibraryTrackId))?
    .collect()
}

/// The Library tracks that have a file to follow to, by id, each with its
/// candidates by file id: the evidence query. A candidate is a present file
/// of the Library track's own track with the gone linked file's
/// `audio_hash`, both hashes current, that no Library track points at; and
/// only where no other Library track's linked file holds that audio.
fn leads(conn: &Connection) -> rusqlite::Result<Vec<Lead>> {
    let gone_current = crate::relink::hash_current_of("a");
    let new_current = crate::relink::hash_current_of("n");
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT lt.id, lt.recording_id, a.id, n.id, rf.role = 'best'
         FROM library_track lt
         JOIN file a ON a.id = lt.linked_file_id
         JOIN recording_file rf ON rf.recording_id = lt.recording_id
         JOIN file n ON n.id = rf.file_id
         WHERE lt.kind = 'linked'
           AND a.present = 0 AND length(a.audio_hash) > 0 AND {gone_current}
           AND n.present = 1 AND n.id <> a.id AND n.audio_hash = a.audio_hash
           AND {new_current}
           AND NOT EXISTS (SELECT 1 FROM library_track x
                           WHERE x.linked_file_id = n.id OR x.source_file_id = n.id)
           AND NOT EXISTS (SELECT 1 FROM file xf
                           JOIN library_track x ON x.linked_file_id = xf.id
                           WHERE xf.audio_hash = a.audio_hash AND x.id <> lt.id)
         ORDER BY lt.id, n.id"
    ))?;
    let rows = stmt.query_map([crate::relink::hash_version()], |r| {
        Ok((
            LibraryTrackId(r.get(0)?),
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            Candidate {
                file: r.get(3)?,
                best: r.get(4)?,
            },
        ))
    })?;
    let mut leads: Vec<Lead> = Vec::new();
    for row in rows {
        let (library_track, recording, from, candidate) = row?;
        match leads.last_mut() {
            Some(lead) if lead.library_track == library_track => lead.candidates.push(candidate),
            _ => leads.push(Lead {
                library_track,
                recording,
                from,
                candidates: vec![candidate],
            }),
        }
    }
    Ok(leads)
}

/// The rekordbox rows matched to each candidate file (trusted or probable).
fn rows_matched_to(conn: &Connection, leads: &[Lead]) -> rusqlite::Result<HashMap<i64, Vec<i64>>> {
    let files: HashSet<i64> = leads
        .iter()
        .flat_map(|l| l.candidates.iter().map(|c| c.file))
        .collect();
    let wanted = serde_json::to_string(&files.into_iter().collect::<Vec<_>>())
        .expect("numbers always serialize");
    let mut rows_on: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut stmt = conn.prepare_cached(
        "SELECT file_id, id FROM rekordbox_track
         WHERE file_id IN (SELECT value FROM json_each(?1))",
    )?;
    for row in stmt.query_map([&wanted], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (file, id): (i64, i64) = row?;
        rows_on.entry(file).or_default().push(id);
    }
    Ok(rows_on)
}
