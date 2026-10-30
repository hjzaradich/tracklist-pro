//! The `index` job: it brings the index up to date in the background, from
//! what `search_due` holds, and is asked for wherever something is due.

use super::support::{Lib, Mount};
use crate::db::Writer;
use crate::jobs::{self, JobKind, JobQueue, JobRecord, JobStatus, Priority};
use crate::scan::chain::after_index;
use crate::search::{index_job, indexer_with, request, settle_page};

fn queue(lib: &Lib) -> JobQueue {
    JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(JobKind::Index, after_index(indexer_with(|| Mount, 2)))
        .start()
        .unwrap()
}

fn index_jobs(writer: &Writer) -> Vec<JobRecord> {
    writer
        .call(|c| {
            let ids: Vec<i64> = c
                .prepare("SELECT id FROM job WHERE kind = 'index' ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            ids.into_iter()
                .map(|id| jobs::store::get(c, jobs::JobId(id)).map(Option::unwrap))
                .collect()
        })
        .unwrap()
}

/// Waits until nothing is queued or running.
fn wait_idle(queue: &JobQueue, writer: &Writer) -> Vec<JobRecord> {
    crate::jobs::idle::wait_until_idle(queue);
    index_jobs(writer)
}

fn request_now(lib: &Lib, queue: &JobQueue) {
    request(&lib.writer, |j| queue.enqueue(j)).unwrap();
}

#[test]
fn the_index_job_works_out_every_track_that_is_due_and_leaves_the_queue_idle() {
    let lib = Lib::new();
    for n in 0..7 {
        lib.track(Some(&format!("Track {n}")), None, &format!("{n}.mp3"));
    }
    let queue = queue(&lib);
    request_now(&lib, &queue);
    let jobs = wait_idle(&queue, &lib.writer);
    queue.shutdown();

    assert_eq!(
        jobs.iter().map(|j| j.status).collect::<Vec<_>>(),
        [JobStatus::Done]
    );
    assert!(!lib.pending());
    assert_eq!(lib.found_as_is("track").unwrap().tracks.len(), 7);
}

#[test]
fn nothing_is_queued_when_nothing_is_due_and_never_a_second_job_while_one_waits() {
    let lib = Lib::new();
    // No worker takes what is queued: the queue is started and stopped
    // before anything is asked for (`workers(0)` would still start one, as
    // a queue always has at least one), so a queued job stays queued.
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .start()
        .unwrap();
    queue.shutdown();
    request_now(&lib, &queue);
    assert!(index_jobs(&lib.writer).is_empty());

    lib.track(Some("Due"), None, "a.mp3");
    request_now(&lib, &queue);
    request_now(&lib, &queue);
    let waiting = index_jobs(&lib.writer);
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].priority, Priority::BACKGROUND);
    assert_eq!(waiting[0].status, JobStatus::Queued);
}

#[test]
fn adding_a_track_to_the_library_then_asking_makes_the_new_shown_title_findable() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Tag title"), None, "a.mp3");
    lib.rekordbox_entry(file, &[("Name", "Their title")]);
    let queue = queue(&lib);
    request_now(&lib, &queue);
    wait_idle(&queue, &lib.writer);
    assert_eq!(lib.found_as_is("their").unwrap().tracks, Vec::<i64>::new());

    // The user adds it to the Library: nothing is queued by that alone, and
    // a search says the index is behind.
    lib.library(track, file);
    let behind = lib.found_as_is("tag").unwrap();
    assert!(behind.behind);
    assert!(index_jobs(&lib.writer)
        .iter()
        .all(|j| j.status == JobStatus::Done));

    // The search command asks, as the later screen task does.
    request_now(&lib, &queue);
    wait_idle(&queue, &lib.writer);
    queue.shutdown();
    let after = lib.found_as_is("their").unwrap();
    assert_eq!(after.tracks, vec![track]);
    assert!(!after.behind);
}

