//! Missing list tests. Everything is synthetic: made-up volumes, folders
//! and tracks in a migrated database in a temp dir, and a fake disk that
//! records what it was asked. No real path is touched.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::*;
use crate::db::Writer;
use crate::rekordbox::location;
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

/// A fake PC: drive `D:` is one internal volume.
struct Pc {
    id: VolumeId,
}

impl Pc {
    fn new() -> Pc {
        Pc {
            id: identity(IdentitySignals {
                kind: VolumeKind::Internal,
                unc_share: None,
                serial: Some(7),
                filesystem: "NTFS",
                guid: None,
            })
            .unwrap(),
        }
    }
}

impl Volumes for Pc {
    fn volume_for(&self, path: &Path) -> io::Result<Volume> {
        let text = path.to_string_lossy().to_uppercase();
        if !text.starts_with(r"\\?\D:") {
            return Err(io::Error::new(io::ErrorKind::NotFound, "not on D:"));
        }
        Ok(Volume {
            id: self.id.clone(),
            label: String::new(),
            mount_path: PathBuf::from(r"D:\"),
            kind: VolumeKind::Internal,
        })
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (*id == self.id).then(|| PathBuf::from(r"D:\"))
    }
}

/// What's on the fake disk, and what it was asked.
#[derive(Default)]
struct Disk {
    connected: HashSet<char>,
    folders: HashSet<String>,
    asked_folders: Mutex<Vec<String>>,
}

impl Disk {
    fn with(connected: &[char], folders: &[&str]) -> Disk {
        Disk {
            connected: connected.iter().copied().collect(),
            folders: folders.iter().map(|f| (*f).to_owned()).collect(),
            asked_folders: Mutex::default(),
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked_folders.lock().unwrap().clone()
    }
}

impl DiskProbe for Disk {
    fn drive_connected(&self, letter: char) -> bool {
        self.connected.contains(&letter.to_ascii_uppercase())
    }

    fn folder_exists(&self, folder: &str) -> bool {
        self.asked_folders.lock().unwrap().push(folder.to_owned());
        self.folders.contains(folder)
    }
}

struct Lib {
    _dir: tempfile::TempDir,
    writer: Writer,
    next: std::cell::Cell<i64>,
}

impl Lib {
    fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        Lib {
            _dir: dir,
            writer,
            next: std::cell::Cell::new(1),
        }
    }

    fn exec(&self, sql: &'static str, values: Vec<rusqlite::types::Value>) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, rusqlite::params_from_iter(values))?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// A rekordbox track at `location`, with no file.
    fn track(&self, location: &str, name: &str, artist: &str) -> i64 {
        let id = self.next.get();
        self.next.set(id + 1);
        let attrs = serde_json::json!({
            "TrackID": id.to_string(), "Name": name, "Artist": artist, "Location": location
        });
        let key = location::decode(location)
            .map(|l| l.match_key())
            .unwrap_or_else(|_| "undecodable".into());
        self.exec(
            "INSERT INTO rekordbox_track (attributes, location_key, read_at)
             VALUES (?1, ?2, '2026-09-30T12:00:00.000Z')",
            vec![attrs.to_string().into(), key.into()],
        )
    }

    /// A music folder at `rel` on the fake PC's `D:`.
    fn music_folder(&self, rel: &str) -> i64 {
        let volume = self.writer.call(|c| {
            c.query_row("SELECT id FROM volume", [], |r| r.get(0))
                .or_else(|_| {
                    c.execute(
                        "INSERT INTO volume (identity, kind, last_mount_path)
                         VALUES (?1, 'internal', 'D:\\')",
                        [Pc::new().id.as_str()],
                    )?;
                    Ok(c.last_insert_rowid())
                })
        });
        let volume: i64 = volume.unwrap();
        self.exec(
            "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (?1, ?2, ?2)",
            vec![volume.into(), rel.to_owned().into()],
        )
    }

    /// Matches `track` to a new file, the way relink does.
    fn match_to_file(&self, track: i64, probable: bool) {
        let folder = self.music_folder(&format!("Matched{track}"));
        let file = self.exec(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size)
             VALUES (?1, 'a.mp3', 'a.mp3', 1)",
            vec![folder.into()],
        );
        self.exec(
            "UPDATE rekordbox_track
             SET file_id = ?2, relink_method = 'path', relink_probable = ?3 WHERE id = ?1",
            vec![track.into(), file.into(), i64::from(probable).into()],
        );
    }

    fn list(&self, disk: &Disk) -> MissingList {
        let path = crate::write_guard::test_path(self._dir.path(), crate::db::DB_FILE_NAME);
        let gathered = crate::db::ReadPool::open(&path)
            .unwrap()
            .read(gather)
            .unwrap();
        group(gathered, &Pc::new(), disk)
    }
}

const OLD: &str = "file://localhost/D:/Old%20Music/Set/";

#[test]
fn a_track_with_no_file_is_listed_with_its_last_known_path() {
    let lib = Lib::new();
    let id = lib.track(&format!("{OLD}a.mp3"), "Song", "Artist");
    let list = lib.list(&Disk::default());
    assert_eq!(list.total, 1);
    assert_eq!(
        list.groups[0].tracks,
        vec![MissingTrack {
            id,
            title: "Song".into(),
            artist: "Artist".into(),
            last_known_path: Some(r"D:\Old Music\Set\a.mp3".into()),
        }]
    );
}

#[test]
fn matched_tracks_are_not_missing_and_neither_are_probable_matches() {
    let lib = Lib::new();
    let missing = lib.track(&format!("{OLD}a.mp3"), "Missing", "A");
    let matched = lib.track(&format!("{OLD}b.mp3"), "Matched", "A");
    let probable = lib.track(&format!("{OLD}c.mp3"), "Probable", "A");
    lib.match_to_file(matched, false);
    lib.match_to_file(probable, true);
    let list = lib.list(&Disk::default());
    assert_eq!(list.total, 1);
    assert_eq!(list.groups[0].tracks[0].id, missing);
}

#[test]
fn streaming_entries_never_appear() {
    let lib = Lib::new();
    lib.track("soundcloud:tracks:1234", "Stream", "A");
    lib.track("spotify:track:abc", "Stream 2", "A");
    let list = lib.list(&Disk::default());
    assert_eq!((list.total, list.groups), (0, vec![]));
}

#[test]
fn tracks_are_grouped_by_last_known_folder_ignoring_letter_case() {
    let lib = Lib::new();
    lib.track("file://localhost/D:/Crate/a.mp3", "B", "Zed");
    lib.track("file://localhost/D:/crate/b.mp3", "A", "Zed");
    lib.track("file://localhost/D:/Other/c.mp3", "C", "Yan");
    let list = lib.list(&Disk::default());
    let shown: Vec<_> = list
        .groups
        .iter()
        .map(|g| (g.folder.clone().unwrap(), g.tracks.len()))
        .collect();
    assert_eq!(
        shown,
        vec![(r"D:\Crate".to_owned(), 2), (r"D:\Other".to_owned(), 1)]
    );
    // Within a group: by artist, then title.
    let titles: Vec<_> = list.groups[0].tracks.iter().map(|t| &*t.title).collect();
    assert_eq!(titles, ["A", "B"]);
}

#[test]
fn a_file_at_the_top_of_a_drive_has_the_drive_as_its_folder() {
    let lib = Lib::new();
    lib.track("file://localhost/D:/a.mp3", "A", "A");
    let list = lib.list(&Disk::default());
    assert_eq!(list.groups[0].folder.as_deref(), Some(r"D:\"));
}

#[test]
fn a_location_that_cant_be_decoded_is_listed_without_a_folder_or_path() {
    let lib = Lib::new();
    lib.track("file://localhost/D:/Known/a.mp3", "A", "A");
    let id = lib.track("not a location", "Odd", "A");
    let list = lib.list(&Disk::default());
    assert_eq!(list.total, 2);
    let last = list.groups.last().unwrap();
    assert_eq!((last.folder.clone(), last.can_add), (None, false));
    assert_eq!(last.tracks[0].id, id);
    assert_eq!(last.tracks[0].last_known_path, None);
}

#[test]
fn an_existing_folder_outside_the_music_folders_can_be_added() {
    let lib = Lib::new();
    lib.track(&format!("{OLD}a.mp3"), "A", "A");
    lib.music_folder("Elsewhere");
    let disk = Disk::with(&['D'], &[r"D:\Old Music\Set"]);
    assert!(lib.list(&disk).groups[0].can_add);
}

#[test]
fn a_folder_that_is_gone_is_not_offered() {
    let lib = Lib::new();
    lib.track(&format!("{OLD}a.mp3"), "A", "A");
    let disk = Disk::with(&['D'], &[]);
    assert!(!lib.list(&disk).groups[0].can_add);
}

#[test]
fn a_folder_inside_a_music_folder_is_not_offered() {
    let lib = Lib::new();
    lib.track(&format!("{OLD}a.mp3"), "A", "A");
    lib.music_folder("Old Music");
    let disk = Disk::with(&['D'], &[r"D:\Old Music\Set"]);
    assert!(!lib.list(&disk).groups[0].can_add);
}

#[test]
fn a_sibling_sharing_a_music_folders_name_prefix_is_still_offered() {
    let lib = Lib::new();
    lib.track("file://localhost/D:/Old%20Music2/a.mp3", "A", "A");
    lib.music_folder("Old Music");
    let disk = Disk::with(&['D'], &[r"D:\Old Music2"]);
    assert!(lib.list(&disk).groups[0].can_add);
}

#[cfg(windows)]
#[test]
fn only_a_directory_that_is_really_there_counts_as_a_usable_folder() {
    use crate::scan::online_only::{ATTRIBUTE_RECALL_ON_DATA_ACCESS, ATTRIBUTE_RECALL_ON_OPEN};
    assert!(usable_folder(true, 0));
    assert!(!usable_folder(false, 0));
    assert!(!usable_folder(true, ATTRIBUTE_RECALL_ON_DATA_ACCESS));
    assert!(!usable_folder(true, ATTRIBUTE_RECALL_ON_OPEN));
}

#[test]
fn a_folder_that_holds_a_music_folder_is_not_offered() {
    let lib = Lib::new();
    lib.track(&format!("{OLD}a.mp3"), "A", "A");
    lib.music_folder("Old Music/Set/Deeper");
    let disk = Disk::with(&['D'], &[r"D:\Old Music\Set"]);
    assert!(!lib.list(&disk).groups[0].can_add);
}

#[test]
fn a_disconnected_drive_is_not_asked_about_its_folders() {
    let lib = Lib::new();
    lib.track("file://localhost/F:/Gone/a.mp3", "A", "A");
    let disk = Disk::with(&['D'], &[r"F:\Gone"]);
    let list = lib.list(&disk);
    assert!(!list.groups[0].can_add);
    assert_eq!(disk.asked(), Vec::<String>::new());
}

#[test]
fn network_and_mac_paths_are_listed_but_never_checked_or_offered() {
    let lib = Lib::new();
    lib.track("file://localhost//nas/share/a.mp3", "A", "A");
    lib.track("file://localhost/Users/someone/Music/b.mp3", "B", "A");
    let disk = Disk::with(&['D'], &[]);
    let list = lib.list(&disk);
    assert_eq!(list.total, 2);
    assert!(list.groups.iter().all(|g| !g.can_add));
    assert_eq!(disk.asked(), Vec::<String>::new());
}

#[test]
fn with_no_missing_tracks_the_list_is_empty() {
    let lib = Lib::new();
    let list = lib.list(&Disk::default());
    assert_eq!((list.total, list.groups), (0, vec![]));
}
