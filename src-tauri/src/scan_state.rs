//! Which files each later scan stage still has to do (ROADMAP 1.1 stages
//! 2 and 3), kept in `file_stage` (migration 0007).
//!
//! A stage (reading tags, hashing, fingerprinting) records, for every file
//! it finished, failed or skipped, the file's size and mtime as it saw them
//! and the stage's own version. A file is **due** for a stage when it's
//! present and:
//! - the stage has no row for it yet,
//! - its size or mtime differs from the row's (rekordbox rewrites tags and
//!   bumps mtimes, §5.1),
//! - the row's version differs from the stage's current one (a bumped
//!   version redoes every file), or
//! - the stage skipped it last time (e.g. an online-only OneDrive file):
//!   a skip means the stage never tried, so the next run decides again.
//!
//! A failed file is not due again until it changes, so a file whose content
//! a stage can't process isn't retried on every run.
//!
//! **Failed is only for content.** Record [`Outcome::Failed`] only when the
//! stage read the file and can't process what it found (e.g. a decoder
//! error). A file the stage couldn't reach or open (an unplugged drive, a
//! locked file, no permission, a OneDrive hiccup, gone since the walk)
//! isn't failed: nothing about it is known, and its size and mtime won't
//! change when it's back. Record [`Outcome::unreachable`] (a skip, so the
//! next run tries again), or record nothing.
//!
//! Nothing here writes on its own: [`record`] takes the caller's
//! connection, so a stage writes its results and its rows in the same
//! transaction, one batch at a time, through the [`crate::db::Writer`].

use rusqlite::Connection;

/// A scan stage after the walk. Stored in `file_stage.stage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Tags and audio properties (stage 2).
    Read,
    /// `blake3` and `audio_hash` (stage 3).
    Hash,
    /// The acoustic fingerprint (stage 3).
    Fingerprint,
}

impl Stage {
    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Read => "read",
            Stage::Hash => "hash",
            Stage::Fingerprint => "fingerprint",
        }
    }
}

/// Which files to look at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Scope {
    /// Every file.
    #[default]
    All,
    /// The files in these music folders (`music_folder.id`).
    Folders(Vec<i64>),
    /// These files (`file.id`), e.g. the tracks on screen first.
    Files(Vec<i64>),
}

impl Scope {
    /// The scope's two JSON-array parameters: folder ids and file ids, each
    /// NULL when not limited by it.
    fn params(&self) -> (Option<String>, Option<String>) {
        let json = |ids: &[i64]| serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
        match self {
            Scope::All => (None, None),
            Scope::Folders(ids) => (Some(json(ids)), None),
            Scope::Files(ids) => (None, Some(json(ids))),
        }
    }
}

/// A file due for a stage, with what the stage needs to open it and the
/// stat it should record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueFile {
    /// `file.id`.
    pub id: i64,
    pub music_folder_id: i64,
    /// From the music folder, `/`-separated, in the on-disk spelling.
    pub rel_path: String,
    /// `file.size` and `file.mtime` now. Pass them back to [`record`].
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    /// The walk found it online only (a OneDrive placeholder). Ask
    /// [`crate::scan::ReadGate::allows`] before reading it.
    pub online_only: bool,
}

/// How a stage left one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The stage finished the file (even if what it found is that the file
    /// is broken).
    Done,
    /// The stage read the file and can't process its content, e.g. a
    /// decoder error. Not retried until the file changes. **Never** for a
    /// file the stage couldn't reach or open: that's
    /// [`Outcome::unreachable`] (see the module docs). The reason is a
    /// short, non-empty code.
    Failed(&'static str),
    /// The stage didn't try the file, e.g. [`ONLINE_ONLY`] or
    /// [`UNREACHABLE`]. Due again on the next run. The reason is a short,
    /// non-empty code.
    Skipped(&'static str),
}

/// The reason for a file a stage couldn't reach or open.
pub const UNREACHABLE: &str = "unreachable";
/// The reason for a OneDrive online-only file, which reading would
/// download (1aB-8).
pub const ONLINE_ONLY: &str = "online_only";

impl Outcome {
    /// The file couldn't be reached or opened: an unplugged drive, a locked
    /// file, no permission, gone since the walk. A skip, so it's tried
    /// again on the next run.
    pub fn unreachable() -> Outcome {
        Outcome::Skipped(UNREACHABLE)
    }

    /// The file is online-only and the user hasn't opted in to reading
    /// those. A skip, so it's decided again on the next run.
    pub fn online_only() -> Outcome {
        Outcome::Skipped(ONLINE_ONLY)
    }

    fn status(&self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Failed(_) => "failed",
            Outcome::Skipped(_) => "skipped",
        }
    }

    fn reason(&self) -> Option<&'static str> {
        match self {
            Outcome::Done => None,
            Outcome::Failed(r) | Outcome::Skipped(r) => Some(r),
        }
    }
}

