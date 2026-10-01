//! The Missing list (1aD-5, ROADMAP §1.3): rekordbox tracks that have no
//! file, with their last known path from rekordbox's Location, grouped by
//! the folder they were last in.
//!
//! The list only reads the database. A row is missing when it has no file
//! (`rekordbox_track.file_id IS NULL`). Streaming entries are left out (they
//! never have a file), and so are the skipped kinds, which the reader never
//! stores (§5.3). A probable match has a file, so it isn't here; 1bE reviews
//! those.
//!
//! A group offers "add folder" when that folder exists on the disk now and
//! isn't inside (or around) a music folder. The check is made through
//! [`DiskProbe`], which never opens a file, skips drives that aren't
//! connected, and leaves network paths and OneDrive online-only folders
//! alone, so it can't hang or download anything.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use tauri::State;
use unicode_normalization::UnicodeNormalization;

use crate::db::ReadPool;
use crate::ipc::{ErrorKind, IpcError};
use crate::paths::{StoredPath, Volumes};
use crate::rekordbox::location::{decode, FilePath, Location, PathStyle};
use crate::scan::folders::{overlap, stored, StoredFolder};

#[cfg(test)]
mod tests;

/// What the Missing list asks of the disk.
pub trait DiskProbe {
    /// Whether a drive is connected now (a disk, or removable media in the
    /// drive). Never true for a network drive: those can take seconds to
    /// answer.
    fn drive_connected(&self, letter: char) -> bool;

    /// Whether `folder` (a Windows path on a connected drive) is a folder
    /// on the disk now, and is really there, not an online-only
    /// placeholder. Reads attributes only.
    fn folder_exists(&self, folder: &str) -> bool;
}

/// A rekordbox track with no file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MissingTrack {
    /// The row's id, for the app. Not rekordbox's TrackID.
    pub id: i64,
    pub title: String,
    pub artist: String,
    /// Where rekordbox last had the file, as Windows writes it. `None` if
    /// the Location couldn't be decoded.
    pub last_known_path: Option<String>,
}

/// The missing tracks that were last in one folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MissingGroup {
    /// The last known folder, as Windows writes it. `None` for tracks whose
    /// Location couldn't be decoded.
    pub folder: Option<String>,
    /// Whether the folder exists now and is outside every music folder, so
    /// adding it as a music folder (`add_music_folder`) makes sense.
    pub can_add: bool,
    pub tracks: Vec<MissingTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct MissingList {
    pub total: u32,
    /// By folder, in folder order; tracks with no known folder last.
    pub groups: Vec<MissingGroup>,
}

/// A last known folder.
struct Folder {
    /// As Windows writes it (`D:\Music\Old`).
    shown: String,
    style: PathStyle,
    drive: Option<char>,
}

impl Folder {
    fn of(path: &FilePath) -> Folder {
        let parent = path.as_str().rsplit_once('/').map_or("", |(p, _)| p);
        // A file at the top of a drive: `C:` is `C:/`.
        let parent = if path.style() == PathStyle::WindowsDrive && !parent.contains('/') {
            format!("{parent}/")
        } else {
            parent.to_owned()
        };
        let shown = match path.style() {
            PathStyle::Posix => parent,
            _ => parent.replace('/', "\\"),
        };
        let drive = match path.style() {
            PathStyle::WindowsDrive => shown.chars().next(),
            _ => None,
        };
        Folder {
            shown,
            style: path.style(),
            drive,
        }
    }

    /// Two spellings of one folder share a key: NFC, and letter case
    /// ignored for Windows paths.
    fn key(&self) -> String {
        let nfc: String = self.shown.nfc().collect();
        match self.style {
            PathStyle::Posix => nfc,
            _ => nfc.to_uppercase(),
        }
    }
}

/// The path as the user sees it: Windows form for Windows paths.
fn shown(path: &FilePath) -> String {
    path.to_windows()
        .unwrap_or_else(|| path.as_str().to_owned())
}

