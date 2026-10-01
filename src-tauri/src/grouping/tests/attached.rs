//! What's attached to a track: Library tracks, the rekordbox snapshot and
//! other references decide what moves and what's kept.

use super::support::*;

// ---- Library-linked files -------------------------------------------------

#[test]
fn a_lower_track_with_the_same_audio_merges_into_the_track_a_library_track_points_at() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(2, "b.mp3", Some(B));
    db.group();
    let (low, high) = (db.track(a).unwrap(), db.track(b).unwrap());
    assert!(low < high);
    db.sql(&format!(
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES ({high}, {b})"
    ));
    db.set_hash(b, Some(A));
    db.group();
    // Not the lowest id: the linked file's track survives.
    assert_eq!((db.track(a), db.track(b)), (Some(high), Some(high)));
    assert_eq!(db.tracks(), 1);
}

#[test]
fn a_track_keeps_the_audio_of_its_linked_file_even_against_the_majority() {
    let db = db();
    let p = db.file(1, "p.mp3", Some(A));
    let q = db.file(1, "q.mp3", Some(A));
    let r = db.file(1, "r.mp3", Some(A));
    db.group();
    let track = db.track(p).unwrap();
    db.sql(&format!(
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES ({track}, {p})"
    ));
    db.set_hash(q, Some(B));
    db.set_hash(r, Some(B));
    db.group();
    assert_eq!(db.track(p), Some(track));
    assert_ne!(db.track(q), Some(track));
    assert_eq!(db.track(q), db.track(r));
}

#[test]
fn a_linked_file_that_loses_its_audio_hash_stays_in_its_track() {
    let db = db();
    let p = db.file(1, "p.mp3", Some(A));
    let q = db.file(1, "q.mp3", Some(A));
    db.group();
    let track = db.track(p).unwrap();
    db.sql(&format!(
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES ({track}, {p})"
    ));
    // A failed re-hash of the linked file.
    db.set_hash(p, None);
    db.group();
    assert_eq!(db.track(p), Some(track));
    assert_eq!(db.track(q), Some(track));
}

// ---- the rekordbox snapshot -----------------------------------------------

/// A rekordbox row matched to a file, up to its `file_id` value.
const MATCHED_ROW: &str = "INSERT INTO rekordbox_track
    (attributes, location_key, read_at, relink_method, file_id, recording_id)
    VALUES ('{\"TrackID\":\"7\",\"Location\":\"file://localhost/E:/a.mp3\"}', 'E:/a.mp3',
            '2026-09-29T00:00:00.000Z', 'path', ";

const RECORDING_OF_ROW: &str = "SELECT recording_id FROM rekordbox_track WHERE track_id = 7";

#[test]
fn a_rekordbox_row_on_the_file_that_moves_follows_it() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(2, "b.mp3", Some(B));
    db.group();
    let (low, high) = (db.track(a).unwrap(), db.track(b).unwrap());
    // Both tracks are referenced, so the lowest id wins the merge.
    db.sql(&format!(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES ({low}, 'local', 120)"
    ));
    db.sql(&format!("{MATCHED_ROW}{b}, {high})"));
    db.set_hash(b, Some(A));
    db.group();
    assert_eq!(db.track(b), Some(low));
    assert_eq!(db.count(RECORDING_OF_ROW), low);
}

#[test]
fn a_matched_rekordbox_row_gets_its_files_track_even_when_nothing_moves() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    // Matched by relink (the file only), as a fresh read leaves a row.
    db.sql(&format!("{MATCHED_ROW}{a}, NULL)"));
    db.group();
    let track = db.track(a).unwrap();
    assert_eq!(db.count(RECORDING_OF_ROW), track);
    // The snapshot is read again: the column is cleared, then set again.
    db.sql("UPDATE rekordbox_track SET recording_id = NULL");
    db.group();
    assert_eq!(db.count(RECORDING_OF_ROW), track);
}

// ---- what a track is kept for ---------------------------------------------

#[test]
fn a_track_in_a_version_link_that_a_move_empties_is_kept() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(1, "b.mp3", Some(B));
    let c = db.file(1, "c.mp3", Some(C));
    db.group();
    let (ta, tb, tc) = (
        db.track(a).unwrap(),
        db.track(b).unwrap(),
        db.track(c).unwrap(),
    );
    db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({tb}, {tc}, 'cut', 'user')"
    ));
    db.set_hash(b, Some(A));
    db.group();
    assert_eq!(db.track(b), Some(ta));
    assert_eq!(db.tracks(), 3, "the linked track is still there");
    assert_eq!(db.track(c), Some(tc));
}

#[test]
fn a_track_a_rekordbox_row_still_names_is_kept_when_its_folder_is_removed() {
    // Removing a folder sweeps tracks without going through the sync: the
    // row is left naming the track, and the delete must not trip its FK.
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    db.group();
    let track = db.track(a).unwrap();
    db.sql(&format!(
        "INSERT INTO rekordbox_track (attributes, location_key, read_at, recording_id)
         VALUES ('{{\"TrackID\":\"9\",\"Location\":\"file://localhost/E:/x.mp3\"}}', 'E:/x.mp3',
                 '2026-09-29T00:00:00.000Z', {track})"
    ));
    let removed = db
        .writer
        .call(|c| crate::scan::folders::remove(c, crate::scan::folders::MusicFolderId(1)))
        .unwrap();
    assert_eq!(removed, Ok(()));
    assert_eq!(db.tracks(), 1, "the track the rekordbox row names is kept");
}

