//! 0031: a `file` or `recording` row id is never used twice, so an id the
//! operation log holds as a value names its own row or none (ROADMAP §2
//! `operation` / `change`). Both tables are rebuilt for it, so everything
//! they had is proven again here: every row of every table, every index,
//! trigger and view (text-identical), the columns and foreign keys; and
//! that the rebuild is one transaction, rolled back whole when a step
//! fails.

use rusqlite::types::Value;
use rusqlite::Connection;

use super::{accepts, db_at, insert, one};
use crate::db::migrations::{applied, run, Migration, MigrationError, MIGRATIONS};
use crate::db::DbError;

const NAME: &str = "0031_file_and_track_ids_never_reused.sql";

/// A volume, a music folder, three files and three tracks with every column
/// filled somewhere, and a row in every table that points at a file or a
/// track (the ones that cascade too).
const POPULATED: &str = r#"
    INSERT INTO volume (identity, kind) VALUES ('serial=1A2B', 'external');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'music');
    INSERT INTO file (id, music_folder_id, rel_path, rel_path_key, size, mtime, blake3, audio_hash,
                      fingerprint, sniffed_format, codec, bitrate, sample_rate, duration_ms,
                      quality_verdict, raw_tags, present, hidden_reason, first_seen_at,
                      last_seen_at, file_id, online_only, partial_hash, fingerprint_audio_hash)
    VALUES (3, 1, 'A/Velo.mp3', 'a/velo.mp3', 9000, 1700000000123456789, zeroblob(32),
            x'0102', x'0304', 'mp3', 'mp3', 320, 44100, 215000, 'ok', '{"TIT2":"Velo"}', 1, NULL,
            '2026-01-02T03:04:05.000Z', '2026-01-03T03:04:05.000Z',
            '0123456789abcdef-0123456789abcdef0123456789abcdef', 0, zeroblob(33), zeroblob(34)),
           (5, 1, 'A/Velo.flac', 'a/velo.flac', 30000, 1, NULL, x'0506', NULL, 'flac', 'flac',
            NULL, 48000, 215001, NULL, NULL, 0, 'user', '2026-01-02T03:04:06.000Z', NULL, NULL,
            1, NULL, NULL),
           (9, 1, 'B/Nightdrive.mp3', 'b/nightdrive.mp3', 1, 2, NULL, NULL, NULL, NULL, NULL,
            NULL, NULL, NULL, NULL, NULL, 1, NULL, '2026-01-02T03:04:07.000Z', NULL, NULL, 0,
            NULL, NULL);
    INSERT INTO recording (id, title, artist, version, remixers, label, year, genre, provenance,
                           created_at)
    VALUES (2, 'Velo', 'Halden Rook', 'Extended Mix', 'Someone', 'Label', 2024, 'House',
            '{"title":{"source":"tag","confidence":1.0}}', '2026-01-02T03:04:05.000Z'),
           (4, 'Nightdrive', NULL, NULL, NULL, NULL, NULL, NULL, '{}', '2026-01-02T03:04:06.000Z'),
           (6, 'Gone', NULL, NULL, NULL, NULL, NULL, NULL, '{}', '2026-01-02T03:04:07.000Z');
    INSERT INTO recording_file (recording_id, file_id, role, match_confidence)
    VALUES (2, 3, 'best', NULL), (2, 5, 'extra', 0.9), (4, 9, 'best', NULL);
    INSERT INTO analysis (recording_id, source, bpm, key) VALUES (2, 'tag', 124.0, '8A');
    INSERT INTO version_link (recording_a, recording_b, kind, source)
    VALUES (2, 4, 'rework', 'user');
    INSERT INTO library_track (recording_id, kind, linked_file_id, source_file_id)
    VALUES (2, 'linked', 3, 5);
    INSERT INTO library_removal (recording_id) VALUES (4);
    INSERT INTO track_merge (recording_id, into_id) VALUES (6, 2);
    INSERT INTO rekordbox_track (attributes, location_key, read_at, file_id, recording_id,
                                 relink_method)
    VALUES (json_object('TrackID', '7', 'Location', 'file://localhost/E:/Music/A/Velo.mp3'),
            'E:/Music/A/Velo.mp3', '2026-01-02T03:04:05.000Z', 3, 2, 'path');
    INSERT INTO relink (location_key, file_key, file_id, method)
    VALUES ('E:/Music/A/Velo.mp3', x'0102', 3, 'path');
    INSERT INTO file_stage (file_id, stage, version, status)
    VALUES (3, 'read', 1, 'done'), (5, 'hash', 1, 'done');
    INSERT INTO fingerprint_match (file_a, file_b, version, items_a, items_b, coverage_a,
                                   coverage_b, score, segments)
    VALUES (3, 5, 1, 10, 10, 1, 1, 2, '[]');
    INSERT INTO fingerprint_matched (file_id, version, stands) VALUES (5, 1, 3);
    INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_hz, decoded_ms, ended)
    VALUES (3, zeroblob(34), 1, 16000, 215000, 'complete');
    INSERT INTO file_verdict (file_id, rules_version, inputs, verdict) VALUES (3, 1, 'x', 'ok');
    INSERT INTO grouping_pair (file_a, file_b, outcome, reasons, state)
    VALUES (3, 5, 'duplicate', '[]', 'merged');
    INSERT INTO grouping_file (file_id, placed_key) VALUES (3, x'0102'), (9, NULL);
    INSERT INTO better_file (recording_id, file_id, step) VALUES (2, 5, 'format');"#;

