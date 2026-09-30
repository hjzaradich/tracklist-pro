//! 0005: `crate`, `crate_entry`, `operation`, `change`, `job`, `setting`,
//! `service_optin` (ROADMAP 0.1, 1.14, §2).

use rusqlite::Connection;

use super::library::with_a_library_track;
use super::{accepts, columns, db, insert, one, refuses};

/// Folder 1 "Friday" holding static crate 2 "Warmup" and smart crate 3
/// "Peak", plus Library track 1.
fn with_crates() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = with_a_library_track();
    accepts(
        &conn,
        r#"INSERT INTO crate (kind, name) VALUES ('folder', 'Friday');
           INSERT INTO crate (parent_id, kind, name) VALUES (1, 'static', 'Warmup');
           INSERT INTO crate (parent_id, kind, name, rules) VALUES (1, 'smart', 'Peak', '{"energy_at_least": 7}');"#,
    );
    (dir, conn)
}

// Crates.

#[test]
fn a_crate_is_a_folder_a_static_crate_or_a_smart_crate() {
    let (_dir, conn) = db();
    refuses(
        &conn,
        "INSERT INTO crate (kind, name) VALUES ('playlist', 'x')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO crate (kind, name) VALUES ('static', '')",
        "CHECK",
    );
}

#[test]
fn only_smart_crates_have_rules_and_rules_are_json() {
    let (_dir, conn) = db();
    refuses(
        &conn,
        "INSERT INTO crate (kind, name, rules) VALUES ('static', 'a', '{}')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO crate (kind, name, rules) VALUES ('folder', 'a', '{}')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO crate (kind, name) VALUES ('smart', 'a')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO crate (kind, name, rules) VALUES ('smart', 'a', 'energy > 7')",
        "CHECK",
    );
}

#[test]
fn two_crates_in_one_folder_cannot_share_a_name() {
    // rekordbox replaces a same-name playlist in the same folder (§5.2).
    let (_dir, conn) = with_crates();
    refuses(
        &conn,
        "INSERT INTO crate (parent_id, kind, name) VALUES (1, 'static', 'Warmup')",
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO crate (kind, name) VALUES ('static', 'Friday')",
        "UNIQUE",
    );
    // The same name in another folder, or at the top, is fine.
    accepts(
        &conn,
        "INSERT INTO crate (kind, name) VALUES ('static', 'Warmup');
         INSERT INTO crate (kind, name) VALUES ('folder', 'Saturday');
         INSERT INTO crate (parent_id, kind, name) VALUES (5, 'static', 'Warmup');",
    );
}

#[test]
fn a_crate_can_only_be_inside_a_folder() {
    let (_dir, conn) = with_crates();
    refuses(
        &conn,
        "INSERT INTO crate (parent_id, kind, name) VALUES (2, 'static', 'Inside a crate')",
        "a crate can only be inside a folder",
    );
    refuses(
        &conn,
        "INSERT INTO crate (parent_id, kind, name) VALUES (99, 'static', 'Nowhere')",
        "a crate can only be inside a folder",
    );
    refuses(
        &conn,
        "UPDATE crate SET parent_id = 2 WHERE id = 3",
        "a crate can only be inside a folder",
    );
}

#[test]
fn a_folder_cannot_be_moved_inside_itself() {
    let (_dir, conn) = with_crates();
    accepts(
        &conn,
        "INSERT INTO crate (parent_id, kind, name) VALUES (1, 'folder', 'Sub');   -- 4
         INSERT INTO crate (parent_id, kind, name) VALUES (4, 'folder', 'Deeper'); -- 5",
    );
    for target in [1, 4, 5] {
        refuses(
            &conn,
            &format!("UPDATE crate SET parent_id = {target} WHERE id = 1"),
            "a folder cannot be moved inside itself",
        );
    }
    // Moving a folder somewhere else is fine.
    accepts(
        &conn,
        "INSERT INTO crate (kind, name) VALUES ('folder', 'Other'); -- 6
         UPDATE crate SET parent_id = 6 WHERE id = 4;
         UPDATE crate SET parent_id = NULL WHERE id = 4;",
    );
}

