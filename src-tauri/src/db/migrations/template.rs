//! Test databases, built once per test process and copied (audit 1cB, A10
//! and B15).
//!
//! Running the 40 migrations costs about two seconds on CI (each one is a
//! committed transaction on a file, and Windows scans every new file), and
//! about 450 tests used to do it for a database of their own. Here each
//! schema version a test asks for is built once, by the real [`run`], and
//! kept in memory; a test then writes its own copy of the file. So tests
//! stay independent (no file is shared; only the bytes it starts from are).
//!
//! The tests that are about the migrations themselves don't use this: they
//! apply them step by step or in full, on their own file (see
//! `migrations::tests`, `append_only` and the `schema` modules that say
//! "survive every later migration"). A test that starts from an older
//! version and applies the migration under test on top still runs that
//! migration for real.

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use rusqlite::Connection;

use super::{run, MIGRATIONS};
use crate::db::Writer;

/// One cell per schema version: the database file after the first `n`
/// migrations, journal mode left at the default (as a plain `Connection`
/// that ran the migrations leaves it).
static PREFIXES: [OnceLock<Vec<u8>>; MIGRATIONS.len() + 1] =
    [const { OnceLock::new() }; MIGRATIONS.len() + 1];

/// The database the writer leaves after opening a new file, every
/// migration applied: write-ahead log mode, log folded in.
static WRITER_DATABASE: OnceLock<Vec<u8>> = OnceLock::new();

/// The bytes of a database file with the first `version` real migrations
/// applied. Built on first use from the one before it (the same step the
/// app takes when it upgrades), so every version costs one migration, once.
fn prefix(version: usize) -> &'static [u8] {
    assert!(
        version <= MIGRATIONS.len(),
        "there are only {} migrations",
        MIGRATIONS.len()
    );
    PREFIXES[version].get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("template.db");
        if version > 0 {
            fs::write(&path, prefix(version - 1)).unwrap();
        }
        let mut conn = Connection::open(&path).unwrap();
        // Nothing here is under test, and the file is copied, not kept
        // open: skip the journal and the flushes that make each migration
        // slow. What is written is the same.
        conn.execute_batch("PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;")
            .unwrap();
        run(&mut conn, &MIGRATIONS[..version]).unwrap();
        drop(conn);
        fs::read(&path).unwrap()
    })
}

/// Writes a database with the first `version` real migrations applied to
/// `path`, which must not exist yet. Open it with a plain connection and
/// switch foreign keys on, as the writer does.
pub(crate) fn seed_prefix(path: &Path, version: usize) {
    fs::write(path, prefix(version)).unwrap();
}

/// The database [`Writer::open`] makes in a new file: built once through
/// the real open (WAL mode, every migration, the log folded in), then
/// closed.
fn writer_database() -> &'static [u8] {
    WRITER_DATABASE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::write_guard::test_path(dir.path(), "template.db");
        let writer = Writer::open_with(&path, MIGRATIONS).unwrap();
        // Closed, so SQLite has folded the log in and removed its files:
        // the one file is the whole database.
        writer.close();
        drop(writer);
        fs::read(dir.path().join("template.db")).unwrap()
    })
}

