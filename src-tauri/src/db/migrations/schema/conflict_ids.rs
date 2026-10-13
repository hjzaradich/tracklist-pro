//! 0025: a `conflict` row id is never used twice, so undo can put a
//! deleted row back under its id although detecting conflicts inserts rows
//! outside the operation log (ROADMAP §2 `operation` / `change`, 1.10).
//! The table is rebuilt for it, so everything it had before is proven
//! again here: its columns, its checks, its indexes, its foreign key, the
//! rows it held, and 0014's guard against analysis fields.

use rusqlite::Connection;

use super::{accepts, db, db_at, insert, one, refuses};
use crate::db::migrations::{run, MIGRATIONS};

const SETUP: &str = "
    INSERT INTO recording (title) VALUES ('Track 1'), ('Track 2');
    INSERT INTO library_track (recording_id, source_status)
    VALUES (1, 'missing'), (2, 'missing');";

const ANALYSIS_FIELDS: [&str; 4] = ["AverageBpm", "Tonality", "TEMPO", "POSITION_MARK"];

/// How many migrations come before this one: the schema as it was.
fn before() -> usize {
    MIGRATIONS
        .iter()
        .position(|m| m.name == "0025_conflict_ids_never_reused.sql")
        .expect("the migration is in the list")
}

/// A database as it was before this migration, with two Library tracks.
fn as_it_was() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db_at(before());
    accepts(&conn, SETUP);
    (dir, conn)
}

/// A database with every migration, with two Library tracks.
fn as_it_is() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db();
    accepts(&conn, SETUP);
    (dir, conn)
}

fn migrate(conn: &mut Connection) {
    assert!(run(conn, MIGRATIONS).unwrap() >= 1);
    conn.pragma_update(None, "foreign_keys", true).unwrap();
}

fn conflict_sql(track: i64, field: &str, app: &str, rekordbox: &str, base: &str) -> String {
    format!(
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
         VALUES ({track}, '{field}', {app}, {rekordbox}, {base})"
    )
}

/// An open conflict on `field` of `track`; returns its id.
fn conflict(conn: &Connection, track: i64, field: &str) -> i64 {
    insert(conn, &conflict_sql(track, field, "'app'", "'rb'", "'base'"))
}

/// Every `conflict` row, every column, as text.
fn rows(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT concat_ws('|', id, library_track_id, field, quote(app_value),
                              quote(rekordbox_value), quote(base_value), status,
                              quote(resolved_value), detected_at, quote(resolved_at))
             FROM conflict ORDER BY id",
        )
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

/// A query's rows, each as its columns joined into one text.
fn listed(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    let columns = stmt.column_count();
    let rows = stmt
        .query_map([], |r| {
            let mut row = Vec::new();
            for i in 0..columns {
                row.push(format!("{:?}", r.get::<_, rusqlite::types::Value>(i)?));
            }
            Ok(row.join(" | "))
        })
        .unwrap();
    rows.map(Result::unwrap).collect()
}

// ---- Ids ----------------------------------------------------------------

#[test]
fn before_this_migration_a_new_conflict_took_the_id_of_the_last_deleted_one() {
    // The fault being fixed, shown on the schema as it was.
    let (_dir, conn) = as_it_was();
    let first = conflict(&conn, 1, "Rating");
    accepts(&conn, "DELETE FROM conflict");
    assert_eq!(conflict(&conn, 2, "Rating"), first);
}

#[test]
fn a_new_conflict_never_takes_the_id_of_a_deleted_one() {
    let (_dir, conn) = as_it_is();
    let first = conflict(&conn, 1, "Rating");
    let second = conflict(&conn, 1, "Name");
    accepts(&conn, "DELETE FROM conflict");
    let third = conflict(&conn, 2, "Rating");
    assert!(third > second && second > first, "{first} {second} {third}");
    // The deleted row can go back under its own id, as undo puts it back.
    accepts(
        &conn,
        &format!(
            "INSERT INTO conflict (id, library_track_id, field, app_value, rekordbox_value)
             VALUES ({second}, 1, 'Name', 'app', 'rb')"
        ),
    );
    assert!(conflict(&conn, 2, "Name") > third);
}

