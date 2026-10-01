//! What grouping does with files, tested through the database.

use std::io::Cursor;

use super::support::*;
use crate::hash::hash_reader;

// ---- real audio hashes ----------------------------------------------------

/// A mono 16-bit WAVE whose audio depends on `seed`, with a `LIST`/`INFO`
/// title if given: the audio is the same whatever the tags.
fn wav(title: Option<&str>, seed: u8) -> Vec<u8> {
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }
    let mut fmt = Vec::new();
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(8000u32.to_le_bytes());
    fmt.extend(16000u32.to_le_bytes());
    fmt.extend(2u16.to_le_bytes());
    fmt.extend(16u16.to_le_bytes());
    let samples: Vec<u8> = (0..4000u32)
        .map(|i| (i.wrapping_mul(31).wrapping_add(u32::from(seed) * 17) % 251) as u8)
        .collect();
    let mut chunks = vec![chunk(b"fmt ", &fmt)];
    if let Some(title) = title {
        let mut info = b"INFO".to_vec();
        info.extend(chunk(b"INAM", format!("{title}\0").as_bytes()));
        chunks.push(chunk(b"LIST", &info));
    }
    chunks.push(chunk(b"data", &samples));
    let body: Vec<u8> = [b"WAVE".to_vec(), chunks.concat()].concat();
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

/// The audio_hash the hash stage stores for `bytes`.
fn audio_hash(bytes: &[u8]) -> [u8; 34] {
    let hashes = hash_reader(&mut Cursor::new(bytes), &mut [0u8; 4096], &mut |_| false)
        .unwrap()
        .unwrap();
    hashes.audio.expect("a WAVE has an audio_hash").to_bytes()
}

#[test]
fn the_same_audio_with_different_tags_is_one_track() {
    let db = db();
    let tagged = wav(Some("Original Mix"), 1);
    let retagged = wav(Some("Original Mix (rekordbox rewrote this)"), 1);
    let untagged = wav(None, 1);
    assert_ne!(tagged, retagged, "the files differ");
    let files = [
        db.file(1, "a.wav", Some(audio_hash(&tagged))),
        db.file(2, "copy/b.wav", Some(audio_hash(&retagged))),
        db.file(1, "c.wav", Some(audio_hash(&untagged))),
    ];
    let summary = db.group();
    assert_eq!(db.tracks(), 1);
    assert!(files.iter().all(|&f| db.track(f) == db.track(files[0])));
    assert_eq!((summary.placed, summary.recordings_made), (3, 1));
}

#[test]
fn different_audio_stays_in_separate_tracks() {
    let db = db();
    let a = db.file(1, "a.wav", Some(audio_hash(&wav(Some("T"), 1))));
    let b = db.file(1, "b.wav", Some(audio_hash(&wav(Some("T"), 2))));
    db.group();
    assert_eq!(db.tracks(), 2);
    assert_ne!(db.track(a), db.track(b));
}

#[test]
fn files_with_no_audio_hash_are_never_merged() {
    let db = db();
    let none = [
        db.file(1, "1.mp3", None),
        db.file(1, "2.mp3", None),
        db.file(2, "3.mp3", None),
    ];
    let hashed = db.file(1, "4.mp3", Some(A));
    db.group();
    assert_eq!(db.tracks(), 4);
    let tracks: std::collections::BTreeSet<_> = none
        .iter()
        .chain([&hashed])
        .map(|&f| db.track(f).unwrap())
        .collect();
    assert_eq!(tracks.len(), 4);
    // An empty hash says nothing, like none.
    db.sql("UPDATE file SET audio_hash = x'' WHERE rel_path IN ('1.mp3', '2.mp3')");
    let again = db.group();
    assert_eq!(db.tracks(), 4);
    assert_eq!((again.moved, again.placed), (0, 0));
}

#[test]
fn every_present_file_gets_exactly_one_track() {
    let db = db();
    db.file(1, "a.mp3", Some(A));
    db.file(1, "b.mp3", Some(A));
    db.file(2, "c.mp3", Some(B));
    db.file(2, "d.mp3", None);
    db.group();
    assert_eq!(
        db.count(
            "SELECT count(*) FROM file f WHERE present = 1 AND
             (SELECT count(*) FROM recording_file WHERE file_id = f.id) <> 1"
        ),
        0
    );
    assert_eq!(db.count("SELECT count(*) FROM recording_file"), 4);
    assert_eq!(db.tracks(), 3);
}

#[test]
fn a_file_that_is_not_on_disk_and_has_no_track_isnt_given_one() {
    let db = db();
    let gone = db.file(1, "gone.mp3", Some(A));
    db.set_present(gone, false);
    db.group();
    assert_eq!(db.track(gone), None);
    assert_eq!(db.tracks(), 0);
}

// ---- change and re-runs ---------------------------------------------------