#[test]
fn a_folder_with_crates_in_it_cannot_be_deleted() {
    let (_dir, conn) = with_crates();
    refuses(&conn, "DELETE FROM crate WHERE id = 1", "FOREIGN KEY");
}

#[test]
fn a_crate_cannot_change_kind() {
    let (_dir, conn) = with_crates();
    refuses(
        &conn,
        "UPDATE crate SET kind = 'folder' WHERE id = 2",
        "a crate cannot change kind",
    );
    refuses(
        &conn,
        "UPDATE crate SET kind = 'static', rules = NULL WHERE id = 3",
        "a crate cannot change kind",
    );
}

// Crate entries.

const FITS: &str = "a folder holds no tracks, a static crate holds members, and a smart crate holds always-include and never-include tracks";

#[test]
fn a_static_crate_holds_members_only() {
    let (_dir, conn) = with_crates();
    accepts(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (2, 1)",
    );
    refuses(
        &conn,
        "UPDATE crate_entry SET kind = 'always_include'",
        FITS,
    );
}

#[test]
fn a_smart_crate_holds_always_include_and_never_include_tracks_only() {
    let (_dir, conn) = with_crates();
    refuses(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (3, 1)",
        FITS,
    );
    accepts(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id, kind) VALUES (3, 1, 'never_include');
         UPDATE crate_entry SET kind = 'always_include';",
    );
}

#[test]
fn a_folder_holds_no_tracks() {
    let (_dir, conn) = with_crates();
    for kind in ["member", "always_include", "never_include"] {
        refuses(
            &conn,
            &format!(
                "INSERT INTO crate_entry (crate_id, library_track_id, kind) VALUES (1, 1, '{kind}')"
            ),
            FITS,
        );
    }
    accepts(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (2, 1)",
    );
    refuses(&conn, "UPDATE crate_entry SET crate_id = 1", FITS);
}

#[test]
fn a_track_is_in_a_crate_once() {
    let (_dir, conn) = with_crates();
    let sql = "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (2, 1)";
    accepts(&conn, sql);
    refuses(&conn, sql, "UNIQUE");
}

#[test]
fn only_library_tracks_go_in_crates() {
    let (_dir, conn) = with_crates();
    // Track 2 is in All music but not in the Library, so it has no
    // library_track row to point at.
    refuses(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (2, 2)",
        "FOREIGN KEY",
    );
}

#[test]
fn deleting_a_crate_empties_it_but_keeps_its_tracks() {
    let (_dir, conn) = with_crates();
    accepts(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (2, 1);
         DELETE FROM crate WHERE id = 2;",
    );
    let (entries, tracks): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM crate_entry), (SELECT COUNT(*) FROM library_track)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((entries, tracks), (0, 1));
}

#[test]
fn a_library_track_in_a_crate_cannot_be_deleted_by_accident() {
    let (_dir, conn) = with_crates();
    accepts(
        &conn,
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (2, 1)",
    );
    refuses(&conn, "DELETE FROM library_track", "FOREIGN KEY");
}

// The operation log.

#[test]
fn an_operation_stores_no_ui_text_only_what_the_ui_needs_to_describe_it() {
    // UI strings go through i18next; a stored English summary would bypass it.
    let (_dir, conn) = db();
    let cols = columns(&conn, "operation");
    assert!(
        !cols.iter().any(|c| c == "summary" || c == "description"),
        "{cols:?}"
    );
    accepts(
        &conn,
        r#"INSERT INTO operation (kind, details) VALUES ('add_to_crate', '{"crate": 2, "tracks": 3}')"#,
    );
    refuses(
        &conn,
        "INSERT INTO operation (kind, details) VALUES ('add_to_crate', 'Added 3 tracks')",
        "CHECK",
    );
    refuses(&conn, "INSERT INTO operation (kind) VALUES ('')", "CHECK");
}

