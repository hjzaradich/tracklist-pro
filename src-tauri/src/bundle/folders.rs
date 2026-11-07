//! The bundle's folders in app data, and the only places this module makes
//! or deletes anything there (named exemptions in the write guard's scan).

use std::fs;
use std::path::{Path, PathBuf};

use crate::write_guard::{GuardError, WriteGuard};

/// Makes `path` (and missing parents) a folder; returns it as checked.
pub(super) fn make_folder(guard: &WriteGuard, path: &Path) -> Result<PathBuf, GuardError> {
    Ok(guard.create_dir_all(path)?.as_path().to_path_buf())
}

/// Deletes the file or folder at `path`; a folder with all in it.
pub(super) fn remove(guard: &WriteGuard, path: &Path) -> Result<(), GuardError> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        guard.remove_dir_all(path)
    } else {
        guard.remove_file(path)
    }
}

/// Whether anything (file, folder or link) is at `path`.
pub(super) fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// The names in `folder` that `is_ours` accepts, oldest first (the names
/// are sortable stamps).
pub(super) fn ours(folder: &Path, is_ours: fn(&str) -> bool) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|name| is_ours(name))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Deletes the oldest entries `is_ours` accepts in `folder` until `keep`
/// are left, counting `made` (the entry just made) as one of them. `made`
/// is never deleted, whatever the clock said when it was named (a stamp
/// older than the others still stays), as in `save_keeping_newest`. One
/// that can't be deleted is left for the next time.
pub(super) fn keep_newest(
    guard: &WriteGuard,
    folder: &Path,
    keep: usize,
    is_ours: fn(&str) -> bool,
    made: Option<&str>,
) {
    let names: Vec<String> = ours(folder, is_ours)
        .into_iter()
        .filter(|name| Some(name.as_str()) != made)
        .collect();
    let room = keep.saturating_sub(usize::from(made.is_some()));
    for old in names.iter().take(names.len().saturating_sub(room)) {
        let _ = remove(guard, &folder.join(old));
    }
}
