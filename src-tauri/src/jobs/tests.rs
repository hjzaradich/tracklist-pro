//! Behavior tests for the job queue: priorities, failures, progress,
//! cancellation and the writer staying free.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use super::*;
use crate::db::{DbError, Writer};

/// Long enough that a passing test never hits it.
const PATIENCE: Duration = Duration::from_secs(10);

/// A fresh database in a temp dir. Every test opens its database here.
fn temp_writer() -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    (dir, writer)
}

/// Every batch of updates the queue sent, in order.
#[derive(Clone, Default)]
struct Updates(Arc<Mutex<Vec<Vec<JobUpdate>>>>);

impl Updates {
    fn sink(&self) -> impl Fn(&[JobUpdate]) + Send + 'static {
        let list = self.0.clone();
        move |batch| list.lock().unwrap().push(batch.to_vec())
    }

    /// Every update, batches joined.
    fn all(&self) -> Vec<JobUpdate> {
        self.0.lock().unwrap().iter().flatten().cloned().collect()
    }

    fn for_job(&self, id: JobId) -> Vec<(JobStatus, Option<f64>)> {
        self.all()
            .into_iter()
            .filter(|u| u.id == id)
            .map(|u| (u.status, u.progress))
            .collect()
    }
}

fn stored(writer: &Writer, id: JobId) -> JobRecord {
    writer.call(move |c| store::get(c, id)).unwrap().unwrap()
}

/// Waits until `id` has finished and returns it.
fn finished(writer: &Writer, id: JobId) -> JobRecord {
    let start = Instant::now();
    loop {
        let job = stored(writer, id);
        if job.status.is_finished() {
            return job;
        }
        assert!(
            start.elapsed() < PATIENCE,
            "job {id} never finished: {job:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < PATIENCE, "timed out waiting until {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// A handler that says when it starts, then waits to be released.
/// `started` gets the job id; send on `release` to let one job finish.
fn gate() -> (impl JobHandler, mpsc::Receiver<JobId>, mpsc::Sender<()>) {
    let (started_tx, started) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let started_tx = Mutex::new(started_tx);
    let release_rx = Mutex::new(release_rx);
    let handler = move |job: &JobContext| {
        started_tx.lock().unwrap().send(job.id()).unwrap();
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(PATIENCE)
            .map_err(|_| JobError::failed("never released"))
    };
    (handler, started, release)
}

// 0E-1: model, persistence and enqueue.

#[test]
fn enqueue_stores_the_job_and_returns_its_id() {
    let (_dir, writer) = temp_writer();
    // Stop the workers first, so the job is still queued when we look.
    let queue = JobQueue::builder(writer.clone()).start().unwrap();
    queue.shutdown();
    let id = queue
        .enqueue(
            NewJob::new(JobKind::Hash)
                .target(json!({"file_id": 7}))
                .priority(Priority::USER),
        )
        .unwrap();
    let job = stored(&writer, id);
    assert_eq!(job.id, id);
    assert_eq!(job.kind, Some(JobKind::Hash));
    assert_eq!(job.target, Some(json!({"file_id": 7})));
    assert_eq!(job.priority, Priority::USER);
    assert_eq!(job.status, JobStatus::Queued);
}

#[test]
fn each_enqueue_gets_a_new_id() {
    let (_dir, writer) = temp_writer();
    let queue = JobQueue::builder(writer).start().unwrap();
    queue.shutdown();
    let ids: Vec<_> = JobKind::ALL
        .iter()
        .map(|k| queue.enqueue(NewJob::new(*k)).unwrap())
        .collect();
    assert_eq!(ids.len(), JobKind::ALL.len());
    assert!(ids.windows(2).all(|w| w[1] > w[0]), "{ids:?}");
}

#[test]
fn jobs_queued_before_the_app_started_are_shown_and_run() {
    let (_dir, writer) = temp_writer();
    let earlier = writer
        .call(|c| store::insert(c, &NewJob::new(JobKind::Scan)))
        .unwrap();
    let (handler, started, release) = gate();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, handler)
        .start()
        .unwrap();
    assert_eq!(started.recv_timeout(PATIENCE).unwrap(), earlier);
    let snapshot = queue.activity().unwrap();
    assert_eq!(snapshot.jobs.len(), 1);
    assert_eq!(snapshot.jobs[0].id, earlier);
    assert_eq!(snapshot.jobs[0].status, JobStatus::Running);
    release.send(()).unwrap();
    assert_eq!(finished(&writer, earlier).status, JobStatus::Done);
}

// 0E-2: the worker pool.

#[test]
fn workers_take_the_highest_priority_job_first_then_the_oldest() {
    let (_dir, writer) = temp_writer();
    let (handler, started, release) = gate();
    let ran = Arc::new(Mutex::new(Vec::new()));
    let log = ran.clone();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, handler)
        .handler(JobKind::Hash, move |job: &JobContext| {
            log.lock().unwrap().push(job.id());
            Ok(())
        })
        .start()
        .unwrap();
    // Keep the one worker busy while the rest are queued.
    let busy = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    assert_eq!(started.recv_timeout(PATIENCE).unwrap(), busy);

    let hash = |p| NewJob::new(JobKind::Hash).priority(p);
    let low = queue.enqueue(hash(Priority::BACKGROUND)).unwrap();
    let normal_a = queue.enqueue(hash(Priority::NORMAL)).unwrap();
    let high = queue.enqueue(hash(Priority::USER)).unwrap();
    let normal_b = queue.enqueue(hash(Priority::NORMAL)).unwrap();
    release.send(()).unwrap();

    finished(&writer, low);
    assert_eq!(*ran.lock().unwrap(), [high, normal_a, normal_b, low]);
}

#[test]
fn n_workers_run_n_jobs_at_once() {
    let (_dir, writer) = temp_writer();
    let (handler, started, release) = gate();
    let queue = JobQueue::builder(writer.clone())
        .workers(3)
        .handler(JobKind::Fingerprint, handler)
        .start()
        .unwrap();
    let ids: Vec<_> = (0..4)
        .map(|_| queue.enqueue(NewJob::new(JobKind::Fingerprint)).unwrap())
        .collect();
    let mut running: Vec<_> = (0..3)
        .map(|_| started.recv_timeout(PATIENCE).unwrap())
        .collect();
    running.sort();
    assert_eq!(running, ids[..3]);
    // The fourth waits for a free worker.
    assert!(started.recv_timeout(Duration::from_millis(100)).is_err());
    assert_eq!(stored(&writer, ids[3]).status, JobStatus::Queued);
    for _ in 0..4 {
        release.send(()).unwrap();
    }
    for id in ids {
        assert_eq!(finished(&writer, id).status, JobStatus::Done);
    }
}

#[test]
fn a_failed_job_is_marked_failed_with_its_error_and_the_next_job_still_runs() {
    let (_dir, writer) = temp_writer();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Convert, |_: &JobContext| {
            Err(JobError::failed("unsupported format"))
        })
        .handler(JobKind::Export, |_: &JobContext| Ok(()))
        .start()
        .unwrap();
    let bad = queue.enqueue(NewJob::new(JobKind::Convert)).unwrap();
    let good = queue.enqueue(NewJob::new(JobKind::Export)).unwrap();

    let bad = finished(&writer, bad);
    assert_eq!(bad.status, JobStatus::Failed);
    assert_eq!(bad.error.as_deref(), Some("unsupported format"));
    assert!(bad.finished_at.is_some());
    assert_eq!(finished(&writer, good).status, JobStatus::Done);
}

#[test]
fn a_panicking_job_is_marked_failed_and_the_pool_keeps_working() {
    let (_dir, writer) = temp_writer();
    // One worker: if the panic killed it, nothing after would run.
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Embed, |_: &JobContext| -> Result<(), JobError> {
            panic!("model file is corrupt")
        })
        .handler(JobKind::Export, |_: &JobContext| Ok(()))
        .start()
        .unwrap();
    let panics: Vec<_> = (0..3)
        .map(|_| queue.enqueue(NewJob::new(JobKind::Embed)).unwrap())
        .collect();
    let good = queue.enqueue(NewJob::new(JobKind::Export)).unwrap();

    for id in panics {
        let job = finished(&writer, id);
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(
            job.error.as_deref(),
            Some("the job panicked: model file is corrupt")
        );
    }
    assert_eq!(finished(&writer, good).status, JobStatus::Done);
    assert!(
        queue.activity().unwrap().jobs.is_empty(),
        "nothing is left running"
    );
}

