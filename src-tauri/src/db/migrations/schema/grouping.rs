//! 0022: `grouping_pair`, `grouping_file` and `grouping_decision` (1bC-1;
//! ROADMAP 1.4, 1.5), and the triggers that make a file due again.

use rusqlite::Connection;

use super::super::{run, MIGRATIONS};
use super::files::with_a_file;
use super::{accepts, one, refuses};

const PAIR: &str = "INSERT INTO grouping_pair (file_a, file_b, outcome, reasons) VALUES";

/// Files 1, 2 and 3, each with a fingerprint and an `audio_hash`, each with
/// a record that says it's decided. The pairs 1–2 and 1–3 are stored.
fn with_three_decided_files() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = with_a_file();
    accepts(
        &conn,
        &format!(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'b.mp3', 'b.mp3');
             INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'c.mp3', 'c.mp3');
             UPDATE file SET fingerprint = x'0102', audio_hash = x'AA', blake3 = zeroblob(32),
                             raw_tags = '{{}}';
             INSERT INTO grouping_file (file_id, placed_key, decided_as, rev)
                 SELECT id, audio_hash, 7, 5 FROM file;
             {PAIR} (1, 2, 'duplicate', '[]');
             {PAIR} (1, 3, 'review', '[]');"
        ),
    );
    (dir, conn)
}

fn pairs(conn: &Connection) -> Vec<(i64, i64)> {
    let mut stmt = conn
        .prepare("SELECT file_a, file_b FROM grouping_pair ORDER BY file_a, file_b")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows
}

