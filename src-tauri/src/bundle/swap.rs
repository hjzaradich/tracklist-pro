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
//! A failure never stops the app opening while a Library is in place: see
//! [`start`].
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
use crate::diagnostics::{EventKind, EventLog, EventReason, LogEvent};
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

/// What the swap at startup did.
#[derive(Debug, PartialEq)]
pub enum Swapped {
    /// No confirmed import or restore.
    Nothing,
    /// The staged Library is in place; the old one is kept as this backup
    /// (none when there was no old one).
    Replaced(Option<String>),
    /// It failed, the Library in place opens, and the confirm is withdrawn
    /// so the failure isn't met again at every start.
    NotReplaced(EventReason),
}

/// Finishes a confirmed import or restore, if there is one, and records a
/// failure in the event log. Fails only when no Library is in place.
pub fn on_startup(guard: &WriteGuard, events: &EventLog) -> Result<Swapped, BundleError> {
    let swapped = start(
        guard,
        &Layout::of(guard),
        crate::diagnostics::now_ms(),
        &mut || false,
    )?;
    if let Swapped::NotReplaced(reason) = &swapped {
        events.record(LogEvent::new(EventKind::LibraryNotReplaced).reason(*reason));
    }
    Ok(swapped)
}

/// [`on_startup`], with [`finish`]'s crash points open to tests.
///
/// On a failure the old Library's pieces come home where they can: when
/// its file is still in place (the pieces set aside are its own) or when
/// nothing is (the new file never moved in). When the new file is in place
/// nothing moves, so an old `-wal` never lands beside it. Then, if a
/// Library is in place, it opens: the failure is logged, and the confirm
/// withdrawn (a Library left set aside is kept as a backup at the next
/// start). Only with no Library in place does the error stop startup,
/// rather than an empty Library being made.
pub(super) fn start(
    guard: &WriteGuard,
    layout: &Layout,
    now_ms: i64,
    crash: &mut dyn FnMut() -> bool,
) -> Result<Swapped, BundleError> {
    let error = match finish(guard, layout, now_ms, crash) {
        Ok(swapped) => return Ok(swapped),
        Err(error) => error,
    };
    eprintln!("startup: the Library couldn't be replaced: {error}");
    if !exists(&layout.live) || !exists(&layout.swapping.join(DB_FILE_NAME)) {
        if let Err(again) = put_back(guard, layout) {
            eprintln!("startup: the old Library couldn't all be put back: {again}");
        }
    }
    if !exists(&layout.live) {
        return Err(error);
    }
    if exists(&layout.marker) {
        if let Err(e) = remove(guard, &layout.marker) {
            eprintln!("startup: the import's confirm couldn't be withdrawn: {e}");
        }
    }
    Ok(Swapped::NotReplaced(match error {
        BundleError::Blocked => EventReason::FileInTheWay,
        _ => EventReason::CantWrite,
    }))
}

