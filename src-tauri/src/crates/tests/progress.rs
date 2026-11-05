//! Progress of an add of tracks from outside the Library (1cE-P2): the
//! sink answers while the add holds the writer, follows the add through its
//! two phases, starts over for the next add, and costs the add nothing it
//! promised (one operation, one Undo).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::crates::add_progress::{AddPhase, AddProgress, AddProgressState};

/// Longer than every wait here needs, so a hang is a failure, not a freeze.
const PATIENCE: Duration = Duration::from_secs(60);

/// More tracks than a publish covers, so the loops publish more than once.
const MANY: i64 = 600;

/// A Library with `count` tracks outside it (ids 1 to `count`), each with a
/// file on disk.
fn lib_with_outside(count: i64) -> Lib {
    let lib = Lib::new();
    lib.writer
        .call(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES ('serial=TEST', 'external', 'E:\\');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
                 INSERT INTO recording (title) SELECT 'Out ' || i FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
                 INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
                     SELECT 1, i || '.mp3', i || '.mp3', 1 FROM n;
                 INSERT INTO recording_file (recording_id, file_id, role)
                     SELECT file.id, file.id, 'best' FROM file;"
            ))
        })
        .unwrap();
    lib
}

fn state(phase: AddPhase, done: u32, total: u32) -> AddProgressState {
    AddProgressState { phase, done, total }
}

#[test]
fn the_progress_answers_while_an_add_holds_the_writer_and_reports_its_phase_and_count() {
    let lib = lib_with_outside(MANY);
    let id = lib.made("Big");
    let progress = AddProgress::default();
    let tracking = progress.begin();

    // The add stops at its first publish of the writing phase and waits.
    let (entered, inside) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let stopped = AtomicBool::new(false);
    progress.on_publish({
        let seen = seen.clone();
        let entered = Mutex::new(entered);
        move |now| {
            seen.lock().unwrap().push(now);
            if now.phase == AddPhase::Adding && !stopped.swap(true, Ordering::SeqCst) {
                entered.lock().unwrap().send(()).unwrap();
                let _ = released.lock().unwrap().recv_timeout(PATIENCE);
            }
        }
    });
    let recordings: Vec<i64> = (1..=MANY).collect();
    let adding = std::thread::spawn({
        let (writer, progress) = (lib.writer.clone(), progress.clone());
        move || writer.call(move |c| Ok(add_recordings_tracked(c, id, &recordings, &progress)))
    });
    inside.recv_timeout(PATIENCE).unwrap();

    // The writer is inside the add right now. The reading doesn't need it.
    assert_eq!(progress.read(), state(AddPhase::Adding, 0, MANY as u32));
    assert!(!adding.is_finished());

    release.send(()).unwrap();
    let done = adding.join().unwrap().unwrap().unwrap();
    assert_eq!(done.unwrap().added, MANY as u32);
    drop(tracking);

    // The count reached the total in each phase, in order, and the add is over.
    let seen = seen.lock().unwrap().clone();
    let total = MANY as u32;
    assert_eq!(
        seen.first(),
        Some(&state(AddPhase::Checking, 0, MANY as u32))
    );
    assert!(seen.contains(&state(AddPhase::Checking, 0, total)));
    assert!(seen.contains(&state(AddPhase::Checking, total, total)));
    assert!(seen.contains(&state(AddPhase::Adding, 0, total)));
    assert!(seen.contains(&state(AddPhase::Adding, total, total)));
    let first_adding = seen
        .iter()
        .position(|s| s.phase == AddPhase::Adding)
        .unwrap();
    assert!(seen[..first_adding]
        .iter()
        .all(|s| s.phase != AddPhase::Done));
    assert!(
        seen.iter().any(|s| s.done > 0 && s.done < total),
        "in between too"
    );
    assert_eq!(progress.read(), state(AddPhase::Done, total, total));
}

