//! 0023: `best_file_due`, `better_file` and the triggers that make a track
//! due for its files to be ranked again (ROADMAP 1.7; 1bC-4).
//!
//! One test per trigger: it fires on the change it exists for, and not on
//! an update of something the ranking doesn't read.

use rusqlite::Connection;

use super::super::{run, MIGRATIONS};
use super::{accepts, db, db_at, one, refuses};

/// Two tracks. Track 1 holds files 1 and 2, track 2 holds file 3. Nothing
/// is due yet.
fn with_two_tracks() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'DJ', 'DJ');
         INSERT INTO recording DEFAULT VALUES;
         INSERT INTO recording DEFAULT VALUES;
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'b.mp3', 'b.mp3');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'c.mp3', 'c.mp3');
         INSERT INTO recording_file (recording_id, file_id) VALUES (1, 1);
         INSERT INTO recording_file (recording_id, file_id) VALUES (1, 2);
         INSERT INTO recording_file (recording_id, file_id) VALUES (2, 3);
         DELETE FROM best_file_due;",
    );
    (dir, conn)
}

/// The tracks that are due, and nothing else is changed.
fn due(conn: &Connection) -> Vec<i64> {
    let mut stmt = conn
        .prepare("SELECT recording_id FROM best_file_due ORDER BY recording_id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows
}

fn clear(conn: &Connection) {
    accepts(conn, "DELETE FROM best_file_due");
}

/// Runs `sql` and says which tracks became due by it.
fn due_after(conn: &Connection, sql: &str) -> Vec<i64> {
    clear(conn);
    accepts(conn, sql);
    due(conn)
}

#[test]
fn a_file_placed_in_a_track_makes_that_track_due() {
    let (_dir, conn) = with_two_tracks();
    let sql = "INSERT INTO file (music_folder_id, rel_path, rel_path_key)
               VALUES (1, 'd.mp3', 'd.mp3');
               INSERT INTO recording_file (recording_id, file_id) VALUES (2, 4);";
    assert_eq!(due_after(&conn, sql), vec![2]);
}

#[test]
fn a_file_added_to_the_file_table_alone_makes_no_track_due() {
    let (_dir, conn) = with_two_tracks();
    let sql = "INSERT INTO file (music_folder_id, rel_path, rel_path_key)
               VALUES (1, 'd.mp3', 'd.mp3')";
    assert_eq!(due_after(&conn, sql), Vec::<i64>::new());
}

#[test]
fn a_file_let_go_of_makes_its_track_due() {
    let (_dir, conn) = with_two_tracks();
    assert_eq!(
        due_after(&conn, "DELETE FROM recording_file WHERE file_id = 2"),
        vec![1]
    );
}

#[test]
fn a_file_moved_between_tracks_makes_both_tracks_due() {
    let (_dir, conn) = with_two_tracks();
    assert_eq!(
        due_after(
            &conn,
            "UPDATE recording_file SET recording_id = 2 WHERE file_id = 2"
        ),
        vec![1, 2]
    );
}

#[test]
fn a_role_written_on_a_file_of_a_track_makes_nothing_due() {
    let (_dir, conn) = with_two_tracks();
    let sql = "UPDATE recording_file SET role = 'best' WHERE file_id = 1;
               UPDATE recording_file SET role = 'extra' WHERE file_id = 2;
               UPDATE recording_file SET role = 'undecided', match_confidence = 0.5
               WHERE file_id = 2;";
    assert_eq!(due_after(&conn, sql), Vec::<i64>::new());
}

#[test]
fn a_file_that_changes_in_a_way_the_ranking_reads_makes_its_track_due() {
    let (_dir, conn) = with_two_tracks();
    for change in [
        "present = 0",
        "sniffed_format = 'flac'",
        "codec = 'flac'",
        "bitrate = 320",
        "sample_rate = 44100",
        "audio_hash = x'AA'",
        "quality_verdict = 'truncated'",
        "raw_tags = '{}'",
    ] {
        // Back to the start each time, so every change is a change.
        accepts(
            &conn,
            "UPDATE file SET present = 1, sniffed_format = NULL, codec = NULL, bitrate = NULL,
                             sample_rate = NULL, audio_hash = NULL, quality_verdict = NULL,
                             raw_tags = NULL WHERE id = 2",
        );
        assert_eq!(
            due_after(&conn, &format!("UPDATE file SET {change} WHERE id = 2")),
            vec![1],
            "{change}"
        );
    }
}

#[test]
fn a_file_moved_to_another_music_folder_makes_its_track_due() {
    let (_dir, conn) = with_two_tracks();
    accepts(
        &conn,
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Other', 'Other')",
    );
    assert_eq!(
        due_after(&conn, "UPDATE file SET music_folder_id = 2 WHERE id = 3"),
        vec![2]
    );
}

#[test]
fn an_update_that_leaves_what_the_ranking_reads_as_it_was_makes_nothing_due() {
    let (_dir, conn) = with_two_tracks();
    accepts(
        &conn,
        "UPDATE file SET present = 1, codec = 'mp3', bitrate = 320, raw_tags = '{}' WHERE id = 2",
    );
    // What a walk does to every file it sees: write the same facts again.
    let sql = "UPDATE file SET present = 1, codec = 'mp3', bitrate = 320, raw_tags = '{}',
                               size = 10, mtime = 5, last_seen_at = '2026-10-11T10:00:00.000Z'
               WHERE id = 2;
               UPDATE file SET duration_ms = 1000, fingerprint = x'01', blake3 = zeroblob(32),
                               hidden_reason = 'user' WHERE id = 3;";
    assert_eq!(due_after(&conn, sql), Vec::<i64>::new());
}

/// A measurement that decoded, with a cutoff of 15 kHz.
fn quality_row(file: i64) -> String {
    format!(
        "INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_hz, decoded_ms,
                                  header_ms, ended)
         VALUES ({file}, zeroblob(34), 1, 15000, 1000, 1000, 'complete')"
    )
}

