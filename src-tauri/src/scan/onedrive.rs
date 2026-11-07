//! Whether a music folder lives in OneDrive (ROADMAP 4.2, 5.5).
//!
//! OneDrive can keep files online only or free up space by removing local
//! copies, and the app then can't read them. This tells the screen which
//! folders to put a plain note next to. Two signs, either is enough:
//! - the folder's path is under one of the OneDrive folders Windows names
//!   in the `OneDrive`, `OneDriveConsumer` and `OneDriveCommercial`
//!   environment variables;
//! - the folder, or one of a small sample of its files, carries the cloud
//!   attributes [`super::online_only`] reads.
//!
//! Read-only: it reads environment variables, a directory listing and
//! attributes. It never changes an attribute or a file.

use std::ffi::OsString;
use std::path::Path;

use tauri::State;

use crate::db::ReadPool;
use crate::ipc::IpcError;

use super::folders::{stored, MusicFolderId};
use super::online_only::{attributes, is_online_only};
use super::system_volumes;

/// The environment variables that name OneDrive's folders.
const ROOT_VARIABLES: [&str; 3] = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"];

/// How many entries of a folder are asked about. A fixed small cap: the
/// check runs whenever the folders are listed.
const SAMPLE: usize = 20;

/// A path in one comparable form: no `\?\` prefix, backslashes, lower
/// case (Windows paths ignore case), no trailing separator.
fn normal(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    path.replace('/', r"\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Whether `path` is one of the OneDrive folders `lookup` names, or inside
/// one. `lookup` reads an environment variable (injected so a test needn't
/// touch the real environment).
pub(crate) fn under_onedrive_root(path: &str, lookup: impl Fn(&str) -> Option<OsString>) -> bool {
    let path = normal(path);
    ROOT_VARIABLES.iter().any(|name| {
        let Some(root) = lookup(name) else {
            return false;
        };
        let root = normal(&root.to_string_lossy());
        !root.is_empty()
            && (path == root
                || path
                    .strip_prefix(&root)
                    .is_some_and(|rest| rest.starts_with('\\')))
    })
}

/// Whether the folder, or a sample of the first [`SAMPLE`] entries in it,
/// has cloud attributes according to `read` (the attributes of a path).
pub(crate) fn has_cloud_attributes(dir: &Path, read: impl Fn(&Path) -> Option<u32>) -> bool {
    let cloud = |path: &Path| read(path).is_some_and(is_online_only);
    if cloud(dir) {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries
        .take(SAMPLE)
        .filter_map(Result::ok)
        .any(|entry| cloud(&entry.path()))
}

/// The attributes of `path`, if it can be asked about.
fn read_attributes(path: &Path) -> Option<u32> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|meta| attributes(&meta))
}

/// Whether the folder at `path` is in OneDrive, by either sign.
pub(crate) fn is_onedrive_folder(path: &Path, lookup: impl Fn(&str) -> Option<OsString>) -> bool {
    under_onedrive_root(&path.to_string_lossy(), lookup)
        || has_cloud_attributes(path, read_attributes)
}

/// The music folders (scan or Inbox) that are in OneDrive. A folder whose
/// drive isn't connected is left out: it can't be asked about.
#[tauri::command]
#[specta::specta]
pub async fn onedrive_music_folders(
    reads: State<'_, ReadPool>,
) -> Result<Vec<MusicFolderId>, IpcError> {
    let folders = reads.read(stored)?;
    let volumes = system_volumes();
    Ok(folders
        .iter()
        .filter_map(|folder| {
            let path = folder.stored_path().resolve(&volumes).ok()?;
            is_onedrive_folder(&path, |name| std::env::var_os(name)).then_some(folder.id)
        })
        .collect())
}
