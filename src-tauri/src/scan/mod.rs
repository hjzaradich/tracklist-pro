//! Music folders and the walk that indexes them (ROADMAP 1.1, §5.5).
//!
//! [`folders`] adds, removes and lists the folders the user points the app
//! at. The walk (stage 1 of the scan) lists every audio file in them with
//! its size, modified time and file id, into `file` rows.
//!
//! Everything here only reads the music folders. The only writes go to the
//! database, through the [`crate::db::Writer`].

pub mod chain;
#[cfg(windows)]
mod file_id;
pub mod folders;
pub mod online_only;
mod unchanged;
pub mod volumes;
pub mod walk;
pub mod watch;

pub use folders::{MusicFolder, MusicFolderError, MusicFolderId, MusicFolderRole};
pub use online_only::ReadGate;
pub use volumes::VolumesChanged;
pub use walk::{scan_job, ScannedFile, ScannedFiles, Walker};
pub use watch::Watchers;

use std::path::Path;

use tauri::{AppHandle, Runtime, State};
use tauri_specta::Event;

use crate::ipc::IpcError;
use crate::jobs::{JobHandler, JobId, JobQueue};

/// Whether the walk indexes a file with this name: one with an audio
/// extension ([`crate::tags::AUDIO_EXTENSIONS`], the one list, any letter
/// case) that isn't [skipped](is_skipped). The format itself is sniffed
/// from the bytes later (§5.5).
pub fn is_indexed(name: &str) -> bool {
    !is_skipped(name) && crate::tags::is_audio_file(Path::new(name))
}

/// Files that are never music, whatever their extension (§5.5): macOS
/// AppleDouble files (`._Track.mp3`, the resource fork of `Track.mp3`,
/// left on drives a Mac has written to) and Finder's `.DS_Store`. Any
/// letter case, as Windows compares names.
pub fn is_skipped(name: &str) -> bool {
    name.starts_with("._") || name.eq_ignore_ascii_case(".DS_Store")
}

/// The scan job's handler for the app: asks Windows which volumes are
/// mounted at the start of each job, and sends new rows to the webview.
pub fn walker<R: Runtime>(app: &AppHandle<R>) -> impl JobHandler {
    let app = app.clone();
    Walker::new(system_volumes, move |files| {
        // Only fails if the app is closing.
        let _ = ScannedFiles(files).emit(&app);
    })
}

/// Scans the music folders `ids`, or all of them. Returns the job's id; its
/// progress shows in Activity, and new files arrive as `ScannedFiles`.
#[tauri::command(async)]
#[specta::specta]
pub fn scan_music_folders(
    jobs: State<'_, JobQueue>,
    ids: Option<Vec<MusicFolderId>>,
) -> Result<JobId, IpcError> {
    Ok(jobs.enqueue(scan_job(ids))?)
}

/// The volumes mounted right now, as the OS reports them, with clones told
/// apart by what the library remembers. It looks again whenever drives
/// come or go (1aB-9).
#[cfg(windows)]
pub(crate) fn system_volumes() -> crate::paths::SystemVolumes {
    crate::paths::SystemVolumes::scan_remembering(volumes::remembered_now())
}

#[cfg(not(windows))]
pub(crate) use unsupported::system_volumes;

/// macOS and Linux are built in CI but not supported (ROADMAP §1.1): paths
/// there never parse as Windows paths, so adding a folder fails cleanly.
#[cfg(not(windows))]
mod unsupported {
    use std::io;
    use std::path::{Path, PathBuf};

    use crate::paths::Volumes;
    use crate::volume::{Volume, VolumeId};

    pub(crate) fn system_volumes() -> UnsupportedVolumes {
        UnsupportedVolumes
    }

    /// No volume is ever mounted.
    pub(crate) struct UnsupportedVolumes;

    impl Volumes for UnsupportedVolumes {
        fn volume_for(&self, path: &Path) -> io::Result<Volume> {
            crate::volume::volume_for(path)
        }

        fn mount_path(&self, _id: &VolumeId) -> Option<PathBuf> {
            None
        }
    }
}

/// A path as a person reads it: without the `\\?\` prefix the app opens
/// files through (`\\?\UNC\server\share` becomes `\\server\share`).
pub(crate) fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = s.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
mod tests;
