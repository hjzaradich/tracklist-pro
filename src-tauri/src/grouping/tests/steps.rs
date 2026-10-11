//! A grouping pass in several transactions: any number of steps ends where
//! one does, and what a step leaves stands by itself.

use super::support::*;
use crate::grouping::store::{regroup_in_batches, regroup_step};
use crate::grouping::Summary;

/// Seven files: three hold one audio, two another, two have their own.
fn seven_files(db: &Db) {
    for (name, audio) in [
        ("a1.mp3", Some(A)),
        ("b1.mp3", Some(B)),
        ("a2.mp3", Some(A)),
        ("lone.mp3", None),
        ("b2.mp3", Some(B)),
        ("a3.mp3", Some(A)),
        ("c.mp3", Some(C)),
    ] {
        db.file(1, name, audio);
    }
}

#[test]
fn a_pass_in_small_steps_ends_with_the_tracks_one_step_gives() {
    let whole = db();
    seven_files(&whole);
    let at_once = whole.group();
    assert_eq!((at_once.placed, at_once.recordings_made), (7, 4));
    for batch in [1, 2, 3, 7] {
        let db = db();
        seven_files(&db);
        let summary = db
            .writer
            .call(move |c| regroup_in_batches(c, batch))
            .unwrap();
        assert_eq!(db.groups(), whole.groups(), "{batch} files a step");
        assert_eq!(
            (summary.placed, summary.recordings_made, summary.moved),
            (7, 4, 0),
            "{batch} files a step"
        );
        // And nothing is left to do.
        let again = db.group();
        assert_eq!(
            (again.placed, again.moved, again.recordings_made),
            (0, 0, 0)
        );
    }
}

#[test]
fn every_step_leaves_tracks_that_stand_by_themselves() {
    let db = db();
    seven_files(&db);
    let mut steps = 0;
    loop {
        let (step, more): (Summary, bool) = db.writer.call(|c| regroup_step(c, 2)).unwrap();
        steps += 1;
        // Whatever was placed so far is whole: no file in two tracks, no
        // track without a file, every track with a best file, and files
        // holding the same audio never in two tracks.
        assert_eq!(
            db.count(
                "SELECT count(*) FROM recording r
                 WHERE NOT EXISTS (SELECT 1 FROM recording_file WHERE recording_id = r.id)"
            ),
            0
        );
        assert_eq!(
            db.count(
                "SELECT count(*) FROM (SELECT recording_id FROM recording_file
                                       GROUP BY recording_id
                                       HAVING sum(role = 'best') <> 1)"
            ),
            0
        );
        assert_eq!(
            db.count(
                "SELECT count(*) FROM (SELECT f.audio_hash FROM recording_file rf
                                       JOIN file f ON f.id = rf.file_id
                                       WHERE f.audio_hash IS NOT NULL
                                       GROUP BY f.audio_hash
                                       HAVING count(DISTINCT rf.recording_id) > 1)"
            ),
            0
        );
        assert!(step.placed <= 2, "at most a batch a step");
        if !more {
            break;
        }
        assert!(steps < 20, "the pass never ended");
    }
    assert!(steps > 1, "it took several steps");
    assert_eq!(db.count("SELECT count(*) FROM recording_file"), 7);
    assert_eq!(db.tracks(), 4);
}

#[test]
fn tracks_made_before_placements_were_recorded_are_kept_and_recorded_in_steps() {
    // An upgrade: tracks exist, nothing says what their files were placed
    // at.
    let db = db();
    seven_files(&db);
    db.group();
    let before = db.groups();
    db.sql("DELETE FROM grouping_file");
    let summary = db.writer.call(|c| regroup_in_batches(c, 3)).unwrap();
    assert_eq!(db.groups(), before);
    assert_eq!((summary.placed, summary.moved), (0, 0));
    assert_eq!(db.count("SELECT count(*) FROM grouping_file"), 7);
    assert_eq!(
        db.count(
            "SELECT count(*) FROM grouping_file g JOIN file f ON f.id = g.file_id
             WHERE g.placed_key IS NOT f.audio_hash"
        ),
        0,
        "each is recorded as placed at what it holds"
    );
    // From then on a file that holds other audio is known to have changed.
    db.set_hash(1, Some(C));
    db.group();
    assert_eq!(db.track(1), db.track(7), "it joined the track of its audio");
}
