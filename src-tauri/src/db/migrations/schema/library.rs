//! 0004: `library_track`, `rekordbox_track`, `sync_base`, `conflict`
//! (ROADMAP 1.8–1.10, §2, §5.2).

use rusqlite::Connection;

use super::tracks::with_tracks;
use super::{accepts, columns, one, refuses};

/// `with_tracks` plus Library track 1: track 1, linked to file 1.
pub(in crate::db::migrations) fn with_a_library_track() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = with_tracks();
    accepts(
        &conn,
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 1)",
    );
    (dir, conn)
}

// Library tracks.

#[test]
fn a_library_track_stores_no_rekordbox_track_id() {
    // rekordbox reassigns TrackIDs on import (§5.2); tracks are matched by
    // Location.
    let (_dir, conn) = with_a_library_track();
    for column in columns(&conn, "library_track") {
        let c = column.to_lowercase();
        assert!(
            !c.contains("trackid") && !c.contains("track_id") && !c.contains("rekordbox"),
            "library_track.{column}"
        );
    }
}

#[test]
fn every_library_track_is_linked_in_phase_1() {
    // CLAUDE.md: the MVP never writes an audio file.
    let (_dir, conn) = with_a_library_track();
    let kind: String = one(&conn, "SELECT kind FROM library_track");
    assert_eq!(kind, "linked");
    refuses(
        &conn,
        "INSERT INTO library_track (recording_id, kind, managed_rel_path) VALUES (2, 'copy', 'Library/a.mp3')",
        "every Library track is linked until Phase 2",
    );
    refuses(
        &conn,
        "UPDATE library_track SET kind = 'copy', linked_file_id = NULL, managed_rel_path = 'a.mp3'",
        "every Library track is linked until Phase 2",
    );
}

#[test]
fn a_copy_would_need_a_library_path_and_no_linked_file_once_phase_2_allows_it() {
    // The shape rule that stays after Phase 2 drops the linked-only
    // triggers, tested here with them dropped.
    let (_dir, conn) = with_tracks();
    accepts(
        &conn,
        "DROP TRIGGER library_track_linked_only_in_phase_1_insert;
         DROP TRIGGER library_track_linked_only_in_phase_1_update;",
    );
    refuses(
        &conn,
        "INSERT INTO library_track (recording_id, kind) VALUES (1, 'copy')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO library_track (recording_id, kind, managed_rel_path, linked_file_id) VALUES (1, 'copy', 'a.mp3', 1)",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO library_track (recording_id, kind, managed_rel_path) VALUES (1, 'copy', '../a.mp3')",
        "CHECK",
    );
    accepts(
        &conn,
        "INSERT INTO library_track (recording_id, kind, managed_rel_path, source_file_id) VALUES (1, 'copy', 'House/a.mp3', 1)",
    );
}

#[test]
fn a_linked_track_points_at_a_file_and_has_no_library_path() {
    let (_dir, conn) = with_tracks();
    refuses(
        &conn,
        "INSERT INTO library_track (recording_id) VALUES (1)",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO library_track (recording_id, linked_file_id, managed_rel_path) VALUES (1, 1, 'a.mp3')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 99)",
        "FOREIGN KEY",
    );
}

#[test]
fn a_linked_track_may_lack_a_file_only_while_its_file_is_missing() {
    // A rekordbox track whose file was already gone when the Library was
    // started from rekordbox can still become a Library track, flagged.
    let (_dir, conn) = with_tracks();
    refuses(
        &conn,
        "INSERT INTO library_track (recording_id, source_status) VALUES (1, 'ok')",
        "CHECK",
    );
    accepts(
        &conn,
        "INSERT INTO library_track (recording_id, source_status) VALUES (1, 'missing')",
    );
    // Once it's back to ok, it must point at a file again.
    refuses(
        &conn,
        "UPDATE library_track SET source_status = 'ok'",
        "CHECK",
    );
    accepts(
        &conn,
        "UPDATE library_track SET source_status = 'ok', linked_file_id = 1",
    );
}

#[test]
fn a_track_is_in_the_library_once() {
    let (_dir, conn) = with_a_library_track();
    refuses(
        &conn,
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 2)",
        "UNIQUE",
    );
}

#[test]
fn deleting_a_track_or_a_file_cannot_take_a_library_track_with_it() {
    // Library tracks are removed only by an explicit in-app action.
    let (_dir, conn) = with_a_library_track();
    accepts(&conn, "DELETE FROM recording_file");
    refuses(&conn, "DELETE FROM recording WHERE id = 1", "FOREIGN KEY");
    refuses(&conn, "DELETE FROM file WHERE id = 1", "FOREIGN KEY");
    let left: i64 = one(&conn, "SELECT COUNT(*) FROM library_track");
    assert_eq!(left, 1);
}

