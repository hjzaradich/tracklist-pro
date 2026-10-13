#![cfg(test)]
//! 1cA-13: what the startup step queues when work is due and no job is.
//!
//! Each test builds a library the real chain has been through, puts the
//! database as a run that died (or an upgrade) would leave it, and asks
//! [`ask_what_is_due`] with an `enqueue` that only stores the row, so the
//! test sees exactly what was queued.

use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;

use super::chain::{chained_queue, jobs, two_copies_and_another, LookedAt};
use super::support::{db, TempVolume};
use super::walk::{add_music, drive};
use crate::db::{DbError, ReadPool, Writer};
use crate::jobs::startup::{ask_what_is_due, ask_with};
use crate::jobs::{JobId, JobQueue, NewJob};
use crate::scan::chain::reach_by;
use crate::scan::walk::scan_job;

/// Long enough that a passing test never hits it; a failing one ends here
/// with a message instead of hanging.
const PATIENCE: Duration = Duration::from_secs(60);

/// Waits until nothing is queued or running. No time limit of its own: how
/// long the chain takes on a loaded machine is not what these tests are
/// about (the test runner's own limit ends a hung one).
fn wait_until_idle(queue: &JobQueue) {
    while !queue.activity().unwrap().jobs.is_empty() {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The library every test starts from, scanned through the whole chain once
/// per run of the tests (the chain fingerprints real audio, which is slow
/// and, run for every test at once, is what made them depend on how busy
/// the machine was). Each test gets its own copy of the database.
struct Template {
    volume: TempVolume,
    /// The database as the chain left it, log folded in.
    database: std::path::PathBuf,
    _music: tempfile::TempDir,
    _db: tempfile::TempDir,
}

fn template() -> &'static Template {
    static TEMPLATE: OnceLock<Template> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let (music_dir, volume, music) = drive();
        two_copies_and_another(&music);
        let (db_dir, writer, _reads) = db();
        add_music(&writer, &volume, &music);
        let queue = chained_queue(&writer, &volume, &LookedAt::default());
        queue.enqueue(scan_job(None)).unwrap();
        wait_until_idle(&queue);
        queue.shutdown();
        // The queue here has no index job: the search index counts as
        // current, as it is in an app that has run its jobs.
        writer
            .call(|c| c.execute("DELETE FROM search_due", []))
            .unwrap();
        // Fold the log into the database file, so the file alone is the
        // whole database.
        while writer.checkpoint().unwrap() != crate::db::Checkpoint::Emptied {
            std::thread::sleep(Duration::from_millis(10));
        }
        Template {
            volume,
            database: writer.path().to_path_buf(),
            _music: music_dir,
            _db: db_dir,
        }
    })
}

/// A copy of the scanned library's database (two copies of a song and
/// another, taken through the whole chain). Nothing runs; the tests look at
/// what the startup step queues.
fn scanned() -> ((), TempVolume, tempfile::TempDir, Writer) {
    let template = template();
    let dir = tempfile::tempdir().unwrap();
    let path = crate::write_guard::test_path(dir.path(), crate::db::DB_FILE_NAME);
    std::fs::copy(&template.database, path.as_path()).unwrap();
    let writer = Writer::open(&path).unwrap();
    // A volume of its own over the same folder: clones of one share the
    // plugged-in switch, and a test that unplugs it must not unplug it for
    // the others running beside it.
    let volume = TempVolume::new(&template.volume.mount, 0x0BAD_F00D);
    ((), volume, dir, writer)
}

/// Asks, storing each job the way the queue does (a queued row), and
/// returns how many asks failed and the jobs now queued: kind and target.
fn ask(writer: &Writer, volume: &TempVolume) -> (usize, Vec<(String, Option<String>)>) {
    let stored = writer.clone();
    let enqueue = move |job: NewJob| -> Result<JobId, DbError> {
        stored.call(move |c| {
            c.execute(
                "INSERT INTO job (kind, target, priority) VALUES (?1, ?2, ?3)",
                (
                    job.kind.as_str(),
                    job.target.map(|t| t.to_string()),
                    job.priority.0,
                ),
            )?;
            Ok(JobId(c.last_insert_rowid()))
        })
    };
    let volume = volume.clone();
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    let failed =
        ask_what_is_due(writer, &reads, &enqueue, &reach_by(move || volume.clone())).failed;
    (failed, queued(writer))
}

