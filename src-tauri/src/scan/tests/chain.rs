#![cfg(test)]
//! 1aC-8: after a walk, the read of its folders, then their hashes, then
//! the fingerprints, for exactly the new or changed files; no duplicate
//! jobs; no loop while online-only files stay due.

use std::collections::HashMap;
use std::fs;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use super::support::{db, TempVolume};
use super::walk::{add_music, at, drive, put};
use crate::db::Writer;
use crate::fingerprint::{Fingerprinter, FirstUp};
use crate::hash::{Hasher, Summary};
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, JobQueue, NewJob, Priority};
use crate::read::{read_job, Reader};
use crate::scan::chain::{after_hash, after_read, after_walk, unless_queued};
use crate::scan::folders::MusicFolderId;
use crate::scan::walk::{scan_job, Walker};
use crate::scan_state::{self, Scope, Stage};
use crate::tags::test_audio as audio;

/// How many files each stage looked at, over every job so far.
#[derive(Clone, Default)]
pub(super) struct LookedAt {
    pub read: Arc<AtomicU64>,
    pub hashed: Arc<AtomicU64>,
    pub fingerprinted: Arc<AtomicU64>,
}

impl LookedAt {
    fn counts(&self) -> (u64, u64, u64) {
        (
            self.read.load(Ordering::SeqCst),
            self.hashed.load(Ordering::SeqCst),
            self.fingerprinted.load(Ordering::SeqCst),
        )
    }
}

/// A queue with every stage's handler, chained as the app chains them.
pub(super) fn chained_queue(writer: &Writer, volume: &TempVolume, looked: &LookedAt) -> JobQueue {
    let (v1, v2, v3, v4) = (
        volume.clone(),
        volume.clone(),
        volume.clone(),
        volume.clone(),
    );
    let (read, hashed, fingerprinted) = (
        looked.read.clone(),
        looked.hashed.clone(),
        looked.fingerprinted.clone(),
    );
    JobQueue::builder(writer.clone())
        .workers(2)
        .handler(
            JobKind::Scan,
            after_walk(Walker::new(move || v1.clone(), |_| {})),
        )
        .handler(
            JobKind::Read,
            after_read(Reader::new(move || v2.clone()).on_file(move |_| {
                read.fetch_add(1, Ordering::SeqCst);
            })),
        )
        .handler(
            JobKind::Hash,
            after_hash(
                Hasher::new(move || v3.clone()).on_summary(move |s: Summary| {
                    // Every file the job looked at, whatever became of it.
                    let looked_at = s.hashed
                        + s.no_audio_hash
                        + s.changed_since_walk
                        + s.unreachable
                        + s.online_only
                        + s.offline;
                    hashed.fetch_add(looked_at, Ordering::SeqCst);
                }),
            ),
        )
        .handler(
            JobKind::Fingerprint,
            Fingerprinter::new(move || v4.clone(), FirstUp::default())
                .threads(1)
                .on_file(move |_| {
                    fingerprinted.fetch_add(1, Ordering::SeqCst);
                }),
        )
        .start()
        .unwrap()
}

