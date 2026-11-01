//! 0045: `grouping_names`, the stored keys of the deciding run's index of
//! names (1cD-G1). What the run does with them is tested in
//! `grouping/tests/stored_names.rs`.

use super::{accepts, db, db_at, refuses};
use crate::db::migrations::{run, MIGRATIONS};

/// A file with a record, and its keys stored at `rev` 7.
const FILE: &str = r#"
    INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
    INSERT INTO grouping_file (file_id, rev) VALUES (1, 7);
    INSERT INTO grouping_names (file_id, rev, rules_version, entry)
        VALUES (1, 7, 3, '[["a"],[],[]]');"#;

#[test]
fn an_upgraded_library_starts_with_no_stored_keys_and_keeps_every_record() {
    let before = MIGRATIONS
        .iter()
        .position(|m| m.name.ends_with("_grouping_names.sql"))
        .expect("the migration");
    let (_dir, mut conn) = db_at(before);
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
         INSERT INTO grouping_file (file_id, decided_as, rev) VALUES (1, 9, 7);",
    );
    run(&mut conn, MIGRATIONS).unwrap();
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count("SELECT count(*) FROM grouping_names"), 0);
    assert_eq!(
        count("SELECT count(*) FROM grouping_file WHERE decided_as = 9 AND rev = 7"),
        1,
        "nothing is due again because of the upgrade"
    );
}

#[test]
fn stored_keys_go_with_the_file() {
    let (_dir, conn) = db();
    accepts(&conn, FILE);
    accepts(&conn, "DELETE FROM file WHERE id = 1");
    let left: i64 = conn
        .query_row("SELECT count(*) FROM grouping_names", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn stored_keys_need_a_record_and_must_be_json() {
    let (_dir, conn) = db();
    accepts(&conn, FILE);
    refuses(
        &conn,
        "INSERT INTO grouping_names (file_id, rev, rules_version, entry)
         VALUES (2, 1, 3, '[[],[],[]]')",
        "FOREIGN KEY",
    );
    refuses(
        &conn,
        "UPDATE grouping_names SET entry = '[[' WHERE file_id = 1",
        "CHECK",
    );
}
