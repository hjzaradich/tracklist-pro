//! Grouping as a background job, and what it leaves on disk.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use super::support::*;
use crate::grouping::{decide_job, group_job, start, Decided, Grouper, Summary, DECIDE_PRIORITY};
use crate::jobs::{self, JobId, JobKind, JobQueue, JobRecord, JobStatus, Priority};
use crate::matching::{matching_job, Matcher};
use crate::scan::chain::{after_group, after_matching};

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
fn a_failure_to_ask_for_an_attach_after_grouping_does_not_fail_the_job() {
    let db = db();
    db.file(1, "a.mp3", Some(A));
    // Rekordbox data in the library, so an attach is asked for.
    db.sql(
        r#"INSERT INTO rekordbox_track (attributes, location_key, read_at)
           VALUES ('{"TrackID":"1","Location":"file://localhost/C:/Kit/1.mp3"}',
                   'c:/kit/1.mp3', '2026-01-01T00:00:00.000Z')"#,
    );
    // Queuing an attach job fails.
    db.sql(
        "CREATE TRIGGER no_attach_jobs BEFORE INSERT ON job WHEN new.kind = 'attach'
         BEGIN SELECT RAISE(ABORT, 'the queue is closing'); END",
    );
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
    assert_eq!(job.status, JobStatus::Done, "{job:?}");
    assert_eq!(db.tracks(), 1);
    assert_eq!(seen.lock().unwrap().len(), 1, "the summary was still heard");
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
    let first = start(&queue, &db.writer).unwrap();
    let second = start(&queue, &db.writer).unwrap();
    assert_eq!(first, second);
    release.send(()).unwrap();
    assert_eq!(wait(&db, first).status, JobStatus::Done);
    wait(&db, scan);
    // Once it has run, asking makes a new one.
    let third = start(&queue, &db.writer).unwrap();
    assert_ne!(third, first);
    assert_eq!(wait(&db, third).status, JobStatus::Done);
    queue.shutdown();
}