/// One file's result, to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub file: i64,
    /// The size and mtime the stage saw: [`DueFile::size`] and
    /// [`DueFile::mtime`].
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub outcome: Outcome,
}

impl Recorded {
    /// `due`'s result.
    pub fn of(due: &DueFile, outcome: Outcome) -> Recorded {
        Recorded {
            file: due.id,
            size: due.size,
            mtime: due.mtime,
            outcome,
        }
    }
}

/// The condition that makes a file due. Parameters: ?1 stage, ?2 version,
/// ?3 folder ids (JSON array or NULL), ?4 file ids (JSON array or NULL).
const DUE_WHERE: &str = "
    f.present = 1
    AND (?3 IS NULL OR f.music_folder_id IN (SELECT value FROM json_each(?3)))
    AND (?4 IS NULL OR f.id IN (SELECT value FROM json_each(?4)))
    AND (s.file_id IS NULL
         OR s.status = 'skipped'
         OR s.version <> ?2
         OR s.size IS NOT f.size
         OR s.mtime IS NOT f.mtime)";

/// Up to `limit` files due for `stage` at `version` in `scope`, with ids
/// above `after`, in id order. Page through them by passing the last id
/// back as `after` (start at 0).
///
/// Within one run, always advance `after` and never restart from 0:
/// skipped files stay due, so a run that restarted would meet them again
/// and never end.
pub fn due(
    conn: &Connection,
    stage: Stage,
    version: i64,
    scope: &Scope,
    after: i64,
    limit: usize,
) -> rusqlite::Result<Vec<DueFile>> {
    let (folders, files) = scope.params();
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT f.id, f.music_folder_id, f.rel_path, f.size, f.mtime, f.online_only
         FROM file f
         LEFT JOIN file_stage s ON s.file_id = f.id AND s.stage = ?1
         WHERE {DUE_WHERE} AND f.id > ?5
         ORDER BY f.id
         LIMIT ?6"
    ))?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map(
        (stage.as_str(), version, folders, files, after, limit),
        |r| {
            Ok(DueFile {
                id: r.get(0)?,
                music_folder_id: r.get(1)?,
                rel_path: r.get(2)?,
                size: r.get(3)?,
                mtime: r.get(4)?,
                online_only: r.get(5)?,
            })
        },
    )?;
    rows.collect()
}

/// How many files are due for `stage` at `version` in `scope`, for
/// progress.
///
/// Skipped files stay due, so this never reaches 0 while any are skipped
/// (e.g. online-only files). Don't loop "while `count_due` > 0"; run one
/// pass over [`due`] instead.
pub fn count_due(
    conn: &Connection,
    stage: Stage,
    version: i64,
    scope: &Scope,
) -> rusqlite::Result<u64> {
    let (folders, files) = scope.params();
    let count: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*)
             FROM file f
             LEFT JOIN file_stage s ON s.file_id = f.id AND s.stage = ?1
             WHERE {DUE_WHERE}"
        ),
        (stage.as_str(), version, folders, files),
        |r| r.get(0),
    )?;
    Ok(u64::try_from(count).unwrap_or(0))
}

/// Records `results` for `stage` at `version`, replacing each file's row.
///
/// Run it on the caller's transaction, with the stage's own writes for the
/// same files, so a batch lands whole or not at all. A file whose row is
/// gone (removed mid-batch) is skipped.
pub fn record(
    conn: &Connection,
    stage: Stage,
    version: i64,
    results: &[Recorded],
) -> rusqlite::Result<()> {
    let mut upsert = conn.prepare_cached(
        "INSERT INTO file_stage (file_id, stage, version, size, mtime, status, reason)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 WHERE EXISTS (SELECT 1 FROM file WHERE id = ?1)
         ON CONFLICT (file_id, stage) DO UPDATE SET
             version = excluded.version,
             size = excluded.size,
             mtime = excluded.mtime,
             status = excluded.status,
             reason = excluded.reason,
             done_at = excluded.done_at",
    )?;
    for r in results {
        debug_assert!(
            r.outcome.reason().is_none_or(|reason| !reason.is_empty()),
            "an empty reason is refused by file_stage and rolls back the whole batch"
        );
        upsert.execute((
            r.file,
            stage.as_str(),
            version,
            r.size,
            r.mtime,
            r.outcome.status(),
            r.outcome.reason(),
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
