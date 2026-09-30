//! 0009: `file.partial_hash`, for the unchanged check (1aC-1).

use super::files::with_a_file;
use super::{accepts, one, refuses};

#[test]
fn a_new_file_has_no_partial_hash() {
    let (_dir, conn) = with_a_file();
    let hash: Option<Vec<u8>> = one(&conn, "SELECT partial_hash FROM file");
    assert_eq!(hash, None);
}

#[test]
fn a_partial_hash_is_33_bytes_or_absent() {
    // A definition byte, then a 32-byte digest (hash/partial.rs).
    let (_dir, conn) = with_a_file();
    accepts(&conn, "UPDATE file SET partial_hash = zeroblob(33)");
    accepts(&conn, "UPDATE file SET partial_hash = NULL");
    for bad in ["zeroblob(32)", "zeroblob(34)", "zeroblob(0)", "x''"] {
        refuses(
            &conn,
            &format!("UPDATE file SET partial_hash = {bad}"),
            "CHECK",
        );
    }
    refuses(
        &conn,
        "UPDATE file SET partial_hash = 'text'",
        "cannot store TEXT value in BLOB column",
    );
}
