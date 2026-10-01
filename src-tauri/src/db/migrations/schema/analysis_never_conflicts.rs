//! 0014: analysis fields never become conflicts (ROADMAP 1.9 rule 3, 1.10).

use rusqlite::Connection;

use super::{accepts, db, db_at, one, refuses};
use crate::db::migrations::{run, MIGRATIONS};

const SETUP: &str = "
    INSERT INTO recording (title) VALUES ('Track 1');
    INSERT INTO library_track (recording_id, source_status) VALUES (1, 'missing');";

const ANALYSIS_FIELDS: [&str; 4] = ["AverageBpm", "Tonality", "TEMPO", "POSITION_MARK"];

fn conflict(field: &str) -> String {
    format!(
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
         VALUES (1, '{field}', 'app', 'rekordbox', 'base')"
    )
}

fn with_a_library_track() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db();
    accepts(&conn, SETUP);
    (dir, conn)
}

#[test]
fn the_database_refuses_a_conflict_on_an_analysis_field() {
    let (_dir, conn) = with_a_library_track();
    for field in ANALYSIS_FIELDS {
        refuses(
            &conn,
            &conflict(field),
            "analysis fields never become conflicts",
        );
    }
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM conflict"), 0);
}

#[test]
fn a_conflict_cannot_be_moved_onto_an_analysis_field() {
    let (_dir, conn) = with_a_library_track();
    accepts(&conn, &conflict("Rating"));
    for field in ANALYSIS_FIELDS {
        refuses(
            &conn,
            &format!("UPDATE conflict SET field = '{field}'"),
            "analysis fields never become conflicts",
        );
    }
    assert_eq!(one::<String>(&conn, "SELECT field FROM conflict"), "Rating");
}

#[test]
fn every_other_field_can_still_conflict_and_be_settled() {
    let (_dir, conn) = with_a_library_track();
    // The fields 1.10 names, an attribute the app doesn't know, and names
    // that only look like analysis fields.
    for field in [
        "Name",
        "Genre",
        "Comments",
        "Rating",
        "Colour",
        "FutureField",
        "Bpm",
        "tonality",
        "averagebpm",
        "Tempo",
    ] {
        accepts(&conn, &conflict(field));
    }
    accepts(
        &conn,
        "UPDATE conflict SET status = 'kept_rekordbox', resolved_at = '2026-10-01T00:00:00Z'
         WHERE field = 'Rating'",
    );
    accepts(
        &conn,
        "UPDATE conflict SET field = 'Label' WHERE field = 'Genre'",
    );
}

#[test]
fn the_app_and_the_database_name_the_same_analysis_attributes() {
    // The two `TRACK` attributes the writer never sends are the two the
    // database refuses, alongside the two child elements.
    let mut attributes = crate::send_values::ANALYSIS_ATTRIBUTES.to_vec();
    attributes.extend(["TEMPO", "POSITION_MARK"]);
    attributes.sort_unstable();
    let mut refused = ANALYSIS_FIELDS.to_vec();
    refused.sort_unstable();
    assert_eq!(attributes, refused);
}

#[test]
fn an_analysis_conflict_made_before_the_rule_is_dropped_and_the_others_are_kept() {
    let (_dir, mut conn) = db_at(13);
    accepts(&conn, SETUP);
    accepts(&conn, &conflict("Tonality"));
    accepts(&conn, &conflict("TEMPO"));
    accepts(&conn, &conflict("Rating"));
    run(&mut conn, &MIGRATIONS[..14]).unwrap();
    assert_eq!(
        one::<String>(&conn, "SELECT group_concat(field) FROM conflict"),
        "Rating"
    );
}
