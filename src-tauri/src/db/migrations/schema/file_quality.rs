//! 0017: `file_quality`, the quality job's measurements (ROADMAP 1.6), and
//! the `file.cutoff_hz` column it replaces.

use super::files::with_a_file;
use super::{accepts, columns, one, refuses};

fn measured(set: &str, values: &str) -> String {
    format!(
        "INSERT INTO file_quality (file_id, audio_hash, method_version, decoded_ms, ended{set})
         VALUES (1, zeroblob(34), 1, 1000, 'complete'{values})"
    )
}

#[test]
fn a_measurement_belongs_to_one_file_and_names_the_audio_and_method_it_came_from() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, &measured(", cutoff_hz", ", 16000"));
    // One row per file.
    refuses(&conn, &measured(", cutoff_hz", ", 16000"), "UNIQUE");
    // A file that isn't there.
    refuses(
        &conn,
        "INSERT INTO file_quality (file_id, audio_hash, method_version, failure)
         VALUES (99, zeroblob(34), 1, 'damaged')",
        "FOREIGN KEY",
    );
    // The audio hash is the 34-byte form, and required.
    for bad in ["zeroblob(32)", "zeroblob(0)", "NULL"] {
        refuses(
            &conn,
            &format!(
                "INSERT INTO file_quality (file_id, audio_hash, method_version, failure)
                 VALUES (1, {bad}, 1, 'damaged')"
            ),
            if bad == "NULL" { "NOT NULL" } else { "CHECK" },
        );
    }
    // The method version is positive.
    refuses(&conn, "UPDATE file_quality SET method_version = 0", "CHECK");
}

#[test]
fn a_measurement_is_deleted_with_its_file() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, &measured(", cutoff_hz", ", 16000"));
    accepts(&conn, "DELETE FROM file");
    let rows: i64 = one(&conn, "SELECT COUNT(*) FROM file_quality");
    assert_eq!(rows, 0);
}

#[test]
fn a_cutoff_is_a_positive_frequency_or_a_reason_there_is_none() {
    let (_dir, conn) = with_a_file();
    refuses(&conn, &measured(", cutoff_hz", ", 0"), "CHECK");
    refuses(&conn, &measured(", cutoff_hz", ", -5"), "CHECK");
    // A decoded file has a cutoff or the reason it has none, never both and
    // never neither.
    refuses(&conn, &measured("", ""), "CHECK");
    refuses(
        &conn,
        &measured(", cutoff_hz, cutoff_gap", ", 16000, 'silent'"),
        "CHECK",
    );
    refuses(&conn, &measured(", cutoff_gap", ", 'unknown'"), "CHECK");
    for gap in ["too_short", "silent", "low_rate"] {
        accepts(&conn, "DELETE FROM file_quality");
        accepts(&conn, &measured(", cutoff_gap", &format!(", '{gap}'")));
    }
}

#[test]
fn durations_are_not_negative_and_the_header_may_say_nothing() {
    let (_dir, conn) = with_a_file();
    refuses(
        &conn,
        &measured(", cutoff_hz, header_ms", ", 16000, -1"),
        "CHECK",
    );
    accepts(&conn, &measured(", cutoff_hz", ", 16000"));
    let header: Option<i64> = one(&conn, "SELECT header_ms FROM file_quality");
    assert_eq!(header, None);
    refuses(&conn, "UPDATE file_quality SET decoded_ms = -1", "CHECK");
}

#[test]
fn how_a_stream_ended_is_one_of_three_ways() {
    let (_dir, conn) = with_a_file();
    for ended in ["complete", "cut_short", "gave_up"] {
        accepts(&conn, "DELETE FROM file_quality");
        accepts(
            &conn,
            &format!(
                "INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_hz, decoded_ms, ended)
                 VALUES (1, zeroblob(34), 1, 16000, 1000, '{ended}')"
            ),
        );
    }
    refuses(&conn, "UPDATE file_quality SET ended = 'finished'", "CHECK");
    // A decoded file always says how it ended.
    refuses(&conn, "UPDATE file_quality SET ended = NULL", "CHECK");
}

#[test]
fn decode_errors_are_a_count_and_a_json_object_of_counts_by_kind() {
    let (_dir, conn) = with_a_file();
    accepts(
        &conn,
        &measured(
            ", cutoff_hz, decode_errors, error_kinds",
            r#", 16000, 3, '{"decode": 2, "container": 1}'"#,
        ),
    );
    let zero: i64 = one(
        &conn,
        "SELECT COUNT(*) FROM file_quality WHERE decode_errors = 3",
    );
    assert_eq!(zero, 1);
    refuses(&conn, "UPDATE file_quality SET decode_errors = -1", "CHECK");
    refuses(
        &conn,
        "UPDATE file_quality SET error_kinds = 'nope'",
        "CHECK",
    );
    refuses(
        &conn,
        "UPDATE file_quality SET error_kinds = '[1]'",
        "CHECK",
    );
    accepts(
        &conn,
        "UPDATE file_quality SET error_kinds = NULL, decode_errors = 0",
    );
    // The default is no errors.
    accepts(&conn, "DELETE FROM file_quality");
    accepts(&conn, &measured(", cutoff_hz", ", 16000"));
    let errors: i64 = one(&conn, "SELECT decode_errors FROM file_quality");
    assert_eq!(errors, 0);
}

#[test]
fn a_file_that_cannot_be_decoded_is_a_failure_with_no_measurements() {
    let (_dir, conn) = with_a_file();
    for why in [
        "unsupported_format",
        "unsupported_codec",
        "no_audio",
        "damaged",
        "decoder_crashed",
    ] {
        accepts(&conn, "DELETE FROM file_quality");
        accepts(
            &conn,
            &format!(
                "INSERT INTO file_quality (file_id, audio_hash, method_version, failure)
                 VALUES (1, zeroblob(34), 1, '{why}')"
            ),
        );
    }
    refuses(
        &conn,
        "UPDATE file_quality SET failure = 'unreadable'",
        "CHECK",
    );
    // Not alongside a measurement.
    for set in [
        "cutoff_hz = 16000",
        "decoded_ms = 5",
        "header_ms = 5",
        "decode_errors = 1",
        "ended = 'complete'",
    ] {
        refuses(&conn, &format!("UPDATE file_quality SET {set}"), "CHECK");
    }
}

#[test]
fn a_row_with_neither_a_failure_nor_durations_is_refused() {
    let (_dir, conn) = with_a_file();
    refuses(
        &conn,
        "INSERT INTO file_quality (file_id, audio_hash, method_version) VALUES (1, zeroblob(34), 1)",
        "CHECK",
    );
}

#[test]
fn the_cutoff_has_one_home_and_it_is_not_the_file_table() {
    let (_dir, conn) = with_a_file();
    let file = columns(&conn, "file");
    assert!(!file.contains(&"cutoff_hz".to_string()), "{file:?}");
    // The verdict stays on the file, for 1bB-6.
    assert!(file.contains(&"quality_verdict".to_string()));
    assert!(columns(&conn, "file_quality").contains(&"cutoff_hz".to_string()));
}
