//! What the user decided, and what the job does with it: remembered
//! decisions (`grouping_decision`), the user's own version links, and the
//! move that refuses to merge versions.

use super::support::*;
use crate::grouping::{remember, CantRemember, Refused, Verdict};

const LEN: usize = 900;

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

impl Db {
    /// The one way a decision is written.
    fn remember(&self, a: i64, b: i64, verdict: Verdict) -> Result<(), CantRemember> {
        self.writer
            .call(move |c| remember(c, a, b, verdict))
            .unwrap()
    }
}

/// A track and its re-encode: duplicates by every rule.
fn two_duplicates(db: &Db) -> (i64, i64) {
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.group();
    (a, b)
}

/// The same title, half the audio shared: the rules send it to review.
fn a_review_pair(db: &Db) -> (i64, i64) {
    let velo = track(1, LEN);
    let other_take = [&velo[..LEN / 2], &track(9, LEN / 2)[..]].concat();
    let a = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", "Velo", 2, other_take));
    db.group();
    (a, b)
}

// ---- keep apart -------------------------------------------------------------

#[test]
fn a_remembered_keep_apart_survives_every_later_run_and_the_pair_is_not_open_again() {
    let db = db();
    let (a, b) = two_duplicates(&db);
    assert_eq!(db.remember(a, b, Verdict::Apart), Ok(()));
    for run in 0..3 {
        db.decide();
        assert_ne!(db.track(a), db.track(b), "run {run}");
        assert_eq!(
            db.stands(a, b),
            ("duplicate".into(), "decided".into()),
            "run {run}"
        );
    }
    assert_eq!(db.tracks(), 2);
}

#[test]
fn keep_both_for_two_library_tracks_with_identical_audio_is_remembered() {
    let db = db();
    let a = db.file(1, "Velo.mp3", Some(hash(1)));
    let b = db.file(1, "gigs/Velo.mp3", Some(hash(2)));
    db.group();
    db.to_library(a);
    db.to_library(b);
    db.set_hash(b, Some(hash(1)));
    db.decide();
    assert_eq!(db.stands(a, b).1, "both_in_library");
    assert_eq!(db.remember(a, b, Verdict::Apart), Ok(()));
    db.decide();
    assert_eq!(db.stands(a, b).1, "decided");
    assert_eq!(db.tracks(), 2);
    // A third copy outside the Library isn't decided by it: it's the same
    // as both, and joins neither.
    let c = db.file(1, "new/Velo.mp3", Some(hash(1)));
    db.decide();
    assert_eq!(db.tracks(), 3);
    assert_eq!(db.stands(a, c).1, "set_disagrees");
    assert_eq!(db.stands(a, b).1, "decided");
}

#[test]
fn a_decision_is_asked_once_per_pair_of_tracks_not_once_per_pair_of_files() {
    // Two tracks of two files each, every pair between them a review.
    let db = db();
    let (velo, take) = (track(1, LEN), track(9, LEN / 2));
    let other_take = [&velo[..LEN / 2], &take[..]].concat();
    let a1 = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let a2 = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    let b1 = db.add(file("live/Velo.flac", "Velo", 3, other_take.clone()));
    let b2 = db.add(file(
        "live/Velo.mp3",
        "Velo",
        4,
        reencoded(&other_take, 2, 7),
    ));
    db.decide();
    assert_eq!(db.groups(), [vec![a1, a2], vec![b1, b2]]);
    for (a, b) in [(a1, b1), (a1, b2), (a2, b1), (a2, b2)] {
        assert_eq!(db.stands(a, b), ("review".into(), "open".into()));
    }
    // One answer, on one pair of files.
    assert_eq!(db.remember(a2, b1, Verdict::Apart), Ok(()));
    db.decide();
    for (a, b) in [(a1, b1), (a1, b2), (a2, b1), (a2, b2)] {
        assert_eq!(db.stands(a, b).1, "decided", "files {a} and {b}");
    }
    assert_eq!(db.groups(), [vec![a1, a2], vec![b1, b2]]);
}

// ---- the same ------------------------------------------------------------------

#[test]
fn the_same_makes_a_review_pair_one_track_whatever_the_rules_say() {
    let db = db();
    let (a, b) = a_review_pair(&db);
    db.decide();
    assert_ne!(db.track(a), db.track(b));
    assert_eq!(db.stands(a, b), ("review".into(), "open".into()));
    assert_eq!(db.remember(a, b, Verdict::Same), Ok(()));
    db.decide();
    assert_eq!(db.track(a), db.track(b));
    assert_eq!(db.tracks(), 1);
    // The rules' answer is still what's stored; the track is the user's.
    assert_eq!(db.stands(a, b), ("review".into(), "in_one_track".into()));
}