#[test]
fn a_job_with_no_handler_fails_and_says_so() {
    let (_dir, writer) = temp_writer();
    let queue = JobQueue::builder(writer.clone()).start().unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Analyze)).unwrap();
    let job = finished(&writer, id);
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.error.as_deref(), Some("no handler for analyze jobs"));
}

#[test]
fn a_stored_job_of_a_kind_this_build_does_not_know_fails_without_stopping_the_pool() {
    let (_dir, writer) = temp_writer();
    let unknown = writer
        .call(|c| {
            c.execute("INSERT INTO job (kind) VALUES ('teleport')", [])?;
            Ok(JobId(c.last_insert_rowid()))
        })
        .unwrap();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, |_: &JobContext| Ok(()))
        .start()
        .unwrap();
    assert!(
        queue.activity().unwrap().jobs.is_empty(),
        "an unknown kind isn't shown"
    );
    let next = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    let job = finished(&writer, unknown);
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.error.as_deref(), Some("unknown job kind \"teleport\""));
    assert_eq!(finished(&writer, next).status, JobStatus::Done);
}

#[test]
fn a_job_runs_off_the_writer_thread_and_writes_through_the_writer() {
    let (_dir, writer) = temp_writer();
    let thread_name = Arc::new(Mutex::new(None));
    let seen = thread_name.clone();
    let queue = JobQueue::builder(writer.clone())
        .handler(JobKind::Scan, move |job: &JobContext| {
            *seen.lock().unwrap() = thread::current().name().map(str::to_owned);
            let folder = job.target().unwrap()["folder"].as_str().unwrap().to_owned();
            job.writer().call(move |c| {
                c.execute(
                    "INSERT INTO setting (key, value) VALUES ('last_scan', json_quote(?1))",
                    [folder],
                )
            })?;
            Ok(())
        })
        .start()
        .unwrap();
    let id = queue
        .enqueue(NewJob::new(JobKind::Scan).target(json!({"folder": "D:\\Music"})))
        .unwrap();
    assert_eq!(finished(&writer, id).status, JobStatus::Done);

    let name = thread_name.lock().unwrap().clone().unwrap();
    assert!(name.starts_with("job-worker-"), "ran on {name:?}");
    let value: String = writer
        .call(|c| {
            c.query_row(
                "SELECT value FROM setting WHERE key = 'last_scan'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(value, r#""D:\\Music""#);
}

#[test]
fn a_long_running_job_does_not_block_the_db_writer() {
    let (_dir, writer) = temp_writer();
    let (handler, started, release) = gate();
    let queue = JobQueue::builder(writer.clone())
        .workers(2)
        .handler(JobKind::Fingerprint, handler)
        .start()
        .unwrap();
    // Both workers are now in the middle of a job that won't finish.
    let ids = [
        queue.enqueue(NewJob::new(JobKind::Fingerprint)).unwrap(),
        queue.enqueue(NewJob::new(JobKind::Fingerprint)).unwrap(),
    ];
    started.recv_timeout(PATIENCE).unwrap();
    started.recv_timeout(PATIENCE).unwrap();

    // The writer still answers at once, many times over.
    let start = Instant::now();
    for _ in 0..50 {
        let one: i64 = writer
            .call(|c| c.query_row("SELECT 1", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(one, 1);
    }
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "writer calls took {:?} while jobs ran",
        start.elapsed()
    );
    // And the queue itself still takes new jobs.
    let more = queue.enqueue(NewJob::new(JobKind::Fingerprint)).unwrap();
    assert_eq!(stored(&writer, more).status, JobStatus::Queued);
    for _ in 0..3 {
        release.send(()).unwrap();
    }
    for id in ids.into_iter().chain([more]) {
        finished(&writer, id);
    }
}

#[test]
fn a_job_can_enqueue_a_follow_up_job() {
    let (_dir, writer) = temp_writer();
    let follow_up = Arc::new(Mutex::new(None));
    let slot = follow_up.clone();
    let queue = JobQueue::builder(writer.clone())
        .handler(JobKind::Scan, move |job: &JobContext| {
            let id = job.enqueue(NewJob::new(JobKind::Hash).target(json!({"file_id": 1})))?;
            *slot.lock().unwrap() = Some(id);
            Ok(())
        })
        .handler(JobKind::Hash, |_: &JobContext| Ok(()))
        .start()
        .unwrap();
    let scan = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    finished(&writer, scan);
    let hash = follow_up.lock().unwrap().unwrap();
    assert_eq!(finished(&writer, hash).status, JobStatus::Done);
}

// 0E-3: progress events.

#[test]
fn every_change_is_sent_in_order() {
    let (_dir, writer) = temp_writer();
    let updates = Updates::default();
    // The job waits before each step until the test has seen the last one
    // arrive, so every step goes out in a batch of its own.
    let (step, steps) = mpsc::channel::<()>();
    let steps = Mutex::new(steps);
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(updates.sink())
        .handler(JobKind::Analyze, move |job: &JobContext| {
            let next = || steps.lock().unwrap().recv_timeout(PATIENCE).unwrap();
            for n in 1..=4 {
                next();
                job.progress(n as f64 / 4.0)?;
            }
            next();
            Ok(())
        })
        .start()
        .unwrap();
    let id = queue
        .enqueue(NewJob::new(JobKind::Analyze).priority(Priority::USER))
        .unwrap();
    let last = |updates: &Updates| updates.for_job(id).last().copied();
    wait_until("it's running", || {
        last(&updates) == Some((JobStatus::Running, None))
    });
    for n in 1..=4 {
        step.send(()).unwrap();
        let want = Some((JobStatus::Running, Some(n as f64 / 4.0)));
        wait_until("the step is sent", || last(&updates) == want);
    }
    step.send(()).unwrap();
    wait_until("the last update is sent", || {
        last(&updates).map(|u| u.0) == Some(JobStatus::Done)
    });

    // Queued and running can share a batch, which keeps only the newer.
    let mut seen = updates.for_job(id);
    if seen[0] == (JobStatus::Queued, None) {
        seen.remove(0);
    }
    assert_eq!(
        seen,
        [
            (JobStatus::Running, None),
            (JobStatus::Running, Some(0.25)),
            (JobStatus::Running, Some(0.5)),
            (JobStatus::Running, Some(0.75)),
            (JobStatus::Running, Some(1.0)),
            (JobStatus::Done, Some(1.0)),
        ]
    );
    let all = updates.all();
    assert!(all
        .iter()
        .all(|u| u.kind == JobKind::Analyze && u.priority == 10));
    let seqs: Vec<_> = all.iter().map(|u| u.seq).collect();
    assert!(seqs.windows(2).all(|w| w[1] > w[0]), "{seqs:?}");
    assert_eq!(seqs.last(), Some(&7), "one seq per change");
    // Progress is stored too.
    assert_eq!(stored(&writer, id).progress, Some(1.0));
}

#[test]
fn tiny_progress_steps_are_not_sent() {
    let (_dir, writer) = temp_writer();
    let updates = Updates::default();
    let queue = JobQueue::builder(writer.clone())
        .on_updates(updates.sink())
        .handler(JobKind::Hash, |job: &JobContext| {
            for i in 0..=10_000 {
                job.progress(i as f64 / 10_000.0)?;
            }
            Ok(())
        })
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Hash)).unwrap();
    finished(&writer, id);
    wait_until("the last update is sent", || {
        updates.for_job(id).last().map(|u| u.0) == Some(JobStatus::Done)
    });
    let progress: Vec<f64> = updates
        .for_job(id)
        .into_iter()
        .filter_map(|(_, p)| p)
        .collect();
    // At most one per step (0, 0.01, …, 1), not 10 001; batching can
    // merge more.
    assert!(progress.len() <= 103, "{} progress updates", progress.len());
    assert_eq!(progress.last(), Some(&1.0));
    // Each stored and sent change takes a seq: about 101, not 10 001.
    let last_seq = updates.all().last().unwrap().seq;
    assert!(last_seq <= 105, "{last_seq} changes sent");
    // Never goes back. (Running at 1 and done at 1 can arrive separately.)
    assert!(progress.windows(2).all(|w| w[1] >= w[0]), "{progress:?}");
}

#[test]
fn the_snapshot_lists_running_then_queued_jobs_with_the_latest_seq() {
    let (_dir, writer) = temp_writer();
    let updates = Updates::default();
    let (handler, started, release) = gate();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(updates.sink())
        .handler(JobKind::Scan, handler)
        .start()
        .unwrap();
    let running = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    started.recv_timeout(PATIENCE).unwrap();
    let low = queue
        .enqueue(NewJob::new(JobKind::Scan).priority(Priority::BACKGROUND))
        .unwrap();
    let high = queue
        .enqueue(NewJob::new(JobKind::Scan).priority(Priority::USER))
        .unwrap();

    let snapshot = queue.activity().unwrap();
    let order: Vec<_> = snapshot.jobs.iter().map(|j| (j.id, j.status)).collect();
    assert_eq!(
        order,
        [
            (running, JobStatus::Running),
            (high, JobStatus::Queued),
            (low, JobStatus::Queued),
        ]
    );
    wait_until("the snapshot's last update is sent", || {
        updates.all().last().map(|u| u.seq) == Some(snapshot.seq)
    });
    for _ in 0..3 {
        release.send(()).unwrap();
    }
    finished(&writer, low);
    wait_until("the snapshot is empty", || {
        queue.activity().unwrap().jobs.is_empty()
    });
}

// 0E-4: cancellation.

#[test]
fn a_queued_job_cancelled_before_it_starts_never_runs() {
    let (_dir, writer) = temp_writer();
    let updates = Updates::default();
    let (handler, started, release) = gate();
    let ran = Arc::new(Mutex::new(false));
    let flag = ran.clone();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(updates.sink())
        .handler(JobKind::Scan, handler)
        .handler(JobKind::Export, move |_: &JobContext| {
            *flag.lock().unwrap() = true;
            Ok(())
        })
        .start()
        .unwrap();
    let busy = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    started.recv_timeout(PATIENCE).unwrap();
    let doomed = queue.enqueue(NewJob::new(JobKind::Export)).unwrap();

    assert_eq!(queue.cancel(doomed).unwrap(), CancelOutcome::Cancelled);
    assert_eq!(stored(&writer, doomed).status, JobStatus::Cancelled);
    wait_until("the cancel is sent", || {
        updates.for_job(doomed).last() == Some(&(JobStatus::Cancelled, None))
    });
    // Queued may have gone out in the same batch, which keeps only the
    // newer.
    assert!(updates
        .for_job(doomed)
        .iter()
        .all(|u| u.0 == JobStatus::Queued || u.0 == JobStatus::Cancelled));
    assert!(queue
        .activity()
        .unwrap()
        .jobs
        .iter()
        .all(|j| j.id != doomed));

    release.send(()).unwrap();
    finished(&writer, busy);
    // Give the worker a chance to (wrongly) pick it up.
    let after = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    release.send(()).unwrap();
    finished(&writer, after);
    assert!(!*ran.lock().unwrap(), "a cancelled job ran");
    let job = stored(&writer, doomed);
    assert_eq!((job.status, job.attempts), (JobStatus::Cancelled, 0));
}

#[test]
fn a_running_job_sees_the_cancel_promptly_and_stops_cleanly() {
    let (_dir, writer) = temp_writer();
    let updates = Updates::default();
    let (started_tx, started) = mpsc::channel();
    let started_tx = Mutex::new(started_tx);
    let steps = Arc::new(Mutex::new(0u32));
    let count = steps.clone();
    let queue = JobQueue::builder(writer.clone())
        .on_updates(updates.sink())
        .handler(JobKind::Fingerprint, move |job: &JobContext| {
            started_tx.lock().unwrap().send(()).unwrap();
            // Would take 100 s if nobody cancelled it.
            for step in 0..100_000u32 {
                job.check_cancelled()?;
                *count.lock().unwrap() = step;
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Fingerprint)).unwrap();
    started.recv_timeout(PATIENCE).unwrap();
    thread::sleep(Duration::from_millis(20));

    let asked = Instant::now();
    assert_eq!(queue.cancel(id).unwrap(), CancelOutcome::Stopping);
    let job = finished(&writer, id);
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "took {:?} to stop",
        asked.elapsed()
    );
    assert_eq!(job.status, JobStatus::Cancelled);
    assert_eq!(job.error, None);
    assert!(*steps.lock().unwrap() < 100_000);
    wait_until("the cancelled update is sent", || {
        updates.for_job(id).last().map(|u| u.0) == Some(JobStatus::Cancelled)
    });
    assert!(queue.activity().unwrap().jobs.is_empty());
}

#[test]
fn progress_reports_the_cancel_so_a_job_stops_at_its_next_report() {
    let (_dir, writer) = temp_writer();
    let (started_tx, started) = mpsc::channel();
    let started_tx = Mutex::new(started_tx);
    let queue = JobQueue::builder(writer.clone())
        .handler(JobKind::Hash, move |job: &JobContext| {
            started_tx.lock().unwrap().send(()).unwrap();
            for i in 0..100_000 {
                job.progress(i as f64 / 100_000.0)?;
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Hash)).unwrap();
    started.recv_timeout(PATIENCE).unwrap();
    assert_eq!(queue.cancel(id).unwrap(), CancelOutcome::Stopping);
    let job = finished(&writer, id);
    assert_eq!(job.status, JobStatus::Cancelled);
    assert!(job.progress.unwrap_or(0.0) < 1.0);
}

#[test]
fn cancelling_a_finished_or_unknown_job_does_nothing() {
    let (_dir, writer) = temp_writer();
    let queue = JobQueue::builder(writer.clone())
        .handler(JobKind::Export, |_: &JobContext| Ok(()))
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Export)).unwrap();
    finished(&writer, id);
    assert_eq!(queue.cancel(id).unwrap(), CancelOutcome::NotActive);
    assert_eq!(stored(&writer, id).status, JobStatus::Done);
    assert_eq!(queue.cancel(JobId(4242)).unwrap(), CancelOutcome::NotActive);
}

#[test]
fn a_job_that_finishes_its_work_despite_a_cancel_is_done() {
    let (_dir, writer) = temp_writer();
    let (handler, started, release) = gate();
    let queue = JobQueue::builder(writer.clone())
        .handler(JobKind::Export, handler)
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Export)).unwrap();
    started.recv_timeout(PATIENCE).unwrap();
    assert_eq!(queue.cancel(id).unwrap(), CancelOutcome::Stopping);
    // The gate never checks for cancellation, so its work completes.
    release.send(()).unwrap();
    assert_eq!(finished(&writer, id).status, JobStatus::Done);
}

