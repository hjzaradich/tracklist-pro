//! 0037: `file_choice`, the file the user chose to show for a track outside
//! the Library (ROADMAP 1.11 "Hidden filter", 1.7; 1cB-22).

use super::{accepts, db, db_at, one, refuses};

const TRACK_WITH_A_FILE: &str = "
    INSERT INTO volume (identity, kind, last_mount_path) VALUES ('serial=NTFS-1', 'external', 'E:\');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');
    INSERT INTO recording (title) VALUES ('Track');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, audio_hash)
        VALUES (1, 'a.mp3', 'a.mp3', x'0102');
    INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 1, 'best');
    DELETE FROM best_file_due;";

#[test]
fn the_table_arrives_with_migration_0037() {
    let (_dir, conn) = db_at(36);
    assert_eq!(
        one::<i64>(
            &conn,
            "SELECT count(*) FROM sqlite_master WHERE name = 'file_choice'"
        ),
        0
    );
    let (_dir, conn) = db_at(37);
    assert_eq!(
        one::<i64>(
            &conn,
            "SELECT count(*) FROM sqlite_master WHERE name = 'file_choice'"
        ),
        1
    );
}

#[test]
fn a_choice_needs_a_key_and_a_whole_file_hash_that_is_empty_or_32_bytes() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO file_choice (content_key) VALUES (x'01');
         INSERT INTO file_choice (content_key, blake3) VALUES (x'01', zeroblob(32))",
    );
    refuses(
        &conn,
        "INSERT INTO file_choice (content_key) VALUES (x'')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO file_choice (content_key, blake3) VALUES (x'02', x'0102')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO file_choice (content_key) VALUES (NULL)",
        "NOT NULL",
    );
}

#[test]
fn the_same_choice_is_stored_once() {
    let (_dir, conn) = db();
    accepts(&conn, "INSERT INTO file_choice (content_key) VALUES (x'01')");
    refuses(
        &conn,
        "INSERT INTO file_choice (content_key) VALUES (x'01')",
        "UNIQUE",
    );
}

#[test]
fn making_or_taking_back_a_choice_makes_the_tracks_with_that_audio_due() {
    let (_dir, conn) = db();
    accepts(&conn, TRACK_WITH_A_FILE);
    accepts(&conn, "INSERT INTO file_choice (content_key) VALUES (x'0102')");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM best_file_due"), 1);
    accepts(&conn, "DELETE FROM best_file_due; DELETE FROM file_choice");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM best_file_due"), 1);
    // Audio no file holds makes nothing due.
    accepts(&conn, "DELETE FROM best_file_due");
    accepts(&conn, "INSERT INTO file_choice (content_key) VALUES (x'FF')");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM best_file_due"), 0);
}

#[test]
fn a_changed_whole_file_hash_makes_its_track_due_only_while_a_choice_exists() {
    let (_dir, conn) = db();
    accepts(&conn, TRACK_WITH_A_FILE);
    accepts(&conn, "UPDATE file SET blake3 = zeroblob(32)");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM best_file_due"), 0);
    accepts(
        &conn,
        "INSERT INTO file_choice (content_key) VALUES (x'FF'); DELETE FROM best_file_due",
    );
    accepts(&conn, "UPDATE file SET blake3 = randomblob(32)");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM best_file_due"), 1);
}
