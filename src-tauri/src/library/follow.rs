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
//! - **One track, one file** ([`NO_OTHER_TRACK_ON_THE_AUDIO`]): no other
//!   Library track's linked file holds that audio. That covers the new file
//!   itself (a file a Library track is linked to is never followed to) and
//!   two Library tracks that are one recording, which are the user's to
//!   merge or keep (1bE-9). Where two tracks could claim one file, nobody
//!   follows.
//! - **The automatic follow never changes which rekordbox entry a track is
//!   sent as.** Two checks, both needed:
//!   - *No other entry on the new file.* No rekordbox entry other than the
//!     one the track is sent as today may be matched to the new file,
//!     whatever the match is (trusted, probable or confirmed). So when
//!     rekordbox already holds a second entry for the moved file, nothing
//!     moves, whichever of the two a send would pick; and a track rekordbox
//!     doesn't know never follows onto a file rekordbox holds any entry
//!     for. Nor does a follow ever take a file another track's entry is
//!     matched to.
//!   - *The same answer from the send.* [`crate::send_values::sent_as`] is
//!     asked what the track is sent as now and what it would be sent as
//!     linked to the new file. A track sent as rekordbox's own entry
//!     follows only where it is still sent as that entry: relink has
//!     matched the entry to the new file, and trusts the match. While the
//!     entry is unmatched, only probable there, or on another file, nothing
//!     moves.
//!
//!   Where nothing moves the track keeps its old link, is still sent as
//!   rekordbox's own entry, and the choice is the user's in Review. A track
//!   rekordbox doesn't know follows on the app's own evidence, and is then
//!   sent as a new track at the new place; not while a file of its track is
//!   the probable match of a rekordbox entry, where an add is refused too
//!   ([`super::match_not_confirmed`]).
//!
//! Of several files that qualify, the track's best file, else the lowest
//! file id.
//!
//! # How it runs
//!
//! As the second step of the relink job ([`crate::relink::job`]), after
//! relink has committed: that job runs after the hash stage, after every
//! grouping run and after every read of rekordbox, which are the moments
//! the evidence above becomes current. Never on the UI path, and never
//! inside relink's transaction. It reads only the database: no file is
//! opened, moved or written, and no drive is asked anything here. The job
//! asks where the drives are before it calls this, off the database's
//! writer, and hands the answer in ([`crate::relink::Mounted`]).
//!
//! Each follow is one automatic operation in the log
//! ([`crate::ops::AutomaticKind::FollowFile`]) with its own transaction.
//! Undo skips it; undoing an earlier action on the same Library track (its
//! add) takes the follow back with it. Running it again changes nothing: a
//! followed track's linked file is on disk.
//!
//! Cost: one pass over the Library tracks whose linked file is gone, found
//! by primary key from `library_track`, and the files of their own tracks.
//! With no linked file gone it is one query. The send is only asked when a
//! track has a file to follow to.
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
    /// same audio in their track, but didn't follow: another rekordbox
    /// entry is matched to that file, the track wouldn't be sent as the
    /// entry it is sent as now, or a file of the track is the probable
    /// match of an entry.
    pub held: u32,
}

/// One track, one file, as SQL over a Library track `lt` and its gone
/// linked file `a`: no other Library track's linked file holds `a`'s audio.
/// The one guard against two Library tracks on one file, asked in one place
/// ([`leads`]). A file a Library
/// track is linked to holds its own audio, so a candidate (which holds
/// `a`'s) that is linked is refused by it too; and so are two Library
/// tracks that are the same recording, whichever of them the file is in.
const NO_OTHER_TRACK_ON_THE_AUDIO: &str = "NOT EXISTS (
        SELECT 1 FROM file xf JOIN library_track x ON x.linked_file_id = xf.id
        WHERE xf.audio_hash = a.audio_hash AND x.id <> lt.id)";

/// A Library track whose linked file is gone, with the files of the same
/// audio in its own track.
struct Lead {
    library_track: LibraryTrackId,
    recording: i64,
    from: i64,
    /// The files it could follow to: the track's best file first, then by
    /// file id.
    candidates: Vec<i64>,
}

