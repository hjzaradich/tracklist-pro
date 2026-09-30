//! 0003: `recording`, `recording_file`, `version_link`, `analysis` and the
//! `recording_display` view (ROADMAP §1.2, §2).

use rusqlite::Connection;

use super::files::with_a_file;
use super::{accepts, columns, one, refuses};

/// Three files (ids 1–3) and three tracks (ids 1–3), no links yet. File 1
/// belongs to track 1.
pub(in crate::db::migrations) fn with_tracks() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = with_a_file();
    accepts(
        &conn,
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'b.mp3', 'b.mp3');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'c.mp3', 'c.mp3');
         INSERT INTO recording (title) VALUES ('Track 1'), ('Track 2'), ('Track 3');
         INSERT INTO recording_file (recording_id, file_id) VALUES (1, 1);",
    );
    (dir, conn)
}

// Files and duplicates.

#[test]
fn a_file_holds_exactly_one_track() {
    let (_dir, conn) = with_tracks();
    refuses(
        &conn,
        "INSERT INTO recording_file (recording_id, file_id) VALUES (2, 1)",
        "UNIQUE",
    );
}

#[test]
fn a_track_can_hold_many_files_but_only_one_best_file() {
    let (_dir, conn) = with_tracks();
    accepts(
        &conn,
        "INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 2, 'extra');
         INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 3, 'best');",
    );
    refuses(
        &conn,
        "UPDATE recording_file SET role = 'best' WHERE file_id = 1",
        "UNIQUE",
    );
    // Another track has its own best file.
    accepts(
        &conn,
        "UPDATE recording_file SET recording_id = 2, role = 'best' WHERE file_id = 2",
    );
}

#[test]
fn a_file_role_is_best_undecided_or_extra_and_undecided_by_default() {
    let (_dir, conn) = with_tracks();
    let role: String = one(&conn, "SELECT role FROM recording_file WHERE file_id = 1");
    assert_eq!(role, "undecided");
    for bad in ["keep", "superseded", "Best"] {
        refuses(
            &conn,
            &format!("UPDATE recording_file SET role = '{bad}'"),
            "CHECK",
        );
    }
    refuses(
        &conn,
        "UPDATE recording_file SET match_confidence = 1.5",
        "CHECK",
    );
}

#[test]
fn a_file_must_be_known_and_a_track_or_file_with_duplicates_cannot_be_deleted() {
    let (_dir, conn) = with_tracks();
    refuses(
        &conn,
        "INSERT INTO recording_file (recording_id, file_id) VALUES (2, 99)",
        "FOREIGN KEY",
    );
    refuses(&conn, "DELETE FROM recording WHERE id = 1", "FOREIGN KEY");
    refuses(&conn, "DELETE FROM file WHERE id = 1", "FOREIGN KEY");
}

#[test]
fn duplicates_merge_a_file_can_move_to_another_track() {
    let (_dir, conn) = with_tracks();
    accepts(
        &conn,
        "UPDATE recording_file SET recording_id = 2 WHERE file_id = 1",
    );
}

// Versions.

#[test]
fn a_track_cannot_be_linked_to_itself() {
    let (_dir, conn) = with_tracks();
    refuses(
        &conn,
        "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES (1, 1, 'cut', 'user')",
        "CHECK",
    );
}

