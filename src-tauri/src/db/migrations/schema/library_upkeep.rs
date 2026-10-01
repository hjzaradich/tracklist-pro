//! 0013: `library_track.source_status` follows the file, `library_removal`,
//! and `crate_entry` / `sync_base` as rowid tables (ROADMAP 1.3, 1.9 rule 7,
//! §2).

use rusqlite::Connection;

use super::{accepts, db, db_at, one, refuses};
use crate::db::migrations::{run, MIGRATIONS};

const SETUP: &str = "
    INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'DJ Music', 'DJ Music');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, present) VALUES (1, 'b.mp3', 'b.mp3', 0);
    INSERT INTO recording (title) VALUES ('Track 1'), ('Track 2');
    INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 1);";

/// A library with files 1–2 (2 is missing), tracks 1–2, and Library track 1
/// on file 1.
fn with_a_library_track() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db();
    accepts(&conn, SETUP);
    (dir, conn)
}

fn status(conn: &Connection) -> String {
    one(conn, "SELECT source_status FROM library_track WHERE id = 1")
}

// The flag follows the file.

#[test]
fn a_linked_tracks_status_follows_its_files_presence() {
    let (_dir, conn) = with_a_library_track();
    assert_eq!(status(&conn), "ok");
    accepts(&conn, "UPDATE file SET present = 0 WHERE id = 1");
    assert_eq!(status(&conn), "missing");
    accepts(&conn, "UPDATE file SET present = 1 WHERE id = 1");
    assert_eq!(status(&conn), "ok");
}

#[test]
fn another_files_presence_does_not_change_the_status() {
    let (_dir, conn) = with_a_library_track();
    accepts(&conn, "UPDATE file SET present = 1 WHERE id = 2");
    accepts(&conn, "UPDATE file SET present = 0 WHERE id = 2");
    assert_eq!(status(&conn), "ok");
}

#[test]
fn a_track_linked_to_a_missing_file_starts_missing_and_takes_the_next_files_state() {
    let (_dir, conn) = with_a_library_track();
    accepts(
        &conn,
        "UPDATE library_track SET linked_file_id = 2 WHERE id = 1",
    );
    assert_eq!(status(&conn), "missing");
    accepts(
        &conn,
        "UPDATE library_track SET linked_file_id = 1 WHERE id = 1",
    );
    assert_eq!(status(&conn), "ok");
    accepts(
        &conn,
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES (2, 2)",
    );
    let second: String = one(
        &conn,
        "SELECT source_status FROM library_track WHERE id = 2",
    );
    assert_eq!(second, "missing");
}

