//! 0010: `file.fingerprint_audio_hash`, for skipping a re-decode (1aC-10).

use super::files::with_a_file;
use super::{accepts, one, refuses};

#[test]
fn a_new_file_records_no_audio_hash_for_its_fingerprint() {
    let (_dir, conn) = with_a_file();
    let hash: Option<Vec<u8>> = one(&conn, "SELECT fingerprint_audio_hash FROM file");
    assert_eq!(hash, None);
}

#[test]
fn the_audio_hash_a_fingerprint_came_from_is_34_bytes_or_absent() {
    // The same form as `audio_hash`: definition, format, digest.
    let (_dir, conn) = with_a_file();
    accepts(
        &conn,
        "UPDATE file SET fingerprint_audio_hash = zeroblob(34)",
    );
    accepts(&conn, "UPDATE file SET fingerprint_audio_hash = NULL");
    for bad in ["zeroblob(32)", "zeroblob(33)", "zeroblob(0)"] {
        refuses(
            &conn,
            &format!("UPDATE file SET fingerprint_audio_hash = {bad}"),
            "CHECK",
        );
    }
}