/// The swap. `crash` is asked before each change; `true` stops right there,
/// as a crash would (tests).
pub(super) fn finish(
    guard: &WriteGuard,
    l: &Layout,
    now_ms: i64,
    crash: &mut dyn FnMut() -> bool,
) -> Result<Swapped, BundleError> {
    let mut step = || {
        if crash() {
            Err(BundleError::Interrupted)
        } else {
            Ok(())
        }
    };
    let aside = l.swapping.join(DB_FILE_NAME);
    if !exists(&l.marker) {
        // Nothing confirmed: a staged file nobody confirmed goes, and a
        // Library a failed swap left set aside is kept as a backup.
        if exists(&l.pending) {
            remove(guard, &l.pending)?;
        }
        // Set aside with no Library in place: bring it home, so an empty
        // one is never made over a real one.
        if exists(&aside) && !exists(&l.live) {
            put_back(guard, l)?;
        }
        if exists(&aside) && exists(&l.live) {
            let id = set_aside(guard, l, now_ms)?;
            keep_newest(guard, &l.backups, KEEP_BACKUPS, is_stamp, Some(&id));
        }
        return Ok(Swapped::Nothing);
    }
    if exists(&l.pending) {
        // A piece in place whose name is already taken aside would be
        // written over: refuse before anything moves, with both kept.
        if PIECES
            .iter()
            .any(|s| exists(&piece(&l.live, s)) && exists(&piece(&aside, s)))
        {
            return Err(BundleError::Blocked);
        }
        // `swapping` there means a swap under way: its pieces may already
        // be split, so the file isn't touched. (An empty one left by a crash
        // just after it was made skips the fold too; the pieces then travel
        // together, so nothing is lost.)
        if !exists(&l.swapping) {
            fold(guard, &l.live);
        }
        step()?;
        make_folder(guard, &l.swapping)?;
        for suffix in PIECES {
            let from = piece(&l.live, suffix);
            if exists(&from) {
                step()?;
                guard.rename(&from, &piece(&aside, suffix))?;
            }
        }
        step()?;
        guard.rename(&l.pending, &l.live)?;
    } else if !exists(&l.live) || !exists(&aside) {
        // Stopped before the swap with the staged file gone: the old
        // Library's pieces go home.
        put_back(guard, l)?;
    }
    let mut kept = None;
    if exists(&aside) {
        step()?;
        kept = Some(set_aside(guard, l, now_ms)?);
    } else if exists(&l.swapping) && ours(&l.swapping, |_| true).is_empty() {
        remove(guard, &l.swapping)?;
    }
    step()?;
    remove(guard, &l.marker)?;
    keep_newest(guard, &l.backups, KEEP_BACKUPS, is_stamp, kept.as_deref());
    Ok(Swapped::Replaced(kept))
}

/// Renames `backups/swapping` to a backup named for `now_ms`, or the next
/// free millisecond. Returns its id.
fn set_aside(guard: &WriteGuard, l: &Layout, now_ms: i64) -> Result<String, BundleError> {
    let mut ms = now_ms;
    while exists(&l.backups.join(stamp(ms))) {
        ms += 1;
    }
    let id = stamp(ms);
    guard.rename(&l.swapping, &l.backups.join(&id))?;
    Ok(id)
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
///
/// When a move fails and the file isn't in place, the pieces already home
/// would sit beside no file, where a different file moving in could meet
/// them: they go back to `swapping` with their file. A piece that can't go
/// back is renamed in place to `<piece>.stray`, a name SQLite never reads
/// (never deleted: it may be the only copy of the log); if that fails too
/// it is logged and left. With the file in place, its own pieces
/// stay: they are its set.
fn put_back(guard: &WriteGuard, l: &Layout) -> Result<(), BundleError> {
    put_back_with(guard, l, &mut |from, to| Ok(guard.rename(from, to)?))
}

/// [`put_back`], with the move open to tests (which make it fail).
pub(super) fn put_back_with(
    guard: &WriteGuard,
    l: &Layout,
    mv: &mut dyn FnMut(&Path, &Path) -> Result<(), BundleError>,
) -> Result<(), BundleError> {
    let aside = l.swapping.join(DB_FILE_NAME);
    let mut home: Vec<(PathBuf, PathBuf)> = Vec::new();
    for suffix in PIECES {
        let from = piece(&aside, suffix);
        let to = piece(&l.live, suffix);
        if exists(&from) && !exists(&to) {
            if let Err(error) = mv(&from, &to) {
                if !exists(&l.live) {
                    for (from, to) in home.iter().rev() {
                        if mv(to, from).is_err() {
                            // Never deleted: it may be the only copy of the log.
                            let stray = piece(to, ".stray");
                            if let Err(e) = guard.rename(to, &stray) {
                                eprintln!(
                                    "startup: a piece of the old Library stays beside no file: {e}"
                                );
                            }
                        }
                    }
                }
                return Err(error);
            }
            home.push((from, to));
        }
    }
    Ok(())
}
