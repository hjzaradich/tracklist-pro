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

use super::passes::{db, pass, Db};
use super::synthetic::{fingerprint, reencoded, track};

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
fn matching_is_due_until_a_pass_has_been_through_every_file_and_again_when_a_fingerprint_arrives() {
    let db = db();
    assert!(!due(&db), "no fingerprints: nothing to do");
    let a = track(1, 400);
    db.add_items("a.flac", a.clone());
    assert!(due(&db));

    // A pass that stops before the file is through leaves it due.
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

    // A fingerprint that goes away and comes back the same lost its
    // results on the way: it's due.
    let blob = fingerprint(a.clone()).to_blob();
    db.sql("UPDATE file SET fingerprint = NULL WHERE id = 1", ());
    assert!(!due(&db), "a file with no fingerprint has nothing to match");
    db.sql("UPDATE file SET fingerprint = ?1 WHERE id = 1", (blob,));
    assert!(due(&db));
    refresh(&db.writer).unwrap();
    assert!(!due(&db));
    assert_eq!(db.pairs(), [(1, copy)]);

    // A new version of the comparison makes every file due, with nothing
    // deleted.
    db.sql("UPDATE fingerprint_matched SET version = version + 1", ());
    assert!(due(&db));
}

#[test]
fn a_file_that_goes_missing_is_not_due_and_one_that_comes_back_is_compared_with_what_arrived_meanwhile(
) {
    let db = db();
    let a = track(1, 400);
    db.add_items("a.flac", a.clone());
    db.add_items("b.flac", track(2, 400));
    let matcher = Matcher::new();
    pass(&matcher, &db);
    assert!(!due(&db));

    // File 1 is gone from its folder. Nothing to do about that.
    db.sql("UPDATE file SET present = 0 WHERE id = 1", ());
    assert!(!due(&db));
    // A copy of it arrives while it's away, and is matched against what's
    // there: nothing.
    let copy = db.add_items("a.mp3", reencoded(&a, 1, 7));
    assert!(due(&db));
    pass(&matcher, &db);
    assert!(!due(&db));
    assert_eq!(db.pairs(), []);

    // It comes back: it's due, and finds the copy that arrived meanwhile.
    db.sql("UPDATE file SET present = 1 WHERE id = 1", ());
    assert!(due(&db));
    let summary = pass(&matcher, &db);
    assert_eq!((summary.compared, summary.stored), (1, 1));
    assert_eq!(db.pairs(), [(1, copy)]);
    assert!(!due(&db));
}

#[test]
fn when_the_file_standing_for_identical_ones_goes_missing_the_rest_are_due_even_after_a_restart() {
    let db = db();
    let a = track(1, 400);
    db.add_items("a.flac", a.clone());
    db.add_items("a.wav", a.clone());
    db.add_items("a (backup).wav", a.clone());
    let mp3 = db.add_items("a.mp3", reencoded(&a, 1, 7));
    refresh(&db.writer).unwrap();
    assert_eq!(db.pairs(), [(1, 2), (1, 3), (1, mp3)]);
    assert!(!due(&db));

    // File 1 held the results for all three. With it gone, file 2 has to.
    db.sql("UPDATE file SET present = 0 WHERE id = 1", ());
    assert!(due(&db));
    // A new matcher, as after a restart: it knows nothing but the tables.
    let summary = refresh(&db.writer).unwrap();
    assert_eq!((summary.compared, summary.stored), (1, 2));
    assert_eq!(
        db.pairs(),
        [(1, 2), (1, 3), (1, mp3), (2, 3), (2, mp3)],
        "file 1's results are kept; file 2 has its own now"
    );
    assert!(!due(&db));
}

#[test]
fn a_fingerprint_this_build_cannot_read_does_not_keep_matching_due() {
    let db = db();
    db.add_items("a.flac", track(1, 400));
    db.sql(
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime, fingerprint)
         VALUES (1, 'odd.flac', 'odd.flac', 1000, 1000, x'00010203')",
        (),
    );
    assert!(due(&db));
    refresh(&db.writer).unwrap();
    assert!(!due(&db));
    assert_eq!(db.pairs(), []);
}

#[test]
fn a_pass_stopped_partway_leaves_every_file_either_through_with_all_its_results_or_due() {
    let expected = all_pairs();
    for stop_after in [3, 10, 40, 90, 130] {
        let db = library();
        let mut ticks = 0;
        let stopped = Matcher::new().pass(&db.writer, &mut |_| {
            ticks += 1;
            if ticks > stop_after {
                Err(DbError::WriterGone)
            } else {
                Ok(())
            }
        });
        assert!(stopped.is_err(), "stopping after {stop_after} reports");
        assert_consistent(
            &db,
            &expected,
            &format!("stopped after {stop_after} reports"),
        );
        assert!(due(&db));
        // And a new matcher finishes from the tables alone.
        refresh(&db.writer).unwrap();
        assert_eq!(db.pairs(), expected);
        assert!(!due(&db));
    }
}

