#![cfg(test)]
//! 1cA-13: what the startup step queues when work is due and no job is.
//!
//! Each test builds a library the real chain has been through, puts the
//! database as a run that died (or an upgrade) would leave it, and asks
//! [`ask_what_is_due`] with an `enqueue` that only stores the row, so the
//! test sees exactly what was queued.

use super::chain::{chained_queue, jobs, wait_idle, LookedAt};
use super::support::{db, TempVolume};
use super::walk::{add_music, drive, put};
use crate::db::{DbError, Writer};
use crate::jobs::startup::ask_what_is_due;
use crate::jobs::{JobId, NewJob};
use crate::scan::chain::reach_by;
use crate::scan::walk::scan_job;
use crate::tags::test_audio as audio;

/// A music folder with a file, scanned through the whole chain; the
/// queue is stopped, so nothing runs while a test looks.
fn scanned() -> (tempfile::TempDir, TempVolume, tempfile::TempDir, Writer) {
    let (dir, volume, music) = drive();
    put(&music, "a.mp3", &audio::mp3());
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
    let failed = ask_what_is_due(writer, &enqueue, &reach_by(move || volume.clone()));
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
                "UPDATE file_stage SET version = 0 WHERE stage = ?1",
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
    // Deciding waits for matching, as in the chain.
    assert_eq!((failed, kinds(&queued)), (0, vec!["group", "match"]));
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
    assert_eq!((failed, kinds(&queued)), (0, Vec::<&str>::new()));

    // Plugged back in, the same database asks for the hashes.
    volume.set_online(true);
    let (_, queued) = ask(&writer, &volume);
    assert_eq!(kinds(&queued), ["hash"]);
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
    // Two stages have work: grouping and matching.
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
    let failed = ask_what_is_due(&writer, &enqueue, &reach_by(move || reach_volume.clone()));
    assert_eq!(failed, 2);
    assert_eq!(
        tried.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the second stage was asked though the first failed"
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