#[test]
fn the_same_never_merges_two_library_tracks() {
    let db = db();
    let (a, b) = two_duplicates(&db);
    db.to_library(a);
    db.to_library(b);
    assert_eq!(db.remember(a, b, Verdict::Same), Ok(()));
    db.decide();
    assert_ne!(db.track(a), db.track(b));
    assert_eq!(db.count("SELECT count(*) FROM library_track"), 2);
    assert_eq!(db.stands(a, b).1, "decided");
}

#[test]
fn a_later_decision_about_the_same_two_pieces_of_audio_replaces_the_earlier_one() {
    let db = db();
    let (a, b) = two_duplicates(&db);
    assert_eq!(db.remember(a, b, Verdict::Apart), Ok(()));
    db.decide();
    assert_ne!(db.track(a), db.track(b));
    // Asked the other way round: the same two pieces of audio.
    assert_eq!(db.remember(b, a, Verdict::Same), Ok(()));
    assert_eq!(db.count("SELECT count(*) FROM grouping_decision"), 1);
    db.decide();
    assert_eq!(db.track(a), db.track(b));
}

// ---- a decision names the audio, not the file -------------------------------------

#[test]
fn a_decision_survives_a_retag() {
    let db = db();
    let (a, b) = two_duplicates(&db);
    db.remember(a, b, Verdict::Apart).unwrap();
    db.decide();
    db.retag(b, Some("Velo"), Some("Halden Rook & Ivo Marsh"));
    db.decide();
    assert_ne!(db.track(a), db.track(b));
    assert_eq!(db.stands(a, b).1, "decided");
}

