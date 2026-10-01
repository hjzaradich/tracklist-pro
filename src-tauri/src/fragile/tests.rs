//! Fragile-location tests. Synthetic paths and volumes only; the pure rule
//! is checked with made-up Downloads and temp folders, and the query with
//! a migrated database in a temp dir.

use std::io;

use super::*;
use crate::db::Writer;
use crate::volume::Volume;

fn dirs() -> FragileDirs {
    FragileDirs {
        downloads: Some(PathBuf::from(r"C:\Users\someone\Downloads")),
        temp: vec![
            PathBuf::from(r"C:\Users\someone\AppData\Local\Temp"),
            PathBuf::from(r"C:\Windows\Temp"),
        ],
    }
}

fn why(path: &str, kind: VolumeKind) -> Option<FragileReason> {
    reason(Path::new(path), kind, &dirs())
}

#[test]
fn a_file_in_downloads_is_fragile_for_that_reason() {
    assert_eq!(
        why(r"C:\Users\someone\Downloads\a.mp3", VolumeKind::Internal),
        Some(FragileReason::Downloads)
    );
    assert_eq!(
        why(
            r"C:\Users\someone\Downloads\Sub\Deeper\a.mp3",
            VolumeKind::Internal
        ),
        Some(FragileReason::Downloads)
    );
}

#[test]
fn downloads_is_the_known_folder_not_a_folder_with_that_name() {
    // The known folder was moved to D:\Stuff; a folder called Downloads
    // elsewhere is just a folder.
    let moved = FragileDirs {
        downloads: Some(PathBuf::from(r"D:\Stuff")),
        temp: vec![],
    };
    let check = |p: &str| reason(Path::new(p), VolumeKind::Internal, &moved);
    assert_eq!(check(r"D:\Stuff\a.mp3"), Some(FragileReason::Downloads));
    assert_eq!(check(r"C:\Users\someone\Downloads\a.mp3"), None);
}

#[test]
fn a_file_in_a_temp_folder_is_fragile_for_that_reason() {
    for path in [
        r"C:\Users\someone\AppData\Local\Temp\x\a.mp3",
        r"C:\Windows\Temp\a.mp3",
    ] {
        assert_eq!(
            why(path, VolumeKind::Internal),
            Some(FragileReason::Temp),
            "{path}"
        );
    }
}

#[test]
fn folders_are_matched_whole_ignoring_case_and_the_verbatim_prefix() {
    assert_eq!(
        why(
            r"\\?\c:\users\SOMEONE\downloads\a.mp3",
            VolumeKind::Internal
        ),
        Some(FragileReason::Downloads)
    );
    // A sibling that merely starts with the same letters isn't inside.
    assert_eq!(
        why(
            r"C:\Users\someone\Downloads old\a.mp3",
            VolumeKind::Internal
        ),
        None
    );
    // Nor is the folder's parent.
    assert_eq!(why(r"C:\Users\someone\a.mp3", VolumeKind::Internal), None);
}

#[test]
fn a_file_on_an_external_drive_is_fragile_for_that_reason() {
    assert_eq!(
        why(r"E:\Music\a.mp3", VolumeKind::External),
        Some(FragileReason::External)
    );
}

#[test]
fn a_file_on_a_network_drive_is_fragile_for_that_reason() {
    assert_eq!(
        why(r"\\nas\share\a.mp3", VolumeKind::Network),
        Some(FragileReason::Network)
    );
}

#[test]
fn downloads_on_an_external_drive_is_called_downloads_first() {
    let dirs = FragileDirs {
        downloads: Some(PathBuf::from(r"E:\Downloads")),
        temp: vec![],
    };
    assert_eq!(
        reason(
            Path::new(r"E:\Downloads\a.mp3"),
            VolumeKind::External,
            &dirs
        ),
        Some(FragileReason::Downloads)
    );
}

#[test]
fn a_file_on_an_internal_drive_outside_those_folders_is_not_fragile() {
    assert_eq!(why(r"D:\Music\a.mp3", VolumeKind::Internal), None);
    assert_eq!(
        why(r"C:\Users\someone\Music\a.mp3", VolumeKind::Internal),
        None
    );
}