/// The jobs waiting in the queue, oldest first.
fn queued(writer: &Writer) -> Vec<(String, Option<String>)> {
    writer
        .call(|c| {
            let mut s =
                c.prepare("SELECT kind, target FROM job WHERE status = 'queued' ORDER BY id")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap()
}

fn kinds(queued: &[(String, Option<String>)]) -> Vec<&str> {
    queued.iter().map(|(kind, _)| kind.as_str()).collect()
}

/// Makes `stage` of every file due, as a version bump in an upgrade does.
fn bump(writer: &Writer, stage: &str) {
    let stage = stage.to_owned();
    writer
        .call(move |c| {
            c.execute(
                "UPDATE file_stage SET version = 999 WHERE stage = ?1",
                [stage],
            )
        })
        .unwrap();
}

#[test]
fn a_library_with_nothing_due_queues_nothing() {
    let (_dir, volume, _db, writer) = scanned();
    let (failed, queued) = ask(&writer, &volume);
    assert_eq!((failed, kinds(&queued)), (0, Vec::<&str>::new()));
}

#[test]
fn an_app_that_died_between_hashing_and_fingerprinting_asks_for_the_fingerprint_stage_at_the_next_start(
) {
    let (_dir, volume, _db, writer) = scanned();
    bump(&writer, "fingerprint");
    let (failed, queued) = ask(&writer, &volume);
    // The fingerprint job, and nothing after it: it chains on to the rest.
    assert_eq!((failed, kinds(&queued)), (0, vec!["fingerprint"]));
}

#[test]
fn a_hash_stage_made_due_by_an_upgrade_is_asked_for_on_the_folders_that_can_be_reached() {
    let (_dir, volume, _db, writer) = scanned();
    bump(&writer, "hash");
    let (failed, queued) = ask(&writer, &volume);
    assert_eq!((failed, kinds(&queued)), (0, vec!["hash"]));
    let folder: i64 = writer
        .call(|c| c.query_row("SELECT id FROM music_folder", [], |r| r.get(0)))
        .unwrap();
    let target: serde_json::Value = serde_json::from_str(queued[0].1.as_deref().unwrap()).unwrap();
    assert_eq!(target, serde_json::json!({ "music_folder_ids": [folder] }));
}

#[test]
fn a_read_that_is_due_is_left_to_the_catch_up_scan_and_nothing_after_it_is_asked() {
    let (_dir, volume, _db, writer) = scanned();
    bump(&writer, "read");
    bump(&writer, "hash");
    bump(&writer, "fingerprint");
    let (failed, queued) = ask(&writer, &volume);
    assert_eq!((failed, kinds(&queued)), (0, Vec::<&str>::new()));
}

#[test]
fn files_with_no_track_are_grouped_and_fingerprints_not_yet_compared_are_matched() {
    let (_dir, volume, _db, writer) = scanned();
    writer
        .call(|c| {
            c.execute("DELETE FROM recording_file", [])?;
            c.execute("DELETE FROM fingerprint_matched", [])
        })
        .unwrap();
    let (failed, queued) = ask(&writer, &volume);
    // Deciding waits for matching, as in the chain. Then the roles job (a
    // group job with a target) and the search index: tracks that lost their
    // files are due for both.
    assert_eq!(
        (failed, kinds(&queued)),
        (0, vec!["group", "match", "group", "index"])
    );
}

#[test]
fn a_drive_that_is_not_plugged_in_gets_no_hash_fingerprint_or_quality_job() {
    let (_dir, volume, _db, writer) = scanned();
    bump(&writer, "hash");
    bump(&writer, "fingerprint");
    writer
        .call(|c| c.execute("DELETE FROM file_quality", []))
        .unwrap();
    volume.set_online(false);
    let (failed, queued) = ask(&writer, &volume);
    // Only the roles job (the dropped measurements made tracks due; roles
    // are worked out from the database, whatever is plugged in).
    assert_eq!((failed, kinds(&queued)), (0, vec!["group"]));
    let roles = crate::grouping::GroupTarget::Roles.to_json().unwrap();
    let target: serde_json::Value = serde_json::from_str(queued[0].1.as_deref().unwrap()).unwrap();
    assert_eq!(target, roles);

    // Plugged back in, the same database asks for the hashes.
    volume.set_online(true);
    let (_, queued) = ask(&writer, &volume);
    assert_eq!(kinds(&queued), ["group", "hash"]);
}

#[test]
fn asking_twice_queues_each_job_once() {
    let (_dir, volume, _db, writer) = scanned();
    bump(&writer, "fingerprint");
    ask(&writer, &volume);
    let (failed, queued) = ask(&writer, &volume);
    assert_eq!((failed, kinds(&queued)), (0, vec!["fingerprint"]));
}

#[test]
fn nothing_is_queued_for_a_library_with_no_music_folders() {
    let (_db, writer, _reads) = db();
    let (_dir, volume, _music) = drive();
    let (failed, queued) = ask(&writer, &volume);
    assert_eq!((failed, kinds(&queued)), (0, Vec::<&str>::new()));
}

#[test]
fn an_ask_that_fails_is_counted_and_the_stages_after_it_are_still_asked() {
    let (_dir, volume, _db, writer) = scanned();
    // Four stages have work: grouping, matching, the roles and the index.
    writer
        .call(|c| {
            c.execute("DELETE FROM recording_file", [])?;
            c.execute("DELETE FROM fingerprint_matched", [])
        })
        .unwrap();
    let tried = std::sync::atomic::AtomicUsize::new(0);
    let enqueue = |_: NewJob| -> Result<JobId, DbError> {
        tried.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(DbError::WriterGone)
    };
    let reach_volume = volume.clone();
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    let failed = ask_what_is_due(
        &writer,
        &reads,
        &enqueue,
        &reach_by(move || reach_volume.clone()),
    )
    .failed;
    assert_eq!(failed, 4);
    assert_eq!(
        tried.load(std::sync::atomic::Ordering::SeqCst),
        4,
        "the later stage was asked though the first failed"
    );
}

#[test]
fn a_walk_with_nothing_to_read_does_not_go_on_to_the_hashes_or_the_fingerprints() {
    // Why the startup list has the hash and fingerprint stages: the
    // catch-up scan at startup is a walk, and a walk that finds nothing
    // changed asks for grouping, quality, matching and deciding only.
    let (_dir, volume, _db, writer) = scanned();
    bump(&writer, "hash");
    bump(&writer, "fingerprint");
    let before = jobs(&writer).len();
    let queue = chained_queue(&writer, &volume, &LookedAt::default());
    queue.enqueue(scan_job(None)).unwrap();
    wait_until_idle(&queue);
    queue.shutdown();
    let after: Vec<String> = jobs(&writer)
        .into_iter()
        .skip(before)
        .map(|j| j.0)
        .collect();
    assert!(
        !after.iter().any(|k| k == "hash" || k == "fingerprint"),
        "{after:?}"
    );
}

#[test]
fn after_an_upgrade_that_left_every_track_due_the_next_start_asks_for_the_roles_job() {
    let (_dir, volume, _db, writer) = scanned();
    // The roles were made by an older build: every track is due, and no job
    // is queued.
    writer
        .call(|c| c.execute("DELETE FROM setting WHERE key = 'file_roles_version'", []))
        .unwrap();
    let (failed, queued) = ask(&writer, &volume);
    assert_eq!((failed, kinds(&queued)), (0, vec!["group"]));
    let target: serde_json::Value = serde_json::from_str(queued[0].1.as_deref().unwrap()).unwrap();
    assert_eq!(
        target,
        crate::grouping::GroupTarget::Roles.to_json().unwrap()
    );
    // Asked again with that job waiting: still one.
    let (_, queued) = ask(&writer, &volume);
    assert_eq!(kinds(&queued), ["group"]);
}

#[test]
fn a_search_index_that_is_behind_is_asked_for_at_the_next_start_and_a_current_one_is_not() {
    let (_dir, volume, _db, writer) = scanned();
    writer
        .call(|c| c.execute("DELETE FROM search_due", []))
        .unwrap();
    let (failed, queued) = ask(&writer, &volume);
    assert_eq!((failed, kinds(&queued)), (0, Vec::<&str>::new()));

    // A run that died with a track's row out of date (-1 stands for every
    // Library track, as after a rekordbox read).
    writer
        .call(|c| c.execute("INSERT INTO search_due (recording_id) VALUES (-1)", []))
        .unwrap();
    let (failed, queued) = ask(&writer, &volume);
    assert_eq!((failed, kinds(&queued)), (0, vec!["index"]));
    // Asked again with that job waiting: still one.
    let (_, queued) = ask(&writer, &volume);
    assert_eq!(kinds(&queued), ["index"]);
}

#[test]
fn quality_and_matching_wait_while_the_fingerprints_are_owed() {
    let (_dir, volume, _db, writer) = scanned();
    // The fingerprint stage is out of date, and quality and matching each
    // have files due too: they wait for the fingerprint job's chain.
    bump(&writer, "fingerprint");
    writer
        .call(|c| {
            c.execute("DELETE FROM file_quality", [])?;
            c.execute("DELETE FROM fingerprint_matched", [])
        })
        .unwrap();
    let (failed, queued) = ask(&writer, &volume);
    // (The second is the roles job: the dropped measurements made tracks due.)
    assert_eq!((failed, kinds(&queued)), (0, vec!["fingerprint", "group"]));
}

#[test]
fn grouping_and_deciding_wait_while_a_file_is_still_to_be_read() {
    let (_dir, volume, _db, writer) = scanned();
    // A file the walk has found and nothing has read yet: it has no track,
    // and no decision. Grouping and deciding wait for its read and hash.
    writer
        .call(|c| {
            c.execute(
                "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime, present)
                 VALUES ((SELECT min(id) FROM music_folder), 'new.wav', 'new.wav', 1, 1, 1)",
                [],
            )
        })
        .unwrap();
    let (failed, queued) = ask(&writer, &volume);
    assert_eq!(failed, 0);
    // Nothing but the roles job (a group job with that target).
    let roles = crate::grouping::GroupTarget::Roles.to_json().unwrap();
    for (kind, target) in &queued {
        let target: Option<serde_json::Value> =
            target.as_deref().map(|t| serde_json::from_str(t).unwrap());
        assert!(
            kind == "group" && target.as_ref() == Some(&roles),
            "{kind} {target:?}"
        );
    }
}

/// Runs `work` on a thread and fails the test, instead of hanging, if it has
/// not finished within [`PATIENCE`].
fn finishes<T: Send + 'static>(what: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(work());
    });
    finished
        .recv_timeout(PATIENCE)
        .unwrap_or_else(|_| panic!("{what} never finished"))
}

