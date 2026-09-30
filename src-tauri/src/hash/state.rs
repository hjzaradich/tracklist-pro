//! Which files the hash job works on, and where its results go: the one
//! place the job touches the database. The job is stage `hash` in
//! `file_stage` ([`crate::scan_state`]), at version [`DEFINITION`], so
//! bumping the audio_hash definition hashes every file again.
//!
//! How each file is recorded:
//! - hashed, with an audio_hash: `done`;
//! - hashed, with no audio_hash: `failed`, with the [`Skip`] reason
//!   (`unknown_format`, `malformed`, …). It has a blake3; it isn't retried
//!   until the file changes or the definition does;
//! - couldn't be opened or read: skipped, `unreachable` (tried again next
//!   run);
//! - OneDrive online-only: skipped, `online_only`;
//! - changed since the walk, or on an unplugged drive: nothing, since the
//!   next walk or run sees it afresh.
//!
//! A result is written only if the `file` row still has the size and
//! modified time the file was hashed at, in the same transaction as its
//! `file_stage` row. The file's partial hash (1aC-1, [`super::partial`])
//! goes in with it: the walk compares against it when only the file's
//! modified time moves later.
//!
//! OneDrive online-only files (1aB-8) go through [`ReadGate`], made once
//! per job: a row the walk marked online only is skipped unless the user
//! opted in, and [`ReadGate::may_open`] is asked again just before a file
//! is opened.

use rusqlite::Connection;

use super::partial::PartialHash;
use super::{Skip, AUDIO_HASH_LEN, DEFINITION};
use crate::scan::MusicFolderId;
pub(crate) use crate::scan::ReadGate;
use crate::scan_state::{self, DueFile, Outcome, Recorded, Scope, Stage};

/// The stage this job records, and its version.
const STAGE: Stage = Stage::Hash;

fn version() -> i64 {
    i64::from(DEFINITION)
}

/// A file the job should hash, as the walk last saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Due {
    pub id: i64,
    pub folder: MusicFolderId,
    /// From the music folder, `/`-separated, in the on-disk spelling.
    pub rel_path: String,
    pub size: Option<i64>,
    /// Nanoseconds since the Unix epoch.
    pub mtime: Option<i64>,
    /// False for a OneDrive online-only file the user didn't opt in to.
    pub may_read: bool,
}

/// What became of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Result {
    Hashed {
        blake3: [u8; 32],
        /// The stored audio_hash, or why there's none.
        audio: std::result::Result<[u8; AUDIO_HASH_LEN], Skip>,
        /// The partial hash, if the file has one ([`super::partial`]).
        /// Stored with the hashes; `None` clears an older one.
        partial: Option<PartialHash>,
    },
    /// Not tried: the reason, e.g. [`scan_state::UNREACHABLE`].
    Skipped(&'static str),
}

/// One file's result, with the stat it was hashed at (the row's, checked
/// against the open file before and after reading).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub id: i64,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub result: Result,
}

impl Record {
    pub(crate) fn of(due: &Due, result: Result) -> Record {
        Record {
            id: due.id,
            size: due.size,
            mtime: due.mtime,
            result,
        }
    }
}

fn scope(folders: Option<&[MusicFolderId]>) -> Scope {
    match folders {
        Some(ids) => Scope::Folders(ids.iter().map(|id| id.0).collect()),
        None => Scope::All,
    }
}

/// How many files in `folders` (or everywhere) are due, for progress.
pub(crate) fn count_due(
    conn: &Connection,
    folders: Option<&[MusicFolderId]>,
) -> rusqlite::Result<u64> {
    scan_state::count_due(conn, STAGE, version(), &scope(folders))
}

/// Up to `limit` due files with ids above `after`, in id order. Within a
/// run, pass the last id back as `after`; never start again from 0.
pub(crate) fn due(
    conn: &Connection,
    gate: &ReadGate,
    folders: Option<&[MusicFolderId]>,
    after: i64,
    limit: usize,
) -> rusqlite::Result<Vec<Due>> {
    let files = scan_state::due(conn, STAGE, version(), &scope(folders), after, limit)?;
    Ok(files
        .into_iter()
        .map(|f: DueFile| Due {
            may_read: gate.allows(f.online_only),
            id: f.id,
            folder: MusicFolderId(f.music_folder_id),
            rel_path: f.rel_path,
            size: f.size,
            mtime: f.mtime,
        })
        .collect())
}

/// Writes `records` and their `file_stage` rows in one transaction. A
/// hashed file whose row changed size or modified time since (a walk ran
/// meanwhile) is left alone: it's due again anyway.
pub(crate) fn record(conn: &mut Connection, records: &[Record]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let mut stages = Vec::with_capacity(records.len());
    {
        let mut update = tx.prepare_cached(
            "UPDATE file SET blake3 = ?2, audio_hash = ?3, partial_hash = ?6
             WHERE id = ?1 AND size IS ?4 AND mtime IS ?5",
        )?;
        for r in records {
            let outcome = match &r.result {
                Result::Hashed {
                    blake3,
                    audio,
                    partial,
                } => {
                    let stored = audio.as_ref().ok().map(|a| &a[..]);
                    let partial = partial.as_ref().map(|p| &p[..]);
                    let row = (r.id, &blake3[..], stored, r.size, r.mtime, partial);
                    if update.execute(row)? == 0 {
                        continue;
                    }
                    match audio {
                        Ok(_) => Outcome::Done,
                        Err(skip) => Outcome::Failed(skip.as_str()),
                    }
                }
                Result::Skipped(reason) => Outcome::Skipped(reason),
            };
            stages.push(Recorded {
                file: r.id,
                size: r.size,
                mtime: r.mtime,
                outcome,
            });
        }
    }
    scan_state::record(&tx, STAGE, version(), &stages)?;
    tx.commit()
}
