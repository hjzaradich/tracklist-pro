//! `current_duplicates`: what relink asks before it trusts a track for a
//! file that is gone (1bC-7). Through the real matching pass, deciding job
//! and grouping pass.

use super::support::*;

/// About two minutes of made-up audio.
const LEN: usize = 900;

/// No file.
const NONE: [i64; 0] = [];

fn file<'a>(name: &'a str, title: &'a str, n: u8, items: Vec<u32>) -> NewFile<'a> {
    NewFile {
        name,
        title: Some(title),
        artist: Some("Halden Rook"),
        hash: Some(hash(n)),
        items: Some(items),
        blake3: None,
    }
}

/// A FLAC and its MP3 re-encode, decided and merged into one track.
fn velo_and_its_reencode(db: &Db) -> (i64, i64) {
    let velo = track(1, LEN);
    let flac = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let mp3 = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.decide();
    assert_eq!(db.stands(flac, mp3), ("duplicate".into(), "merged".into()));
    (flac, mp3)
}

fn current(db: &Db, file: i64) -> Vec<i64> {
    db.writer
        .call(move |c| crate::grouping::current_duplicates(c, file))
        .unwrap()
}

#[test]
fn files_the_job_merged_are_current_duplicates() {
    let db = db();
    let (flac, mp3) = velo_and_its_reencode(&db);
    let other = db.add(file("Nightdrive.flac", "Nightdrive", 3, track(2, LEN)));
    db.decide();
    assert_eq!(current(&db, flac), [mp3]);
    assert_eq!(current(&db, mp3), [flac]);
    assert_eq!(current(&db, other), NONE);
}

#[test]
fn a_retagged_file_isnt_until_decided_again() {
    let db = db();
    let (flac, mp3) = velo_and_its_reencode(&db);
    // Another spelling of the same title: the file is due again.
    db.retag(mp3, Some("VELO"), Some("Halden Rook"));
    assert_eq!(current(&db, flac), NONE);
    assert_eq!(current(&db, mp3), NONE);
    db.decide();
    assert_eq!(db.stands(flac, mp3), ("duplicate".into(), "merged".into()));
    assert_eq!(current(&db, flac), [mp3]);
    assert_eq!(current(&db, mp3), [flac]);
}

#[test]
fn an_overwritten_file_isnt() {
    let db = db();
    let (flac, mp3) = velo_and_its_reencode(&db);
    // Other audio under the same name, hashed again; no grouping run since,
    // so it still sits in the track.
    db.overwrite(mp3, Some(hash(9)), None);
    assert_eq!(db.track(flac), db.track(mp3));
    assert_eq!(current(&db, flac), NONE);
    assert_eq!(current(&db, mp3), NONE);
    // Nor once grouping has moved it out.
    db.group();
    assert_ne!(db.track(flac), db.track(mp3));
    assert_eq!(current(&db, flac), NONE);
    assert_eq!(current(&db, mp3), NONE);
}

#[test]
fn files_sharing_a_track_that_the_rules_no_longer_call_duplicates_arent() {
    let db = db();
    let (flac, mp3) = velo_and_its_reencode(&db);
    // The second file turns out to be labeled as the Dirty cut. A merge is
    // never undone: both are decided, and still one track.
    db.retag(mp3, Some("Velo (Dirty)"), Some("Halden Rook"));
    db.decide();
    assert_eq!(db.track(flac), db.track(mp3));
    assert_eq!(db.decided(), [flac, mp3]);
    assert_eq!(current(&db, flac), NONE);
    assert_eq!(current(&db, mp3), NONE);
}

#[test]
fn files_decided_under_other_rules_arent() {
    let db = db();
    let (flac, mp3) = velo_and_its_reencode(&db);
    // As a build with another rules version would find one of them.
    db.sql(&format!(
        "UPDATE grouping_file SET decided_as = decided_as + 65536 WHERE file_id = {flac}"
    ));
    assert_eq!(current(&db, flac), NONE);
    assert_eq!(current(&db, mp3), NONE);
}

#[test]
fn a_file_placed_with_other_audio_than_it_holds_isnt() {
    let db = db();
    let (flac, mp3) = velo_and_its_reencode(&db);
    // Only the record of what it was placed with differs: the pair and both
    // files' decisions are as they were.
    db.sql(&format!(
        "UPDATE grouping_file SET placed_key = x'0909' WHERE file_id = {mp3}"
    ));
    assert_eq!(current(&db, flac), NONE);
    assert_eq!(current(&db, mp3), NONE);
}

#[test]
fn a_file_without_a_grouping_record_has_none() {
    let db = db();
    let (flac, mp3) = velo_and_its_reencode(&db);
    db.sql(&format!("DELETE FROM grouping_file WHERE file_id = {mp3}"));
    assert_eq!(current(&db, flac), NONE);
    assert_eq!(current(&db, mp3), NONE);
}

#[test]
fn a_file_that_went_keeps_its_duplicates_until_one_is_decided_again_without_it() {
    let db = db();
    let (flac, mp3) = velo_and_its_reencode(&db);
    db.set_present(flac, false);
    db.decide();
    assert_eq!(current(&db, flac), [mp3]);
    // The file left is decided again (another spelling of its title): the
    // gone file is no partner, so their pair goes, and nothing says any
    // more what the two are to each other.
    db.retag(mp3, Some("VELO"), Some("Halden Rook"));
    db.decide();
    assert_eq!(db.track(flac), db.track(mp3), "the track stays whole");
    assert_eq!(db.pair(flac, mp3), None);
    assert_eq!(current(&db, flac), NONE);
}

#[test]
fn a_file_that_joined_through_a_third_one_is_no_duplicate_of_a_file_it_was_never_compared_with() {
    let db = db();
    let (flac, mp3) = velo_and_its_reencode(&db);
    db.set_present(flac, false);
    // A third encode arrives while the first file is away, and joins the
    // track on its pair with the second.
    let aac = db.add(file("Velo.m4a", "Velo", 3, reencoded(&track(1, LEN), 2, 7)));
    db.decide();
    assert_eq!(db.track(aac), db.track(flac));
    assert_eq!(db.stands(mp3, aac), ("duplicate".into(), "merged".into()));
    assert_eq!(db.pair(flac, aac), None);
    assert_eq!(current(&db, flac), [mp3]);
    assert_eq!(current(&db, aac), [mp3]);
}
