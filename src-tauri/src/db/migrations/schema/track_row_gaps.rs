//! 0042: the missing-tags and missing-art flags of `track_row` (1cC-U1b).

use super::{accepts, db, db_at, one};
use crate::db::migrations::{run, MIGRATIONS};

fn due(conn: &rusqlite::Connection) -> Vec<i64> {
    conn.prepare("SELECT recording_id FROM search_due ORDER BY 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn an_upgrade_makes_every_library_track_due_and_no_other_track() {
    let (dir, conn) = db_at(MIGRATIONS.len() - 1);
    accepts(
        &conn,
        "INSERT INTO recording DEFAULT VALUES; INSERT INTO recording DEFAULT VALUES;
         INSERT INTO recording DEFAULT VALUES;
         INSERT INTO library_track (recording_id, kind, source_status)
             VALUES (1, 'linked', 'missing'), (3, 'linked', 'missing');
         DELETE FROM search_due;",
    );
    drop(conn);
    let mut conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
    run(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(due(&conn), vec![1, 3], "the Library tracks, whose bits are unknown");
}

#[test]
fn a_merge_made_or_undone_marks_the_track_it_went_into_due() {
    let (_dir, conn) = db();
    accepts(
        &conn,
        "INSERT INTO recording DEFAULT VALUES; INSERT INTO recording DEFAULT VALUES;
         DELETE FROM search_due;",
    );
    accepts(
        &conn,
        "INSERT INTO track_merge (recording_id, into_id) VALUES (2, 1)",
    );
    assert_eq!(due(&conn), vec![1]);
    accepts(&conn, "DELETE FROM search_due");
    accepts(&conn, "DELETE FROM track_merge");
    assert_eq!(due(&conn), vec![1]);
    let flags: i64 = one(&conn, "SELECT count(*) FROM track_row");
    assert_eq!(flags, 0, "the triggers only note which track");
}