/// Every file with a `fingerprint_matched` row has every result it should
/// have: `expected` is what a full pass stores.
fn assert_consistent(db: &Db, expected: &[(i64, i64)], when: &str) {
    let through: Vec<i64> = db
        .writer
        .call(|c| {
            c.prepare("SELECT file_id FROM fingerprint_matched")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    let stored = db.pairs();
    for &(a, b) in expected {
        if through.contains(&a) || through.contains(&b) {
            assert!(
                stored.contains(&(a, b)),
                "{when}: a file of the pair ({a}, {b}) is marked through, but the pair has no result"
            );
        }
    }
}

#[test]
fn matching_finishes_though_a_relink_arrives_during_every_run_and_each_stop_leaves_the_ledger_consistent(
) {
    let expected = all_pairs();
    let db = library();
    // Every matching run stops at its fifth report until a relink has been
    // queued behind it: relinks keep arriving for as long as it runs.
    let (reached, reached_rx) = mpsc::channel::<()>();
    let (go, go_rx) = mpsc::channel::<()>();
    let go_rx = Mutex::new(go_rx);
    let matcher = Matcher::new().on_report(move |n| {
        if n == 5 {
            reached.send(()).unwrap();
            go_rx.lock().unwrap().recv().unwrap();
        }
    });
    // What each relink saw: whether the ledger agreed with the results.
    let writer = db.writer.clone();
    let wanted = expected.clone();
    let checks: Arc<Mutex<Vec<Result<(), String>>>> = Arc::default();
    let seen = checks.clone();
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        .handler(JobKind::Match, after_matching(matcher))
        .handler(JobKind::Relink, move |_: &JobContext| {
            let wanted = wanted.clone();
            let check = writer
                .call(move |c| {
                    let through: Vec<i64> = c
                        .prepare("SELECT file_id FROM fingerprint_matched")?
                        .query_map([], |r| r.get(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    let stored: Vec<(i64, i64)> = c
                        .prepare("SELECT file_a, file_b FROM fingerprint_match")?
                        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                        .collect::<rusqlite::Result<_>>()?;
                    Ok(wanted
                        .iter()
                        .find(|(a, b)| {
                            (through.contains(a) || through.contains(b))
                                && !stored.contains(&(*a, *b))
                        })
                        .copied())
                })
                .map_err(jobs::JobError::from)?;
            seen.lock().unwrap().push(match check {
                None => Ok(()),
                Some(pair) => Err(format!("{pair:?} is marked through with no result")),
            });
            Ok(())
        })
        .start()
        .unwrap();
    queue.enqueue(matching_job()).unwrap();

    // Feed a relink to every run that gets as far as its fifth report,
    // until matching has nothing left queued or running.
    let mut runs = 0;
    loop {
        match reached_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(()) => {
                runs += 1;
                assert!(runs <= 20, "matching never got through");
                queue.enqueue(crate::relink::relink_job()).unwrap();
                go.send(()).unwrap();
            }
            Err(_) if queue.activity().unwrap().jobs.is_empty() => break,
            Err(_) => {}
        }
    }
    queue.shutdown();

    let all = jobs(&db.writer);
    assert!(all.iter().all(|j| j.status == JobStatus::Done), "{all:?}");
    let matching = all
        .iter()
        .filter(|j| j.kind == Some(JobKind::Match))
        .count();
    let relinks = all
        .iter()
        .filter(|j| j.kind == Some(JobKind::Relink))
        .count();
    assert!(
        matching >= 2 && relinks >= 2,
        "it made way more than once: {matching} matching runs, {relinks} relinks"
    );
    // Each run but the last ended by making way, and the relink ran next.
    for pair in all.windows(2) {
        if pair[0].kind == Some(JobKind::Match) {
            assert_eq!(pair[1].kind, Some(JobKind::Relink), "{all:?}");
        }
    }
    let checks = checks.lock().unwrap();
    assert_eq!(checks.len(), relinks);
    for check in checks.iter() {
        assert_eq!(*check, Ok(()));
    }
    assert_eq!(db.pairs(), expected);
    assert!(!due(&db));
}

#[test]
fn an_index_over_the_budget_is_dropped_after_a_pass_and_the_next_pass_builds_it_again_with_the_same_results(
) {
    // One matcher keeps its index, the other may keep none at all.
    let (kept, dropped) = (library(), library());
    let (keeping, dropping) = (Matcher::new(), Matcher::new().keep_index_up_to(0));
    let first = (pass(&keeping, &kept), pass(&dropping, &dropped));
    assert_eq!(first.0, first.1);
    assert_eq!(kept.all(), dropped.all());

    // The same new files for both: a copy of a track that had none, an
    // exact copy, and a track of its own.
    for db in [&kept, &dropped] {
        db.add_items("7 (new).mp3", reencoded(&track(7, 400), 91, 7));
        db.add_items("8 (backup).flac", track(8, 400));
        db.add_items("900.flac", track(900, 400));
    }
    let later = (pass(&keeping, &kept), pass(&dropping, &dropped));
    // The one that kept its index read three new fingerprints; the one
    // that dropped it read them all again…
    assert_eq!(later.0.changed, 3);
    assert_eq!(later.1.changed, later.1.files);
    // …and they found and stored the same.
    assert_eq!(
        (later.0.candidates, later.0.compared, later.0.stored),
        (later.1.candidates, later.1.compared, later.1.stored)
    );
    assert_eq!(later.0.stored, 2);
    assert_eq!(kept.all(), dropped.all());
    assert!(!due(&kept) && !due(&dropped));
}

#[test]
fn the_index_of_an_ordinary_library_is_kept_between_passes() {
    assert_eq!(crate::matching::KEEP_INDEX_UP_TO, 250 * 1024 * 1024);
    let db = library();
    let matcher = Matcher::new();
    pass(&matcher, &db);
    db.add_items("900.flac", track(900, 400));
    let later = pass(&matcher, &db);
    assert_eq!(
        later.changed, 1,
        "only the new fingerprint was read into it"
    );
}
