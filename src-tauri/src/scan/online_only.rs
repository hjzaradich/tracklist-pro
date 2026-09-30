//! OneDrive "online-only" files (1aB-8, ROADMAP 1.1, §5.5).
//!
//! An online-only file is a placeholder: its name, size and times are on
//! disk, its bytes are in the cloud, and reading them makes OneDrive
//! download the whole file. The directory listing says which files are
//! placeholders, through their attributes, so the walk tells them apart
//! without opening anything. It indexes them like any file and marks them
//! (`file.online_only`); no later stage reads their bytes unless the user
//! opts in.
//!
//! **[`ReadGate`] is the only way in** for stages that read a file's bytes
//! (tags, hashes, fingerprints). Make one per job ([`ReadGate::for_job`]),
//! then for each file:
//! 1. [`ReadGate::may_read`] (or [`ReadGate::allows`] on a selected
//!    `online_only`): the walk didn't mark it online only, or the user
//!    opted in.
//! 2. [`ReadGate::may_open`] on the resolved path, just before opening:
//!    OneDrive can free up space and turn a local file into a placeholder
//!    at any time after the walk. `Err` means the file couldn't be asked
//!    about (gone, no access, drive unplugged): unreachable, not opened.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use tauri::State;

use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;

/// The `setting` key for the opt-in: a JSON `true` lets later stages read
/// online-only files (and so download them). Off by default.
pub const READ_ONLINE_ONLY_FILES: &str = "read_online_only_files";

/// `FILE_ATTRIBUTE_OFFLINE`: the data isn't available right away.
pub const ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
/// `FILE_ATTRIBUTE_RECALL_ON_OPEN`: opening the file fetches it.
pub const ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`: reading any byte fetches it.
/// This is the one OneDrive's online-only files carry.
pub const ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

/// Whether a file with these attributes (from the directory listing) is
/// online only: reading it would download it first. "Always keep on this
/// device" (pinned) and downloaded files carry none of these.
pub fn is_online_only(attributes: u32) -> bool {
    attributes & (ATTRIBUTE_OFFLINE | ATTRIBUTE_RECALL_ON_OPEN | ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

/// Whether a job may read files' bytes, with the user's opt-in read once
/// for the whole job. See the module doc for how a stage uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadGate {
    /// The user opted in to reading (and so downloading) online-only files.
    opted_in: bool,
}

impl ReadGate {
    /// The gate for one job: reads the opt-in once.
    pub fn for_job(conn: &Connection) -> rusqlite::Result<ReadGate> {
        Ok(ReadGate {
            opted_in: read_opt_in(conn)?,
        })
    }

    /// Whether the job may read `file` (a `file` row id): yes unless the
    /// walk marked it online only and the user hasn't opted in. No for an
    /// id with no row.
    pub fn may_read(&self, conn: &Connection, file: i64) -> rusqlite::Result<bool> {
        let online_only: Option<bool> = conn
            .query_row("SELECT online_only FROM file WHERE id = ?1", [file], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(online_only.is_some_and(|o| self.allows(o)))
    }

    /// [`ReadGate::may_read`]'s rule, for a row whose `online_only` the
    /// caller already selected.
    pub fn allows(&self, online_only: bool) -> bool {
        !online_only || self.opted_in
    }

    /// Whether to open the file at `path` (a `\\?\` path) now, just
    /// before opening it: `Ok(true)` open it; `Ok(false)` it's online only
    /// and the user hasn't opted in, so don't; `Err` it couldn't be asked
    /// about (gone since the walk, no access, drive unplugged), so treat it
    /// as unreachable and don't open it either.
    ///
    /// Opted in, it's `Ok(true)` without asking. Otherwise it asks for the
    /// file's attributes only: that query opens no handle that can read
    /// data (access 0, the link or placeholder itself), so it never
    /// downloads anything.
    pub fn may_open(&self, path: &Path) -> std::io::Result<bool> {
        if self.opted_in {
            return Ok(true);
        }
        is_local(path)
    }
}

// Stage jobs move their gate into writer calls and worker threads: keep it
// Send + Sync + Clone + Copy (a compile error otherwise).
const _: () = {
    fn crosses_threads<T: Send + Sync + Clone + Copy>() {}
    let _ = crosses_threads::<ReadGate>;
};

/// Whether the file at `path` is on this PC right now, from its attributes:
/// `Ok(false)` if it's online only, `Err` if it can't be asked about.
fn is_local(path: &Path) -> std::io::Result<bool> {
    let meta = std::fs::symlink_metadata(path)?;
    Ok(!is_online_only(attributes(&meta)))
}

/// A file's attributes, as the listing or an attribute query reports them.
#[cfg(windows)]
pub(crate) fn attributes(meta: &std::fs::Metadata) -> u32 {
    std::os::windows::fs::MetadataExt::file_attributes(meta)
}

/// No placeholders outside Windows (unsupported, ROADMAP §1.1).
#[cfg(not(windows))]
pub(crate) fn attributes(_meta: &std::fs::Metadata) -> u32 {
    0
}

/// Whether the user opted in to reading online-only files. Off when unset,
/// or when the stored value isn't a JSON boolean. Stages use
/// [`ReadGate::for_job`], which reads it once per job.
pub(in crate::scan) fn read_opt_in(conn: &Connection) -> rusqlite::Result<bool> {
    let on: Option<Option<bool>> = conn
        .query_row(
            "SELECT CASE json_type(value) WHEN 'true' THEN 1 WHEN 'false' THEN 0 END
             FROM setting WHERE key = ?1",
            [READ_ONLINE_ONLY_FILES],
            |r| r.get(0),
        )
        .optional()?;
    Ok(on.flatten().unwrap_or(false))
}

/// Stores the opt-in.
pub(in crate::scan) fn write_opt_in(conn: &Connection, on: bool) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, json(?2))
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (READ_ONLINE_ONLY_FILES, if on { "true" } else { "false" }),
    )?;
    Ok(())
}

/// Whether the app may read (and so download) online-only files. Off until
/// the user turns it on.
#[tauri::command]
#[specta::specta]
pub async fn read_online_only_files(reads: State<'_, ReadPool>) -> Result<bool, IpcError> {
    Ok(reads.read(read_opt_in)?)
}

/// Lets the app read online-only files, or stops it.
#[tauri::command]
#[specta::specta]
pub async fn set_read_online_only_files(
    writer: State<'_, Writer>,
    on: bool,
) -> Result<(), IpcError> {
    Ok(writer.call(move |c| write_opt_in(c, on))?)
}
