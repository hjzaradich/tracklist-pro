//! The single writer connection.
//!
//! One thread owns the connection. Callers hand it jobs over a channel and
//! wait for the answer, so writes are serialized without a lock around the
//! connection, and a slow job can't be interleaved with another.

use std::cell::Cell;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::Connection;

use super::migrations::{self, Migration, MigrationError};
use crate::write_guard::GuardedPath;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// Why a database call failed.
#[derive(Debug)]
pub enum DbError {
    /// SQLite rejected the call.
    Sqlite(rusqlite::Error),
    /// The writer thread could not be started.
    Spawn(std::io::Error),
    /// The job panicked. The writer rolled back any open transaction and
    /// keeps running.
    JobPanicked,
    /// The writer thread has stopped.
    WriterGone,
    /// A job called the writer again. It would wait for itself forever, so
    /// the nested call is refused; do the work on the connection it was given.
    Reentrant,
    /// The database couldn't be brought up to date. See [`MigrationError`].
    Migration(MigrationError),
    /// SQLite wouldn't put the database in WAL mode; it reported this mode
    /// instead.
    NotWal(String),
    /// Every read connection was in use, and waiting could deadlock, so the
    /// read was refused. Either the read asked for a second connection while
    /// holding one (use the one it was given), or a writer job read through
    /// the pool while a read that is waiting on the writer held a connection
    /// (a writer job reads through its own connection).
    NestedRead,
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Sqlite(e) => write!(f, "database error: {e}"),
            DbError::Spawn(e) => write!(f, "could not start the database writer: {e}"),
            DbError::JobPanicked => write!(f, "a database job panicked and was rolled back"),
            DbError::WriterGone => write!(f, "the database writer has stopped"),
            DbError::Reentrant => write!(f, "a database job called the writer from inside itself"),
            DbError::Migration(e) => write!(f, "{e}"),
            DbError::NotWal(mode) => write!(f, "the database is in {mode} mode, not WAL"),
            DbError::NestedRead => write!(
                f,
                "every read connection was in use and waiting could deadlock"
            ),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbError::Sqlite(e) => Some(e),
            DbError::Spawn(e) => Some(e),
            DbError::Migration(e) => Some(e),
            DbError::JobPanicked
            | DbError::WriterGone
            | DbError::Reentrant
            | DbError::NotWal(_)
            | DbError::NestedRead => None,
        }
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sqlite(e)
    }
}

/// A handle to the one writer connection. Cheap to clone and safe to share
/// between threads; every clone talks to the same connection.
///
/// The connection closes when the last handle is dropped.
#[derive(Clone)]
pub struct Writer {
    inner: Arc<Inner>,
}

struct Inner {
    path: GuardedPath,
    jobs: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    /// Opens (or creates) the database at `path`, puts it in WAL mode,
    /// applies any pending migrations and starts the writer thread. The
    /// folder must already exist. Only the write guard hands out a
    /// [`GuardedPath`], so the database is only ever written inside the
    /// app's own folders.
    ///
    /// If a migration fails, no writer is started and the database stays at
    /// the last version that applied cleanly.
    pub fn open(path: &GuardedPath) -> Result<Writer, DbError> {
        Writer::open_with(path, migrations::MIGRATIONS)
    }

    /// [`Writer::open`] with a given list of migrations, for tests.
    pub(crate) fn open_with(
        path: &GuardedPath,
        migrations: &[Migration],
    ) -> Result<Writer, DbError> {
        // Set up on the caller's thread so a bad path or a failed migration
        // fails here, not later.
        let mut conn = path.open_database()?;
        configure(&conn)?;
        migrations::run(&mut conn, migrations)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let (jobs, queue) = mpsc::channel::<Job>();
        let thread = thread::Builder::new()
            .name("db-writer".into())
            .spawn(move || run(conn, queue))
            .map_err(DbError::Spawn)?;
        Ok(Writer {
            inner: Arc::new(Inner {
                path: path.clone(),
                jobs: Some(jobs),
                thread: Some(thread),
            }),
        })
    }

    /// The database file this writer owns.
    pub fn path(&self) -> &Path {
        self.inner.path.as_path()
    }

    /// The database file as the write guard checked it, for opening the
    /// [`ReadPool`](super::ReadPool) on it.
    pub fn guarded_path(&self) -> &GuardedPath {
        &self.inner.path
    }