/// Moves the linked file of every Library track whose file is gone to the
/// same audio at a new place (see the module docs for the rule). Each move
/// is recorded as its own automatic operation; a run with nothing to follow
/// writes nothing.
///
/// `volumes` says where each drive is, for the send's own answer; no file
/// is opened. It is asked on the thread this runs on, which is the
/// database's writer: hand in an answer taken beforehand
/// ([`crate::relink::Mounted`]), never something that asks the system.
///
/// If a write fails, the moves made before it stay (each is whole), the
/// error is returned, and the next run goes on from there.
pub fn follow_files(conn: &mut Connection, volumes: &impl Volumes) -> Result<Summary, OpsError> {
    let leads = leads(conn)?;
    if leads.is_empty() {
        return Ok(Summary::default());
    }

    // What every Library track with a gone linked file is sent as now, in
    // one send: the entry each of the tracks here is on.
    let gone = gone_tracks(conn)?;
    let asked: Vec<(LibraryTrackId, Option<i64>)> = gone.iter().map(|id| (*id, None)).collect();
    let before: HashMap<LibraryTrackId, SentAs> = gone
        .iter()
        .copied()
        .zip(sent_as(conn, volumes, &asked)?)
        .collect();
    // The rekordbox rows matched to each candidate file, whatever the match.
    let rows_on = rows_matched_to(conn, &leads)?;

    // Each track's candidates no other entry is matched to: only the entry
    // the track is sent as today may be on the file.
    let clear: Vec<Vec<i64>> = leads
        .iter()
        .map(|lead| {
            let own = match before.get(&lead.library_track) {
                Some(SentAs::Entry(row)) => Some(*row),
                _ => None,
            };
            lead.candidates
                .iter()
                .copied()
                .filter(|file| {
                    rows_on
                        .get(file)
                        .into_iter()
                        .flatten()
                        .all(|row| Some(*row) == own)
                })
                .collect()
        })
        .collect();

    // What each track would be sent as on each of those, a round of
    // candidates at a time (a fixed number of queries a round).
    let rounds = clear.iter().map(Vec::len).max().unwrap_or(0);
    let mut after: HashMap<(LibraryTrackId, i64), SentAs> = HashMap::new();
    for round in 0..rounds {
        let asked: Vec<(LibraryTrackId, Option<i64>)> = leads
            .iter()
            .zip(&clear)
            .filter_map(|(lead, clear)| Some((lead.library_track, Some(*clear.get(round)?))))
            .collect();
        for ((track, file), sent) in asked.iter().zip(sent_as(conn, volumes, &asked)?) {
            if let Some(file) = file {
                after.insert((*track, *file), sent);
            }
        }
    }

    let mut summary = Summary::default();
    for (lead, clear) in leads.iter().zip(&clear) {
        let was = before
            .get(&lead.library_track)
            .copied()
            .unwrap_or(SentAs::Nothing);
        let Some(to) = target(conn, lead, was, clear, &after)? else {
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

/// The file `lead`'s track follows to, if any, among `clear` (its
/// candidates no other entry is matched to, the best file first): see the
/// module docs.
fn target(
    conn: &Connection,
    lead: &Lead,
    was: SentAs,
    clear: &[i64],
    after: &HashMap<(LibraryTrackId, i64), SentAs>,
) -> rusqlite::Result<Option<i64>> {
    let on = |file: i64| after.get(&(lead.library_track, file)).copied();
    if let SentAs::Entry(entry) = was {
        // Sent as rekordbox's own entry: only to a file on which it still
        // is that entry.
        return Ok(clear
            .iter()
            .copied()
            .find(|&file| on(file) == Some(SentAs::Entry(entry))));
    }
    // rekordbox has no entry for the track, and none for the file: it
    // stays a track rekordbox doesn't know.
    let Some(file) = clear
        .iter()
        .copied()
        .find(|&file| matches!(on(file), Some(SentAs::New | SentAs::Nothing)))
    else {
        return Ok(None);
    };
    // It would go to rekordbox as a new track. Not beside an entry that is
    // probably its own (the probable match of another file of the track):
    // that waits for the match to be confirmed, as an add does (ROADMAP
    // 1.8).
    if on(file) == Some(SentAs::New) && super::match_not_confirmed(conn, lead.recording)? {
        return Ok(None);
    }
    Ok(Some(file))
}

/// Moves one link, as one automatic operation. What the write itself rests
/// on is checked again inside the operation's own transaction: the link is
/// still on the gone file, and the new file is a file of the track, on
/// disk, with the same audio. (Whether another Library track is on that
/// audio is asked once, in [`leads`]: a follow never changes the audio a
/// track is linked to, so no follow of this run can change that answer.)
/// Returns whether the link moved.
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
               AND a.present = 0 AND n.present = 1 AND n.audio_hash = a.audio_hash",
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
/// candidates (the track's best file first, then by file id): the evidence
/// query. A candidate is a present file of the Library track's own track
/// with the gone linked file's `audio_hash`, both hashes current; and only
/// where no other Library track's linked file holds that audio.
fn leads(conn: &Connection) -> rusqlite::Result<Vec<Lead>> {
    let gone_current = crate::relink::hash_current_of("a");
    let new_current = crate::relink::hash_current_of("n");
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT lt.id, lt.recording_id, a.id, n.id
         FROM library_track lt
         JOIN file a ON a.id = lt.linked_file_id
         JOIN recording_file rf ON rf.recording_id = lt.recording_id
         JOIN file n ON n.id = rf.file_id
         WHERE lt.kind = 'linked'
           AND a.present = 0 AND length(a.audio_hash) > 0 AND {gone_current}
           AND n.present = 1 AND n.id <> a.id AND n.audio_hash = a.audio_hash
           AND {new_current}
           AND {NO_OTHER_TRACK_ON_THE_AUDIO}
         ORDER BY lt.id, rf.role = 'best' DESC, n.id"
    ))?;
    let rows = stmt.query_map([crate::relink::hash_version()], |r| {
        Ok((
            LibraryTrackId(r.get(0)?),
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
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
        .flat_map(|l| l.candidates.iter().copied())
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
