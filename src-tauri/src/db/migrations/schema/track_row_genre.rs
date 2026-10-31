//! 0044: the genre the Overview counts, in two columns of `track_row` and an
//! index (1cD-R1b). What a writer leaves in the row is tested in
//! `search/tests/genre.rs`.

use rusqlite::Connection;

use super::{accepts, db, db_at};
use crate::db::migrations::{run, MIGRATIONS};
use crate::overview::overview_at;
use crate::search::settle;
use crate::search::tests::support::Mount;

/// How many migrations come before this one's.
fn before() -> usize {
    MIGRATIONS
        .iter()
        .position(|m| m.name.ends_with("_track_row_genre.sql"))
        .expect("the migration")
}

fn due(conn: &Connection) -> Vec<i64> {
    conn.prepare("SELECT recording_id FROM search_due ORDER BY 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Recordings 1 and 2 are Library tracks whose files are tagged with the
/// same genre in two spellings; recording 3 has a tagged file and no Library
/// track.
const TRACKS: &str = r#"
    INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
    INSERT INTO recording DEFAULT VALUES;
    INSERT INTO recording DEFAULT VALUES;
    INSERT INTO recording DEFAULT VALUES;
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, raw_tags)
        VALUES (1, 'a.mp3', 'a.mp3',
                '{"id3v2": [{"key": "TCON", "value": {"type": "text", "text": "House"}}]}');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, raw_tags)
        VALUES (1, 'b.mp3', 'b.mp3',
                '{"id3v2": [{"key": "TCON", "value": {"type": "text", "text": "house"}}]}');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, raw_tags)
        VALUES (1, 'c.mp3', 'c.mp3',
                '{"id3v2": [{"key": "TCON", "value": {"type": "text", "text": "Techno"}}]}');
    INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 1, 'best');
    INSERT INTO recording_file (recording_id, file_id, role) VALUES (2, 2, 'best');
    INSERT INTO recording_file (recording_id, file_id, role) VALUES (3, 3, 'best');
    INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
        VALUES (1, 'linked', 1, 'ok');
    INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
        VALUES (2, 'linked', 2, 'ok');
    DELETE FROM search_due;"#;

#[test]
fn the_row_gets_the_genre_and_its_group_and_an_index_to_count_and_filter_by() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO recording DEFAULT VALUES;
         INSERT INTO track_row (recording_id, title_key, genre, genre_group)
         VALUES (1, 'a', 'House', 'house');",
    );
    let columns: Vec<String> = conn
        .prepare(
            "SELECT name FROM pragma_table_info('track_row') WHERE name LIKE 'genre%' ORDER BY 1",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(columns, ["genre", "genre_group", "genre_key"]);
    // The Overview's count and the door's rows are read from the index alone.
    for sql in [
        "SELECT genre, count(*) FROM track_row WHERE genre_group IS NOT NULL GROUP BY genre_group, genre",
        "SELECT recording_id, flags FROM track_row WHERE genre_group = 'house' ORDER BY title_key, artist_key, recording_id",
    ] {
        let plan: Vec<String> = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let reads: Vec<&String> = plan
            .iter()
            .filter(|p| p.starts_with("SCAN") || p.starts_with("SEARCH"))
            .collect();
        assert!(
            !reads.is_empty()
                && reads
                    .iter()
                    .all(|p| p.contains("COVERING INDEX track_row_in_genre")),
            "{sql}: {plan:?}"
        );
    }
}

#[test]
fn an_upgrade_makes_every_library_track_due_and_no_other_track() {
    let (dir, conn) = db_at(before());
    accepts(&conn, TRACKS);
    drop(conn);
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    run(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(
        due(&conn),
        [1, 2],
        "the Library tracks, whose genres are unknown"
    );
}

#[test]
fn an_existing_database_gets_its_genres_from_the_index_job_and_says_still_counting_until_then() {
    let (dir, conn) = db_at(before());
    accepts(&conn, TRACKS);
    // The rows an older build wrote (this build's index job writes columns
    // that database doesn't have yet), without genres.
    accepts(
        &conn,
        "INSERT INTO track_row (recording_id, flags, title_key)
         SELECT id, CASE WHEN id IN (SELECT recording_id FROM library_track) THEN 1 ELSE 0 END, 'x'
         FROM recording;
         DELETE FROM search_due;",
    );
    drop(conn);
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    run(&mut conn, MIGRATIONS).unwrap();

    let waiting = overview_at(&conn, 1_800_000_000_000).unwrap();
    assert!(waiting.counting.index, "still counting, not a wrong number");
    assert!(waiting.genres.is_empty(), "nothing is stored yet");

    settle(&mut conn, &Mount, 500).unwrap();

    let counted = overview_at(&conn, 1_800_000_000_000).unwrap();
    assert!(!counted.counting.index);
    let genres: Vec<(&str, u32)> = counted
        .genres
        .iter()
        .map(|g| (g.name.as_str(), g.tracks))
        .collect();
    assert_eq!(
        genres,
        [("House", 2)],
        "two spellings are one genre, and a track outside the Library has none"
    );
    assert_eq!(counted.no_genre, 0);
}