#[test]
fn closing_the_app_puts_running_jobs_back_in_the_queue_not_cancelled() {
    let (_dir, writer) = temp_writer();
    let (started_tx, started) = mpsc::channel();
    let started_tx = Mutex::new(started_tx);
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, move |job: &JobContext| {
            started_tx.lock().unwrap().send(()).unwrap();
            while !job.is_cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
            Err(JobError::Cancelled)
        })
        .start()
        .unwrap();
    let running = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    let waiting = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    started.recv_timeout(PATIENCE).unwrap();
    drop(queue);

    for id in [running, waiting] {
        let job = stored(&writer, id);
        assert_eq!(job.status, JobStatus::Queued, "job {id}");
        assert!(job.started_at.is_none() && job.finished_at.is_none());
    }
    // The next launch runs them again, in their original order.
    let (handler, started, release) = gate();
    let next = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, handler)
        .start()
        .unwrap();
    assert_eq!(started.recv_timeout(PATIENCE).unwrap(), running);
    let shown: Vec<_> = next
        .activity()
        .unwrap()
        .jobs
        .iter()
        .map(|j| (j.id, j.status))
        .collect();
    assert_eq!(
        shown,
        [(running, JobStatus::Running), (waiting, JobStatus::Queued)]
    );
    release.send(()).unwrap();
    release.send(()).unwrap();
    assert_eq!(finished(&writer, waiting).status, JobStatus::Done);
    // Being put back on close didn't count as an attempt.
    assert_eq!(stored(&writer, running).attempts, 1);
}