#[test]
fn a_new_add_starts_the_progress_over() {
    let lib = lib_with_outside(MANY);
    let id = lib.made("Big");
    let progress = AddProgress::default();
    let tracking = progress.begin();
    let all: Vec<i64> = (1..=MANY).collect();
    lib.writer
        .call({
            let progress = progress.clone();
            move |c| Ok(add_recordings_tracked(c, id, &all, &progress))
        })
        .unwrap()
        .unwrap()
        .unwrap();
    drop(tracking);
    assert_eq!(
        progress.read(),
        state(AddPhase::Done, MANY as u32, MANY as u32)
    );

    // The next add begins: nothing of the last one shows, even before its
    // work has started (it may be waiting behind another write).
    let tracking = progress.begin();
    assert_eq!(progress.read(), state(AddPhase::Idle, 0, 0));
    lib.writer
        .call({
            let progress = progress.clone();
            move |c| {
                Ok(add_recordings_to_new_crate_tracked(
                    c,
                    "Small",
                    &[1, 2, 3],
                    &progress,
                ))
            }
        })
        .unwrap()
        .unwrap()
        .unwrap();
    drop(tracking);
    assert_eq!(progress.read(), state(AddPhase::Done, 3, 3));
}

#[test]
fn an_add_that_is_refused_leaves_the_progress_done_not_stuck_and_records_nothing() {
    let lib = lib_with_outside(3);
    let id = lib.made("Small");
    let before = lib.operations();
    let progress = AddProgress::default();
    let tracking = progress.begin();
    // A track that doesn't exist refuses the whole call before any write.
    let refused = lib
        .writer
        .call({
            let progress = progress.clone();
            move |c| Ok(add_recordings_tracked(c, id, &[1, 99_999], &progress))
        })
        .unwrap();
    assert!(refused.unwrap().is_err());
    drop(tracking);
    assert_eq!(progress.read().phase, AddPhase::Done);
    assert_eq!(lib.operations(), before);
}

#[test]
fn undoing_a_large_add_is_still_one_operation_and_one_undo() {
    let lib = lib_with_outside(MANY);
    let id = lib.made("Big");
    let before = lib.everything();
    let progress = AddProgress::default();

    let all: Vec<i64> = (1..=MANY).collect();
    let done = lib
        .writer
        .call(move |c| Ok(add_recordings_tracked(c, id, &all, &progress)))
        .unwrap()
        .unwrap();

    assert_eq!(done.unwrap().added, MANY as u32);
    assert_eq!(lib.operations(), before.2 + 1);
    lib.undo();
    let after = lib.everything();
    assert_eq!((after.0, after.1), (before.0, before.1));
}

mod ipc {
    use serde_json::json;
    use tauri::Manager;

    use crate::crates::add_progress::AddProgress;
    use crate::db::Writer;
    use crate::ipc::testing::{app, invoke};

    #[test]
    fn the_frontend_reads_the_progress_of_an_add_over_ipc() {
        let (_data, app) = app();
        assert_eq!(
            invoke(&app, "crate_add_progress", json!({})).unwrap(),
            json!({ "phase": "idle", "done": 0, "total": 0 })
        );
        let recording = app
            .state::<Writer>()
            .call(|c| {
                c.execute("INSERT INTO recording (title) VALUES ('Tune')", [])?;
                let recording = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO library_track (recording_id, source_status) VALUES (?1, 'missing')",
                    [recording],
                )?;
                Ok(recording)
            })
            .unwrap();

        let made = invoke(
            &app,
            "add_recordings_to_new_crate",
            json!({ "name": "Warm up", "recordings": [recording] }),
        )
        .unwrap();

        assert_eq!(made["added"]["added"], json!(1));
        assert_eq!(
            invoke(&app, "crate_add_progress", json!({})).unwrap(),
            json!({ "phase": "done", "done": 1, "total": 1 })
        );
        assert_eq!(app.state::<AddProgress>().read().total, 1);
    }
}
