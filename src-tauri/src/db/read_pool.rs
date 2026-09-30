//! The pool of read-only connections.
//!
//! The database is in WAL mode, so readers see the last committed state and
//! never wait for the writer, even while it's in the middle of a transaction.
//! Each reader is opened read-only by SQLite and also has `query_only` set,
//! so a read can't change anything.
//!
//! A read runs on the caller's thread with a connection checked out of the
//! pool, and hands it back when done. Only idle connections sit behind the
//! pool's lock; no connection is ever used while holding it.

use std::cell::RefCell;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::Connection;

use super::writer::on_a_writer_thread;
use super::DbError;
use crate::write_guard::GuardedPath;

/// A handle to the read pool. Cheap to clone and safe to share between
/// threads; every clone uses the same connections.
#[derive(Clone)]
pub struct ReadPool {
    inner: Arc<Inner>,
}

struct Inner {
    path: GuardedPath,
    size: usize,
    state: Mutex<State>,
    /// Signalled when a connection is handed back.
    returned: Condvar,
}

struct State {
    idle: Vec<Connection>,
    /// Connections opened so far, idle or checked out. Never above `size`.
    open: usize,
}

thread_local! {
    /// The pools this thread has a connection checked out from, once per
    /// connection. Lets a nested read fail instead of waiting for itself.
    static HELD: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

impl ReadPool {
    /// How many read connections the app opens at most.
    pub const DEFAULT_SIZE: usize = 4;

    /// Opens a pool of up to [`Self::DEFAULT_SIZE`] read connections to the
    /// database at `path`. The database must already exist; open the
    /// [`Writer`](super::Writer) first, which creates it and turns on WAL.
    pub fn open(path: &GuardedPath) -> Result<ReadPool, DbError> {
        ReadPool::with_size(path, Self::DEFAULT_SIZE)
    }

    /// Like [`Self::open`], with at most `size` connections (at least 1).
    pub fn with_size(path: &GuardedPath, size: usize) -> Result<ReadPool, DbError> {
        let size = size.max(1);
        // Open one now so a bad path fails here, not on the first read.
        let first = open_reader(path)?;
        Ok(ReadPool {
            inner: Arc::new(Inner {
                path: path.clone(),
                size,
                state: Mutex::new(State {
                    idle: vec![first],
                    open: 1,
                }),
                returned: Condvar::new(),
            }),
        })
    }

    /// The database file this pool reads.
    pub fn path(&self) -> &Path {
        self.inner.path.as_path()
    }

    /// Runs `read` with a read-only connection and returns its result.
    ///
    /// Runs on the caller's thread, alongside other reads and the writer.
    /// Waits only if every connection in the pool is in use. Where waiting
    /// could deadlock it returns [`DbError::NestedRead`] instead: a read
    /// that asks for a second connection, or a writer job that reads through
    /// the pool (another read holding a connection may be waiting on that
    /// job). A writer job should read through its own connection.
    pub fn read<T, F>(&self, read: F) -> Result<T, DbError>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T>,
    {
        let lease = self.checkout()?;
        read(lease.conn()).map_err(DbError::Sqlite)
    }

    fn id(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }

    fn checkout(&self) -> Result<Lease<'_>, DbError> {
        let mut state = self.inner.lock();
        loop {
            if let Some(conn) = state.idle.pop() {
                return Ok(self.lease(conn));
            }
            if state.open < self.inner.size {
                // Reserve the slot, then open without holding the lock.
                state.open += 1;
                drop(state);
                return match open_reader(&self.inner.path) {
                    Ok(conn) => Ok(self.lease(conn)),
                    Err(e) => {
                        self.inner.lock().open -= 1;
                        self.inner.returned.notify_one();
                        Err(e)
                    }
                };
            }
            if HELD.with(|h| h.borrow().contains(&self.id())) || on_a_writer_thread() {
                return Err(DbError::NestedRead);
            }
            state = self
                .inner
                .returned
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    fn lease(&self, conn: Connection) -> Lease<'_> {
        HELD.with(|h| h.borrow_mut().push(self.id()));
        Lease {
            pool: self,
            conn: Some(conn),
        }
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Nothing panics while holding the lock, but don't let a poisoned
        // lock take the pool down if it ever does.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A checked-out connection. Handing it back happens on drop, so it also
/// happens when the read panics.
struct Lease<'a> {
    pool: &'a ReadPool,
    conn: Option<Connection>,
}

impl Lease<'_> {
    fn conn(&self) -> &Connection {
        self.conn
            .as_ref()
            .expect("a lease holds its connection until dropped")
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else { return };
        if !conn.is_autocommit() {
            // The read began a transaction and didn't finish it; a stale
            // snapshot must not leak into the next read.
            let _ = conn.execute_batch("ROLLBACK");
        }
        let id = self.pool.id();
        HELD.with(|h| {
            let mut held = h.borrow_mut();
            if let Some(i) = held.iter().position(|&p| p == id) {
                held.swap_remove(i);
            }
        });
        // A read may have switched query_only off; don't hand that on. If it
        // can't be switched back on, close the connection instead.
        let reset = conn.pragma_update(None, "query_only", true);
        let mut state = self.pool.inner.lock();
        if reset.is_ok() {
            state.idle.push(conn);
        } else {
            state.open -= 1;
        }
        drop(state);
        self.pool.inner.returned.notify_one();
    }
}

/// Opens one read connection: read-only at the SQLite level, and
/// `query_only` on top. The write guard opens it, so sorts and temp tables
/// stay in memory and no other file can be attached (ROADMAP §5.6).
fn open_reader(path: &GuardedPath) -> Result<Connection, DbError> {
    let conn = path.open_database_read_only()?;
    conn.pragma_update(None, "query_only", true)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Writer;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::{mpsc, Barrier};
    use std::thread;

    /// A migrated database with a one-row table `t`, its writer and a pool.
    fn open_temp(size: usize) -> (tempfile::TempDir, Writer, ReadPool) {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(dir.path(), "test.db")).unwrap();
        writer
            .call(|c| c.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);"))
            .unwrap();
        let pool = ReadPool::with_size(writer.guarded_path(), size).unwrap();
        (dir, writer, pool)
    }

    fn read_x(pool: &ReadPool) -> i64 {
        pool.read(|c| c.query_row("SELECT x FROM t", [], |r| r.get(0)))
            .unwrap()
    }

    fn is_read_only_error(e: &DbError) -> bool {
        matches!(
            e,
            DbError::Sqlite(rusqlite::Error::SqliteFailure(f, _))
                if f.code == rusqlite::ErrorCode::ReadOnly
        )
    }

    #[test]
    fn a_read_sees_what_the_writer_committed() {
        let (_dir, writer, pool) = open_temp(2);
        assert_eq!(read_x(&pool), 1);
        writer
            .call(|c| c.execute("UPDATE t SET x = 2", []))
            .unwrap();
        assert_eq!(read_x(&pool), 2);
    }

    #[test]
    fn reads_do_not_wait_for_a_writer_holding_an_open_transaction() {
        let (_dir, writer, pool) = open_temp(2);
        let (in_tx, writer_in_tx) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let slow_writer = thread::spawn(move || {
            writer.call(move |c| {
                let tx = c.transaction()?;
                tx.execute("UPDATE t SET x = 99", [])?;
                in_tx.send(()).unwrap();
                // Hold the write transaction open until the test says so.
                released.recv_timeout(Duration::from_secs(30)).ok();
                tx.commit()
            })
        });
        writer_in_tx
            .recv_timeout(Duration::from_secs(10))
            .expect("the writer never started its transaction");

        // Read on a helper thread so a regression (the read waiting for the
        // writer) fails at the timeout instead of hanging.
        let reader = pool.clone();
        let (done, answer) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(read_x(&reader));
        });
        let seen = answer
            .recv_timeout(Duration::from_secs(5))
            .expect("the read waited for the writer's transaction");
        assert_eq!(seen, 1, "the read saw the writer's uncommitted change");

        release.send(()).unwrap();
        slow_writer.join().unwrap().unwrap();
        assert_eq!(read_x(&pool), 99);
    }

    #[test]
    fn the_writer_commits_while_a_read_is_in_progress_and_the_read_keeps_its_snapshot() {
        // Without WAL, a commit has to wait for every reader to finish.
        let (_dir, writer, pool) = open_temp(1);
        let (x_before, x_after) = pool
            .read(|c| {
                let tx = c.unchecked_transaction()?;
                let before: i64 = tx.query_row("SELECT x FROM t", [], |r| r.get(0))?;
                // The writer runs on its own thread; the read is still open.
                writer
                    .call(|w| w.execute("UPDATE t SET x = 3", []))
                    .expect("the writer could not commit during a read");
                let after: i64 = tx.query_row("SELECT x FROM t", [], |r| r.get(0))?;
                Ok((before, after))
            })
            .unwrap();
        assert_eq!((x_before, x_after), (1, 1), "the read's snapshot changed");
        assert_eq!(read_x(&pool), 3);
    }

    #[test]
    fn read_connections_cannot_write() {
        let (_dir, _writer, pool) = open_temp(1);
        for sql in [
            "INSERT INTO t VALUES (2)",
            "UPDATE t SET x = 2",
            "DELETE FROM t",
            "CREATE TABLE other (y INTEGER)",
            "DROP TABLE t",
        ] {
            let result = pool.read(|c| c.execute_batch(sql));
            let err = result.expect_err(sql);
            assert!(
                matches!(err, DbError::Sqlite(_)),
                "{sql}: unexpected error {err:?}"
            );
        }
        assert_eq!(read_x(&pool), 1);
        let rows: i64 = pool
            .read(|c| c.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn read_connections_cannot_write_even_with_query_only_switched_off() {
        // `query_only` is the second lock; the connection is also opened
        // read-only, which a read can't undo.
        let (_dir, _writer, pool) = open_temp(1);
        let result = pool.read(|c| {
            c.pragma_update(None, "query_only", false)?;
            c.execute("INSERT INTO t VALUES (2)", [])
        });
        let err = result.expect_err("the insert went through");
        assert!(is_read_only_error(&err), "unexpected error {err:?}");
        assert_eq!(read_x(&pool), 1);
    }

    #[test]
    fn a_reader_sees_the_database_in_wal_mode() {
        let (_dir, _writer, pool) = open_temp(1);
        let mode: String = pool
            .read(|c| c.query_row("PRAGMA journal_mode", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    fn query_only_and_busy_timeout(pool: &ReadPool) -> (bool, i64) {
        pool.read(|c| {
            let q = c.query_row("PRAGMA query_only", [], |r| r.get(0))?;
            let b = c.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?;
            Ok((q, b))
        })
        .unwrap()
    }

    #[test]
    fn a_fresh_read_connection_is_query_only_with_a_five_second_busy_timeout() {
        // Size 2 and two leases at once: both the connection opened with
        // the pool and one opened later are checked.
        let (_dir, _writer, pool) = open_temp(2);
        let inner = pool.clone();
        let (outer, nested) = pool
            .read(move |c| {
                let q: bool = c.query_row("PRAGMA query_only", [], |r| r.get(0))?;
                let b: i64 = c.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?;
                Ok(((q, b), query_only_and_busy_timeout(&inner)))
            })
            .unwrap();
        assert_eq!(outer, (true, 5000));
        assert_eq!(nested, (true, 5000));
    }

    #[test]
    fn a_read_that_switches_query_only_off_hands_its_connection_back_with_it_on() {
        let (_dir, _writer, pool) = open_temp(1);
        pool.read(|c| c.pragma_update(None, "query_only", false))
            .unwrap();
        assert!(
            query_only_and_busy_timeout(&pool).0,
            "the next read got query_only off"
        );
    }

    #[test]
    fn several_reads_run_at_the_same_time() {
        // Every reader waits at the barrier while holding its connection. If
        // the pool handed out one connection at a time, they'd never all get
        // there.
        const READERS: usize = 4;
        let (_dir, _writer, pool) = open_temp(READERS);
        let barrier = Arc::new(Barrier::new(READERS));
        let (done, finished) = mpsc::channel();
        for _ in 0..READERS {
            let (pool, barrier, done) = (pool.clone(), barrier.clone(), done.clone());
            thread::spawn(move || {
                let x = pool
                    .read(|c| {
                        barrier.wait();
                        c.query_row("SELECT x FROM t", [], |r| r.get::<_, i64>(0))
                    })
                    .unwrap();
                done.send(x).unwrap();
            });
        }
        for _ in 0..READERS {
            let x = finished
                .recv_timeout(Duration::from_secs(10))
                .expect("reads were serialized");
            assert_eq!(x, 1);
        }
    }

    #[test]
    fn the_pool_never_opens_more_than_its_size_and_a_read_waits_for_a_free_connection() {
        let (_dir, _writer, pool) = open_temp(1);
        let (holding, holder_has_it) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let holder_pool = pool.clone();
        let holder = thread::spawn(move || {
            holder_pool
                .read(|_| {
                    holding.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(30)).ok();
                    Ok(())
                })
                .unwrap();
        });
        holder_has_it.recv_timeout(Duration::from_secs(10)).unwrap();

        let (done, answer) = mpsc::channel();
        let waiter_pool = pool.clone();
        thread::spawn(move || done.send(read_x(&waiter_pool)).unwrap());
        thread::sleep(Duration::from_millis(200));
        assert!(
            answer.try_recv().is_err(),
            "a second connection was opened past the pool's size"
        );
        assert_eq!(pool.inner.lock().open, 1);

        release.send(()).unwrap();
        holder.join().unwrap();
        let x = answer
            .recv_timeout(Duration::from_secs(10))
            .expect("the waiting read never got the returned connection");
        assert_eq!(x, 1);
        assert_eq!(pool.inner.lock().open, 1);
    }

    #[test]
    fn a_nested_read_on_an_exhausted_pool_is_refused_instead_of_deadlocking() {
        let (_dir, _writer, pool) = open_temp(1);
        let inner = pool.clone();
        let (done, outcome) = mpsc::channel();
        thread::spawn(move || {
            let nested = pool.read(move |_| Ok(inner.read(|_| Ok(()))));
            let _ = done.send(nested);
        });
        let nested = outcome
            .recv_timeout(Duration::from_secs(10))
            .expect("the nested read deadlocked")
            .unwrap();
        assert!(matches!(nested, Err(DbError::NestedRead)), "got {nested:?}");
    }

    #[test]
    fn a_writer_job_reading_through_an_exhausted_pool_is_refused_instead_of_deadlocking() {
        // A read holds the only connection and waits on the writer; the
        // writer job then asks the pool for a connection. Waiting would
        // hang both forever.
        let (_dir, writer, pool) = open_temp(1);
        let (done, outcome) = mpsc::channel();
        thread::spawn(move || {
            let job_pool = pool.clone();
            let refused = pool.read(move |_| {
                Ok(writer.call(move |_| {
                    Ok(matches!(
                        job_pool.read(|_| Ok(())),
                        Err(DbError::NestedRead)
                    ))
                }))
            });
            let _ = done.send(refused);
        });
        let refused = outcome
            .recv_timeout(Duration::from_secs(10))
            .expect("the writer job's read deadlocked")
            .unwrap()
            .unwrap();
        assert!(refused, "the writer job's read did not return NestedRead");
    }

    #[test]
    fn a_writer_job_can_read_through_the_pool_while_a_connection_is_free() {
        let (_dir, writer, pool) = open_temp(2);
        let x = writer.call(move |_| Ok(read_x(&pool))).unwrap();
        assert_eq!(x, 1);
    }

    #[test]
    fn a_nested_read_is_fine_while_the_pool_has_a_free_connection() {
        let (_dir, _writer, pool) = open_temp(2);
        let inner = pool.clone();
        let x = pool.read(move |_| Ok(read_x(&inner))).unwrap();
        assert_eq!(x, 1);
    }

    #[test]
    fn a_panicking_read_hands_its_connection_back_and_ends_its_transaction() {
        let (_dir, writer, pool) = open_temp(1);
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            pool.read::<(), _>(|c| {
                c.execute_batch("BEGIN")?;
                c.query_row("SELECT x FROM t", [], |r| r.get::<_, i64>(0))?;
                panic!("read blew up mid-transaction");
            })
        }));
        assert!(result.is_err());
        // The only connection is back, and not stuck on the old snapshot.
        writer
            .call(|c| c.execute("UPDATE t SET x = 5", []))
            .unwrap();
        let (x, in_tx) = pool
            .read(|c| {
                Ok((
                    c.query_row("SELECT x FROM t", [], |r| r.get::<_, i64>(0))?,
                    !c.is_autocommit(),
                ))
            })
            .unwrap();
        assert_eq!(x, 5, "the read kept the panicked read's snapshot");
        assert!(!in_tx);
    }

    #[test]
    fn a_reader_refuses_to_attach_another_database() {
        // Even read-only, SQLite writes `-wal`/`-shm` beside an attached
        // WAL database.
        let (dir, _writer, pool) = open_temp(1);
        let target = dir.path().join("elsewhere.db");
        let sql = format!(
            "ATTACH 'file:{}?mode=ro' AS x",
            target.to_str().unwrap().replace('\'', "''")
        );
        let result = pool.read(|c| c.execute_batch(&sql));
        assert!(result.is_err(), "allowed: {result:?}");
        assert!(!target.exists());
    }

    #[test]
    fn every_reader_keeps_sqlite_temp_data_in_memory_not_in_the_temp_folder() {
        // ROADMAP §5.6: a big sort must not spill a temp file to %TEMP%.
        // Two reads held at once use two connections, so both are checked.
        let (_dir, _writer, pool) = open_temp(2);
        let both_held = Barrier::new(2);
        thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| {
                    let temp_store: i64 = pool
                        .read(|c| {
                            let t = c.query_row("PRAGMA temp_store", [], |r| r.get(0))?;
                            both_held.wait();
                            Ok(t)
                        })
                        .unwrap();
                    assert_eq!(temp_store, 2, "temp_store is not MEMORY");
                });
            }
        });
    }

    #[test]
    fn opening_a_pool_on_a_missing_database_fails_and_does_not_create_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.db");
        let guarded = crate::write_guard::test_path(dir.path(), "missing.db");
        assert!(matches!(ReadPool::open(&guarded), Err(DbError::Sqlite(_))));
        assert!(!path.exists());
    }

    #[test]
    fn handles_are_clone_send_and_sync() {
        fn assert_shareable<T: Clone + Send + Sync + 'static>() {}
        assert_shareable::<ReadPool>();
    }
}
