//! Schema migrations (ROADMAP §0.1, §5.6).
//!
//! The migration files live in `src-tauri/migrations/` and are embedded in
//! the binary. The writer applies the pending ones in number order when it
//! opens the database, each in its own transaction, and records each one in
//! the `schema_migration` table. The rules for adding one are in
//! `src-tauri/migrations/README.md`.

#[cfg(test)]
mod append_only;
#[cfg(test)]
mod schema;

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::DbError;
use crate::write_guard;

/// One migration: its file name and its SQL.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// The file name, e.g. `0001_init.sql`.
    pub name: &'static str,
    pub sql: &'static str,
}

/// Embeds `src-tauri/migrations/<name>.sql`.
macro_rules! migration {
    ($name:literal) => {
        Migration {
            name: concat!($name, ".sql"),
            sql: include_str!(concat!("../../../migrations/", $name, ".sql")),
        }
    };
}

/// Every migration, in order. Append new ones at the end; never edit,
/// rename or remove one that's on `main`.
#[rustfmt::skip] // one per line, so parallel additions merge cleanly
pub const MIGRATIONS: &[Migration] = &[
    migration!("0001_init"),
    migration!("0002_volume_music_folder_file"),
    migration!("0003_recording_analysis"),
    migration!("0004_library_track_sync"),
    migration!("0005_crate_operation_job_setting"),
    migration!("0006_file_id"),
    migration!("0007_file_stage"),
    migration!("0008_scan_follow_ups"),
    migration!("0009_partial_hash"),
    migration!("0010_fingerprint_audio_hash"),
    migration!("0011_relink_probable"),
    migration!("0012_relink_audio_hash"),
    migration!("0013_library_upkeep"),
];

/// The table that records which migrations a database has had.
const CREATE_BOOKKEEPING: &str = "
    CREATE TABLE IF NOT EXISTS schema_migration (
        version    INTEGER PRIMARY KEY,
        name       TEXT NOT NULL,
        checksum   TEXT NOT NULL,
        applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    )";

impl Migration {
    /// The number the file name starts with: `0007_x.sql` is version 7.
    ///
    /// Panics on a malformed name. The names are compile-time constants,
    /// and a test checks every one.
    pub fn version(&self) -> u32 {
        parse_version(self.name)
            .unwrap_or_else(|| panic!("malformed migration name {:?}", self.name))
    }

    /// The SHA-256 of the SQL, as lowercase hex. Line endings are
    /// normalized first, so a CRLF checkout hashes the same as an LF one.
    pub fn checksum(&self) -> String {
        let normalized = self.sql.replace("\r\n", "\n");
        Sha256::digest(normalized.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// The version in a well-formed migration file name, `NNNN_words.sql`
/// with lowercase words joined by `_`. `None` if the name is malformed.
fn parse_version(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".sql")?;
    let (number, words) = stem.split_once('_')?;
    let well_formed = number.len() == 4
        && number.bytes().all(|b| b.is_ascii_digit())
        && !words.is_empty()
        && words.split('_').all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    well_formed.then(|| number.parse().ok()).flatten()
}

/// Why the migrations could not be brought up to date.
#[derive(Debug)]
pub enum MigrationError {
    /// A statement in the migration failed. The migration was rolled back,
    /// so the database is still at the version before it.
    Failed {
        name: String,
        error: rusqlite::Error,
    },
    /// The migration has its own `BEGIN`, `COMMIT`, `END`, `ROLLBACK`,
    /// `SAVEPOINT` or `RELEASE`. It was refused before any of it ran.
    /// Migration files must not control the transaction the runner wraps
    /// them in. (A trigger's `BEGIN … END` body is fine.)
    TransactionStatement { name: String },
    /// The migration left rows pointing at rows that don't exist. It was
    /// rolled back.
    BrokeForeignKeys { name: String },
    /// The database records this migration with different content than the
    /// one this build carries: an applied migration was edited.
    Edited { name: String },
    /// The database records a migration this build doesn't have, so it was
    /// last opened by a newer version of the app.
    Unknown { name: String },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationError::Failed { name, error } => {
                write!(f, "migration {name} failed and was rolled back: {error}")
            }
            MigrationError::TransactionStatement { name } => write!(
                f,
                "migration {name} has its own BEGIN, COMMIT, END, ROLLBACK, SAVEPOINT or RELEASE; migration files must not"
            ),
            MigrationError::BrokeForeignKeys { name } => write!(
                f,
                "migration {name} left broken foreign key references and was rolled back"
            ),
            MigrationError::Edited { name } => write!(
                f,
                "migration {name} was changed after this database applied it"
            ),
            MigrationError::Unknown { name } => write!(
                f,
                "the database has migration {name}, which this version of the app doesn't know; it was made by a newer version"
            ),
        }
    }
}

