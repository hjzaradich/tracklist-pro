//! The app killed mid-job, and started again (1aG-10).
//!
//! A kill, a crash or a dead battery leaves the job that was running as
//! `running` in the database. These tests put such rows there the only
//! way a test can (the app is stopped, then the rows are written as the
//! dead run would have left them) and start the app again on that data
//! folder.

use tracklist_pro_lib::jobs::JobStatus;

use super::fixtures::{KNOWN, UNKNOWN};
use super::harness::World;

/// A scanned music folder of two files, then a third the user adds while
/// the app is about to die.
fn scanned_then_a_file_added() -> World {
    let mut world = World::new(&KNOWN[..2]);
    world.add_music_folder_and_scan();
    assert_eq!(world.all_music().total, 2);
    world.user_adds(&UNKNOWN[0]);
    world
}

#[test]
fn scan_stages_left_running_by_a_killed_app_run_again_at_the_next_start_and_the_chain_completes() {
    let mut world = scanned_then_a_file_added();
    let before = world.jobs().len();

    // Each scan stage, with the target the chain gives it, left running.
    world.kill_and_start_again(|c| {
        c.execute(
            "INSERT INTO job (kind, target, priority, status, started_at, progress, attempts)
             SELECT kind, target, max(priority), 'running',
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 0.5, 1
             FROM job
             WHERE kind IN ('scan', 'read', 'hash', 'fingerprint', 'group', 'relink')
             GROUP BY kind, target",
            [],
        )
        .unwrap();
    });
    world.settle();

    // Nothing was asked of the app after it started: the jobs the dead run
    // left ran again from the top, the walk among them found the new file,
    // and the stages after it took it all the way to a track.
    let jobs = world.jobs();
    let left_running = &jobs[before..];
    assert!(left_running.len() >= 5, "{left_running:?}");
    for (id, kind, _) in left_running.iter().take(5) {
        assert_eq!(world.job(*id).status, JobStatus::Done, "{kind}");
    }
    assert!(
        jobs.iter()
            .all(|(_, _, status)| status != "running" && status != "queued"),
        "{jobs:?}"
    );
    assert_eq!(world.all_music().total, 3);
}

#[test]
fn a_job_nothing_can_restart_ends_failed_a_send_job_is_dropped_and_neither_blocks_a_scan() {
    let mut world = scanned_then_a_file_added();
    let before = world.jobs().len();

    world.kill_and_start_again(|c| {
        c.execute_batch(
            r#"INSERT INTO job (kind, status, started_at, attempts) VALUES
                 ('analyze', 'running', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 1),
                 ('made_up', 'running', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 1);
               INSERT INTO job (kind, target, status, started_at, attempts) VALUES
                 ('export', '{"send":"write","run":"the dead run","token":"t","confirmed":false}',
                  'running', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 1);"#,
        )
        .unwrap();
    });
    world.settle();

    let jobs = world.jobs();
    let ended: Vec<(&str, &str)> = jobs[before..before + 3]
        .iter()
        .map(|(_, kind, status)| (kind.as_str(), status.as_str()))
        .collect();
    assert_eq!(
        ended,
        [
            ("analyze", "failed"),
            ("made_up", "failed"),
            // A send only ever follows a click in this run.
            ("export", "cancelled"),
        ]
    );
    assert!(!world.sent_file().exists(), "no send was written");

    // And the scan goes all the way through.
    world.scan();
    assert_eq!(world.all_music().total, 3);
}
