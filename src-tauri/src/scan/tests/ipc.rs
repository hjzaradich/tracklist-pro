#![cfg(test)]
//! The music folder and scan commands, called over IPC as the frontend
//! calls them, on the real volume lookup and the app's own job queue.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::Manager;

use crate::ipc::testing::{app, invoke};
use crate::ipc::{ErrorKind, IpcError};
use crate::jobs;
use crate::scan::MusicFolderError;

#[test]
fn the_frontend_adds_scans_lists_and_removes_a_music_folder() {
    let (_data, app) = app();

    let drive = tempfile::tempdir().unwrap();
    let music = std::fs::canonicalize(drive.path()).unwrap().join("Music");
    std::fs::create_dir_all(music.join("House")).unwrap();
    std::fs::write(music.join("House").join("a.mp3"), b"a").unwrap();
    let path = crate::scan::display_path(&music);

    let added = invoke(
        &app,
        "add_music_folder",
        json!({ "path": path, "role": null }),
    )
    .unwrap();
    assert_eq!(added["path"], json!(path));
    assert_eq!(added["role"], json!("scan"));
    assert_eq!(added["online"], json!(true));
    let id = added["id"].clone();

    // A refusal arrives as a kind and its message's parameters.
    let inside = format!(r"{path}\House");
    assert_eq!(
        invoke(
            &app,
            "add_music_folder",
            json!({ "path": inside, "role": "inbox" })
        ),
        Err(json!({ "kind": "insideMusicFolder", "params": { "musicFolder": path } }))
    );

    // The scan runs on the app's own queue with the real walker.
    let job = invoke(&app, "scan_music_folders", json!({ "ids": [id] })).unwrap();
    let job = jobs::JobId(job.as_i64().unwrap());
    let writer = app.state::<crate::db::Writer>();
    let start = Instant::now();
    loop {
        let status = writer
            .call(move |c| jobs::store::get(c, job))
            .unwrap()
            .unwrap()
            .status;
        if status.is_finished() {
            assert_eq!(status, jobs::JobStatus::Done);
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(10));
    }
    let files: i64 = writer
        .call(|c| c.query_row("SELECT count(*) FROM file", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(files, 1);

    // Listed as added, plus what the walk found: it read everything.
    let listed = invoke(&app, "music_folders", json!({})).unwrap();
    let walked_at = listed[0]["walkedAt"].clone();
    assert!(walked_at.is_string(), "{listed}");
    let mut expected = added.clone();
    assert_eq!(expected["walkedAt"], json!(null));
    assert_eq!(expected["unreadableFolders"], json!(null));
    expected["walkedAt"] = walked_at;
    expected["unreadableFolders"] = json!(0);
    expected["unreadableFiles"] = json!(0);
    assert_eq!(expected["onlineOnlyFiles"], json!(0));
    assert_eq!(listed, json!([expected]));

    assert_eq!(
        invoke(&app, "remove_music_folder", json!({ "id": id })),
        Ok(Value::Null)
    );
    assert_eq!(invoke(&app, "music_folders", json!({})), Ok(json!([])));
    assert_eq!(
        invoke(&app, "remove_music_folder", json!({ "id": id })),
        Err(json!({ "kind": "musicFolderNotFound", "params": {} }))
    );
    // The file on disk is untouched.
    assert_eq!(
        std::fs::read(music.join("House").join("a.mp3")).unwrap(),
        b"a"
    );
}

#[test]
fn every_music_folder_error_becomes_its_kind_with_exactly_its_messages_parameters() {
    let cases = [
        (
            MusicFolderError::NotAFolder {
                path: "E:\\Nope".into(),
            },
            json!({ "kind": "notAFolder", "params": { "path": "E:\\Nope" } }),
        ),
        (
            MusicFolderError::BadPath {
                path: "Music".into(),
            },
            json!({ "kind": "badPath", "params": { "path": "Music" } }),
        ),
        (
            MusicFolderError::AlreadyAdded {
                path: "E:\\Music".into(),
            },
            json!({ "kind": "alreadyAdded", "params": { "path": "E:\\Music" } }),
        ),
        (
            MusicFolderError::InsideMusicFolder {
                path: "E:\\Music\\House".into(),
                music_folder: "E:\\Music".into(),
            },
            json!({ "kind": "insideMusicFolder", "params": { "musicFolder": "E:\\Music" } }),
        ),
        (
            MusicFolderError::ContainsMusicFolder {
                path: "E:\\".into(),
                music_folder: "E:\\Music".into(),
            },
            json!({ "kind": "containsMusicFolder", "params": { "musicFolder": "E:\\Music" } }),
        ),
        (
            MusicFolderError::NotFound,
            json!({ "kind": "musicFolderNotFound", "params": {} }),
        ),
        (
            MusicFolderError::InUse,
            json!({ "kind": "musicFolderInUse", "params": {} }),
        ),
    ];
    for (error, sent) in cases {
        let ipc = IpcError::from(error.clone());
        assert_eq!(serde_json::to_value(&ipc).unwrap(), sent, "{error:?}");
        // Each key lives in the music folders' own namespace.
        assert!(ipc.kind().key().starts_with("musicFolders:"), "{error:?}");
    }
}

#[test]
fn a_database_failure_in_a_music_folder_command_is_shown_as_the_shared_database_error() {
    let (_data, app) = app();
    app.state::<crate::db::Writer>()
        .call(|c| c.execute_batch("ALTER TABLE music_folder RENAME TO gone"))
        .unwrap();
    for (cmd, args) in [
        ("music_folders", json!({})),
        ("remove_music_folder", json!({ "id": 1 })),
    ] {
        let error = invoke(&app, cmd, args).unwrap_err();
        assert_eq!(error, json!({ "kind": "database", "params": {} }), "{cmd}");
        assert!(
            !error.to_string().contains("music_folder"),
            "{cmd}: {error}"
        );
    }
    // A full disk, say, keeps its own message rather than a vague one.
    let full =
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL), None);
    let error = MusicFolderError::from(crate::db::DbError::Sqlite(full));
    assert_eq!(IpcError::from(error).kind(), ErrorKind::DiskFull);
}