// Review fixes: races, deadlocks and volume.

#[test]
fn once_the_database_says_a_job_finished_the_queue_agrees() {
    let (_dir, writer) = temp_writer();
    let queue = JobQueue::builder(writer.clone())
        .workers(2)
        .handler(JobKind::Export, |_: &JobContext| Ok(()))
        .start()
        .unwrap();
    for _ in 0..300 {
        let id = queue.enqueue(NewJob::new(JobKind::Export)).unwrap();
        // Poll without pausing, to land right after the finish is stored.
        loop {
            let status = stored(&writer, id).status;
            if status.is_finished() {
                assert_eq!(queue.cancel(id).unwrap(), CancelOutcome::NotActive);
                let active = queue.activity().unwrap();
                assert!(active.jobs.iter().all(|j| j.id != id), "{id} still active");
                break;
            }
        }
    }
}

#[test]
fn queue_calls_from_inside_a_writer_call_are_refused_instead_of_deadlocking() {
    let (_dir, writer) = temp_writer();
    let queue = Arc::new(
        JobQueue::builder(writer.clone())
            .workers(1)
            .start()
            .unwrap(),
    );

    // Another caller takes the queue's lock and then waits for the writer,
    // which is busy running the call below.
    let (in_call, entered) = mpsc::channel();
    let other_queue = queue.clone();
    let other = thread::spawn(move || {
        entered.recv().unwrap();
        other_queue.enqueue(NewJob::new(JobKind::Export))
    });

    let (answer, answers) = mpsc::channel();
    let inner_queue = queue.clone();
    let inner_writer = writer.clone();
    thread::spawn(move || {
        let result = inner_writer.call(move |_| {
            in_call.send(()).unwrap();
            // Let the other caller take the lock.
            thread::sleep(Duration::from_millis(200));
            let refused = |e: &DbError| matches!(e, DbError::Reentrant);
            Ok([
                inner_queue
                    .enqueue(NewJob::new(JobKind::Export))
                    .is_err_and(|e| refused(&e)),
                inner_queue.cancel(JobId(1)).is_err_and(|e| refused(&e)),
                inner_queue.activity().is_err_and(|e| refused(&e)),
            ])
        });
        let _ = answer.send(result.unwrap());
    });

    let refused = answers
        .recv_timeout(PATIENCE)
        .expect("a queue call inside a writer call deadlocked");
    assert_eq!(refused, [true, true, true]);
    // The other caller wasn't stuck either.
    assert!(other.join().unwrap().is_ok());
}