#[test]
fn asking_while_a_grouping_job_runs_makes_it_run_once_more_after_it_ends() {
    let db = db();
    let (release, hold) = mpsc::channel::<()>();
    let hold = Mutex::new(hold);
    let runs = Arc::new(Mutex::new(0u32));
    let counted = runs.clone();
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        .handler(
            JobKind::Group,
            after_group(move |_: &jobs::JobContext| {
                *counted.lock().unwrap() += 1;
                // The first run waits to be let go; the second finds it open.
                let _ = hold.lock().unwrap().recv_timeout(Duration::from_secs(20));
                Ok(())
            }),
        )
        .start()
        .unwrap();
    let first = start(&queue, &db.writer).unwrap();
    let begun = Instant::now();
    while *runs.lock().unwrap() == 0 {
        assert!(begun.elapsed() < PATIENCE, "the first job never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Files may have changed since the running job loaded them: another
    // pass is needed, not the running one.
    let second = start(&queue, &db.writer).unwrap();
    assert_eq!(first, second, "asked, not parked beside it");
    release.send(()).unwrap();
    assert_eq!(wait(&db, first).status, JobStatus::Done);
    drop(release);
    // Its wrapper queued the rerun before it finished.
    let begun = Instant::now();
    while *runs.lock().unwrap() < 2 || !queue.activity().unwrap().jobs.is_empty() {
        assert!(begun.elapsed() < PATIENCE, "the rerun never ran");
        std::thread::sleep(Duration::from_millis(5));
    }
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

// ---- the deciding job -------------------------------------------------------

/// Waits until no job is queued or running: a chained job queues what
/// follows it before it ends.
fn wait_idle(queue: &JobQueue) {
    let begun = Instant::now();
    while !queue.activity().unwrap().jobs.is_empty() {
        assert!(begun.elapsed() < PATIENCE, "the jobs never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn song<'a>(name: &'a str, title: &'a str, n: u8, items: Option<Vec<u32>>) -> NewFile<'a> {
    NewFile {
        name,
        title: Some(title),
        artist: Some("Halden Rook"),
        hash: Some(hash(n)),
        items,
        blake3: None,
    }
}

#[test]
fn the_quick_job_gives_every_file_a_track_and_decides_no_pair() {
    let db = db();
    let velo = track(1, 900);
    db.add(song("Velo.flac", "Velo", 1, Some(velo.clone())));
    db.add(song("Velo.mp3", "Velo", 2, Some(reencoded(&velo, 1, 7))));
    let queue = JobQueue::builder(db.writer.clone())
        .handler(JobKind::Group, after_group(Grouper::default()))
        .start()
        .unwrap();
    let id = queue.enqueue(group_job()).unwrap();
    assert_eq!(wait(&db, id).status, JobStatus::Done);
    wait_idle(&queue);
    queue.shutdown();
    assert_eq!(db.tracks(), 2, "other audio hashes: two tracks for now");
    assert!(db.decided().is_empty());
    assert_eq!(db.count("SELECT count(*) FROM grouping_pair"), 0);
    assert_eq!(
        db.count("SELECT count(*) FROM job WHERE kind = 'group'"),
        1,
        "no deciding job follows it"
    );
}

#[test]
fn once_matching_has_been_through_the_deciding_job_follows_it_and_merges_the_duplicates() {
    let db = db();
    let velo = track(1, 900);
    let a = db.add(song("Velo.flac", "Velo", 1, Some(velo.clone())));
    let b = db.add(song("Velo.mp3", "Velo", 2, Some(reencoded(&velo, 1, 7))));
    let heard = Arc::new(Mutex::new(Vec::<(Decided, Summary)>::new()));
    let sink = heard.clone();
    let queue = JobQueue::builder(db.writer.clone())
        .handler(
            JobKind::Group,
            after_group(
                Grouper::default().on_decided(move |d, s| sink.lock().unwrap().push((d, s))),
            ),
        )
        .handler(JobKind::Match, after_matching(Matcher::new()))
        .start()
        .unwrap();
    // The fingerprints are in: the chain asks for matching, and nothing
    // else has to be asked for.
    let id = queue.enqueue(matching_job()).unwrap();
    assert_eq!(wait(&db, id).status, JobStatus::Done);
    wait_idle(&queue);
    queue.shutdown();
    assert_eq!(db.track(a), db.track(b));
    assert_eq!(db.tracks(), 1);
    assert_eq!(db.decided(), [a, b]);
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 1, "one deciding job");
    assert_eq!((heard[0].0.files, heard[0].1.moved), (2, 1));
    // It ran at matching's priority, below the chain's own work.
    assert_eq!(
        db.count("SELECT priority FROM job WHERE kind = 'group' AND target IS NOT NULL"),
        DECIDE_PRIORITY.0
    );
    assert!(DECIDE_PRIORITY < Priority::BACKGROUND);
}

#[test]
fn a_deciding_job_is_not_asked_for_while_matching_still_has_files_to_compare() {
    let db = db();
    db.add(song("Velo.flac", "Velo", 1, Some(track(1, 900))));
    db.group();
    let ask = |db: &Db| {
        let mut asked = 0;
        crate::grouping::request_decide(&db.writer, |_| -> Result<JobId, crate::db::DbError> {
            asked += 1;
            Ok(JobId(99))
        })
        .unwrap();
        asked
    };
    assert_eq!(ask(&db), 0, "its fingerprint is still to be compared");
    db.compare_fingerprints();
    assert_eq!(ask(&db), 1);
    // Decided: nothing to ask for.
    db.decide();
    assert_eq!(ask(&db), 0);
}

#[test]
fn the_deciding_job_makes_way_for_a_job_of_a_higher_priority_and_goes_on_afterwards() {
    let db = db();
    for n in 1..=6u8 {
        let (name, title) = (format!("{n}.mp3"), format!("Song {n}"));
        db.add(song(&name, &title, n, None));
    }
    let asked = Arc::new(Mutex::new(false));
    let writer = db.writer.clone();
    // What the higher job found when it ran: how many files were decided.
    let found = Arc::new(Mutex::new(Vec::<i64>::new()));
    let (sink, reader) = (found.clone(), db.writer.clone());
    let heard = Arc::new(Mutex::new(Vec::<Decided>::new()));
    let decided = heard.clone();
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        .handler(
            JobKind::Group,
            after_group(
                Grouper::default()
                    .page(1)
                    // As the run starts, a job the user asked for arrives.
                    .on_tick(move || {
                        let mut asked = asked.lock().unwrap();
                        if !*asked {
                            *asked = true;
                            writer
                                .call(|c| {
                                    c.execute(
                                        "INSERT INTO job (kind, priority, status)
                                         VALUES ('scan', 10, 'queued')",
                                        [],
                                    )
                                })
                                .unwrap();
                        }
                    })
                    .on_decided(move |d, _| decided.lock().unwrap().push(d)),
            ),
        )
        .handler(JobKind::Scan, move |_: &jobs::JobContext| {
            let stamp = crate::grouping::decided_as();
            let so_far = reader.call(move |c| {
                c.query_row(
                    "SELECT count(*) FROM grouping_file WHERE decided_as IS ?1",
                    [stamp],
                    |r| r.get(0),
                )
            })?;
            sink.lock().unwrap().push(so_far);
            Ok(())
        })
        .start()
        .unwrap();
    queue.enqueue(decide_job()).unwrap();
    let begun = Instant::now();
    while db.decided().len() < 6 || !queue.activity().unwrap().jobs.is_empty() {
        assert!(begun.elapsed() < PATIENCE, "the deciding never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    queue.shutdown();
    // It stopped after its first page, the other job ran, and the run
    // queued after it decided the rest.
    assert_eq!(*found.lock().unwrap(), [1]);
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 1, "one run got to the end");
    assert_eq!(heard[0].files, 5, "and decided what the first left");
    // The run queued after it is a deciding job like the first, at the
    // deciding job's priority: it makes way in its turn.
    let group_jobs: Vec<(Option<String>, i64)> = db
        .writer
        .call(|c| {
            let mut s =
                c.prepare("SELECT target, priority FROM job WHERE kind = 'group' ORDER BY id")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap();
    assert_eq!(group_jobs.len(), 2);
    for (target, priority) in group_jobs {
        assert!(target.is_some_and(|t| t.contains("decide")));
        assert_eq!(priority, DECIDE_PRIORITY.0);
    }
}

#[test]
fn the_quick_job_asks_for_a_deciding_job_when_a_set_waits_for_a_pair_nobody_decided() {
    // 1-2 and 2-3 are proposed and duplicates; nothing leads from 1 to 3
    // (other names, not an item in common bit for bit).
    let db = db();
    let one = track(1, 900);
    let two = reencoded(&one, 1, 7);
    let three: Vec<u32> = one
        .iter()
        .zip(&two)
        .map(|(a, b)| if a == b { a ^ 0b101 } else { *b })
        .collect();
    let a = db.add(song("Velo.flac", "Velo", 1, Some(one)));
    let b = db.add(song("Velo.mp3", "Velo", 2, Some(two)));
    let c = db.add(song("Nightdrive.mp3", "Nightdrive", 3, Some(three)));
    db.group();
    db.compare_fingerprints();
    // A deciding job that was cut short after its pages: every file is
    // decided, the pair of 1 and 3 isn't.
    db.decide_in_pages(10);
    assert_eq!(db.decided(), [a, b, c]);
    assert_eq!(db.pair(a, c), None);
    assert_eq!(db.tracks(), 3);
    let queue = JobQueue::builder(db.writer.clone())
        .handler(JobKind::Group, after_group(Grouper::default()))
        .start()
        .unwrap();
    let id = queue.enqueue(group_job()).unwrap();
    assert_eq!(wait(&db, id).status, JobStatus::Done);
    wait_idle(&queue);
    queue.shutdown();
    assert_eq!(
        db.count("SELECT count(*) FROM job WHERE kind = 'group' AND target IS NOT NULL"),
        1,
        "the quick job asked for a deciding job"
    );
    assert_eq!(db.stands(a, c), ("duplicate".into(), "merged".into()));
    assert_eq!(db.tracks(), 1);
}
