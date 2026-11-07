#![cfg(test)]
#![cfg(windows)]
//! 4.1: a music folder that moved is pointed at its new place.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::support::{db, link_dir, TempVolume};
use super::walk::{rows, scan};
use crate::db::Writer;
use crate::ops::{undo_last_via, UndoOutcome};
use crate::paths::Volumes;
use crate::scan::folders::{add, MusicFolderId};
use crate::scan::relocate::{apply, relocate, Plan, RelocateError};
use crate::scan::MusicFolderRole;
use crate::volume::{Volume, VolumeId};

/// Two "drives": the one the folder was on (unplugged in these tests) and
/// the one it is on now.
#[derive(Clone)]
struct Pair {
    old: TempVolume,
    new: TempVolume,
}

impl Volumes for Pair {
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }
    fn volume_for(&self, path: &Path) -> io::Result<Volume> {
        self.new
            .volume_for(path)
            .or_else(|_| self.old.volume_for(path))
    }
    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        self.new.mount_path(id).or_else(|| self.old.mount_path(id))
    }
}

const FILES: [(&str, &str); 3] = [
    ("a.mp3", "aaaa"),
    ("House/b.mp3", "bbbbbb"),
    ("House/Deep/c.mp3", "c"),
];

fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

struct Setup {
    _old_dir: tempfile::TempDir,
    _new_dir: tempfile::TempDir,
    _db: tempfile::TempDir,
    writer: Writer,
    drives: Arc<Pair>,
    folder: MusicFolderId,
}

/// A folder "Music" on the old drive with three known files, the old drive
/// unplugged, and the same music under "Moved/Music" on the new drive.
fn setup() -> Setup {
    setup_with(&FILES, FILES.len())
}

/// [`setup`] with these known files, of which the first `at_new` are at the
/// new place.
fn setup_with(files: &[(&str, &str)], at_new: usize) -> Setup {
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let drives = Arc::new(Pair {
        old: TempVolume::new(old_dir.path(), 11),
        new: TempVolume::new(new_dir.path(), 22),
    });
    fs::create_dir_all(drives.old.mount.join("Music")).unwrap();
    let (db_dir, writer, _reads) = db();
    let folder = add(
        &writer,
        &drives.old,
        &drives.old.mount.join("Music"),
        MusicFolderRole::Scan,
    )
    .unwrap()
    .id;
    for (n, (rel, body)) in files.iter().enumerate() {
        if n < at_new {
            put(&drives.new.mount.join("Moved/Music"), rel, body);
        }
        let size = body.len() as i64;
        let rel = rel.to_string();
        writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size)
                     VALUES (?1, ?2, ?2, ?3)",
                    (folder.0, rel, size),
                )
            })
            .unwrap();
    }
    drives.old.set_online(false);
    Setup {
        _old_dir: old_dir,
        _new_dir: new_dir,
        _db: db_dir,
        writer,
        drives,
        folder,
    }
}

