//! The search index migration (1cA-6): the FTS5 table and the list of
//! tracks due, and the triggers that note which track a change touches.

use super::super::{run, MIGRATIONS};
use super::{accepts, db, db_at, one, refuses};

/// How many migrations come before the search index's.
fn before() -> usize {
    MIGRATIONS
        .iter()
        .position(|m| m.name.ends_with("_search_index.sql"))
        .expect("the search index migration")
}

/// The tracks due, sorted.
fn due(conn: &rusqlite::Connection) -> Vec<i64> {
    conn.prepare("SELECT recording_id FROM search_due ORDER BY 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn nothing_due(conn: &rusqlite::Connection) {
    accepts(conn, "DELETE FROM search_due");
}

/// Two tracks with a file each, a Library track for the first, and a
/// rekordbox row: the database the triggers are tried on.
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
         INSERT INTO rekordbox_track (attributes, location_key, read_at)
             VALUES ('{\"TrackID\":\"7\",\"Location\":\"file://localhost/E:/Music/a.mp3\"}',
                     'E:/MUSIC/A.MP3', '2026-09-30T10:00:00.000Z');",
    );
    nothing_due(&conn);
    (dir, conn)
}

#[test]
fn every_track_is_due_when_the_index_arrives_on_a_database_that_has_tracks() {
    let (dir, conn) = db_at(before());
    accepts(
        &conn,
        "INSERT INTO recording DEFAULT VALUES; INSERT INTO recording DEFAULT VALUES;
         INSERT INTO recording DEFAULT VALUES;",
    );
    drop(conn);
    let mut conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
    run(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(due(&conn), vec![1, 2, 3]);
    let rows: i64 = one(&conn, "SELECT count(*) FROM search_track");
    assert_eq!(rows, 0, "the rows are the job's to write");
}

#[test]
fn the_list_of_tracks_due_holds_whole_numbers_once_each() {
    let (_dir, conn) = db();
    accepts(&conn, "INSERT INTO search_due VALUES (5)");
    refuses(&conn, "INSERT INTO search_due VALUES (5)", "UNIQUE");
    refuses(&conn, "INSERT INTO search_due VALUES ('five')", "INTEGER");
    refuses(&conn, "INSERT INTO search_due VALUES (NULL)", "NULL");
}

#[test]
fn the_index_is_full_text_with_the_eight_fields_and_keeps_no_text() {
    let (_dir, conn) = db();
    let sql: String = one(
        &conn,
        "SELECT sql FROM sqlite_master WHERE name = 'search_track'",
    );
    assert!(sql.contains("USING fts5"), "{sql}");
    assert!(sql.contains("unicode61 remove_diacritics 2"), "{sql}");
    assert!(sql.contains("content = ''"), "{sql}");
    let columns = super::columns(&conn, "search_track");
    assert_eq!(
        columns[..8],
        [
            "title",
            "artist",
            "album",
            "remixer",
            "label",
            "genre",
            "comment",
            "file_name"
        ]
    );
    accepts(
        &conn,
        "INSERT INTO search_track (rowid, title, file_name) VALUES (1, 'Caf\u{e9} del Mar', 'x');",
    );
    let found: i64 = one(
        &conn,
        "SELECT count(*) FROM search_track WHERE search_track MATCH '\"CAFE\"*'",
    );
    assert_eq!(found, 1);
    // A row can be deleted (it is replaced when its track changes).
    accepts(&conn, "DELETE FROM search_track WHERE rowid = 1");
    let found: i64 = one(
        &conn,
        "SELECT count(*) FROM search_track WHERE search_track MATCH '\"CAFE\"*'",
    );
    assert_eq!(found, 0);
}

#[test]
fn each_change_that_can_alter_a_tracks_row_notes_that_track_as_due() {
    for (change, expected) in [
        // A file's tags, name, or presence.
        ("UPDATE file SET raw_tags = '{}' WHERE id = 1", vec![1]),
        ("UPDATE file SET rel_path = 'c.mp3', rel_path_key = 'c.mp3' WHERE id = 2", vec![2]),
        ("UPDATE file SET present = 0 WHERE id = 1", vec![1]),
        // A file joining, leaving, moving or changing role.
        ("INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'n.mp3', 'n.mp3');
          INSERT INTO recording_file (recording_id, file_id) VALUES (2, 3)", vec![2]),
        ("DELETE FROM recording_file WHERE file_id = 1", vec![1]),
        ("UPDATE recording_file SET recording_id = 2, role = 'extra' WHERE file_id = 1", vec![1, 2]),
        ("UPDATE recording_file SET role = 'extra' WHERE file_id = 2", vec![2]),
        // An upsert has a conflict policy of its own, which a trigger's
        // INSERT OR IGNORE would have to give way to.
        ("INSERT INTO recording_file (recording_id, file_id, role) VALUES (2, 1, 'extra')
          ON CONFLICT (file_id) DO UPDATE SET recording_id = excluded.recording_id,
              role = excluded.role", vec![1, 2]),
        // A track appearing, being named, going.
        ("INSERT INTO recording DEFAULT VALUES", vec![3]),
        ("UPDATE recording SET title = 'T' WHERE id = 2", vec![2]),
        ("UPDATE recording SET artist = 'A' WHERE id = 2", vec![2]),
        ("DELETE FROM recording_file WHERE recording_id = 2; DELETE FROM recording WHERE id = 2", vec![2]),
        // The Library.
        ("INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
          VALUES (2, 'linked', 2, 'ok')", vec![2]),
        ("UPDATE library_track SET linked_file_id = 2 WHERE id = 1", vec![1]),
        ("DELETE FROM library_track WHERE id = 1", vec![1]),
        // rekordbox: every Library track.
        ("INSERT INTO rekordbox_track (attributes, location_key, read_at)
          VALUES ('{\"TrackID\":\"8\",\"Location\":\"file://localhost/E:/Music/b.mp3\"}',
                  'E:/MUSIC/B.MP3', '2026-09-30T10:00:00.000Z')", vec![-1]),
        ("DELETE FROM rekordbox_track", vec![-1]),
        ("UPDATE rekordbox_track SET file_id = 1, relink_method = 'path'", vec![-1]),
    ] {
        let (_dir, conn) = with_tracks();
        accepts(&conn, change);
        assert_eq!(due(&conn), expected, "{change}");
    }
}

#[test]
fn a_change_that_leaves_the_text_alone_notes_nothing() {
    let (_dir, conn) = with_tracks();
    for change in [
        "UPDATE file SET size = 10, mtime = 5, blake3 = NULL, quality_verdict = 'ok' WHERE id = 1",
        "UPDATE file SET hidden_reason = 'user' WHERE id = 1",
        "UPDATE file SET raw_tags = NULL WHERE id = 1",
        "UPDATE file SET present = 1 WHERE id = 1",
        "UPDATE recording SET title = NULL WHERE id = 1",
        "UPDATE library_track SET added_at = added_at WHERE id = 1",
        "UPDATE recording_file SET match_confidence = 0.5 WHERE file_id = 1",
        "UPDATE rekordbox_track SET relink_probable = 0",
    ] {
        accepts(&conn, change);
        assert_eq!(due(&conn), Vec::<i64>::new(), "{change}");
    }
}