#[test]
fn conflicts_that_exist_when_the_migration_runs_keep_their_ids_and_every_value() {
    let (_dir, mut conn) = as_it_was();
    accepts(
        &conn,
        "INSERT INTO conflict (id, library_track_id, field, app_value, rekordbox_value,
                               base_value, status, resolved_value, detected_at, resolved_at)
         VALUES
             (3, 1, 'Rating', '204', '153', '0', 'open', NULL,
              '2026-02-03T04:05:06.000Z', NULL),
             (7, 1, 'Genre', 'Deep House', 'Tech House', NULL, 'kept_app', NULL,
              '2026-02-03T04:05:07.000Z', '2026-02-04T00:00:00.000Z'),
             (8, 2, 'Genre', 'Deep House', '', 'House', 'kept_rekordbox', NULL,
              '2026-02-03T04:05:08.000Z', '2026-02-04T00:00:01.000Z'),
             (12, 2, 'Comments', 'warm up', 'closing', 'intro', 'edited', 'warm up, closing',
              '2026-02-03T04:05:09.000Z', '2026-02-04T00:00:02.000Z');",
    );
    let before = rows(&conn);
    assert_eq!(before.len(), 4);

    migrate(&mut conn);

    assert_eq!(rows(&conn), before);
    // New rows carry on after the highest id that was there.
    assert_eq!(conflict(&conn, 2, "Rating"), 13);
}

/// Logs, as a removal does, that the conflict with this id was deleted.
fn log_a_deleted_conflict(conn: &Connection, id: i64) {
    accepts(
        conn,
        &format!(
            "INSERT INTO operation (kind) VALUES ('remove_from_library');
             INSERT INTO change (operation_id, entity, entity_id, action, field, before)
             VALUES (last_insert_rowid(), 'conflict', {id}, 'delete', 'field', 'Rating');"
        ),
    );
}

#[test]
fn a_conflict_deleted_before_the_migration_keeps_its_id_free_if_the_operation_log_names_it() {
    // A removal made before the upgrade deleted the highest ids. They're
    // gone from the table, but the log still holds them for an undo.
    let (_dir, mut conn) = as_it_was();
    conflict(&conn, 1, "Rating");
    let deleted = [conflict(&conn, 1, "Name"), conflict(&conn, 1, "Artist")];
    for id in deleted {
        accepts(&conn, &format!("DELETE FROM conflict WHERE id = {id}"));
        log_a_deleted_conflict(&conn, id);
    }

    migrate(&mut conn);

    assert!(conflict(&conn, 2, "Rating") > deleted[1]);
}

#[test]
fn a_logged_conflict_id_stays_free_even_when_no_conflict_is_left_at_the_migration() {
    let (_dir, mut conn) = as_it_was();
    let only = conflict(&conn, 1, "Rating");
    accepts(&conn, "DELETE FROM conflict");
    log_a_deleted_conflict(&conn, only + 4);

    migrate(&mut conn);

    assert_eq!(conflict(&conn, 2, "Rating"), only + 5);
}

#[test]
fn with_no_conflicts_and_none_in_the_log_the_first_conflict_after_the_migration_is_number_one() {
    let (_dir, mut conn) = as_it_was();
    // Other tables' ids in the log don't count.
    accepts(
        &conn,
        "INSERT INTO operation (kind) VALUES ('create_crate');
         INSERT INTO change (operation_id, entity, entity_id, action, field, after)
         VALUES (1, 'crate', 900, 'insert', 'name', 'x');",
    );
    migrate(&mut conn);
    assert_eq!(conflict(&conn, 1, "Rating"), 1);
}

#[test]
fn sync_base_ids_are_still_never_reused_after_the_migration() {
    // The two tables share SQLite's `sqlite_sequence`; rebuilding one
    // mustn't disturb the other's count.
    let (_dir, mut conn) = as_it_was();
    let base = |conn: &Connection, field: &str| {
        insert(
            conn,
            &format!(
                "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, '{field}', 'x')"
            ),
        )
    };
    let first = base(&conn, "Rating");
    let second = base(&conn, "Name");
    accepts(&conn, &format!("DELETE FROM sync_base WHERE id = {second}"));

    migrate(&mut conn);

    assert!(base(&conn, "Genre") > second && second > first);
}

