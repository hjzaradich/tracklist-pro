//! The matching job (1bA-14): it makes way for a relink or attach queued
//! while it runs, a failure or panic in it ends that job alone, and a job
//! cut short by a crash runs again and finishes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::db::{DbError, Writer};
use crate::jobs::{self, JobContext, JobKind, JobQueue, JobRecord, JobStatus};
use crate::matching::{matching_job, refresh, store, Matcher, PRIORITY};
use crate::scan::chain::after_matching;

use super::passes::{db, Db};
use super::synthetic::{reencoded, track};

/// 120 made-up tracks, every tenth with a re-encoded copy: a pass reports
/// progress well over a hundred times.
fn library() -> Db {
    let db = db();
    for seed in 1..=120u64 {
        let items = track(seed, 400);
        db.add_items(&format!("{seed}.flac"), items.clone());
        if seed % 10 == 0 {
            db.add_items(&format!("{seed}.mp3"), reencoded(&items, seed, 7));
        }
    }
    db
}

/// The pairs a full pass stores for [`library`].
fn all_pairs() -> Vec<(i64, i64)> {
    let db = library();
    refresh(&db.writer).unwrap();
    db.pairs()
}

/// Every job, in the order they were queued.
fn jobs(writer: &Writer) -> Vec<JobRecord> {
    writer
        .call(|c| {
            let ids: Vec<i64> = c
                .prepare("SELECT id FROM job ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            ids.into_iter()
                .map(|id| jobs::store::get(c, jobs::JobId(id)).map(Option::unwrap))
                .collect()
        })
        .unwrap()
}

/// Waits until `count` jobs exist and every one has finished.
fn wait_for_jobs(writer: &Writer, count: usize) -> Vec<JobRecord> {
    let start = Instant::now();
    loop {
        let all = jobs(writer);
        if all.len() >= count && all.iter().all(|j| j.status.is_finished()) {
            return all;
        }
        assert!(
            start.elapsed() < Duration::from_secs(300),
            "the jobs never finished: {all:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn kinds(jobs: &[JobRecord]) -> Vec<(Option<JobKind>, JobStatus)> {
    jobs.iter().map(|j| (j.kind, j.status)).collect()
}

#[test]
fn a_matching_job_is_queued_below_the_rest_of_the_chain() {
    let job = matching_job();
    assert_eq!(job.kind, JobKind::Match);
    assert_eq!(job.priority, PRIORITY);
    assert!(PRIORITY < jobs::Priority::BACKGROUND);
    assert!(PRIORITY < crate::relink::relink_job().priority);
    assert!(PRIORITY < crate::attach::attach_job().priority);
}

#[test]
fn a_matching_job_makes_way_for_a_relink_queued_while_it_runs_and_finishes_after_it() {
    let expected = all_pairs();
    let db = library();
    // The pass stops at its fifth report until the test has queued a relink.
    let (reached, reached_rx) = mpsc::channel::<()>();
    let (go, go_rx) = mpsc::channel::<()>();
    let go_rx = Mutex::new(go_rx);
    let once = AtomicBool::new(false);
    let matcher = Matcher::new().on_report(move |n| {
        if n == 5 && !once.swap(true, Ordering::SeqCst) {
            reached.send(()).unwrap();
            go_rx.lock().unwrap().recv().unwrap();
        }
    });
    // What the table held when the relink ran.
    let seen_by_relink: Arc<Mutex<Option<usize>>> = Arc::default();
    let seen = seen_by_relink.clone();
    let writer = db.writer.clone();
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        .handler(JobKind::Match, after_matching(matcher))
        .handler(JobKind::Relink, move |_: &JobContext| {
            let rows: i64 = writer
                .call(|c| c.query_row("SELECT COUNT(*) FROM fingerprint_match", [], |r| r.get(0)))
                .map_err(jobs::JobError::from)?;
            *seen.lock().unwrap() = Some(rows as usize);
            Ok(())
        })
        .start()
        .unwrap();
    queue.enqueue(matching_job()).unwrap();
    reached_rx.recv().unwrap();
    queue.enqueue(crate::relink::relink_job()).unwrap();
    go.send(()).unwrap();

    let all = wait_for_jobs(&db.writer, 3);
    queue.shutdown();
    // The first run made way; the relink went next, then matching again.
    assert_eq!(
        kinds(&all),
        [
            (Some(JobKind::Match), JobStatus::Done),
            (Some(JobKind::Relink), JobStatus::Done),
            (Some(JobKind::Match), JobStatus::Done),
        ]
    );
    assert_eq!(all[2].priority, PRIORITY, "the rerun keeps its priority");
    let seen = seen_by_relink.lock().unwrap().unwrap();
    assert!(
        seen < expected.len(),
        "the relink ran before matching was through: {seen} of {} rows",
        expected.len()
    );
    assert_eq!(db.pairs(), expected, "and matching finished afterwards");
}

#[test]
fn a_matching_job_that_panics_ends_failed_and_the_next_one_finishes_the_work() {
    let expected = all_pairs();
    let db = library();
    let once = AtomicBool::new(false);
    let matcher = Matcher::new().on_report(move |n| {
        if n == 20 && !once.swap(true, Ordering::SeqCst) {
            panic!("a made-up failure in the middle of a pass");
        }
    });
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        .handler(JobKind::Match, after_matching(matcher))
        .start()
        .unwrap();
    queue.enqueue(matching_job()).unwrap();
    let first = wait_for_jobs(&db.writer, 1);
    assert_eq!(kinds(&first), [(Some(JobKind::Match), JobStatus::Failed)]);
    assert!(db.pairs().len() < expected.len(), "it really stopped early");

    // Asking again queues a job of its own: the failed one swallows nothing.
    crate::scan::chain::queue_once(&db.writer, matching_job(), |j| queue.enqueue(j)).unwrap();
    let all = wait_for_jobs(&db.writer, 2);
    queue.shutdown();
    assert_eq!(all[1].status, JobStatus::Done, "{all:?}");
    assert_eq!(db.pairs(), expected);
}

#[test]
fn a_matching_job_cut_short_by_a_crash_is_queued_again_and_finishes() {
    let expected = all_pairs();
    let db = library();
    // A pass that stopped partway, and its job left running, as a crash
    // leaves them.
    let mut ticks = 0;
    let stopped = Matcher::new().pass(&db.writer, &mut |_| {
        ticks += 1;
        if ticks > 30 {
            Err(DbError::WriterGone)
        } else {
            Ok(())
        }
    });
    assert!(stopped.is_err());
    assert!(db.pairs().len() < expected.len());
    db.writer
        .call(move |c| {
            c.execute(
                "INSERT INTO job (kind, priority, status, started_at, progress)
                 VALUES (?1, ?2, 'running', 'now', 0.3)",
                (JobKind::Match.as_str(), PRIORITY.0),
            )?;
            jobs::store::recover_interrupted(c)
        })
        .unwrap();
    assert_eq!(
        kinds(&jobs(&db.writer)),
        [(Some(JobKind::Match), JobStatus::Queued)]
    );

    // The app starts again.
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        .handler(JobKind::Match, after_matching(Matcher::new()))
        .start()
        .unwrap();
    let all = wait_for_jobs(&db.writer, 1);
    queue.shutdown();
    assert_eq!(kinds(&all), [(Some(JobKind::Match), JobStatus::Done)]);
    assert_eq!(db.pairs(), expected);
}

fn due(db: &Db) -> bool {
    db.writer.call(|c| store::any_due(c)).unwrap()
}

#[test]
fn matching_is_due_until_a_pass_finishes_and_again_when_a_fingerprint_arrives_or_a_file_comes_or_goes(
) {
    let db = db();
    assert!(!due(&db), "no fingerprints: nothing to do");
    let a = track(1, 400);
    db.add_items("a.flac", a.clone());
    assert!(due(&db));

    // A pass that stops early has covered nothing.
    let stopped = Matcher::new().pass(&db.writer, &mut |_| Err::<(), _>(DbError::WriterGone));
    assert!(stopped.is_err());
    assert!(due(&db));
    refresh(&db.writer).unwrap();
    assert!(!due(&db));

    // A tag rewrite or a new modified time isn't new audio (ROADMAP 5.1).
    db.sql(
        "UPDATE file SET size = size + 4096, mtime = mtime + 5000000",
        (),
    );
    assert!(!due(&db));

    // A new file with a fingerprint.
    let copy = db.add_items("a.mp3", reencoded(&a, 1, 7));
    assert!(due(&db));
    refresh(&db.writer).unwrap();
    assert!(!due(&db));
    assert_eq!(db.pairs(), [(1, copy)]);

    // A file that goes missing, and one that comes back.
    db.sql("UPDATE file SET present = 0 WHERE id = 1", ());
    assert!(due(&db));
    refresh(&db.writer).unwrap();
    assert!(!due(&db));
    db.sql("UPDATE file SET present = 1 WHERE id = 1", ());
    assert!(due(&db));

    // The fingerprint stage did a file again (its audio changed).
    refresh(&db.writer).unwrap();
    assert!(!due(&db));
    db.sql(
        "INSERT INTO file_stage (file_id, stage, version, status, done_at)
         VALUES (1, 'fingerprint', 1, 'done', '2999-01-01T00:00:00.000Z')",
        (),
    );
    assert!(due(&db));
}
