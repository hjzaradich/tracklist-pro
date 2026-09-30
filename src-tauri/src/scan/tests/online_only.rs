#![cfg(test)]
//! 1aB-8: OneDrive online-only files are indexed and marked from the
//! directory listing, never opened for reading, and later stages skip them
//! unless the user opts in.
//!
//! A real OneDrive placeholder needs a sync provider, so these tests use a
//! real file with `FILE_ATTRIBUTE_OFFLINE` set, which the listing reports
//! the same way (the recall attributes can only be set by a sync provider;
//! [`is_online_only`] treats all three alike).

use std::fs;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

use serde_json::json;
use tauri::Manager;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_OFFLINE, INVALID_FILE_ATTRIBUTES,
};

use super::support::db;
use super::walk::{add_music, at, drive, put, rows, scan};
use crate::db::Writer;
use crate::ipc::testing::{app, invoke};
use crate::scan::online_only::{
    is_online_only, read_opt_in, write_opt_in, ATTRIBUTE_OFFLINE, ATTRIBUTE_RECALL_ON_DATA_ACCESS,
    ATTRIBUTE_RECALL_ON_OPEN, READ_ONLINE_ONLY_FILES,
};
use crate::scan::ReadGate;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

fn attributes_of(path: &Path) -> u32 {
    let attributes = unsafe { GetFileAttributesW(wide(path).as_ptr()) };
    assert_ne!(attributes, INVALID_FILE_ATTRIBUTES, "{}", path.display());
    attributes
}

/// Makes `path` look online only in its folder's listing, or local again.
fn set_online_only(path: &Path, online_only: bool) {
    let attributes = attributes_of(path);
    let attributes = if online_only {
        attributes | FILE_ATTRIBUTE_OFFLINE
    } else {
        attributes & !FILE_ATTRIBUTE_OFFLINE
    };
    assert_ne!(
        unsafe { SetFileAttributesW(wide(path).as_ptr(), attributes) },
        0
    );
}

/// Holds `path` open with no sharing at all: while the handle lives, any
/// attempt to open the file's data (to read, write or delete) fails.
/// Asking for its attributes or id (access 0) still works.
fn lock_against_reading(path: &Path) -> fs::File {
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap();
    let refused = fs::read(path).unwrap_err();
    assert_eq!(refused.raw_os_error(), Some(32), "{refused}"); // ERROR_SHARING_VIOLATION
    lock
}

fn id_of(writer: &Writer, rel: &str) -> i64 {
    rows(writer)
        .into_iter()
        .find(|r| r.rel_path == rel)
        .unwrap_or_else(|| panic!("{rel} isn't indexed"))
        .id
}

