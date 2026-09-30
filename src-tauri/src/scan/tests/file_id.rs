#![cfg(test)]
//! The walk's file id and the query-only handle it's read through.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::mem::ManuallyDrop;
use std::os::windows::io::FromRawHandle;

use crate::scan::file_id::{file_id, QueryHandle};

fn temp_file(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap().join("track.mp3");
    fs::write(&path, bytes).unwrap();
    (dir, path)
}

#[test]
fn the_file_id_handle_can_neither_read_nor_write_the_file() {
    let (_dir, path) = temp_file(b"audio bytes");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let handle = QueryHandle::open(&path).unwrap();
    // Borrow the raw handle as a File to try it; the QueryHandle closes it.
    let mut file = ManuallyDrop::new(unsafe { fs::File::from_raw_handle(handle.raw()) });
    let mut buf = [0u8; 4];
    assert!(file.read(&mut buf).is_err(), "the handle could read");
    assert!(file.write(b"x").is_err(), "the handle could write");
    assert!(file.set_len(0).is_err(), "the handle could truncate");
    assert!(file.seek(SeekFrom::Start(0)).is_ok());
    assert!(file.write_all(b"xyz").is_err());
    drop(handle);
    assert_eq!(fs::read(&path).unwrap(), b"audio bytes");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn the_file_id_handle_never_creates_a_missing_file() {
    let (_dir, path) = temp_file(b"x");
    let missing = path.with_file_name("missing.mp3");
    assert!(QueryHandle::open(&missing).is_err());
    assert!(!missing.exists());
}

#[test]
fn the_file_id_handle_does_not_lock_other_programs_out() {
    let (_dir, path) = temp_file(b"audio");
    let _held = QueryHandle::open(&path).unwrap();
    // rekordbox (or anything else) can still read, write and rename it.
    assert_eq!(fs::read(&path).unwrap(), b"audio");
    fs::write(&path, b"retagged").unwrap();
    let renamed = path.with_file_name("renamed.mp3");
    fs::rename(&path, &renamed).unwrap();
    assert_eq!(fs::read(&renamed).unwrap(), b"retagged");
}

#[test]
fn a_rename_keeps_the_file_id_and_a_copy_gets_its_own() {
    let (_dir, path) = temp_file(b"audio");
    let id = file_id(&path).unwrap();
    assert_eq!(id.len(), 49, "{id}");
    assert_eq!(&id[16..17], "-");
    assert!(id
        .chars()
        .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c)));

    let renamed = path.with_file_name("renamed.mp3");
    fs::rename(&path, &renamed).unwrap();
    assert_eq!(file_id(&renamed).unwrap(), id);

    let copy = path.with_file_name("copy.mp3");
    fs::copy(&renamed, &copy).unwrap();
    let copy_id = file_id(&copy).unwrap();
    assert_ne!(copy_id, id);
    assert_eq!(copy_id[..16], id[..16], "same volume, same serial");
}