/// Puts a ready-migrated database where [`Writer::open`] is about to
/// create one. Does nothing if something is there already (a test that
/// reopens a file, or one that made it a database of its own) or the
/// folder is missing (the open then fails as it should).
pub(crate) fn seed_new_writer_database(path: &Path) {
    // `symlink_metadata`, not `exists`: a link that leads nowhere is
    // something (the write guard refuses it), and writing through it would
    // create its target.
    if fs::symlink_metadata(path).is_ok() || !path.parent().is_some_and(Path::is_dir) {
        return;
    }
    // A copy that can't be made is no reason to fail: the open then makes
    // the database the slow way.
    if fs::write(path, writer_database()).is_err() {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// What a database is, for comparing two: the schema (every table,
    /// index and trigger as SQLite stored its text), the migrations
    /// recorded, and every table's row count.
    #[derive(Debug, PartialEq)]
    struct Described {
        schema: Vec<(String, String)>,
        recorded: Vec<(i64, String, String)>,
        rows: BTreeMap<String, i64>,
    }

    fn describe(conn: &Connection) -> Described {
        let schema = conn
            .prepare("SELECT name, COALESCE(sql, '') FROM sqlite_master ORDER BY type, name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<(String, String)>, _>>()
            .unwrap();
        let recorded = conn
            .prepare("SELECT version, name, checksum FROM schema_migration ORDER BY version")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut rows = BTreeMap::new();
        for (name, _) in &schema {
            let is_table: bool = conn
                .query_row(
                    "SELECT type = 'table' FROM sqlite_master WHERE name = ?1",
                    [name],
                    |r| r.get(0),
                )
                .unwrap();
            // Virtual tables keep their shadow tables, which are tables too.
            if is_table {
                let count = conn
                    .query_row(&format!("SELECT COUNT(*) FROM \"{name}\""), [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                rows.insert(name.clone(), count);
            }
        }
        Described {
            schema,
            recorded,
            rows,
        }
    }

    #[test]
    fn a_copied_template_is_the_database_the_real_migrations_make() {
        // The one test that compares a template with a database made the
        // slow way, so the others can trust the copy.
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.db");
        let mut conn = Connection::open(&real).unwrap();
        run(&mut conn, MIGRATIONS).unwrap();

        let copy = dir.path().join("copy.db");
        seed_prefix(&copy, MIGRATIONS.len());
        let copied = Connection::open(&copy).unwrap();
        assert_eq!(describe(&copied), describe(&conn));
        // Nothing left to apply, and each migration recorded as it is now.
        let mut copied = copied;
        assert_eq!(run(&mut copied, MIGRATIONS).unwrap(), 0);
    }

    #[test]
    fn a_template_of_an_older_version_stops_there_and_the_next_migration_applies_on_top() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        seed_prefix(&path, 12);
        let mut conn = Connection::open(&path).unwrap();
        let versions: Vec<u32> = super::super::applied(&conn)
            .unwrap()
            .iter()
            .map(|a| a.version)
            .collect();
        assert_eq!(versions, (1..=12).collect::<Vec<u32>>());
        assert_eq!(
            run(&mut conn, MIGRATIONS).unwrap(),
            MIGRATIONS.len() - 12,
            "the later migrations were applied for real"
        );
    }

    #[test]
    fn a_writer_opened_on_a_new_file_gets_the_database_the_real_open_makes() {
        let dir = tempfile::tempdir().unwrap();
        // The real open: no seeding, so every migration runs.
        let real = Writer::open_with(
            &crate::write_guard::test_path(dir.path(), "real.db"),
            MIGRATIONS,
        )
        .unwrap();
        // The test open: a copy of the template.
        let copy_path = crate::write_guard::test_path(dir.path(), "copy.db");
        let copy = Writer::open(&copy_path).unwrap();
        let describe_of = |w: &Writer| w.call(|c| Ok(describe(c))).unwrap();
        assert_eq!(describe_of(&copy), describe_of(&real));
        let mode = |w: &Writer| -> String {
            w.call(|c| c.query_row("PRAGMA journal_mode", [], |r| r.get(0)))
                .unwrap()
        };
        assert_eq!(mode(&copy), "wal");
        assert_eq!(mode(&real), "wal");
    }

    #[test]
    fn two_writers_opened_from_the_template_do_not_share_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = Writer::open(&crate::write_guard::test_path(dir.path(), "a.db")).unwrap();
        let b = Writer::open(&crate::write_guard::test_path(dir.path(), "b.db")).unwrap();
        a.call(|c| c.execute("INSERT INTO recording DEFAULT VALUES", []))
            .unwrap();
        let in_b: i64 = b
            .call(|c| c.query_row("SELECT COUNT(*) FROM recording", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(in_b, 0);
    }

    #[test]
    fn an_existing_file_is_left_alone_and_a_missing_folder_still_fails_the_open() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("mine.db");
        fs::write(&existing, b"not a template").unwrap();
        seed_new_writer_database(&existing);
        assert_eq!(fs::read(&existing).unwrap(), b"not a template");

        let missing = dir.path().join("nowhere").join("x.db");
        seed_new_writer_database(&missing);
        assert!(!missing.exists());
        assert!(Writer::open(&crate::write_guard::test_path(dir.path(), "nowhere/x.db")).is_err());
    }
}