#[test]
fn a_library_track_with_send_history_or_a_conflict_cannot_be_deleted_by_accident() {
    for child in [
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Rating', '0')",
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value) VALUES (1, 'Name', 'a', 'b')",
    ] {
        let (_dir, conn) = with_a_library_track();
        accepts(&conn, child);
        refuses(&conn, "DELETE FROM library_track", "FOREIGN KEY");
    }
}

#[test]
fn a_library_tracks_source_is_ok_or_missing_and_ok_by_default() {
    let (_dir, conn) = with_a_library_track();
    let status: String = one(&conn, "SELECT source_status FROM library_track");
    assert_eq!(status, "ok");
    accepts(&conn, "UPDATE library_track SET source_status = 'missing'");
    refuses(
        &conn,
        "UPDATE library_track SET source_status = 'gone'",
        "CHECK",
    );
}

// The rekordbox snapshot.

const ATTRIBUTES: &str = r#"{"TrackID": "12", "Name": "Déjà Nu", "Artist": "A, B & C",
    "Location": "file://localhost/E:/Music/%23hashtag/Track (1).mp3", "AverageBpm": "126.00",
    "Tonality": "8A", "PlayCount": "7", "Rating": "204", "Colour": "0xFF0000",
    "Comments": "/* Peak Time */", "Grouping": "Red", "Size": "123", "Kind": "MP3 File"}"#;

/// ATTRIBUTES' Location, decoded and in NFC, as the reader would key it.
const LOCATION_KEY: &str = "E:/Music/#hashtag/Track (1).mp3";

fn snapshot_row(conn: &Connection, attributes: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO rekordbox_track (attributes, location_key, read_at)
         VALUES (?1, ?2, '2026-09-28T00:00:00Z')",
        [attributes, LOCATION_KEY],
    )
}

#[test]
fn every_rekordbox_attribute_is_kept_verbatim_and_the_typed_columns_come_from_it() {
    let (_dir, conn) = with_tracks();
    snapshot_row(&conn, ATTRIBUTES).unwrap();
    let kept: String = one(&conn, "SELECT attributes FROM rekordbox_track");
    assert_eq!(kept, ATTRIBUTES, "the attributes were changed");
    let row = conn
        .query_row(
            "SELECT track_id, location, bpm, tonality, play_count, rating, colour, comments
             FROM rekordbox_track",
            [],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            12,
            "file://localhost/E:/Music/%23hashtag/Track (1).mp3".into(),
            126.0,
            "8A".into(),
            7,
            204,
            "0xFF0000".into(),
            "/* Peak Time */".into()
        )
    );
}

#[test]
fn the_typed_snapshot_columns_cannot_be_set_apart_from_the_attributes() {
    let (_dir, conn) = with_tracks();
    snapshot_row(&conn, ATTRIBUTES).unwrap();
    for sql in [
        "UPDATE rekordbox_track SET bpm = 1",
        "UPDATE rekordbox_track SET location = 'x'",
        "INSERT INTO rekordbox_track (attributes, location_key, read_at, track_id) VALUES ('{}', 'k', 'now', 1)",
    ] {
        let err = conn.execute_batch(sql).unwrap_err().to_string();
        assert!(err.contains("generated column"), "{sql}: {err}");
    }
}

#[test]
fn a_track_id_appears_once_per_read() {
    let (_dir, conn) = with_tracks();
    snapshot_row(&conn, ATTRIBUTES).unwrap();
    let again = ATTRIBUTES.replace("E:/Music", "F:/Other");
    let err = snapshot_row(&conn, &again).unwrap_err().to_string();
    assert!(err.contains("UNIQUE"), "{err}");
}

#[test]
fn a_snapshot_row_needs_a_whole_number_track_id_and_a_location() {
    let (_dir, conn) = with_tracks();
    for bad in [
        r#"{"Location": "file://localhost/E:/a.mp3"}"#,
        r#"{"TrackID": "12"}"#,
        r#"{"TrackID": "abc", "Location": "file://localhost/E:/a.mp3"}"#,
        r#"{"TrackID": "12.5", "Location": "file://localhost/E:/a.mp3"}"#,
        r#"["TrackID", "12"]"#,
        "not json",
    ] {
        let err = snapshot_row(&conn, bad).unwrap_err().to_string();
        // Text that isn't JSON fails already in the columns derived from it.
        assert!(
            err.contains("constraint failed") || err.contains("malformed JSON"),
            "{bad}: refused for another reason: {err}"
        );
    }
}

