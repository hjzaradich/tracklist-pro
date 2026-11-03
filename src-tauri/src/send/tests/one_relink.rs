//! The relink a send's prepare's read queues is withdrawn once the
//! prepare's own matching has succeeded, if it hasn't started (1cD-X1). If
//! the prepare stops, fails or panics before that, it stays queued, as it
//! does if the app dies. A relink queued by anything else is never
//! withdrawn. Every name is invented.

use std::sync::mpsc;
use std::sync::Mutex;

use super::*;
use crate::jobs::JobId;

/// Relink jobs in the table that were not withdrawn (cancelled).
fn relink_jobs(w: &World) -> i64 {
    w.writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM job WHERE kind = 'relink' AND status != 'cancelled'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap()
}

#[test]
fn a_prepare_that_matches_the_tracks_itself_withdraws_the_reads_relink() {
    let (w, ..) = world_with_a_send_waiting();
    assert!(w.prepare().is_some());
    assert_eq!(relink_jobs(&w), 0);
}

#[test]
fn a_prepare_cancelled_after_its_read_and_before_its_matching_leaves_a_relink_queued() {
    let (w, ..) = world_with_a_send_waiting();
    let (reached, at) = mpsc::channel::<()>();
    let (go_on, resume) = mpsc::channel::<()>();
    let (reached, resume) = (Mutex::new(reached), Mutex::new(resume));
    let sender = w.sender().after_read(move || {
        reached.lock().unwrap().send(()).unwrap();
        let _ = resume
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(30));
    });
    let queue = JobQueue::builder(w.writer.clone())
        .workers(1)
        .handler(JobKind::Export, sender)
        .start()
        .unwrap();
    let id: JobId = queue
        .enqueue(w.flow.prepare_job(&w.export.to_string_lossy()))
        .unwrap();
    at.recv_timeout(std::time::Duration::from_secs(60)).unwrap();
    queue.cancel(id).unwrap();
    go_on.send(()).unwrap();
    let start = Instant::now();
    let status = loop {
        let job = w
            .writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            break job.status;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(60));
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    queue.shutdown();
    assert_eq!(status, JobStatus::Cancelled);
    assert_eq!(w.flow.preflight(), None);
    assert_eq!(relink_jobs(&w), 1);
}

#[test]
fn a_prepare_that_fails_after_its_read_and_before_its_matching_leaves_a_relink_queued() {
    let (w, ..) = world_with_a_send_waiting();
    let writer = w.writer.clone();
    // The matching reads the volume table; take it away once the read is in.
    let sender = w.sender().after_read(move || {
        writer
            .call(|c| c.execute_batch("ALTER TABLE volume RENAME TO volume_gone"))
            .unwrap();
    });
    let queue = JobQueue::builder(w.writer.clone())
        .workers(1)
        .handler(JobKind::Export, sender)
        .start()
        .unwrap();
    let id: JobId = queue
        .enqueue(w.flow.prepare_job(&w.export.to_string_lossy()))
        .unwrap();
    let start = Instant::now();
    let status = loop {
        let job = w
            .writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            break job.status;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(60));
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    queue.shutdown();
    assert!(matches!(status, JobStatus::Failed), "{status:?}");
    assert_eq!(relink_jobs(&w), 1);
}

#[test]
fn a_prepare_that_panics_after_its_read_and_before_its_matching_leaves_a_relink_queued() {
    let (w, ..) = world_with_a_send_waiting();
    let sender = w.sender().after_read(|| panic!("invented panic"));
    let queue = JobQueue::builder(w.writer.clone())
        .workers(1)
        .handler(JobKind::Export, sender)
        .start()
        .unwrap();
    let id: JobId = queue
        .enqueue(w.flow.prepare_job(&w.export.to_string_lossy()))
        .unwrap();
    let start = Instant::now();
    let status = loop {
        let job = w
            .writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            break job.status;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(60));
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    queue.shutdown();
    assert!(matches!(status, JobStatus::Failed), "{status:?}");
    assert_eq!(relink_jobs(&w), 1);
}

#[test]
fn a_relink_queued_by_something_else_is_never_withdrawn() {
    let (w, ..) = world_with_a_send_waiting();
    // Waiting before the prepare begins, say from Review: the read's own
    // request finds it and queues nothing more.
    let other = w
        .writer
        .call(|c| {
            c.execute(
                "INSERT INTO job (kind, status, priority) VALUES ('relink', 'queued', 0)",
                [],
            )?;
            Ok(c.last_insert_rowid())
        })
        .unwrap();
    assert!(w.prepare().is_some());
    let status: String = w
        .writer
        .call(move |c| {
            c.query_row("SELECT status FROM job WHERE id = ?1", [other], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_ne!(status, "cancelled");
}