const FOLDER: &str = "
    INSERT INTO volume (identity, kind) VALUES ('serial=1A2B', 'external');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'music');";

/// How many migrations come before this one: the schema as it was.
fn before() -> usize {
    MIGRATIONS
        .iter()
        .position(|m| m.name == NAME)
        .expect("the migration is in the list")
}

/// A database as it was before this migration, with one music folder.
fn as_it_was() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db_at(before());
    accepts(&conn, FOLDER);
    (dir, conn)
}

/// A database with this migration (and none after it), with one music folder.
fn as_it_is() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db_at(before() + 1);
    accepts(&conn, FOLDER);
    (dir, conn)
}

fn migrate(conn: &mut Connection) {
    assert_eq!(run(conn, &MIGRATIONS[..=before()]).unwrap(), 1);
    conn.pragma_update(None, "foreign_keys", true).unwrap();
}

/// A new file, as a scan makes one; returns its id.
fn file(conn: &Connection, name: &str) -> i64 {
    insert(
        conn,
        &format!(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key)
             VALUES (1, '{name}', '{name}')"
        ),
    )
}

/// A new track, as the grouping job makes one; returns its id.
fn track(conn: &Connection) -> i64 {
    insert(conn, "INSERT INTO recording (title) VALUES ('Velo')")
}

/// A query's rows, each as its columns joined into one text, sorted.
fn listed(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    let columns = stmt.column_count();
    let rows = stmt
        .query_map([], |r| {
            let mut row = Vec::new();
            for i in 0..columns {
                row.push(format!("{:?}", r.get::<_, Value>(i)?));
            }
            Ok(row.join(" | "))
        })
        .unwrap();
    let mut rows: Vec<String> = rows.map(Result::unwrap).collect();
    rows.sort();
    rows
}

/// Every row of every table, every column, by table. `sqlite_sequence` and
/// `schema_migration` are left out: the migration adds to them on purpose.
fn every_row(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT IN ('sqlite_sequence', 'schema_migration')
             ORDER BY name",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let rows = listed(conn, &format!("SELECT * FROM \"{table}\""));
            (table, rows)
        })
        .collect()
}

/// Every schema object: type, name, table and its SQL as SQLite holds it.
fn schema(conn: &Connection) -> Vec<String> {
    listed(conn, "SELECT type, name, tbl_name, sql FROM sqlite_master")
}

// ---- Ids ------------------------------------------------------------------

#[test]
fn before_this_migration_a_new_file_and_a_new_track_took_the_ids_of_the_last_deleted_ones() {
    // The fault being fixed, shown on the schema as it was: removing a
    // music folder deletes the newest file, the grouping job sweeps the
    // newest track, and the next ones made take their ids.
    let (_dir, conn) = as_it_was();
    let (f, t) = (file(&conn, "a.mp3"), track(&conn));
    accepts(&conn, "DELETE FROM file; DELETE FROM recording;");
    assert_eq!((file(&conn, "b.mp3"), track(&conn)), (f, t));
}