#[test]
fn a_decision_survives_a_file_moved_and_a_music_folder_removed_and_added_again() {
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.group();
    db.remember(a, b, Verdict::Apart).unwrap();
    db.decide();
    // Folder 1 is removed: its file rows go, with everything kept per file.
    let removed = db
        .writer
        .call(|c| crate::scan::folders::remove(c, crate::scan::folders::MusicFolderId(1)))
        .unwrap();
    assert_eq!(removed, Ok(()));
    assert_eq!(db.count("SELECT count(*) FROM file"), 0);
    assert_eq!(db.count("SELECT count(*) FROM grouping_pair"), 0);
    assert_eq!(db.count("SELECT count(*) FROM grouping_decision"), 1);
    // The same two files are found again, elsewhere, as new rows: after
    // another file, and the other way round, so neither has the row id it
    // had. A third copy of the same track comes with them.
    db.sql("INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'A', 'A')");
    db.sql("UPDATE music_folder SET id = 1 WHERE rel_path = 'A'");
    let other = db.add(file("moved/Velo.m4a", "Velo", 7, reencoded(&velo, 2, 7)));
    let b = db.add(file("moved/Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    let a = db.add(file("moved/Velo.flac", "Velo", 1, velo.clone()));
    assert_eq!((other, b, a), (1, 2, 3), "the row ids are used again");
    db.decide();
    assert_ne!(db.track(a), db.track(b), "still kept apart");
    assert_eq!(db.stands(a, b), ("duplicate".into(), "decided".into()));
    // The decision was about those two pieces of audio, and no other pair.
    assert_ne!(db.stands(other, b).1, "decided");
    assert_ne!(db.stands(other, a).1, "decided");
}

/// Track A (a flac and its re-encode, merged) and file b, another re-encode
/// that would join them: the user keeps b apart, answering on the pair of
/// the flac and b.
fn a_track_of_two_and_a_file_kept_apart(db: &Db) -> (i64, i64, i64) {
    let velo = track(1, LEN);
    let a1 = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let a2 = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.decide();
    assert_eq!(db.track(a1), db.track(a2));
    let b = db.add(file("other/Velo.mp3", "Velo", 3, reencoded(&velo, 2, 7)));
    db.group();
    assert_eq!(db.remember(a1, b, Verdict::Apart), Ok(()));
    assert_eq!(
        db.count("SELECT count(*) FROM grouping_decision"),
        2,
        "one row for each kind of audio in track A, against b's"
    );
    db.decide();
    assert_eq!(db.groups(), [vec![a1, a2], vec![b]]);
    assert_eq!(db.stands(a2, b), ("duplicate".into(), "decided".into()));
    (a1, a2, b)
}

#[test]
fn a_decision_still_holds_when_the_file_it_was_made_on_is_overwritten() {
    let db = db();
    let (a1, a2, b) = a_track_of_two_and_a_file_kept_apart(&db);
    // The flac is overwritten with another song, and every stage has been
    // through it.
    db.retag(a1, Some("Nightdrive"), Some("Halden Rook"));
    db.overwrite(a1, Some(hash(9)), None);
    db.set_items(a1, track(77, LEN));
    db.decide();
    db.decide();
    assert_ne!(db.track(a1), db.track(a2), "it left track A");
    assert_ne!(db.track(a2), db.track(b), "A and b are still kept apart");
    assert_eq!(db.stands(a2, b), ("duplicate".into(), "decided".into()));
    assert_eq!(db.tracks(), 3);
}

#[test]
fn a_decision_still_holds_when_the_music_folder_of_the_file_it_was_made_on_is_removed() {
    let db = db();
    let velo = track(1, LEN);
    // The flac is in music folder 2, the other two files in folder 1.
    let a1 = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    db.sql(&format!(
        "UPDATE file SET music_folder_id = 2 WHERE id = {a1}"
    ));
    let a2 = db.add(file("Velo.mp3", "Velo", 2, reencoded(&velo, 1, 7)));
    db.decide();
    assert_eq!(db.track(a1), db.track(a2));
    let b = db.add(file("other/Velo.mp3", "Velo", 3, reencoded(&velo, 2, 7)));
    db.group();
    assert_eq!(db.remember(a1, b, Verdict::Apart), Ok(()));
    db.decide();
    assert_eq!(db.groups(), [vec![a1, a2], vec![b]]);
    let removed = db
        .writer
        .call(|c| crate::scan::folders::remove(c, crate::scan::folders::MusicFolderId(2)))
        .unwrap();
    assert_eq!(removed, Ok(()));
    assert_eq!(db.count("SELECT count(*) FROM file"), 2);
    db.decide();
    db.decide();
    assert_ne!(db.track(a2), db.track(b), "A and b are still kept apart");
    assert_eq!(db.stands(a2, b), ("duplicate".into(), "decided".into()));
}

#[test]
fn keeping_apart_two_files_that_already_share_a_track_changes_nothing() {
    // A merge is never undone by the job: taking a file out of a track is
    // an action of its own.
    let db = db();
    let (a, b) = two_duplicates(&db);
    db.decide();
    assert_eq!(db.track(a), db.track(b));
    assert_eq!(db.remember(a, b, Verdict::Apart), Ok(()));
    assert_eq!(db.count("SELECT count(*) FROM grouping_decision"), 1);
    db.decide();
    assert_eq!(db.track(a), db.track(b));
    assert_eq!(db.stands(a, b), ("duplicate".into(), "merged".into()));
}

#[test]
fn a_decision_cannot_be_remembered_for_one_file_or_for_audio_the_app_cannot_name() {
    let db = db();
    let hashed = db.file(1, "a.mp3", Some(A));
    let unhashed = db.file(1, "b.mp3", None);
    assert_eq!(
        db.remember(hashed, hashed, Verdict::Apart),
        Err(CantRemember::SameFile)
    );
    assert_eq!(
        db.remember(hashed, unhashed, Verdict::Apart),
        Err(CantRemember::NoAudioKnown)
    );
    assert_eq!(
        db.remember(hashed, 999, Verdict::Same),
        Err(CantRemember::NoAudioKnown)
    );
    assert_eq!(db.count("SELECT count(*) FROM grouping_decision"), 0);
    // A file with no audio hash is named by its whole-file hash.
    db.sql("UPDATE file SET blake3 = zeroblob(32) WHERE rel_path = 'b.mp3'");
    assert_eq!(db.remember(hashed, unhashed, Verdict::Apart), Ok(()));
}

// ---- the user's own version link ----------------------------------------------------

#[test]
fn a_version_link_the_user_made_settles_its_tracks_never_merged_never_asked_never_touched() {
    let db = db();
    let (a, b) = two_duplicates(&db);
    let (ta, tb) = (db.track(a).unwrap(), db.track(b).unwrap());
    db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, label, source)
         VALUES ({tb}, {ta}, 'cut', 'the user''s own', 'user')"
    ));
    let link = "SELECT id || ' ' || recording_a || ' ' || recording_b || ' ' || kind || ' '
                || label || ' ' || source || ' ' || confirmed || ' ' || created_at
                FROM version_link";
    let before: String = db
        .writer
        .call(move |c| c.query_row(link, [], |r| r.get(0)))
        .unwrap();
    for _ in 0..2 {
        let (_, summary) = db.decide();
        assert_eq!((db.track(a), db.track(b)), (Some(ta), Some(tb)));
        assert_eq!(db.stands(a, b), ("duplicate".into(), "decided".into()));
        assert_eq!((summary.links_made, summary.links_removed), (0, 0));
    }
    let after: String = db
        .writer
        .call(move |c| c.query_row(link, [], |r| r.get(0)))
        .unwrap();
    assert_eq!(after, before);
    assert_eq!(db.count("SELECT count(*) FROM version_link"), 1);
}