impl std::error::Error for MigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MigrationError::Failed { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl From<MigrationError> for DbError {
    fn from(e: MigrationError) -> Self {
        DbError::Migration(e)
    }
}

/// A migration a database has had, as recorded in `schema_migration`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub version: u32,
    pub name: String,
    pub checksum: String,
}

/// The migrations `conn`'s database has had, oldest first. Empty for a
/// database no runner has touched.
pub fn applied(conn: &Connection) -> rusqlite::Result<Vec<Applied>> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migration')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut stmt =
        conn.prepare("SELECT version, name, checksum FROM schema_migration ORDER BY version")?;
    let rows = stmt.query_map([], |r| {
        Ok(Applied {
            version: r.get(0)?,
            name: r.get(1)?,
            checksum: r.get(2)?,
        })
    })?;
    rows.collect()
}

/// Brings `conn`'s database up to date with `migrations`, which must be in
/// version order. Returns how many this call applied.
///
/// First checks that every migration the database already has matches the
/// one in `migrations`, byte for byte. Then applies each pending one in its
/// own transaction. On the first failure it stops: the failed migration is
/// rolled back, the ones before it stay applied.
///
/// Leaves foreign key enforcement off; the caller turns it back on.
pub fn run(conn: &mut Connection, migrations: &[Migration]) -> Result<usize, DbError> {
    // A no-op inside a transaction, so it's set here, before any begins.
    conn.pragma_update(None, "foreign_keys", false)?;
    conn.execute_batch(CREATE_BOOKKEEPING)?;

    let applied = applied(conn)?;
    for done in &applied {
        let known = migrations.iter().find(|m| m.version() == done.version);
        match known {
            None => {
                return Err(MigrationError::Unknown {
                    name: done.name.clone(),
                }
                .into())
            }
            Some(m) if m.name != done.name || m.checksum() != done.checksum => {
                return Err(MigrationError::Edited {
                    name: done.name.clone(),
                }
                .into());
            }
            Some(_) => {}
        }
    }

    let current = applied.last().map_or(0, |a| a.version);
    let mut count = 0;
    for m in migrations.iter().filter(|m| m.version() > current) {
        if apply(conn, m)? {
            count += 1;
        }
    }
    Ok(count)
}

/// Applies one migration and records it, in one transaction. Returns
/// `false` if another connection (a second app instance starting at the
/// same moment) applied it first.
fn apply(conn: &mut Connection, m: &Migration) -> Result<bool, DbError> {
    let name = || m.name.to_string();
    // Dropping `tx` without committing rolls it back.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    // Look again now that this connection holds the write lock: someone
    // may have applied it since `run` read the table.
    let recorded: Option<(String, String)> = tx
        .query_row(
            "SELECT name, checksum FROM schema_migration WHERE version = ?1",
            [m.version()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((done_name, done_checksum)) = recorded {
        if done_name != m.name || done_checksum != m.checksum() {
            return Err(MigrationError::Edited { name: done_name }.into());
        }
        return Ok(false);
    }

    // Refuse transaction statements as SQLite prepares them, before any
    // runs: a COMMIT in the file would otherwise commit half a migration.
    // The guard is dropped at the end of this block, before anything else
    // runs: the rollback on drop and the commit below are transaction
    // statements too.
    let (result, refused) = {
        let guard = RefuseTransactionStatements::install(&tx)?;
        (tx.execute_batch(m.sql), guard.refused())
    };
    if refused || tx.is_autocommit() {
        return Err(MigrationError::TransactionStatement { name: name() }.into());
    }
    result.map_err(|error| MigrationError::Failed {
        name: name(),
        error,
    })?;
    let broken: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM pragma_foreign_key_check)",
        [],
        |r| r.get(0),
    )?;
    if broken {
        return Err(MigrationError::BrokeForeignKeys { name: name() }.into());
    }
    tx.execute(
        "INSERT INTO schema_migration (version, name, checksum) VALUES (?1, ?2, ?3)",
        (m.version(), m.name, m.checksum()),
    )?;
    tx.commit()?;
    Ok(true)
}

