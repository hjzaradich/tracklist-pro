//! Intake: which files become Inbox items, and in which batch.
//!
//! It hooks two places and adds no job:
//! - [`baseline`], in the walk's finish transaction: the first walk of a new
//!   Inbox folder puts what is already there into one `already_there`
//!   batch.
//! - [`after_hash`] wraps the hash job: when it ends, [`run`] makes items
//!   for the files that have held still since.
//!
//! # Holding still
//!
//! A file isn't counted as arrived until its size and modified time were
//! the same on two looks. The first look is the walk's (its listing is
//! stored in the `file` row). The second is the hash stage's, which opens
//! the file and compares its size and modified time with the walk's, before
//! and after reading all of it (`hash/job.rs`); a file that moved on is left
//! unrecorded and is looked at again on the next walk. So a `file_stage`
//! row for `hash`, `done`, at the `file` row's size and mtime is the proof,
//! and nothing new is stored. A copy still growing never gets one; the
//! watcher's next burst (or any later scan) walks again and the hash stage
//! tries again.
//!
//! An online-only (OneDrive) placeholder is the exception: nothing reads
//! it without the user's opt-in, so there is no second look to wait for.
//! Its listing counts. Nothing here opens a file in any case.
//!
//! `.crdownload`, `.part` and `.tmp` files are never indexed (their
//! extensions aren't audio's), so the walk never gives them a `file` row,
//! and so they never get an item.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension};

use crate::jobs::{JobContext, JobError, JobHandler};
use crate::scan::MusicFolderId;

/// Puts the files already in an Inbox folder into one `already_there`
/// batch, if this is its first walk (it has never finished one). Called in
/// the transaction that finishes the walk, before the folder is marked as
/// walked. A folder that isn't an Inbox folder, or was walked before, does
/// nothing. Returns how many items it made.
pub(crate) fn baseline(conn: &Connection, folder: MusicFolderId) -> rusqlite::Result<usize> {
    let first_walk: Option<bool> = conn
        .query_row(
            "SELECT walked_at IS NULL FROM music_folder WHERE id = ?1 AND role = 'inbox'",
            [folder.0],
            |r| r.get(0),
        )
        .optional()?;
    if first_walk != Some(true) {
        return Ok(0);
    }
    let there = "SELECT id FROM file WHERE music_folder_id = ?1 AND present = 1
                   AND id NOT IN (SELECT file_id FROM inbox_item)";
    let any: bool = conn.query_row(&format!("SELECT EXISTS ({there})"), [folder.0], |r| {
        r.get(0)
    })?;
    if !any {
        return Ok(0);
    }
    // A first walk that was cut short and redone finds its batch again.
    let batch: Option<i64> = conn
        .query_row(
            "SELECT id FROM inbox_batch WHERE music_folder_id = ?1 AND kind = 'already_there'",
            [folder.0],
            |r| r.get(0),
        )
        .optional()?;
    let batch = match batch {
        Some(id) => id,
        None => {
            conn.execute(
                "INSERT INTO inbox_batch (music_folder_id, kind) VALUES (?1, 'already_there')",
                [folder.0],
            )?;
            conn.last_insert_rowid()
        }
    };
    conn.execute(
        &format!("INSERT INTO inbox_item (file_id, batch_id) SELECT id, ?2 FROM ({there})"),
        (folder.0, batch),
    )
}

/// Makes items for the files in Inbox folders that have arrived since:
/// present, not yet in the Inbox, and holding still (see the module doc).
/// Files that came in together, in the same folder, form one batch: a
/// `folder` batch for those in a subfolder, a `loose` one for the rest.
/// Returns how many items it made.
pub fn run(conn: &mut Connection) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    // A folder whose first walk hasn't finished has no baseline yet: its
    // files are not arrivals.
    let arrived: Vec<(i64, i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT f.id, f.music_folder_id, f.rel_path
             FROM file f JOIN music_folder m ON m.id = f.music_folder_id
             WHERE m.role = 'inbox' AND m.walked_at IS NOT NULL
               AND f.present = 1 AND f.hidden_reason IS NULL
               AND NOT EXISTS (SELECT 1 FROM inbox_item i WHERE i.file_id = f.id)
               AND (f.online_only = 1
                    OR EXISTS (SELECT 1 FROM file_stage s
                               WHERE s.file_id = f.id AND s.stage = 'hash'
                                 AND s.status = 'done'
                                 AND s.size IS f.size AND s.mtime IS f.mtime))
             ORDER BY f.music_folder_id, f.rel_path",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // One batch per music folder and subfolder (`None`: the folder itself).
    let mut groups: BTreeMap<(i64, Option<String>), Vec<i64>> = BTreeMap::new();
    for (file, folder, rel) in arrived {
        let dir = rel.rsplit_once('/').map(|(dir, _)| dir.to_owned());
        groups.entry((folder, dir)).or_default().push(file);
    }
    let mut made = 0;
    for ((folder, dir), files) in groups {
        let kind = if dir.is_some() { "folder" } else { "loose" };
        tx.execute(
            "INSERT INTO inbox_batch (music_folder_id, kind, label) VALUES (?1, ?2, ?3)",
            (folder, kind, dir),
        )?;
        let batch = tx.last_insert_rowid();
        let mut insert =
            tx.prepare_cached("INSERT INTO inbox_item (file_id, batch_id) VALUES (?1, ?2)")?;
        for file in files {
            made += insert.execute((file, batch))?;
        }
    }
    tx.commit()?;
    Ok(made)
}

/// `hasher` with [`run`] after it, whether the hash job finished or was
/// stopped: the files it did hash have held still either way.
pub fn after_hash<H: JobHandler>(hasher: H) -> Intaking<H> {
    Intaking { inner: hasher }
}

/// See [`after_hash`].
pub struct Intaking<H> {
    inner: H,
}

impl<H: JobHandler> JobHandler for Intaking<H> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let result = self.inner.run(job);
        job.writer().call(run)?;
        result
    }
}
