//! The track table's sort and filter rows (0032; 1cA-3, 1cA-4).

use super::{accepts, db, db_at, one, refuses};
use crate::db::migrations::{run, MIGRATIONS};

/// How many migrations come before this one's.
fn before() -> usize {
    MIGRATIONS
        .iter()
        .position(|m| m.name.ends_with("_track_row.sql"))
        .expect("the track row migration")
}

fn due(conn: &rusqlite::Connection) -> Vec<i64> {
    conn.prepare("SELECT recording_id FROM search_due ORDER BY 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Two tracks with a file each, a Library track for each, two crates, the
/// first track in the first crate, and a stored verdict for the second file; nothing due.
fn with_tracks() -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
         INSERT INTO recording DEFAULT VALUES;
         INSERT INTO recording DEFAULT VALUES;
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'b.mp3', 'b.mp3');
         INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 1, 'best');
         INSERT INTO recording_file (recording_id, file_id, role) VALUES (2, 2, 'best');
         INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (1, 'linked', 1, 'ok');
         INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (2, 'linked', 2, 'ok');
         INSERT INTO crate (name, kind) VALUES ('One', 'static');
         INSERT INTO crate (name, kind) VALUES ('Two', 'static');
         INSERT INTO crate_entry (crate_id, library_track_id, kind) VALUES (1, 1, 'member');
         INSERT INTO file_verdict (file_id, rules_version, inputs, verdict)
             VALUES (2, 1, '[]', 'low_bitrate');
         INSERT INTO analysis (recording_id, source, bpm, key) VALUES (1, 'rekordbox', 124, '8A');
         DELETE FROM search_due;",
    );
    (dir, conn)
}

#[test]
fn every_track_is_due_when_the_rows_arrive_and_none_is_written_yet() {
    let (dir, conn) = db_at(before());
    accepts(
        &conn,
        "INSERT INTO recording DEFAULT VALUES; INSERT INTO recording DEFAULT VALUES;
         DELETE FROM search_due;",
    );
    drop(conn);
    let mut conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
    run(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(due(&conn), vec![1, 2]);
    let rows: i64 = one(&conn, "SELECT count(*) FROM track_row");
    assert_eq!(rows, 0, "the rows are the index job's to write");
}

#[test]
fn a_row_holds_a_title_key_and_whole_flags_and_one_row_to_a_track() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO track_row (recording_id, title_key) VALUES (1, 'a')",
    );
    let flags: i64 = one(&conn, "SELECT flags FROM track_row");
    assert_eq!(flags, 0, "no flag until the job says so");
    refuses(
        &conn,
        "INSERT INTO track_row (recording_id) VALUES (2)",
        "NULL",
    );
    refuses(
        &conn,
        "INSERT INTO track_row (recording_id, title_key) VALUES (1, 'b')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO track_row (recording_id, title_key, flags) VALUES (3, 'b', 'many')",
        "INTEGER",
    );
}

#[test]
fn every_sort_has_an_index_that_holds_the_flags_and_the_format_and_a_descending_sort_has_none() {
    let (_dir, conn) = db();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'track_row' AND name LIKE 'track_row_%' ORDER BY 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for column in [
        "title", "artist", "genre", "bpm", "key", "time", "rating", "added", "file", "folder",
    ] {
        let name = format!("track_row_{column}");
        assert!(names.contains(&name), "{name} in {names:?}");
        let sql: String = one(
            &conn,
            &format!("SELECT sql FROM sqlite_master WHERE name = '{name}'"),
        );
        assert!(sql.ends_with("recording_id, flags, format)"), "{sql}");
        // A descending sort reads this index backwards.
        assert!(!sql.contains("DESC"), "{sql}");
    }
    // And one narrow index to count a filter's rows by.
    assert!(names.contains(&"track_row_flags".to_owned()));
    assert_eq!(names.len(), 11, "{names:?}");
}

#[test]
fn a_change_of_bpm_key_or_length_notes_the_track_as_due() {
    for (change, expected) in [
        (
            "INSERT INTO analysis (recording_id, source, bpm) VALUES (2, 'tag', 120)",
            vec![2],
        ),
        (
            "UPDATE analysis SET bpm = 125 WHERE recording_id = 1",
            vec![1],
        ),
        (
            "UPDATE analysis SET key = '9A' WHERE recording_id = 1",
            vec![1],
        ),
        ("DELETE FROM analysis WHERE recording_id = 1", vec![1]),
        (
            "UPDATE analysis SET recording_id = 2 WHERE recording_id = 1",
            vec![1, 2],
        ),
        ("UPDATE file SET duration_ms = 180000 WHERE id = 2", vec![2]),
        (
            "UPDATE file SET sniffed_format = 'flac' WHERE id = 1",
            vec![1],
        ),
    ] {
        let (_dir, conn) = with_tracks();
        accepts(&conn, change);
        assert_eq!(due(&conn), expected, "{change}");
    }
}

#[test]
fn a_change_the_filters_depend_on_notes_the_track_as_due() {
    for (change, expected) in [
        // The scan's mark on a Library track.
        (
            "UPDATE library_track SET source_status = 'missing' WHERE id = 1",
            vec![1],
        ),
        // Crates.
        (
            "INSERT INTO crate_entry (crate_id, library_track_id, kind) VALUES (2, 1, 'member')",
            vec![1],
        ),
        ("DELETE FROM crate_entry", vec![1]),
        // An entry pointed at another Library track: both tracks change.
        ("UPDATE crate_entry SET library_track_id = 2", vec![1, 2]),
        (
            "UPDATE crate_entry SET kind = 'member', crate_id = 2",
            vec![],
        ),
        ("UPDATE crate_entry SET library_track_id = 1", vec![]),
        // A crate going takes its entries, and notes the track.
        ("DELETE FROM crate WHERE id = 1", vec![1]),
    ] {
        let (_dir, conn) = with_tracks();
        accepts(&conn, change);
        assert_eq!(due(&conn), expected, "{change}");
    }
}

#[test]
fn a_change_that_leaves_bpm_key_format_and_the_rest_alone_notes_nothing() {
    let (_dir, conn) = with_tracks();
    for change in [
        "UPDATE analysis SET bpm = bpm, key = key WHERE recording_id = 1",
        "UPDATE analysis SET confidence = 0.5 WHERE recording_id = 1",
        "UPDATE file SET duration_ms = duration_ms, sniffed_format = sniffed_format",
        "UPDATE library_track SET source_status = source_status",
        "UPDATE crate_entry SET kind = kind, added_at = added_at",
    ] {
        accepts(&conn, change);
        assert_eq!(due(&conn), Vec::<i64>::new(), "{change}");
    }
}

#[test]
fn a_quality_verdict_a_decision_or_a_better_file_notes_nothing_because_the_filter_reads_reviews_definition(
) {
    let (_dir, conn) = with_tracks();
    for change in [
        "INSERT INTO file_verdict (file_id, rules_version, inputs, verdict) VALUES (1, 1, '[]', 'ok')",
        "UPDATE file_verdict SET verdict = 'ok', rules_version = 2 WHERE file_id = 2",
        "DELETE FROM file_verdict WHERE file_id = 2",
        "INSERT INTO better_file (recording_id, file_id, step) VALUES (1, 2, 'format')",
        "INSERT INTO review_decision (kind, linked_key) VALUES ('keep_anyway', x'0102')",
    ] {
        accepts(&conn, change);
        assert_eq!(due(&conn), Vec::<i64>::new(), "{change}");
    }
}