// ---- The table is what it was -------------------------------------------

#[test]
fn the_rebuilt_table_has_the_columns_indexes_and_triggers_it_had() {
    let columns = "SELECT name, type, \"notnull\", dflt_value, pk
                   FROM pragma_table_xinfo('conflict') ORDER BY cid";
    // Each index and trigger, exactly as it was declared.
    let attached = "SELECT type, name, sql FROM sqlite_master
                    WHERE tbl_name = 'conflict' AND type IN ('index', 'trigger')
                    ORDER BY type, name";
    let foreign_keys = "SELECT \"table\", \"from\", \"to\", on_update, on_delete
                        FROM pragma_foreign_key_list('conflict')";
    let (_old_dir, old) = as_it_was();
    let (_new_dir, new) = as_it_is();
    for sql in [columns, attached, foreign_keys] {
        let was = listed(&old, sql);
        assert!(!was.is_empty(), "{sql}");
        assert_eq!(listed(&new, sql), was, "{sql}");
    }
    let names: Vec<String> = listed(&new, attached);
    for name in [
        "conflict_by_library_track",
        "conflict_one_open_per_field",
        "conflict_never_on_an_analysis_field_insert",
        "conflict_never_on_an_analysis_field_update",
    ] {
        assert!(names.iter().any(|row| row.contains(name)), "{name} is gone");
    }
    // Nothing of the rebuild is left behind.
    assert_eq!(
        one::<i64>(
            &new,
            "SELECT count(*) FROM sqlite_master WHERE name LIKE 'conflict_new%'"
        ),
        0
    );
    let strict: bool = one(
        &new,
        "SELECT strict FROM pragma_table_list WHERE name = 'conflict'",
    );
    assert!(strict);
}

#[test]
fn after_the_migration_a_conflict_is_still_only_a_field_both_sides_changed_to_different_values() {
    let (_dir, conn) = as_it_is();
    let on = |app, rekordbox, base| conflict_sql(1, "Genre", app, rekordbox, base);
    // Only one side changed: that's a send or a pull, not a conflict.
    refuses(&conn, &on("'base'", "'rb'", "'base'"), "CHECK");
    refuses(&conn, &on("'app'", "'base'", "'base'"), "CHECK");
    // Both changed to the same value.
    refuses(&conn, &on("'same'", "'same'", "'base'"), "CHECK");
    refuses(&conn, &on("NULL", "NULL", "'base'"), "CHECK");
    // A field never sent has no base; the two sides must still differ.
    refuses(&conn, &on("'same'", "'same'", "NULL"), "CHECK");
    refuses(&conn, &on("'app'", "NULL", "NULL"), "CHECK");
    accepts(&conn, &on("'app'", "'rb'", "NULL"));
    accepts(&conn, "DELETE FROM conflict");
    accepts(&conn, &on("'app'", "'rb'", "'base'"));
}

#[test]
fn after_the_migration_there_is_still_one_open_conflict_per_track_and_field() {
    let (_dir, conn) = as_it_is();
    conflict(&conn, 1, "Genre");
    refuses(
        &conn,
        &conflict_sql(1, "Genre", "'d'", "'e'", "'c'"),
        "UNIQUE",
    );
    // Another field, and another track, are other conflicts.
    conflict(&conn, 1, "Rating");
    conflict(&conn, 2, "Genre");
    // Once settled, the field can conflict again; settled ones pile up.
    for _ in 0..2 {
        accepts(
            &conn,
            "UPDATE conflict SET status = 'kept_app', resolved_at = '2026-09-28T00:00:00Z'
             WHERE library_track_id = 1 AND field = 'Genre' AND status = 'open'",
        );
        conflict(&conn, 1, "Genre");
    }
    assert_eq!(
        one::<i64>(
            &conn,
            "SELECT count(*) FROM conflict WHERE library_track_id = 1 AND field = 'Genre'"
        ),
        3
    );
}