/// The missing tracks, grouped by last known folder.
pub fn missing_list(
    conn: &Connection,
    volumes: &impl Volumes,
    disk: &impl DiskProbe,
) -> rusqlite::Result<MissingList> {
    let mut groups: BTreeMap<String, (Folder, Vec<MissingTrack>)> = BTreeMap::new();
    let mut unknown = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, location,
                coalesce(json_extract(attributes, '$.Name'), ''),
                coalesce(json_extract(attributes, '$.Artist'), '')
         FROM rekordbox_track WHERE file_id IS NULL ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut total = 0;
    for row in rows {
        let (id, location, title, artist) = row?;
        let mut track = MissingTrack {
            id,
            title,
            artist,
            last_known_path: None,
        };
        match decode(&location) {
            Ok(Location::Streaming(_)) => continue,
            Ok(Location::File(path)) => {
                track.last_known_path = Some(shown(&path));
                let folder = Folder::of(&path);
                groups
                    .entry(folder.key())
                    .or_insert_with(|| (folder, Vec::new()))
                    .1
                    .push(track);
            }
            Err(_) => unknown.push(track),
        }
        total += 1;
    }

    let music_folders = stored(conn)?;
    let mut list = Vec::new();
    for (_, (folder, tracks)) in groups {
        let can_add = offer(&folder, &music_folders, volumes, disk);
        list.push(MissingGroup {
            folder: Some(folder.shown),
            can_add,
            tracks: sorted(tracks),
        });
    }
    if !unknown.is_empty() {
        list.push(MissingGroup {
            folder: None,
            can_add: false,
            tracks: sorted(unknown),
        });
    }
    Ok(MissingList {
        total,
        groups: list,
    })
}

fn sorted(mut tracks: Vec<MissingTrack>) -> Vec<MissingTrack> {
    tracks.sort_by_key(|t| (t.artist.to_lowercase(), t.title.to_lowercase(), t.id));
    tracks
}

/// Whether to offer adding `folder`: it's on a connected local drive, it's
/// there now, and it neither sits inside nor holds a music folder.
fn offer(
    folder: &Folder,
    music_folders: &[StoredFolder],
    volumes: &impl Volumes,
    disk: &impl DiskProbe,
) -> bool {
    let Some(letter) = folder.drive else {
        return false;
    };
    // The drive first: nothing else is asked about one that isn't there.
    if !disk.drive_connected(letter) || !disk.folder_exists(&folder.shown) {
        return false;
    }
    let Ok(stored) = StoredPath::from_absolute(Path::new(&folder.shown), volumes) else {
        return false;
    };
    !music_folders
        .iter()
        .any(|m| m.volume == *stored.volume() && overlap(stored.rel(), &m.rel).is_some())
}

/// The real disk. Only attribute reads; drives and shares that could stall
/// aren't asked.
pub struct SystemDisk;

#[cfg(windows)]
impl DiskProbe for SystemDisk {
    fn drive_connected(&self, letter: char) -> bool {
        use crate::volume::DriveType;
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        if !letter.is_ascii_alphabetic() {
            return false;
        }
        let root: Vec<u16> = format!("{}:\\", letter.to_ascii_uppercase())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: `root` is a NUL-terminated wide string that outlives the call.
        let kind = DriveType::from_win32(unsafe { GetDriveTypeW(root.as_ptr()) });
        matches!(
            kind,
            DriveType::Fixed | DriveType::Removable | DriveType::RamDisk
        )
    }

    fn folder_exists(&self, folder: &str) -> bool {
        // The `\\?\` form, so names ending in a dot or space open as written.
        let Ok(meta) = std::fs::metadata(format!(r"\\?\{folder}")) else {
            return false;
        };
        meta.is_dir()
            && !crate::scan::online_only::is_online_only(crate::scan::online_only::attributes(
                &meta,
            ))
    }
}

#[cfg(not(windows))]
impl DiskProbe for SystemDisk {
    fn drive_connected(&self, _letter: char) -> bool {
        false
    }

    fn folder_exists(&self, _folder: &str) -> bool {
        false
    }
}

/// The rekordbox tracks that have no file, by last known folder, with an
/// offer to add each folder that exists now (1aD-5).
#[tauri::command]
#[specta::specta]
pub async fn missing_tracks(reads: State<'_, ReadPool>) -> Result<MissingList, IpcError> {
    let reads = reads.inner().clone();
    // The disk is asked off the async threads.
    let list = tauri::async_runtime::spawn_blocking(move || {
        reads.read(|conn| missing_list(conn, &crate::scan::system_volumes(), &SystemDisk))
    })
    .await
    .map_err(|_| IpcError::new(ErrorKind::Internal))??;
    Ok(list)
}
