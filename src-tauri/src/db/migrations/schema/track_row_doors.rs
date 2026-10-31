//! 0043: the doors' two stored answers in `track_row.flags`, and the triggers
//! that mark a track when something they read changes (1cD-R1a). What a
//! writer leaves in the row is tested in `search/tests/door_answers.rs`.

use rusqlite::Connection;

use super::{accepts, db, db_at};
use crate::db::migrations::{run, MIGRATIONS};
use crate::overview::overview_at;
use crate::quality::{store::INPUTS, RULES_VERSION};
use crate::search::tests::support::Mount;
use crate::search::{flag, settle};

/// How many migrations come before this one's.
fn before() -> usize {
    MIGRATIONS
        .iter()
        .position(|m| m.name.ends_with("_track_row_doors.sql"))
        .expect("the migration")
}

fn due(conn: &Connection) -> Vec<i64> {
    conn.prepare("SELECT recording_id FROM search_due ORDER BY 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Recordings 1 and 2 are Library tracks linked to files 1 and 2, recording 3
/// has file 3 and no Library track; recording 2 is a low-bitrate MP3.
const TRACKS: &str = "
    INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
    INSERT INTO recording DEFAULT VALUES;
    INSERT INTO recording DEFAULT VALUES;
    INSERT INTO recording DEFAULT VALUES;
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, codec, bitrate)
        VALUES (1, 'a.mp3', 'a.mp3', 'mp3', 320);
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, codec, bitrate)
        VALUES (1, 'b.mp3', 'b.mp3', 'mp3', 128);
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, codec, bitrate, raw_tags)
        VALUES (1, 'c.mp3', 'c.mp3', 'mp3', 128, '{\"id3v2\": []}');
    INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 1, 'best');
    INSERT INTO recording_file (recording_id, file_id, role) VALUES (2, 2, 'best');
    INSERT INTO recording_file (recording_id, file_id, role) VALUES (3, 3, 'best');
    INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
        VALUES (1, 'linked', 1, 'ok');
    INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
        VALUES (2, 'linked', 2, 'ok');
    DELETE FROM search_due;";

