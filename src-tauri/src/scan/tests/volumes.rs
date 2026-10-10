#![cfg(test)]
//! 1aB-9: after drives come or go, the `volume` rows note where each known
//! volume is now, and what the library remembers feeds the clone rule.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::support::{db, serial};
use crate::db::Writer;
use crate::paths::Volumes;
use crate::scan::folders::{add, stored};
use crate::scan::volumes::{note_mounted, remembered};
use crate::scan::MusicFolderRole;
use crate::volume::{Remembered, Volume, VolumeId, VolumeKind};

const GUID_A: &str = "{aaaaaaaa-0000-0000-0000-000000000001}";
const GUID_B: &str = "{bbbbbbbb-0000-0000-0000-000000000002}";

fn volume(id: &VolumeId, mount: &str, label: &str) -> Volume {
    Volume {
        id: id.clone(),
        label: label.to_owned(),
        mount_path: PathBuf::from(mount),
        kind: VolumeKind::External,
    }
}

/// Each volume row: identity, label, guid, last mount path.
fn volume_rows(writer: &Writer) -> Vec<(String, String, Option<String>, Option<String>)> {
    writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT identity, label, guid, last_mount_path FROM volume ORDER BY identity",
            )?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })
        .unwrap()
}

fn insert_volume(writer: &Writer, identity: &str, guid: Option<&str>, mount: &str) {
    let (identity, guid, mount) = (
        identity.to_owned(),
        guid.map(str::to_owned),
        mount.to_owned(),
    );
    writer
        .call(move |c| {
            c.execute(
                "INSERT INTO volume (identity, label, guid, kind, last_mount_path)
                 VALUES (?1, 'GIG USB', ?2, 'external', ?3)",
                (identity, guid, mount),
            )
        })
        .unwrap();
}

