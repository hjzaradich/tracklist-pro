//! Grouping as a background job, and what it leaves on disk.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use super::support::*;
use crate::grouping::{group_job, start, Grouper, Summary};
use crate::jobs::{self, JobId, JobKind, JobQueue, JobRecord, JobStatus};

/// Long enough that a passing test never hits it.
const PATIENCE: Duration = Duration::from_secs(30);

fn wait(db: &Db, id: JobId) -> JobRecord {
    let begun = Instant::now();
    loop {
        let job = db
            .writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            return job;
        }
        assert!(
            begun.elapsed() < PATIENCE,
            "job {id} never finished: {job:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_job_groups_the_files_and_reports_what_it_did() {
    let db = db();
    db.file(1, "a.mp3", Some(A));
    db.file(2, "a copy.mp3", Some(A));
    db.file(1, "b.mp3", None);
    let seen = Arc::new(Mutex::new(Vec::<Summary>::new()));
    let sink = seen.clone();
    let queue = JobQueue::builder(db.writer.clone())
        .handler(
            JobKind::Group,
            Grouper::default().on_summary(move |s| sink.lock().unwrap().push(s)),
        )
        .start()
        .unwrap();
    let id = queue.enqueue(group_job()).unwrap();
    let job = wait(&db, id);
    queue.shutdown();
    assert_eq!(job.status, JobStatus::Done);
    assert_eq!(db.tracks(), 2);
    let seen = seen.lock().unwrap();
    assert_eq!((seen[0].placed, seen[0].recordings_made), (3, 2));
}

#[test]
fn asking_again_while_a_grouping_job_is_waiting_returns_that_job() {
    let db = db();
    let (release, hold) = mpsc::channel::<()>();
    let hold = Mutex::new(hold);
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        // Keeps the only worker busy, so grouping jobs stay queued.
        .handler(JobKind::Scan, move |_: &jobs::JobContext| {
            let _ = hold.lock().unwrap().recv();
            Ok(())
        })
        .handler(JobKind::Group, Grouper::default())
        .start()
        .unwrap();
    let scan = queue.enqueue(jobs::NewJob::new(JobKind::Scan)).unwrap();
    let first = start(&queue).unwrap();
    let second = start(&queue).unwrap();
    assert_eq!(first, second);
    release.send(()).unwrap();
    assert_eq!(wait(&db, first).status, JobStatus::Done);
    wait(&db, scan);
    // Once it has run, asking makes a new one.
    let third = start(&queue).unwrap();
    assert_ne!(third, first);
    assert_eq!(wait(&db, third).status, JobStatus::Done);
    queue.shutdown();
}

#[test]
fn asking_while_a_grouping_job_runs_queues_another_that_runs_after_it() {
    let db = db();
    let (release, hold) = mpsc::channel::<()>();
    let hold = Mutex::new(hold);
    let runs = Arc::new(Mutex::new(0u32));
    let counted = runs.clone();
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        .handler(JobKind::Group, move |_: &jobs::JobContext| {
            *counted.lock().unwrap() += 1;
            // The first run waits to be let go; the second finds it open.
            let _ = hold.lock().unwrap().recv_timeout(Duration::from_secs(20));
            Ok(())
        })
        .start()
        .unwrap();
    let first = start(&queue).unwrap();
    let begun = Instant::now();
    while *runs.lock().unwrap() == 0 {
        assert!(begun.elapsed() < PATIENCE, "the first job never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Files may have changed since the running job loaded them: another
    // pass is needed, not the running one.
    let second = start(&queue).unwrap();
    assert_ne!(first, second);
    release.send(()).unwrap();
    assert_eq!(wait(&db, first).status, JobStatus::Done);
    drop(release);
    assert_eq!(wait(&db, second).status, JobStatus::Done);
    assert_eq!(*runs.lock().unwrap(), 2);
    queue.shutdown();
}

// ---- nothing outside the app data folder ----------------------------------

/// Every file and folder under `dir`, with its bytes and modified time.
fn snapshot(dir: &Path, skip: &Path, out: &mut BTreeMap<PathBuf, (Vec<u8>, SystemTime)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path == skip {
            continue;
        }
        let meta = fs::symlink_metadata(&path).unwrap();
        let bytes = if meta.is_dir() {
            snapshot(&path, skip, out);
            Vec::new()
        } else {
            fs::read(&path).unwrap()
        };
        out.insert(path, (bytes, meta.modified().unwrap()));
    }
}

#[test]
fn grouping_writes_nothing_outside_the_app_data_folder() {
    let sandbox = tempfile::tempdir().unwrap();
    let sandbox = fs::canonicalize(sandbox.path()).unwrap();
    let data = sandbox.join("data");
    let music = sandbox.join("music");
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&music).unwrap();
    let db = db_in(&data);
    // Real files on disk, so a stray write to one would show.
    for (name, hash) in [("a.mp3", Some(A)), ("b.mp3", Some(A)), ("c.mp3", None)] {
        fs::write(music.join(name), name.as_bytes()).unwrap();
        db.file(1, name, hash);
    }
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    for entry in fs::read_dir(&music).unwrap() {
        fs::File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let mut before = BTreeMap::new();
    snapshot(&sandbox, &data, &mut before);

    // Group, change a file's audio, group again, and remove the folder.
    let queue = JobQueue::builder(db.writer.clone())
        .handler(JobKind::Group, Grouper::default())
        .start()
        .unwrap();
    let id = queue.enqueue(group_job()).unwrap();
    assert_eq!(wait(&db, id).status, JobStatus::Done);
    db.set_hash(2, Some(B));
    let id = queue.enqueue(group_job()).unwrap();
    assert_eq!(wait(&db, id).status, JobStatus::Done);
    queue.shutdown();
    let removed = db
        .writer
        .call(|c| crate::scan::folders::remove(c, crate::scan::folders::MusicFolderId(1)))
        .unwrap();
    assert_eq!(removed, Ok(()));

    let mut after = BTreeMap::new();
    snapshot(&sandbox, &data, &mut after);
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .collect();
    assert!(
        changed.is_empty(),
        "changed outside the data folder: {changed:?}"
    );
    assert_eq!(after.len(), 4, "the music folder and its three files");
}
