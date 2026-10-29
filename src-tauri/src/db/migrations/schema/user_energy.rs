//! 0039: an `analysis` row id is never used twice, so the user's energy can
//! go through the operation log (ROADMAP §2 `analysis`; 1cB-8).

use rusqlite::Connection;

use super::{accepts, db, db_at, insert, one, refuses, run, MIGRATIONS};
use crate::db::migrations::Migration;

/// How many migrations come before 0039: its index in [`MIGRATIONS`].
fn before() -> usize {
    MIGRATIONS.iter().position(|m| m.version() == 39).unwrap()
}

const SETUP: &str = "
    INSERT INTO recording (title) VALUES ('Track 1'), ('Track 2');
    DELETE FROM search_due;";

/// Every `analysis` row, id included, as text.
fn rows(conn: &Connection) -> String {
    one(
        conn,
        "SELECT ifnull(group_concat(line, ' '), '') FROM (
             SELECT id || ':' || recording_id || ':' || source || ':' || ifnull(bpm, '-') || ':'
                    || ifnull(key, '-') || ':' || ifnull(energy, '-') || ':'
                    || ifnull(confidence, '-') || ':' || analyzed_at AS line
             FROM analysis ORDER BY id)",
    )
}

fn analysis(conn: &Connection, recording: i64, source: &str, energy: i64) -> i64 {
    insert(
        conn,
        &format!(
            "INSERT INTO analysis (recording_id, source, energy) VALUES ({recording}, '{source}', {energy})"
        ),
    )
}

fn due(conn: &Connection) -> i64 {
    one(conn, "SELECT count(*) FROM search_due")
}

#[test]
fn before_this_migration_a_new_analysis_row_took_the_id_of_the_last_deleted_one() {
    // The fault being fixed, shown on the schema as it was.
    let (_dir, conn) = db_at(before());
    accepts(&conn, SETUP);
    let first = analysis(&conn, 1, "user", 5);
    accepts(&conn, "DELETE FROM analysis");
    assert_eq!(analysis(&conn, 2, "mik", 5), first);
}

#[test]
fn a_new_analysis_row_never_takes_the_id_of_a_deleted_one() {
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    let first = analysis(&conn, 1, "user", 5);
    let second = analysis(&conn, 2, "user", 6);
    accepts(&conn, "DELETE FROM analysis");
    let third = analysis(&conn, 1, "mik", 7);
    assert!(third > second && second > first, "{first} {second} {third}");
    // The deleted row can go back under its own id, as Undo puts it.
    accepts(
        &conn,
        &format!("INSERT INTO analysis (id, recording_id, source, energy) VALUES ({second}, 2, 'user', 6)"),
    );
    assert!(analysis(&conn, 2, "mik", 7) > third);
}

#[test]
fn analysis_rows_that_exist_when_the_migration_runs_keep_their_ids_and_values() {
    let (_dir, mut conn) = db_at(before());
    accepts(&conn, SETUP);
    accepts(
        &conn,
        "INSERT INTO analysis (id, recording_id, source, bpm, key, energy, confidence, analyzed_at)
         VALUES (3, 1, 'rekordbox', 128.5, '8A', NULL, NULL, '2026-02-03T04:05:06.000Z'),
                (7, 1, 'mik', NULL, '9A', 7, 0.5, '2026-02-03T04:05:07.000Z'),
                (8, 2, 'user', NULL, NULL, 10, NULL, '2026-02-03T04:05:08.000Z');",
    );
    let before_rows = rows(&conn);
    assert_eq!(before_rows.split(' ').count(), 3);

    run(&mut conn, &MIGRATIONS[..=before()]).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    assert_eq!(rows(&conn), before_rows);
    // New rows carry on after the highest id that was there.
    assert_eq!(analysis(&conn, 2, "mik", 4), 9);
}

#[test]
fn with_no_analysis_rows_the_first_one_is_number_one() {
    let (_dir, mut conn) = db_at(before());
    accepts(&conn, SETUP);
    run(&mut conn, &MIGRATIONS[..=before()]).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    assert_eq!(analysis(&conn, 1, "user", 5), 1);
}

#[test]
fn after_the_migration_analysis_keeps_its_rules() {
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    analysis(&conn, 1, "user", 5);
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source, energy) VALUES (1, 'user', 6)",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source, energy) VALUES (99, 'user', 6)",
        "FOREIGN KEY",
    );
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source, energy) VALUES (2, 'user', 11)",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source, energy) VALUES (2, 'user', 0)",
        "CHECK",
    );
    // The user gives energy only.
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (2, 'user', 120)",
        "CHECK",
    );
    // rekordbox gives no energy.
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source, energy) VALUES (2, 'rekordbox', 5)",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source) VALUES (2, 'tag')",
        "CHECK",
    );
    refuses(&conn, "DELETE FROM recording WHERE id = 1", "FOREIGN KEY");
}

