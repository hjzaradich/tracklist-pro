//! 0041: `library_track.stars_held`, the marker that a track's
//! stars stay rekordbox's (ROADMAP 1.10; 1cC-2).

use super::{accepts, db, one, refuses};

const TRACK: &str = "
    INSERT INTO recording (title) VALUES ('Track 1');
    INSERT INTO library_track (recording_id, kind, source_status)
    VALUES (1, 'linked', 'missing');";

#[test]
fn a_library_track_is_not_marked_until_something_marks_it() {
    let (_dir, conn) = db();
    accepts(&conn, TRACK);

    assert_eq!(one::<i64>(&conn, "SELECT stars_held FROM library_track"), 0);
}

#[test]
fn the_marker_is_either_set_or_not() {
    let (_dir, conn) = db();
    accepts(&conn, TRACK);

    accepts(&conn, "UPDATE library_track SET stars_held = 1");
    refuses(&conn, "UPDATE library_track SET stars_held = 2", "CHECK");
    refuses(
        &conn,
        "UPDATE library_track SET stars_held = NULL",
        "NOT NULL",
    );
}