#[test]
fn a_new_file_or_track_never_takes_the_id_of_a_deleted_one() {
    let (_dir, conn) = as_it_is();
    let files = [file(&conn, "a.mp3"), file(&conn, "b.mp3")];
    let tracks = [track(&conn), track(&conn)];
    accepts(&conn, "DELETE FROM file; DELETE FROM recording;");
    let (f, t) = (file(&conn, "c.mp3"), track(&conn));
    assert!(f > files[1] && t > tracks[1], "{f} {t}");
    // A deleted row can still go back under its own id.
    accepts(
        &conn,
        &format!(
            "INSERT INTO file (id, music_folder_id, rel_path, rel_path_key)
             VALUES ({}, 1, 'b.mp3', 'b.mp3');
             INSERT INTO recording (id, title) VALUES ({}, 'Velo');",
            files[1], tracks[1]
        ),
    );
    assert!(file(&conn, "d.mp3") > f && track(&conn) > t);
}

#[test]
fn files_and_tracks_keep_their_ids_and_new_ones_carry_on_after_the_highest() {
    let (_dir, mut conn) = db_at(before());
    accepts(&conn, POPULATED);
    migrate(&mut conn);
    assert_eq!(file(&conn, "c.mp3"), 10);
    assert_eq!(track(&conn), 7);
}

#[test]
fn with_no_files_or_tracks_and_none_in_the_log_the_first_of_each_is_number_one() {
    let (_dir, mut conn) = as_it_was();
    // Ids of other tables in the log don't count.
    accepts(
        &conn,
        "INSERT INTO operation (kind, details) VALUES ('create_crate', '{\"crateId\": 900}');
         INSERT INTO change (operation_id, entity, entity_id, action, field, after)
         VALUES (1, 'crate', 900, 'insert', 'name', 'x'),
                (1, 'crate_entry', 901, 'insert', 'library_track_id', '902');",
    );
    migrate(&mut conn);
    assert_eq!((file(&conn, "a.mp3"), track(&conn)), (1, 1));
}

/// Which of a change's values holds the id.
#[derive(Clone, Copy)]
enum Side {
    Before,
    After,
}

/// The next file, or track, made after the migration.
fn next(conn: &Connection, table: &str) -> i64 {
    if table == "file" {
        file(conn, "a.mp3")
    } else {
        track(conn)
    }
}

/// After the migration, the next row of `table` takes an id past the one an
/// operation recorded as `entity`.`field`, although no row has it.
fn past_a_logged_value(table: &str, entity: &str, field: &str, side: Side) {
    let (_dir, mut conn) = as_it_was();
    let (action, before, after) = match side {
        Side::Before => ("delete", "'77'", "NULL"),
        Side::After => ("insert", "NULL", "'77'"),
    };
    accepts(
        &conn,
        &format!(
            "INSERT INTO operation (kind) VALUES ('test');
             INSERT INTO change (operation_id, entity, entity_id, action, field, before, after)
             VALUES (1, '{entity}', 3, '{action}', '{field}', {before}, {after});"
        ),
    );
    migrate(&mut conn);
    assert_eq!(next(&conn, table), 78, "{entity}.{field}");
}

/// The same, for an id in an operation's details.
fn past_a_detail(table: &str, details: &str) {
    let (_dir, mut conn) = as_it_was();
    accepts(
        &conn,
        &format!("INSERT INTO operation (kind, details) VALUES ('test', '{details}')"),
    );
    migrate(&mut conn);
    assert_eq!(next(&conn, table), 78, "{details}");
}

#[test]
fn a_file_id_the_log_names_stays_free_although_the_file_was_deleted_before_the_migration() {
    for (entity, field) in [
        ("library_track", "linked_file_id"),
        ("library_track", "source_file_id"),
        ("relink", "file_id"),
    ] {
        past_a_logged_value("file", entity, field, Side::Before);
        past_a_logged_value("file", entity, field, Side::After);
    }
    // `recording_file` is named by its file's id (a merge moves files).
    let (_dir, mut conn) = as_it_was();
    accepts(
        &conn,
        "INSERT INTO operation (kind) VALUES ('merge_library_tracks');
         INSERT INTO change (operation_id, entity, entity_id, action, field, before, after)
         VALUES (1, 'recording_file', 77, 'set', 'recording_id', '1', '2');",
    );
    migrate(&mut conn);
    assert_eq!(file(&conn, "a.mp3"), 78);
    // The details a switch, a follow, a dismissal, a merge and a split keep.
    for key in ["from", "to", "fileId", "keepLinked", "file"] {
        past_a_detail("file", &format!("{{\"{key}\": 77}}"));
    }
    // A split's and "same track"'s moved files, and a split's files that
    // stayed.
    for key in ["moved", "stayed"] {
        past_a_detail(
            "file",
            &format!("{{\"{key}\": [{{\"file\": 3, \"key\": \"\"}}, {{\"file\": 77, \"key\": \"01\"}}]}}"),
        );
    }
}

