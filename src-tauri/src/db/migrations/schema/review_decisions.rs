//! 0027: `review_decision` (what the user decided in Review about a linked
//! file, kept by what the files hold) and `replaced_entry` (a rekordbox
//! entry a switch of the linked file left behind). ROADMAP 1.12, 1.9 rule 7.

use super::{accepts, db, db_at, one, refuses};
use crate::db::migrations::{run, MIGRATIONS};

#[test]
fn a_dismissal_names_the_linked_files_audio_and_the_better_files_audio() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO review_decision (kind, linked_key, other_key)
         VALUES ('dismissed_better', x'01', x'02')",
    );
    refuses(
        &conn,
        "INSERT INTO review_decision (kind, linked_key) VALUES ('dismissed_better', x'01')",
        "CHECK",
    );
}

#[test]
fn keep_anyway_and_want_better_name_only_the_linked_files_audio() {
    let (_dir, conn) = db();
    for kind in ["keep_anyway", "want_better"] {
        accepts(
            &conn,
            &format!("INSERT INTO review_decision (kind, linked_key) VALUES ('{kind}', x'01')"),
        );
        refuses(
            &conn,
            &format!(
                "INSERT INTO review_decision (kind, linked_key, other_key)
                 VALUES ('{kind}', x'03', x'02')"
            ),
            "CHECK",
        );
    }
    let empty: i64 = one(
        &conn,
        "SELECT count(*) FROM review_decision WHERE length(other_key) = 0",
    );
    assert_eq!(empty, 2);
}

#[test]
fn a_decision_needs_a_known_kind_and_a_key() {
    let (_dir, conn) = db();
    refuses(
        &conn,
        "INSERT INTO review_decision (kind, linked_key) VALUES ('hidden', x'01')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO review_decision (kind, linked_key) VALUES ('keep_anyway', x'')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO review_decision (kind, linked_key) VALUES ('keep_anyway', NULL)",
        "NOT NULL",
    );
}

#[test]
fn the_same_decision_is_stored_once_and_another_better_file_is_another_row() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO review_decision (kind, linked_key, other_key)
         VALUES ('dismissed_better', x'01', x'02');
         INSERT INTO review_decision (kind, linked_key) VALUES ('keep_anyway', x'01');",
    );
    refuses(
        &conn,
        "INSERT INTO review_decision (kind, linked_key, other_key)
         VALUES ('dismissed_better', x'01', x'02')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO review_decision (kind, linked_key) VALUES ('keep_anyway', x'01')",
        "UNIQUE",
    );
    // Another better file, another linked file and another kind are each
    // their own decision.
    accepts(
        &conn,
        "INSERT INTO review_decision (kind, linked_key, other_key)
         VALUES ('dismissed_better', x'01', x'03'), ('dismissed_better', x'04', x'02');
         INSERT INTO review_decision (kind, linked_key) VALUES ('want_better', x'01');",
    );
}

#[test]
fn a_decision_is_looked_up_by_kind_and_key_through_its_unique_index() {
    let (_dir, conn) = db();
    let mut stmt = conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT 1 FROM review_decision
             WHERE kind = 'keep_anyway' AND linked_key = x'01'",
        )
        .unwrap();
    let details: Vec<String> = stmt
        .query_map([], |r| r.get(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        details
            .iter()
            .any(|d| d.contains("USING") && d.contains("INDEX")),
        "{details:?}"
    );
}

#[test]
fn one_record_per_replaced_rekordbox_entry() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO replaced_entry (recording_id, location, location_key)
         VALUES (1, 'file://localhost/E:/Old/a.mp3', 'e:/old/a.mp3')",
    );
    refuses(
        &conn,
        "INSERT INTO replaced_entry (recording_id, location, location_key)
         VALUES (2, 'file://localhost/E:/OLD/a.mp3', 'e:/old/a.mp3')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO replaced_entry (recording_id, location, location_key)
         VALUES (1, 'file://localhost/E:/Old/b.mp3', '')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO replaced_entry (recording_id, location, location_key)
         VALUES (1, '', 'e:/old/b.mp3')",
        "CHECK",
    );
}

#[test]
fn a_replaced_entry_outlives_its_library_track_and_its_track() {
    // rekordbox still holds the entry after the track is removed from the
    // Library, or merged away, so the record isn't tied to either row.
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
         INSERT INTO recording (title) VALUES ('Made-up Tune');
         INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 1);
         INSERT INTO replaced_entry (recording_id, location, location_key)
         VALUES (1, 'file://localhost/E:/Old/a.mp3', 'e:/old/a.mp3');
         DELETE FROM library_track WHERE id = 1;
         DELETE FROM recording WHERE id = 1;",
    );
    let kept: i64 = one(&conn, "SELECT count(*) FROM replaced_entry");
    assert_eq!(kept, 1);
}

#[test]
fn the_migration_applies_to_a_database_that_already_holds_a_library() {
    let (_dir, mut conn) = db_at(26);
    accepts(
        &conn,
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
         INSERT INTO recording (title) VALUES ('Made-up Tune');
         INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 1);",
    );
    run(&mut conn, &MIGRATIONS[..27]).unwrap();
    let tracks: i64 = one(&conn, "SELECT count(*) FROM library_track");
    let decisions: i64 = one(&conn, "SELECT count(*) FROM review_decision");
    let replaced: i64 = one(&conn, "SELECT count(*) FROM replaced_entry");
    assert_eq!((tracks, decisions, replaced), (1, 0, 0));
}