#[test]
fn running_it_again_changes_nothing() {
    let db = db();
    db.file(1, "a.mp3", Some(A));
    db.file(1, "b.mp3", Some(A));
    db.file(2, "c.mp3", Some(B));
    db.file(2, "d.mp3", None);
    db.group();
    let ids = "SELECT sum(id * id) FROM recording";
    let before = (db.groups(), db.count(ids));
    let again = db.group();
    assert_eq!(
        (
            again.placed,
            again.moved,
            again.recordings_made,
            again.recordings_removed
        ),
        (0, 0, 0, 0)
    );
    assert_eq!((db.groups(), db.count(ids)), before);
}

#[test]
fn a_file_added_later_joins_its_track_without_disturbing_the_others() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(1, "b.mp3", Some(B));
    db.group();
    let (track_a, track_b) = (db.track(a), db.track(b));
    let later = db.file(2, "a copy.mp3", Some(A));
    let other = db.file(2, "other.mp3", None);
    let summary = db.group();
    assert_eq!(db.track(later), track_a);
    assert_eq!((db.track(a), db.track(b)), (track_a, track_b));
    assert_ne!(db.track(other), track_a);
    assert_eq!((summary.placed, summary.moved), (2, 0));
}

#[test]
fn a_file_whose_audio_changes_moves_to_the_track_its_audio_belongs_to() {
    let db = db();
    let a1 = db.file(1, "a1.mp3", Some(A));
    let a2 = db.file(1, "a2.mp3", Some(A));
    let b = db.file(2, "b.mp3", Some(B));
    db.group();
    assert_eq!(db.track(a1), db.track(a2));
    // a2's file is replaced by B's audio.
    db.set_hash(a2, Some(B));
    let summary = db.group();
    assert_eq!(db.track(a2), db.track(b));
    assert_ne!(db.track(a1), db.track(a2));
    assert_eq!(summary.moved, 1);
    assert_eq!(db.tracks(), 2);
}

#[test]
fn a_file_whose_audio_changes_to_something_new_gets_a_track_of_its_own() {
    let db = db();
    let a1 = db.file(1, "a1.mp3", Some(A));
    let a2 = db.file(1, "a2.mp3", Some(A));
    db.group();
    let old = db.track(a1);
    db.set_hash(a2, Some(C));
    db.group();
    assert_eq!(db.track(a1), old, "the unchanged file keeps its track");
    assert_ne!(db.track(a2), old);
    assert_eq!(db.tracks(), 2);
}

#[test]
fn a_file_that_loses_its_audio_hash_leaves_its_track() {
    let db = db();
    let a1 = db.file(1, "a1.mp3", Some(A));
    let a2 = db.file(1, "a2.mp3", Some(A));
    db.group();
    db.set_hash(a2, None);
    db.group();
    assert_ne!(db.track(a1), db.track(a2));
    assert_eq!(db.tracks(), 2);
}

#[test]
fn a_track_emptied_by_a_move_is_removed() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(2, "b.mp3", Some(B));
    db.group();
    // The higher track is the one that goes: the lowest id survives a merge.
    let emptied = db.track(b).unwrap();
    db.set_hash(b, Some(A));
    let summary = db.group();
    assert_eq!(db.track(a), db.track(b));
    assert_eq!(db.tracks(), 1);
    assert_eq!(summary.recordings_removed, 1);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM recording WHERE id = {emptied}"
        )),
        0
    );
}

#[test]
fn a_track_emptied_by_a_move_hands_its_analysis_to_the_track_its_files_went_to() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(2, "b.mp3", Some(B));
    db.group();
    // Both tracks are referenced, so the lowest id wins the merge. The
    // other is left with no files; its analysis goes (the survivor already
    // has one from that source, so its own stays) and the track with it.
    for (track, bpm) in [(db.track(a).unwrap(), 120), (db.track(b).unwrap(), 121)] {
        db.sql(&format!(
            "INSERT INTO analysis (recording_id, source, bpm) VALUES ({track}, 'local', {bpm})"
        ));
    }
    db.set_hash(b, Some(A));
    let summary = db.group();
    assert_eq!(db.track(a), db.track(b));
    assert_eq!(summary.recordings_removed, 1);
    assert_eq!(db.tracks(), 1);
    assert_eq!(db.count("SELECT CAST(bpm AS INTEGER) FROM analysis"), 120);
}

#[test]
fn tracks_holding_the_same_audio_merge_into_the_lowest_one() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(1, "b.mp3", Some(B));
    db.group();
    let lowest = db.track(a).unwrap();
    // Two tracks that turn out to hold the same audio.
    db.set_hash(b, Some(A));
    let summary = db.group();
    assert_eq!(db.track(a), Some(lowest));
    assert_eq!(db.track(b), Some(lowest));
    assert_eq!(db.tracks(), 1);
    assert_eq!(summary.recordings_removed, 1);
}

#[test]
fn a_copy_of_a_file_that_has_gone_missing_joins_its_track() {
    let db = db();
    let old = db.file(1, "old/a.mp3", Some(A));
    db.group();
    let track = db.track(old);
    // The folder was moved: the old file is gone, the same audio is found.
    db.set_present(old, false);
    let moved = db.file(1, "new/a.mp3", Some(A));
    db.group();
    assert_eq!(db.track(moved), track);
    assert_eq!(db.track(old), track, "a missing file keeps its track");
    assert_eq!(db.tracks(), 1);
}

