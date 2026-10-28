//! "Use this file instead" (1cB-22; ROADMAP 1.11 "Hidden filter", 1.7): on an
//! extra file's row, a button that makes that file the one its track shows.
//! Owner decision, 2026-10-28: for tracks outside the Library only. Switching
//! a Library track's file would make it a new rekordbox entry whose cues stay
//! behind; that is revisited with 2.6.
//!
//! # What is stored
//!
//! A row of `file_choice` (migration 0037), kept by what the file holds: its
//! `audio_hash`, else its `blake3`, and its `blake3` as well when it has one
//! ([`crate::file_roles::choice`] says what a choice covers). The ranking
//! reads it as the first thing it looks at ([`crate::best_file::Step::Chosen`],
//! reason "you chose this file"), so the next ranking run, a rescan and a
//! retag don't undo it, and it survives a move and a music folder removed and
//! added again. When the chosen file is gone, or is encoded again (other
//! audio, so another key), the choice covers nothing and the ranking decides
//! again; the choice stays, and covers the file again if it comes back.
//!
//! # The operation
//!
//! One operation ([`USE_THIS_FILE_OPERATION`]): it takes away the track's
//! earlier choice, if there is one, and adds the new one. Undo takes it back
//! exactly. The track is ranked again when the operation returns, and when
//! its Undo returns ([`crate::ops::undo_for_user`]), so what the lists show
//! is right at once. (`recording_file.role` is derived and is not logged.)
//!
//! # Refused
//!
//! Only an extra file of a track outside the Library, that is on disk, that
//! has something to be kept by (a hash), and that isn't the very same bytes
//! as the file the track shows now (nothing could tell the two apart, so the
//! choice would change nothing): [`Refused`].

use std::collections::HashMap;

use rusqlite::{Connection, Result as SqlResult};
use serde::Serialize;
use serde_json::json;
use specta::Type;

use crate::db::DbError;
use crate::grouping::CONTENT_KEY;
use crate::ipc::{ErrorKind, IpcError};
use crate::ops::{self, OpsError};

/// The operation "Use this file instead" records.
pub const USE_THIS_FILE_OPERATION: &str = "use_this_file";

/// Why a file can't be chosen.
#[derive(Debug)]
pub enum Refused {
    /// It isn't an extra file of a track any more, is hidden for another
    /// reason, or isn't on disk.
    Gone,
    /// Its track is in the Library: the linked file is pinned (ROADMAP 1.7).
    InLibrary,
    /// It has no hash yet, so there is nothing to remember it by.
    NoAudioKnown,
    /// It holds the very same bytes as the file the track shows.
    SameAsShown,
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
            Refused::InLibrary => IpcError::new(ErrorKind::HiddenChoiceInLibrary),
            Refused::NoAudioKnown => IpcError::new(ErrorKind::ReviewNoAudioKnown),
            Refused::Gone | Refused::SameAsShown => IpcError::new(ErrorKind::HiddenChoiceChanged),
            Refused::Ops(e) => e.into(),
        }
    }
}

/// An extra file, and what decides whether it can be chosen.
pub(super) struct Candidate {
    pub recording: i64,
    role: String,
    hidden_reason: Option<String>,
    present: bool,
    pub in_library: bool,
    key: Option<Vec<u8>>,
    blake3: Option<Vec<u8>>,
    shown_key: Option<Vec<u8>>,
    shown_blake3: Option<Vec<u8>>,
    /// The file the track shows now is one the user chose.
    pub shown_is_chosen: bool,
}

impl Candidate {
    /// Why this file can't be chosen, if it can't.
    pub fn refusal(&self) -> Option<Refused> {
        if self.role != "extra" || self.hidden_reason.is_some() || !self.present {
            return Some(Refused::Gone);
        }
        if self.in_library {
            return Some(Refused::InLibrary);
        }
        let Some(key) = &self.key else {
            return Some(Refused::NoAudioKnown);
        };
        // The same audio is told apart by the bytes: with the same bytes, or
        // with no whole-file hash to tell them by, a choice could not pick
        // this file over the one shown.
        let same_audio = self.shown_key.as_ref() == Some(key);
        let told_apart = matches!(
            (&self.blake3, &self.shown_blake3),
            (Some(mine), Some(shown)) if mine != shown
        );
        if same_audio && !told_apart {
            return Some(Refused::SameAsShown);
        }
        None
    }
}