#[test]
fn thousands_of_quick_changes_reach_the_sink_in_a_few_batches() {
    let (_dir, writer) = temp_writer();
    let updates = Updates::default();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(updates.sink())
        .start()
        .unwrap();
    queue.shutdown();
    let ids: Vec<_> = (0..2_000)
        .map(|_| queue.enqueue(NewJob::new(JobKind::Scan)).unwrap())
        .collect();
    let last = *ids.last().unwrap();
    wait_until("the last job's update is sent", || {
        updates.all().iter().any(|u| u.id == last)
    });
    let batches = updates.0.lock().unwrap().len();
    // 2 000 updates; at least a few per 50 ms batch even on a slow runner.
    assert!(batches <= 400, "{batches} batches for 2 000 updates");
    let seen: std::collections::BTreeSet<_> = updates.all().iter().map(|u| u.id).collect();
    assert_eq!(seen.len(), ids.len(), "every job's update arrived");
}

// 1aA-13: shutdown that can't hang or deadlock, and progress that never
// goes back.

/// A handler that ignores cancellation until it's released, like a job
/// stuck in a slow call. `started` gets the job id.
fn stuck() -> (impl JobHandler, mpsc::Receiver<JobId>, mpsc::Sender<()>) {
    let (started_tx, started) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let started_tx = Mutex::new(started_tx);
    let release_rx = Mutex::new(release_rx);
    let handler = move |job: &JobContext| {
        started_tx.lock().unwrap().send(job.id()).unwrap();
        let _ = release_rx.lock().unwrap().recv_timeout(PATIENCE);
        Ok(())
    };
    (handler, started, release)
}

