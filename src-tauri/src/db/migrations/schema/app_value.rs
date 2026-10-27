//! 0035: the app's own value per field of a Library track (ROADMAP §2
//! `app_value`, 1.10; 1cB-18).

use rusqlite::Connection;

use super::{accepts, db, db_at, insert, one, refuses};
use crate::conflicts::ANALYSIS_FIELDS;

const SETUP: &str = "
    INSERT INTO recording (title) VALUES ('Track 1'), ('Track 2');
    INSERT INTO library_track (recording_id, source_status) VALUES (1, 'missing'), (2, 'missing');";

fn with_library_tracks() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db();
    accepts(&conn, SETUP);
    (dir, conn)
}

fn own(track: i64, field: &str) -> String {
    format!(
        "INSERT INTO app_value (library_track_id, field, value, replaced)
         VALUES ({track}, '{field}', 'mine', 'theirs')"
    )
}

#[test]
fn the_table_arrives_with_migration_0035() {
    let (_dir, conn) = db_at(34);
    assert_eq!(
        one::<i64>(
            &conn,
            "SELECT count(*) FROM sqlite_master WHERE name = 'app_value'"
        ),
        0
    );
    let (_dir, conn) = db_at(35);
    let sql: String = one(
        &conn,
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'app_value'",
    );
    // What the operation log needs: a STRICT rowid table whose ids are
    // never used twice.
    assert!(sql.contains("STRICT"), "{sql}");
    assert!(sql.contains("AUTOINCREMENT"), "{sql}");
}

#[test]
fn the_database_refuses_an_app_value_on_an_analysis_field_track_id_or_location() {
    let (_dir, conn) = with_library_tracks();
    for field in ANALYSIS_FIELDS.into_iter().chain(["TrackID", "Location", ""]) {
        refuses(&conn, &own(1, field), "CHECK");
    }
    accepts(&conn, &own(1, "Rating"));
    for field in ANALYSIS_FIELDS {
        refuses(
            &conn,
            &format!("UPDATE app_value SET field = '{field}'"),
            "CHECK",
        );
    }
    assert_eq!(
        one::<String>(&conn, "SELECT field FROM app_value"),
        "Rating"
    );
}

#[test]
fn one_value_per_track_and_field_held_to_its_library_track() {
    let (_dir, conn) = with_library_tracks();
    accepts(&conn, &own(1, "Genre"));
    accepts(&conn, &own(2, "Genre"));
    refuses(&conn, &own(1, "Genre"), "UNIQUE");
    refuses(&conn, &own(99, "Genre"), "FOREIGN KEY");
    refuses(&conn, "DELETE FROM library_track WHERE id = 1", "FOREIGN KEY");
    refuses(
        &conn,
        "INSERT INTO app_value (library_track_id, field, value) VALUES (1, 'Name', NULL)",
        "NOT NULL",
    );
    // Empty is a value (the app wants the field empty); an unknown replaced
    // value is NULL.
    accepts(
        &conn,
        "INSERT INTO app_value (library_track_id, field, value) VALUES (1, 'Comments', '')",
    );
}

#[test]
fn a_new_app_value_never_takes_the_id_of_a_deleted_one() {
    let (_dir, conn) = with_library_tracks();
    let first = insert(&conn, &own(1, "Genre"));
    let second = insert(&conn, &own(1, "Name"));
    accepts(&conn, "DELETE FROM app_value");
    let third = insert(&conn, &own(2, "Genre"));
    assert!(third > second && second > first, "{first} {second} {third}");
}
