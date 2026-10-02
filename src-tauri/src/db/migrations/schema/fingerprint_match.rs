//! 0016: `fingerprint_match`, what comparing two files' fingerprints found
//! (1bA-2; ROADMAP 1.4, 5.1).

use rusqlite::Connection;

use super::files::with_a_file;
use super::{accepts, one, refuses};

const INSERT: &str = "INSERT INTO fingerprint_match
    (file_a, file_b, version, items_a, items_b, coverage_a, coverage_b, score, segments) VALUES";

/// Files 1, 2 and 3, each with a fingerprint, and a result for every pair.
fn with_three_compared_files() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = with_a_file();
    accepts(
        &conn,
        &format!(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'b.mp3', 'b.mp3');
             INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'c.mp3', 'c.mp3');
             UPDATE file SET fingerprint = x'0102', size = 10, mtime = 100;
             {INSERT} (1, 2, 1, 100, 100, 1.0, 1.0, 0.5, '[[0,0,100,0.5,0]]');
             {INSERT} (1, 3, 1, 100, 80, 0.0, 0.0, 32.0, '[]');
             {INSERT} (2, 3, 1, 100, 80, 0.8, 1.0, 1.5, '[[20,0,80,1.5,0]]');"
        ),
    );
    (dir, conn)
}

fn pairs(conn: &Connection) -> Vec<(i64, i64)> {
    let mut stmt = conn
        .prepare("SELECT file_a, file_b FROM fingerprint_match ORDER BY file_a, file_b")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows
}

#[test]
fn a_pair_of_files_has_one_result_with_the_lower_file_first() {
    let (_dir, conn) = with_three_compared_files();
    refuses(
        &conn,
        &format!("{INSERT} (1, 2, 1, 100, 100, 1.0, 1.0, 0.0, '[]')"),
        "UNIQUE",
    );
    // The other way round, or a file with itself, is never stored.
    for (a, b) in [(2, 1), (1, 1)] {
        refuses(
            &conn,
            &format!("{INSERT} ({a}, {b}, 1, 100, 100, 1.0, 1.0, 0.0, '[]')"),
            "CHECK",
        );
    }
}

#[test]
fn a_result_is_between_known_files_and_goes_when_either_file_row_goes() {
    let (_dir, conn) = with_three_compared_files();
    refuses(
        &conn,
        &format!("{INSERT} (1, 9, 1, 100, 100, 1.0, 1.0, 0.0, '[]')"),
        "FOREIGN KEY",
    );
    accepts(&conn, "DELETE FROM file WHERE id = 3");
    assert_eq!(pairs(&conn), [(1, 2)]);
    accepts(&conn, "DELETE FROM file WHERE id = 1");
    assert_eq!(pairs(&conn), []);
}

#[test]
fn coverage_is_a_share_the_score_is_0_to_32_bits_and_segments_are_a_json_list() {
    let (_dir, conn) = with_three_compared_files();
    accepts(&conn, "DELETE FROM fingerprint_match");
    let row = |coverage_a: &str, score: &str, segments: &str, version: &str| {
        format!("{INSERT} (1, 2, {version}, 100, 100, {coverage_a}, 1.0, {score}, '{segments}')")
    };
    accepts(&conn, &row("0.0", "32.0", "[]", "1"));
    accepts(&conn, "DELETE FROM fingerprint_match");
    for bad in [
        row("1.5", "0.0", "[]", "1"),
        row("-0.1", "0.0", "[]", "1"),
        row("1.0", "33.0", "[]", "1"),
        row("1.0", "-1.0", "[]", "1"),
        row("1.0", "0.0", "{}", "1"),
        row("1.0", "0.0", "not json", "1"),
        row("1.0", "0.0", "[]", "0"),
    ] {
        refuses(&conn, &bad, "CHECK");
    }
}

#[test]
fn a_changed_fingerprint_drops_that_files_results_and_no_others() {
    let (_dir, conn) = with_three_compared_files();
    accepts(&conn, "UPDATE file SET fingerprint = x'0103' WHERE id = 3");
    assert_eq!(pairs(&conn), [(1, 2)]);
    // A fingerprint taken away (the file can't be fingerprinted any more)
    // is a change too.
    accepts(&conn, "UPDATE file SET fingerprint = NULL WHERE id = 2");
    assert_eq!(pairs(&conn), []);
}

#[test]
fn a_changed_fingerprint_drops_the_results_where_the_file_is_the_first_of_the_pair() {
    let (_dir, conn) = with_three_compared_files();
    accepts(&conn, "UPDATE file SET fingerprint = x'0103' WHERE id = 1");
    assert_eq!(pairs(&conn), [(2, 3)]);
}

#[test]
fn a_new_size_or_modified_time_or_the_same_fingerprint_written_again_keeps_the_results() {
    // rekordbox rewrites tags and bumps modified times (ROADMAP 5.1): none
    // of that is new audio.
    let (_dir, conn) = with_three_compared_files();
    accepts(
        &conn,
        "UPDATE file SET size = 11, mtime = 200, present = 0;
         UPDATE file SET fingerprint = x'0102';
         UPDATE file SET present = 1",
    );
    assert_eq!(pairs(&conn).len(), 3);
    let fingerprints: i64 = one(
        &conn,
        "SELECT COUNT(*) FROM file WHERE fingerprint = x'0102'",
    );
    assert_eq!(fingerprints, 3);
}