#[test]
fn rows_that_exist_when_the_migration_runs_are_brought_up_to_date() {
    let (_dir, mut conn) = db_at(12);
    accepts(&conn, SETUP);
    accepts(&conn, "UPDATE file SET present = 0 WHERE id = 1");
    assert_eq!(status(&conn), "ok");
    run(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(status(&conn), "missing");
}

// The record of removed tracks.

#[test]
fn a_track_has_one_removal_record_and_it_blocks_deleting_the_track() {
    let (_dir, conn) = with_a_library_track();
    accepts(
        &conn,
        "INSERT INTO library_removal (recording_id) VALUES (2)",
    );
    refuses(
        &conn,
        "INSERT INTO library_removal (recording_id) VALUES (2)",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO library_removal (recording_id) VALUES (99)",
        "FOREIGN KEY",
    );
    refuses(&conn, "DELETE FROM recording WHERE id = 2", "FOREIGN KEY");
    let at: String = one(&conn, "SELECT removed_at FROM library_removal");
    assert!(at.ends_with('Z'), "{at}");
}

// crate_entry and sync_base as rowid tables.

/// Both tables' rows, without their row ids, as text.
fn dump(conn: &Connection) -> (Vec<String>, Vec<String>) {
    let rows = |sql: &str| -> Vec<String> {
        let mut stmt = conn.prepare(sql).unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(Result::unwrap).collect()
    };
    (
        rows(
            "SELECT crate_id || '|' || library_track_id || '|' || kind || '|' || added_at
             FROM crate_entry ORDER BY crate_id, library_track_id",
        ),
        rows(
            "SELECT library_track_id || '|' || field || '|' || value || '|' || synced_at
             FROM sync_base ORDER BY library_track_id, field",
        ),
    )
}

#[test]
fn crate_entries_and_sync_bases_that_exist_when_the_migration_runs_survive_unchanged() {
    let (_dir, mut conn) = db_at(12);
    accepts(&conn, SETUP);
    accepts(
        &conn,
        "INSERT INTO crate (kind, name) VALUES ('static', 'One');
         INSERT INTO crate (kind, name, rules) VALUES ('smart', 'Two', '{}');
         INSERT INTO crate_entry (crate_id, library_track_id, kind, added_at)
             VALUES (2, 1, 'always_include', '2026-01-02T03:04:05.000Z');
         INSERT INTO crate_entry (crate_id, library_track_id, added_at)
             VALUES (1, 1, '2026-01-03T03:04:05.000Z');
         INSERT INTO sync_base (library_track_id, field, value, synced_at)
             VALUES (1, 'Rating', '204', '2026-02-03T04:05:06.000Z');
         INSERT INTO sync_base (library_track_id, field, value, synced_at)
             VALUES (1, 'Name', 'x', '2026-02-03T04:05:07.000Z');",
    );
    let before = dump(&conn);
    assert_eq!(before.0.len(), 2);
    assert_eq!(before.1.len(), 2);

    run(&mut conn, MIGRATIONS).unwrap();

    assert_eq!(dump(&conn), before);
    // The rows now have row ids, and the index is back.
    let entries: i64 = one(&conn, "SELECT count(DISTINCT id) FROM crate_entry");
    let bases: i64 = one(&conn, "SELECT count(DISTINCT id) FROM sync_base");
    assert_eq!((entries, bases), (2, 2));
    let index: i64 = one(
        &conn,
        "SELECT count(*) FROM sqlite_master WHERE name = 'crate_entry_by_library_track'",
    );
    assert_eq!(index, 1);
}

#[test]
fn a_sync_base_upsert_on_its_key_columns_still_works() {
    let (_dir, conn) = with_a_library_track();
    for value in ["1", "2"] {
        accepts(
            &conn,
            &format!(
                "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Rating', '{value}')
                 ON CONFLICT (library_track_id, field) DO UPDATE SET value = excluded.value"
            ),
        );
    }
    let rows: i64 = one(&conn, "SELECT count(*) FROM sync_base");
    let value: String = one(&conn, "SELECT value FROM sync_base");
    assert_eq!((rows, value.as_str()), (1, "2"));
}

#[test]
fn a_crate_entry_upsert_on_its_key_columns_still_works() {
    let (_dir, conn) = with_a_library_track();
    accepts(
        &conn,
        "INSERT INTO crate (kind, name, rules) VALUES ('smart', 'One', '{}')",
    );
    for kind in ["always_include", "never_include"] {
        accepts(
            &conn,
            &format!(
                "INSERT INTO crate_entry (crate_id, library_track_id, kind) VALUES (1, 1, '{kind}')
                 ON CONFLICT (crate_id, library_track_id) DO UPDATE SET kind = excluded.kind"
            ),
        );
    }
    let rows: i64 = one(&conn, "SELECT count(*) FROM crate_entry");
    let kind: String = one(&conn, "SELECT kind FROM crate_entry");
    assert_eq!((rows, kind.as_str()), (1, "never_include"));
}

#[test]
fn crate_entries_and_sync_bases_are_still_refused_when_they_repeat_or_dangle() {
    let (_dir, conn) = with_a_library_track();
    accepts(
        &conn,
        "INSERT INTO crate (kind, name) VALUES ('static', 'One');
         INSERT INTO crate_entry (crate_id, library_track_id) VALUES (1, 1);
         INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Rating', '0');",
    );
    refuses(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (1, 1)",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Rating', '1')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "DELETE FROM library_track WHERE id = 1",
        "FOREIGN KEY",
    );
    refuses(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, '', '0')",
        "CHECK",
    );
}