/// Waits until no job is queued or running. A chained job queues its
/// follow-up before it finishes, so an empty list means the chain ended.
pub(super) fn wait_idle(queue: &JobQueue) {
    let start = Instant::now();
    loop {
        if queue.activity().unwrap().jobs.is_empty() {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "the jobs never finished"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Every job so far: kind, target, priority and status, oldest first.
pub(super) fn jobs(writer: &Writer) -> Vec<(String, Option<String>, i64, String)> {
    writer
        .call(|c| {
            let mut s = c.prepare("SELECT kind, target, priority, status FROM job ORDER BY id")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })
        .unwrap()
}

/// When each job started and finished, oldest first.
fn spans(writer: &Writer) -> Vec<(String, String)> {
    writer
        .call(|c| {
            let mut s = c.prepare("SELECT started_at, finished_at FROM job ORDER BY id")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap()
}

fn kinds(writer: &Writer) -> Vec<String> {
    jobs(writer).into_iter().map(|j| j.0).collect()
}

/// A file's mtime moved on, as rekordbox does when it rewrites tags.
fn touch_later(path: &Path) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
}

#[test]
fn a_finished_walk_leads_to_read_then_hash_and_fingerprint_for_exactly_the_new_or_changed_files() {
    let (_dir, volume, music) = drive();
    put(&music, "House/a.mp3", &audio::mp3());
    put(&music, "House/b.wav", &audio::wav());
    let (_db, writer, _reads) = db();
    let folder = add_music(&writer, &volume, &music);
    let looked = LookedAt::default();
    let queue = chained_queue(&writer, &volume, &looked);

    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    let background = Priority::BACKGROUND.0;
    assert_eq!(
        jobs(&writer),
        [
            ("scan".into(), None, Priority::USER.0, "done".into()),
            ("read".into(), None, background, "done".into()),
            ("hash".into(), None, background, "done".into()),
            ("fingerprint".into(), None, background, "done".into()),
        ]
    );
    assert_eq!(looked.counts(), (2, 2, 2));
    // One stage at a time, even with two workers: the read ended before
    // the hashes began, and the hashes before the fingerprints.
    let spans = spans(&writer);
    assert!(
        spans[1].1 <= spans[2].0,
        "hash began before the read ended: {spans:?}"
    );
    assert!(
        spans[2].1 <= spans[3].0,
        "fingerprint began before the hash ended: {spans:?}"
    );
    // Every stage has a row for every file: nothing is left due.
    let due = |stage, version| {
        writer
            .call(move |c| scan_state::count_due(c, stage, version, &Scope::All))
            .unwrap()
    };
    assert_eq!(due(Stage::Read, crate::read::READ_VERSION), 0);
    assert_eq!(due(Stage::Hash, i64::from(crate::hash::DEFINITION)), 0);
    assert_eq!(
        due(Stage::Fingerprint, i64::from(crate::fingerprint::VERSION)),
        0
    );

    // One new file, one rewritten: the next walk of that folder leads to
    // exactly those two in every stage.
    put(&music, "House/c.mp3", &audio::mp3());
    touch_later(&at(&music, "House/b.wav"));
    queue.enqueue(scan_job(Some(vec![folder]))).unwrap();
    wait_idle(&queue);
    let target = Some(format!(r#"{{"music_folder_ids":[{}]}}"#, folder.0));
    assert_eq!(
        jobs(&writer)[4..],
        [
            (
                "scan".into(),
                target.clone(),
                Priority::USER.0,
                "done".into()
            ),
            ("read".into(), target.clone(), background, "done".into()),
            ("hash".into(), target, background, "done".into()),
            ("fingerprint".into(), None, background, "done".into()),
        ]
    );
    assert_eq!(looked.counts(), (4, 4, 4));
    queue.shutdown();
}

#[test]
fn a_walk_that_finds_nothing_new_or_changed_queues_no_stage_job() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &audio::mp3());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    let looked = LookedAt::default();
    let queue = chained_queue(&writer, &volume, &looked);
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(kinds(&writer), ["scan", "read", "hash", "fingerprint"]);

    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(
        kinds(&writer),
        ["scan", "read", "hash", "fingerprint", "scan"]
    );
    assert_eq!(looked.counts(), (1, 1, 1));
    queue.shutdown();
}

#[test]
fn a_walk_that_stops_early_queues_nothing() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &audio::mp3());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    // A walk that fails after finding the file: its rows are there, so a
    // read would have work, but a stopped walk never asks for one.
    let inner = Walker::new(move || volume.clone(), |_| {});
    let failing = move |job: &JobContext| {
        inner.run(job)?;
        Err(JobError::failed("the drive went away"))
    };
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, after_walk(failing))
        .start()
        .unwrap();
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(
        jobs(&writer),
        [("scan".into(), None, Priority::USER.0, "failed".into())]
    );
    let files: i64 = writer
        .call(|c| c.query_row("SELECT count(*) FROM file", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(files, 1, "the walk's own rows are kept");
    queue.shutdown();
}

#[test]
fn an_identical_job_still_waiting_is_not_queued_twice() {
    let (_db, writer, _reads) = db();
    // No workers: whatever is queued stays queued.
    let queue = JobQueue::builder(writer.clone()).start().unwrap();
    queue.shutdown();
    let enqueue = |job: NewJob| queue.enqueue(job);

    let first = unless_queued(&writer, read_job(None), enqueue).unwrap();
    let again = unless_queued(&writer, read_job(None), enqueue).unwrap();
    assert_eq!(again, first);
    let other = unless_queued(&writer, read_job(Some(vec![MusicFolderId(3)])), enqueue).unwrap();
    assert_ne!(other, first);
    let same_target = unless_queued(&writer, read_job(Some(vec![MusicFolderId(3)])), enqueue);
    assert_eq!(same_target.unwrap(), other);
    // A different kind with the same target is its own job.
    let hash = unless_queued(
        &writer,
        crate::hash::hash_job(Some(vec![MusicFolderId(3)])),
        enqueue,
    )
    .unwrap();
    assert!(hash != other && hash != first);
    assert_eq!(kinds(&writer), ["read", "read", "hash"]);

    // Once it's no longer waiting (cancelled here; done or running in the
    // app), the same job can be queued again.
    queue.cancel(first).unwrap();
    let after = unless_queued(&writer, read_job(None), enqueue).unwrap();
    assert_ne!(after, first);
    assert_eq!(
        jobs(&writer)
            .iter()
            .map(|j| j.3.as_str())
            .collect::<Vec<_>>(),
        ["cancelled", "queued", "queued", "queued"]
    );
}

/// Marks `path` offline (`FILE_ATTRIBUTE_OFFLINE`), as a OneDrive
/// online-only placeholder is marked: the walk records it online only,
/// and no stage reads it without the opt-in.
fn mark_offline(path: &Path) {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_OFFLINE, INVALID_FILE_ATTRIBUTES,
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    unsafe {
        let attributes = GetFileAttributesW(wide.as_ptr());
        assert_ne!(attributes, INVALID_FILE_ATTRIBUTES);
        assert_ne!(
            SetFileAttributesW(wide.as_ptr(), attributes | FILE_ATTRIBUTE_OFFLINE),
            0
        );
    }
}

#[test]
fn online_only_files_that_stay_due_do_not_keep_the_chain_going() {
    let (_dir, volume, music) = drive();
    put(&music, "cloud.mp3", &audio::mp3());
    put(&music, "local.mp3", &audio::mp3());
    mark_offline(&at(&music, "cloud.mp3"));
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    let looked = LookedAt::default();
    let queue = chained_queue(&writer, &volume, &looked);

    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    // The local file went through every stage; the placeholder was skipped
    // by each and is still due for each (count_due never reaches 0).
    assert_eq!(kinds(&writer), ["scan", "read", "hash", "fingerprint"]);
    let due = writer
        .call(|c| scan_state::count_due(c, Stage::Read, crate::read::READ_VERSION, &Scope::All))
        .unwrap();
    assert_eq!(due, 1);
    let stages: HashMap<(String, String), String> = writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT f.rel_path, s.stage, s.status FROM file_stage s
                 JOIN file f ON f.id = s.file_id",
            )?;
            let rows = s.query_map([], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?;
            rows.collect()
        })
        .unwrap();
    for stage in ["read", "hash", "fingerprint"] {
        assert_eq!(
            stages[&("cloud.mp3".to_owned(), stage.to_owned())],
            "skipped"
        );
        assert_ne!(
            stages[&("local.mp3".to_owned(), stage.to_owned())],
            "skipped"
        );
    }

    // Another walk finds nothing a stage may try: no stage job follows,
    // however often it runs.
    for _ in 0..2 {
        queue.enqueue(scan_job(None)).unwrap();
        wait_idle(&queue);
    }
    assert_eq!(
        kinds(&writer),
        ["scan", "read", "hash", "fingerprint", "scan", "scan"]
    );
    assert_eq!(
        looked.counts(),
        (2, 2, 2),
        "the placeholder was looked at once per stage"
    );

    // With the opt-in, the placeholder is fair game again.
    writer
        .call(|c| crate::scan::online_only::write_opt_in(c, true))
        .unwrap();
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(kinds(&writer)[6..], ["scan", "read", "hash", "fingerprint"]);
    queue.shutdown();
}

#[test]
fn chained_stage_jobs_run_after_anything_the_user_is_waiting_on() {
    // The chain queues at background priority: a job the user asked for
    // while the walk ran is taken before the walk's read.
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &audio::mp3());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    // Each handler notes when it runs.
    let ran: Arc<Mutex<Vec<JobKind>>> = Arc::default();
    let note = |kind: JobKind| {
        let ran = ran.clone();
        move || ran.lock().unwrap().push(kind)
    };
    let (scan_ran, export_ran, read_ran) = (
        note(JobKind::Scan),
        note(JobKind::Export),
        note(JobKind::Read),
    );
    // The walk, held at its end until the test says go.
    let (release, held) = mpsc::channel::<()>();
    let held = Mutex::new(held);
    let walker = Walker::new(move || volume.clone(), |_| {});
    let holding_walk = move |job: &JobContext| {
        scan_ran();
        walker.run(job)?;
        held.lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| JobError::failed("never released"))
    };
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, after_walk(holding_walk))
        .handler(JobKind::Read, move |_: &JobContext| {
            read_ran();
            Ok(())
        })
        .handler(JobKind::Export, move |_: &JobContext| {
            export_ran();
            Ok(())
        })
        .start()
        .unwrap();
    queue.enqueue(scan_job(None)).unwrap();
    queue
        .enqueue(NewJob::new(JobKind::Export).priority(Priority::USER))
        .unwrap();
    release.send(()).unwrap();
    wait_idle(&queue);
    queue.shutdown();
    assert_eq!(kinds(&writer), ["scan", "export", "read"]);
    let ran = ran.lock().unwrap().clone();
    assert_eq!(ran, [JobKind::Scan, JobKind::Export, JobKind::Read]);
}