#[test]
fn two_tracks_are_linked_once_whichever_way_round() {
    let (_dir, conn) = with_tracks();
    accepts(
        &conn,
        "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES (1, 2, 'cut', 'parser')",
    );
    refuses(
        &conn,
        "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES (1, 2, 'rework', 'user')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES (2, 1, 'cut', 'user')",
        "UNIQUE",
    );
    // The direction is kept for labels that have one.
    accepts(
        &conn,
        "INSERT INTO version_link (recording_a, recording_b, kind, label, source) VALUES (3, 1, 'rework', 'mashup-contains', 'user')",
    );
    let (a, b): (i64, i64) = conn
        .query_row(
            "SELECT recording_a, recording_b FROM version_link WHERE label = 'mashup-contains'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((a, b), (3, 1));
}

#[test]
fn a_version_link_is_a_cut_or_a_rework_from_a_known_source() {
    let (_dir, conn) = with_tracks();
    for (n, kind) in ["cut", "rework"].iter().enumerate() {
        accepts(
            &conn,
            &format!(
                "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES (1, {}, '{kind}', 'fingerprint')",
                n + 2
            ),
        );
    }
    for (kind, source) in [
        ("remix", "user"),
        ("edit", "user"),
        ("cut", "guess"),
        ("cut", ""),
    ] {
        refuses(
            &conn,
            &format!(
                "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES (2, 3, '{kind}', '{source}')"
            ),
            "CHECK",
        );
    }
    refuses(&conn, "UPDATE version_link SET confirmed = 2", "CHECK");
}

#[test]
fn linked_versions_never_merge_a_file_cannot_move_from_one_to_the_other() {
    // File 1 is on track 1. Each way of moving it to track 2 is refused,
    // with the link stored either way round.
    let moves = [
        "UPDATE recording_file SET recording_id = 2 WHERE file_id = 1",
        "INSERT OR REPLACE INTO recording_file (recording_id, file_id) VALUES (2, 1)",
        "REPLACE INTO recording_file (recording_id, file_id, role) VALUES (2, 1, 'best')",
        "INSERT INTO recording_file (recording_id, file_id) VALUES (2, 1)
         ON CONFLICT (file_id) DO UPDATE SET recording_id = excluded.recording_id",
    ];
    for (a, b) in [(1, 2), (2, 1)] {
        for sql in moves {
            let (_dir, conn) = with_tracks();
            accepts(
                &conn,
                &format!(
                    "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES ({a}, {b}, 'cut', 'parser')"
                ),
            );
            refuses(&conn, sql, "versions never merge");
            let track: i64 = one(
                &conn,
                "SELECT recording_id FROM recording_file WHERE file_id = 1",
            );
            assert_eq!(track, 1, "link {a}->{b}: {sql} moved the file");
        }
    }
}

#[test]
fn a_file_can_still_move_to_a_track_that_is_not_a_linked_version() {
    // Moving it to an unlinked track is a duplicate merge, which is fine,
    // however it's written.
    for sql in [
        "UPDATE recording_file SET recording_id = 3 WHERE file_id = 1",
        "INSERT OR REPLACE INTO recording_file (recording_id, file_id) VALUES (3, 1)",
        "INSERT INTO recording_file (recording_id, file_id) VALUES (3, 1)
         ON CONFLICT (file_id) DO UPDATE SET recording_id = excluded.recording_id",
    ] {
        let (_dir, conn) = with_tracks();
        accepts(
            &conn,
            "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES (1, 2, 'cut', 'parser')",
        );
        accepts(&conn, sql);
        let track: i64 = one(
            &conn,
            "SELECT recording_id FROM recording_file WHERE file_id = 1",
        );
        assert_eq!(track, 3, "{sql}");
    }
}

#[test]
fn a_linked_version_cannot_be_deleted_until_the_link_is_removed() {
    let (_dir, conn) = with_tracks();
    accepts(
        &conn,
        "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES (2, 3, 'rework', 'user')",
    );
    refuses(&conn, "DELETE FROM recording WHERE id = 3", "FOREIGN KEY");
    accepts(
        &conn,
        "DELETE FROM version_link; DELETE FROM recording WHERE id = 3",
    );
}

#[test]
fn a_version_link_needs_two_known_tracks() {
    let (_dir, conn) = with_tracks();
    refuses(
        &conn,
        "INSERT INTO version_link (recording_a, recording_b, kind, source) VALUES (1, 99, 'cut', 'user')",
        "FOREIGN KEY",
    );
}

// Analysis and the values a track shows.

#[test]
fn a_track_has_no_bpm_key_or_energy_of_its_own_to_edit() {
    let (_dir, conn) = with_tracks();
    for column in columns(&conn, "recording") {
        for derived in ["bpm", "key", "energy", "tonality", "tempo"] {
            assert!(
                !column.to_lowercase().contains(derived),
                "recording.{column} would let {derived} be edited directly"
            );
        }
    }
}

#[test]
fn the_displayed_values_cannot_be_written_to() {
    let (_dir, conn) = with_tracks();
    for sql in [
        "UPDATE recording_display SET bpm = 128",
        "INSERT INTO recording_display (recording_id, bpm) VALUES (1, 128)",
        "DELETE FROM recording_display",
    ] {
        let err = conn.execute_batch(sql).unwrap_err().to_string();
        assert!(err.contains("it is a view"), "{sql}: {err}");
    }
}

/// (bpm, bpm_source, key, key_source, energy, energy_source) shown for
/// track 1.
type Shown = (
    Option<f64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
);

fn shown(conn: &Connection) -> Shown {
    conn.query_row(
        "SELECT bpm, bpm_source, key, key_source, energy, energy_source
         FROM recording_display WHERE recording_id = 1",
        [],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .unwrap()
}

#[test]
fn a_track_with_no_analysis_is_shown_with_no_values() {
    let (_dir, conn) = with_tracks();
    assert_eq!(shown(&conn), (None, None, None, None, None, None));
    let rows: i64 = one(&conn, "SELECT COUNT(*) FROM recording_display");
    assert_eq!(rows, 3, "every track has a row");
}

#[test]
fn bpm_and_key_come_from_rekordbox_then_the_file_tag_then_mik_then_the_local_estimate() {
    let (_dir, conn) = with_tracks();
    // Added lowest first, so insertion order can't be what decides.
    let sources = [
        ("local", 120.0, "1A"),
        ("mik", 121.0, "2A"),
        ("tag", 122.0, "3A"),
        ("rekordbox", 123.0, "4A"),
    ];
    for (source, bpm, key) in sources {
        accepts(
            &conn,
            &format!(
                "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (1, '{source}', {bpm}, '{key}')"
            ),
        );
    }
    // Remove the winner each time; the next one down takes over.
    for (source, bpm, key) in sources.iter().rev() {
        let (b, bs, k, ks, _, _) = shown(&conn);
        assert_eq!((b, bs.as_deref()), (Some(*bpm), Some(*source)));
        assert_eq!((k.as_deref(), ks.as_deref()), (Some(*key), Some(*source)));
        accepts(
            &conn,
            &format!("DELETE FROM analysis WHERE source = '{source}'"),
        );
    }
    assert_eq!(shown(&conn).0, None);
}

#[test]
fn energy_comes_from_the_user_then_mik_then_the_local_estimate_then_the_audio_model() {
    let (_dir, conn) = with_tracks();
    let sources = [("ml", 2), ("local", 3), ("mik", 4), ("user", 5)];
    for (source, energy) in sources {
        accepts(
            &conn,
            &format!(
                "INSERT INTO analysis (recording_id, source, energy) VALUES (1, '{source}', {energy})"
            ),
        );
    }
    for (source, energy) in sources.iter().rev() {
        let (_, _, _, _, e, es) = shown(&conn);
        assert_eq!((e, es.as_deref()), (Some(*energy), Some(*source)));
        accepts(
            &conn,
            &format!("DELETE FROM analysis WHERE source = '{source}'"),
        );
    }
    assert_eq!(shown(&conn).4, None);
}

#[test]
fn each_value_is_taken_from_the_best_source_that_has_it() {
    // rekordbox knows the key but hasn't analyzed the BPM; the tag has a BPM.
    let (_dir, conn) = with_tracks();
    accepts(
        &conn,
        "INSERT INTO analysis (recording_id, source, key) VALUES (1, 'rekordbox', '8A');
         INSERT INTO analysis (recording_id, source, bpm, key) VALUES (1, 'tag', 126, '9A');
         INSERT INTO analysis (recording_id, source, energy) VALUES (1, 'mik', 7);",
    );
    assert_eq!(
        shown(&conn),
        (
            Some(126.0),
            Some("tag".into()),
            Some("8A".into()),
            Some("rekordbox".into()),
            Some(7),
            Some("mik".into())
        )
    );
}

#[test]
fn a_changed_analysis_changes_what_the_track_shows_at_once() {
    // The shown value is derived on every read, never stored.
    let (_dir, conn) = with_tracks();
    accepts(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (1, 'rekordbox', 128)",
    );
    accepts(&conn, "UPDATE analysis SET bpm = 64 WHERE recording_id = 1");
    assert_eq!(shown(&conn).0, Some(64.0));
    // Another track's analysis doesn't leak into this one.
    accepts(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (2, 'rekordbox', 90)",
    );
    assert_eq!(shown(&conn).0, Some(64.0));
}

#[test]
fn each_source_gives_one_opinion_per_track() {
    let (_dir, conn) = with_tracks();
    accepts(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (1, 'tag', 128)",
    );
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (1, 'tag', 129)",
        "UNIQUE",
    );
}

#[test]
fn an_analysis_source_is_one_of_the_six_sources() {
    let (_dir, conn) = with_tracks();
    for bad in ["serato", "MIK", "estimate", ""] {
        refuses(
            &conn,
            &format!("INSERT INTO analysis (recording_id, source, bpm) VALUES (1, '{bad}', 128)"),
            "CHECK",
        );
    }
}

#[test]
fn analysis_values_have_sane_ranges_and_keys_are_camelot() {
    let (_dir, conn) = with_tracks();
    for bad in [
        "bpm) VALUES (1, 'tag', 0",
        "bpm) VALUES (1, 'tag', -120",
        "bpm) VALUES (1, 'tag', 1000",
        "key) VALUES (1, 'tag', 'Am'",
        "key) VALUES (1, 'tag', '13A'",
        "key) VALUES (1, 'tag', '8a'",
        "key) VALUES (1, 'tag', '0B'",
        "energy) VALUES (1, 'mik', 0",
        "energy) VALUES (1, 'mik', 11",
    ] {
        refuses(
            &conn,
            &format!("INSERT INTO analysis (recording_id, source, {bad})"),
            "CHECK",
        );
    }
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm, confidence) VALUES (1, 'local', 128, 1.1)",
        "CHECK",
    );
    for (recording, source, key) in [
        (1, "tag", "1A"),
        (1, "mik", "12A"),
        (2, "tag", "1B"),
        (2, "mik", "12B"),
        (3, "rekordbox", "8A"),
    ] {
        accepts(
            &conn,
            &format!(
                "INSERT INTO analysis (recording_id, source, key) VALUES ({recording}, '{source}', '{key}')"
            ),
        );
    }
}