/// The scan jobs so far: each one's target and priority, oldest first.
fn scans(app: &tauri::App<tauri::test::MockRuntime>) -> Vec<(Option<String>, i64)> {
    app.state::<crate::db::Writer>()
        .call(|c| {
            let mut stmt =
                c.prepare("SELECT target, priority FROM job WHERE kind = 'scan' ORDER BY id")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap()
}

fn a_music_folder() -> (tempfile::TempDir, String) {
    let drive = tempfile::tempdir().unwrap();
    let music = std::fs::canonicalize(drive.path()).unwrap().join("Music");
    std::fs::create_dir_all(&music).unwrap();
    std::fs::write(music.join("a.mp3"), b"a").unwrap();
    let path = crate::scan::display_path(&music);
    (drive, path)
}

#[test]
fn adding_a_music_folder_queues_exactly_one_scan_of_it_at_the_users_priority() {
    let (_data, app) = app();
    // The watchers have had their first look (no folder yet).
    app.state::<crate::scan::Watchers>().status();
    let (_drive, path) = a_music_folder();

    let added = invoke(
        &app,
        "add_music_folder",
        json!({ "path": path, "role": null }),
    )
    .unwrap();
    // The watchers have looked at the new folder too, and scanned nothing
    // more for it.
    app.state::<crate::scan::Watchers>().status();
    let target = json!({ "music_folder_ids": [added["id"]] }).to_string();
    assert_eq!(
        scans(&app),
        [(Some(target), jobs::Priority::USER.0)],
        "one scan, as asked for by the user"
    );
}

#[test]
fn adding_a_music_folder_scans_it_even_when_the_watchers_are_not_running() {
    let (_data, app) = app();
    // As when the watch thread couldn't start: nothing there will ever
    // queue a scan.
    app.state::<crate::scan::Watchers>().shutdown();
    let (_drive, path) = a_music_folder();

    let added = invoke(
        &app,
        "add_music_folder",
        json!({ "path": path, "role": null }),
    )
    .unwrap();
    let target = json!({ "music_folder_ids": [added["id"]] }).to_string();
    assert_eq!(scans(&app), [(Some(target), jobs::Priority::USER.0)]);
}

#[test]
fn asking_for_the_same_scan_twice_while_it_waits_queues_it_once() {
    let (_data, app) = app();
    // The workers are stopped, so a queued scan stays queued: the state a
    // double click meets.
    app.state::<jobs::JobQueue>().shutdown();

    let first = invoke(&app, "scan_music_folders", json!({ "ids": [1] })).unwrap();
    let second = invoke(&app, "scan_music_folders", json!({ "ids": [1] })).unwrap();
    assert_eq!(first, second, "the second click gets the waiting job");
    assert_eq!(scans(&app).len(), 1);

    // Another folder's scan is its own job.
    let other = invoke(&app, "scan_music_folders", json!({ "ids": [2] })).unwrap();
    assert_ne!(other, first);
    assert_eq!(scans(&app).len(), 2);
}

#[test]
fn a_scan_the_user_asks_for_lifts_the_same_scan_waiting_in_the_background() {
    let (_data, app) = app();
    app.state::<jobs::JobQueue>().shutdown();
    // What a watcher queues: the same scan, at background priority, with
    // another background job ahead of it.
    let queue = app.state::<jobs::JobQueue>();
    let ahead = queue
        .enqueue(jobs::NewJob::new(jobs::JobKind::Hash).priority(jobs::Priority::BACKGROUND))
        .unwrap();
    let waiting = queue
        .enqueue(
            crate::scan::scan_job(Some(vec![crate::scan::MusicFolderId(1)]))
                .priority(jobs::Priority::BACKGROUND),
        )
        .unwrap();
    assert!(ahead < waiting);

    let asked = invoke(&app, "scan_music_folders", json!({ "ids": [1] })).unwrap();
    assert_eq!(asked, json!(waiting.0), "no second scan is queued");
    assert_eq!(scans(&app).len(), 1);
    assert_eq!(scans(&app)[0].1, jobs::Priority::USER.0);

    // A background request for it afterwards doesn't take it back down.
    let writer = app.state::<crate::db::Writer>();
    let again = crate::scan::chain::queue_once(
        &writer,
        crate::scan::scan_job(Some(vec![crate::scan::MusicFolderId(1)]))
            .priority(jobs::Priority::BACKGROUND),
        |job| queue.enqueue(job),
    )
    .unwrap();
    assert_eq!(again, waiting);
    assert_eq!(scans(&app)[0].1, jobs::Priority::USER.0);
}