#[test]
fn shutdown_with_a_stuck_job_returns_within_the_time_limit() {
    let (_dir, writer) = temp_writer();
    let (handler, started, release) = stuck();
    let limit = Duration::from_millis(300);
    let queue = JobQueue::builder(writer.clone())
        .workers(2)
        .shutdown_timeout(limit)
        .handler(JobKind::Scan, handler)
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    started.recv_timeout(PATIENCE).unwrap();

    let start = Instant::now();
    queue.shutdown();
    let took = start.elapsed();
    assert!(took >= limit, "gave up after {took:?}, before the limit");
    assert!(took < limit + Duration::from_secs(2), "took {took:?}");
    // Left for crash resume to find, not marked as finished.
    assert_eq!(stored(&writer, id).status, JobStatus::Running);
    // Calling it again (here, on drop) doesn't wait again.
    let again = Instant::now();
    drop(queue);
    assert!(again.elapsed() < limit, "{:?}", again.elapsed());
    let _ = release.send(());
}

#[test]
fn shutdown_waits_for_jobs_that_stop_in_time() {
    let (_dir, writer) = temp_writer();
    let (started_tx, started) = mpsc::channel();
    let started_tx = Mutex::new(started_tx);
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .shutdown_timeout(PATIENCE)
        .handler(JobKind::Scan, move |job: &JobContext| {
            started_tx.lock().unwrap().send(()).unwrap();
            while !job.is_cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
            // Takes a moment to wind down, well within the limit.
            thread::sleep(Duration::from_millis(100));
            Err(JobError::Cancelled)
        })
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    started.recv_timeout(PATIENCE).unwrap();
    queue.shutdown();
    // It waited: the job is already back in the queue.
    assert_eq!(stored(&writer, id).status, JobStatus::Queued);
}