#[test]
fn a_rekordbox_row_with_no_matched_file_has_no_track() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    db.group();
    let track = db.track(a).unwrap();
    db.sql(&format!(
        "INSERT INTO rekordbox_track (attributes, location_key, read_at, recording_id)
         VALUES ('{{\"TrackID\":\"9\",\"Location\":\"file://localhost/E:/x.mp3\"}}', 'E:/x.mp3',
                 '2026-09-29T00:00:00.000Z', {track})"
    ));
    db.group();
    assert_eq!(
        db.count("SELECT count(*) FROM rekordbox_track WHERE recording_id IS NOT NULL"),
        0
    );
}

#[test]
fn a_track_only_a_matched_rekordbox_row_points_at_takes_a_merge() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(1, "b.mp3", Some(B));
    db.group();
    let high = db.track(b).unwrap();
    assert!(db.track(a).unwrap() < high);
    // Only the higher track is referenced: by a matched rekordbox row.
    db.sql(&format!("{MATCHED_ROW}{b}, {high})"));
    db.set_hash(b, Some(A));
    db.group();
    assert_eq!((db.track(a), db.track(b)), (Some(high), Some(high)));
    assert_eq!(db.tracks(), 1);
}

#[test]
fn a_merge_prefers_the_track_with_the_linked_file_over_one_that_is_only_referenced() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let p = db.file(1, "p.mp3", Some(B));
    let q = db.file(1, "q.mp3", Some(B));
    db.group();
    let (low, high) = (db.track(a).unwrap(), db.track(p).unwrap());
    assert!(low < high);
    assert_eq!(db.track(q), Some(high));
    // Both tracks are referenced; only the higher holds a linked file.
    db.sql(&format!(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES ({low}, 'local', 120)"
    ));
    db.sql(&format!(
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES ({high}, {p})"
    ));
    db.set_hash(p, Some(A));
    db.set_hash(q, Some(A));
    db.group();
    // Everything lands in the track of the linked file, none split off.
    for file in [a, p, q] {
        assert_eq!(db.track(file), Some(high));
    }
    // The lower track is emptied, and its analysis goes with its files.
    assert_eq!(db.tracks(), 1);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM analysis WHERE recording_id = {high} AND bpm = 120"
        )),
        1
    );
}

#[test]
fn every_table_that_points_at_a_track_is_in_the_reference_list() {
    // Deleting an unreferenced track must never trip a foreign key that
    // store.rs's UNREFERENCED forgot (§2 adds more tables that point at
    // `recording`).
    let db = db();
    let mut found: Vec<(String, String)> = db
        .writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT m.name, f.\"from\" FROM sqlite_master m, pragma_foreign_key_list(m.name) f
                 WHERE m.type = 'table' AND f.\"table\" = 'recording'",
            )?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap();
    found.sort();
    let expected: Vec<(String, String)> = [
        ("analysis", "recording_id"),
        ("library_track", "recording_id"),
        ("recording_file", "recording_id"),
        ("rekordbox_track", "recording_id"),
        ("version_link", "recording_a"),
        ("version_link", "recording_b"),
    ]
    .iter()
    .map(|(t, c)| (t.to_string(), c.to_string()))
    .collect();
    assert_eq!(
        found, expected,
        "a table now points at `recording`: add it to UNREFERENCED in store.rs, then here"
    );
}

#[test]
fn a_track_kept_for_a_reference_is_swept_once_the_reference_goes() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(2, "b.mp3", Some(B));
    db.group();
    let (low, high) = (db.track(a).unwrap(), db.track(b).unwrap());
    db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({low}, {high}, 'cut', 'user')"
    ));
    // Removing the second folder leaves its track with no files, but it's
    // still a linked version of the first.
    db.writer
        .call(|c| crate::grouping::release_folder_files(c, 2))
        .unwrap();
    db.set_present(b, false);
    db.group();
    assert_eq!(db.tracks(), 2, "kept for its link");
    db.sql("DELETE FROM version_link");
    let summary = db.group();
    assert_eq!(db.tracks(), 1);
    assert_eq!(summary.recordings_removed, 1);
}

#[test]
fn two_files_that_both_lose_their_audio_hash_are_split_into_their_own_tracks() {
    let db = db();
    let a1 = db.file(1, "a1.mp3", Some(A));
    let a2 = db.file(1, "a2.mp3", Some(A));
    db.group();
    assert_eq!(db.track(a1), db.track(a2));
    db.set_hash(a1, None);
    db.set_hash(a2, None);
    db.group();
    assert_ne!(db.track(a1), db.track(a2));
    assert_eq!(db.tracks(), 2);
}

#[test]
fn a_merge_keeps_the_track_something_refers_to_not_just_the_lowest_id() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(1, "b.mp3", Some(B));
    db.group();
    let (low, high) = (db.track(a).unwrap(), db.track(b).unwrap());
    db.sql(&format!(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES ({high}, 'local', 120)"
    ));
    db.set_hash(b, Some(A));
    db.group();
    assert_eq!((db.track(a), db.track(b)), (Some(high), Some(high)));
    assert_eq!(db.tracks(), 1);
    assert_eq!(
        db.count(&format!("SELECT count(*) FROM recording WHERE id = {low}")),
        0
    );
}