fn online_only_rows(writer: &Writer) -> Vec<(String, bool)> {
    writer
        .call(|c| {
            let mut s = c.prepare("SELECT rel_path, online_only FROM file ORDER BY rel_path")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap()
}

/// A job's gate, as a stage would make it now.
fn gate(writer: &Writer) -> ReadGate {
    writer.call(|c| ReadGate::for_job(c)).unwrap()
}

fn may_read_file(writer: &Writer, rel: &str) -> bool {
    let (gate, id) = (gate(writer), id_of(writer, rel));
    writer.call(move |c| gate.may_read(c, id)).unwrap()
}

#[test]
fn recall_on_data_access_recall_on_open_or_offline_make_a_file_online_only() {
    for attributes in [
        ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        ATTRIBUTE_RECALL_ON_OPEN,
        ATTRIBUTE_OFFLINE,
        // What OneDrive's online-only files carry, with the usual extras.
        0x0040_0000 | 0x0010_0000 | 0x0000_0400 | 0x0000_0020,
    ] {
        assert!(is_online_only(attributes), "{attributes:#x}");
    }
    for attributes in [
        0,
        0x0000_0020,                             // archive
        0x0000_0080,                             // normal
        0x0008_0000,                             // pinned: "Always keep on this device"
        0x0010_0000,                             // unpinned, but downloaded: its bytes are here
        0x0000_0400,                             // a reparse point, e.g. a downloaded cloud file
        0x0000_0001 | 0x0000_0002 | 0x0000_0004, // read-only, hidden, system
    ] {
        assert!(!is_online_only(attributes), "{attributes:#x}");
    }
}

#[test]
fn an_online_only_file_is_indexed_and_marked_without_ever_being_opened_for_reading() {
    let (_dir, volume, music) = drive();
    put(&music, "Local.mp3", b"local bytes");
    put(&music, "House/In The Cloud.mp3", b"cloud bytes");
    let cloud = at(&music, "House/In The Cloud.mp3");
    set_online_only(&cloud, true);
    let attributes_before = attributes_of(&cloud);
    let modified_before = fs::metadata(&cloud).unwrap().modified().unwrap();

    // Nothing may open the file's data during the walk: a read would fail.
    let lock = lock_against_reading(&cloud);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    let sent = scan(&writer, &volume);
    drop(lock);

    // Indexed like any file, with its size and file id, and marked.
    let row = rows(&writer)
        .into_iter()
        .find(|r| r.rel_path == "House/In The Cloud.mp3")
        .unwrap();
    assert_eq!(row.size, Some(11));
    assert!(row.file_id.is_some());
    assert!(row.present);
    assert_eq!(
        online_only_rows(&writer),
        [
            ("House/In The Cloud.mp3".to_owned(), true),
            ("Local.mp3".to_owned(), false)
        ]
    );
    // The frontend hears which files are online only.
    let mut heard: Vec<_> = sent
        .iter()
        .flatten()
        .map(|f| (f.rel_path.as_str(), f.online_only))
        .collect();
    heard.sort();
    assert_eq!(
        heard,
        [("House/In The Cloud.mp3", true), ("Local.mp3", false)]
    );
    // Its music folder counts it.
    let folder = writer
        .call(|c| crate::scan::folders::stored(c))
        .unwrap()
        .remove(0);
    assert_eq!(folder.online_only_files, 1);
    // The walk changed nothing about the file.
    assert_eq!(attributes_of(&cloud), attributes_before);
    assert_eq!(
        fs::metadata(&cloud).unwrap().modified().unwrap(),
        modified_before
    );
}

#[test]
fn later_stages_may_not_read_an_online_only_file_until_the_user_opts_in() {
    let (_dir, volume, music) = drive();
    put(&music, "Local.mp3", b"a");
    put(&music, "Cloud.mp3", b"b");
    set_online_only(&at(&music, "Cloud.mp3"), true);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    // Off by default.
    assert!(!writer.call(|c| read_opt_in(c)).unwrap());
    assert!(may_read_file(&writer, "Local.mp3"));
    assert!(!may_read_file(&writer, "Cloud.mp3"));

    writer.call(|c| write_opt_in(c, true)).unwrap();
    assert!(may_read_file(&writer, "Cloud.mp3"));
    assert!(may_read_file(&writer, "Local.mp3"));

    writer.call(|c| write_opt_in(c, false)).unwrap();
    assert!(!may_read_file(&writer, "Cloud.mp3"));
}

#[test]
fn a_gate_reads_the_opt_in_once_for_its_job() {
    let (_db, writer, _reads) = db();
    let before = gate(&writer);
    writer.call(|c| write_opt_in(c, true)).unwrap();
    // The job that started before the change keeps its answer.
    assert!(!before.allows(true));
    assert!(before.allows(false));
    assert!(gate(&writer).allows(true));
}

#[test]
fn may_read_refuses_a_file_that_is_not_in_the_index() {
    let (_db, writer, _reads) = db();
    writer.call(|c| write_opt_in(c, true)).unwrap();
    let gate = gate(&writer);
    assert!(!writer.call(move |c| gate.may_read(c, 42)).unwrap());
}

#[test]
fn a_file_downloaded_since_the_last_walk_loses_its_mark_and_one_freed_up_gains_it() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"a");
    put(&music, "b.mp3", b"b");
    set_online_only(&at(&music, "a.mp3"), true);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);
    let ids: Vec<i64> = rows(&writer).iter().map(|r| r.id).collect();

    // OneDrive downloads a.mp3 and frees up space taken by b.mp3.
    set_online_only(&at(&music, "a.mp3"), false);
    set_online_only(&at(&music, "b.mp3"), true);
    scan(&writer, &volume);
    assert_eq!(
        online_only_rows(&writer),
        [("a.mp3".to_owned(), false), ("b.mp3".to_owned(), true)]
    );
    // Same rows, refreshed in place.
    assert_eq!(rows(&writer).iter().map(|r| r.id).collect::<Vec<_>>(), ids);
}

