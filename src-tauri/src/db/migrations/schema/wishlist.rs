//! 0048: `wishlist`, `wish_item` (ROADMAP 2.3, §2). What the app does with
//! them is tested in `wishlist/tests.rs`.

use super::files::with_a_file;
use super::{accepts, columns, one, refuses};

/// A wishlist (id 1) with an item (id 1), and a file (id 1).
fn with_an_item() -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, conn) = with_a_file();
    accepts(
        &conn,
        "INSERT INTO wishlist (name, source) VALUES ('Friday', 'paste');
         INSERT INTO wish_item (wishlist_id, position, raw_artist, raw_title)
         VALUES (1, 0, 'Made Up', 'Nothing');",
    );
    (dir, conn)
}

#[test]
fn a_wishlist_has_a_name_and_comes_from_paste_or_csv() {
    let (_dir, conn) = with_a_file();
    accepts(
        &conn,
        "INSERT INTO wishlist (name, source) VALUES ('A', 'paste');
         INSERT INTO wishlist (name, source) VALUES ('B', 'csv');",
    );
    refuses(
        &conn,
        "INSERT INTO wishlist (name, source) VALUES ('C', 'spotify')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO wishlist (name, source) VALUES ('  ', 'paste')",
        "CHECK",
    );
    assert_eq!(
        columns(&conn, "wishlist"),
        ["id", "name", "source", "created_at"]
    );
}

#[test]
fn an_item_has_an_artist_and_a_title_and_starts_unbought() {
    let (_dir, conn) = with_an_item();
    assert_eq!(one::<i64>(&conn, "SELECT bought FROM wish_item"), 0);
    assert_eq!(
        one::<Option<i64>>(&conn, "SELECT acquired_file_id FROM wish_item"),
        None
    );
    for (artist, title) in [("", "T"), ("A", " "), ("A", "")] {
        refuses(
            &conn,
            &format!(
                "INSERT INTO wish_item (wishlist_id, position, raw_artist, raw_title)
                 VALUES (1, 5, '{artist}', '{title}')"
            ),
            "CHECK",
        );
    }
    refuses(&conn, "UPDATE wish_item SET bought = 2", "CHECK");
    refuses(&conn, "UPDATE wish_item SET raw_isrc = ''", "CHECK");
    refuses(&conn, "UPDATE wish_item SET raw_version = ' '", "CHECK");
}

#[test]
fn an_item_has_one_position_in_its_wishlist() {
    let (_dir, conn) = with_an_item();
    refuses(
        &conn,
        "INSERT INTO wish_item (wishlist_id, position, raw_artist, raw_title)
         VALUES (1, 0, 'A', 'T')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO wish_item (wishlist_id, position, raw_artist, raw_title)
         VALUES (1, -1, 'A', 'T')",
        "CHECK",
    );
    accepts(
        &conn,
        "INSERT INTO wishlist (name, source) VALUES ('Other', 'csv');
         INSERT INTO wish_item (wishlist_id, position, raw_artist, raw_title)
         VALUES (2, 0, 'A', 'T');",
    );
}

#[test]
fn deleting_a_wishlist_deletes_its_items() {
    let (_dir, conn) = with_an_item();
    accepts(&conn, "DELETE FROM wishlist WHERE id = 1");
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM wish_item"), 0);
}

#[test]
fn deleting_the_acquired_file_leaves_the_item_acquired_with_no_file() {
    let (_dir, conn) = with_an_item();
    accepts(
        &conn,
        "UPDATE wish_item SET acquired_file_id = 1, acquired_at = '2026-01-01T00:00:00.000Z'",
    );
    accepts(&conn, "DELETE FROM file WHERE id = 1");
    assert_eq!(
        one::<Option<i64>>(&conn, "SELECT acquired_file_id FROM wish_item"),
        None
    );
    assert_eq!(
        one::<Option<String>>(&conn, "SELECT acquired_at FROM wish_item"),
        Some("2026-01-01T00:00:00.000Z".to_owned())
    );
}

#[test]
fn a_wishlist_and_an_item_never_get_an_id_back() {
    let (_dir, conn) = with_an_item();
    accepts(
        &conn,
        "DELETE FROM wishlist;
         INSERT INTO wishlist (name, source) VALUES ('Again', 'paste');
         INSERT INTO wish_item (wishlist_id, position, raw_artist, raw_title)
         VALUES (2, 0, 'A', 'T');",
    );
    assert_eq!(one::<i64>(&conn, "SELECT id FROM wishlist"), 2);
    assert_eq!(one::<i64>(&conn, "SELECT id FROM wish_item"), 2);
}
