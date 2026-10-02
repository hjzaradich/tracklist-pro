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
use std::time::{Duration, Instant};

use super::support::{db, TempVolume};
use super::walk::{add_music, at, drive, put};
use crate::db::Writer;
use crate::fingerprint::{Fingerprinter, FirstUp};
use crate::hash::{Hasher, Summary};
use crate::jobs::{
    JobContext, JobError, JobHandler, JobKind, JobQueue, JobQueueBuilder, NewJob, Priority,
};
use crate::read::{read_job, Reader};
use crate::relink::Relinker;
use crate::scan::chain::{
    after_fingerprint, after_group, after_hash, after_quality, after_read, after_walk, queue_once,
};
use crate::scan::folders::MusicFolderId;
use crate::scan::walk::{scan_job, Walker};
use crate::scan_state::{self, Scope, Stage};
use crate::tags::test_audio as audio;

/// How many files each stage looked at, over every job so far, and when
/// each stage's handler was last at work.
#[derive(Clone, Default)]
pub(super) struct LookedAt {
    pub read: Arc<AtomicU64>,
    pub hashed: Arc<AtomicU64>,
    pub fingerprinted: Arc<AtomicU64>,
    pub marks: Arc<Mutex<Marks>>,
    /// Each grouping and relink run, in the order they ended: a grouping
    /// run when its summary is out (before it asks for anything), a relink
    /// run when it's done.
    pub runs: Arc<Mutex<Vec<&'static str>>>,
    /// Holds the first fingerprint job at its first file until released.
    pub hold_fingerprint: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
}

/// When each stage's handler was seen at work (from its hooks).
#[derive(Debug, Default)]
pub(super) struct Marks {
    /// The last file the read job started on.
    pub last_read: Option<Instant>,
    /// The first buffer the hash job read.
    pub first_hash: Option<Instant>,
    /// The hash job's summary: its end.
    pub hash_end: Option<Instant>,
    /// The end of the grouping job.
    pub group_end: Option<Instant>,
    /// The first file the fingerprint job took.
    pub first_fingerprint: Option<Instant>,
}

impl LookedAt {
    fn counts(&self) -> (u64, u64, u64) {
        (
            self.read.load(Ordering::SeqCst),
            self.hashed.load(Ordering::SeqCst),
            self.fingerprinted.load(Ordering::SeqCst),
        )
    }

    /// Holds the next fingerprint job at its first file; send on the
    /// returned sender to let it go.
    pub(super) fn hold_next_fingerprint(&self) -> mpsc::Sender<()> {
        let (release, held) = mpsc::channel();
        *self.hold_fingerprint.lock().unwrap() = Some(held);
        release
    }
}

/// A queue with every stage's handler, chained as the app chains them.
pub(super) fn chained_queue(writer: &Writer, volume: &TempVolume, looked: &LookedAt) -> JobQueue {
    chained_queue_with(writer, volume, looked, 2, |b| b)
}

