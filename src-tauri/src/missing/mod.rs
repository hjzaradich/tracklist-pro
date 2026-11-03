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
//! The list is sent a page at a time (X3): each group carries its count (the
//! length of `track_ids`), every track's id in the order shown, and only the
//! first [`FIRST_PAGE`] tracks in full. [`missing_track_page`] fills in the
//! rest, a bounded number of ids at a time. The group's one action, "add
//! folder", is about the folder, so it doesn't depend on which rows are
//! loaded.
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
    /// Every missing track in the group, in the order shown. Its length is
    /// the group's count.
    pub track_ids: Vec<i64>,
    /// The first tracks of the group, in full; the rest come from
    /// `missing_track_page`.
    pub tracks: Vec<MissingTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct MissingList {
    pub total: u32,
    /// By folder, in folder order; tracks with no known folder last.
    pub groups: Vec<MissingGroup>,
}

/// How many tracks of each group come with the list.
pub const FIRST_PAGE: usize = 100;

/// The most ids one `missing_track_page` call answers; more are ignored.
pub const MAX_PAGE: usize = 200;

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

/// What the database holds for the list, read in one short database read
/// so the disk is asked afterwards, with no connection held.
pub struct Gathered {
    /// Row id, raw Location, title, artist.
    rows: Vec<(i64, String, String, String)>,
    music_folders: Vec<StoredFolder>,
}

/// Reads the rows that have no file, and the music folders.
pub fn gather(conn: &Connection) -> rusqlite::Result<Gathered> {
    let mut stmt = conn.prepare(
        "SELECT id, CAST(location AS TEXT),
                coalesce(CAST(json_extract(attributes, '$.Name') AS TEXT), ''),
                coalesce(CAST(json_extract(attributes, '$.Artist') AS TEXT), '')
         FROM rekordbox_track WHERE file_id IS NULL ORDER BY id",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Gathered {
        rows,
        music_folders: stored(conn)?,
    })
}

/// The missing tracks, grouped by last known folder, with the offer to add
/// each folder that exists now. Asks the disk, so call it with no database
/// connection held.
pub fn group(gathered: Gathered, volumes: &impl Volumes, disk: &impl DiskProbe) -> MissingList {
    let mut groups: BTreeMap<String, (Folder, Vec<MissingTrack>)> = BTreeMap::new();
    let mut unknown = Vec::new();
    let mut total = 0;
    for (id, location, title, artist) in gathered.rows {
        match read(id, &location, title, artist) {
            None => continue,
            Some((track, Some(path))) => {
                let folder = Folder::of(&path);
                groups
                    .entry(folder.key())
                    .or_insert_with(|| (folder, Vec::new()))
                    .1
                    .push(track);
            }
            Some((track, None)) => unknown.push(track),
        }
        total += 1;
    }

    let mut list = Vec::new();
    for (_, (folder, tracks)) in groups {
        let can_add = offer(&folder, &gathered.music_folders, volumes, disk);
        list.push(MissingGroup {
            folder: Some(folder.shown),
            can_add,
            ..page_of(sorted(tracks))
        });
    }
    if !unknown.is_empty() {
        list.push(MissingGroup {
            folder: None,
            can_add: false,
            ..page_of(sorted(unknown))
        });
    }
    MissingList {
        total,
        groups: list,
    }
}

/// One missing row as the list shows it: the track, and its decoded file
/// path (`None` when the Location can't be decoded). `None` for a streaming
/// entry, which is never listed. Bad data in the row only leaves the path
/// out; it never fails.
fn read(
    id: i64,
    location: &str,
    title: String,
    artist: String,
) -> Option<(MissingTrack, Option<FilePath>)> {
    let mut track = MissingTrack {
        id,
        title,
        artist,
        last_known_path: None,
    };
    match decode(location) {
        Ok(Location::Streaming(_)) => None,
        Ok(Location::File(path)) => {
            track.last_known_path = Some(shown(&path));
            // Like every list: the file's name stands in for a missing title.
            if track.title.trim().is_empty() {
                track.title = crate::send_values::file_name(path.as_str()).to_owned();
            }
            Some((track, Some(path)))
        }
        Err(_) => Some((track, None)),
    }
}