#[test]
fn an_upgrade_makes_every_library_track_due_and_no_other_track() {
    let (dir, conn) = db_at(before());
    accepts(&conn, TRACKS);
    drop(conn);
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    run(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(
        due(&conn),
        [1, 2],
        "the Library tracks, whose bits are unknown"
    );
}

#[test]
fn an_existing_database_gets_its_stored_answers_from_the_index_job_and_says_still_counting_until_then(
) {
    let (dir, conn) = db_at(before());
    accepts(&conn, TRACKS);
    // What an older build left: a current flagged verdict and energies, every
    // row written, nothing due, and the flags without the new bits.
    accepts(
        &conn,
        &format!(
            "INSERT INTO file_verdict (file_id, rules_version, inputs, verdict, findings)
             SELECT f.id, {RULES_VERSION}, {INPUTS}, 'low_bitrate', '[]'
             FROM file f LEFT JOIN file_quality q ON q.file_id = f.id WHERE f.id IN (2, 3);
             INSERT INTO analysis (recording_id, source, energy) VALUES (1, 'user', 7);
             INSERT INTO analysis (recording_id, source, energy) VALUES (3, 'user', 2);
             INSERT OR IGNORE INTO search_due (recording_id) SELECT id FROM recording;"
        ),
    );
    let mut conn = conn;
    settle(&mut conn, &Mount, 500).unwrap();
    accepts(
        &conn,
        &format!(
            "UPDATE track_row SET flags = flags & ~{};
             DELETE FROM search_due;",
            flag::LOW_QUALITY | flag::ENERGY
        ),
    );
    drop(conn);
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    run(&mut conn, MIGRATIONS).unwrap();

    let waiting = overview_at(&conn, 1_800_000_000_000).unwrap();
    assert!(waiting.counting.index, "still counting, not the old zeroes");

    settle(&mut conn, &Mount, 500).unwrap();

    let counted = overview_at(&conn, 1_800_000_000_000).unwrap();
    assert!(!counted.counting.index);
    assert_eq!(counted.low_quality, 1);
    assert_eq!(counted.energy[6], 1);
    assert_eq!(counted.no_energy, 1);
    let flags_of = |recording: i64| -> i64 {
        conn.query_row(
            "SELECT flags FROM track_row WHERE recording_id = ?1",
            [recording],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(flag::energy_of(flags_of(1)), 7);
    assert_eq!(flags_of(1) & flag::LOW_QUALITY, 0);
    assert_eq!(flag::energy_of(flags_of(2)), 0);
    assert_ne!(flags_of(2) & flag::LOW_QUALITY, 0);
    assert_eq!(
        flags_of(3) & (flag::LOW_QUALITY | flag::ENERGY),
        0,
        "a track outside the Library holds neither"
    );
}

#[test]
fn the_new_bits_fit_beside_the_others_and_none_is_used_twice() {
    let bits = [
        flag::IN_LIBRARY,
        flag::HAS_FILE,
        flag::MISSING,
        flag::LOW_QUALITY,
        flag::IN_REKORDBOX,
        flag::IN_CRATE,
        flag::MISSING_TAGS,
        flag::MISSING_ART,
        flag::ENERGY,
    ];
    let mut seen = 0;
    for bits in bits {
        assert_eq!(seen & bits, 0, "{bits} is taken");
        seen |= bits;
    }
    for level in 1..=10 {
        assert_eq!(flag::energy(level) & !flag::ENERGY, 0, "{level} fits");
        assert_eq!(flag::energy_of(flag::energy(level) | 1), i64::from(level));
    }
}

/// Runs each change in turn, each asserted to leave `due`, with nothing due
/// before it.
fn each(conn: &Connection, changes: &[(&str, &[i64])]) {
    for (change, expected) in changes {
        accepts(conn, "DELETE FROM search_due");
        accepts(conn, change);
        assert_eq!(due(conn), *expected, "{change}");
    }
}

const VERDICT: &str = "INSERT INTO file_verdict (file_id, rules_version, inputs, verdict)";

#[test]
fn a_verdict_that_may_flag_marks_its_library_track_and_a_healthy_one_nothing() {
    let (_dir, conn) = db();
    accepts(&conn, TRACKS);
    let (new_flagged, new_ok, new_nothing, outside) = (
        format!("{VERDICT} VALUES (2, 1, '[]', 'low_bitrate')"),
        format!("{VERDICT} VALUES (1, 1, '[]', 'ok')"),
        format!("{VERDICT} VALUES (1, 1, '[]', NULL)"),
        format!("{VERDICT} VALUES (3, 1, '[]', 'broken')"),
    );
    each(
        &conn,
        &[
            (new_nothing.as_str(), &[]),
            ("DELETE FROM file_verdict WHERE file_id = 1", &[]),
            (new_ok.as_str(), &[]),
            (outside.as_str(), &[]),
            (new_flagged.as_str(), &[2]),
            // Each column that makes it another answer, flagged before or after.
            (
                "UPDATE file_verdict SET rules_version = 2 WHERE file_id = 2",
                &[2],
            ),
            (
                "UPDATE file_verdict SET inputs = '[1]' WHERE file_id = 2",
                &[2],
            ),
            (
                "UPDATE file_verdict SET verdict = 'broken' WHERE file_id = 2",
                &[2],
            ),
            (
                "UPDATE file_verdict SET verdict = 'ok' WHERE file_id = 2",
                &[2],
            ),
            (
                "UPDATE file_verdict SET verdict = 'truncated' WHERE file_id = 2",
                &[2],
            ),
            // Never flagged either side: the answer is 0 before and after.
            (
                "UPDATE file_verdict SET verdict = NULL, inputs = '[2]' WHERE file_id = 1",
                &[],
            ),
            (
                "UPDATE file_verdict SET verdict = 'broken' WHERE file_id = 3",
                &[],
            ),
            (
                "UPDATE file_verdict SET findings = '[1]' WHERE file_id = 2",
                &[],
            ),
            ("DELETE FROM file_verdict WHERE file_id = 1", &[]),
            ("DELETE FROM file_verdict WHERE file_id = 3", &[]),
            ("DELETE FROM file_verdict WHERE file_id = 2", &[2]),
        ],
    );
}

#[test]
fn each_fact_of_a_verdict_marks_the_library_track_of_a_file_with_one_that_may_flag() {
    let (_dir, conn) = db();
    accepts(&conn, TRACKS);
    accepts(
        &conn,
        &format!(
            "{VERDICT} VALUES (2, 1, '[]', 'low_bitrate');
             {VERDICT} VALUES (1, 1, '[]', 'ok');
             {VERDICT} VALUES (3, 1, '[]', 'low_bitrate');"
        ),
    );
    each(
        &conn,
        &[
            ("UPDATE file SET audio_hash = x'0102' WHERE id = 2", &[2]),
            ("UPDATE file SET codec = 'aac' WHERE id = 2", &[2]),
            ("UPDATE file SET bitrate = 129 WHERE id = 2", &[2]),
            ("UPDATE file SET sample_rate = 44100 WHERE id = 2", &[2]),
            (
                "UPDATE file SET quality_verdict = 'broken' WHERE id = 2",
                &[2],
            ),
            // Nothing changed.
            (
                "UPDATE file SET bitrate = 129, codec = 'aac' WHERE id = 2",
                &[],
            ),
            // Not what a verdict reads.
            ("UPDATE file SET size = 10, mtime = 5 WHERE id = 2", &[]),
            // A healthy verdict, and a file no Library track links.
            ("UPDATE file SET bitrate = 256 WHERE id = 1", &[]),
            ("UPDATE file SET bitrate = 127 WHERE id = 3", &[]),
        ],
    );
}

#[test]
fn a_measurement_marks_the_library_track_of_a_file_with_a_verdict_that_may_flag() {
    let (_dir, conn) = db();
    accepts(&conn, TRACKS);
    accepts(
        &conn,
        &format!(
            "{VERDICT} VALUES (2, 1, '[]', 'low_bitrate');
             {VERDICT} VALUES (1, 1, '[]', 'ok');
             {VERDICT} VALUES (3, 1, '[]', 'low_bitrate');"
        ),
    );
    let measure = |file: i64| {
        format!(
            "INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_hz, decoded_ms,
                                       header_ms, ended)
             VALUES ({file}, zeroblob(34), 1, 20000, 1000, 1000, 'complete')"
        )
    };
    let (flagged, healthy, outside) = (measure(2), measure(1), measure(3));
    each(
        &conn,
        &[
            (flagged.as_str(), &[2]),
            (healthy.as_str(), &[]),
            (outside.as_str(), &[]),
            (
                "UPDATE file_quality SET cutoff_hz = 15000 WHERE file_id = 2",
                &[2],
            ),
            (
                "UPDATE file_quality SET cutoff_hz = 15000 WHERE file_id = 1",
                &[],
            ),
            ("DELETE FROM file_quality WHERE file_id = 2", &[2]),
            ("DELETE FROM file_quality WHERE file_id = 1", &[]),
        ],
    );
}

#[test]
fn an_energy_marks_a_library_track_and_nothing_else_about_analysis_does() {
    let (_dir, conn) = db();
    accepts(&conn, TRACKS);
    each(
        &conn,
        &[
            (
                "INSERT INTO analysis (recording_id, source, energy) VALUES (1, 'user', 5)",
                &[1],
            ),
            (
                "UPDATE analysis SET energy = 6 WHERE recording_id = 1",
                &[1],
            ),
            ("UPDATE analysis SET energy = 6 WHERE recording_id = 1", &[]),
            (
                "UPDATE analysis SET confidence = 0.5 WHERE recording_id = 1",
                &[],
            ),
            ("DELETE FROM analysis WHERE recording_id = 1", &[1]),
            // Outside the Library no row holds it.
            (
                "INSERT INTO analysis (recording_id, source, energy) VALUES (3, 'mik', 5)",
                &[],
            ),
            ("UPDATE analysis SET energy = 6 WHERE recording_id = 3", &[]),
            ("DELETE FROM analysis WHERE recording_id = 3", &[]),
            // A row with no energy is `search_analysis_*` of 0039 alone.
            (
                "INSERT INTO analysis (recording_id, source, key) VALUES (3, 'tag', '8A')",
                &[3],
            ),
            ("DELETE FROM analysis WHERE recording_id = 3", &[3]),
            // A row that moves is that too, and takes its energy along.
            (
                "INSERT INTO analysis (recording_id, source, energy) VALUES (1, 'user', 5)",
                &[1],
            ),
            (
                "UPDATE analysis SET recording_id = 2 WHERE recording_id = 1",
                &[1, 2],
            ),
        ],
    );
}