#[test]
fn a_track_id_the_log_names_stays_free_although_the_track_was_swept_before_the_migration() {
    for (entity, field) in [
        ("library_track", "recording_id"),
        ("library_removal", "recording_id"),
        ("replaced_entry", "recording_id"),
        ("track_merge", "recording_id"),
        ("track_merge", "into_id"),
        ("recording_file", "recording_id"),
    ] {
        past_a_logged_value("recording", entity, field, Side::Before);
        past_a_logged_value("recording", entity, field, Side::After);
    }
    for details in [
        "{\"recordingId\": 77}",
        "{\"keep\": 77}",
        "{\"recordingIds\": [3, 77, 5]}",
        // A split's track the files left, and the track they went to.
        "{\"from\": 77}",
        "{\"into\": 77}",
    ] {
        past_a_detail("recording", details);
    }
}

#[test]
fn each_table_counts_on_from_its_own_ids_and_the_other_counts_stay() {
    let (_dir, mut conn) = as_it_was();
    accepts(
        &conn,
        "INSERT INTO operation (kind, details) VALUES ('test', '{\"keep\": 40, \"to\": 30}');
         UPDATE sqlite_sequence SET seq = 12 WHERE name = 'sync_base';",
    );
    let others = listed(
        &conn,
        "SELECT * FROM sqlite_sequence WHERE name NOT IN ('file', 'recording')",
    );
    assert_eq!(
        others.len(),
        3,
        "sync_base, conflict and relink: {others:?}"
    );
    migrate(&mut conn);
    assert_eq!(
        listed(
            &conn,
            "SELECT * FROM sqlite_sequence WHERE name IN ('file', 'recording')"
        ),
        [
            "Text(\"file\") | Integer(30)",
            "Text(\"recording\") | Integer(40)"
        ]
    );
    assert_eq!(
        listed(
            &conn,
            "SELECT * FROM sqlite_sequence WHERE name NOT IN ('file', 'recording')"
        ),
        others
    );
}

// ---- Nothing else changed -------------------------------------------------

#[test]
fn every_row_of_every_table_is_exactly_as_it_was() {
    // Dropping the old tables with foreign keys on would have deleted every
    // row that cascades from a file or a track (`file_stage`,
    // `fingerprint_match`, `better_file`, …) and emptied `relink.file_id`.
    // The runner switches them off before the migration's transaction.
    let (_dir, mut conn) = db_at(before());
    accepts(&conn, POPULATED);
    let rows = every_row(&conn);
    for (table, expected) in [
        ("file", 3),
        ("recording", 3),
        ("recording_file", 3),
        ("file_stage", 2),
        ("fingerprint_match", 1),
        ("fingerprint_matched", 1),
        ("file_quality", 1),
        ("file_verdict", 1),
        ("grouping_pair", 1),
        ("grouping_file", 2),
        ("better_file", 1),
        ("relink", 1),
        ("rekordbox_track", 1),
        ("library_track", 1),
        ("library_removal", 1),
        ("track_merge", 1),
        ("analysis", 1),
        ("version_link", 1),
    ] {
        let held = rows.iter().find(|(t, _)| t == table).unwrap();
        assert_eq!(held.1.len(), expected, "{table}");
    }

    migrate(&mut conn);

    let after = every_row(&conn);
    assert_eq!(after.len(), rows.len());
    for (was, is) in rows.iter().zip(&after) {
        assert_eq!(is, was, "{}", was.0);
    }
    assert_eq!(
        one::<i64>(&conn, "SELECT count(*) FROM pragma_foreign_key_check"),
        0
    );
    assert_eq!(one::<String>(&conn, "PRAGMA integrity_check"), "ok");
}