#[test]
fn a_measurement_landing_changing_or_going_makes_its_tracks_due() {
    let (_dir, conn) = with_two_tracks();
    assert_eq!(due_after(&conn, &quality_row(2)), vec![1]);
    // Each change from the row above (or from the one named), one column at
    // a time.
    let from_gap = "DELETE FROM file_quality;
        INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_gap, decoded_ms,
                                  header_ms, ended)
        VALUES (2, zeroblob(34), 1, 'silent', 1000, 1000, 'complete')";
    let from_failure = "DELETE FROM file_quality;
        INSERT INTO file_quality (file_id, audio_hash, method_version, failure)
        VALUES (2, zeroblob(34), 1, 'no_audio')";
    for (base, change) in [
        (
            None,
            "audio_hash = x'01010101010101010101010101010101010101010101010101010101010101010101'",
        ),
        (None, "method_version = 2"),
        (None, "cutoff_hz = 16000"),
        (None, "decoded_ms = 2000"),
        (None, "header_ms = 2000"),
        (None, "decode_errors = 3"),
        (None, "error_kinds = '{\"decode\": 3}'"),
        (None, "ended = 'cut_short'"),
        (Some(from_gap), "cutoff_gap = 'too_short'"),
        (Some(from_failure), "failure = 'damaged'"),
    ] {
        accepts(&conn, "DELETE FROM file_quality");
        match base {
            Some(sql) => accepts(&conn, sql),
            None => accepts(&conn, &quality_row(2)),
        }
        assert_eq!(
            due_after(
                &conn,
                &format!("UPDATE file_quality SET {change} WHERE file_id = 2")
            ),
            vec![1],
            "{change}"
        );
    }
    assert_eq!(due_after(&conn, "DELETE FROM file_quality"), vec![1]);
}

#[test]
fn a_measurement_rewritten_with_the_same_numbers_makes_nothing_due() {
    let (_dir, conn) = with_two_tracks();
    accepts(&conn, &quality_row(2));
    let sql = "UPDATE file_quality SET cutoff_hz = cutoff_hz, decode_errors = 0,
                                       measured_at = '2026-10-11T10:00:00.000Z'";
    assert_eq!(due_after(&conn, sql), Vec::<i64>::new());
}

