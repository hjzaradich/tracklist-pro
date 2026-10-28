//! 0036: what each send replaced (`sync_base.replaced`) and each track's
//! sent playlist entries (`sent_entry`) (ROADMAP §2, 1.10; 1cB-19).

use rusqlite::Connection;

use super::{accepts, columns, db, db_at, insert, one, refuses, run, MIGRATIONS};

const SETUP: &str = "
    INSERT INTO recording (title) VALUES ('Track 1'), ('Track 2');
    INSERT INTO library_track (recording_id, source_status) VALUES (1, 'missing'), (2, 'missing');";

fn with_library_tracks() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db();
    accepts(&conn, SETUP);
    (dir, conn)
}

fn entry(track: i64, path: &str, key: &str) -> String {
    format!(
        "INSERT INTO sent_entry (library_track_id, path, path_key, sent_at)
         VALUES ({track}, '{path}', '{key}', '2026-10-27T00:00:00.000Z')"
    )
}

#[test]
fn a_base_from_before_0036_has_no_record_of_what_it_replaced() {
    let (_dir, mut conn) = db_at(35);
    accepts(&conn, SETUP);
    accepts(
        &conn,
        "INSERT INTO sync_base (library_track_id, field, value) VALUES (1, 'Genre', 'House')",
    );
    assert!(!columns(&conn, "sync_base").contains(&"replaced".to_owned()));
    assert_eq!(
        one::<i64>(
            &conn,
            "SELECT count(*) FROM sqlite_master WHERE name = 'sent_entry'"
        ),
        0
    );

    run(&mut conn, &MIGRATIONS[..36]).unwrap();

    assert_eq!(
        one::<Option<String>>(&conn, "SELECT replaced FROM sync_base"),
        None
    );
    assert_eq!(one::<String>(&conn, "SELECT value FROM sync_base"), "House");
    // A record is any text, empty included; the table stays STRICT and
    // never reuses ids.
    accepts(&conn, "UPDATE sync_base SET replaced = ''");
    refuses(
        &conn,
        "UPDATE sync_base SET replaced = X'00'",
        "cannot store",
    );
    let sql: String = one(
        &conn,
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'sync_base'",
    );
    assert!(
        sql.contains("STRICT") && sql.contains("AUTOINCREMENT"),
        "{sql}"
    );
    let sql: String = one(
        &conn,
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'sent_entry'",
    );
    // What the operation log needs: a STRICT rowid table whose ids are
    // never used twice.
    assert!(sql.contains("STRICT"), "{sql}");
    assert!(sql.contains("AUTOINCREMENT"), "{sql}");
}

#[test]
fn a_sent_entry_is_one_per_track_and_playlist_held_to_its_library_track() {
    let (_dir, conn) = with_library_tracks();
    let warm_up = (r#"["Crates","Warm up"]"#, r#"["crates","warm up"]"#);
    accepts(&conn, &entry(1, warm_up.0, warm_up.1));
    accepts(&conn, &entry(2, warm_up.0, warm_up.1));
    accepts(
        &conn,
        &entry(1, r#"["Playlists","Peak"]"#, r#"["playlists","peak"]"#),
    );
    refuses(&conn, &entry(1, warm_up.0, warm_up.1), "UNIQUE");
    refuses(&conn, &entry(99, warm_up.0, warm_up.1), "FOREIGN KEY");
    refuses(
        &conn,
        "DELETE FROM library_track WHERE id = 1",
        "FOREIGN KEY",
    );
    // A path is the names from a top-level folder down: a JSON array of at
    // least two, and its key has as many.
    refuses(&conn, &entry(2, r#"["Crates"]"#, r#"["crates"]"#), "CHECK");
    refuses(
        &conn,
        &entry(2, "Crates/Peak", r#"["crates","peak"]"#),
        "CHECK",
    );
    refuses(
        &conn,
        &entry(2, r#"["Crates","Peak"]"#, r#"["crates"]"#),
        "CHECK",
    );
    refuses(
        &conn,
        &entry(2, r#"{"a":1}"#, r#"["crates","peak"]"#),
        "CHECK",
    );
}

#[test]
fn a_new_sent_entry_never_takes_the_id_of_a_deleted_one() {
    let (_dir, conn) = with_library_tracks();
    let first = insert(&conn, &entry(1, r#"["Crates","A"]"#, r#"["crates","a"]"#));
    let second = insert(&conn, &entry(1, r#"["Crates","B"]"#, r#"["crates","b"]"#));
    accepts(&conn, "DELETE FROM sent_entry");
    let third = insert(&conn, &entry(2, r#"["Crates","A"]"#, r#"["crates","a"]"#));
    assert!(third > second && second > first, "{first} {second} {third}");
}
