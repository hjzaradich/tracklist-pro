#![cfg(test)]
//! 1aB-14: the walk counts the folders and audio files it couldn't read,
//! instead of skipping them silently, and stores the counts on the music
//! folder.

use std::ffi::OsString;
use std::fs;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::support::db;
use super::walk::{add_music, at, drive, put, rel_paths, scan};
use crate::db::Writer;
use crate::scan::folders::{stored_one, MusicFolderId, StoredFolder};

/// Everyone is refused a folder's listing until this is dropped.
pub(super) struct Unlistable(PathBuf);

impl Unlistable {
    pub(super) fn new(dir: &Path) -> Unlistable {
        as_an_ordinary_user();
        icacls(dir, &["/deny", "*S-1-1-0:(RD)"]);
        // Made first, so the deny is lifted even if the check below fails.
        let unlistable = Unlistable(dir.to_path_buf());
        if fs::read_dir(dir).is_ok() {
            let privileges = Command::new("whoami").arg("/priv").output();
            let privileges = privileges
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_else(|e| format!("(whoami failed: {e})"));
            panic!(
                "{} is still listable after denying Everyone its listing\n\
                 whoami /priv:\n{privileges}",
                dir.display()
            );
        }
        unlistable
    }
}

/// Makes this test process an ordinary user as far as reading folders
/// goes: removes the backup and restore privileges from its token, once.
///
/// An admin account's full token (e.g. CI's runner, a Windows service
/// running as the owner) can hold SeBackupPrivilege, and a folder opened
/// for backup ignores the folder's permissions, so a denied listing would
/// still succeed. Nothing in the app asks for backup access.
fn as_an_ordinary_user() {
    use std::sync::Once;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LUID};
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_REMOVED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let mut token: HANDLE = std::ptr::null_mut();
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        };
        assert_ne!(opened, 0, "{}", std::io::Error::last_os_error());
        for name in ["SeBackupPrivilege", "SeRestorePrivilege"] {
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let mut luid = LUID::default();
            if unsafe { LookupPrivilegeValueW(std::ptr::null(), wide.as_ptr(), &mut luid) } == 0 {
                continue;
            }
            let removed = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_REMOVED,
                }],
            };
            // A token without the privilege (an ordinary user's) is fine.
            unsafe {
                AdjustTokenPrivileges(
                    token,
                    0,
                    &removed,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
        }
        unsafe { CloseHandle(token) };
    });
}

impl Drop for Unlistable {
    fn drop(&mut self) {
        icacls(&self.0, &["/remove:d", "*S-1-1-0"]);
    }
}

fn icacls(dir: &Path, args: &[&str]) {
    let out = Command::new("icacls").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "icacls: {out:?}");
}

/// A name NTFS allows but no Rust string can hold: a lone UTF-16
/// surrogate, as some old taggers and broken copies leave behind.
fn unstorable(stem: &str, extension: &str) -> OsString {
    let mut name: Vec<u16> = stem.encode_utf16().collect();
    name.push(0xD800);
    name.extend(extension.encode_utf16());
    OsString::from_wide(&name)
}

fn folder(writer: &Writer, id: MusicFolderId) -> StoredFolder {
    writer.call(move |c| stored_one(c, id)).unwrap().unwrap()
}

fn counts(writer: &Writer, id: MusicFolderId) -> (Option<u32>, Option<u32>) {
    let f = folder(writer, id);
    (f.unreadable_folders, f.unreadable_files)
}

#[test]
fn a_folder_the_walk_cannot_list_is_counted_and_the_rest_is_still_walked() {
    let (_dir, volume, music) = drive();
    put(&music, "A/a.mp3", b"a");
    put(&music, "Locked/hidden.mp3", b"h");
    put(&music, "Z/z.mp3", b"z");
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);

    let _locked = Unlistable::new(&at(&music, "Locked"));
    scan(&writer, &volume);
    assert_eq!(
        rel_paths(&writer).into_iter().collect::<Vec<_>>(),
        ["A/a.mp3", "Z/z.mp3"]
    );
    assert_eq!(counts(&writer, id), (Some(1), Some(0)));
}

