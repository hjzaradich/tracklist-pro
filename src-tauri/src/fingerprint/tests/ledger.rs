//! Asking the ledger whether one file is due costs the same wherever the
//! file is in the table. Database only: no audio.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rusqlite::Connection;

use crate::db::migrations::{template, MIGRATIONS};
use crate::fingerprint::ledger;
use crate::scan_state::due_sql;

/// A migrated database with one volume, one music folder and `files` files,
/// ids 1..=files.
fn db(files: i64) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    // A copy of the fully migrated database, built once per test run.
    template::seed_prefix(&path, MIGRATIONS.len());
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    conn.execute_batch(
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');
         BEGIN;",
    )
    .unwrap();
    for n in 0..files {
        conn.execute(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime)
             VALUES (1, ?1, ?1, 100, 1000)",
            [format!("{n}.mp3")],
        )
        .unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
    (dir, conn)
}

/// How many steps SQLite's virtual machine takes to answer
/// `ledger::due(conn, id)`.
fn steps_to_ask_about(conn: &Connection, id: i64) -> usize {
    let steps = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&steps);
    conn.progress_handler(
        1,
        Some(move || {
            counter.fetch_add(1, Ordering::Relaxed);
            false
        }),
    )
    .unwrap();
    let due = ledger::due(conn, id).unwrap();
    conn.progress_handler(1, None::<fn() -> bool>).unwrap();
    assert_eq!(due.map(|d| d.id), Some(id), "file {id} is due");
    steps.load(Ordering::Relaxed)
}

#[test]
fn asking_whether_one_file_is_due_starts_at_that_file_and_never_walks_the_table_from_the_start() {
    let (_dir, conn) = db(2000);
    // Warm the statement cache so compiling it isn't counted.
    steps_to_ask_about(&conn, 1);
    let first = steps_to_ask_about(&conn, 1);
    let last = steps_to_ask_about(&conn, 2000);
    // The fingerprint job asks this of every file it fingerprints: at
    // 100,000 files, a walk from the first file to this one for each is
    // minutes on the writer.
    assert!(
        last <= first + 50,
        "the last file took {last} steps, the first {first}"
    );
}

#[test]
fn the_due_query_pages_by_id_and_never_reads_the_file_table_in_full() {
    let (_dir, conn) = db(3);
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {}", due_sql()))
        .unwrap();
    let plan = stmt
        .query_map(
            rusqlite::params!["fingerprint", 1, None::<String>, "[2]", 1, 1],
            |r| r.get::<_, String>(3),
        )
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap();
    let file_steps: Vec<_> = plan
        .into_iter()
        .filter(|step| step.contains(" f ") || step.ends_with(" f"))
        .collect();
    assert_eq!(file_steps, ["SEARCH f USING INTEGER PRIMARY KEY (rowid>?)"]);
}