#[test]
fn a_change_belongs_to_an_operation_and_goes_with_it() {
    let (_dir, conn) = db();
    let op = insert(&conn, "INSERT INTO operation (kind) VALUES ('edit')");
    accepts(
        &conn,
        &format!(
            "INSERT INTO change (operation_id, entity, entity_id, action, field, before, after)
             VALUES ({op}, 'recording', 1, 'set', 'title', 'Old', 'New')"
        ),
    );
    refuses(
        &conn,
        "INSERT INTO change (operation_id, entity, entity_id, action, field, before, after)
         VALUES (99, 'recording', 1, 'set', 'title', 'Old', 'New')",
        "FOREIGN KEY",
    );
    accepts(&conn, "DELETE FROM operation");
    let left: i64 = one(&conn, "SELECT COUNT(*) FROM change");
    assert_eq!(left, 0);
}

fn change(op: i64, action: &str, before: &str, after: &str) -> String {
    format!(
        "INSERT INTO change (operation_id, entity, entity_id, action, field, before, after)
         VALUES ({op}, 'recording', 1, {action}, 'title', {before}, {after})"
    )
}

#[test]
fn a_change_says_whether_a_field_was_set_or_its_row_created_or_removed() {
    // So a NULL before or after is always a real NULL value, and undo can
    // tell "set to empty" from "row created" or "row removed".
    let (_dir, conn) = db();
    let op = insert(&conn, "INSERT INTO operation (kind) VALUES ('edit')");
    for action in ["NULL", "'update'", "'Set'"] {
        refuses(&conn, &change(op, action, "'a'", "'b'"), "");
    }
    // Setting a field to NULL, or from NULL, is a set like any other.
    accepts(&conn, &change(op, "'set'", "'Old'", "NULL"));
    accepts(&conn, &change(op, "'set'", "NULL", "'New'"));
    // A created row had no value before; its new value may be NULL.
    accepts(&conn, &change(op, "'insert'", "NULL", "'New'"));
    accepts(&conn, &change(op, "'insert'", "NULL", "NULL"));
    refuses(&conn, &change(op, "'insert'", "'Old'", "'New'"), "CHECK");
    // A removed row has no value after; its old value may be NULL.
    accepts(&conn, &change(op, "'delete'", "'Old'", "NULL"));
    accepts(&conn, &change(op, "'delete'", "NULL", "NULL"));
    refuses(&conn, &change(op, "'delete'", "'Old'", "'New'"), "CHECK");
}

#[test]
fn a_set_change_changes_something() {
    let (_dir, conn) = db();
    let op = insert(&conn, "INSERT INTO operation (kind) VALUES ('edit')");
    for (before, after) in [("'same'", "'same'"), ("NULL", "NULL")] {
        refuses(&conn, &change(op, "'set'", before, after), "CHECK");
    }
}

// Jobs.

#[test]
fn a_new_job_is_queued_and_has_not_finished() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        r#"INSERT INTO job (kind, target) VALUES ('scan', '{"music_folder": 1}')"#,
    );
    let (status, attempts): (String, i64) = conn
        .query_row("SELECT status, attempts FROM job", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((status.as_str(), attempts), ("queued", 0));
}

#[test]
fn a_job_status_is_one_of_five_and_a_job_has_finished_exactly_when_it_is_done_failed_or_cancelled()
{
    let (_dir, conn) = db();
    accepts(&conn, "INSERT INTO job (kind) VALUES ('scan')");
    refuses(&conn, "UPDATE job SET status = 'paused'", "CHECK");
    refuses(&conn, "UPDATE job SET status = 'done'", "CHECK");
    refuses(
        &conn,
        "UPDATE job SET status = 'running', finished_at = '2026-09-28T00:00:00Z'",
        "CHECK",
    );
    accepts(
        &conn,
        "UPDATE job SET status = 'running', started_at = '2026-09-28T00:00:00Z'",
    );
    for status in ["done", "cancelled"] {
        accepts(
            &conn,
            &format!("UPDATE job SET status = '{status}', finished_at = '2026-09-28T00:00:01Z'"),
        );
    }
}