// ---- versions, Library tracks, rekordbox ----------------------------------

#[test]
fn versions_never_merge() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(1, "b.mp3", Some(B));
    db.group();
    let (ta, tb) = (db.track(a).unwrap(), db.track(b).unwrap());
    db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({ta}, {tb}, 'cut', 'user')"
    ));
    // Their audio now matches, but they're linked versions: leave them.
    db.set_hash(b, Some(A));
    let summary = db.group();
    assert_eq!((db.track(a), db.track(b)), (Some(ta), Some(tb)));
    assert_eq!((summary.moved, summary.left_alone), (0, 1));
    // And again: still nothing to do, and no error.
    assert_eq!(db.group().left_alone, 1);
}

#[test]
fn a_file_a_library_track_points_at_never_moves() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(2, "b.mp3", Some(B));
    db.group();
    let ta = db.track(a).unwrap();
    db.sql(&format!(
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES ({ta}, {a})"
    ));
    db.set_hash(a, Some(B));
    db.group();
    assert_eq!(db.track(a), Some(ta));
    // The other file, not pinned, joins it instead of the other way round.
    assert_eq!(db.track(b), Some(ta));
    assert_eq!(db.tracks(), 1);
}

#[test]
fn a_moved_file_takes_its_matched_rekordbox_track_with_it() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(2, "b.mp3", Some(B));
    db.group();
    let ta = db.track(a).unwrap();
    db.sql(&format!(
        "INSERT INTO rekordbox_track (attributes, location_key, read_at, file_id, recording_id,
                                      relink_method)
         VALUES ('{{\"TrackID\":\"7\",\"Location\":\"file://localhost/E:/a.mp3\"}}', 'E:/a.mp3',
                 '2026-09-29T00:00:00.000Z', {a}, {ta}, 'path')"
    ));
    db.set_hash(a, Some(B));
    db.group();
    assert_eq!(db.track(a), db.track(b));
    assert_eq!(
        db.count("SELECT recording_id FROM rekordbox_track WHERE track_id = 7"),
        db.track(a).unwrap()
    );
    // The track it left is no longer needed, and nothing was refused.
    assert_eq!(db.tracks(), 1);
}

// ---- best file ------------------------------------------------------------

#[test]
fn a_track_with_no_best_gets_its_lowest_id_present_file_and_files_marked_extra_are_skipped() {
    let db = db();
    let gone = db.file(1, "a.mp3", Some(A));
    let extra = db.file(1, "b.mp3", Some(A));
    let a3 = db.file(2, "c.mp3", Some(A));
    let lone = db.file(2, "d.mp3", None);
    db.group();
    assert_eq!(db.role(gone), "best");
    assert_eq!(db.role(lone), "best");
    db.sql("UPDATE recording_file SET role = 'undecided' WHERE role = 'best'");
    db.sql(&format!(
        "UPDATE recording_file SET role = 'extra' WHERE file_id = {extra}"
    ));
    db.set_present(gone, false);
    db.group();
    assert_eq!(db.role(gone), "undecided", "a file on disk beats one gone");
    assert_eq!(db.role(extra), "extra");
    assert_eq!(db.role(a3), "best");
    assert_eq!(
        db.count("SELECT count(*) FROM recording_file WHERE role = 'best'"),
        2
    );
}

#[test]
fn a_best_that_is_already_chosen_is_kept() {
    let db = db();
    let a1 = db.file(1, "a1.mp3", Some(A));
    let a2 = db.file(1, "a2.mp3", Some(A));
    db.group();
    assert_eq!(db.role(a1), "best");
    db.sql("UPDATE recording_file SET role = 'undecided' WHERE role = 'best'");
    db.sql(&format!(
        "UPDATE recording_file SET role = 'best' WHERE file_id = {a2}"
    ));
    let summary = db.group();
    assert_eq!(db.role(a1), "undecided");
    assert_eq!(db.role(a2), "best");
    assert_eq!(summary.moved, 0);
}

#[test]
fn a_file_that_moves_starts_over_as_undecided_and_the_track_it_left_gets_a_new_best() {
    let db = db();
    let a1 = db.file(1, "a1.mp3", Some(A));
    let a2 = db.file(1, "a2.mp3", Some(A));
    db.file(1, "a3.mp3", Some(A));
    let b = db.file(2, "b.mp3", Some(B));
    db.group();
    assert_eq!(db.role(a1), "best");
    // The best file's audio changes to B's.
    db.set_hash(a1, Some(B));
    db.group();
    assert_eq!(db.track(a1), db.track(b));
    assert_eq!(db.role(b), "best");
    assert_eq!(db.role(a1), "undecided");
    assert_eq!(db.role(a2), "best", "the track it left picked a new best");
}
