//! 0019: `fingerprint_matched`, which files matching has covered (1bA-14;
//! ROADMAP 1.4, 5.1), and its triggers beside 0016's.

use rusqlite::Connection;

use super::super::{run, MIGRATIONS};
use super::files::with_a_file;
use super::{accepts, one, refuses};

const MATCHED: &str = "INSERT INTO fingerprint_matched (file_id, version, stands) VALUES";

const RESULT: &str = "INSERT INTO fingerprint_match
    (file_a, file_b, version, items_a, items_b, coverage_a, coverage_b, score, segments) VALUES";

/// Files 1, 2 and 3, each with a fingerprint. All three have been through
/// matching: 1 and 2 hold the same fingerprint and 1 stands for both, 3
/// stands for itself. There's a result for 1–2 and for 1–3.
fn with_three_matched_files() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = with_a_file();
    accepts(
        &conn,
        &format!(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'b.mp3', 'b.mp3');
             INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'c.mp3', 'c.mp3');
             UPDATE file SET fingerprint = x'0102', size = 10, mtime = 100;
             UPDATE file SET fingerprint = x'0304' WHERE id = 3;
             {MATCHED} (1, 1, 1);
             {MATCHED} (2, 1, 1);
             {MATCHED} (3, 1, 3);
             {RESULT} (1, 2, 1, 100, 100, 1.0, 1.0, 0.0, '[[0,0,100,0.0,0]]');
             {RESULT} (1, 3, 1, 100, 80, 0.0, 0.0, 32.0, '[]');"
        ),
    );
    (dir, conn)
}

fn matched(conn: &Connection) -> Vec<i64> {
    let mut stmt = conn
        .prepare("SELECT file_id FROM fingerprint_matched ORDER BY file_id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows
}

fn results(conn: &Connection) -> i64 {
    one(conn, "SELECT COUNT(*) FROM fingerprint_match")
}

#[test]
fn a_file_has_one_row_naming_a_known_file_that_stands_for_it() {
    let (_dir, conn) = with_three_matched_files();
    refuses(&conn, &format!("{MATCHED} (1, 1, 1)"), "UNIQUE");
    accepts(&conn, "DELETE FROM fingerprint_matched");
    refuses(&conn, &format!("{MATCHED} (9, 1, 9)"), "FOREIGN KEY");
    refuses(&conn, &format!("{MATCHED} (1, 1, 9)"), "FOREIGN KEY");
    refuses(&conn, &format!("{MATCHED} (1, 0, 1)"), "CHECK");
    accepts(&conn, &format!("{MATCHED} (1, 2, 1)"));
}

#[test]
fn a_row_goes_when_its_file_row_goes_or_the_file_standing_for_it_does() {
    let (_dir, conn) = with_three_matched_files();
    accepts(&conn, "DELETE FROM file WHERE id = 3");
    assert_eq!(matched(&conn), [1, 2]);
    // File 1 stood for file 2: with it gone, file 2 is due again.
    accepts(&conn, "DELETE FROM file WHERE id = 1");
    assert_eq!(matched(&conn), [] as [i64; 0]);
}

#[test]
fn a_changed_or_cleared_fingerprint_drops_that_files_row_and_no_others() {
    let (_dir, conn) = with_three_matched_files();
    accepts(&conn, "UPDATE file SET fingerprint = x'0305' WHERE id = 3");
    assert_eq!(matched(&conn), [1, 2]);
    // A fingerprint taken away is a change too.
    accepts(&conn, "UPDATE file SET fingerprint = NULL WHERE id = 2");
    assert_eq!(matched(&conn), [1]);
}

#[test]
fn a_new_size_or_modified_time_or_the_same_fingerprint_written_again_keeps_the_row() {
    // rekordbox rewrites tags and bumps modified times (ROADMAP 5.1): none
    // of that is new audio.
    let (_dir, conn) = with_three_matched_files();
    accepts(
        &conn,
        "UPDATE file SET size = 11, mtime = 200;
         UPDATE file SET fingerprint = x'0102' WHERE id IN (1, 2);
         UPDATE file SET present = 1",
    );
    assert_eq!(matched(&conn), [1, 2, 3]);
}

#[test]
fn a_file_that_goes_missing_keeps_its_row_as_it_keeps_its_results() {
    let (_dir, conn) = with_three_matched_files();
    accepts(&conn, "UPDATE file SET present = 0 WHERE id = 3");
    assert_eq!(matched(&conn), [1, 2, 3]);
    assert_eq!(results(&conn), 2);
}

#[test]
fn a_missing_file_that_comes_back_is_due_again_and_keeps_its_results() {
    // Files that arrived while it was away were never compared with it.
    let (_dir, conn) = with_three_matched_files();
    accepts(
        &conn,
        "UPDATE file SET present = 0 WHERE id = 3;
         UPDATE file SET present = 1 WHERE id = 3",
    );
    assert_eq!(matched(&conn), [1, 2]);
    assert_eq!(
        results(&conn),
        2,
        "its results stay: nothing is compared twice"
    );
}

#[test]
fn one_fingerprint_change_drops_both_the_files_results_and_its_row() {
    // 0016's trigger and 0019's are on the same change: both run.
    let (_dir, conn) = with_three_matched_files();
    accepts(&conn, "UPDATE file SET fingerprint = x'0305' WHERE id = 3");
    assert_eq!(matched(&conn), [1, 2]);
    assert_eq!(results(&conn), 1, "the 1–3 result went, 1–2 stayed");
}

#[test]
fn the_triggers_on_a_files_fingerprint_survive_every_later_migration() {
    // A later migration that rebuilds `file` would silently drop the
    // triggers on it. After each migration from the one that adds it on,
    // each trigger is still there.
    let since = [
        (
            "0016_fingerprint_match.sql",
            "fingerprint_match_goes_with_its_fingerprint",
        ),
        (
            "0019_fingerprint_matched.sql",
            "fingerprint_matched_goes_with_its_fingerprint",
        ),
        (
            "0019_fingerprint_matched.sql",
            "fingerprint_matched_goes_when_a_missing_file_comes_back",
        ),
    ];
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    let mut expected: Vec<&str> = Vec::new();
    for (n, migration) in MIGRATIONS.iter().enumerate() {
        run(&mut conn, &MIGRATIONS[..=n]).unwrap();
        expected.extend(
            since
                .iter()
                .filter(|(added_by, _)| *added_by == migration.name)
                .map(|(_, trigger)| *trigger),
        );
        for trigger in &expected {
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
    assert_eq!(expected.len(), since.len(), "every trigger was added");
}