#[test]
fn a_link_the_job_made_goes_when_the_pair_no_longer_asks_for_it_and_a_confirmed_one_stays() {
    // Only the tags say which cut each file is.
    let db = db();
    let velo = track(1, LEN);
    let clean = db.add(file("a/Velo.mp3", "Velo (Clean)", 1, velo.clone()));
    let dirty = db.add(file(
        "b/Velo.mp3",
        "Velo (Dirty)",
        2,
        reencoded(&velo, 1, 7),
    ));
    db.decide();
    assert!(db.link(clean, dirty).is_some());
    // The second file was mislabeled: it's the Clean cut too.
    db.retag(dirty, Some("Velo (Clean)"), Some("Halden Rook"));
    let (_, summary) = db.decide();
    assert_eq!(summary.links_removed, 1);
    assert_eq!(db.count("SELECT count(*) FROM version_link"), 0);
    assert_eq!(db.track(clean), db.track(dirty), "duplicates now");

    // The same again, but the user confirmed the link: it's theirs.
    let db = super::support::db();
    let clean = db.add(file("a/Velo.mp3", "Velo (Clean)", 1, velo.clone()));
    let dirty = db.add(file(
        "b/Velo.mp3",
        "Velo (Dirty)",
        2,
        reencoded(&velo, 1, 7),
    ));
    db.decide();
    db.sql("UPDATE version_link SET confirmed = 1");
    db.retag(dirty, Some("Velo (Clean)"), Some("Halden Rook"));
    db.decide();
    assert_eq!(db.count("SELECT count(*) FROM version_link"), 1);
    assert_ne!(db.track(clean), db.track(dirty));
    assert_eq!(db.stands(clean, dirty).1, "decided");
}

#[test]
fn the_same_on_a_pair_the_job_linked_merges_it_in_one_pass() {
    // Clean and Dirty of one track, by their names: linked by the job.
    let db = db();
    let velo = track(1, LEN);
    let clean = db.add(file("Velo (Clean).mp3", "Velo (Clean)", 1, velo.clone()));
    let dirty = db.add(file(
        "Velo (Dirty).mp3",
        "Velo (Dirty)",
        2,
        reencoded(&velo, 1, 7),
    ));
    db.decide();
    assert_eq!(db.stands(clean, dirty), ("cut".into(), "linked".into()));
    assert!(db.link(clean, dirty).is_some());
    // The user says they're the same. One quick pass does all of it.
    assert_eq!(db.remember(clean, dirty, Verdict::Same), Ok(()));
    let summary = db.group();
    assert_eq!(db.track(clean), db.track(dirty));
    assert_eq!(
        (summary.moved, summary.links_removed, summary.left_alone),
        (1, 1, 0)
    );
    assert_eq!(db.count("SELECT count(*) FROM version_link"), 0);
    assert_eq!(
        db.stands(clean, dirty),
        ("cut".into(), "in_one_track".into())
    );
    // And a second pass, or a deciding job, changes nothing.
    let before = db.dump();
    let again = db.group();
    assert!(!again.changed_tracks());
    assert_eq!((again.links_made, again.links_removed), (0, 0));
    assert_eq!(db.dump(), before);
    assert_eq!(db.decide().0, Default::default());
    assert_eq!(db.dump(), before);
}

