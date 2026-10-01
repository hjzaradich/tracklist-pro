//! What a merge does with the record of a track the user removed from the
//! Library (`library_removal`, 1aE-6).

use super::support::*;

/// Two tracks (the survivor is the lower one) that then turn out to hold
/// the same audio. Returns the survivor and the loser, before the merge.
fn two_tracks(db: &Db) -> (i64, i64, i64, i64) {
    let a = db.file(1, "a.mp3", Some(A));
    let b = db.file(1, "b.mp3", Some(B));
    db.group();
    (db.track(a).unwrap(), db.track(b).unwrap(), a, b)
}

fn removal(db: &Db, track: i64, at: &str) {
    db.sql(&format!(
        "INSERT INTO library_removal (recording_id, removed_at, last_sent_location)
         VALUES ({track}, '{at}', 'loc-{at}')"
    ));
}

/// Every record as `(track, removed_at)`.
fn records(db: &Db) -> Vec<(i64, String)> {
    db.writer
        .call(|c| {
            let mut s = c.prepare("SELECT recording_id, removed_at FROM library_removal")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap()
}

#[test]
fn a_removal_record_moves_to_the_surviving_track_of_a_merge() {
    let db = db();
    let (keep, lose, _a, b) = two_tracks(&db);
    removal(&db, lose, "2026-01-02T00:00:00.000Z");
    db.set_hash(b, Some(A));
    db.group();
    assert_eq!(db.tracks(), 1);
    assert_eq!(
        records(&db),
        [(keep, "2026-01-02T00:00:00.000Z".to_owned())]
    );
}

#[test]
fn a_removal_record_is_dropped_when_the_surviving_track_is_in_the_library() {
    let db = db();
    let (keep, lose, a, b) = two_tracks(&db);
    db.sql(&format!(
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES ({keep}, {a})"
    ));
    removal(&db, lose, "2026-01-02T00:00:00.000Z");
    db.set_hash(b, Some(A));
    db.group();
    assert_eq!(db.tracks(), 1);
    assert!(records(&db).is_empty());
}

#[test]
fn when_both_tracks_were_removed_the_earlier_record_is_kept_on_the_survivor() {
    for (survivor_at, loser_at) in [
        ("2026-03-01T00:00:00.000Z", "2026-02-01T00:00:00.000Z"),
        ("2026-02-01T00:00:00.000Z", "2026-03-01T00:00:00.000Z"),
    ] {
        let db = db();
        let (keep, lose, _a, b) = two_tracks(&db);
        removal(&db, keep, survivor_at);
        removal(&db, lose, loser_at);
        db.set_hash(b, Some(A));
        db.group();
        assert_eq!(db.tracks(), 1);
        let earlier = survivor_at.min(loser_at);
        assert_eq!(records(&db), [(keep, earlier.to_owned())]);
        // The kept record's own details came with it.
        let location: String = db
            .writer
            .call(|c| {
                c.query_row("SELECT last_sent_location FROM library_removal", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        assert_eq!(location, format!("loc-{earlier}"));
    }
}

#[test]
fn a_removal_record_stays_while_its_track_keeps_a_file() {
    let db = db();
    let (keep, lose, _a, _b) = two_tracks(&db);
    removal(&db, lose, "2026-01-02T00:00:00.000Z");
    db.group();
    assert_eq!(db.tracks(), 2);
    assert_eq!(
        records(&db),
        [(lose, "2026-01-02T00:00:00.000Z".to_owned())]
    );
    let _ = keep;
}

#[test]
fn a_removed_track_left_with_no_files_is_swept_with_its_record_if_it_was_never_sent() {
    let db = db();
    let (_keep, lose, _a, b) = two_tracks(&db);
    db.sql(&format!(
        "INSERT INTO library_removal (recording_id) VALUES ({lose})"
    ));
    // Its only file leaves the track (e.g. the folder was released).
    db.sql(&format!("DELETE FROM recording_file WHERE file_id = {b}"));
    db.set_present(b, false);
    let summary = db.group();
    assert_eq!(summary.recordings_removed, 1);
    assert_eq!(db.tracks(), 1);
    assert!(records(&db).is_empty());
}

#[test]
fn a_removed_track_left_with_no_files_stays_with_its_record_if_it_was_sent() {
    // The record is what tells the user to remove it in rekordbox by hand.
    let db = db();
    let (_keep, lose, _a, b) = two_tracks(&db);
    removal(&db, lose, "2026-01-02T00:00:00.000Z");
    db.sql(&format!("DELETE FROM recording_file WHERE file_id = {b}"));
    db.set_present(b, false);
    let summary = db.group();
    assert_eq!(summary.recordings_removed, 0);
    assert_eq!(db.tracks(), 2);
    assert_eq!(records(&db).len(), 1);
}
