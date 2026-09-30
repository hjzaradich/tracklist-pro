//! The unchanged check (1aC-1, ROADMAP 1.1, §5.1): is a file the walk sees
//! again the same file, with the same content?
//!
//! rekordbox bumps the mtime of every file it adds or analyzes, and cloud
//! sync and backups touch files too, so a new mtime alone doesn't mean new
//! audio. Before its upsert, the walk compares what the `file` row stored
//! with what the listing says:
//!
//! | The listing, against the row | Verdict |
//! |---|---|
//! | Same size, mtime and file id | [`Change::Same`]: nothing to do. |
//! | Both file ids known and different, and the size or mtime differs too | [`Change::Replaced`]: another file under the same name. Its `file_stage` rows are dropped, so every stage redoes it. |
//! | Both file ids known and different, but size and mtime match | [`Change::Reissued`]: restores, sync tools and network shares hand out new ids for identical files, so it's decided by the partial hash like a touch: equal keeps the stages (the row just takes the new id); different, or nothing to compare, drops them like a replaced file. |
//! | The size differs | [`Change::Resized`]: a normal change; the stages redo it (their rows record the old size). |
//! | Only the mtime differs | [`Change::Touched`]: decided by the partial hash. |
//!
//! **The partial hash** ([`crate::hash::partial`]) is stored by the hash
//! stage, which reads the whole file anyway, so the first walk of a file
//! opens nothing. For a touched (or reissued) file that has one stored, the walk works
//! out the partial hash of the file as it is now: equal means the content is
//! unchanged, so the walk moves the row's mtime and the recorded mtime of
//! every `file_stage` row that was current at the old size and mtime, and
//! no stage redoes the file ([`Verdict::TouchedOnly`]). Anything else
//! (a different hash, none stored because the hash stage hasn't run or the
//! format has no shortcut, or a file that couldn't be read) is a change
//! ([`Verdict::Changed`]), and the stored hash is cleared: it's the old
//! content's, and the hash stage stores the new one when it next reads the
//! file. The trade-offs are in [`crate::hash::partial`].
//!
//! Reads go through [`ReadGate`], like a stage's: never a file the listing
//! says is online only (unless the user opted in), and the file is asked
//! about again just before it's opened, since OneDrive can turn a local file
//! into a placeholder at any time.

use std::fs::File;
use std::path::Path;

use rusqlite::Connection;

use super::online_only::ReadGate;
use super::walk::Found;
use crate::hash::partial::{self, PartialHash};

/// What a `file` row stored the last time the walk saw the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stored {
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub file_id: Option<String>,
    pub partial_hash: Option<Vec<u8>>,
}

/// How a file the listing shows differs from its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    /// Same size, mtime and file id.
    Same,
    /// Only the mtime differs.
    Touched,
    /// The size differs.
    Resized,
    /// Both file ids are known and differ, and so does the size or mtime:
    /// not the file that was indexed.
    Replaced,
    /// Both file ids are known and differ, but the size and mtime match:
    /// probably the same content under a new id, which the partial hash
    /// tells.
    Reissued,
}

/// Compares the row with the listing. A file id counts only when both are
/// known: a row from before file ids were stored, or a lookup that failed
/// (some FAT and network drives report none), can't say the file was
/// replaced, so size and mtime decide.
pub(crate) fn compare(stored: &Stored, found: &Found) -> Change {
    let replaced = match (&stored.file_id, &found.file_id) {
        (Some(was), Some(now)) => was != now,
        _ => false,
    };
    let same_stat = stored.size == Some(found.size) && stored.mtime == Some(found.mtime_ns);
    if replaced && same_stat {
        Change::Reissued
    } else if replaced {
        Change::Replaced
    } else if stored.size != Some(found.size) {
        Change::Resized
    } else if stored.mtime != Some(found.mtime_ns) {
        Change::Touched
    } else {
        Change::Same
    }
}

/// Whether the walk reads the file to decide: it's touched or reissued, and
/// there's a stored partial hash to compare with. A file that's the same is
/// never read, and one that changed is a change whatever it holds now.
pub(crate) fn wants_read(change: Change, stored: &Stored) -> bool {
    matches!(change, Change::Touched | Change::Reissued) && stored.partial_hash.is_some()
}

/// What the walk decides for a file it saw before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Nothing changed.
    Unchanged,
    /// Only the mtime moved, and the content is the same.
    TouchedOnly,
    /// Only the file id moved, and the content is the same: the stages'
    /// rows already match the file's size and mtime, so they stand.
    NewIdOnly,
    /// A change: the stages redo the file. `replaced` says it's another file
    /// under the same name.
    Changed { replaced: bool },
}

/// The verdict on a file that's `change` from its row, given the partial
/// hash of it now (`None` if it wasn't read, has no shortcut, or couldn't
/// be read).
pub(crate) fn verdict(change: Change, stored: &Stored, now: Option<&PartialHash>) -> Verdict {
    match change {
        Change::Same => Verdict::Unchanged,
        Change::Touched => match (&stored.partial_hash, now) {
            (Some(was), Some(now)) if was.as_slice() == now.as_slice() => Verdict::TouchedOnly,
            _ => Verdict::Changed { replaced: false },
        },
        Change::Reissued => match (&stored.partial_hash, now) {
            (Some(was), Some(now)) if was.as_slice() == now.as_slice() => Verdict::NewIdOnly,
            _ => Verdict::Changed { replaced: true },
        },
        Change::Resized => Verdict::Changed { replaced: false },
        Change::Replaced => Verdict::Changed { replaced: true },
    }
}

/// What `folder`'s rows at `rels` stored, in the same order; `None` for a
/// file the index doesn't have yet.
pub(crate) fn stored(
    conn: &Connection,
    folder: super::MusicFolderId,
    rels: &[String],
) -> rusqlite::Result<Vec<Option<Stored>>> {
    rels.iter()
        .map(|rel| stored_one(conn, folder, rel).map(|s| s.map(|(_, stored)| stored)))
        .collect()
}

/// The row at `rel` in `folder`: its id and what it stored.
pub(crate) fn stored_one(
    conn: &Connection,
    folder: super::MusicFolderId,
    rel: &str,
) -> rusqlite::Result<Option<(i64, Stored)>> {
    use rusqlite::OptionalExtension;
    conn.prepare_cached(
        "SELECT id, size, mtime, file_id, partial_hash FROM file
         WHERE music_folder_id = ?1 AND rel_path = ?2",
    )?
    .query_row((folder.0, rel), |r| {
        Ok((
            r.get(0)?,
            Stored {
                size: r.get(1)?,
                mtime: r.get(2)?,
                file_id: r.get(3)?,
                partial_hash: r.get(4)?,
            },
        ))
    })
    .optional()
}

/// Whether the walk may open the file at `path` (a `\\?\` path) now:
/// `online_only` is what the listing said. `false` for an online-only file
/// the user hasn't opted in to reading, and for one that can't be asked
/// about (gone since the listing, no access).
pub(crate) fn may_read(gate: &ReadGate, path: &Path, online_only: bool) -> bool {
    gate.allows(online_only) && gate.may_open(path).unwrap_or(false)
}

/// The partial hash of the file at `path` as it is now, read-only; `None`
/// if it can't be opened or read, or has no shortcut. Call [`may_read`]
/// first.
pub(crate) fn read_now(path: &Path) -> Option<PartialHash> {
    // `File::open` asks for read access only.
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    partial::partial_hash(&mut file, len).ok().flatten()
}