#[test]
fn the_startup_checks_finish_while_the_database_writer_is_busy_with_something_else() {
    let (_dir, volume, _db, writer) = scanned();
    // The writer is taken by a job that doesn't end until released.
    let (entered, inside) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let busy = writer.clone();
    let holder = std::thread::spawn(move || {
        busy.call(move |_| {
            entered.send(()).unwrap();
            let _ = released.recv_timeout(PATIENCE);
            Ok(())
        })
    });
    inside.recv_timeout(PATIENCE).unwrap();

    // Nothing is due, so every ask is only checks: all on read connections.
    let asked = finishes("the startup step", {
        let writer = writer.clone();
        move || {
            let reads = ReadPool::open(writer.guarded_path()).unwrap();
            let enqueue = |_: NewJob| -> Result<JobId, DbError> { panic!("nothing is due") };
            let reach = reach_by(move || volume.clone());
            ask_what_is_due(&writer, &reads, &enqueue, &reach)
        }
    });
    assert_eq!(asked.failed, 0);
    assert_eq!(asked.took.len(), 8, "every stage was asked");
    release.send(()).unwrap();
    holder.join().unwrap().unwrap();
}

#[test]
fn a_write_on_the_writer_is_not_blocked_while_a_startup_check_is_held_open() {
    let (_dir, volume, _db, writer) = scanned();
    bump(&writer, "fingerprint");
    let (entered, inside) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let asking = std::thread::spawn({
        let writer = writer.clone();
        move || {
            let reads = ReadPool::open(writer.guarded_path()).unwrap();
            let stored = writer.clone();
            let enqueue = move |job: NewJob| -> Result<JobId, DbError> {
                stored.call(move |c| {
                    c.execute(
                        "INSERT INTO job (kind, target, priority) VALUES (?1, ?2, ?3)",
                        (job.kind.as_str(), None::<String>, job.priority.0),
                    )?;
                    Ok(JobId(c.last_insert_rowid()))
                })
            };
            let reach = reach_by(move || volume.clone());
            // The first check holds, on its read connection, until released.
            let held = std::sync::atomic::AtomicBool::new(false);
            let hold = || {
                if !held.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    entered.send(()).unwrap();
                    let _ = released.lock().unwrap().recv_timeout(PATIENCE);
                }
            };
            ask_with(&writer, &reads, &enqueue, &reach, &hold)
        }
    });
    inside.recv_timeout(PATIENCE).unwrap();

    // A check is open right now. A write goes through.
    finishes("a write on the writer", {
        let writer = writer.clone();
        move || {
            writer
                .call(|c| {
                    c.execute(
                        "INSERT INTO setting (key, value) VALUES ('a_user_write', '1')",
                        [],
                    )
                })
                .unwrap()
        }
    });

    // And the held check, once released, leads to the same job as ever.
    release.send(()).unwrap();
    let asked = asking.join().unwrap();
    assert_eq!(asked.failed, 0);
    assert_eq!(kinds(&queued(&writer)), ["fingerprint"]);
}