#[test]
fn a_run_over_an_unchanged_database_changes_nothing_after_the_users_decisions_and_links() {
    let db = db();
    // A linked pair the user calls the same.
    let velo = track(1, LEN);
    let clean = db.add(file("Velo (Clean).mp3", "Velo (Clean)", 1, velo.clone()));
    let dirty = db.add(file(
        "Velo (Dirty).mp3",
        "Velo (Dirty)",
        2,
        reencoded(&velo, 1, 7),
    ));
    // Two duplicates the user keeps apart.
    let drive = track(2, LEN);
    let kept = db.add(file("Nightdrive.flac", "Nightdrive", 3, drive.clone()));
    let apart = db.add(file(
        "Nightdrive.mp3",
        "Nightdrive",
        4,
        reencoded(&drive, 1, 7),
    ));
    // Two duplicates the user linked as versions.
    let harbor = track(3, LEN);
    let one = db.add(file("Paper Harbor.flac", "Paper Harbor", 5, harbor.clone()));
    let two = db.add(file(
        "Paper Harbor.mp3",
        "Paper Harbor",
        6,
        reencoded(&harbor, 1, 7),
    ));
    db.group();
    assert_eq!(db.remember(kept, apart, Verdict::Apart), Ok(()));
    let (t1, t2) = (db.track(one).unwrap(), db.track(two).unwrap());
    db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({t1}, {t2}, 'cut', 'user')"
    ));
    db.decide();
    assert_eq!(db.remember(clean, dirty, Verdict::Same), Ok(()));
    db.decide();
    assert_eq!(db.track(clean), db.track(dirty));
    assert_ne!(db.track(kept), db.track(apart));
    assert_ne!(db.track(one), db.track(two));
    let before = db.dump();
    assert!(before.contains("decision ") && before.contains(" user "));
    for _ in 0..2 {
        let (again, summary) = db.decide();
        assert_eq!(again, Default::default());
        assert!(!summary.changed_tracks());
        assert_eq!((summary.links_made, summary.links_removed), (0, 0));
        assert_eq!(db.dump(), before, "not a row was written");
        *db.names.lock().unwrap() = Default::default();
    }
    let quick = db.group();
    assert!(!quick.changed_tracks());
    assert_eq!(db.dump(), before);
}

// ---- the move itself ----------------------------------------------------------------

#[test]
fn moving_a_file_between_linked_versions_is_refused_by_the_move_itself() {
    // The schema refuses an UPDATE or a REPLACE that merges two linked
    // versions, but can't see a file taken out of one track and put into
    // the other (a delete, then an insert). So the move doesn't lean on the
    // schema: here the schema's triggers are gone, and it still refuses.
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(1, "b.mp3", Some(B));
    db.group();
    let (ta, tb) = (db.track(a).unwrap(), db.track(b).unwrap());
    db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({ta}, {tb}, 'cut', 'user');
         DROP TRIGGER recording_file_versions_never_merge;
         DROP TRIGGER recording_file_versions_never_merge_replace;"
    ));
    // What the triggers can't see goes through, with them or without:
    // that is the gap.
    db.sql(&format!(
        "SAVEPOINT gap;
         DELETE FROM recording_file WHERE file_id = {b};
         INSERT INTO recording_file (recording_id, file_id) VALUES ({ta}, {b});
         ROLLBACK TO gap;
         RELEASE gap;"
    ));
    assert_eq!(db.track(b), Some(tb));
    // The move refuses, either way round, and changes nothing.
    for (file, to) in [(b, ta), (a, tb)] {
        let moved = db
            .writer
            .call(move |c| crate::grouping::store::relocate(c, file, to))
            .unwrap();
        assert_eq!(moved, Err(Refused::LinkedVersions));
    }
    assert_eq!((db.track(a), db.track(b)), (Some(ta), Some(tb)));
    // And the grouping pass, given the same audio in both, leaves them.
    db.set_hash(b, Some(A));
    let summary = db.group();
    assert_eq!((db.track(a), db.track(b)), (Some(ta), Some(tb)));
    assert_eq!(
        (summary.moved, summary.refused, summary.left_alone),
        (0, 0, 1)
    );
}

#[test]
fn moving_a_file_a_library_track_points_at_is_refused_by_the_move_itself() {
    let db = db();
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(1, "b.mp3", Some(B));
    db.group();
    let (ta, tb) = (db.to_library(a), db.track(b).unwrap());
    let moved = db
        .writer
        .call(move |c| crate::grouping::store::relocate(c, a, tb))
        .unwrap();
    assert_eq!(moved, Err(Refused::Pinned));
    assert_eq!(db.track(a), Some(ta));
    // A file nothing points at moves, and starts over in its new track.
    let moved = db
        .writer
        .call(move |c| crate::grouping::store::relocate(c, b, ta))
        .unwrap();
    assert_eq!(moved, Ok(tb));
    assert_eq!(db.track(b), Some(ta));
    assert_eq!(db.role(b), "undecided");
}
