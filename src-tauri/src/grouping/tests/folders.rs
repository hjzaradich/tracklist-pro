//! Removing a music folder lets go of its files' tracks (folder 1 is the
//! one removed).

use super::support::*;
use crate::scan::folders::{remove, MusicFolderError, MusicFolderId};

fn remove_folder(db: &Db, id: i64) -> Result<(), MusicFolderError> {
    db.writer
        .call(move |c| remove(c, MusicFolderId(id)))
        .unwrap()
}

#[test]
fn removing_a_grouped_folder_succeeds() {
    let db = db();
    db.file(1, "a.mp3", Some(A));
    db.file(1, "b.mp3", Some(B));
    db.file(1, "c.mp3", None);
    db.group();
    assert_eq!(remove_folder(&db, 1), Ok(()));
    assert_eq!(db.count("SELECT count(*) FROM file"), 0);
    assert_eq!(
        db.count("SELECT count(*) FROM music_folder WHERE id = 1"),
        0
    );
}

#[test]
fn an_orphaned_track_is_deleted() {
    let db = db();
    let keep = db.file(2, "keep.mp3", Some(C));
    db.file(1, "a.mp3", Some(A));
    db.file(1, "b.mp3", Some(A));
    db.file(1, "c.mp3", None);
    db.group();
    assert_eq!(db.tracks(), 3);
    remove_folder(&db, 1).unwrap();
    assert_eq!(db.tracks(), 1, "no orphan tracks");
    assert_eq!(db.count("SELECT count(*) FROM recording_file"), 1);
    assert!(db.track(keep).is_some());
}

#[test]
fn a_track_shared_with_another_folder_survives_with_its_other_file() {
    let db = db();
    let removed = db.file(1, "a.mp3", Some(A));
    let other = db.file(2, "a copy.mp3", Some(A));
    db.group();
    let track = db.track(other).unwrap();
    assert_eq!(db.track(removed), Some(track));
    remove_folder(&db, 1).unwrap();
    assert_eq!(db.tracks(), 1);
    assert_eq!(db.track(other), Some(track));
}

#[test]
fn a_track_something_else_points_at_is_kept() {
    let db = db();
    db.file(1, "a.mp3", Some(A));
    db.group();
    db.sql("INSERT INTO analysis (recording_id, source, bpm) VALUES (1, 'local', 120)");
    remove_folder(&db, 1).unwrap();
    assert_eq!(db.tracks(), 1, "its analysis still points at it");
    assert_eq!(db.count("SELECT count(*) FROM recording_file"), 0);
}

#[test]
fn a_file_decided_on_is_let_go_whatever_its_role() {
    let db = db();
    db.file(1, "a.mp3", Some(A));
    db.file(1, "b.mp3", Some(A));
    db.file(1, "c.mp3", Some(A));
    db.group();
    db.sql("UPDATE recording_file SET role = 'undecided' WHERE file_id = 1");
    db.sql("UPDATE recording_file SET role = 'extra' WHERE file_id = 2");
    db.sql("UPDATE recording_file SET role = 'best' WHERE file_id = 3");
    assert_eq!(remove_folder(&db, 1), Ok(()));
    assert_eq!(db.tracks(), 0);
}

#[test]
fn a_file_a_library_track_links_still_refuses_the_removal() {
    let db = db();
    let file = db.file(1, "a.mp3", Some(A));
    db.group();
    db.sql(&format!(
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, {file})"
    ));
    assert_eq!(remove_folder(&db, 1), Err(MusicFolderError::InUse));
    assert_eq!(db.track(file), Some(1), "its track row is still there");
}

#[test]
fn everything_rolls_back_together_if_the_delete_is_refused() {
    let db = db();
    let plain = db.file(1, "a.mp3", Some(A));
    let linked = db.file(1, "b.mp3", Some(B));
    let elsewhere = db.file(2, "c.mp3", Some(A));
    db.group();
    let before = db.groups();
    db.sql(&format!(
        "INSERT INTO library_track (recording_id, linked_file_id)
         VALUES ({}, {linked})",
        db.track(linked).unwrap()
    ));
    assert_eq!(remove_folder(&db, 1), Err(MusicFolderError::InUse));
    // The plain file's track row, and the track it shares, are back.
    assert_eq!(db.groups(), before);
    assert_eq!(db.track(plain), db.track(elsewhere));
    assert_eq!(db.count("SELECT count(*) FROM file"), 3);
    assert_eq!(db.tracks(), 2);
}

#[test]
fn a_track_that_lost_its_best_file_gets_a_new_best_on_the_next_pass() {
    let db = db();
    let best = db.file(1, "a.mp3", Some(A));
    let other = db.file(2, "a copy.mp3", Some(A));
    db.group();
    assert_eq!(db.role(best), "best");
    assert_eq!(db.role(other), "undecided");
    remove_folder(&db, 1).unwrap();
    assert_eq!(db.role(other), "undecided", "released, not yet re-picked");
    db.group();
    assert_eq!(db.role(other), "best");
    assert_eq!(db.tracks(), 1);
}