#[test]
fn an_index_job_cut_short_by_a_crash_is_queued_again_and_finishes_from_what_is_still_due() {
    let lib = Lib::new();
    for n in 0..5 {
        lib.track(Some(&format!("Track {n}")), None, &format!("{n}.mp3"));
    }
    // A run that stopped after its first page, and its job left running, as
    // a crash leaves them.
    assert_eq!(lib.writer.call(|c| settle_page(c, &Mount, 2)).unwrap(), 2);
    assert_eq!(lib.found_as_is("track").unwrap().tracks.len(), 2);
    lib.writer
        .call(|c| {
            c.execute(
                "INSERT INTO job (kind, priority, status, started_at, progress)
                 VALUES (?1, ?2, 'running', 'now', 0.4)",
                (JobKind::Index.as_str(), Priority::BACKGROUND.0),
            )?;
            jobs::store::recover_interrupted(c)
        })
        .unwrap();
    assert_eq!(
        index_jobs(&lib.writer)
            .iter()
            .map(|j| j.status)
            .collect::<Vec<_>>(),
        [JobStatus::Queued]
    );

    // The app starts again.
    let queue = queue(&lib);
    let jobs = wait_idle(&queue, &lib.writer);
    queue.shutdown();
    assert_eq!(
        jobs.iter().map(|j| j.status).collect::<Vec<_>>(),
        [JobStatus::Done]
    );
    assert_eq!(lib.found_as_is("track").unwrap().tracks.len(), 5);
    assert!(!lib.pending());
}

#[test]
fn a_rebuild_job_throws_the_rows_away_and_works_them_all_out_again() {
    let lib = Lib::new();
    let (track, _) = lib.track(Some("Real"), None, "a.mp3");
    lib.settle();
    lib.exec("INSERT INTO search_track (rowid, title) VALUES (99, 'stray')");
    assert_eq!(lib.found_as_is("stray").unwrap().tracks, vec![99]);

    let queue = queue(&lib);
    queue.enqueue(index_job(true)).unwrap();
    wait_idle(&queue, &lib.writer);
    queue.shutdown();
    assert_eq!(lib.found_as_is("stray").unwrap().tracks, Vec::<i64>::new());
    assert_eq!(lib.found_as_is("real").unwrap().tracks, vec![track]);
}

#[test]
fn the_index_job_gives_way_to_a_waiting_job_of_a_higher_priority_and_finishes_after_it() {
    let lib = Lib::new();
    for n in 0..6 {
        lib.track(Some(&format!("Track {n}")), None, &format!("{n}.mp3"));
    }
    // When the first index job starts, a job of a higher priority is waiting.
    let writer = lib.writer.clone();
    let once = std::sync::atomic::AtomicBool::new(false);
    let volumes = move || {
        if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
            writer
                .call(|c| {
                    c.execute(
                        "INSERT INTO job (kind, priority, status) VALUES ('scan', ?1, 'queued')",
                        [Priority::BACKGROUND.0 + 1],
                    )
                })
                .unwrap();
        }
        Mount
    };
    // What was still due when the other job ran.
    let due_then = std::sync::Arc::new(std::sync::Mutex::new(None));
    let seen = due_then.clone();
    let writer = lib.writer.clone();
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(JobKind::Index, after_index(indexer_with(volumes, 2)))
        .handler(JobKind::Scan, move |_: &jobs::JobContext| {
            let due: i64 = writer
                .call(|c| c.query_row("SELECT count(*) FROM search_due", [], |r| r.get(0)))
                .map_err(jobs::JobError::from)?;
            *seen.lock().unwrap() = Some(due);
            Ok(())
        })
        .start()
        .unwrap();
    request_now(&lib, &queue);
    let jobs = wait_idle(&queue, &lib.writer);
    queue.shutdown();

    // The first run settled nothing before the other job; a second run
    // (queued as the first gave way) did the rest from `search_due`.
    assert_eq!(*due_then.lock().unwrap(), Some(6));
    assert_eq!(
        jobs.iter().map(|j| j.status).collect::<Vec<_>>(),
        [JobStatus::Done, JobStatus::Done]
    );
    assert!(!lib.pending());
    assert_eq!(lib.found_as_is("track").unwrap().tracks.len(), 6);
}