#[test]
fn a_known_volume_is_noted_where_it_is_now_and_an_unknown_one_is_not_added() {
    let (_db, writer, _reads) = db();
    let known = serial(1);
    insert_volume(&writer, known.as_str(), None, r"E:\");

    let mounted = vec![
        (volume(&known, r"F:\", "GIG USB 2"), Some(GUID_A.to_owned())),
        (
            volume(&serial(2), r"G:\", "STRANGER"),
            Some(GUID_B.to_owned()),
        ),
    ];
    let updated = writer.call(move |c| note_mounted(c, &mounted)).unwrap();
    assert_eq!(updated, 1);
    assert_eq!(
        volume_rows(&writer),
        [(
            known.as_str().to_owned(),
            "GIG USB 2".to_owned(),
            Some(GUID_A.to_owned()),
            Some(r"F:\".to_owned())
        )]
    );
}

#[test]
fn a_backup_never_told_apart_plugged_in_alone_never_writes_its_guid_into_the_originals_row() {
    let (_db, writer, _reads) = db();
    let shared = serial(1);
    insert_volume(&writer, shared.as_str(), Some(GUID_A), r"E:\");

    // Alone and never told apart, the backup answers to the plain serial.
    let mounted = vec![(volume(&shared, r"G:\", "GIG USB"), Some(GUID_B.to_owned()))];
    writer.call(move |c| note_mounted(c, &mounted)).unwrap();
    let rows = volume_rows(&writer);
    assert_eq!(rows[0].2.as_deref(), Some(GUID_A));
    // Where it was last seen still follows the drive letter.
    assert_eq!(rows[0].3.as_deref(), Some(r"G:\"));
}

#[test]
fn a_volume_with_no_remembered_guid_gets_the_one_it_reports() {
    let (_db, writer, _reads) = db();
    insert_volume(&writer, serial(1).as_str(), None, r"E:\");
    let mounted = vec![(
        volume(&serial(1), r"E:\", "GIG USB"),
        Some(GUID_A.to_owned()),
    )];
    writer.call(move |c| note_mounted(c, &mounted)).unwrap();
    assert_eq!(volume_rows(&writer)[0].2.as_deref(), Some(GUID_A));
}

#[test]
fn a_backup_the_library_knows_updates_its_own_row_and_not_the_originals() {
    let (_db, writer, _reads) = db();
    let shared = serial(1);
    let backup = shared.with_guid(GUID_B);
    insert_volume(&writer, shared.as_str(), Some(GUID_A), r"E:\");
    insert_volume(&writer, backup.as_str(), Some(GUID_B), r"F:\");

    // The clone rule names it by its own identity (volume::tell_clones_apart).
    let mounted = vec![(volume(&backup, r"G:\", "GIG USB"), Some(GUID_B.to_owned()))];
    writer.call(move |c| note_mounted(c, &mounted)).unwrap();
    let rows = volume_rows(&writer);
    let row = |id: &VolumeId| rows.iter().find(|r| r.0 == id.as_str()).unwrap().clone();
    assert_eq!(row(&shared).2.as_deref(), Some(GUID_A));
    assert_eq!(row(&shared).3.as_deref(), Some(r"E:\"));
    assert_eq!(row(&backup).3.as_deref(), Some(r"G:\"));
}

#[test]
fn the_library_remembers_each_volumes_identity_guid_and_label() {
    let (_db, writer, _reads) = db();
    insert_volume(&writer, serial(1).as_str(), Some(GUID_A), r"E:\");
    insert_volume(&writer, r"unc=\\nas\music", None, r"\\nas\music\");
    let remembered = writer.call(|c| remembered(c)).unwrap();
    assert_eq!(
        remembered,
        [
            Remembered {
                id: serial(1),
                guid: Some(GUID_A.to_owned()),
                label: "GIG USB".to_owned()
            },
            Remembered {
                id: VolumeId::from_stored(r"unc=\\nas\music".to_owned()).unwrap(),
                guid: None,
                label: "GIG USB".to_owned()
            },
        ]
    );
}

/// One drive at a temp folder, reporting a GUID.
struct Drive {
    mount: PathBuf,
    id: VolumeId,
    guid: String,
}

impl Volumes for Drive {
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    fn volume_for(&self, _path: &Path) -> io::Result<Volume> {
        Ok(Volume {
            id: self.id.clone(),
            label: "GIG USB".to_owned(),
            mount_path: self.mount.clone(),
            kind: VolumeKind::External,
        })
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (*id == self.id).then(|| self.mount.clone())
    }

    fn guid(&self, id: &VolumeId) -> Option<String> {
        (*id == self.id).then(|| self.guid.clone())
    }
}

fn drive(dir: &tempfile::TempDir, guid: &str) -> Drive {
    Drive {
        mount: fs::canonicalize(dir.path()).unwrap(),
        id: serial(1),
        guid: guid.to_owned(),
    }
}

// Adds a folder at a real temp path, which only parses as a path on
// Windows (ROADMAP §1.1: elsewhere adding a folder fails cleanly).
#[cfg(windows)]
#[test]
fn a_folder_added_on_a_drive_is_stored_with_the_drives_guid() {
    let dir = tempfile::tempdir().unwrap();
    let drive = drive(&dir, GUID_A);
    let (_db, writer, _reads) = db();
    add(&writer, &drive, &drive.mount, MusicFolderRole::Scan).unwrap();
    assert_eq!(volume_rows(&writer)[0].2.as_deref(), Some(GUID_A));
}

#[cfg(windows)]
#[test]
fn adding_a_folder_on_a_backup_never_told_apart_keeps_the_originals_remembered_guid() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("Backup Music")).unwrap();
    let backup = drive(&dir, GUID_B);
    let (_db, writer, _reads) = db();
    insert_volume(&writer, serial(1).as_str(), Some(GUID_A), r"E:\");

    add(
        &writer,
        &backup,
        &backup.mount.join("Backup Music"),
        MusicFolderRole::Scan,
    )
    .unwrap();
    assert_eq!(volume_rows(&writer)[0].2.as_deref(), Some(GUID_A));
    assert_eq!(writer.call(|c| stored(c)).unwrap().len(), 1);
}
