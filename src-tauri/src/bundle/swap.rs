//! Replacing the Library file with a staged one, keeping the old one as a
//! backup (ROADMAP 4.1). Runs at startup, before anything opens the
//! database.
//!
//! The steps, each a rename inside app data:
//! 1. fold the old Library into its one file (its log written in), unless
//!    a swap was already under way;
//! 2. move its `-journal`, `-wal`, `-shm` and then the file itself into
//!    `backups/swapping`;
//! 3. move the staged `import/pending.db` into its place (the swap is done);
//! 4. rename `backups/swapping` to `backups/<stamp>`;
//! 5. delete the marker, then keep the newest [`KEEP_BACKUPS`].
//!
//! Every start runs the steps again while the marker is there, and each
//! step skips what is already done, so a crash at any point is finished at
//! the next start. The new file never joins an old `-wal`: it moves in only
//! after every piece of the old one has moved out. When the staged file is
//! gone before step 3, the old Library's pieces are moved back instead.

use std::path::{Path, PathBuf};

use super::folders::{exists, keep_newest, make_folder, ours, remove};
use super::BundleError;
use crate::db::{db_path, DB_FILE_NAME};
use crate::write_guard::WriteGuard;

/// How many replaced Libraries are kept.
pub const KEEP_BACKUPS: usize = 3;

/// The old Library's files, the file itself last.
const PIECES: [&str; 4] = ["-journal", "-wal", "-shm", ""];

/// Where everything lives, in app data.
pub(super) struct Layout {
    pub live: PathBuf,
    pub import: PathBuf,
    pub pending: PathBuf,
    pub marker: PathBuf,
    pub backups: PathBuf,
    pub swapping: PathBuf,
}

impl Layout {
    pub fn of(guard: &WriteGuard) -> Layout {
        let dir = guard.app_data_dir();
        let import = dir.join("import");
        let backups = dir.join("backups");
        Layout {
            live: db_path(dir),
            pending: import.join("pending.db"),
            marker: import.join("confirmed"),
            swapping: backups.join("swapping"),
            import,
            backups,
        }
    }
}

/// A backup's id: its time to the millisecond, as sortable text.
pub(super) fn stamp(ms: i64) -> String {
    format!("{ms:015}")
}

pub(super) fn is_stamp(name: &str) -> bool {
    name.len() == 15 && name.bytes().all(|b| b.is_ascii_digit())
}

fn piece(base: &Path, suffix: &str) -> PathBuf {
    let mut name = base.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Finishes a confirmed import or restore, if there is one. Returns the new
/// backup's id. On a failure with no Library in place, the old one is put
/// back first; the marker stays, so the next start tries again.
pub fn on_startup(guard: &WriteGuard) -> Result<Option<String>, BundleError> {
    start(
        guard,
        &Layout::of(guard),
        crate::diagnostics::now_ms(),
        &mut || false,
    )
}

/// [`on_startup`], with [`finish`]'s crash points open to tests.
pub(super) fn start(
    guard: &WriteGuard,
    layout: &Layout,
    now_ms: i64,
    crash: &mut dyn FnMut() -> bool,
) -> Result<Option<String>, BundleError> {
    finish(guard, layout, now_ms, crash).inspect_err(|_| {
        if !exists(&layout.live) {
            if let Err(again) = put_back(guard, layout) {
                eprintln!("startup: the old Library couldn't be put back: {again}");
            }
        }
    })
}

/// The swap. `crash` is asked before each change; `true` stops right there,
/// as a crash would (tests).
pub(super) fn finish(
    guard: &WriteGuard,
    l: &Layout,
    now_ms: i64,
    crash: &mut dyn FnMut() -> bool,
) -> Result<Option<String>, BundleError> {
    let mut step = || {
        if crash() {
            Err(BundleError::Interrupted)
        } else {
            Ok(())
        }
    };
    if !exists(&l.marker) {
        // Nothing confirmed: a staged file nobody confirmed goes.
        if exists(&l.pending) {
            remove(guard, &l.pending)?;
        }
        return Ok(None);
    }
    if exists(&l.pending) {
        if !exists(&l.swapping) {
            fold(guard, &l.live);
        }
        step()?;
        make_folder(guard, &l.swapping)?;
        for suffix in PIECES {
            let from = piece(&l.live, suffix);
            if exists(&from) {
                let to = piece(&l.swapping.join(DB_FILE_NAME), suffix);
                if exists(&to) {
                    // Never written over: stop with both kept.
                    return Err(BundleError::Blocked);
                }
                step()?;
                guard.rename(&from, &to)?;
            }
        }
        step()?;
        guard.rename(&l.pending, &l.live)?;
    } else if !exists(&l.live) || !exists(&l.swapping.join(DB_FILE_NAME)) {
        // Stopped before the swap with the staged file gone: the old
        // Library's pieces go home.
        put_back(guard, l)?;
    }
    let mut kept = None;
    if exists(&l.swapping.join(DB_FILE_NAME)) {
        let id = stamp(now_ms);
        step()?;
        guard.rename(&l.swapping, &l.backups.join(&id))?;
        kept = Some(id);
    } else if exists(&l.swapping) && ours(&l.swapping, |_| true).is_empty() {
        remove(guard, &l.swapping)?;
    }
    step()?;
    remove(guard, &l.marker)?;
    keep_newest(guard, &l.backups, KEEP_BACKUPS, is_stamp);
    Ok(kept)
}

/// Writes the old Library's log into its file and makes it one file, so
/// its backup is whole on its own. Best effort: a Library SQLite can't
/// open is still moved aside whole, with its pieces.
fn fold(guard: &WriteGuard, live: &Path) {
    if !exists(live) {
        return;
    }
    let folded = guard
        .check(live)
        .map_err(|e| e.to_string())
        .and_then(|p| p.open_database().map_err(|e| e.to_string()))
        .and_then(|c| {
            c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                .and_then(|()| c.pragma_update(None, "journal_mode", "DELETE"))
                .map_err(|e| e.to_string())
        });
    if let Err(e) = folded {
        eprintln!("startup: the Library's log couldn't be folded in before the swap: {e}");
    }
}

/// Moves the old Library's pieces back from `backups/swapping`, each where
/// nothing is in its place, the file itself last (as they left).
fn put_back(guard: &WriteGuard, l: &Layout) -> Result<(), BundleError> {
    for suffix in PIECES {
        let from = piece(&l.swapping.join(DB_FILE_NAME), suffix);
        let to = piece(&l.live, suffix);
        if exists(&from) && !exists(&to) {
            guard.rename(&from, &to)?;
        }
    }
    Ok(())
}
