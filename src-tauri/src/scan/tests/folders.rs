#![cfg(test)]
//! 1aA-3: adding, listing and removing music folders.

use crate::paths::RelPath;
use crate::scan::folders::{overlap, Overlap};

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

#[test]
fn overlap_tells_same_inside_contains_and_apart() {
    assert_eq!(overlap(&rel("Music"), &rel("Music")), Some(Overlap::Same));
    assert_eq!(overlap(&rel("music"), &rel("MUSIC")), Some(Overlap::Same));
    assert_eq!(
        overlap(&rel("Music/House"), &rel("Music")),
        Some(Overlap::Inside)
    );
    assert_eq!(
        overlap(&rel("Music"), &rel("Music/House/Deep")),
        Some(Overlap::Contains)
    );
    // The volume root holds every folder on it.
    assert_eq!(overlap(&rel(""), &rel("Music")), Some(Overlap::Contains));
    assert_eq!(overlap(&rel("Music"), &rel("")), Some(Overlap::Inside));
    // A shared name prefix isn't nesting.
    assert_eq!(overlap(&rel("Music2"), &rel("Music")), None);
    assert_eq!(overlap(&rel("Music"), &rel("Music 2/House")), None);
    // Trailing dots are part of the name: `Q.X.Z.` isn't `Q.X.Z`.
    assert_eq!(overlap(&rel("Q.X.Z./Live"), &rel("Q.X.Z")), None);
}

#[test]
fn nfc_and_nfd_spellings_of_a_folder_are_different_folders() {
    // NTFS doesn't normalize names, so these are two folders on disk.
    let nfc = rel("Caf\u{e9}");
    let nfd = rel("Cafe\u{301}");
    assert_eq!(overlap(&nfc, &nfd), None);
    assert_eq!(overlap(&rel("Caf\u{e9}/A"), &nfd), None);
}

#[test]
fn apple_double_and_ds_store_files_are_skipped_whatever_their_extension() {
    use crate::scan::{is_indexed, is_skipped};
    for skipped in [
        "._Track.mp3",
        "._Track.FLAC",
        "._",
        ".DS_Store",
        ".ds_store",
        ".DS_STORE",
    ] {
        assert!(is_skipped(skipped), "{skipped}");
        assert!(!is_indexed(skipped), "{skipped}");
    }
    // Look-alikes are music.
    for kept in [
        "_Track.mp3",
        "Track._mp3.mp3",
        ".Track.mp3",
        "DS_Store.mp3",
        ".DS_Store.mp3",
    ] {
        assert!(!is_skipped(kept), "{kept}");
        assert!(is_indexed(kept), "{kept}");
    }
}

#[cfg(windows)]
mod on_disk {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::super::support::{db, link_dir, TempVolume};
    use crate::db::{ReadPool, Writer};
    use crate::scan::folders::{self, add, remove, stored, MusicFolderId};
    use crate::scan::{MusicFolder, MusicFolderError, MusicFolderRole};

    /// A temp "drive" with the given folders made on it.
    fn drive(folders: &[&str]) -> (tempfile::TempDir, TempVolume) {
        let dir = tempfile::tempdir().unwrap();
        let volume = TempVolume::new(dir.path(), 0x1A2B_3C4D);
        // Made through the `\?\` mount, so `Q.X.Z.` keeps its dot.
        for f in folders {
            fs::create_dir_all(at(&volume, f)).unwrap();
        }
        (dir, volume)
    }