#[test]
fn may_open_says_no_for_an_online_only_file_without_needing_to_open_it() {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let local = base.join("local.mp3");
    let cloud = base.join("cloud.mp3");
    fs::write(&local, b"a").unwrap();
    fs::write(&cloud, b"b").unwrap();
    set_online_only(&cloud, true);
    let (_db, writer, _reads) = db();
    let gate = gate(&writer);

    // Both locked: the check needs no access to either file's data.
    let _locks = (lock_against_reading(&local), lock_against_reading(&cloud));
    assert!(gate.may_open(&local).unwrap());
    assert!(!gate.may_open(&cloud).unwrap());
}

#[test]
fn once_the_user_opts_in_may_open_lets_an_online_only_file_be_opened() {
    let dir = tempfile::tempdir().unwrap();
    let cloud = fs::canonicalize(dir.path()).unwrap().join("cloud.mp3");
    fs::write(&cloud, b"b").unwrap();
    set_online_only(&cloud, true);
    let (_db, writer, _reads) = db();
    writer.call(|c| write_opt_in(c, true)).unwrap();
    let gate = gate(&writer);
    assert!(gate.may_open(&cloud).unwrap());
    assert!(gate.allows(true));
}

#[test]
fn once_the_user_opts_in_may_open_says_yes_without_asking_about_the_file_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let missing = fs::canonicalize(dir.path()).unwrap().join("missing.mp3");
    let (_db, writer, _reads) = db();
    writer.call(|c| write_opt_in(c, true)).unwrap();
    // Asking about a missing file would fail; the gate doesn't ask.
    assert!(gate(&writer).may_open(&missing).unwrap());
    assert!(!missing.exists());
}

#[test]
fn may_open_says_it_cannot_tell_for_a_file_gone_since_the_walk_rather_than_online_only() {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let (_db, writer, _reads) = db();
    let gate = gate(&writer);
    let gone = gate.may_open(&base.join("missing.mp3")).unwrap_err();
    assert_eq!(gone.kind(), std::io::ErrorKind::NotFound);
    // Nor is a missing folder taken for an online-only file.
    let unplugged = gate
        .may_open(&base.join("Unplugged").join("a.mp3"))
        .unwrap_err();
    assert_eq!(unplugged.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn an_unset_or_malformed_opt_in_reads_as_off() {
    let (_db, writer, _reads) = db();
    assert!(!writer.call(|c| read_opt_in(c)).unwrap());
    for (value, on) in [
        ("true", true),
        ("false", false),
        ("1", false),
        (r#""true""#, false),
        ("null", false),
        (r#"{"on":true}"#, false),
    ] {
        writer
            .call(move |c| {
                c.execute(
                    "INSERT OR REPLACE INTO setting (key, value) VALUES (?1, ?2)",
                    (READ_ONLINE_ONLY_FILES, value),
                )
            })
            .unwrap();
        assert_eq!(writer.call(|c| read_opt_in(c)).unwrap(), on, "{value}");
    }
}

#[test]
fn the_frontend_reads_the_opt_in_as_off_and_can_turn_it_on_and_off() {
    let (_data, app) = app();
    assert_eq!(
        invoke(&app, "read_online_only_files", json!({})),
        Ok(json!(false))
    );
    for on in [true, false, true] {
        assert_eq!(
            invoke(&app, "set_read_online_only_files", json!({ "on": on })),
            Ok(json!(null))
        );
        assert_eq!(
            invoke(&app, "read_online_only_files", json!({})),
            Ok(json!(on))
        );
    }
    assert!(invoke(&app, "set_read_online_only_files", json!({ "on": "yes" })).is_err());
    let writer = app.state::<Writer>();
    let stored: String = writer
        .call(|c| {
            c.query_row(
                "SELECT value FROM setting WHERE key = ?1",
                [READ_ONLINE_ONLY_FILES],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(stored, "true");
}