/// The candidates among `files` (a file that is in no track is not one).
pub(super) fn candidates(
    conn: &Connection,
    files: &[i64],
) -> SqlResult<HashMap<i64, Candidate>> {
    let wanted = serde_json::to_string(files).expect("numbers always serialize");
    let key = CONTENT_KEY;
    let shown_key = CONTENT_KEY.replace("f.", "bf.");
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT f.id, rf.recording_id, rf.role, f.hidden_reason, f.present,
                EXISTS (SELECT 1 FROM library_track lt WHERE lt.recording_id = rf.recording_id),
                {key}, f.blake3, {shown_key}, bf.blake3,
                bf.id IS NOT NULL AND bf.present = 1
                    AND EXISTS (SELECT 1 FROM file_choice c WHERE c.content_key = {shown_key})
         FROM file f
         JOIN recording_file rf ON rf.file_id = f.id
         LEFT JOIN recording_file brf
                ON brf.recording_id = rf.recording_id AND brf.role = 'best'
         LEFT JOIN file bf ON bf.id = brf.file_id
         WHERE f.id IN (SELECT value FROM json_each(?1))"
    ))?;
    let rows = stmt.query_map([&wanted], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            Candidate {
                recording: r.get(1)?,
                role: r.get(2)?,
                hidden_reason: r.get(3)?,
                present: r.get(4)?,
                in_library: r.get(5)?,
                key: r.get(6)?,
                blake3: r.get(7)?,
                shown_key: r.get(8)?,
                shown_blake3: r.get(9)?,
                shown_is_chosen: r.get(10)?,
            },
        ))
    })?;
    rows.collect()
}

/// What "Use this file instead" did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Chosen {
    /// The operation to offer Undo for.
    pub operation_id: Option<i64>,
}

/// Makes `file` the file its track shows, as one operation (see the module
/// docs). The track is ranked again before this returns.
pub fn use_this_file(conn: &mut Connection, file: i64) -> Result<Chosen, Refused> {
    let found = candidates(conn, &[file])?;
    let Some(candidate) = found.get(&file) else {
        return Err(Refused::Gone);
    };
    if let Some(refusal) = candidate.refusal() {
        return Err(refusal);
    }
    let recording = candidate.recording;
    let key = candidate.key.clone().unwrap_or_default();
    let blake3 = candidate.blake3.clone().unwrap_or_default();

    let recorded = ops::record(
        conn,
        USE_THIS_FILE_OPERATION,
        &json!({ "fileId": file, "recordingId": recording }),
        |rec| {
            // The track's earlier choice, if any: this one replaces it.
            let earlier: Vec<i64> = rec.read_rows(
                "SELECT c.id FROM file_choice c
                 WHERE EXISTS (SELECT 1 FROM recording_file rf JOIN file f ON f.id = rf.file_id
                               WHERE rf.recording_id = ?1
                                 AND (f.audio_hash = c.content_key OR f.blake3 = c.content_key))
                 ORDER BY c.id",
                [recording],
                |r| r.get(0),
            )?;
            for id in earlier {
                rec.delete("file_choice", id)?;
            }
            rec.insert(
                "file_choice",
                &[
                    ("content_key", rusqlite::types::Value::Blob(key)),
                    ("blake3", rusqlite::types::Value::Blob(blake3)),
                ],
            )?;
            Ok(())
        },
    )?;
    // The choice made the track due (a trigger): ranked again at once. It
    // has committed by now, so a failure is logged and left to the roles
    // job, which finds the track due.
    if let Err(e) = crate::file_roles::settle_tracks(conn, &[recording]) {
        eprintln!("hidden: couldn't rank the track again: {e}");
    }
    Ok(Chosen {
        operation_id: recorded.operation_id,
    })
}