fn place(writer: &Writer, folder: MusicFolderId) -> (String, String) {
    writer
        .call(move |c| {
            c.query_row(
                "SELECT v.identity, mf.rel_path FROM music_folder mf
                 JOIN volume v ON v.id = mf.volume_id WHERE mf.id = ?1",
                [folder.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap()
}

fn file_ids(writer: &Writer) -> Vec<(i64, String)> {
    writer
        .call(|c| {
            let mut stmt = c.prepare("SELECT id, rel_path FROM file ORDER BY id")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap()
}

fn moved_to(s: &Setup) -> PathBuf {
    s.drives.new.mount.join("Moved/Music")
}

#[test]
fn a_moved_folder_is_pointed_at_its_new_place_and_its_files_keep_their_ids() {
    let s = setup();
    let before = file_ids(&s.writer);
    let done = relocate(&s.writer, &s.drives, s.folder, &moved_to(&s)).unwrap();
    assert_eq!((done.found, done.checked), (3, 3));
    assert_eq!(
        place(&s.writer, s.folder),
        (
            s.drives.new.id.as_str().to_owned(),
            "Moved/Music".to_owned()
        )
    );
    assert_eq!(file_ids(&s.writer), before);
}

#[test]
fn a_folder_that_is_reachable_is_refused() {
    let s = setup();
    s.drives.old.set_online(true);
    let refused = relocate(&s.writer, &s.drives, s.folder, &moved_to(&s)).unwrap_err();
    assert_eq!(refused, RelocateError::NotMissing);
    assert_eq!(place(&s.writer, s.folder).1, "Music");
}

#[test]
fn a_folder_without_the_known_files_is_refused_and_nothing_changes() {
    let s = setup();
    let elsewhere = s.drives.new.mount.join("Other");
    put(&elsewhere, "a.mp3", "aaaa");
    // Same name, wrong size, is not the same file.
    put(&elsewhere, "House/b.mp3", "different");
    let before = place(&s.writer, s.folder);
    let refused = relocate(&s.writer, &s.drives, s.folder, &elsewhere).unwrap_err();
    assert_eq!(
        refused,
        RelocateError::TooFewFiles {
            found: 1,
            checked: 3
        }
    );
    assert_eq!(place(&s.writer, s.folder), before);
    let operations: i64 = s
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(operations, 0);
}

#[test]
fn a_place_inside_or_around_another_music_folder_is_refused() {
    let s = setup();
    let other = s.drives.new.mount.join("Moved");
    // Another music folder around the new place.
    add(&s.writer, &*s.drives, &other, MusicFolderRole::Scan).unwrap();
    let before = place(&s.writer, s.folder);
    let refused = relocate(&s.writer, &s.drives, s.folder, &moved_to(&s)).unwrap_err();
    assert!(
        matches!(refused, RelocateError::Overlaps { .. }),
        "{refused:?}"
    );
    assert_eq!(place(&s.writer, s.folder), before);
}

#[test]
fn undo_puts_the_old_place_back() {
    let s = setup();
    let before = place(&s.writer, s.folder);
    relocate(&s.writer, &s.drives, s.folder, &moved_to(&s)).unwrap();
    assert_ne!(place(&s.writer, s.folder), before);
    let undone = undo_last_via(&s.writer).unwrap();
    assert!(matches!(undone, UndoOutcome::Undone { .. }), "{undone:?}");
    assert_eq!(place(&s.writer, s.folder), before);
}

fn volume_rows(writer: &Writer) -> i64 {
    writer
        .call(|c| c.query_row("SELECT count(*) FROM volume", [], |r| r.get(0)))
        .unwrap()
}

#[test]
fn a_folder_that_came_back_before_the_change_is_refused_and_nothing_is_stored() {
    let s = setup();
    // The checks passed while the drive was away; it is back by the time
    // the change is made.
    let new = &s.drives.new;
    let plan = Plan::for_test(
        s.folder,
        new.volume_for(&moved_to(&s)).unwrap(),
        "Moved/Music",
    );
    s.drives.old.set_online(true);
    let known = volume_rows(&s.writer);
    let refused = apply(&s.writer, &s.drives, plan).unwrap_err();
    assert_eq!(refused, RelocateError::NotMissing);
    assert_eq!(place(&s.writer, s.folder).1, "Music");
    // The new drive's volume row was rolled back with the refusal.
    assert_eq!(volume_rows(&s.writer), known);
}

#[test]
fn four_of_five_checked_files_are_enough_and_three_of_five_are_not() {
    let five = [
        ("a.mp3", "1"),
        ("b.mp3", "22"),
        ("c.mp3", "333"),
        ("d.mp3", "4444"),
        ("e.mp3", "55555"),
    ];
    let s = setup_with(&five, 4);
    let done = relocate(&s.writer, &s.drives, s.folder, &moved_to(&s)).unwrap();
    assert_eq!((done.found, done.checked), (4, 5));

    let s = setup_with(&five, 3);
    let refused = relocate(&s.writer, &s.drives, s.folder, &moved_to(&s)).unwrap_err();
    assert_eq!(
        refused,
        RelocateError::TooFewFiles {
            found: 3,
            checked: 5
        }
    );
}

#[test]
fn a_folder_with_no_known_files_has_nothing_to_be_recognised_by() {
    let s = setup_with(&[], 0);
    fs::create_dir_all(moved_to(&s)).unwrap();
    let refused = relocate(&s.writer, &s.drives, s.folder, &moved_to(&s)).unwrap_err();
    assert_eq!(refused, RelocateError::NoKnownFiles);
    assert_eq!(place(&s.writer, s.folder).1, "Music");
}

#[test]
fn a_junction_as_the_new_place_is_stored_where_it_really_is() {
    let s = setup();
    let link = s.drives.new.mount.join("Shortcut");
    link_dir(&link, &moved_to(&s));
    relocate(&s.writer, &s.drives, s.folder, &link).unwrap();
    assert_eq!(place(&s.writer, s.folder).1, "Moved/Music");
}

#[test]
fn undo_after_the_rescan_still_puts_the_old_place_back() {
    let s = setup();
    let before = place(&s.writer, s.folder);
    let ids = file_ids(&s.writer);
    relocate(&s.writer, &s.drives, s.folder, &moved_to(&s)).unwrap();
    // The rescan the command queues, run to the end.
    scan(&s.writer, &s.drives.new);
    assert_eq!(file_ids(&s.writer), ids);
    assert!(rows(&s.writer).iter().all(|r| r.present));
    let undone = undo_last_via(&s.writer).unwrap();
    assert!(matches!(undone, UndoOutcome::Undone { .. }), "{undone:?}");
    assert_eq!(place(&s.writer, s.folder), before);
    assert_eq!(file_ids(&s.writer), ids);
}
