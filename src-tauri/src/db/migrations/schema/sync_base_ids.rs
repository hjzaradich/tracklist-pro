//! 0018: a `sync_base` row id is never used twice, so undo can put a
//! deleted row back under its id (ROADMAP §2 `operation` / `change`).

use rusqlite::Connection;

use super::{accepts, db, db_at, insert, one, refuses};
use crate::db::migrations::{run, MIGRATIONS};

const SETUP: &str = "
    INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'DJ Music', 'DJ Music');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
    INSERT INTO recording (title) VALUES ('Track 1'), ('Track 2');
    INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 1), (2, 1);";

/// Every `sync_base` row, id included, as text.
fn rows(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT id || '|' || library_track_id || '|' || field || '|' || value || '|' || synced_at
             FROM sync_base ORDER BY id",
        )
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn base(conn: &Connection, track: i64, field: &str) -> i64 {
    insert(
        conn,
        &format!(
            "INSERT INTO sync_base (library_track_id, field, value) VALUES ({track}, '{field}', 'x')"
        ),
    )
}

#[test]
fn before_this_migration_a_new_sync_base_took_the_id_of_the_last_deleted_one() {
    // The fault being fixed, shown on the schema as it was.
    let (_dir, conn) = db_at(17);
    accepts(&conn, SETUP);
    let first = base(&conn, 1, "Rating");
    accepts(&conn, "DELETE FROM sync_base");
    assert_eq!(base(&conn, 2, "Rating"), first);
}

#[test]
fn a_new_sync_base_never_takes_the_id_of_a_deleted_one() {
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    let first = base(&conn, 1, "Rating");
    let second = base(&conn, 1, "Name");
    accepts(&conn, "DELETE FROM sync_base");
    let third = base(&conn, 2, "Rating");
    assert!(third > second && second > first, "{first} {second} {third}");
    // The deleted row can go back under its own id.
    accepts(
        &conn,
        &format!(
            "INSERT INTO sync_base (id, library_track_id, field, value)
             VALUES ({second}, 1, 'Name', 'x')"
        ),
    );
    assert!(base(&conn, 2, "Name") > third);
}

#[test]
fn sync_bases_that_exist_when_the_migration_runs_keep_their_ids_and_values() {
    let (_dir, mut conn) = db_at(17);
    accepts(&conn, SETUP);
    accepts(
        &conn,
        "INSERT INTO sync_base (id, library_track_id, field, value, synced_at) VALUES
             (3, 1, 'Rating', '204', '2026-02-03T04:05:06.000Z'),
             (7, 1, 'Name', 'x', '2026-02-03T04:05:07.000Z'),
             (8, 2, 'Rating', '', '2026-02-03T04:05:08.000Z');",
    );
    let before = rows(&conn);
    assert_eq!(before.len(), 3);

    assert_eq!(run(&mut conn, MIGRATIONS).unwrap(), MIGRATIONS.len() - 17);
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    assert_eq!(rows(&conn), before);
    // New rows carry on after the highest id that was there.
    assert_eq!(base(&conn, 2, "Name"), 9);
}

#[test]
fn after_the_migration_a_sync_base_is_still_one_per_track_and_field_and_needs_its_track() {
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    base(&conn, 1, "Rating");
    refuses(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Rating', '1')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (99, 'Rating', '1')",
        "FOREIGN KEY",
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
    refuses(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Name', NULL)",
        "NOT NULL",
    );
}

#[test]
fn after_the_migration_a_sends_upsert_on_track_and_field_keeps_the_row_and_its_id() {
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    let id = base(&conn, 1, "Rating");
    accepts(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Rating', '2')
         ON CONFLICT (library_track_id, field) DO UPDATE SET value = excluded.value",
    );
    let (rows, kept, value): (i64, i64, String) = (
        one(&conn, "SELECT count(*) FROM sync_base"),
        one(&conn, "SELECT id FROM sync_base"),
        one(&conn, "SELECT value FROM sync_base"),
    );
    assert_eq!((rows, kept, value.as_str()), (1, id, "2"));
}

/// Logs, as a removal does, that the base with this id was deleted.
fn log_a_deleted_base(conn: &Connection, id: i64) {
    accepts(
        conn,
        &format!(
            "INSERT INTO operation (kind) VALUES ('remove_from_library');
             INSERT INTO change (operation_id, entity, entity_id, action, field, before)
             VALUES (last_insert_rowid(), 'sync_base', {id}, 'delete', 'value', 'x');"
        ),
    );
}

#[test]
fn a_base_deleted_before_the_migration_keeps_its_id_free_if_the_operation_log_names_it() {
    // A removal made before the upgrade deleted the highest ids. They're
    // gone from the table, but the log still holds them for an undo.
    let (_dir, mut conn) = db_at(17);
    accepts(&conn, SETUP);
    base(&conn, 1, "Rating");
    let deleted = [base(&conn, 1, "Name"), base(&conn, 1, "Artist")];
    for id in deleted {
        accepts(&conn, &format!("DELETE FROM sync_base WHERE id = {id}"));
        log_a_deleted_base(&conn, id);
    }

    run(&mut conn, MIGRATIONS).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    assert!(base(&conn, 2, "Rating") > deleted[1]);
}

#[test]
fn a_logged_base_id_stays_free_even_when_no_base_is_left_at_the_migration() {
    let (_dir, mut conn) = db_at(17);
    accepts(&conn, SETUP);
    let only = base(&conn, 1, "Rating");
    accepts(&conn, "DELETE FROM sync_base");
    log_a_deleted_base(&conn, only + 4);

    run(&mut conn, MIGRATIONS).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    assert_eq!(base(&conn, 2, "Rating"), only + 5);
}

#[test]
fn with_no_bases_and_none_in_the_log_the_first_base_after_the_migration_is_number_one() {
    let (_dir, mut conn) = db_at(17);
    accepts(&conn, SETUP);
    // Other tables' ids in the log don't count.
    accepts(
        &conn,
        "INSERT INTO operation (kind) VALUES ('create_crate');
         INSERT INTO change (operation_id, entity, entity_id, action, field, after)
         VALUES (1, 'crate', 900, 'insert', 'name', 'x');",
    );
    run(&mut conn, MIGRATIONS).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    assert_eq!(base(&conn, 1, "Rating"), 1);
}
