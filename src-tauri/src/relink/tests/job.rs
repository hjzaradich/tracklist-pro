//! The relink job and how it's queued.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::jobs::{self, JobContext, JobError, JobId, JobKind, JobQueue, JobStatus, NewJob};
use crate::paths::Volumes;
use crate::relink::{request, Relinker};
use crate::volume::{Volume, VolumeId};

const PATIENCE: Duration = Duration::from_secs(60);

/// Volumes plugged in at fixed mount points.
#[derive(Clone)]
struct Plugged(Vec<(String, PathBuf)>);

impl Volumes for Plugged {
    fn volume_for(&self, path: &Path) -> io::Result<Volume> {
        Err(io::Error::other(format!("not asked: {}", path.display())))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        self.0
            .iter()
            .find(|(known, _)| known == id.as_str())
            .map(|(_, mount)| mount.clone())
    }
}

fn plugged_e() -> Plugged {
    Plugged(vec![(serial(1), PathBuf::from(r"E:\"))])
}

fn status(writer: &Writer, id: JobId) -> JobStatus {
    writer
        .call(move |c| jobs::store::get(c, id))
        .unwrap()
        .unwrap()
        .status
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < PATIENCE, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_done(writer: &Writer, id: JobId) {
    wait_until(&format!("job {id} finished"), || {
        status(writer, id).is_finished()
    });
    assert_eq!(status(writer, id), JobStatus::Done);
}

/// Relink jobs in the table, by status.
fn relink_jobs(writer: &Writer) -> Vec<(i64, String)> {
    writer
        .call(|c| {
            c.prepare("SELECT id, status FROM job WHERE kind = 'relink' ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })
        .unwrap()
}

/// A handler that says when it starts, then waits to be released.
fn gate() -> (
    impl Fn(&JobContext) -> Result<(), JobError> + Send + Sync + 'static,
    mpsc::Receiver<JobId>,
    mpsc::Sender<()>,
) {
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

#[test]
fn a_relink_job_matches_tracks_on_the_volumes_plugged_in_now() {
    let (lib, music, _) = e_music();
    let file = lib.file(music, "Alpha.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Alpha.mp3"), Some("200"));
    let (tx, summaries) = mpsc::channel();
    let tx = Mutex::new(tx);
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(
            JobKind::Relink,
            Relinker::new(plugged_e).on_summary(move |s| tx.lock().unwrap().send(s).unwrap()),
        )
        .start()
        .unwrap();
    let id = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    wait_done(&lib.writer, id);
    assert_eq!(lib.matched(track), path(file));
    let summary = summaries.recv_timeout(PATIENCE).unwrap();
    assert_eq!(summary.path, 1);
    queue.shutdown();
}

#[test]
fn a_relink_job_with_the_drive_unplugged_matches_by_where_it_was_last_mounted() {
    let (lib, music, _) = e_music();
    let file = lib.file(music, "Alpha.mp3", None);
    let track = lib.track(&loc("E:/Music/Alpha.mp3"), None);
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(JobKind::Relink, Relinker::new(|| Plugged(Vec::new())))
        .start()
        .unwrap();
    let id = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    wait_done(&lib.writer, id);
    assert_eq!(lib.matched(track), Some((file, "path".to_owned(), 0.9)));
    queue.shutdown();
}

#[test]
fn asking_for_a_relink_while_one_is_waiting_queues_no_second() {
    let lib = Lib::new();
    let (blocker, started, release) = gate();
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(JobKind::Scan, blocker)
        .handler(JobKind::Relink, Relinker::new(plugged_e))
        .start()
        .unwrap();
    // Keep the only worker busy so the relink waits.
    let scan = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    assert_eq!(started.recv_timeout(PATIENCE).unwrap(), scan);

    let first = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    let again = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    let third = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    assert_eq!((again, third), (first, first));
    assert_eq!(relink_jobs(&lib.writer), [(first.0, "queued".to_owned())]);

    release.send(()).unwrap();
    wait_done(&lib.writer, first);
    assert_eq!(relink_jobs(&lib.writer), [(first.0, "done".to_owned())]);
    queue.shutdown();
}

#[test]
fn asking_for_a_relink_while_one_runs_queues_one_more_to_run_after_it() {
    let lib = Lib::new();
    let runs = Arc::new(AtomicUsize::new(0));
    let (gated, started, release) = gate();
    let counted = runs.clone();
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(JobKind::Relink, move |job: &JobContext| {
            gated(job)?;
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .start()
        .unwrap();
    let running = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    assert_eq!(started.recv_timeout(PATIENCE).unwrap(), running);

    // It may have read the database before this change: one more is due,
    // but only one, however many ask.
    let next = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    let again = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    assert_ne!(next, running);
    assert_eq!(again, next);
    assert_eq!(
        relink_jobs(&lib.writer),
        [
            (running.0, "running".to_owned()),
            (next.0, "queued".to_owned())
        ]
    );

    release.send(()).unwrap();
    wait_done(&lib.writer, running);
    assert_eq!(started.recv_timeout(PATIENCE).unwrap(), next);
    release.send(()).unwrap();
    wait_done(&lib.writer, next);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    queue.shutdown();
}

#[test]
fn two_callers_asking_at_once_queue_one_relink() {
    let lib = Lib::new();
    let (blocker, started, release) = gate();
    let queue = Arc::new(
        JobQueue::builder(lib.writer.clone())
            .workers(1)
            .handler(JobKind::Scan, blocker)
            .handler(JobKind::Relink, Relinker::new(plugged_e))
            .start()
            .unwrap(),
    );
    let scan = queue.enqueue(NewJob::new(JobKind::Scan)).unwrap();
    assert_eq!(started.recv_timeout(PATIENCE).unwrap(), scan);
    let ids: Vec<JobId> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (writer, queue) = (lib.writer.clone(), queue.clone());
                s.spawn(move || request(&writer, |j| queue.enqueue(j)).unwrap())
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(ids.iter().all(|&id| id == ids[0]), "{ids:?}");
    assert_eq!(relink_jobs(&lib.writer).len(), 1);
    release.send(()).unwrap();
    wait_done(&lib.writer, ids[0]);
    queue.shutdown();
}

#[test]
fn reading_a_rekordbox_export_queues_a_relink_that_matches_its_tracks() {
    let (lib, music, _) = e_music();
    let file = lib.file(music, "Alpha One.mp3", Some(200_000));
    let dir = tempfile::tempdir().unwrap();
    let xml = dir.path().join("export.xml");
    std::fs::write(
        &xml,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="2">
    <TRACK TrackID="1" Name="Synthetic 1" TotalTime="200" Location="file://localhost/E:/Music/Alpha%20One.mp3"/>
    <TRACK TrackID="2" Name="Synthetic 2" TotalTime="200" Location="soundcloud:tracks:2"/>
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="0"/>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#,
    )
    .unwrap();
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(
            JobKind::ReadRekordbox,
            crate::rekordbox::source::XmlReader::default(),
        )
        .handler(JobKind::Relink, Relinker::new(plugged_e))
        .start()
        .unwrap();
    let read = queue
        .enqueue(crate::rekordbox::source::read_job(&xml))
        .unwrap();
    wait_done(&lib.writer, read);
    wait_until("a relink ran", || {
        relink_jobs(&lib.writer)
            .iter()
            .any(|(_, s)| s == "done" || s == "failed")
    });
    assert_eq!(relink_jobs(&lib.writer).len(), 1);
    let matches: Vec<Option<i64>> = lib
        .all_matches()
        .into_iter()
        .map(|(_, m)| m.map(|(f, _, _)| f))
        .collect();
    assert_eq!(matches, [Some(file), None]);
    queue.shutdown();
}

/// Attach jobs in the table, by status.
fn attach_jobs(writer: &Writer) -> Vec<String> {
    writer
        .call(|c| {
            c.prepare("SELECT status FROM job WHERE kind = 'attach' ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap()
}

#[test]
fn a_finished_relink_job_queues_an_attach() {
    let (lib, music, _) = e_music();
    let file = lib.file(music, "Alpha.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Alpha.mp3"), Some("200"));
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(JobKind::Relink, Relinker::new(plugged_e))
        .handler(JobKind::Attach, crate::attach::Attacher::default())
        .start()
        .unwrap();
    let id = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    wait_done(&lib.writer, id);
    assert_eq!(lib.matched(track), path(file));
    // Queued by the time the relink job is done; then it runs.
    assert_eq!(attach_jobs(&lib.writer).len(), 1);
    wait_until("the attach job ran", || {
        attach_jobs(&lib.writer) == ["done"]
    });
    queue.shutdown();
}

#[test]
fn a_failure_to_ask_for_an_attach_after_relinking_does_not_fail_the_job() {
    let (lib, music, _) = e_music();
    let file = lib.file(music, "Alpha.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Alpha.mp3"), Some("200"));
    // Queuing an attach job fails.
    lib.writer
        .call(|c| {
            c.execute_batch(
                "CREATE TRIGGER no_attach_jobs BEFORE INSERT ON job WHEN new.kind = 'attach'
                 BEGIN SELECT RAISE(ABORT, 'no attach jobs'); END",
            )
        })
        .unwrap();
    let queue = JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(JobKind::Relink, Relinker::new(plugged_e))
        .start()
        .unwrap();
    let id = request(&lib.writer, |j| queue.enqueue(j)).unwrap();
    // Done, not failed, and the matches it made are kept.
    wait_done(&lib.writer, id);
    assert_eq!(lib.matched(track), path(file));
    assert!(attach_jobs(&lib.writer).is_empty());
    queue.shutdown();
}
