//! 0038: `crate_entry` ids are never reused, and `sent_playlist.replaced`
//! (ROADMAP §2, 1.10; 1cB-3).

use super::{accepts, columns, db_at, insert, one, refuses, run, MIGRATIONS};

/// How many migrations come before 0038: its index in [`MIGRATIONS`].
fn before() -> usize {
    MIGRATIONS.iter().position(|m| m.version() == 38).unwrap()
}

const SETUP: &str = "
    INSERT INTO recording (title) VALUES ('Track 1'), ('Track 2'), ('Track 3');
    INSERT INTO library_track (recording_id, source_status)
        VALUES (1, 'missing'), (2, 'missing'), (3, 'missing');
    INSERT INTO crate (kind, name) VALUES ('static', 'Peak');
    INSERT INTO crate_entry (crate_id, library_track_id, added_at)
        VALUES (1, 1, '2026-01-01T00:00:00.000Z'), (1, 2, '2026-01-01T00:00:01.000Z'),
               (1, 3, '2026-01-01T00:00:02.000Z');";

#[test]
fn a_crate_entry_keeps_its_id_and_a_new_one_never_takes_one_the_table_or_the_log_ever_held() {
    let (_dir, mut conn) = db_at(before());
    accepts(&conn, SETUP);
    // Entry 3 is deleted; an operation waiting for an undo names entry 7.
    accepts(&conn, "DELETE FROM crate_entry WHERE id = 3");
    accepts(
        &conn,
        "INSERT INTO operation (kind) VALUES ('add_to_crate');
         INSERT INTO change (operation_id, entity, entity_id, action, field, after)
             VALUES (1, 'crate_entry', 7, 'insert', 'crate_id', '1');",
    );

    run(&mut conn, &MIGRATIONS[..=before()]).unwrap();

    assert_eq!(
        one::<String>(
            &conn,
            "SELECT group_concat(id || ':' || library_track_id || ':' || added_at, ' ')
             FROM (SELECT * FROM crate_entry ORDER BY id)"
        ),
        "1:1:2026-01-01T00:00:00.000Z 2:2:2026-01-01T00:00:01.000Z"
    );
    accepts(&conn, "DELETE FROM search_due");
    let id = insert(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (1, 3)",
    );
    assert_eq!(id, 8);
    // Its checks and triggers are back: a track is in a crate once, a
    // folder holds no tracks, and a track joining a crate is due for the
    // search index.
    refuses(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (1, 3)",
        "UNIQUE",
    );
    accepts(&conn, "INSERT INTO crate (kind, name) VALUES ('folder', 'Fri')");
    refuses(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (2, 1)",
        "a folder holds no tracks",
    );
    assert_eq!(
        one::<i64>(&conn, "SELECT count(*) FROM search_due WHERE recording_id = 3"),
        1
    );
    let sql: String = one(
        &conn,
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'crate_entry'",
    );
    assert!(sql.contains("AUTOINCREMENT") && sql.contains("STRICT"), "{sql}");
}

#[test]
fn what_a_send_changed_in_a_playlist_is_a_json_array_or_nothing() {
    let (_dir, mut conn) = db_at(before());
    accepts(
        &conn,
        r#"INSERT INTO sent_playlist (path, path_key, kind) VALUES ('["Crates","Peak"]', '["crates","peak"]', 'playlist')"#,
    );
    assert!(!columns(&conn, "sent_playlist").contains(&"replaced".to_owned()));

    run(&mut conn, &MIGRATIONS[..=before()]).unwrap();

    assert_eq!(
        one::<Option<String>>(&conn, "SELECT replaced FROM sent_playlist"),
        None
    );
    accepts(&conn, "UPDATE sent_playlist SET replaced = '[1,2]'");
    refuses(&conn, "UPDATE sent_playlist SET replaced = '1'", "CHECK");
    refuses(&conn, "UPDATE sent_playlist SET replaced = 'x'", "CHECK");
}
