//! 0028: `relink` as a rowid table with a verdict, so the operation log can
//! record a confirmation and a "not the same" (1bE-6; ROADMAP 1.2, §2).

use rusqlite::Connection;

use super::{accepts, db, db_at, insert, one, refuses};
use crate::db::migrations::{run, MIGRATIONS};

const SETUP: &str = "
    INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key)
        VALUES (1, 'a.mp3', 'a.mp3'), (1, 'b.mp3', 'b.mp3'), (1, 'c.mp3', 'c.mp3');";

/// Every `relink` row without its id, as text, by Location.
fn rows(conn: &Connection, columns: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {columns} FROM relink ORDER BY location_key, file_id"
        ))
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

const AS_TEXT: &str = "location_key || '|' || file_id || '|' || method || '|'
    || ifnull(confidence, 'none') || '|' || ifnull(hex(audio_hash), 'none') || '|' || confirmed_at";

#[test]
fn the_migration_keeps_every_confirmation_as_it_was() {
    let (_dir, mut conn) = db_at(27);
    accepts(&conn, SETUP);
    accepts(
        &conn,
        "INSERT INTO relink (location_key, file_id, method, confidence, audio_hash, confirmed_at)
         VALUES ('e:/old/zeta.mp3', 2, 'filename_only', 0.5, x'0A0B', '2026-09-30T10:00:00.000Z'),
                ('e:/old/alpha.mp3', 1, 'user', NULL, NULL, '2026-10-01T11:00:00.000Z'),
                ('e:/old/mid.mp3', 3, 'fingerprint', 0.4, x'', '2026-10-02T12:00:00.000Z');",
    );
    let before = rows(&conn, AS_TEXT);
    assert_eq!(before.len(), 3);

    run(&mut conn, &MIGRATIONS[..28]).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    assert_eq!(rows(&conn, AS_TEXT), before);
    // Every row is a confirmation, and has an id of its own.
    let same: i64 = one(&conn, "SELECT count(*) FROM relink WHERE verdict = 'same'");
    let ids: i64 = one(&conn, "SELECT count(DISTINCT id) FROM relink");
    assert_eq!((same, ids), (3, 3));
    // A confirmation is still found by its Location, one per Location.
    refuses(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('e:/old/alpha.mp3', 2, 'user')",
        "UNIQUE",
    );
    let broken: i64 = one(&conn, "SELECT count(*) FROM pragma_foreign_key_check");
    assert_eq!(broken, 0);
}

#[test]
fn the_migration_applies_to_a_database_with_no_confirmations() {
    let (_dir, mut conn) = db_at(27);
    accepts(&conn, SETUP);
    run(&mut conn, &MIGRATIONS[..28]).unwrap();
    let rows: i64 = one(&conn, "SELECT count(*) FROM relink");
    assert_eq!(rows, 0);
}

#[test]
fn the_operation_log_can_name_a_relink_row_by_its_id() {
    // A STRICT table with a rowid: what `ops` records changes to.
    let (_dir, conn) = db();
    let (without_rowid, strict): (bool, bool) = conn
        .query_row(
            "SELECT wr, strict FROM pragma_table_list WHERE schema = 'main' AND name = 'relink'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(!without_rowid && strict);
}

#[test]
fn a_relink_row_never_takes_the_id_of_a_deleted_one() {
    // The relink job writes the table outside the log, so an Undo must
    // always find the id of the row it puts back free.
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    let first = insert(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('e:/a.mp3', 1, 'user')",
    );
    let second = insert(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('e:/b.mp3', 2, 'user')",
    );
    accepts(&conn, "DELETE FROM relink");
    let third = insert(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('e:/c.mp3', 3, 'user')",
    );
    assert!(third > second && second > first, "{first} {second} {third}");
    accepts(
        &conn,
        &format!(
            "INSERT INTO relink (id, location_key, file_id, method)
             VALUES ({second}, 'e:/b.mp3', 2, 'user')"
        ),
    );
}

#[test]
fn a_location_has_one_confirmation_and_says_not_the_same_once_per_file() {
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    accepts(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('e:/a.mp3', 1, 'user');
         INSERT INTO relink (location_key, file_id, verdict, method)
             VALUES ('e:/a.mp3', 2, 'not_same', 'filename_only'),
                    ('e:/a.mp3', 3, 'not_same', 'fingerprint'),
                    ('e:/other.mp3', 2, 'not_same', 'filename_only');",
    );
    refuses(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('e:/a.mp3', 3, 'user')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO relink (location_key, file_id, verdict, method)
         VALUES ('e:/a.mp3', 2, 'not_same', 'unique_duration')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO relink (location_key, file_id, verdict, method)
         VALUES ('e:/b.mp3', 1, 'maybe', 'user')",
        "CHECK",
    );
}

#[test]
fn a_file_named_by_a_confirmation_or_a_not_the_same_cannot_be_deleted_under_it() {
    // So the file's id is never another file's while a row names it.
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    accepts(
        &conn,
        "INSERT INTO relink (location_key, file_id, method) VALUES ('e:/a.mp3', 1, 'user');
         INSERT INTO relink (location_key, file_id, verdict, method)
             VALUES ('e:/b.mp3', 2, 'not_same', 'filename_only');",
    );
    refuses(&conn, "DELETE FROM file WHERE id = 1", "FOREIGN KEY");
    refuses(&conn, "DELETE FROM file WHERE id = 2", "FOREIGN KEY");
    accepts(&conn, "DELETE FROM file WHERE id = 3");
}