#[test]
fn rekordbox_values_are_never_edited_only_replaced_by_a_fresh_read() {
    let (_dir, conn) = with_tracks();
    snapshot_row(&conn, ATTRIBUTES).unwrap();
    for column in [
        "attributes",
        "location_key",
        "tempo",
        "position_marks",
        "my_tags",
        "playlists",
    ] {
        refuses(
            &conn,
            &format!("UPDATE rekordbox_track SET {column} = '[]'"),
            "the rekordbox snapshot is read-only",
        );
    }
    refuses(
        &conn,
        "UPDATE rekordbox_track SET read_at = 'later'",
        "the rekordbox snapshot is read-only",
    );
    // A fresh read replaces the rows.
    accepts(&conn, "DELETE FROM rekordbox_track");
    snapshot_row(&conn, ATTRIBUTES).unwrap();
    // The match for this read can be set or corrected. (A confirmed
    // relink is kept in `relink`, since these rows don't last.)
    accepts(
        &conn,
        "UPDATE rekordbox_track SET file_id = 1, recording_id = 1, relink_method = 'filename_only',
                relink_confidence = 0.5",
    );
}

#[test]
fn a_match_always_says_how_it_was_made() {
    let (_dir, conn) = with_tracks();
    snapshot_row(&conn, ATTRIBUTES).unwrap();
    refuses(&conn, "UPDATE rekordbox_track SET file_id = 1", "CHECK");
    refuses(
        &conn,
        "UPDATE rekordbox_track SET relink_method = 'path'",
        "CHECK",
    );
    refuses(
        &conn,
        "UPDATE rekordbox_track SET file_id = 1, relink_method = 'size'",
        "CHECK",
    );
    for method in [
        "path",
        "filename_duration",
        "unique_duration",
        "fingerprint",
        "filename_only",
        "gig_stick",
        "user",
    ] {
        accepts(
            &conn,
            &format!("UPDATE rekordbox_track SET file_id = 1, relink_method = '{method}'"),
        );
    }
    refuses(
        &conn,
        "UPDATE rekordbox_track SET file_id = 99",
        "FOREIGN KEY",
    );
}

#[test]
fn cues_grid_my_tags_and_playlists_are_json_arrays() {
    let (_dir, conn) = with_tracks();
    for column in ["tempo", "position_marks", "my_tags", "playlists"] {
        let err = conn
            .execute(
                &format!(
                    "INSERT INTO rekordbox_track (attributes, location_key, read_at, {column}) VALUES (?1, 'k', 'now', '{{}}')"
                ),
                [ATTRIBUTES],
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("CHECK"), "{column}: {err}");
    }
    conn.execute(
        r#"INSERT INTO rekordbox_track (attributes, location_key, read_at, tempo, position_marks, my_tags, playlists)
           VALUES (?1, 'k', 'now', '[{"Inizio": "0.025", "Bpm": "126.00"}]',
                   '[{"Name": "", "Type": "0", "Start": "0.025", "Num": "0"}]',
                   '["Peak Time"]', '[["Crates", "Friday"]]')"#,
        [ATTRIBUTES],
    )
    .unwrap();
}

#[test]
fn rekordbox_tracks_are_matched_by_location_through_an_index() {
    let (_dir, conn) = with_tracks();
    let mut stmt = conn
        .prepare("EXPLAIN QUERY PLAN SELECT id FROM rekordbox_track WHERE location = 'x'")
        .unwrap();
    let plan: Vec<String> = stmt
        .query_map([], |r| r.get(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|d| d.contains("rekordbox_track_by_location")),
        "{plan:?}"
    );
}

// Confirmed relinks.