/// While alive, refuses every transaction statement (`BEGIN`, `COMMIT`,
/// `END`, `ROLLBACK`, `SAVEPOINT`, `RELEASE`) on its connection and
/// remembers that it did. Dropping it removes the refusal, however the
/// scope ends: normally, by an early return or `?`, or by a panic. Left in
/// place, it would stop the writer from ever beginning or committing a
/// transaction again.
///
/// A connection has one authorizer, and the write guard's (no `ATTACH` of
/// a file) must hold throughout: this one applies the guard's rule first,
/// and dropping it puts the guard's rule back rather than clearing it.
struct RefuseTransactionStatements<'c> {
    conn: &'c Connection,
    refused: Arc<AtomicBool>,
}

impl<'c> RefuseTransactionStatements<'c> {
    fn install(conn: &'c Connection) -> rusqlite::Result<Self> {
        let refused = Arc::new(AtomicBool::new(false));
        let flag = refused.clone();
        conn.authorizer(Some(move |ctx: AuthContext<'_>| {
            if write_guard::sqlite_rule(&ctx.action) == Authorization::Deny {
                return Authorization::Deny;
            }
            match ctx.action {
                AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } => {
                    flag.store(true, Ordering::SeqCst);
                    Authorization::Deny
                }
                _ => Authorization::Allow,
            }
        }))?;
        Ok(RefuseTransactionStatements { conn, refused })
    }

    /// Whether a transaction statement was refused so far.
    fn refused(&self) -> bool {
        self.refused.load(Ordering::SeqCst)
    }
}