fn verdict_row(file: i64, verdict: &str) -> String {
    format!(
        "INSERT INTO file_verdict (file_id, rules_version, inputs, verdict)
         VALUES ({file}, 1, '[]', {verdict})"
    )
}

#[test]
fn a_verdict_stored_changed_or_dropped_makes_its_track_due() {
    let (_dir, conn) = with_two_tracks();
    assert_eq!(due_after(&conn, &verdict_row(2, "'ok'")), vec![1]);
    assert_eq!(
        due_after(&conn, "UPDATE file_verdict SET verdict = 'truncated'"),
        vec![1]
    );
    assert_eq!(
        due_after(
            &conn,
            "UPDATE file_verdict SET verdict = 'low_bitrate', findings = '[{\"code\": \"x\"}]'"
        ),
        vec![1]
    );
    assert_eq!(due_after(&conn, "DELETE FROM file_verdict"), vec![1]);
}

#[test]
fn a_verdict_that_says_nothing_makes_nothing_due() {
    let (_dir, conn) = with_two_tracks();
    // A row with no verdict reads as no row.
    assert_eq!(due_after(&conn, &verdict_row(2, "NULL")), Vec::<i64>::new());
    assert_eq!(
        due_after(
            &conn,
            "UPDATE file_verdict SET assessed_at = '2026-10-11T10:00:00.000Z', rules_version = 2,
                                    inputs = '[1]'"
        ),
        Vec::<i64>::new()
    );
    assert_eq!(
        due_after(&conn, "DELETE FROM file_verdict"),
        Vec::<i64>::new()
    );
}

fn rekordbox_row(track_id: i64, file: &str, probable: bool) -> String {
    let method = if file == "NULL" { "NULL" } else { "'path'" };
    format!(
        "INSERT INTO rekordbox_track
             (attributes, location_key, read_at, file_id, relink_method, relink_probable)
         VALUES ('{{\"TrackID\": \"{track_id}\", \"Location\": \"file://localhost/E:/{track_id}.mp3\"}}',
                 'E:/{track_id}.mp3', '2026-10-11T10:00:00.000Z', {file}, {method}, {})",
        u8::from(probable)
    )
}

#[test]
fn a_rekordbox_match_made_changed_or_dropped_makes_the_tracks_of_its_files_due() {
    let (_dir, conn) = with_two_tracks();
    assert_eq!(due_after(&conn, &rekordbox_row(1, "2", false)), vec![1]);
    assert_eq!(
        due_after(&conn, "UPDATE rekordbox_track SET relink_probable = 1"),
        vec![1]
    );
    assert_eq!(
        due_after(
            &conn,
            "UPDATE rekordbox_track SET file_id = 3, relink_probable = 0"
        ),
        vec![1, 2],
        "the track it left and the one it joined"
    );
    assert_eq!(due_after(&conn, "DELETE FROM rekordbox_track"), vec![2]);
}

#[test]
fn a_rekordbox_row_with_no_match_makes_nothing_due() {
    let (_dir, conn) = with_two_tracks();
    assert_eq!(
        due_after(&conn, &rekordbox_row(1, "NULL", false)),
        Vec::<i64>::new()
    );
    assert_eq!(
        due_after(
            &conn,
            "UPDATE rekordbox_track SET recording_id = 1, relink_confidence = 0.5"
        ),
        Vec::<i64>::new()
    );
    assert_eq!(
        due_after(&conn, "DELETE FROM rekordbox_track"),
        Vec::<i64>::new()
    );
}

#[test]
fn a_track_entering_leaving_or_relinked_in_the_library_is_due() {
    let (_dir, conn) = with_two_tracks();
    assert_eq!(
        due_after(
            &conn,
            "INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 1)"
        ),
        vec![1]
    );
    assert_eq!(
        due_after(&conn, "UPDATE library_track SET linked_file_id = 2"),
        vec![1]
    );
    assert_eq!(due_after(&conn, "DELETE FROM library_track"), vec![1]);
}

