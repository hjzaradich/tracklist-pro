//! Intake: which files become Inbox items, and in which batch.
//!
//! It hooks two places and adds no job:
//! - [`after_walk`], once the walk's finish is committed: the first complete
//!   walk of a new Inbox folder puts what is already there into one
//!   `already_there` batch ([`baseline`]), and online-only files get their
//!   items ([`online_only`]). A walk that skipped an unreadable folder takes
//!   no baseline; the next complete one does.
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
//! it without the user's opt-in, so no hash job runs for it and there is no
//! second look to wait for; the walk that found it gives it an item.
//! Its listing counts. Nothing here opens a file in any case.
//!
//! `.crdownload`, `.part` and `.tmp` files are never indexed (their
//! extensions aren't audio's), so the walk never gives them a `file` row,
//! and so they never get an item.

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::jobs::{JobContext, JobError, JobHandler};
use crate::scan::MusicFolderId;

/// What the walk's finish does for an Inbox folder, after the walk's own
/// bookkeeping is committed: the [`baseline`], then items for online-only
/// files ([`online_only`]). Best effort. A failure is logged and the
/// walk stays recorded; the next walk tries again. `complete`: the walk
/// listed everything (no unreadable folder or file).
pub(crate) fn after_walk(conn: &mut Connection, folder: MusicFolderId, complete: bool) {
    let result = baseline(conn, folder, complete).and_then(|_| online_only(conn, folder));
    if let Err(e) = result {
        eprintln!("inbox intake after a walk failed: {e}");
    }
}

/// Takes the baseline of an Inbox folder: puts the files already there into
/// one `already_there` batch, on its first complete walk. A walk that could
/// not list everything takes none, so what it missed is in the baseline of
/// a later walk and does not count as an arrival then. The batch (empty, if
/// the folder had no files) is the record that the baseline is taken. A
/// folder that isn't an Inbox folder, or has its baseline, does nothing.
/// Returns how many items it made.
pub(crate) fn baseline(
    conn: &mut Connection,
    folder: MusicFolderId,
    complete: bool,
) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let inbox: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM music_folder WHERE id = ?1 AND role = 'inbox')",
        [folder.0],
        |r| r.get(0),
    )?;
    if !inbox || !complete || taken(&tx, folder.0)? {
        return Ok(0);
    }
    tx.execute(
        "INSERT INTO inbox_batch (music_folder_id, kind) VALUES (?1, 'already_there')",
        [folder.0],
    )?;
    let batch = tx.last_insert_rowid();
    let made = tx.execute(
        "INSERT INTO inbox_item (file_id, batch_id)
         SELECT id, ?2 FROM file
         WHERE music_folder_id = ?1 AND present = 1
           AND id NOT IN (SELECT file_id FROM inbox_item)",
        (folder.0, batch),
    )?;
    tx.commit()?;
    Ok(made)
}

/// Whether the folder has its baseline.
fn taken(conn: &Connection, folder: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM inbox_batch
                        WHERE music_folder_id = ?1 AND kind = 'already_there')",
        [folder],
        |r| r.get(0),
    )
}

/// Makes items for the files in Inbox folders that have arrived since:
/// present, not yet in the Inbox, and holding still (see the module doc).
/// Files that came in together, in the same folder, form one batch: a
/// `folder` batch for those in a subfolder, a `loose` one for the rest.
/// Returns how many items it made.
pub fn run(conn: &mut Connection) -> rusqlite::Result<usize> {
    make(conn, None, false)
}

/// [`run`] for the online-only files of one folder. Nothing reads them, so
/// no hash job runs for them; the walk that found them does this.
pub(crate) fn online_only(conn: &mut Connection, folder: MusicFolderId) -> rusqlite::Result<usize> {
    make(conn, Some(folder.0), true)
}

fn make(conn: &mut Connection, folder: Option<i64>, online_only: bool) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    // A folder without its baseline has no arrivals yet.
    let arrived: Vec<(i64, i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT f.id, f.music_folder_id, f.rel_path
             FROM file f JOIN music_folder m ON m.id = f.music_folder_id
             WHERE m.role = 'inbox'
               AND (?1 IS NULL OR f.music_folder_id = ?1)
               AND EXISTS (SELECT 1 FROM inbox_batch b
                           WHERE b.music_folder_id = m.id AND b.kind = 'already_there')
               AND f.present = 1 AND f.hidden_reason IS NULL
               AND NOT EXISTS (SELECT 1 FROM inbox_item i WHERE i.file_id = f.id)
               AND (f.online_only = 1
                    OR (?2 = 0 AND EXISTS (SELECT 1 FROM file_stage s
                               WHERE s.file_id = f.id AND s.stage = 'hash'
                                 AND s.status = 'done'
                                 AND s.size IS f.size AND s.mtime IS f.mtime)))
             ORDER BY f.music_folder_id, f.rel_path",
        )?;
        let rows = stmt.query_map((folder, online_only), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // One batch per music folder and subfolder (`None`: the folder itself).
    let mut groups: BTreeMap<(i64, Option<String>), Vec<i64>> = BTreeMap::new();
    for (file, folder, rel) in arrived {
        let dir = rel
            .rsplit_once('/')
            .map(|(dir, _)| dir.to_owned())
            .filter(|dir| !dir.is_empty());
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
/// stopped: the files it did hash have held still either way. Intake runs
/// when the whole hash job ends, not after each batch of files.
///
/// Intake is a side effect and never changes the hash job's result: if it
/// fails, the failure is logged and the job's own result stands, so the
/// chain goes on as it would have.
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
        if let Err(e) = job.writer().call(run) {
            eprintln!("inbox intake after a hash job failed: {e}");
        }
        result
    }
}