#[test]
fn a_confirmed_relink_survives_a_fresh_read_of_rekordbox() {
    // The snapshot is replaced on every read (TrackIDs change too); the
    // confirmation is found again by the Location match key.
    let (_dir, conn) = with_tracks();
    snapshot_row(&conn, ATTRIBUTES).unwrap();
    conn.execute(
        "INSERT INTO relink (location_key, file_id, method, confidence) VALUES (?1, 2, 'filename_only', 0.6)",
        [LOCATION_KEY],
    )
    .unwrap();

    // A fresh read: every row replaced, and rekordbox renumbered the track.
    accepts(&conn, "DELETE FROM rekordbox_track");
    snapshot_row(
        &conn,
        &ATTRIBUTES.replace(r#""TrackID": "12""#, r#""TrackID": "40""#),
    )
    .unwrap();

    let (track_id, file, method): (i64, i64, String) = conn
        .query_row(
            "SELECT t.track_id, r.file_id, r.method
             FROM rekordbox_track t JOIN relink r ON r.location_key = t.location_key",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((track_id, file, method.as_str()), (40, 2, "filename_only"));
}

#[test]
fn a_location_has_one_confirmed_relink_to_a_known_file() {
    let (_dir, conn) = with_tracks();
    let sql = "INSERT INTO relink (location_key, file_id, method) VALUES ('E:/a.mp3', 1, 'user')";
    accepts(&conn, sql);
    refuses(&conn, sql, "UNIQUE");
    refuses(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('E:/b.mp3', 99, 'user')",
        "FOREIGN KEY",
    );
    refuses(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('E:/b.mp3', 1, 'size')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('', 1, 'user')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO relink (location_key, file_id, method, confidence) VALUES ('E:/b.mp3', 1, 'user', 2)",
        "CHECK",
    );
    // The file a confirmed relink points at can't be deleted under it.
    accepts(&conn, "DELETE FROM recording_file WHERE file_id = 1");
    refuses(&conn, "DELETE FROM file WHERE id = 1", "FOREIGN KEY");
}

// sync_base.

#[test]
fn a_field_has_one_base_value_per_library_track() {
    let (_dir, conn) = with_a_library_track();
    let sql = "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Rating', '204')";
    accepts(&conn, sql);
    refuses(&conn, sql, "UNIQUE");
    // Another field, or another track, has its own.
    accepts(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Name', 'x')",
    );
}

#[test]
fn a_base_value_belongs_to_a_known_library_track_and_names_its_field() {
    let (_dir, conn) = with_a_library_track();
    refuses(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (99, 'Rating', '0')",
        "FOREIGN KEY",
    );
    refuses(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, '', '0')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO sync_base (library_track_id, field) VALUES (1, 'Rating')",
        "NOT NULL",
    );
}

// Conflicts.

fn conflict(app: &str, rekordbox: &str, base: &str) -> String {
    format!(
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
         VALUES (1, 'Name', {app}, {rekordbox}, {base})"
    )
}

#[test]
fn a_conflict_is_only_a_field_both_sides_changed_to_different_values() {
    let (_dir, conn) = with_a_library_track();
    // Only one side changed: that's a send or a pull, not a conflict.
    refuses(&conn, &conflict("'base'", "'rb'", "'base'"), "CHECK");
    refuses(&conn, &conflict("'app'", "'base'", "'base'"), "CHECK");
    // Both changed to the same value: nothing to settle.
    refuses(&conn, &conflict("'same'", "'same'", "'base'"), "CHECK");
    refuses(&conn, &conflict("NULL", "NULL", "'base'"), "CHECK");
    accepts(&conn, &conflict("'app'", "'rb'", "'base'"));
}

#[test]
fn one_open_conflict_per_library_track_and_field() {
    let (_dir, conn) = with_a_library_track();
    accepts(&conn, &conflict("'a'", "'b'", "'c'"));
    refuses(&conn, &conflict("'d'", "'e'", "'c'"), "UNIQUE");
    // Once settled, a new one can open.
    accepts(
        &conn,
        "UPDATE conflict SET status = 'kept_app', resolved_at = '2026-09-28T00:00:00Z'",
    );
    accepts(&conn, &conflict("'d'", "'e'", "'c'"));
}

#[test]
fn a_conflict_is_open_until_settled_and_settling_records_when_and_how() {
    let (_dir, conn) = with_a_library_track();
    accepts(&conn, &conflict("'a'", "'b'", "'c'"));
    let status: String = one(&conn, "SELECT status FROM conflict");
    assert_eq!(status, "open");
    refuses(
        &conn,
        "UPDATE conflict SET status = 'kept_rekordbox'",
        "CHECK",
    );
    refuses(
        &conn,
        "UPDATE conflict SET resolved_at = '2026-09-28T00:00:00Z'",
        "CHECK",
    );
    refuses(
        &conn,
        "UPDATE conflict SET status = 'edited', resolved_at = '2026-09-28T00:00:00Z'",
        "CHECK",
    );
    refuses(
        &conn,
        "UPDATE conflict SET status = 'merged', resolved_at = '2026-09-28T00:00:00Z'",
        "CHECK",
    );
    accepts(
        &conn,
        "UPDATE conflict SET status = 'edited', resolved_value = 'd', resolved_at = '2026-09-28T00:00:00Z'",
    );
}
