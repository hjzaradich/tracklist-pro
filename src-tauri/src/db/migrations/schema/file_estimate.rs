//! `file_estimate`, the analyze job's measurements (ROADMAP 3.7 "Local
//! estimates"; 3C-E1a).

use super::files::with_a_file;
use super::{accepts, db_at, one, refuses, MIGRATIONS};

fn measured(columns: &str, values: &str) -> String {
    format!(
        "INSERT INTO file_estimate (file_id, audio_hash, method_version, analysed_ms{columns})
         VALUES (1, zeroblob(34), 1, 120000{values})"
    )
}

fn skipped(why: &str) -> String {
    format!(
        "INSERT INTO file_estimate (file_id, audio_hash, method_version, skipped)
         VALUES (1, zeroblob(34), 1, '{why}')"
    )
}

/// Its index in [`MIGRATIONS`], found by name so a renumbering at merge
/// time changes nothing here.
fn before() -> usize {
    MIGRATIONS
        .iter()
        .position(|m| m.name.ends_with("_file_estimate.sql"))
        .unwrap()
}

#[test]
fn the_table_arrives_with_its_migration_and_not_before() {
    let has = |conn: &rusqlite::Connection| {
        one::<i64>(
            conn,
            "SELECT count(*) FROM sqlite_master WHERE name = 'file_estimate'",
        )
    };
    let (_dir, conn) = db_at(before());
    assert_eq!(has(&conn), 0);
    let (_dir, conn) = db_at(before() + 1);
    assert_eq!(has(&conn), 1);
}

#[test]
fn an_estimate_belongs_to_one_file_and_names_the_audio_and_method_it_came_from() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, &measured(", bpm, bpm_confidence", ", 128.0, 0.8"));
    refuses(&conn, &measured("", ""), "UNIQUE");
    refuses(
        &conn,
        "INSERT INTO file_estimate (file_id, audio_hash, method_version, skipped)
         VALUES (99, zeroblob(34), 1, 'silent')",
        "FOREIGN KEY",
    );
    refuses(&conn, "UPDATE file_estimate SET method_version = 0", "CHECK");
    refuses(&conn, "UPDATE file_estimate SET audio_hash = zeroblob(32)", "CHECK");
}

#[test]
fn a_bpm_comes_with_its_confidence_and_a_skipped_file_has_no_measurements() {
    let (_dir, conn) = with_a_file();
    refuses(&conn, &measured(", bpm", ", 128.0"), "CHECK");
    refuses(&conn, &measured(", bpm, bpm_confidence", ", 128.0, 1.5"), "CHECK");
    refuses(&conn, &measured(", bpm, bpm_confidence", ", 0, 0.5"), "CHECK");
    refuses(&conn, &measured(", energy_suggestion", ", 11"), "CHECK");
    refuses(&conn, &skipped("unreadable"), "CHECK");
    // Neither a window nor a reason.
    refuses(
        &conn,
        "INSERT INTO file_estimate (file_id, audio_hash, method_version)
         VALUES (1, zeroblob(34), 1)",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO file_estimate (file_id, audio_hash, method_version, skipped, bpm, bpm_confidence)
         VALUES (1, zeroblob(34), 1, 'silent', 120.0, 0.5)",
        "CHECK",
    );
    // A window with no beat the detector trusts: no BPM, no reason.
    accepts(&conn, &measured("", ""));
    accepts(&conn, "DELETE FROM file_estimate");
    for why in ["too_short", "silent", "damaged"] {
        accepts(&conn, &skipped(why));
        accepts(&conn, "DELETE FROM file_estimate");
    }
}

#[test]
fn an_estimate_goes_with_its_file_and_the_same_audio_is_found_by_an_index() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, &skipped("silent"));
    let plan: String = conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT bpm FROM file_estimate
             WHERE audio_hash = zeroblob(34) AND method_version = 1 AND file_id <> 1 LIMIT 1",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(plan.contains("file_estimate_audio"), "{plan}");
    accepts(&conn, "DELETE FROM file");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM file_estimate"), 0);
}