    fn at(volume: &TempVolume, rel: &str) -> PathBuf {
        let mut path = volume.mount.clone();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            path.push(part);
        }
        path
    }

    fn shown(volume: &TempVolume, rel: &str) -> String {
        crate::scan::display_path(&at(volume, rel))
    }

    fn add_at(
        writer: &Writer,
        volume: &TempVolume,
        rel: &str,
    ) -> Result<MusicFolder, MusicFolderError> {
        add(writer, volume, &at(volume, rel), MusicFolderRole::Scan)
    }

    fn list(reads: &ReadPool, volume: &TempVolume) -> Vec<MusicFolder> {
        reads
            .read(stored)
            .unwrap()
            .iter()
            .map(|f| f.to_music_folder(volume))
            .collect()
    }

    fn count(writer: &Writer, table: &'static str) -> i64 {
        writer
            .call(move |c| c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
            .unwrap()
    }

    #[test]
    fn adding_a_folder_stores_its_volume_and_its_path_from_the_mount_point() {
        let (_dir, volume) = drive(&["DJ Music/House"]);
        let (_db, writer, reads) = db();
        let added = add_at(&writer, &volume, "DJ Music/House").unwrap();
        assert_eq!(added.path, shown(&volume, "DJ Music/House"));
        assert!(added.online);
        assert_eq!(added.role, MusicFolderRole::Scan);
        assert_eq!(added.volume_label, "TEST439041101");

        let (identity, label, kind, rel_path, key): (String, String, String, String, String) =
            writer
                .call(|c| {
                    c.query_row(
                        "SELECT v.identity, v.label, v.kind, mf.rel_path, mf.rel_path_key
                         FROM music_folder mf JOIN volume v ON v.id = mf.volume_id",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                    )
                })
                .unwrap();
        // No drive letter anywhere: the volume by serial, the folder from
        // the mount point.
        assert_eq!(identity, volume.id.as_str());
        assert_eq!(label, volume.label);
        assert_eq!(kind, "external");
        assert_eq!(rel_path, "DJ Music/House");
        assert_eq!(key, "DJ Music/House");

        assert_eq!(list(&reads, &volume), vec![added]);
    }

    #[test]
    fn an_nfd_folder_name_is_stored_as_spelled_with_an_nfc_match_key() {
        // `Café` as a Mac writes it (e + combining accent), and its NFC twin.
        let nfd = "Cafe\u{301} del Mar";
        let nfc = "Caf\u{e9} del Mar";
        let (_dir, volume) = drive(&[nfd, nfc]);
        let (_db, writer, _reads) = db();
        add_at(&writer, &volume, nfd).unwrap();
        // The twin is a different folder on NTFS, so it's added too.
        add_at(&writer, &volume, nfc).unwrap();

        let stored: Vec<(String, String)> = writer
            .call(|c| {
                let mut s =
                    c.prepare("SELECT rel_path, rel_path_key FROM music_folder ORDER BY id")?;
                let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .unwrap();
        assert_eq!(
            stored,
            [
                (nfd.to_owned(), nfc.to_owned()),
                (nfc.to_owned(), nfc.to_owned())
            ]
        );
    }

    #[test]
    fn the_role_is_scan_unless_inbox_is_asked_for() {
        let (_dir, volume) = drive(&["Music", "Downloads"]);
        let (_db, writer, reads) = db();
        add_at(&writer, &volume, "Music").unwrap();
        add(
            &writer,
            &volume,
            &at(&volume, "Downloads"),
            MusicFolderRole::Inbox,
        )
        .unwrap();
        let roles: Vec<_> = list(&reads, &volume).iter().map(|f| f.role).collect();
        assert_eq!(roles, [MusicFolderRole::Scan, MusicFolderRole::Inbox]);
        let stored_roles: Vec<String> = writer
            .call(|c| {
                let mut s = c.prepare("SELECT role FROM music_folder ORDER BY id")?;
                let rows = s.query_map([], |r| r.get(0))?;
                rows.collect()
            })
            .unwrap();
        assert_eq!(stored_roles, ["scan", "inbox"]);
    }

    #[test]
    fn a_folder_inside_an_existing_music_folder_is_refused() {
        let (_dir, volume) = drive(&["Music/House/Deep"]);
        let (_db, writer, _reads) = db();
        add_at(&writer, &volume, "Music").unwrap();
        // Named in any letter case; reported as it's spelled on disk.
        for (inner, on_disk) in [
            ("Music/House", "Music/House"),
            ("Music/House/Deep", "Music/House/Deep"),
            ("music/HOUSE", "Music/House"),
        ] {
            assert_eq!(
                add_at(&writer, &volume, inner),
                Err(MusicFolderError::InsideMusicFolder {
                    path: shown(&volume, on_disk),
                    music_folder: shown(&volume, "Music"),
                }),
                "{inner}"
            );
        }
        assert_eq!(count(&writer, "music_folder"), 1);
    }

    #[test]
    fn a_folder_containing_an_existing_music_folder_is_refused() {
        let (_dir, volume) = drive(&["Music/House/Deep"]);
        let (_db, writer, _reads) = db();
        add_at(&writer, &volume, "Music/House/Deep").unwrap();
        for outer in ["Music/House", "Music", ""] {
            assert_eq!(
                add_at(&writer, &volume, outer),
                Err(MusicFolderError::ContainsMusicFolder {
                    path: shown(&volume, outer),
                    music_folder: shown(&volume, "Music/House/Deep"),
                }),
                "{outer:?}"
            );
        }
        assert_eq!(count(&writer, "music_folder"), 1);
    }

    #[test]
    fn the_same_folder_twice_is_refused_whatever_its_letter_case() {
        let (_dir, volume) = drive(&["Music"]);
        let (_db, writer, _reads) = db();
        add_at(&writer, &volume, "Music").unwrap();
        // Windows opens `MUSIC` as the same folder; it's stored in its
        // on-disk case either way, so both spellings are the same folder.
        for again in ["Music", "MUSIC"] {
            let refused = add_at(&writer, &volume, again).unwrap_err();
            assert!(
                matches!(refused, MusicFolderError::AlreadyAdded { .. }),
                "{again}: {refused:?}"
            );
        }
        assert_eq!(count(&writer, "music_folder"), 1);
    }

    #[test]
    fn neighbours_sharing_a_name_prefix_are_both_music_folders() {
        let (_dir, volume) = drive(&["Music", "Music 2", "Music2", "Q.X.Z", "Q.X.Z."]);
        let (_db, writer, reads) = db();
        for folder in ["Music", "Music 2", "Music2", "Q.X.Z", "Q.X.Z."] {
            add_at(&writer, &volume, folder).unwrap();
        }
        let paths: Vec<_> = list(&reads, &volume).into_iter().map(|f| f.path).collect();
        assert_eq!(
            paths,
            ["Music", "Music 2", "Music2", "Q.X.Z", "Q.X.Z."].map(|f| shown(&volume, f))
        );
    }

    #[test]
    fn a_folder_reached_through_a_junction_is_stored_where_it_really_is() {
        let (_dir, volume) = drive(&["Music/House", "Shortcuts"]);
        let (_db, writer, _reads) = db();
        let link = at(&volume, "Shortcuts/House link");
        link_dir(&link, &at(&volume, "Music/House"));

        let added = add(&writer, &volume, &link, MusicFolderRole::Scan).unwrap();
        assert_eq!(added.path, shown(&volume, "Music/House"));
        // So the overlap check sees the real folder.
        assert!(matches!(
            add_at(&writer, &volume, "Music"),
            Err(MusicFolderError::ContainsMusicFolder { .. })
        ));
    }

    #[test]
    fn a_path_that_is_not_an_existing_folder_is_refused() {
        let (_dir, volume) = drive(&["Music"]);
        fs::write(at(&volume, "Music/a.mp3"), b"x").unwrap();
        let (_db, writer, _reads) = db();

        let missing = at(&volume, "Nowhere");
        assert_eq!(
            add(&writer, &volume, &missing, MusicFolderRole::Scan),
            Err(MusicFolderError::NotAFolder {
                path: missing.to_string_lossy().into_owned()
            })
        );
        let file = at(&volume, "Music/a.mp3");
        assert_eq!(
            add(&writer, &volume, &file, MusicFolderRole::Scan),
            Err(MusicFolderError::NotAFolder {
                path: file.to_string_lossy().into_owned()
            })
        );
        for bad in [r"Music", r"\Music", "C:Music"] {
            assert_eq!(
                add(&writer, &volume, Path::new(bad), MusicFolderRole::Scan),
                Err(MusicFolderError::BadPath { path: bad.into() }),
                "{bad}"
            );
        }
        assert_eq!(count(&writer, "music_folder"), 0);
        assert_eq!(count(&writer, "volume"), 0);
    }

    /// A music folder with two indexed files in it.
    fn with_files(writer: &Writer, volume: &TempVolume) -> MusicFolderId {
        let folder = add_at(writer, volume, "Music").unwrap().id;
        writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key)
                     VALUES (?1, 'a.mp3', 'a.mp3'), (?1, 'b.mp3', 'b.mp3')",
                    [folder.0],
                )
            })
            .unwrap();
        folder
    }

    #[test]
    fn removing_a_folder_drops_its_index_rows_and_leaves_its_files_on_disk() {
        let (_dir, volume) = drive(&["Music", "Other"]);
        fs::write(at(&volume, "Music/a.mp3"), b"audio").unwrap();
        fs::write(at(&volume, "Music/b.mp3"), b"audio").unwrap();
        let (_db, writer, reads) = db();
        let folder = with_files(&writer, &volume);
        let other = add_at(&writer, &volume, "Other").unwrap();

        assert_eq!(writer.call(move |c| remove(c, folder)).unwrap(), Ok(()));
        assert_eq!(count(&writer, "file"), 0);
        assert_eq!(list(&reads, &volume), vec![other]);
        // The volume is remembered; the files are untouched.
        assert_eq!(count(&writer, "volume"), 1);
        assert_eq!(fs::read(at(&volume, "Music/a.mp3")).unwrap(), b"audio");
        assert_eq!(fs::read(at(&volume, "Music/b.mp3")).unwrap(), b"audio");
        // And it can be added again.
        add_at(&writer, &volume, "Music").unwrap();
    }

    #[test]
    fn removing_a_folder_whose_files_are_still_referenced_is_refused_and_keeps_everything() {
        let (_dir, volume) = drive(&["Music"]);
        let (_db, writer, _reads) = db();
        let folder = with_files(&writer, &volume);
        writer
            .call(|c| {
                c.execute(
                    "INSERT INTO relink (location_key, file_id, method)
                     VALUES ('file://localhost/E:/Music/a.mp3', 1, 'user')",
                    [],
                )
            })
            .unwrap();

        assert_eq!(
            writer.call(move |c| remove(c, folder)).unwrap(),
            Err(MusicFolderError::InUse)
        );
        assert_eq!(count(&writer, "file"), 2, "no file row was dropped");
        assert_eq!(count(&writer, "music_folder"), 1);
    }

    #[test]
    fn removing_a_folder_that_is_not_there_says_not_found() {
        let (_db, writer, _reads) = db();
        assert_eq!(
            writer.call(|c| remove(c, MusicFolderId(42))).unwrap(),
            Err(MusicFolderError::NotFound)
        );
    }

    #[test]
    fn a_folder_on_an_unplugged_volume_is_listed_where_it_was_last_seen() {
        let (_dir, volume) = drive(&["Music"]);
        let (_db, writer, reads) = db();
        let added = add_at(&writer, &volume, "Music").unwrap();
        volume.set_online(false);
        let listed = list(&reads, &volume);
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].online);
        assert_eq!(listed[0].path, added.path);
        let one = reads
            .read(|c| folders::stored_one(c, added.id))
            .unwrap()
            .unwrap();
        assert_eq!(one.rel.as_str(), "Music");
        let none = reads
            .read(|c| folders::stored_one(c, MusicFolderId(99)).map(|f| f.map(|f| f.id)))
            .unwrap();
        assert_eq!(none, None);
    }
}
