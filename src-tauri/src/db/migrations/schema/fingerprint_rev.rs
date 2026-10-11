//! 0020: `file.fingerprint_rev`, the stamp that moves with a file's
//! fingerprint (1bA-18; ROADMAP 1.4, 5.1), and its two triggers.

use rusqlite::Connection;

use super::super::{run, MIGRATIONS};
use super::files::with_a_file;
use super::{accepts, db_at, one};

fn stamp(conn: &Connection, id: i64) -> i64 {
    one(
        conn,
        &format!("SELECT fingerprint_rev FROM file WHERE id = {id}"),
    )
}

/// Files 1, 2 and 3: 1 and 2 have a fingerprint, 3 has none.
fn with_two_fingerprinted_files() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = with_a_file();
    accepts(
        &conn,
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'b.mp3', 'b.mp3');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'c.mp3', 'c.mp3');
         UPDATE file SET fingerprint = x'0102' WHERE id IN (1, 2);",
    );
    (dir, conn)
}

#[test]
fn files_that_exist_when_the_migration_runs_start_at_stamp_zero_with_their_fingerprints_kept() {
    let (_dir, mut conn) = db_at(19);
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'b.mp3', 'b.mp3');
         UPDATE file SET fingerprint = x'0102' WHERE id = 1;",
    );
    run(&mut conn, &MIGRATIONS[..20]).unwrap();
    for id in [1, 2] {
        assert_eq!(stamp(&conn, id), 0, "file {id}");
    }
    let kept: Vec<u8> = one(&conn, "SELECT fingerprint FROM file WHERE id = 1");
    assert_eq!(kept, [1, 2]);

    // From here on the stamp follows the bytes.
    accepts(&conn, "UPDATE file SET fingerprint = x'0304' WHERE id = 1");
    assert_ne!(stamp(&conn, 1), 0);
    assert_eq!(stamp(&conn, 2), 0);
}

#[test]
fn a_changed_fingerprint_gets_a_new_stamp_and_no_other_file_does() {
    let (_dir, conn) = with_two_fingerprinted_files();
    let (one_before, two_before) = (stamp(&conn, 1), stamp(&conn, 2));
    accepts(&conn, "UPDATE file SET fingerprint = x'0305' WHERE id = 1");
    assert_ne!(stamp(&conn, 1), one_before);
    assert_eq!(stamp(&conn, 2), two_before);
}

#[test]
fn a_fingerprint_given_to_a_file_or_taken_away_moves_its_stamp() {
    let (_dir, conn) = with_two_fingerprinted_files();
    // From none to one…
    assert_eq!(stamp(&conn, 3), 0);
    accepts(&conn, "UPDATE file SET fingerprint = x'0102' WHERE id = 3");
    let given = stamp(&conn, 3);
    assert_ne!(given, 0);
    // …and from one to none, with no value remembered after it.
    accepts(&conn, "UPDATE file SET fingerprint = NULL WHERE id = 3");
    assert_ne!(stamp(&conn, 3), given);
    // Back to the same bytes: not the stamp they had before.
    let cleared = stamp(&conn, 3);
    accepts(&conn, "UPDATE file SET fingerprint = x'0102' WHERE id = 3");
    assert_ne!(stamp(&conn, 3), given);
    assert_ne!(stamp(&conn, 3), cleared);
}

#[test]
fn a_tag_rewrite_a_new_modified_time_or_the_same_bytes_written_again_keep_the_stamp() {
    // rekordbox rewrites tags and bumps modified times (ROADMAP 5.1): none
    // of that is new audio.
    let (_dir, conn) = with_two_fingerprinted_files();
    let before = stamp(&conn, 1);
    accepts(
        &conn,
        "UPDATE file SET size = 11, mtime = 200 WHERE id = 1;
         UPDATE file SET fingerprint = x'0102' WHERE id = 1;
         UPDATE file SET present = 0 WHERE id = 1;
         UPDATE file SET present = 1 WHERE id = 1",
    );
    assert_eq!(stamp(&conn, 1), before);
}

#[test]
fn a_file_inserted_with_a_fingerprint_is_stamped_and_one_without_is_not() {
    let (_dir, conn) = with_a_file();
    accepts(
        &conn,
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, fingerprint)
             VALUES (1, 'with.mp3', 'with.mp3', x'0102');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key)
             VALUES (1, 'without.mp3', 'without.mp3');",
    );
    assert_ne!(stamp(&conn, 2), 0);
    assert_eq!(stamp(&conn, 3), 0);
}

#[test]
fn a_file_that_takes_a_deleted_files_id_does_not_get_the_stamp_the_deleted_one_had() {
    // A removed music folder's files held the highest ids, and the next
    // files take them again.
    let (_dir, conn) = with_two_fingerprinted_files();
    accepts(&conn, "UPDATE file SET fingerprint = x'0102' WHERE id = 3");
    let old = stamp(&conn, 3);
    accepts(&conn, "DELETE FROM file WHERE id = 3");
    accepts(
        &conn,
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, fingerprint)
             VALUES (1, 'again.mp3', 'again.mp3', x'0102')",
    );
    assert_eq!(
        one::<i64>(&conn, "SELECT id FROM file WHERE rel_path = 'again.mp3'"),
        3
    );
    assert_ne!(stamp(&conn, 3), old);
}

#[test]
fn the_stamp_triggers_survive_every_later_migration() {
    // A later migration that rebuilds `file` would silently drop the
    // triggers on it.
    let triggers = [
        "file_fingerprint_rev_moves_with_its_fingerprint",
        "file_fingerprint_rev_starts_with_its_fingerprint",
    ];
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    let mut added = false;
    for (n, migration) in MIGRATIONS.iter().enumerate() {
        run(&mut conn, &MIGRATIONS[..=n]).unwrap();
        added |= migration.name == "0020_fingerprint_rev.sql";
        if !added {
            continue;
        }
        for trigger in triggers {
            let there: bool = one(
                &conn,
                &format!(
                    "SELECT EXISTS (SELECT 1 FROM sqlite_master
                     WHERE type = 'trigger' AND tbl_name = 'file' AND name = '{trigger}')"
                ),
            );
            assert!(there, "{trigger} is gone after {}", migration.name);
        }
    }
    assert!(added, "0020 was never applied");
}