/// A group's ids and first page; the folder and the offer are filled in by
/// the caller.
fn page_of(tracks: Vec<MissingTrack>) -> MissingGroup {
    MissingGroup {
        folder: None,
        can_add: false,
        track_ids: tracks.iter().map(|t| t.id).collect(),
        tracks: tracks.into_iter().take(FIRST_PAGE).collect(),
    }
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

/// The rows for a page: the given ids (a JSON array, `?1`) that still have
/// no file. Each id is found through the primary key.
const PAGE_SQL: &str = "SELECT id, CAST(location AS TEXT),
        coalesce(CAST(json_extract(attributes, '$.Name') AS TEXT), ''),
        coalesce(CAST(json_extract(attributes, '$.Artist') AS TEXT), '')
 FROM rekordbox_track
 WHERE id IN (SELECT value FROM json_each(?1)) AND file_id IS NULL";

/// The tracks for `ids`, in the order given. An id that is no longer
/// missing (it was matched since the list was read) is left out, and so is
/// any beyond [`MAX_PAGE`].
pub fn page(conn: &Connection, ids: &[i64]) -> rusqlite::Result<Vec<MissingTrack>> {
    let ids = &ids[..ids.len().min(MAX_PAGE)];
    let wanted = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
    let mut stmt = conn.prepare(PAGE_SQL)?;
    let mut found: BTreeMap<i64, MissingTrack> = BTreeMap::new();
    let rows = stmt.query_map([wanted], |r| {
        Ok((r.get(0)?, r.get::<_, String>(1)?, r.get(2)?, r.get(3)?))
    })?;
    for row in rows {
        let (id, location, title, artist) = row?;
        if let Some((track, _)) = read(id, &location, title, artist) {
            found.insert(id, track);
        }
    }
    Ok(ids.iter().filter_map(|id| found.remove(id)).collect())
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
        use windows_sys::Win32::System::Diagnostics::Debug::{
            SetThreadErrorMode, SEM_FAILCRITICALERRORS,
        };
        // An empty card reader fails the call instead of showing Windows'
        // "no disk in the drive" dialog. The mode is put back right after.
        let mut before = 0;
        // SAFETY: plain calls on this thread's error mode; `before` is a
        // valid out-pointer, and null is allowed for the second call.
        let changed = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, &mut before) } != 0;
        // The `\\?\` form, so names ending in a dot or space open as written.
        let meta = std::fs::metadata(format!(r"\\?\{folder}"));
        // Only put back what was really read: a failed first call left
        // `before` at 0, which isn't known to be the old mode.
        if changed {
            // SAFETY: as above.
            unsafe { SetThreadErrorMode(before, std::ptr::null_mut()) };
        }
        match meta {
            Ok(meta) => usable_folder(meta.is_dir(), crate::scan::online_only::attributes(&meta)),
            Err(_) => false,
        }
    }
}

/// Whether what the disk reported is a folder to offer: a directory that
/// isn't an online-only placeholder.
#[cfg(windows)]
fn usable_folder(is_dir: bool, attributes: u32) -> bool {
    is_dir && !crate::scan::online_only::is_online_only(attributes)
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
    // The disk is asked off the async threads, after the database
    // connection is given back.
    let list = tauri::async_runtime::spawn_blocking(move || -> Result<MissingList, IpcError> {
        let gathered = reads.read(gather)?;
        Ok(group(gathered, &crate::scan::system_volumes(), &SystemDisk))
    })
    .await
    .map_err(|_| IpcError::new(ErrorKind::Internal))??;
    Ok(list)
}

/// The next tracks of a group: the ones with these ids (from the list's
/// `track_ids`), in that order. At most [`MAX_PAGE`] are answered.
#[tauri::command]
#[specta::specta]
pub async fn missing_track_page(
    ids: Vec<i64>,
    reads: State<'_, ReadPool>,
) -> Result<Vec<MissingTrack>, IpcError> {
    let reads = reads.inner().clone();
    let tracks = tauri::async_runtime::spawn_blocking(move || reads.read(|c| page(c, &ids)))
        .await
        .map_err(|_| IpcError::new(ErrorKind::Internal))??;
    Ok(tracks)
}