impl Drop for RefuseTransactionStatements<'_> {
    fn drop(&mut self) {
        // Replacing an authorizer only fails on API misuse (a closed
        // connection), and there is nothing to do about it in a drop.
        let _ = write_guard::install_sqlite_rule(self.conn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn table_exists(conn: &Connection, table: &str) -> bool {
        conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn versions(conn: &Connection) -> Vec<u32> {
        applied(conn).unwrap().iter().map(|a| a.version).collect()
    }

    fn open_temp() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("test.db")).unwrap();
        (dir, conn)
    }

    const CREATE_A: Migration = Migration {
        name: "0001_create_a.sql",
        sql: "CREATE TABLE a (x INTEGER); INSERT INTO a VALUES (1);",
    };
    const CREATE_B: Migration = Migration {
        name: "0002_create_b.sql",
        sql: "CREATE TABLE b (y INTEGER);",
    };
    /// Creates a table and fills it, then hits a syntax error.
    const FAILS_HALFWAY: Migration = Migration {
        name: "0002_fails_halfway.sql",
        sql: "CREATE TABLE b (y INTEGER); INSERT INTO a VALUES (2); THIS IS NOT SQL;",
    };

    #[test]
    fn a_fresh_database_gets_every_migration_in_order_and_each_is_recorded() {
        let (_dir, mut conn) = open_temp();
        let n = run(&mut conn, &[CREATE_A, CREATE_B]).unwrap();
        assert_eq!(n, 2);
        assert!(table_exists(&conn, "a") && table_exists(&conn, "b"));
        let applied = applied(&conn).unwrap();
        assert_eq!(
            applied,
            vec![
                Applied {
                    version: 1,
                    name: CREATE_A.name.into(),
                    checksum: CREATE_A.checksum(),
                },
                Applied {
                    version: 2,
                    name: CREATE_B.name.into(),
                    checksum: CREATE_B.checksum(),
                },
            ]
        );
    }

    #[test]
    fn running_again_applies_nothing_and_changes_nothing() {
        let (_dir, mut conn) = open_temp();
        run(&mut conn, &[CREATE_A]).unwrap();
        assert_eq!(run(&mut conn, &[CREATE_A]).unwrap(), 0);
        // CREATE_A inserts a row; it would be there twice if re-applied.
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM a", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(versions(&conn), vec![1]);
    }

    #[test]
    fn only_the_new_migrations_are_applied_to_an_older_database() {
        let (_dir, mut conn) = open_temp();
        run(&mut conn, &[CREATE_A]).unwrap();
        assert_eq!(run(&mut conn, &[CREATE_A, CREATE_B]).unwrap(), 1);
        assert_eq!(versions(&conn), vec![1, 2]);
        assert!(table_exists(&conn, "b"));
    }

    #[test]
    fn a_failed_migration_leaves_the_database_at_the_previous_version() {
        let (_dir, mut conn) = open_temp();
        let result = run(&mut conn, &[CREATE_A, FAILS_HALFWAY]);
        match result {
            Err(DbError::Migration(MigrationError::Failed { name, .. })) => {
                assert_eq!(name, FAILS_HALFWAY.name);
            }
            other => panic!("expected the second migration to fail, got {other:?}"),
        }
        // The first migration stays applied; nothing of the second is left:
        // not its table, not its insert, not its record.
        assert_eq!(versions(&conn), vec![1]);
        assert!(table_exists(&conn, "a"));
        assert!(
            !table_exists(&conn, "b"),
            "the failed migration's table is there"
        );
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM a", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "the failed migration's insert was kept");
        assert!(conn.is_autocommit(), "a transaction was left open");

        // Once fixed, the next run picks up where it stopped.
        assert_eq!(run(&mut conn, &[CREATE_A, CREATE_B]).unwrap(), 1);
        assert_eq!(versions(&conn), vec![1, 2]);
    }

    fn assert_refused_before_anything_committed(sql: &'static str) {
        let (_dir, mut conn) = open_temp();
        let sneaky = Migration {
            name: "0001_sneaky.sql",
            sql,
        };
        let result = run(&mut conn, &[sneaky]);
        assert!(
            matches!(
                result,
                Err(DbError::Migration(
                    MigrationError::TransactionStatement { .. }
                ))
            ),
            "{sql}: got {result:?}"
        );
        assert_eq!(versions(&conn), Vec::<u32>::new(), "{sql}: recorded");
        assert!(!table_exists(&conn, "a"), "{sql}: table a was committed");
        assert!(!table_exists(&conn, "b"), "{sql}: table b was committed");
        assert!(conn.is_autocommit(), "{sql}: a transaction was left open");
        // Nothing is left behind to trip up the fixed migration.
        let fixed = Migration {
            name: "0001_fixed.sql",
            sql: "CREATE TABLE a (x INTEGER);",
        };
        assert_eq!(run(&mut conn, &[fixed]).unwrap(), 1, "{sql}");
    }

    #[test]
    fn a_migration_with_its_own_commit_is_refused_before_anything_is_committed() {
        assert_refused_before_anything_committed("CREATE TABLE a (x INTEGER); COMMIT;");
    }

    #[test]
    fn a_migration_that_commits_and_begins_again_is_refused_before_anything_is_committed() {
        assert_refused_before_anything_committed(
            "CREATE TABLE a (x INTEGER); COMMIT; BEGIN; CREATE TABLE b (y INTEGER);",
        );
    }

    #[test]
    fn a_migration_with_any_other_transaction_statement_is_refused() {
        for sql in [
            "CREATE TABLE a (x INTEGER); END;",
            "CREATE TABLE a (x INTEGER); ROLLBACK;",
            "CREATE TABLE a (x INTEGER); ROLLBACK; BEGIN; CREATE TABLE b (y INTEGER);",
            "SAVEPOINT s; CREATE TABLE a (x INTEGER); RELEASE s;",
            "CREATE TABLE a (x INTEGER); SAVEPOINT s; CREATE TABLE b (y INTEGER); ROLLBACK TO s;",
            "BEGIN; CREATE TABLE a (x INTEGER);",
        ] {
            assert_refused_before_anything_committed(sql);
        }
    }

    #[test]
    fn a_migration_with_a_trigger_body_is_applied() {
        // BEGIN ... END inside CREATE TRIGGER is not a transaction statement.
        let (_dir, mut conn) = open_temp();
        let trigger = Migration {
            name: "0001_trigger.sql",
            sql: "CREATE TABLE a (x INTEGER);
                  CREATE TABLE log (x INTEGER);
                  CREATE TRIGGER a_log AFTER INSERT ON a BEGIN
                      INSERT INTO log VALUES (new.x);
                  END;",
        };
        assert_eq!(run(&mut conn, &[trigger]).unwrap(), 1);
        conn.execute("INSERT INTO a VALUES (7)", []).unwrap();
        let logged: i64 = conn
            .query_row("SELECT x FROM log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logged, 7);
    }

    /// Whether `conn` can begin, savepoint and commit, i.e. no refusal of
    /// transaction statements was left on it.
    fn can_run_transactions(conn: &Connection) -> bool {
        conn.execute_batch("BEGIN; SAVEPOINT s; RELEASE s; COMMIT;")
            .is_ok()
    }

    #[test]
    fn while_the_guard_is_in_place_transaction_statements_are_refused_and_noticed() {
        let (_dir, conn) = open_temp();
        let guard = RefuseTransactionStatements::install(&conn).unwrap();
        assert!(!guard.refused());
        conn.execute_batch("CREATE TABLE a (x INTEGER)").unwrap();
        assert!(!guard.refused(), "an ordinary statement counted as refused");
        for sql in [
            "BEGIN",
            "COMMIT",
            "ROLLBACK",
            "SAVEPOINT s",
            "RELEASE s",
            "END",
        ] {
            assert!(conn.execute_batch(sql).is_err(), "{sql} ran");
        }
        assert!(guard.refused());
        drop(guard);
        assert!(can_run_transactions(&conn));
    }

    #[test]
    fn the_guard_is_removed_when_its_scope_ends_early_with_an_error() {
        fn fails_halfway(conn: &Connection) -> rusqlite::Result<()> {
            let _guard = RefuseTransactionStatements::install(conn)?;
            conn.execute_batch("NOT SQL")?; // returns here
            unreachable!("the statement above fails");
        }
        let (_dir, conn) = open_temp();
        assert!(fails_halfway(&conn).is_err());
        assert!(can_run_transactions(&conn), "the guard outlived its scope");
    }

    #[test]
    fn the_guard_is_removed_when_its_scope_ends_in_a_panic() {
        let (_dir, conn) = open_temp();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = RefuseTransactionStatements::install(&conn).unwrap();
            panic!("blew up while the guard was in place");
        }));
        assert!(result.is_err());
        assert!(can_run_transactions(&conn), "the guard outlived the panic");
        // And the write guard's rule is back in place, not cleared.
        let target = _dir.path().join("elsewhere.db");
        let sql = format!(
            "ATTACH '{}' AS x",
            target.to_str().unwrap().replace('\'', "''")
        );
        assert!(
            conn.execute_batch(&sql).is_err(),
            "ATTACH allowed after the panic"
        );
        assert!(!target.exists());
    }

    #[test]
    fn after_every_way_a_migration_can_fail_the_connection_still_runs_transactions() {
        let failing = [
            Migration {
                name: "0002_syntax_error.sql",
                sql: "CREATE TABLE b (y INTEGER); NOT SQL;",
            },
            Migration {
                name: "0002_commits.sql",
                sql: "CREATE TABLE b (y INTEGER); COMMIT;",
            },
            Migration {
                name: "0002_savepoint.sql",
                sql: "SAVEPOINT s; CREATE TABLE b (y INTEGER);",
            },
            Migration {
                name: "0002_orphans.sql",
                sql: "CREATE TABLE b (y INTEGER REFERENCES a(x)); INSERT INTO b VALUES (99);",
            },
        ];
        for bad in failing {
            let (_dir, mut conn) = open_temp();
            let parent = Migration {
                name: "0001_a.sql",
                sql: "CREATE TABLE a (x INTEGER PRIMARY KEY);",
            };
            assert!(run(&mut conn, &[parent, bad]).is_err(), "{}", bad.name);
            assert!(can_run_transactions(&conn), "{}", bad.name);
        }
    }

    #[test]
    fn after_a_successful_migration_the_writer_runs_transactions() {
        let (_dir, mut conn) = open_temp();
        run(&mut conn, &[CREATE_A, CREATE_B]).unwrap();
        assert!(can_run_transactions(&conn));
    }

    #[test]
    fn a_migration_another_connection_applied_meanwhile_is_skipped_not_reapplied() {
        // Two app instances start at once. Both read the table and see
        // CREATE_A pending; the first applies it; the second must skip it
        // once it holds the write lock, not fail on "table a already exists".
        let (dir, mut first) = open_temp();
        let mut second = Connection::open(dir.path().join("test.db")).unwrap();
        run(&mut first, &[]).unwrap(); // bookkeeping table only
        assert!(versions(&second).is_empty(), "second saw it pending");
        run(&mut first, &[CREATE_A]).unwrap();
        assert!(!apply(&mut second, &CREATE_A).unwrap(), "applied twice");
        let rows: i64 = second
            .query_row("SELECT COUNT(*) FROM a", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(versions(&second), vec![1]);
    }

    #[test]
    fn many_connections_migrating_the_same_database_at_once_all_succeed() {
        use std::sync::{mpsc, Barrier};
        use std::time::Duration;
        const RUNNERS: usize = 6;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let barrier = Arc::new(Barrier::new(RUNNERS));
        let (done, results) = mpsc::channel();
        for _ in 0..RUNNERS {
            let (path, barrier, done) = (path.clone(), barrier.clone(), done.clone());
            std::thread::spawn(move || {
                let mut conn = Connection::open(&path).unwrap();
                conn.busy_timeout(Duration::from_secs(20)).unwrap();
                barrier.wait();
                let _ = done.send(run(&mut conn, &[CREATE_A, CREATE_B]).map_err(|e| e.to_string()));
            });
        }
        let mut total = 0;
        for _ in 0..RUNNERS {
            let applied = results
                .recv_timeout(Duration::from_secs(60))
                .expect("a runner hung")
                .expect("a runner failed");
            total += applied;
        }
        assert_eq!(total, 2, "each migration is applied exactly once");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(versions(&conn), vec![1, 2]);
    }

    #[test]
    fn a_migration_that_breaks_a_foreign_key_is_rolled_back() {
        let (_dir, mut conn) = open_temp();
        let parent = Migration {
            name: "0001_parent.sql",
            sql: "CREATE TABLE parent (id INTEGER PRIMARY KEY);
                  CREATE TABLE child (parent_id INTEGER REFERENCES parent(id));
                  INSERT INTO parent VALUES (1);
                  INSERT INTO child VALUES (1);",
        };
        let orphans = Migration {
            name: "0002_orphans.sql",
            sql: "DELETE FROM parent;",
        };
        let result = run(&mut conn, &[parent, orphans]);
        assert!(
            matches!(
                result,
                Err(DbError::Migration(MigrationError::BrokeForeignKeys { .. }))
            ),
            "got {result:?}"
        );
        assert_eq!(versions(&conn), vec![1]);
        let parents: i64 = conn
            .query_row("SELECT COUNT(*) FROM parent", [], |r| r.get(0))
            .unwrap();
        assert_eq!(parents, 1, "the delete was kept");
    }

    #[test]
    fn a_database_whose_applied_migration_was_since_edited_is_refused() {
        let (_dir, mut conn) = open_temp();
        run(&mut conn, &[CREATE_A]).unwrap();
        let edited = Migration {
            name: CREATE_A.name,
            sql: "CREATE TABLE a (x INTEGER, extra TEXT);",
        };
        let result = run(&mut conn, &[edited, CREATE_B]);
        assert!(
            matches!(result, Err(DbError::Migration(MigrationError::Edited { ref name })) if name == CREATE_A.name),
            "got {result:?}"
        );
        assert_eq!(versions(&conn), vec![1], "a later migration ran anyway");
    }

    #[test]
    fn a_database_whose_applied_migration_was_since_renamed_is_refused() {
        let (_dir, mut conn) = open_temp();
        run(&mut conn, &[CREATE_A]).unwrap();
        let renamed = Migration {
            name: "0001_renamed.sql",
            sql: CREATE_A.sql,
        };
        let result = run(&mut conn, &[renamed]);
        assert!(
            matches!(
                result,
                Err(DbError::Migration(MigrationError::Edited { .. }))
            ),
            "got {result:?}"
        );
    }

    #[test]
    fn a_database_from_a_newer_app_is_refused() {
        let (_dir, mut conn) = open_temp();
        run(&mut conn, &[CREATE_A, CREATE_B]).unwrap();
        let result = run(&mut conn, &[CREATE_A]);
        assert!(
            matches!(result, Err(DbError::Migration(MigrationError::Unknown { ref name })) if name == CREATE_B.name),
            "got {result:?}"
        );
    }

    #[test]
    fn the_checksum_ignores_line_endings_but_not_content() {
        let lf = Migration {
            name: "0001_x.sql",
            sql: "CREATE TABLE a (x);\nCREATE TABLE b (y);\n",
        };
        let crlf = Migration {
            sql: "CREATE TABLE a (x);\r\nCREATE TABLE b (y);\r\n",
            ..lf
        };
        let changed = Migration {
            sql: "CREATE TABLE a (z);\nCREATE TABLE b (y);\n",
            ..lf
        };
        assert_eq!(lf.checksum(), crlf.checksum());
        assert_ne!(lf.checksum(), changed.checksum());
        assert_eq!(lf.checksum().len(), 64);
    }

    #[test]
    fn migration_names_must_be_four_digits_then_lowercase_words() {
        assert_eq!(parse_version("0001_init.sql"), Some(1));
        assert_eq!(parse_version("0042_add_file_table.sql"), Some(42));
        assert_eq!(parse_version("0003_v2_fix.sql"), Some(3));
        for bad in [
            "1_init.sql",
            "00001_init.sql",
            "0001.sql",
            "0001_.sql",
            "0001_Init.sql",
            "0001_add-table.sql",
            "0001_a__b.sql",
            "0001_init.SQL",
            "0001_init",
            "abcd_init.sql",
        ] {
            assert_eq!(parse_version(bad), None, "{bad} was accepted");
        }
    }

    // The real migrations.

    fn migrations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
    }

    #[test]
    fn the_first_migration_is_0001_init_and_it_is_empty() {
        let first = MIGRATIONS[0];
        assert_eq!(first.name, "0001_init.sql");
        let (_dir, mut conn) = open_temp();
        run(&mut conn, &[first]).unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(tables, vec!["schema_migration"], "0001 created tables");
    }

    #[test]
    fn real_migrations_are_numbered_from_0001_with_no_gaps_or_repeats() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            let expected = u32::try_from(i + 1).unwrap();
            assert_eq!(
                parse_version(m.name),
                Some(expected),
                "migration #{expected} is named {:?}; expected {expected:04}_<lowercase_words>.sql \
                 (take the next free number at merge time)",
                m.name
            );
        }
    }

    #[test]
    fn every_migration_file_in_the_folder_is_embedded_and_every_embedded_one_has_a_file() {
        let on_disk: BTreeSet<String> = std::fs::read_dir(migrations_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.to_lowercase().ends_with(".sql"))
            .collect();
        let embedded: BTreeSet<String> = MIGRATIONS.iter().map(|m| m.name.to_string()).collect();
        assert_eq!(
            on_disk, embedded,
            "src-tauri/migrations/*.sql and MIGRATIONS in src/db/migrations/mod.rs disagree"
        );
    }

    #[test]
    fn every_real_migration_applies_cleanly_to_a_fresh_database() {
        let (_dir, mut conn) = open_temp();
        assert_eq!(run(&mut conn, MIGRATIONS).unwrap(), MIGRATIONS.len());
        let expected: Vec<u32> = (1..=MIGRATIONS.len() as u32).collect();
        assert_eq!(versions(&conn), expected);
    }
}
