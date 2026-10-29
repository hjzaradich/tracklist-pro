//! `file_waveform`, the waveform job's peaks (ROADMAP 1.11 "Audio";
//! 1cB-9).

use super::files::with_a_file;
use super::{accepts, db_at, one, refuses, MIGRATIONS};

fn computed(peaks: &str) -> String {
    format!(
        "INSERT INTO file_waveform (file_id, audio_hash, method_version, duration_ms, peaks)
         VALUES (1, zeroblob(34), 1, 1000, {peaks})"
    )
}

/// How many migrations come before this one: its index in [`MIGRATIONS`],
/// found by name so a renumbering at merge time changes nothing here.
fn before() -> usize {
    MIGRATIONS
        .iter()
        .position(|m| m.name.ends_with("_file_waveform.sql"))
        .unwrap()
}

#[test]
fn the_table_arrives_with_its_migration_and_not_before() {
    let has = |conn: &rusqlite::Connection| {
        one::<i64>(
            conn,
            "SELECT count(*) FROM sqlite_master WHERE name = 'file_waveform'",
        )
    };
    let (_dir, conn) = db_at(before());
    assert_eq!(has(&conn), 0);
    let (_dir, conn) = db_at(before() + 1);
    assert_eq!(has(&conn), 1);
}

#[test]
fn a_waveform_belongs_to_one_file_and_names_the_audio_and_method_it_came_from() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, &computed("x'0102'"));
    refuses(&conn, &computed("x'0102'"), "UNIQUE");
    refuses(
        &conn,
        "INSERT INTO file_waveform (file_id, audio_hash, method_version, failure)
         VALUES (99, zeroblob(34), 1, 'damaged')",
        "FOREIGN KEY",
    );
    for bad in ["zeroblob(32)", "zeroblob(0)", "NULL"] {
        refuses(
            &conn,
            &format!(
                "INSERT INTO file_waveform (file_id, audio_hash, method_version, failure)
                 VALUES (1, {bad}, 1, 'damaged')"
            ),
            if bad == "NULL" { "NOT NULL" } else { "CHECK" },
        );
    }
    refuses(
        &conn,
        "UPDATE file_waveform SET method_version = 0",
        "CHECK",
    );
}

#[test]
fn peaks_come_in_pairs_and_a_row_holds_either_peaks_or_the_reason_it_has_none() {
    let (_dir, conn) = with_a_file();
    // A peak and a level per bucket: an odd length or an empty array is wrong.
    refuses(&conn, &computed("x'010203'"), "CHECK");
    refuses(&conn, &computed("x''"), "CHECK");
    // Peaks need a duration, and a failure has neither.
    refuses(
        &conn,
        "INSERT INTO file_waveform (file_id, audio_hash, method_version, peaks)
         VALUES (1, zeroblob(34), 1, x'0102')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO file_waveform (file_id, audio_hash, method_version, duration_ms, failure)
         VALUES (1, zeroblob(34), 1, 1000, 'damaged')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO file_waveform (file_id, audio_hash, method_version, duration_ms, peaks, failure)
         VALUES (1, zeroblob(34), 1, 1000, x'0102', 'damaged')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO file_waveform (file_id, audio_hash, method_version) VALUES (1, zeroblob(34), 1)",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO file_waveform (file_id, audio_hash, method_version, failure)
         VALUES (1, zeroblob(34), 1, 'unreadable')",
        "CHECK",
    );
    accepts(
        &conn,
        "INSERT INTO file_waveform (file_id, audio_hash, method_version, failure)
         VALUES (1, zeroblob(34), 1, 'damaged')",
    );
}

#[test]
fn a_waveform_goes_with_its_file() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, &computed("x'0102'"));
    accepts(&conn, "DELETE FROM file");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM file_waveform"), 0);
}

#[test]
fn another_files_waveform_of_the_same_audio_is_found_by_an_index() {
    let (_dir, conn) = with_a_file();
    let plan: String = conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT peaks FROM file_waveform
             WHERE audio_hash = zeroblob(34) AND method_version = 1 AND file_id <> 1 LIMIT 1",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(plan.contains("file_waveform_audio"), "{plan}");
}