#[test]
fn an_audio_file_or_folder_whose_name_cannot_be_stored_is_counted_and_other_files_are_not() {
    let (_dir, volume, music) = drive();
    put(&music, "ok.mp3", b"ok");
    fs::write(music.join(unstorable("broken", ".mp3")), b"x").unwrap();
    // Not audio: not the scan's business, so not counted.
    fs::write(music.join(unstorable("cover", ".jpg")), b"x").unwrap();
    let odd_folder = music.join(unstorable("Folder", ""));
    fs::create_dir(&odd_folder).unwrap();
    fs::write(odd_folder.join("inside.mp3"), b"x").unwrap();
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);

    scan(&writer, &volume);
    assert_eq!(
        rel_paths(&writer).into_iter().collect::<Vec<_>>(),
        ["ok.mp3"]
    );
    assert_eq!(counts(&writer, id), (Some(1), Some(1)));
}

#[test]
fn a_music_folder_has_no_counts_until_walked_and_zero_once_everything_was_read() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"a");
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);
    assert_eq!(counts(&writer, id), (None, None));
    assert_eq!(folder(&writer, id).walked_at, None);

    scan(&writer, &volume);
    assert_eq!(counts(&writer, id), (Some(0), Some(0)));
    assert!(folder(&writer, id).walked_at.is_some());
}

#[test]
fn the_counts_follow_the_latest_walk_once_the_folder_can_be_read_again() {
    let (_dir, volume, music) = drive();
    put(&music, "Locked/a.mp3", b"a");
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);
    let first_walk = {
        let _locked = Unlistable::new(&at(&music, "Locked"));
        scan(&writer, &volume);
        assert_eq!(counts(&writer, id), (Some(1), Some(0)));
        folder(&writer, id).walked_at.unwrap()
    };

    scan(&writer, &volume);
    assert_eq!(counts(&writer, id), (Some(0), Some(0)));
    assert!(folder(&writer, id).walked_at.unwrap() >= first_walk);
    assert!(rel_paths(&writer).contains("Locked/a.mp3"));
}

#[test]
fn a_music_folder_whose_own_listing_is_refused_counts_as_one_unreadable_folder() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"a");
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);

    let _locked = Unlistable::new(&music);
    scan(&writer, &volume);
    assert_eq!(counts(&writer, id), (Some(1), Some(0)));
    assert!(rel_paths(&writer).is_empty());
}

#[test]
fn a_folder_on_an_unplugged_drive_keeps_the_counts_of_its_last_walk() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"a");
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);
    scan(&writer, &volume);
    let walked = folder(&writer, id);

    volume.set_online(false);
    scan(&writer, &volume);
    assert_eq!(folder(&writer, id), walked);
}

/// Sets a folder's attributes as Windows marks its own folders.
fn set_attributes(path: &Path, attributes: u32) {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(wide.as_ptr(), attributes)
    };
    assert_ne!(ok, 0, "{}", path.display());
}

const HIDDEN: u32 = 0x2;
const SYSTEM: u32 = 0x4;
const NORMAL: u32 = 0x80;

#[test]
fn windows_own_hidden_system_folders_are_neither_walked_nor_counted_but_a_hidden_folder_is() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"a");
    put(&music, "$Recycle.Bin/S-1-5-21/$R0001.mp3", b"deleted");
    put(&music, "System Volume Information/x.mp3", b"x");
    put(&music, "Hidden By Me/b.mp3", b"b");
    let own = [
        at(&music, "$Recycle.Bin"),
        at(&music, "System Volume Information"),
    ];
    for folder in &own {
        set_attributes(folder, HIDDEN | SYSTEM);
    }
    set_attributes(&at(&music, "Hidden By Me"), HIDDEN);
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);

    {
        // As on a real drive, System Volume Information can't be listed.
        let _locked = Unlistable::new(&own[1]);
        scan(&writer, &volume);
    }
    assert_eq!(
        rel_paths(&writer).into_iter().collect::<Vec<_>>(),
        ["Hidden By Me/b.mp3", "a.mp3"]
    );
    assert_eq!(counts(&writer, id), (Some(0), Some(0)));
    for folder in &own {
        set_attributes(folder, NORMAL);
    }
}