#[test]
fn a_library_track_updated_in_a_way_the_ranking_ignores_makes_nothing_due() {
    let (_dir, conn) = with_two_tracks();
    accepts(
        &conn,
        "INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 1)",
    );
    let sql = "UPDATE library_track SET source_status = 'missing',
                                         last_sent_location = 'E:/a.mp3',
                                         last_exported_at = '2026-10-11T10:00:00.000Z'";
    assert_eq!(due_after(&conn, sql), Vec::<i64>::new());
}

#[test]
fn a_music_folder_changing_role_or_volume_makes_the_tracks_of_its_files_due() {
    let (_dir, conn) = with_two_tracks();
    assert_eq!(
        due_after(&conn, "UPDATE music_folder SET role = 'inbox'"),
        vec![1, 2]
    );
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-5E6F7A8B', 'internal')",
    );
    assert_eq!(
        due_after(&conn, "UPDATE music_folder SET volume_id = 2"),
        vec![1, 2]
    );
}

#[test]
fn a_music_folder_changing_in_a_way_the_ranking_ignores_makes_nothing_due() {
    let (_dir, conn) = with_two_tracks();
    let sql =
        "UPDATE music_folder SET watch = 1, role = role, walked_at = '2026-10-11T10:00:00.000Z'";
    assert_eq!(due_after(&conn, sql), Vec::<i64>::new());
}

#[test]
fn a_volume_changing_kind_makes_the_tracks_of_its_files_due() {
    let (_dir, conn) = with_two_tracks();
    assert_eq!(
        due_after(&conn, "UPDATE volume SET kind = 'internal'"),
        vec![1, 2]
    );
}

#[test]
fn a_volume_changing_in_a_way_the_ranking_ignores_makes_nothing_due() {
    let (_dir, conn) = with_two_tracks();
    let sql = "UPDATE volume SET label = 'Stick', last_mount_path = 'F:\\',
                                 last_seen_at = '2026-10-11T10:00:00.000Z', kind = kind";
    assert_eq!(due_after(&conn, sql), Vec::<i64>::new());
}

#[test]
fn a_track_that_already_exists_when_the_migration_runs_is_due() {
    let (_dir, mut conn) = db_at(22);
    accepts(
        &conn,
        "INSERT INTO recording DEFAULT VALUES; INSERT INTO recording DEFAULT VALUES;",
    );
    run(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(due(&conn), vec![1, 2]);
}

#[test]
fn a_track_is_due_once_however_many_things_change() {
    let (_dir, conn) = with_two_tracks();
    accepts(
        &conn,
        "UPDATE file SET present = 0 WHERE id = 1;
         UPDATE file SET present = 0 WHERE id = 2;
         UPDATE recording_file SET recording_id = 1 WHERE file_id = 1;",
    );
    assert_eq!(due(&conn), vec![1]);
}

#[test]
fn a_better_file_names_an_audio_step_and_goes_with_its_track_and_its_file() {
    let (_dir, conn) = with_two_tracks();
    accepts(
        &conn,
        "INSERT INTO better_file (recording_id, file_id, step) VALUES (1, 2, 'format')",
    );
    for step in [
        "onDisk",
        "verdict",
        "playable",
        "cutoff",
        "decodeErrors",
        "format",
    ] {
        accepts(&conn, &format!("UPDATE better_file SET step = '{step}'"));
    }
    // Tags, art, a rekordbox link and a better place don't make a file
    // better: the table refuses them.
    for step in ["tags", "art", "rekordbox", "place", "tied", "onlyFile", ""] {
        refuses(
            &conn,
            &format!("UPDATE better_file SET step = '{step}'"),
            "CHECK",
        );
    }
    refuses(
        &conn,
        "INSERT INTO better_file (recording_id, file_id, step) VALUES (1, 1, 'format')",
        "UNIQUE",
    );
    accepts(&conn, "DELETE FROM recording_file WHERE file_id = 2");
    accepts(&conn, "DELETE FROM file WHERE id = 2");
    let rows: i64 = one(&conn, "SELECT COUNT(*) FROM better_file");
    assert_eq!(rows, 0, "deleted with its file");
}
