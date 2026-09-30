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