#[test]
fn an_empty_analysis_row_is_refused() {
    let (_dir, conn) = with_tracks();
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source) VALUES (1, 'tag')",
        "CHECK",
    );
}

#[test]
fn every_value_a_source_gives_has_a_place_in_the_precedence() {
    // The user and the audio model give energy only; rekordbox and file
    // tags give BPM and key only. Anything else would be stored and then
    // silently never shown.
    let (_dir, conn) = with_tracks();
    for bad in [
        "bpm) VALUES (1, 'user', 128",
        "key) VALUES (1, 'user', '8A'",
        "bpm) VALUES (1, 'ml', 128",
        "key) VALUES (1, 'ml', '8A'",
        "energy) VALUES (1, 'rekordbox', 5",
        "energy) VALUES (1, 'tag', 5",
    ] {
        refuses(
            &conn,
            &format!("INSERT INTO analysis (recording_id, source, {bad})"),
            "CHECK",
        );
    }
}

#[test]
fn analysis_belongs_to_a_known_track_and_is_never_dropped_as_a_side_effect() {
    // Merging tracks moves or drops their analysis on purpose.
    let (_dir, conn) = with_tracks();
    refuses(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (99, 'tag', 128)",
        "FOREIGN KEY",
    );
    accepts(
        &conn,
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (3, 'tag', 128)",
    );
    refuses(&conn, "DELETE FROM recording WHERE id = 3", "FOREIGN KEY");
    accepts(
        &conn,
        "UPDATE analysis SET recording_id = 2 WHERE recording_id = 3;
         DELETE FROM recording WHERE id = 3;",
    );
    let moved: i64 = one(
        &conn,
        "SELECT COUNT(*) FROM analysis WHERE recording_id = 2",
    );
    assert_eq!(moved, 1);
}

#[test]
fn provenance_is_a_json_object_and_years_are_plausible() {
    let (_dir, conn) = with_tracks();
    accepts(
        &conn,
        r#"UPDATE recording SET provenance = '{"title": {"source": "rekordbox", "confidence": 1}}' WHERE id = 1"#,
    );
    refuses(
        &conn,
        "UPDATE recording SET provenance = 'rekordbox'",
        "CHECK",
    );
    refuses(&conn, "UPDATE recording SET provenance = '[]'", "CHECK");
    refuses(&conn, "UPDATE recording SET year = 99", "CHECK");
}