#[test]
fn the_default_shutdown_limit_is_a_few_seconds() {
    assert!(SHUTDOWN_TIMEOUT >= Duration::from_secs(1));
    assert!(SHUTDOWN_TIMEOUT <= Duration::from_secs(5));
}

#[test]
fn shutdown_from_inside_a_writer_call_returns_at_once_instead_of_deadlocking() {
    let (_dir, writer) = temp_writer();
    let (started_tx, started) = mpsc::channel();
    let started_tx = Mutex::new(started_tx);
    // A running job that keeps writing, so its worker is often waiting for
    // the writer while the writer runs the call below.
    let queue = Arc::new(
        JobQueue::builder(writer.clone())
            .workers(1)
            .shutdown_timeout(PATIENCE)
            .handler(JobKind::Scan, move |job: &JobContext| {
                let _ = started_tx.lock().unwrap().send(());
                while !job.is_cancelled() {
                    job.writer().call(|c| c.execute_batch("SELECT 1"))?;
                }
                Err(JobError::Cancelled)
            })
            .start()
            .unwrap(),
    );
    let id = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    started.recv_timeout(PATIENCE).unwrap();

    let (answer, answers) = mpsc::channel();
    let inner = queue.clone();
    let inner_writer = writer.clone();
    thread::spawn(move || {
        let result = inner_writer.call(move |_| {
            inner.shutdown();
            Ok(())
        });
        let _ = answer.send(result);
    });
    answers
        .recv_timeout(PATIENCE)
        .expect("shutdown inside a writer call deadlocked")
        .unwrap();
    // The workers still stop: the running job goes back in the queue.
    wait_until("the job is back in the queue", || {
        stored(&writer, id).status == JobStatus::Queued
    });
}

#[test]
fn a_job_can_shut_down_its_own_queue_without_waiting_for_itself() {
    let (_dir, writer) = temp_writer();
    let own_queue: Arc<Mutex<Option<Arc<JobQueue>>>> = Arc::default();
    let (done_tx, done) = mpsc::channel();
    let done_tx = Mutex::new(done_tx);
    let handler_queue = own_queue.clone();
    let queue = Arc::new(
        JobQueue::builder(writer.clone())
            .workers(1)
            .shutdown_timeout(PATIENCE)
            .handler(JobKind::Scan, move |job: &JobContext| {
                let queue = handler_queue.lock().unwrap().clone().unwrap();
                queue.shutdown();
                // It returned at once; the shutdown then asks this job to
                // stop, from its helper thread.
                let start = Instant::now();
                while !job.is_cancelled() && start.elapsed() < PATIENCE {
                    thread::sleep(Duration::from_millis(1));
                }
                let cancelled = job.is_cancelled();
                done_tx.lock().unwrap().send(cancelled).unwrap();
                Err(JobError::Cancelled)
            })
            .start()
            .unwrap(),
    );
    *own_queue.lock().unwrap() = Some(queue.clone());
    let id = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    let cancelled = done
        .recv_timeout(PATIENCE)
        .expect("a job shutting down its own queue hung");
    assert!(cancelled);
    wait_until("the job is back in the queue", || {
        stored(&writer, id).status == JobStatus::Queued
    });
    // Break the handler's hold on its own queue, so the queue is dropped.
    own_queue.lock().unwrap().take();
}

#[test]
fn progress_never_goes_backwards_under_concurrent_updates() {
    for _ in 0..50 {
        let (_dir, writer) = temp_writer();
        let updates = Updates::default();
        let queue = JobQueue::builder(writer.clone())
            .workers(1)
            .on_updates(updates.sink())
            .handler(JobKind::Hash, |job: &JobContext| {
                // 16 threads each report every 16th step, in rising order,
                // so their reports interleave and race. Steps are bigger
                // than PROGRESS_STEP, so most reports get past the atomic
                // swap and contend for the lock.
                thread::scope(|scope| {
                    for t in 0..16 {
                        scope.spawn(move || -> Result<(), JobError> {
                            for i in (t..=64).step_by(16) {
                                job.progress(i as f64 / 64.0)?;
                            }
                            Ok(())
                        });
                    }
                });
                job.progress(1.0)
            })
            .start()
            .unwrap();
        let id = queue.enqueue(NewJob::new(JobKind::Hash)).unwrap();
        finished(&writer, id);
        wait_until("the last update is sent", || {
            updates.for_job(id).last().map(|u| u.0) == Some(JobStatus::Done)
        });
        let progress: Vec<f64> = updates
            .for_job(id)
            .into_iter()
            .filter_map(|(_, p)| p)
            .collect();
        assert!(progress.windows(2).all(|w| w[1] >= w[0]), "{progress:?}");
        assert_eq!(stored(&writer, id).progress, Some(1.0));
    }
}