/// The files whose record says they're due.
fn due(conn: &Connection) -> Vec<i64> {
    let mut stmt = conn
        .prepare("SELECT file_id FROM grouping_file WHERE decided_as IS NULL ORDER BY file_id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows
}

fn rev(conn: &Connection, file: i64) -> i64 {
    one(
        conn,
        &format!("SELECT rev FROM grouping_file WHERE file_id = {file}"),
    )
}

#[test]
fn a_pair_is_two_known_files_the_lower_id_first_stored_once() {
    let (_dir, conn) = with_three_decided_files();
    refuses(
        &conn,
        &format!("{PAIR} (1, 2, 'duplicate', '[]')"),
        "UNIQUE",
    );
    refuses(&conn, &format!("{PAIR} (3, 2, 'duplicate', '[]')"), "CHECK");
    refuses(&conn, &format!("{PAIR} (2, 2, 'duplicate', '[]')"), "CHECK");
    refuses(
        &conn,
        &format!("{PAIR} (2, 9, 'duplicate', '[]')"),
        "FOREIGN KEY",
    );
    accepts(&conn, &format!("{PAIR} (2, 3, 'different', '[]')"));
}

#[test]
fn a_pair_has_one_of_the_five_outcomes_a_list_of_reasons_and_a_known_state() {
    let (_dir, conn) = with_three_decided_files();
    refuses(&conn, &format!("{PAIR} (2, 3, 'merge', '[]')"), "CHECK");
    refuses(&conn, &format!("{PAIR} (2, 3, 'review', '{{}}')"), "CHECK");
    refuses(
        &conn,
        &format!("{PAIR} (2, 3, 'review', 'not json')"),
        "CHECK",
    );
    refuses(
        &conn,
        "UPDATE grouping_pair SET state = 'somewhere' WHERE file_b = 2",
        "CHECK",
    );
    // A new pair waits for a grouping pass to say where it stands.
    let state: String = one(&conn, "SELECT state FROM grouping_pair WHERE file_b = 2");
    assert_eq!(state, "waiting");
    for state in [
        "waiting",
        "merged",
        "both_in_library",
        "set_disagrees",
        "linked",
        "open",
        "in_one_track",
        "decided",
        "apart",
    ] {
        accepts(
            &conn,
            &format!("UPDATE grouping_pair SET state = '{state}' WHERE file_b = 2"),
        );
    }
}

#[test]
fn only_a_cut_or_a_rework_says_what_made_its_link_and_each_must() {
    let (_dir, conn) = with_three_decided_files();
    let linked = "INSERT INTO grouping_pair
        (file_a, file_b, outcome, reasons, link_label, link_source, link_first) VALUES";
    refuses(&conn, &format!("{PAIR} (2, 3, 'cut', '[]')"), "CHECK");
    refuses(
        &conn,
        &format!("{linked} (2, 3, 'duplicate', '[]', NULL, 'parser', NULL)"),
        "CHECK",
    );
    refuses(
        &conn,
        &format!("{linked} (2, 3, 'cut', '[]', NULL, 'user', NULL)"),
        "CHECK",
    );
    refuses(
        &conn,
        &format!("{linked} (2, 3, 'rework', '[]', NULL, 'parser', 'c')"),
        "CHECK",
    );
    accepts(
        &conn,
        &format!("{linked} (2, 3, 'rework', '[]', 'mashup-contains', 'fingerprint', 'b')"),
    );
}

#[test]
fn a_files_pairs_and_its_record_go_with_its_file_row() {
    let (_dir, conn) = with_three_decided_files();
    accepts(&conn, "DELETE FROM file WHERE id = 3");
    assert_eq!(pairs(&conn), [(1, 2)]);
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM grouping_file"), 2);
    accepts(&conn, "DELETE FROM file WHERE id = 1");
    assert!(pairs(&conn).is_empty());
}

#[test]
fn a_changed_fingerprint_drops_the_files_pairs_and_makes_it_due_and_no_other_file() {
    let (_dir, conn) = with_three_decided_files();
    let before = rev(&conn, 2);
    accepts(&conn, "UPDATE file SET fingerprint = x'0304' WHERE id = 2");
    assert_eq!(pairs(&conn), [(1, 3)]);
    assert_eq!(due(&conn), [2]);
    assert_ne!(rev(&conn, 2), before);
    assert_eq!(rev(&conn, 1), 5);
    // Taken away altogether: the same.
    accepts(&conn, "UPDATE file SET fingerprint = NULL WHERE id = 3");
    assert!(pairs(&conn).is_empty());
    assert_eq!(due(&conn), [2, 3]);
}

#[test]
fn other_audio_drops_the_files_pairs_and_makes_it_due() {
    let (_dir, conn) = with_three_decided_files();
    accepts(&conn, "UPDATE file SET audio_hash = x'BB' WHERE id = 3");
    assert_eq!(pairs(&conn), [(1, 2)]);
    assert_eq!(due(&conn), [3]);
}

#[test]
fn a_tag_rewrite_keeps_the_pairs_of_a_file_with_an_audio_hash() {
    // rekordbox rewrites tags (ROADMAP 5.1): the whole-file hash changes,
    // the audio doesn't. The same bytes written again change nothing at all.
    let (_dir, conn) = with_three_decided_files();
    accepts(
        &conn,
        "UPDATE file SET blake3 = randomblob(32), size = 11, mtime = 200 WHERE id = 1;
         UPDATE file SET fingerprint = x'0102', audio_hash = x'AA' WHERE id = 1;",
    );
    assert_eq!(pairs(&conn), [(1, 2), (1, 3)]);
    assert!(due(&conn).is_empty());
    assert_eq!(rev(&conn, 1), 5);
}

#[test]
fn a_file_with_no_audio_hash_is_known_by_its_whole_file_hash_so_a_new_one_drops_its_pairs() {
    let (_dir, conn) = with_three_decided_files();
    // An empty audio hash says nothing, like none.
    accepts(&conn, "UPDATE file SET audio_hash = x'' WHERE id = 2");
    assert_eq!(pairs(&conn), [(1, 3)], "no audio hash any more");
    accepts(
        &conn,
        &format!(
            "UPDATE grouping_file SET decided_as = 7 WHERE file_id = 2;
             {PAIR} (2, 3, 'duplicate', '[]');
             UPDATE file SET blake3 = randomblob(32) WHERE id = 2;"
        ),
    );
    assert_eq!(pairs(&conn), [(1, 3)]);
    assert_eq!(due(&conn), [2]);
}

#[test]
fn new_tags_or_a_new_name_make_a_file_due_and_keep_its_pairs_until_they_are_decided_again() {
    let (_dir, conn) = with_three_decided_files();
    accepts(
        &conn,
        r#"UPDATE file SET raw_tags = '{"id3v2": []}' WHERE id = 1"#,
    );
    assert_eq!(due(&conn), [1]);
    assert_ne!(rev(&conn, 1), 5);
    accepts(
        &conn,
        "UPDATE file SET rel_path = 'renamed.mp3', rel_path_key = 'renamed.mp3' WHERE id = 2",
    );
    assert_eq!(due(&conn), [1, 2]);
    assert_eq!(pairs(&conn), [(1, 2), (1, 3)]);
    // The same tags written again: nothing new to decide.
    accepts(&conn, "UPDATE file SET raw_tags = '{}' WHERE id = 3");
    assert_eq!(due(&conn), [1, 2]);
}

#[test]
fn a_file_that_comes_back_is_due_and_one_that_goes_missing_is_not() {
    let (_dir, conn) = with_three_decided_files();
    accepts(&conn, "UPDATE file SET present = 0 WHERE id = 3");
    assert!(due(&conn).is_empty());
    assert_eq!(pairs(&conn), [(1, 2), (1, 3)]);
    accepts(&conn, "UPDATE file SET present = 1 WHERE id = 3");
    assert_eq!(due(&conn), [3]);
}

#[test]
fn a_decision_names_two_pieces_of_audio_the_lower_first_once_and_is_apart_or_same() {
    let (_dir, conn) = with_a_file();
    let decision = "INSERT INTO grouping_decision (key_a, key_b, decision) VALUES";
    accepts(&conn, &format!("{decision} (x'01', x'02', 'apart')"));
    refuses(
        &conn,
        &format!("{decision} (x'01', x'02', 'same')"),
        "UNIQUE",
    );
    refuses(
        &conn,
        &format!("{decision} (x'02', x'01', 'same')"),
        "CHECK",
    );
    refuses(&conn, &format!("{decision} (x'', x'01', 'same')"), "CHECK");
    refuses(
        &conn,
        &format!("{decision} (x'01', x'03', 'maybe')"),
        "CHECK",
    );
    // Two tracks holding the very same audio, kept apart.
    accepts(&conn, &format!("{decision} (x'01', x'01', 'apart')"));
}

#[test]
fn a_decision_outlives_the_files_it_was_made_on() {
    // It names what they hold, so a music folder removed and added again
    // finds it still there.
    let (_dir, conn) = with_three_decided_files();
    accepts(
        &conn,
        "INSERT INTO grouping_decision (key_a, key_b, decision) VALUES (x'AA', x'BB', 'apart');
         DELETE FROM file;",
    );
    assert_eq!(
        one::<i64>(&conn, "SELECT count(*) FROM grouping_decision"),
        1
    );
}

#[test]
fn the_grouping_triggers_survive_every_later_migration() {
    // A later migration that rebuilds `file` would silently drop the
    // triggers on it.
    let triggers = [
        "grouping_pair_goes_with_its_audio",
        "grouping_file_is_due_when_its_names_change",
        "grouping_file_is_due_when_it_comes_back",
    ];
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(dir.path().join("test.db")).unwrap();
    let mut added = false;
    for (n, migration) in MIGRATIONS.iter().enumerate() {
        run(&mut conn, &MIGRATIONS[..=n]).unwrap();
        added |= migration.name == "0022_grouping.sql";
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
    assert!(added, "0022 was never applied");
}
