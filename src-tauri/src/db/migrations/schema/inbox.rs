//! 0047: `inbox_batch`, `inbox_item` (ROADMAP 2.1, §2). What intake does
//! with them is tested in `inbox/tests.rs`.

use super::files::with_a_file;
use super::{accepts, one, refuses};

/// A batch (id 1) for the file's music folder.
fn with_a_batch() -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, conn) = with_a_file();
    accepts(
        &conn,
        "INSERT INTO inbox_batch (music_folder_id, kind) VALUES (1, 'loose')",
    );
    (dir, conn)
}

#[test]
fn a_batch_is_loose_a_folder_or_already_there_and_only_a_folder_has_a_label() {
    let (_dir, conn) = with_a_file();
    accepts(
        &conn,
        "INSERT INTO inbox_batch (music_folder_id, kind) VALUES (1, 'loose');
         INSERT INTO inbox_batch (music_folder_id, kind) VALUES (1, 'already_there');
         INSERT INTO inbox_batch (music_folder_id, kind, label) VALUES (1, 'folder', 'Pack');",
    );
    refuses(
        &conn,
        "INSERT INTO inbox_batch (music_folder_id, kind) VALUES (1, 'zip')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO inbox_batch (music_folder_id, kind) VALUES (1, 'folder')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO inbox_batch (music_folder_id, kind, label) VALUES (1, 'loose', 'Pack')",
        "CHECK",
    );
}

#[test]
fn an_item_is_new_not_now_hidden_or_added_and_is_undecided_only_while_new() {
    let (_dir, conn) = with_a_batch();
    accepts(
        &conn,
        "INSERT INTO inbox_item (file_id, batch_id) VALUES (1, 1)",
    );
    assert_eq!(one::<String>(&conn, "SELECT status FROM inbox_item"), "new");
    for status in ["not_now", "hidden", "added"] {
        accepts(
            &conn,
            &format!(
                "UPDATE inbox_item SET status = '{status}', decided_at = '2026-01-01T00:00:00.000Z'"
            ),
        );
    }
    refuses(&conn, "UPDATE inbox_item SET status = 'cleared'", "CHECK");
    refuses(
        &conn,
        "UPDATE inbox_item SET status = 'new', decided_at = '2026-01-01T00:00:00.000Z'",
        "CHECK",
    );
}

#[test]
fn a_file_is_in_the_inbox_once() {
    let (_dir, conn) = with_a_batch();
    accepts(
        &conn,
        "INSERT INTO inbox_item (file_id, batch_id) VALUES (1, 1)",
    );
    refuses(
        &conn,
        "INSERT INTO inbox_item (file_id, batch_id) VALUES (1, 1)",
        "UNIQUE",
    );
}

#[test]
fn deleting_a_file_deletes_its_item_and_deleting_a_folder_its_batches() {
    let (_dir, conn) = with_a_batch();
    accepts(
        &conn,
        "INSERT INTO inbox_item (file_id, batch_id) VALUES (1, 1)",
    );
    accepts(&conn, "DELETE FROM file WHERE id = 1");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM inbox_item"), 0);
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM inbox_batch"), 1);
    accepts(&conn, "DELETE FROM music_folder WHERE id = 1");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM inbox_batch"), 0);
}

#[test]
fn an_item_and_a_batch_never_get_an_id_back() {
    let (_dir, conn) = with_a_batch();
    accepts(
        &conn,
        "INSERT INTO inbox_item (file_id, batch_id) VALUES (1, 1);
         DELETE FROM inbox_item;
         DELETE FROM inbox_batch;
         INSERT INTO inbox_batch (music_folder_id, kind) VALUES (1, 'loose');
         INSERT INTO inbox_item (file_id, batch_id) VALUES (1, 2);",
    );
    assert_eq!(one::<i64>(&conn, "SELECT id FROM inbox_item"), 2);
    assert_eq!(one::<i64>(&conn, "SELECT id FROM inbox_batch"), 2);
}