#[test]
fn every_index_trigger_and_view_is_back_and_only_the_two_tables_say_autoincrement() {
    let (_old_dir, old) = db_at(before());
    let (_new_dir, new) = db_at(before() + 1);
    let table_sql = |conn: &Connection, table: &str| -> String {
        one(
            conn,
            &format!("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = '{table}'"),
        )
    };
    let mut expected = schema(&old);
    let mut found = schema(&new);
    for table in ["file", "recording"] {
        let was = table_sql(&old, table);
        let is = table_sql(&new, table);
        // SQLite quotes a renamed table's name; the rest is the same text,
        // with AUTOINCREMENT.
        let wanted = was
            .replacen(
                &format!("CREATE TABLE {table} ("),
                &format!("CREATE TABLE \"{table}\" ("),
                1,
            )
            .replacen(
                "INTEGER PRIMARY KEY,",
                "INTEGER PRIMARY KEY AUTOINCREMENT,",
                1,
            );
        assert_ne!(wanted, was);
        assert_eq!(is, wanted, "{table}");
        let row = |sql: &str| {
            format!(
                "Text(\"table\") | Text(\"{table}\") | Text(\"{table}\") | {:?}",
                Value::Text(sql.to_owned())
            )
        };
        let (was_row, is_row) = (row(&was), row(&is));
        assert!(expected.contains(&was_row) && found.contains(&is_row));
        expected.retain(|r| *r != was_row);
        found.retain(|r| *r != is_row);
    }
    assert_eq!(found, expected);
    assert!(expected.len() > 100, "{} objects", expected.len());
    // What the migration dropped and put back is among them.
    for name in [
        "recording_display",
        "library_track_status_follows_new_link_update",
        "library_track_status_follows_new_link_insert",
        "best_file_due_folder_changed",
        "best_file_due_volume_changed",
        "best_file_due_dismissal_made",
        "best_file_due_dismissal_taken_back",
        "file_by_audio_hash",
        "file_hidden",
        "library_track_status_follows_file",
        "search_file_text_changed",
        "search_track_is_named",
    ] {
        let named = format!("Text(\"{name}\")");
        assert!(found.iter().any(|r| r.contains(&named)), "{name} is gone");
    }
    assert_eq!(
        one::<i64>(
            &new,
            "SELECT count(*) FROM sqlite_master WHERE name IN ('file_new', 'recording_new')"
        ),
        0,
        "nothing of the rebuild is left behind"
    );
    for table in ["file", "recording"] {
        for sql in [
            format!("SELECT * FROM pragma_table_xinfo('{table}')"),
            format!("SELECT * FROM pragma_foreign_key_list('{table}')"),
            format!("SELECT * FROM pragma_table_list WHERE name = '{table}'"),
        ] {
            assert_eq!(listed(&new, &sql), listed(&old, &sql), "{sql}");
        }
    }
}

// ---- One transaction --------------------------------------------------------

/// The migrations before this one, then this one with `extra` at its end.
fn failing(extra: &str) -> Vec<Migration> {
    let real = MIGRATIONS[before()];
    let sql: &'static str = Box::leak(format!("{}\n{extra}\n", real.sql).into_boxed_str());
    let mut list = MIGRATIONS[..before()].to_vec();
    list.push(Migration { name: NAME, sql });
    list
}

/// Runs this migration with `extra` at its end, which makes it fail;
/// asserts that the database is exactly as it was, and returns the error.
fn fails_and_keeps_everything(extra: &str) -> MigrationError {
    let (_dir, mut conn) = db_at(before());
    accepts(&conn, POPULATED);
    let rows = every_row(&conn);
    let objects = schema(&conn);
    let versions = applied(&conn).unwrap();
    let sequence = listed(&conn, "SELECT * FROM sqlite_sequence");

    let error = match run(&mut conn, &failing(extra)) {
        Err(DbError::Migration(e)) => e,
        other => panic!("{other:?}"),
    };

    assert_eq!(schema(&conn), objects, "the schema is as it was");
    assert_eq!(every_row(&conn), rows, "every row is as it was");
    assert_eq!(listed(&conn, "SELECT * FROM sqlite_sequence"), sequence);
    assert_eq!(applied(&conn).unwrap(), versions, "and so is its version");
    error
}

#[test]
fn a_step_that_fails_leaves_the_database_exactly_as_it_was() {
    // A statement fails after the whole rebuild has run.
    let error = fails_and_keeps_everything("INSERT INTO no_such_table VALUES (1);");
    assert!(matches!(error, MigrationError::Failed { .. }), "{error}");
}

#[test]
fn a_rebuild_that_breaks_a_reference_is_rolled_back_whole() {
    // The runner checks every reference before it commits.
    let error = fails_and_keeps_everything("DELETE FROM file WHERE id = 3;");
    assert!(
        matches!(error, MigrationError::BrokeForeignKeys { .. }),
        "{error}"
    );
}