#[test]
fn a_progress_report_lower_than_the_last_one_is_ignored() {
    let (_dir, writer) = temp_writer();
    let updates = Updates::default();
    let (stored_tx, stored_rx) = mpsc::channel();
    let stored_tx = Mutex::new(stored_tx);
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(updates.sink())
        .handler(JobKind::Hash, move |job: &JobContext| {
            for fraction in [0.5, 0.2, 0.495, 0.6, 0.3] {
                job.progress(fraction)?;
                let id = job.id();
                let now = job.writer().call(move |c| store::get(c, id)).unwrap();
                stored_tx
                    .lock()
                    .unwrap()
                    .send(now.unwrap().progress)
                    .unwrap();
            }
            Ok(())
        })
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Hash)).unwrap();
    let stored_each: Vec<_> = (0..5)
        .map(|_| stored_rx.recv_timeout(PATIENCE).unwrap())
        .collect();
    assert_eq!(
        stored_each,
        [Some(0.5), Some(0.5), Some(0.5), Some(0.6), Some(0.6)]
    );
    finished(&writer, id);
    wait_until("the last update is sent", || {
        updates.for_job(id).last().map(|u| u.0) == Some(JobStatus::Done)
    });
    let sent: Vec<f64> = updates
        .for_job(id)
        .into_iter()
        .filter(|(status, _)| *status == JobStatus::Running)
        .filter_map(|(_, p)| p)
        .collect();
    // Batching can merge updates, but the lower ones are never sent.
    assert!(sent.iter().all(|p| [0.5, 0.6].contains(p)), "{sent:?}");
}

#[test]
fn a_lower_step_that_takes_the_lock_last_never_overwrites_a_higher_one() {
    use super::queue::test_hooks::pause_before_progress_lock;

    let (_dir, writer) = temp_writer();
    let updates = Updates::default();
    let (seen_tx, seen) = mpsc::channel();
    let seen_tx = Mutex::new(seen_tx);
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(updates.sink())
        .handler(JobKind::Hash, move |job: &JobContext| {
            // A claims 0.5 first but stops before the lock; B claims 0.6,
            // stores it, and only then lets A go on.
            let (b_done_tx, b_done) = mpsc::channel::<()>();
            let (a_claimed_tx, a_claimed) = mpsc::channel::<()>();
            thread::scope(|scope| {
                scope.spawn(move || {
                    pause_before_progress_lock(move || {
                        a_claimed_tx.send(()).unwrap();
                        b_done.recv_timeout(PATIENCE).unwrap();
                    });
                    job.progress(0.5)
                });
                scope.spawn(move || {
                    a_claimed.recv_timeout(PATIENCE).unwrap();
                    let result = job.progress(0.6);
                    b_done_tx.send(()).unwrap();
                    result
                });
            });
            let id = job.id();
            let now = job.writer().call(move |c| store::get(c, id)).unwrap();
            seen_tx.lock().unwrap().send(now.unwrap().progress).unwrap();
            Ok(())
        })
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Hash)).unwrap();
    assert_eq!(seen.recv_timeout(PATIENCE).unwrap(), Some(0.6));
    finished(&writer, id);
    wait_until("the last update is sent", || {
        updates.for_job(id).last().map(|u| u.0) == Some(JobStatus::Done)
    });
    let sent: Vec<f64> = updates
        .for_job(id)
        .into_iter()
        .filter_map(|(_, p)| p)
        .collect();
    assert!(!sent.contains(&0.5), "{sent:?}");
    assert!(sent.windows(2).all(|w| w[1] >= w[0]), "{sent:?}");
}

#[test]
fn if_shutdown_cannot_start_its_helper_the_workers_still_stop_taking_jobs() {
    let (_dir, writer) = temp_writer();
    let (handler, started, release) = gate();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, handler)
        .start()
        .unwrap();
    let running = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    let waiting = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    assert_eq!(started.recv_timeout(PATIENCE).unwrap(), running);

    // From the writer thread, as when shutdown is called there and its
    // helper thread can't be started.
    let for_writer = Arc::new(queue);
    let inner = for_writer.clone();
    writer
        .call(move |_| {
            super::queue::signal_stop_for_tests(&inner);
            Ok(())
        })
        .unwrap();
    release.send(()).unwrap();
    finished(&writer, running);
    // A later enqueue wakes the worker, which stops instead of taking a job.
    let later = for_writer.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    thread::sleep(Duration::from_millis(200));
    for id in [waiting, later] {
        assert_eq!(stored(&writer, id).status, JobStatus::Queued, "job {id}");
    }
    assert!(
        started.try_recv().is_err(),
        "a worker took a job after the stop"
    );
}