#[test]
fn after_the_migration_a_track_shows_the_users_energy_before_every_other_sources() {
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    analysis(&conn, 1, "ml", 2);
    analysis(&conn, 1, "local", 3);
    analysis(&conn, 1, "mik", 4);
    let shown = || -> (i64, String) {
        conn.query_row(
            "SELECT energy, energy_source FROM recording_display WHERE recording_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(shown(), (4, "mik".to_owned()));
    analysis(&conn, 1, "user", 9);
    assert_eq!(shown(), (9, "user".to_owned()));
    accepts(&conn, "DELETE FROM analysis WHERE source = 'user'");
    assert_eq!(shown(), (4, "mik".to_owned()));
}

#[test]
fn a_row_with_no_bpm_and_no_key_does_not_make_the_track_due_for_the_search_index() {
    let (_dir, conn) = db();
    accepts(&conn, SETUP);
    let energy = analysis(&conn, 1, "user", 5);
    accepts(&conn, &format!("DELETE FROM analysis WHERE id = {energy}"));
    assert_eq!(due(&conn), 0);
    // BPM and key are what search and the table read: they still do.
    accepts(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (1, 'rekordbox', 128)",
    );
    assert_eq!(due(&conn), 1);
    accepts(
        &conn,
        "DELETE FROM search_due;
         INSERT INTO analysis (recording_id, source, key) VALUES (2, 'tag', '8A');
         DELETE FROM search_due;
         DELETE FROM analysis WHERE recording_id = 2;",
    );
    assert_eq!(due(&conn), 1, "taking a row with a key away is recorded");
}

// ---- What the rebuild must keep ------------------------------------------

/// Every schema object as (type, name, table, SQL), sorted.
fn schema(conn: &Connection) -> Vec<(String, String, String, Option<String>)> {
    let mut stmt = conn
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

/// The names of the indexes and triggers on `analysis`.
fn on_analysis(conn: &Connection, kind: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = ?1 AND tbl_name = 'analysis' ORDER BY name")
        .unwrap();
    let rows = stmt.query_map([kind], |r| r.get(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

/// The tables that have a foreign key to `analysis`.
fn pointing_at_analysis(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT m.name FROM sqlite_master m, pragma_foreign_key_list(m.name) f
             WHERE m.type = 'table' AND f.\"table\" = 'analysis' ORDER BY m.name",
        )
        .unwrap();
    let rows = stmt.query_map([], |r| r.get(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

#[test]
fn the_rebuild_keeps_every_index_and_trigger_of_analysis_and_changes_nothing_else_in_the_schema() {
    let (_dir, mut conn) = db_at(before());
    let old = schema(&conn);
    run(&mut conn, &MIGRATIONS[..=before()]).unwrap();
    let new = schema(&conn);

    // The same indexes (the unique key's) and triggers, by name.
    assert!(!on_analysis(&conn, "index").is_empty());
    assert_eq!(
        on_analysis(&conn, "index"),
        old.iter()
            .filter(|(kind, _, table, _)| kind == "index" && table == "analysis")
            .map(|(_, name, _, _)| name.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        on_analysis(&conn, "trigger"),
        [
            "search_analysis_added",
            "search_analysis_changed",
            "search_analysis_removed"
        ]
    );
    assert_eq!(
        on_analysis(&conn, "trigger"),
        old.iter()
            .filter(|(kind, _, table, _)| kind == "trigger" && table == "analysis")
            .map(|(_, name, _, _)| name.clone())
            .collect::<Vec<_>>()
    );

    // Everything that isn't `analysis` or one of its triggers is text-identical,
    // the view `recording_display` included.
    let rest = |all: &[(String, String, String, Option<String>)]| {
        all.iter()
            .filter(|(_, _, table, _)| table != "analysis")
            .cloned()
            .collect::<Vec<_>>()
    };
    assert!(rest(&old)
        .iter()
        .any(|(_, name, _, _)| name == "recording_display"));
    assert_eq!(rest(&new), rest(&old));

    // The only change to `analysis` is the ids that are never reused.
    let table = |all: &[(String, String, String, Option<String>)]| {
        all.iter()
            .find(|(kind, name, _, _)| kind == "table" && name == "analysis")
            .and_then(|(_, _, _, sql)| sql.clone())
            .unwrap()
    };
    // (SQLite quotes the name when it renames the new table into place.)
    assert_eq!(
        table(&new)
            .replace("CREATE TABLE \"analysis\"", "CREATE TABLE analysis")
            .replace("INTEGER PRIMARY KEY AUTOINCREMENT", "INTEGER PRIMARY KEY"),
        table(&old)
    );
}

#[test]
fn nothing_points_at_analysis_before_or_after_and_no_reference_is_left_broken() {
    let (_dir, mut conn) = db_at(before());
    accepts(&conn, SETUP);
    accepts(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (1, 'rekordbox', 128)",
    );
    assert_eq!(pointing_at_analysis(&conn), Vec::<String>::new());
    run(&mut conn, &MIGRATIONS[..=before()]).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    assert_eq!(pointing_at_analysis(&conn), Vec::<String>::new());
    let broken: i64 = one(&conn, "SELECT count(*) FROM pragma_foreign_key_check");
    assert_eq!(broken, 0);
    // analysis still points at its track.
    let targets: Vec<String> = conn
        .prepare("SELECT \"table\" FROM pragma_foreign_key_list('analysis')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(targets, ["recording"]);
}

#[test]
fn the_rebuild_is_one_transaction_a_failing_step_leaves_the_old_table_whole() {
    let (_dir, mut conn) = db_at(before());
    accepts(&conn, SETUP);
    accepts(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (1, 'rekordbox', 128)",
    );
    let old = schema(&conn);
    let rows_before = rows(&conn);
    // This migration with a last step that fails.
    let real = MIGRATIONS[before()];
    let sql: &'static str = Box::leak(
        format!(
            "{}
SELECT * FROM no_such_table;
",
            real.sql
        )
        .into_boxed_str(),
    );
    let mut list = MIGRATIONS[..before()].to_vec();
    list.push(Migration {
        name: real.name,
        sql,
    });
    assert!(run(&mut conn, &list).is_err());
    assert_eq!(schema(&conn), old);
    assert_eq!(rows(&conn), rows_before);
}