#[test]
fn no_known_downloads_or_temp_folder_leaves_only_the_drive_kind() {
    let none = FragileDirs::default();
    assert_eq!(
        reason(Path::new(r"C:\a.mp3"), VolumeKind::Internal, &none),
        None
    );
    assert_eq!(
        reason(Path::new(r"E:\a.mp3"), VolumeKind::External, &none),
        Some(FragileReason::External)
    );
}

// --- the query ---

/// One volume mounted at `mount` (or not mounted, with a last mount).
struct Mounted(Option<PathBuf>);

impl Volumes for Mounted {
    fn volume_for(&self, _path: &Path) -> io::Result<Volume> {
        Err(io::Error::other("not used"))
    }

    fn mount_path(&self, _id: &VolumeId) -> Option<PathBuf> {
        self.0.clone()
    }
}

struct Lib {
    _dir: tempfile::TempDir,
    writer: Writer,
}

impl Lib {
    fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        Lib { _dir: dir, writer }
    }

    /// A file at `folder`/`rel` on a volume of `kind` last mounted at
    /// `last_mount`. Returns the file's id.
    fn file(&self, kind: &'static str, last_mount: &'static str, folder: &str, rel: &str) -> i64 {
        let (folder, rel) = (folder.to_owned(), rel.to_owned());
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES ('serial=NTFS-0000000A', ?1, ?2)
                     ON CONFLICT (identity) DO UPDATE SET kind = excluded.kind,
                         last_mount_path = excluded.last_mount_path",
                    [kind, last_mount],
                )?;
                let volume: i64 = c.query_row("SELECT id FROM volume", [], |r| r.get(0))?;
                c.execute(
                    "INSERT OR IGNORE INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (?1, ?2, ?2)",
                    (volume, &folder),
                )?;
                let mf: i64 = c.query_row(
                    "SELECT id FROM music_folder WHERE rel_path = ?1",
                    [&folder],
                    |r| r.get(0),
                )?;
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size)
                     VALUES (?1, ?2, ?2, 1)",
                    (mf, &rel),
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    fn reasons(&self, volumes: Mounted, files: Vec<i64>) -> HashMap<i64, FragileReason> {
        self.writer
            .call(move |c| fragile_reasons(c, &volumes, &dirs(), &files))
            .unwrap()
    }
}

#[test]
fn files_are_judged_by_where_their_volume_is_mounted_now() {
    let lib = Lib::new();
    let in_downloads = lib.file("internal", r"C:\", "Users/someone/Downloads", "a.mp3");
    let elsewhere = lib.file("internal", r"C:\", "Users/someone/Music", "b.mp3");
    let got = lib.reasons(
        Mounted(Some(PathBuf::from(r"C:\"))),
        vec![in_downloads, elsewhere],
    );
    assert_eq!(got.get(&in_downloads), Some(&FragileReason::Downloads));
    assert_eq!(got.get(&elsewhere), None);
}

#[test]
fn a_file_with_no_reason_is_left_out_of_the_answer() {
    let lib = Lib::new();
    let fine = lib.file("internal", r"D:\", "Music", "a.mp3");
    let got = lib.reasons(Mounted(Some(PathBuf::from(r"D:\"))), vec![fine]);
    assert!(got.is_empty());
}

#[test]
fn an_unplugged_external_drive_is_still_fragile_by_its_kind() {
    let lib = Lib::new();
    let file = lib.file("external", r"E:\", "Music", "a.mp3");
    let got = lib.reasons(Mounted(None), vec![file]);
    assert_eq!(got.get(&file), Some(&FragileReason::External));
}

#[test]
fn a_network_volume_is_fragile_by_its_kind() {
    let lib = Lib::new();
    let file = lib.file("network", r"\\nas\share\", "Music", "a.mp3");
    let got = lib.reasons(Mounted(Some(PathBuf::from(r"\\nas\share\"))), vec![file]);
    assert_eq!(got.get(&file), Some(&FragileReason::Network));
}

#[test]
fn an_unknown_file_id_is_left_out() {
    let lib = Lib::new();
    assert!(lib.reasons(Mounted(None), vec![999]).is_empty());
}
