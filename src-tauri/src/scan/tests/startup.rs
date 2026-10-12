#![cfg(test)]
//! 1cA-13: what the startup step queues when work is due and no job is.
//!
//! Each test builds a library the real chain has been through, puts the
//! database as a run that died (or an upgrade) would leave it, and asks
//! [`ask_what_is_due`] with an `enqueue` that only stores the row, so the
//! test sees exactly what was queued.

use super::chain::{chained_queue, jobs, two_copies_and_another, wait_idle, LookedAt};
use super::support::{db, TempVolume};
use super::walk::{add_music, drive};
use crate::db::{DbError, Writer};
use crate::jobs::startup::ask_what_is_due;
use crate::jobs::{JobId, NewJob};
use crate::scan::chain::reach_by;
use crate::scan::walk::scan_job;

/// A music folder with two copies of a song and another, scanned through the whole chain; the
/// queue is stopped, so nothing runs while a test looks.
fn scanned() -> (tempfile::TempDir, TempVolume, tempfile::TempDir, Writer) {
    let (dir, volume, music) = drive();
    two_copies_and_another(&music);
    let (db_dir, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    let queue = chained_queue(&writer, &volume, &LookedAt::default());
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    queue.shutdown();
    (dir, volume, db_dir, writer)
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
    let failed = ask_what_is_due(writer, &enqueue, &reach_by(move || volume.clone())).failed;
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
    // Deciding waits for matching, as in the chain. The last is the roles
    // job (a group job with a target): tracks that lost their files are due.
    assert_eq!(
        (failed, kinds(&queued)),
        (0, vec!["group", "match", "group"])
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
    // Three stages have work: grouping, matching and the roles.
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
    let failed = ask_what_is_due(&writer, &enqueue, &reach_by(move || reach_volume.clone())).failed;
    assert_eq!(failed, 3);
    assert_eq!(
        tried.load(std::sync::atomic::Ordering::SeqCst),
        3,
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
    wait_idle(&queue);
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

/// What the startup checks cost when nothing is due, on 100,000 files: the
/// worst case, since each check then looks at every file once (it stops at
/// the first due row otherwise). Prints; asserts only that nothing was
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
    let (_db, writer, _reads) = db();
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
            c.execute("DELETE FROM job", [])
        })
        .unwrap();
    println!("built 100,000 files in {:?}", started.elapsed());

    let time = |what: &str, check: fn(&rusqlite::Connection) -> rusqlite::Result<bool>| {
        let started = Instant::now();
        let due = writer.call(move |c| check(c)).unwrap();
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
    let asked = ask_what_is_due(&writer, &enqueue, &reach_by(move || volume.clone()));
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
