//! 0007: `file_stage`, what each later scan stage last did to each file.

use super::files::with_a_file;
use super::{accepts, one, refuses};

#[test]
fn a_stage_row_is_for_a_known_stage_and_status() {
    let (_dir, conn) = with_a_file();
    for stage in ["read", "hash", "fingerprint"] {
        accepts(
            &conn,
            &format!(
                "INSERT INTO file_stage (file_id, stage, version, status) VALUES (1, '{stage}', 1, 'done')"
            ),
        );
    }
    refuses(
        &conn,
        "INSERT INTO file_stage (file_id, stage, version, status) VALUES (1, 'walk', 1, 'done')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO file_stage (file_id, stage, version, status, reason)
         VALUES (1, 'read', 1, 'pending', 'x')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO file_stage (file_id, stage, version, status) VALUES (1, 'read', 0, 'done')",
        "CHECK",
    );
}

#[test]
fn a_file_has_one_row_per_stage() {
    let (_dir, conn) = with_a_file();
    accepts(
        &conn,
        "INSERT INTO file_stage (file_id, stage, version, status) VALUES (1, 'read', 1, 'done')",
    );
    refuses(
        &conn,
        "INSERT INTO file_stage (file_id, stage, version, status) VALUES (1, 'read', 2, 'done')",
        "UNIQUE",
    );
}

#[test]
fn a_reason_is_given_exactly_when_the_stage_did_not_finish_the_file() {
    let (_dir, conn) = with_a_file();
    refuses(
        &conn,
        "INSERT INTO file_stage (file_id, stage, version, status, reason)
         VALUES (1, 'read', 1, 'done', 'unreadable')",
        "CHECK",
    );
    for status in ["failed", "skipped"] {
        refuses(
            &conn,
            &format!(
                "INSERT INTO file_stage (file_id, stage, version, status) VALUES (1, 'read', 1, '{status}')"
            ),
            "CHECK",
        );
        refuses(
            &conn,
            &format!(
                "INSERT INTO file_stage (file_id, stage, version, status, reason)
                 VALUES (1, 'read', 1, '{status}', '')"
            ),
            "CHECK",
        );
    }
    accepts(
        &conn,
        "INSERT INTO file_stage (file_id, stage, version, status, reason)
         VALUES (1, 'hash', 1, 'failed', 'unreadable');
         INSERT INTO file_stage (file_id, stage, version, status, reason)
         VALUES (1, 'read', 1, 'skipped', 'online_only');",
    );
    let stamped: i64 = one(
        &conn,
        "SELECT COUNT(*) FROM file_stage WHERE done_at GLOB '????-??-??T??:??:??.???Z'",
    );
    assert_eq!(stamped, 2);
}

#[test]
fn a_stage_row_must_be_for_a_known_file_and_goes_when_the_file_row_goes() {
    let (_dir, conn) = with_a_file();
    refuses(
        &conn,
        "INSERT INTO file_stage (file_id, stage, version, status) VALUES (2, 'read', 1, 'done')",
        "FOREIGN KEY",
    );
    accepts(
        &conn,
        "INSERT INTO file_stage (file_id, stage, version, size, mtime, status)
         VALUES (1, 'read', 1, 100, 1000, 'done')",
    );
    accepts(&conn, "DELETE FROM file WHERE id = 1");
    let left: i64 = one(&conn, "SELECT COUNT(*) FROM file_stage");
    assert_eq!(left, 0);
}