#[test]
fn a_failed_job_says_why() {
    let (_dir, conn) = db();
    accepts(&conn, "INSERT INTO job (kind) VALUES ('hash')");
    refuses(
        &conn,
        "UPDATE job SET status = 'failed', finished_at = '2026-09-28T00:00:00Z'",
        "CHECK",
    );
    accepts(
        &conn,
        "UPDATE job SET status = 'failed', finished_at = '2026-09-28T00:00:00Z', error = 'disk unplugged'",
    );
}

#[test]
fn job_fields_have_sane_shapes() {
    let (_dir, conn) = db();
    accepts(&conn, "INSERT INTO job (kind) VALUES ('fingerprint')");
    refuses(&conn, "INSERT INTO job (kind) VALUES ('')", "CHECK");
    refuses(&conn, "UPDATE job SET progress = 1.5", "CHECK");
    refuses(&conn, "UPDATE job SET attempts = -1", "CHECK");
    refuses(&conn, "UPDATE job SET target = 'file 1'", "CHECK");
}

#[test]
fn the_next_job_is_found_through_the_queue_index() {
    let (_dir, conn) = db();
    let mut stmt = conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT id FROM job WHERE status = 'queued'
             ORDER BY priority DESC, id LIMIT 1",
        )
        .unwrap();
    let plan: Vec<String> = stmt
        .query_map([], |r| r.get(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(plan.iter().any(|d| d.contains("job_queue")), "{plan:?}");
}

// Settings and opt-ins.

#[test]
fn a_setting_is_stored_once_as_json() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        r#"INSERT INTO setting (key, value) VALUES ('key_notation', '"camelot"')"#,
    );
    refuses(
        &conn,
        r#"INSERT INTO setting (key, value) VALUES ('key_notation', '"musical"')"#,
        "UNIQUE",
    );
    refuses(
        &conn,
        "INSERT INTO setting (key, value) VALUES ('theme', 'dark')",
        "CHECK",
    );
    refuses(
        &conn,
        "INSERT INTO setting (key, value) VALUES ('', '1')",
        "CHECK",
    );
}

#[test]
fn a_fresh_database_is_opted_in_to_no_service() {
    // Offline by default (ROADMAP §1.1).
    let (_dir, conn) = db();
    let enabled: i64 = one(
        &conn,
        "SELECT COUNT(*) FROM service_optin WHERE enabled <> 0",
    );
    assert_eq!(enabled, 0);
}

#[test]
fn a_service_is_not_opted_in_unless_the_user_opts_in() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO service_optin (service) VALUES ('musicbrainz')",
    );
    let (enabled, at): (i64, Option<String>) = conn
        .query_row(
            "SELECT enabled, enabled_at FROM service_optin WHERE service = 'musicbrainz'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((enabled, at), (0, None));
}

#[test]
fn opting_in_records_when_and_opting_out_clears_it() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO service_optin (service) VALUES ('update_check')",
    );
    refuses(&conn, "UPDATE service_optin SET enabled = 1", "CHECK");
    accepts(
        &conn,
        "UPDATE service_optin SET enabled = 1, enabled_at = '2026-09-28T00:00:00Z'",
    );
    refuses(&conn, "UPDATE service_optin SET enabled = 0", "CHECK");
    accepts(
        &conn,
        "UPDATE service_optin SET enabled = 0, enabled_at = NULL",
    );
    refuses(
        &conn,
        "UPDATE service_optin SET enabled = 2, enabled_at = '2026-09-28T00:00:00Z'",
        "CHECK",
    );
}

#[test]
fn a_service_is_listed_once_by_a_lowercase_name() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO service_optin (service) VALUES ('model_download')",
    );
    refuses(
        &conn,
        "INSERT INTO service_optin (service) VALUES ('model_download')",
        "UNIQUE",
    );
    for bad in ["MusicBrainz", "cover art", "", "discogs-api"] {
        refuses(
            &conn,
            &format!("INSERT INTO service_optin (service) VALUES ('{bad}')"),
            "CHECK",
        );
    }
}