    /// Runs `job` on the writer connection and waits for its result.
    ///
    /// Jobs run one at a time, in the order they arrive. A job that panics
    /// returns [`DbError::JobPanicked`], and any transaction it left open is
    /// rolled back. Calling from inside a job returns [`DbError::Reentrant`]
    /// at once instead of deadlocking.
    pub fn call<T, F>(&self, job: F) -> Result<T, DbError>
    where
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        if self.inner.on_writer_thread() {
            return Err(DbError::Reentrant);
        }
        let (reply, answer) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |conn| {
            let result = panic::catch_unwind(AssertUnwindSafe(|| job(conn)));
            if result.is_err() && !conn.is_autocommit() {
                // The job opened a transaction and never finished it.
                let _ = conn.execute_batch("ROLLBACK");
            }
            // The caller may have stopped waiting; nothing to do then.
            let _ = reply.send(result);
        });
        let jobs = self.inner.jobs.as_ref().ok_or(DbError::WriterGone)?;
        jobs.send(job).map_err(|_| DbError::WriterGone)?;
        match answer.recv() {
            Ok(Ok(result)) => result.map_err(DbError::Sqlite),
            Ok(Err(_panic)) => Err(DbError::JobPanicked),
            Err(_) => Err(DbError::WriterGone),
        }
    }
}

/// Connection settings for the writer. WAL lets readers work alongside it;
/// the mode is stored in the file, so every later connection gets it too.
fn configure(conn: &Connection) -> Result<(), DbError> {
    // First, so switching to WAL waits for another app instance's lock
    // instead of failing at once. rusqlite sets 5 s by default today but
    // may change that; don't depend on it.
    conn.busy_timeout(Duration::from_secs(5))?;
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(DbError::NotWal(mode));
    }
    // With WAL, NORMAL can't corrupt the database; a power cut can lose only
    // the last few commits.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