/// [`chained_queue`] with `workers` workers and `more` handlers.
pub(super) fn chained_queue_with(
    writer: &Writer,
    volume: &TempVolume,
    looked: &LookedAt,
    workers: usize,
    more: impl FnOnce(JobQueueBuilder) -> JobQueueBuilder,
) -> JobQueue {
    let (v1, v2, v3, v4, v5, v6) = (
        volume.clone(),
        volume.clone(),
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
    let (m1, m2, m3, m4, m5) = (
        looked.marks.clone(),
        looked.marks.clone(),
        looked.marks.clone(),
        looked.marks.clone(),
        looked.marks.clone(),
    );
    let hold = looked.hold_fingerprint.clone();
    let (group_runs, relink_runs) = (looked.runs.clone(), looked.runs.clone());
    let builder = JobQueue::builder(writer.clone())
        .workers(workers)
        .handler(
            JobKind::Group,
            after_group(crate::grouping::Grouper::default().on_summary(move |_| {
                m5.lock().unwrap().group_end = Some(Instant::now());
                group_runs.lock().unwrap().push("group");
            })),
        )
        .handler(
            JobKind::Scan,
            after_walk(Walker::new(move || v1.clone(), |_| {})),
        )
        .handler(
            JobKind::Read,
            after_read(Reader::new(move || v2.clone()).on_file(move |_| {
                read.fetch_add(1, Ordering::SeqCst);
                m1.lock().unwrap().last_read = Some(Instant::now());
            })),
        )
        .handler(
            JobKind::Hash,
            after_hash(
                Hasher::new(move || v3.clone())
                    .on_read(move |_, _| {
                        m2.lock()
                            .unwrap()
                            .first_hash
                            .get_or_insert_with(Instant::now);
                    })
                    .on_summary(move |s: Summary| {
                        // Every file the job looked at, whatever became of it.
                        let looked_at = s.hashed
                            + s.no_audio_hash
                            + s.changed_since_walk
                            + s.unreachable
                            + s.online_only
                            + s.offline;
                        hashed.fetch_add(looked_at, Ordering::SeqCst);
                        m3.lock().unwrap().hash_end = Some(Instant::now());
                    }),
            ),
        )
        .handler(
            JobKind::Fingerprint,
            after_fingerprint(
                Fingerprinter::new(move || v4.clone(), FirstUp::default())
                    .threads(1)
                    .on_file(move |_| {
                        m4.lock()
                            .unwrap()
                            .first_fingerprint
                            .get_or_insert_with(Instant::now);
                        // Held only if a test asked, and only once.
                        let held = hold.lock().unwrap().take();
                        if let Some(held) = held {
                            let _ = held.recv_timeout(Duration::from_secs(30));
                        }
                        fingerprinted.fetch_add(1, Ordering::SeqCst);
                    }),
            ),
        )
        .handler(
            JobKind::Relink,
            Relinker::new(move || v5.clone()).on_summary(move |_| {
                relink_runs.lock().unwrap().push("relink");
            }),
        )
        .handler(
            JobKind::Quality,
            after_quality(
                crate::quality::Qualifier::new(move || v6.clone(), FirstUp::default()).threads(1),
            ),
        );
    more(builder).start().unwrap()
}

pub(super) fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "timed out waiting until {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
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

/// Every scan-stage job so far: kind, target, priority and status, oldest
/// first. Relink jobs (queued after each read) are left out; see
/// [`relinks`].
pub(super) fn jobs(writer: &Writer) -> Vec<(String, Option<String>, i64, String)> {
    writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT kind, target, priority, status FROM job
                 WHERE kind <> 'relink' ORDER BY id",
            )?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })
        .unwrap()
}

/// Every relink job so far: its status and the job id that came just
/// before it (the stage that asked), oldest first.
fn relinks(writer: &Writer) -> Vec<(String, String)> {
    writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT r.status, (SELECT kind FROM job p WHERE p.id < r.id ORDER BY p.id DESC)
                 FROM job r WHERE r.kind = 'relink' ORDER BY r.id",
            )?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap()
}

fn kinds(writer: &Writer) -> Vec<String> {
    jobs(writer).into_iter().map(|j| j.0).collect()
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
            ("group".into(), None, background, "done".into()),
            ("fingerprint".into(), None, background, "done".into()),
            ("quality".into(), None, background, "done".into()),
        ]
    );
    assert_eq!(looked.counts(), (2, 2, 2));
    // One stage at a time, even with two workers: the hashes began after
    // the read's last file, and the fingerprints after the hashes ended
    // (from the handlers' own hooks; the job table's timestamps are
    // written a moment after the fact and would race).
    let marks = looked.marks.lock().unwrap();
    assert!(
        marks.last_read.unwrap() <= marks.first_hash.unwrap(),
        "hashing began before the read's last file: {marks:?}"
    );
    assert!(
        marks.hash_end.unwrap() <= marks.first_fingerprint.unwrap(),
        "fingerprinting began before the hashes ended: {marks:?}"
    );
    drop(marks);
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

    // One new file, one rewritten with other bytes: the next walk of that
    // folder leads to exactly those two in every stage. (A touched mtime
    // alone no longer counts: the walk's unchanged check, 1aC-1.)
    put(&music, "House/c.mp3", &audio::mp3());
    put(
        &music,
        "House/b.wav",
        &[audio::wav(), vec![0u8; 64]].concat(),
    );
    queue.enqueue(scan_job(Some(vec![folder]))).unwrap();
    wait_idle(&queue);
    let target = Some(format!(r#"{{"music_folder_ids":[{}]}}"#, folder.0));
    assert_eq!(
        jobs(&writer)[6..],
        [
            (
                "scan".into(),
                target.clone(),
                Priority::USER.0,
                "done".into()
            ),
            ("read".into(), target.clone(), background, "done".into()),
            ("hash".into(), target, background, "done".into()),
            ("group".into(), None, background, "done".into()),
            ("fingerprint".into(), None, background, "done".into()),
            ("quality".into(), None, background, "done".into()),
        ]
    );
    assert_eq!(looked.counts(), (4, 4, 4));
    queue.shutdown();
}

#[test]
fn files_are_grouped_once_the_hashes_are_in_so_the_same_audio_is_one_track() {
    let (_dir, volume, music) = drive();
    // The same audio twice, and different audio.
    put(&music, "House/a.mp3", &audio::mp3());
    put(&music, "House/a copy.mp3", &audio::mp3());
    put(&music, "House/b.wav", &audio::wav());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    let looked = LookedAt::default();
    let queue = chained_queue(&writer, &volume, &looked);

    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(
        kinds(&writer),
        ["scan", "read", "hash", "group", "fingerprint", "quality"]
    );
    let marks = looked.marks.lock().unwrap();
    assert!(
        marks.hash_end.unwrap() <= marks.group_end.unwrap(),
        "grouping ended before the hashes did: {marks:?}"
    );
    drop(marks);
    let tracks = |sql: &'static str| -> i64 {
        writer
            .call(move |c| c.query_row(sql, [], |r| r.get(0)))
            .unwrap()
    };
    assert_eq!(tracks("SELECT count(*) FROM recording_file"), 3);
    assert_eq!(tracks("SELECT count(*) FROM recording"), 2);
    queue.shutdown();
}