/// What the startup checks cost when nothing is due, on 100,000 files: the
/// worst case, since each check then looks at every file once (it stops at
/// the first due row otherwise). The checks run on read connections, as at
/// startup; the figures are the time each takes, not time the writer is held. Prints; asserts only that nothing was
/// queued, so the figures are for a library that really had nothing due.
///
/// ```text
/// cargo test --lib -- --ignored --nocapture what_the_startup_checks_cost
/// ```
#[test]
#[ignore = "a measurement, printed"]
fn what_the_startup_checks_cost_on_100k_files_with_nothing_due() {
    use std::time::Instant;
    let (_dir, volume, music) = drive();
    let (_db, writer, reads) = db();
    add_music(&writer, &volume, &music);
    let started = Instant::now();
    let sql = format!(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000)
         INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime, audio_hash,
                           fingerprint, present)
         SELECT 1, 'f' || i, 'f' || i, 1000, 1000, randomblob(34), randomblob(64), 1 FROM n;
         INSERT INTO file_stage (file_id, stage, version, size, mtime, status)
         SELECT id, 'read', {read}, 1000, 1000, 'done' FROM file;
         INSERT INTO file_stage (file_id, stage, version, size, mtime, status)
         SELECT id, 'hash', {hash}, 1000, 1000, 'done' FROM file;
         INSERT INTO file_stage (file_id, stage, version, size, mtime, status)
         SELECT id, 'fingerprint', {fingerprint}, 1000, 1000, 'done' FROM file;
         INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_gap, decoded_ms, ended)
         SELECT id, audio_hash, {quality}, 'too_short', 1000, 'complete' FROM file;
         INSERT INTO fingerprint_matched (file_id, version, stands)
         SELECT id, {matching}, id FROM file;
         INSERT INTO recording (id) SELECT id FROM file;
         INSERT INTO recording_file (recording_id, file_id, role) SELECT id, id, 'best' FROM file;
         INSERT INTO grouping_file (file_id, rev, decided_as, placed_key)
         SELECT id, 0, {decided}, audio_hash FROM file;
         DELETE FROM best_file_due;",
        read = crate::read::READ_VERSION,
        hash = crate::hash::DEFINITION,
        fingerprint = crate::fingerprint::VERSION,
        quality = crate::quality::VERSION,
        matching = crate::matching::VERSION,
        decided = crate::grouping::decided_as(),
    );
    writer.call(move |c| c.execute_batch(&sql)).unwrap();
    // The roles are of this build's version, so nothing is due there either.
    ask(&writer, &volume);
    writer
        .call(|c| {
            c.execute("DELETE FROM best_file_due", [])?;
            // This library never ran the index job: its index is current.
            c.execute("DELETE FROM search_due", [])?;
            c.execute("DELETE FROM job", [])
        })
        .unwrap();
    println!("built 100,000 files in {:?}", started.elapsed());

    let time = |what: &str, check: fn(&rusqlite::Connection) -> rusqlite::Result<bool>| {
        let started = Instant::now();
        let due = reads.read(check).unwrap();
        println!("{what:>34}: {:>8.1?}  due: {due}", started.elapsed());
    };
    time("matching: any file due", crate::matching::any_due);
    time("grouping: a file with no track", |c| {
        crate::grouping::any_ungrouped(c)
    });
    time("roles: any track due", crate::file_roles::any_due);
    let stored = writer.clone();
    let enqueue = move |job: NewJob| -> Result<JobId, DbError> {
        stored.call(move |c| {
            c.execute(
                "INSERT INTO job (kind, target, priority) VALUES (?1, ?2, ?3)",
                (
                    job.kind.as_str(),
                    job.target.map(|t| t.to_string()),
                    job.priority.0,
                ),
            )?;
            Ok(JobId(c.last_insert_rowid()))
        })
    };
    let volume = volume.clone();
    let started = Instant::now();
    let asked = ask_what_is_due(&writer, &reads, &enqueue, &reach_by(move || volume.clone()));
    println!(
        "{:>34}: {:>8.1?}",
        "the whole startup step",
        started.elapsed()
    );
    for (stage, took) in &asked.took {
        println!("{stage:>34}: {took:>8.1?}");
    }
    assert_eq!(asked.failed, 0);
    assert_eq!(kinds(&queued(&writer)), Vec::<&str>::new());
}
