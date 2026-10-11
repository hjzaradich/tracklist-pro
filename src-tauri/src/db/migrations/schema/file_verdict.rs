//! 0021: `file_verdict`, the stored quality verdict per file (ROADMAP 1.6;
//! 1bC-5).

use super::super::{run, MIGRATIONS};
use super::files::with_a_file;
use super::{accepts, columns, db_at, one, refuses};
use crate::quality::{settle, stored_verdict, verdict::Verdict};

fn verdict_row(set: &str, values: &str) -> String {
    format!(
        "INSERT INTO file_verdict (file_id, rules_version, inputs{set}) VALUES (1, 1, '[]'{values})"
    )
}

#[test]
fn a_verdict_belongs_to_one_file_and_names_the_rules_it_came_from() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, &verdict_row("", ""));
    refuses(&conn, &verdict_row("", ""), "UNIQUE");
    refuses(
        &conn,
        "INSERT INTO file_verdict (file_id, rules_version, inputs) VALUES (99, 1, '[]')",
        "FOREIGN KEY",
    );
    refuses(&conn, "UPDATE file_verdict SET rules_version = 0", "CHECK");
}

#[test]
fn a_verdict_is_deleted_with_its_file() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, &verdict_row(", verdict", ", 'ok'"));
    accepts(&conn, "DELETE FROM file");
    let rows: i64 = one(&conn, "SELECT COUNT(*) FROM file_verdict");
    assert_eq!(rows, 0);
}

#[test]
fn a_verdict_is_one_of_the_five_and_clipped_is_held_back() {
    let (_dir, conn) = with_a_file();
    for verdict in [
        "ok",
        "low_bitrate",
        "suspect_transcode",
        "truncated",
        "broken",
    ] {
        accepts(&conn, "DELETE FROM file_verdict");
        accepts(&conn, &verdict_row(", verdict", &format!(", '{verdict}'")));
    }
    for bad in ["clipped", "fine", ""] {
        refuses(
            &conn,
            &format!("UPDATE file_verdict SET verdict = '{bad}'"),
            "CHECK",
        );
    }
}

#[test]
fn findings_are_a_json_array_and_a_file_with_no_verdict_has_none() {
    let (_dir, conn) = with_a_file();
    accepts(&conn, &verdict_row("", ""));
    let findings: String = one(&conn, "SELECT findings FROM file_verdict");
    assert_eq!(findings, "[]");
    for bad in ["{}", "not json", "\"x\""] {
        refuses(
            &conn,
            &format!("UPDATE file_verdict SET findings = '{bad}'"),
            "CHECK",
        );
    }
    // No verdict, no findings.
    refuses(
        &conn,
        r#"UPDATE file_verdict SET findings = '[{"code":"lowBitrate","kbps":128}]'"#,
        "CHECK",
    );
    accepts(
        &conn,
        r#"UPDATE file_verdict SET verdict = 'low_bitrate',
                  findings = '[{"code":"lowBitrate","kbps":128}]'"#,
    );
}

#[test]
fn the_read_stages_column_is_still_on_file_and_the_verdict_has_its_own_table() {
    let (_dir, conn) = with_a_file();
    assert!(columns(&conn, "file").contains(&"quality_verdict".to_string()));
    assert!(!columns(&conn, "file_verdict").contains(&"quality_verdict".to_string()));
}

#[test]
fn files_that_exist_when_the_migration_runs_have_no_verdict_until_it_is_settled() {
    let (_dir, mut conn) = db_at(20);
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key, codec, bitrate, sample_rate)
         VALUES (1, 'low.mp3', 'low.mp3', 'mp3', 128, 44100);
         INSERT INTO file (music_folder_id, rel_path, rel_path_key, codec, bitrate, sample_rate,
                           quality_verdict)
         VALUES (1, 'cut.mp3', 'cut.mp3', 'mp3', 320, 44100, 'truncated');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'plain.bin', 'plain.bin');",
    );
    run(&mut conn, &MIGRATIONS[..21]).unwrap();

    // Nothing was invented for them.
    let rows: i64 = one(&conn, "SELECT COUNT(*) FROM file_verdict");
    assert_eq!(rows, 0);
    for id in 1..=3 {
        assert_eq!(
            stored_verdict(&conn, id).unwrap().verdict,
            None,
            "file {id}"
        );
    }
    // Settling gives each the verdict its stored values call for, and
    // leaves the read stage's column as it was.
    assert_eq!(settle(&mut conn).unwrap(), 2);
    assert_eq!(
        stored_verdict(&conn, 1).unwrap().verdict,
        Some(Verdict::LowBitrate)
    );
    assert_eq!(
        stored_verdict(&conn, 2).unwrap().verdict,
        Some(Verdict::Truncated)
    );
    assert_eq!(stored_verdict(&conn, 3).unwrap().verdict, None);
    let read: Option<String> = one(&conn, "SELECT quality_verdict FROM file WHERE id = 2");
    assert_eq!(read.as_deref(), Some("truncated"));
    let low: Option<String> = one(&conn, "SELECT quality_verdict FROM file WHERE id = 1");
    assert_eq!(low, None);
}
