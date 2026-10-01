//! 0015: the playlists and folders a send has written (ROADMAP 1.9 rule 6).

use super::{accepts, db, one, refuses};

const INSERT: &str = "INSERT INTO sent_playlist (path, path_key, kind) VALUES";

#[test]
fn a_sent_path_is_a_list_of_names_from_the_top_folder_down() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        &format!(r#"{INSERT} ('["Crates","House"]', '["crates","house"]', 'folder')"#),
    );
    accepts(
        &conn,
        &format!(
            r#"{INSERT} ('["Crates","House","Deep"]', '["crates","house","deep"]', 'playlist')"#
        ),
    );
    // Not a list, a list of one (the top folder alone), or not JSON.
    for path in ["\"Crates\"", r#"["Crates"]"#, "Crates", "{}"] {
        refuses(
            &conn,
            &format!(r#"{INSERT} ('{path}', '["crates"]', 'playlist')"#),
            "CHECK",
        );
    }
    // Names and keys must come in pairs.
    refuses(
        &conn,
        &format!(r#"{INSERT} ('["Crates","A"]', '["crates"]', 'playlist')"#),
        "CHECK",
    );
}

#[test]
fn a_path_is_one_row_per_kind_and_a_kind_is_a_folder_or_a_playlist() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        &format!(r#"{INSERT} ('["Crates","A"]', '["crates","a"]', 'folder')"#),
    );
    // The same normalized path again, as the same kind, is refused.
    refuses(
        &conn,
        &format!(r#"{INSERT} ('["Crates","a "]', '["crates","a"]', 'folder')"#),
        "UNIQUE",
    );
    // As the other kind it's a different row.
    accepts(
        &conn,
        &format!(r#"{INSERT} ('["Crates","A"]', '["crates","a"]', 'playlist')"#),
    );
    refuses(
        &conn,
        &format!(r#"{INSERT} ('["Crates","B"]', '["crates","b"]', 'crate')"#),
        "CHECK",
    );
    assert_eq!(one::<i64>(&conn, "SELECT count(*) FROM sent_playlist"), 2);
}

#[test]
fn a_path_starts_unseen_and_seen_is_a_flag() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        &format!(r#"{INSERT} ('["Crates","A"]', '["crates","a"]', 'playlist')"#),
    );
    assert_eq!(one::<i64>(&conn, "SELECT seen FROM sent_playlist"), 0);
    accepts(&conn, "UPDATE sent_playlist SET seen = 1");
    refuses(&conn, "UPDATE sent_playlist SET seen = 2", "CHECK");
}