#[test]
fn after_the_migration_a_conflict_is_still_open_until_settled_and_settling_records_when_and_how() {
    let (_dir, conn) = as_it_is();
    conflict(&conn, 1, "Genre");
    assert_eq!(one::<String>(&conn, "SELECT status FROM conflict"), "open");
    assert!(!one::<String>(&conn, "SELECT detected_at FROM conflict").is_empty());
    // Settled needs a time; open has none.
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
    // An edit needs its value, and only an edit has one.
    refuses(
        &conn,
        "UPDATE conflict SET status = 'edited', resolved_at = '2026-09-28T00:00:00Z'",
        "CHECK",
    );
    refuses(
        &conn,
        "UPDATE conflict SET status = 'kept_app', resolved_value = 'd',
                             resolved_at = '2026-09-28T00:00:00Z'",
        "CHECK",
    );
    refuses(
        &conn,
        "UPDATE conflict SET status = 'merged', resolved_at = '2026-09-28T00:00:00Z'",
        "CHECK",
    );
    accepts(
        &conn,
        "UPDATE conflict SET status = 'edited', resolved_value = 'd',
                             resolved_at = '2026-09-28T00:00:00Z'",
    );
}

#[test]
fn after_the_migration_a_conflict_still_needs_its_track_and_a_field_and_holds_the_track_in_place() {
    let (_dir, conn) = as_it_is();
    refuses(
        &conn,
        &conflict_sql(99, "Genre", "'a'", "'b'", "'c'"),
        "FOREIGN KEY",
    );
    refuses(&conn, &conflict_sql(1, "", "'a'", "'b'", "'c'"), "CHECK");
    refuses(
        &conn,
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value)
         VALUES (1, NULL, 'a', 'b')",
        "NOT NULL",
    );
    conflict(&conn, 1, "Genre");
    refuses(
        &conn,
        "DELETE FROM library_track WHERE id = 1",
        "FOREIGN KEY",
    );
}

// ---- 0014's guard -------------------------------------------------------

#[test]
fn after_the_migration_the_database_still_refuses_a_conflict_on_an_analysis_field() {
    let (_dir, conn) = as_it_is();
    for field in ANALYSIS_FIELDS {
        refuses(
            &conn,
            &conflict_sql(1, field, "'app'", "'rb'", "'base'"),
            "analysis fields never become conflicts",
        );
    }
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM conflict"), 0);
}

#[test]
fn after_the_migration_a_conflict_still_cannot_be_replaced_by_or_moved_onto_an_analysis_field() {
    let (_dir, conn) = as_it_is();
    let id = conflict(&conn, 1, "Rating");
    for field in ANALYSIS_FIELDS {
        refuses(
            &conn,
            &format!(
                "INSERT OR REPLACE INTO conflict
                     (id, library_track_id, field, app_value, rekordbox_value, base_value)
                 VALUES ({id}, 1, '{field}', 'app', 'rb', 'base')"
            ),
            "analysis fields never become conflicts",
        );
        refuses(
            &conn,
            &format!("UPDATE conflict SET field = '{field}'"),
            "analysis fields never become conflicts",
        );
    }
    assert_eq!(one::<String>(&conn, "SELECT field FROM conflict"), "Rating");
    // Names that only look like analysis fields are ordinary fields.
    for field in ["Bpm", "tonality", "Tempo", "AverageBpm ", "Key"] {
        accepts(&conn, &conflict_sql(2, field, "'app'", "'rb'", "'base'"));
    }
}

#[test]
fn a_conflict_on_an_analysis_field_never_gets_through_the_migration() {
    // 0014 emptied the table of them and has refused them since, so the
    // rebuild has none to carry over; the rows it does carry are the ones
    // that were there.
    let (_dir, mut conn) = as_it_was();
    for field in ANALYSIS_FIELDS {
        refuses(
            &conn,
            &conflict_sql(1, field, "'app'", "'rb'", "'base'"),
            "analysis fields never become conflicts",
        );
    }
    conflict(&conn, 1, "Rating");
    migrate(&mut conn);
    assert_eq!(
        listed(&conn, "SELECT field FROM conflict"),
        ["Text(\"Rating\")"]
    );
}
