//! 0029: `track_merge` (a track the user merged into another in Review,
//! kept with no files while the merge stands) and `replaced_entry.reason`.
//! ROADMAP 1.12, 1.9 rule 7, §2.

use super::{accepts, db, db_at, one, refuses};
use crate::db::migrations::{run, MIGRATIONS};

const TWO_TRACKS: &str = "INSERT INTO recording (title) VALUES ('One'), ('Two');";

#[test]
fn a_merge_names_the_track_that_left_once_and_holds_it_in_place() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        &format!("{TWO_TRACKS} INSERT INTO track_merge (recording_id, into_id) VALUES (2, 1);"),
    );
    // One record per track that left.
    refuses(
        &conn,
        "INSERT INTO track_merge (recording_id, into_id) VALUES (2, 1)",
        "UNIQUE",
    );
    // A track isn't merged into itself.
    refuses(
        &conn,
        "INSERT INTO track_merge (recording_id, into_id) VALUES (1, 1)",
        "CHECK",
    );
    // The track that left can't be deleted while the merge stands.
    refuses(&conn, "DELETE FROM recording WHERE id = 2", "FOREIGN KEY");
    // The track it went into isn't held: the record is what the merge did.
    accepts(&conn, "DELETE FROM recording WHERE id = 1");
    refuses(
        &conn,
        "INSERT INTO track_merge (recording_id, into_id) VALUES (9, 2)",
        "FOREIGN KEY",
    );
}

#[test]
fn a_replaced_entry_says_whether_a_switch_or_a_merge_left_it_behind() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO replaced_entry (recording_id, location, location_key)
         VALUES (1, 'file://localhost/E:/Old/a.mp3', 'e:/old/a.mp3');
         INSERT INTO replaced_entry (recording_id, location, location_key, reason)
         VALUES (2, 'file://localhost/E:/Old/b.mp3', 'e:/old/b.mp3', 'merge');",
    );
    let switch: String = one(&conn, "SELECT reason FROM replaced_entry WHERE id = 1");
    assert_eq!(switch, "switch");
    refuses(
        &conn,
        "INSERT INTO replaced_entry (recording_id, location, location_key, reason)
         VALUES (3, 'file://localhost/E:/Old/c.mp3', 'e:/old/c.mp3', 'removed')",
        "CHECK",
    );
}

#[test]
fn the_migration_keeps_every_replaced_entry_as_a_switch() {
    let (_dir, mut conn) = db_at(28);
    accepts(
        &conn,
        "INSERT INTO replaced_entry (recording_id, location, location_key)
         VALUES (1, 'file://localhost/E:/Old/a.mp3', 'e:/old/a.mp3');",
    );
    run(&mut conn, &MIGRATIONS[..29]).unwrap();
    let switches: i64 = one(
        &conn,
        "SELECT count(*) FROM replaced_entry WHERE reason = 'switch'",
    );
    let merges: i64 = one(&conn, "SELECT count(*) FROM track_merge");
    assert_eq!((switches, merges), (1, 0));
}