thread_local! {
    /// True on a writer thread.
    static ON_A_WRITER_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// True when running inside a writer job, on any writer's thread. The read
/// pool and the job queue use it to refuse a wait that could deadlock.
pub(crate) fn on_a_writer_thread() -> bool {
    ON_A_WRITER_THREAD.with(Cell::get)
}

/// The writer thread: runs jobs until every handle is dropped, then closes
/// the connection.
fn run(mut conn: Connection, queue: mpsc::Receiver<Job>) {
    ON_A_WRITER_THREAD.with(|w| w.set(true));
    for job in queue {
        job(&mut conn);
    }
}

impl Inner {
    /// True when running inside a job, on the writer thread itself.
    fn on_writer_thread(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|t| t.thread().id() == thread::current().id())
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Closing the channel ends the thread's loop, which closes the
        // connection. Waiting for that means the file is released when the
        // last handle goes away.
        drop(self.jobs.take());
        // A job that held the last handle is running on the writer thread
        // itself; it can't wait for itself to finish.
        if !self.on_writer_thread() {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_guard::test_path;
    use rusqlite::OptionalExtension;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    fn open_temp() -> (tempfile::TempDir, Writer) {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&test_path(dir.path(), "test.db")).unwrap();
        (dir, writer)
    }

    #[test]
    fn open_creates_the_database_file() {
        let (dir, writer) = open_temp();
        assert!(dir.path().join("test.db").is_file());
        assert_eq!(
            writer.path(),
            std::fs::canonicalize(dir.path().join("test.db")).unwrap()
        );
    }

    #[test]
    fn open_fails_cleanly_when_the_folder_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let result = Writer::open(&test_path(dir.path(), "missing/test.db"));
        assert!(matches!(result, Err(DbError::Sqlite(_))));
    }

    #[test]
    fn every_call_uses_the_same_connection() {
        // TEMP tables are private to one connection: if a later call saw a
        // different connection, the table would not be there.
        let (_dir, writer) = open_temp();
        writer
            .call(|c| c.execute_batch("CREATE TEMP TABLE t (x INTEGER)"))
            .unwrap();
        writer
            .call(|c| c.execute("INSERT INTO t VALUES (42)", []))
            .unwrap();
        let x: i64 = writer
            .call(|c| c.query_row("SELECT x FROM t", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(x, 42);
    }

    #[test]
    fn handles_are_clone_send_and_sync() {
        fn assert_shareable<T: Clone + Send + Sync + 'static>() {}
        assert_shareable::<Writer>();
    }

    #[test]
    fn calls_from_many_threads_are_serialized_through_the_one_writer() {
        let (_dir, writer) = open_temp();
        writer
            .call(|c| {
                c.execute_batch("CREATE TABLE counter (n INTEGER); INSERT INTO counter VALUES (0);")
            })
            .unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let writer = writer.clone();
                thread::spawn(move || {
                    for _ in 0..50 {
                        // Read, then write in a separate statement: this only
                        // adds up if no other job runs in between.
                        writer
                            .call(|c| {
                                let n: i64 =
                                    c.query_row("SELECT n FROM counter", [], |r| r.get(0))?;
                                c.execute("UPDATE counter SET n = ?1", [n + 1])
                            })
                            .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let n: i64 = writer
            .call(|c| c.query_row("SELECT n FROM counter", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(n, 400);
    }

    #[test]
    fn sql_errors_are_returned_to_the_caller() {
        let (_dir, writer) = open_temp();
        let result = writer.call(|c| c.execute_batch("NOT VALID SQL"));
        assert!(matches!(result, Err(DbError::Sqlite(_))));
        // And the writer still works afterwards.
        writer.call(|c| c.execute_batch("SELECT 1")).unwrap();
    }

    #[test]
    fn a_panicking_job_reports_an_error_and_the_writer_keeps_running() {
        let (_dir, writer) = open_temp();
        let result: Result<(), _> = writer.call(|_| panic!("job blew up"));
        assert!(matches!(result, Err(DbError::JobPanicked)));
        let one: i64 = writer
            .call(|c| c.query_row("SELECT 1", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(one, 1);
    }

    #[test]
    fn a_transaction_left_open_by_a_panicking_job_is_rolled_back() {
        let (_dir, writer) = open_temp();
        writer
            .call(|c| c.execute_batch("CREATE TABLE t (x INTEGER)"))
            .unwrap();
        let result: Result<(), _> = writer.call(|c| {
            c.execute_batch("BEGIN; INSERT INTO t VALUES (1);")?;
            panic!("job blew up mid-transaction");
        });
        assert!(matches!(result, Err(DbError::JobPanicked)));
        let (in_tx, row): (bool, Option<i64>) = writer
            .call(|c| {
                let row = c
                    .query_row("SELECT x FROM t", [], |r| r.get(0))
                    .optional()?;
                Ok((!c.is_autocommit(), row))
            })
            .unwrap();
        assert!(
            !in_tx,
            "the connection is still inside the panicked job's transaction"
        );
        assert_eq!(row, None, "the panicked job's insert was kept");
    }

    #[test]
    fn a_nested_call_from_inside_a_job_is_refused_instead_of_deadlocking() {
        let (_dir, writer) = open_temp();
        let inner = writer.clone();
        let (done, outcome) = mpsc::channel();
        // Run on a helper thread so a regression (a deadlock) fails the test
        // at the timeout below instead of hanging the whole test run.
        thread::spawn(move || {
            let nested =
                writer.call(move |_| Ok(matches!(inner.call(|_| Ok(())), Err(DbError::Reentrant))));
            let _ = done.send(nested);
        });
        let nested_was_refused = outcome
            .recv_timeout(Duration::from_secs(10))
            .expect("the nested call deadlocked")
            .unwrap();
        assert!(
            nested_was_refused,
            "the nested call did not return DbError::Reentrant"
        );
    }

    #[test]
    fn dropping_the_last_handle_waits_until_the_writer_has_shut_down() {
        // A guard left on the writer thread is destroyed only when that thread
        // exits, which is after it has closed the connection. Its destructor
        // is slow, so if dropping the last handle didn't wait for the thread,
        // `drop` would return before the flag is set.
        struct SlowGuard(Arc<AtomicBool>);
        impl Drop for SlowGuard {
            fn drop(&mut self) {
                thread::sleep(Duration::from_millis(300));
                self.0.store(true, Ordering::SeqCst);
            }
        }
        thread_local! {
            static GUARD: RefCell<Option<SlowGuard>> = const { RefCell::new(None) };
        }

        let (dir, writer) = open_temp();
        let shut_down = Arc::new(AtomicBool::new(false));
        let flag = shut_down.clone();
        writer
            .call(move |_| {
                GUARD.with(|g| *g.borrow_mut() = Some(SlowGuard(flag)));
                Ok(())
            })
            .unwrap();
        let clone = writer.clone();
        drop(writer);
        // One handle is still alive, so the writer still answers.
        clone.call(|c| c.execute_batch("SELECT 1")).unwrap();
        assert!(!shut_down.load(Ordering::SeqCst));

        drop(clone);
        assert!(
            shut_down.load(Ordering::SeqCst),
            "drop returned before the writer shut down"
        );
        // And the file is released: Windows refuses to delete an open file.
        std::fs::remove_file(dir.path().join("test.db")).unwrap();
    }

    #[test]
    fn opening_puts_the_database_in_wal_mode_with_foreign_keys_on() {
        let (_dir, writer) = open_temp();
        let (mode, fks): (String, bool) = writer
            .call(|c| {
                let mode = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
                let fks = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
                Ok((mode, fks))
            })
            .unwrap();
        assert_eq!(mode, "wal");
        assert!(fks, "foreign keys are not enforced");
    }

    #[test]
    fn the_writer_refuses_attach_and_vacuum_into_even_after_migrations_ran() {
        // Migrations install their own authorizer for a while; the write
        // guard's rule (no ATTACH of a file) must still hold afterwards.
        let (dir, writer) = open_temp();
        let target = dir.path().join("elsewhere.db");
        let t = target.to_str().unwrap().replace('\'', "''");
        for sql in [
            format!("ATTACH '{t}' AS x"),
            format!("VACUUM INTO '{t}'"),
            format!("VACUUM main INTO '{t}'"),
        ] {
            let result = writer.call(move |c| c.execute_batch(&sql));
            assert!(result.is_err(), "allowed: {result:?}");
            assert!(!target.exists());
        }
        writer.call(|c| c.execute_batch("VACUUM")).unwrap();
    }

    #[test]
    fn the_writer_keeps_sqlite_temp_data_in_memory_not_in_the_temp_folder() {
        // ROADMAP §5.6: nothing is written outside the app's own folders,
        // and SQLite would otherwise spill sorts and temp tables to %TEMP%.
        let (_dir, writer) = open_temp();
        let temp_store: i64 = writer
            .call(|c| c.query_row("PRAGMA temp_store", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(temp_store, 2, "temp_store is not MEMORY");
    }

    #[test]
    fn opening_sets_synchronous_normal_and_a_five_second_busy_timeout() {
        let (_dir, writer) = open_temp();
        let (synchronous, busy_timeout): (i64, i64) = writer
            .call(|c| {
                let s = c.query_row("PRAGMA synchronous", [], |r| r.get(0))?;
                let b = c.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?;
                Ok((s, b))
            })
            .unwrap();
        assert_eq!(synchronous, 1, "synchronous is not NORMAL");
        assert_eq!(busy_timeout, 5000);
    }

    #[test]
    fn opening_waits_for_another_connection_holding_a_lock_instead_of_failing() {
        // Another app instance is mid-write when this one starts. Opening
        // must wait (busy timeout) rather than fail on the WAL switch.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let other = Connection::open(&path).unwrap();
        other
            .execute_batch("CREATE TABLE t (x INTEGER); BEGIN EXCLUSIVE; INSERT INTO t VALUES (1);")
            .unwrap();
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            other.execute_batch("COMMIT").unwrap();
        });
        let writer = Writer::open(&test_path(dir.path(), "test.db"));
        releaser.join().unwrap();
        assert!(writer.is_ok(), "open failed: {:?}", writer.err());
    }

    #[test]
    fn opening_applies_every_migration_and_reopening_applies_none_again() {
        let (dir, writer) = open_temp();
        let expected: Vec<String> = migrations::MIGRATIONS
            .iter()
            .map(|m| m.name.to_string())
            .collect();
        let names = |w: &Writer| -> Vec<String> {
            w.call(|c| {
                Ok(migrations::applied(c)?
                    .into_iter()
                    .map(|a| a.name)
                    .collect())
            })
            .unwrap()
        };
        assert_eq!(names(&writer), expected);
        drop(writer);
        let writer = Writer::open(&test_path(dir.path(), "test.db")).unwrap();
        assert_eq!(names(&writer), expected);
    }

    #[test]
    fn a_failed_migration_at_startup_refuses_to_open_and_keeps_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let first = Migration {
            name: "0001_create_a.sql",
            sql: "CREATE TABLE a (x INTEGER);",
        };
        let broken = Migration {
            name: "0002_broken.sql",
            sql: "CREATE TABLE b (y INTEGER); INSERT INTO nowhere VALUES (1);",
        };
        let guarded = test_path(dir.path(), "test.db");
        drop(Writer::open_with(&guarded, &[first]).unwrap());

        let result = Writer::open_with(&guarded, &[first, broken]);
        assert!(
            matches!(result, Err(DbError::Migration(MigrationError::Failed { ref name, .. })) if name == broken.name),
            "got {:?}",
            result.err()
        );
        let conn = Connection::open(&path).unwrap();
        let versions: Vec<u32> = migrations::applied(&conn)
            .unwrap()
            .iter()
            .map(|a| a.version)
            .collect();
        assert_eq!(versions, vec![1]);
        let b_exists: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'b')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!b_exists, "the failed migration's table was kept");
    }

    #[test]
    fn data_written_by_one_writer_is_there_when_the_database_is_reopened() {
        let (dir, writer) = open_temp();
        writer
            .call(|c| c.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (7);"))
            .unwrap();
        drop(writer);
        let writer = Writer::open(&test_path(dir.path(), "test.db")).unwrap();
        let x: i64 = writer
            .call(|c| c.query_row("SELECT x FROM t", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(x, 7);
    }
}