#[test]
fn a_walk_with_nothing_to_read_still_groups_the_files_that_have_no_track() {
    let (_dir, volume, music) = drive();
    // Online-only files are never read or hashed without the opt-in, but
    // they're present, so they need tracks.
    put(&music, "cloud.mp3", &audio::mp3());
    mark_offline(&at(&music, "cloud.mp3"));
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    let looked = LookedAt::default();
    let queue = chained_queue(&writer, &volume, &looked);

    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(kinds(&writer), ["scan", "group"]);
    let grouped: i64 = writer
        .call(|c| c.query_row("SELECT count(*) FROM recording_file", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(grouped, 1);

    // Nothing is left without a track: the next walk queues nothing.
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(kinds(&writer), ["scan", "group", "scan"]);
    queue.shutdown();
}

#[test]
fn a_grouping_job_asked_for_while_one_runs_runs_once_more_not_beside_it() {
    let (_dir, volume, _music) = drive();
    let (_db, writer, _reads) = db();
    let looked = LookedAt::default();
    let (release, held) = mpsc::channel::<()>();
    let held = Mutex::new(held);
    let runs = Arc::new(AtomicU64::new(0));
    let counted = runs.clone();
    let queue = chained_queue_with(&writer, &volume, &looked, 2, |b| {
        b.handler(
            JobKind::Group,
            after_group(move |_: &JobContext| {
                counted.fetch_add(1, Ordering::SeqCst);
                let _ = held.lock().unwrap().recv_timeout(Duration::from_secs(30));
                Ok(())
            }),
        )
    });
    let first = crate::grouping::start(&queue, &writer).unwrap();
    wait_until("the first run starts", || runs.load(Ordering::SeqCst) == 1);
    // A hash stage ending meanwhile asks again: one rerun, no parked twin.
    let again = crate::grouping::start(&queue, &writer).unwrap();
    let and_again = crate::grouping::start(&queue, &writer).unwrap();
    assert_eq!((again, and_again), (first, first));
    release.send(()).unwrap();
    drop(release);
    wait_idle(&queue);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert_eq!(kinds(&writer), ["group", "group"]);
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
    assert_eq!(
        kinds(&writer),
        ["scan", "read", "hash", "group", "fingerprint", "quality"]
    );

    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(
        kinds(&writer),
        [
            "scan",
            "read",
            "hash",
            "group",
            "fingerprint",
            "quality",
            "scan"
        ]
    );
    assert_eq!(looked.counts(), (1, 1, 1));
    queue.shutdown();
}

#[test]
fn quality_is_measured_when_the_hashes_are_in_even_if_no_fingerprints_are_due() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &audio::mp3());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    let looked = LookedAt::default();
    let queue = chained_queue(&writer, &volume, &looked);
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    let count = |kind: &str| kinds(&writer).iter().filter(|k| *k == kind).count();
    assert_eq!((count("fingerprint"), count("quality")), (1, 1));

    // As after an update that adds a measurement: the file is hashed and
    // fingerprinted already, and has no measurement yet.
    writer
        .call(|c| c.execute("DELETE FROM file_quality", []))
        .unwrap();
    queue.enqueue(crate::hash::hash_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(count("fingerprint"), 1, "nothing was due a fingerprint");
    assert_eq!(count("quality"), 2, "the measurement didn't wait for one");
    let measured: i64 = writer
        .call(|c| c.query_row("SELECT COUNT(*) FROM file_quality", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(measured, 1);
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
    // No workers here: see the next test for a running one.
    let (_db, writer, _reads) = db();
    // No workers: whatever is queued stays queued.
    let queue = JobQueue::builder(writer.clone()).start().unwrap();
    queue.shutdown();
    let enqueue = |job: NewJob| queue.enqueue(job);

    let first = queue_once(&writer, read_job(None), enqueue).unwrap();
    let again = queue_once(&writer, read_job(None), enqueue).unwrap();
    assert_eq!(again, first);
    let other = queue_once(&writer, read_job(Some(vec![MusicFolderId(3)])), enqueue).unwrap();
    assert_ne!(other, first);
    let same_target = queue_once(&writer, read_job(Some(vec![MusicFolderId(3)])), enqueue);
    assert_eq!(same_target.unwrap(), other);
    // A different kind with the same target is its own job.
    let hash = queue_once(
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
    let after = queue_once(&writer, read_job(None), enqueue).unwrap();
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
    assert_eq!(
        kinds(&writer),
        ["scan", "read", "hash", "group", "fingerprint", "quality"]
    );
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
        [
            "scan",
            "read",
            "hash",
            "group",
            "fingerprint",
            "quality",
            "scan",
            "scan"
        ]
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
    assert_eq!(
        kinds(&writer)[8..],
        ["scan", "read", "hash", "group", "fingerprint", "quality"]
    );
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

#[test]
fn a_user_job_is_not_held_up_by_fingerprint_work_asked_for_while_a_fingerprint_job_runs() {
    // The chain never queues a second fingerprint job beside a running
    // one (it would take a worker and sit waiting for the fingerprint
    // threads): it asks the running one to run once more when it ends.
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &audio::mp3());
    let other = volume.mount.join("Other");
    fs::create_dir(&other).unwrap();
    put(&other, "b.mp3", &audio::mp3());
    let (_db, writer, _reads) = db();
    let folder_a = add_music(&writer, &volume, &music);
    let folder_b = add_music(&writer, &volume, &other);
    let looked = LookedAt::default();
    let release = looked.hold_next_fingerprint();
    let export_ran = Arc::new(AtomicU64::new(0));
    let ran = export_ran.clone();
    let queue = chained_queue_with(&writer, &volume, &looked, 2, |b| {
        b.handler(JobKind::Export, move |_: &JobContext| {
            ran.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    });

    // Folder A's chain reaches its fingerprint job, which is held.
    queue.enqueue(scan_job(Some(vec![folder_a]))).unwrap();
    wait_until("the fingerprint job is at its first file", || {
        looked.marks.lock().unwrap().first_fingerprint.is_some()
    });
    // Folder B's chain runs on the other worker and, at its end, wants
    // fingerprints too.
    queue.enqueue(scan_job(Some(vec![folder_b]))).unwrap();
    let target_b = Some(format!(r#"{{"music_folder_ids":[{}]}}"#, folder_b.0));
    wait_until("folder B is hashed", || {
        jobs(&writer)
            .iter()
            .any(|j| j.0 == "hash" && j.1 == target_b && j.3 == "done")
    });
    let fingerprints = |writer: &Writer| -> Vec<String> {
        jobs(writer)
            .into_iter()
            .filter(|j| j.0 == "fingerprint")
            .map(|j| j.3)
            .collect()
    };
    assert_eq!(
        fingerprints(&writer),
        ["running"],
        "no second job beside it"
    );

    // A user's job isn't held up: a worker is free.
    queue
        .enqueue(NewJob::new(JobKind::Export).priority(Priority::USER))
        .unwrap();
    wait_until("the user's job ran", || {
        export_ran.load(Ordering::SeqCst) == 1
    });
    assert_eq!(fingerprints(&writer), ["running"]);

    // Once the first job ends, it runs once more, for folder B's file.
    release.send(()).unwrap();
    wait_idle(&queue);
    assert_eq!(fingerprints(&writer), ["done", "done"]);
    assert_eq!(looked.fingerprinted.load(Ordering::SeqCst), 2);
    queue.shutdown();
}

/// A rekordbox track at `path` (an absolute path) as rekordbox writes its
/// Location, with no TotalTime (a path match needs none).
fn rekordbox_track_at(writer: &Writer, path: &Path) -> i64 {
    let display = crate::scan::display_path(path).replace('\\', "/");
    let escaped: String = display
        .chars()
        .map(|c| match c {
            ' ' => "%20".to_owned(),
            '%' => "%25".to_owned(),
            c => c.to_string(),
        })
        .collect();
    let location = format!("file://localhost/{escaped}");
    let key = crate::rekordbox::location::decode(&location)
        .unwrap()
        .match_key();
    let attributes = serde_json::json!({ "TrackID": "1", "Location": location }).to_string();
    writer
        .call(move |c| {
            c.execute(
                "INSERT INTO rekordbox_track (attributes, location_key, read_at)
                 VALUES (?1, ?2, '2026-09-30T12:00:00.000Z')",
                (attributes, key),
            )?;
            Ok(c.last_insert_rowid())
        })
        .unwrap()
}

#[test]
fn a_finished_read_queues_a_relink_that_matches_rekordbox_tracks_to_the_scanned_files() {
    let (_dir, volume, music) = drive();
    put(&music, "Crate/a.mp3", &audio::mp3());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    let track = rekordbox_track_at(&writer, &at(&music, "Crate/a.mp3"));
    let looked = LookedAt::default();
    let queue = chained_queue(&writer, &volume, &looked);

    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    // The first relink was asked for by the read, and it found the file.
    // More follow: the grouping job and the fingerprint job each ask for
    // one, and whether those fold into one depends on when they run, so
    // they aren't counted. What always holds: every one finished, and a
    // relink ran after the last grouping run (a grouping run asks for one
    // once its summary is out, and that one starts after it was asked).
    let relinked = relinks(&writer);
    assert_eq!(relinked[0], ("done".into(), "read".into()));
    assert!(relinked.iter().all(|r| r.0 == "done"));
    let runs = looked.runs.lock().unwrap().clone();
    let last_group = runs.iter().rposition(|r| *r == "group").unwrap();
    assert!(
        runs[last_group..].contains(&"relink"),
        "no relink after the last grouping run: {runs:?}"
    );
    let (file, method): (Option<i64>, Option<String>) = writer
        .call(move |c| {
            c.query_row(
                "SELECT file_id, relink_method FROM rekordbox_track WHERE id = ?1",
                [track],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap();
    assert!(file.is_some());
    assert_eq!(method.as_deref(), Some("path"));

    // A walk that finds nothing new reads nothing, so asks for no relink.
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(relinks(&writer).len(), relinked.len());
    queue.shutdown();
}

#[test]
fn a_finished_fingerprint_job_queues_a_relink() {
    // Only the two handlers, so nothing else asks for a relink.
    let (_db, writer, _reads) = db();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(
            JobKind::Fingerprint,
            after_fingerprint(|_: &JobContext| Ok(())),
        )
        .handler(JobKind::Relink, |_: &JobContext| Ok(()))
        .start()
        .unwrap();
    queue
        .enqueue(crate::fingerprint::fingerprint_job(None))
        .unwrap();
    wait_idle(&queue);
    assert_eq!(relinks(&writer), [("done".into(), "fingerprint".into())]);
    queue.shutdown();
}

#[test]
fn a_finished_grouping_job_queues_a_relink_and_a_relink_queues_no_grouping() {
    let (_db, writer, _reads) = db();
    let (_dir, volume, _music) = drive();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(
            JobKind::Group,
            after_group(crate::grouping::Grouper::default()),
        )
        .handler(JobKind::Relink, Relinker::new(move || volume.clone()))
        .start()
        .unwrap();
    queue.enqueue(crate::grouping::group_job()).unwrap();
    wait_idle(&queue);
    // One grouping job, one relink after it, and then nothing: no loop.
    assert_eq!(kinds(&writer), ["group"]);
    assert_eq!(relinks(&writer), [("done".into(), "group".into())]);
    queue.shutdown();
}

#[test]
fn a_fingerprint_job_that_stops_early_asks_for_no_relink() {
    let (_db, writer, _reads) = db();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(
            JobKind::Fingerprint,
            after_fingerprint(|_: &JobContext| Err(JobError::failed("the drive went away"))),
        )
        .start()
        .unwrap();
    queue
        .enqueue(crate::fingerprint::fingerprint_job(None))
        .unwrap();
    wait_idle(&queue);
    assert!(relinks(&writer).is_empty());
    queue.shutdown();
}

#[test]
fn a_walk_that_stops_early_asks_for_no_relink() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &audio::mp3());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    let v = volume.clone();
    let queue = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(
            JobKind::Scan,
            after_walk(Walker::new(move || v.clone(), |_| {})),
        )
        .handler(
            JobKind::Read,
            after_read(|_: &JobContext| Err(JobError::failed("the drive went away"))),
        )
        .start()
        .unwrap();
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    assert_eq!(kinds(&writer), ["scan", "read"]);
    assert!(
        relinks(&writer).is_empty(),
        "a failed read asks for nothing"
    );
    queue.shutdown();
}
