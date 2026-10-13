//! The three commands, called the way the frontend calls them.

use serde_json::json;
use tauri::Manager;

use crate::db::Writer;
use crate::ipc::testing::{app, invoke};

/// A Library track (track 1) whose linked file is gone, with a copy on
/// disk in the same track.
const GONE_WITH_A_COPY: &str = "
    INSERT INTO volume (identity, kind, last_mount_path)
        VALUES ('serial=NTFS-1A2B3C4D', 'external', 'Q:\\');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
    INSERT INTO recording (title) VALUES ('Synthetic Tune');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, sniffed_format, codec,
                      bitrate, audio_hash)
        VALUES (1, 'gone.mp3', 'gone.mp3', 0, 'mp3', 'mp3', 320, x'0101'),
               (1, 'copy.mp3', 'copy.mp3', 1, 'mp3', 'mp3', 320, x'0202');
    INSERT INTO recording_file (recording_id, file_id) VALUES (1, 1), (1, 2);
    INSERT INTO library_track (recording_id, kind, linked_file_id) VALUES (1, 'linked', 1);";

/// Writes a library by SQL and leaves it as the background jobs leave
/// one: every file placed in its track by grouping, a grouping pass run,
/// and every track's files ranked (which writes the "better file" rows and
/// the quality verdicts, from the app's own rules).
///
/// The app asks at startup what is due, on a thread of its own (1cA-13), so
/// its jobs can run before or after a test writes its rows. Here the rows
/// and the jobs' work are one writer call, so no job can run in between,
/// and one that runs later finds nothing left to do: the test reads the
/// same thing either way.
fn library(app: &tauri::App<tauri::test::MockRuntime>, sql: &'static str) {
    app.state::<Writer>()
        .call(move |c| {
            c.execute_batch(sql)?;
            c.execute(
                "INSERT INTO grouping_file (file_id, placed_key, decided_as)
                 SELECT id, coalesce(nullif(audio_hash, x''), blake3), ?1 FROM file",
                [crate::grouping::decided_as()],
            )?;
            crate::grouping::regroup(c)?;
            c.execute(
                "INSERT OR IGNORE INTO best_file_due (recording_id) SELECT id FROM recording",
                [],
            )?;
            while crate::file_roles::settle_batch(c, 100)?.more {}
            Ok(())
        })
        .unwrap();
}

#[test]
fn a_fresh_library_has_an_empty_queue_over_ipc() {
    let (_data, app) = app();
    assert_eq!(
        invoke(&app, "review_counts", json!({})),
        Ok(json!({
            "needsCloserLook": 0,
            "clearBestFile": 0,
            "qualityProblems": 0,
            "total": 0
        }))
    );
    let page = invoke(
        &app,
        "review_items",
        json!({ "group": "needsCloserLook", "offset": 0, "limit": 10 }),
    )
    .unwrap();
    assert_eq!(page["total"], json!(0));
    assert_eq!(page["items"], json!([]));
    assert_eq!(
        invoke(
            &app,
            "review_item",
            json!({ "id": { "kind": "relink", "a": 1, "b": null } })
        ),
        Ok(json!(null))
    );
}

#[test]
fn an_item_and_its_comparison_reach_the_frontend_with_a_suggestion_and_a_reason_code() {
    let (_data, app) = app();
    library(&app, GONE_WITH_A_COPY);

    let counts = invoke(&app, "review_counts", json!({})).unwrap();
    assert_eq!(counts["clearBestFile"], json!(1));
    assert_eq!(counts["total"], json!(1));
    let page = invoke(
        &app,
        "review_items",
        json!({ "group": "clearBestFile", "offset": 0, "limit": 10 }),
    )
    .unwrap();
    let item = &page["items"][0];
    assert_eq!(
        item["id"],
        json!({ "kind": "clearBestFile", "a": 1, "b": null })
    );
    assert_eq!(item["tracks"][0]["title"], json!("Synthetic Tune"));

    let comparison = invoke(&app, "review_item", json!({ "id": item["id"] })).unwrap();
    assert_eq!(comparison["group"], json!("clearBestFile"));
    assert_eq!(comparison["suggestion"]["what"], json!(2));
    assert_eq!(
        comparison["suggestion"]["reasons"][0]["key"],
        json!("review:reason.notOnDisk")
    );
    assert_eq!(comparison["rows"][0]["fileId"], json!(2));
    assert_eq!(comparison["rows"][1]["linked"], json!(true));
}

fn counts(app: &tauri::App<tauri::test::MockRuntime>) -> serde_json::Value {
    invoke(app, "review_counts", json!({})).unwrap()
}

#[test]
fn the_switch_its_undo_and_accept_all_are_called_the_way_the_frontend_calls_them() {
    let (_data, app) = app();
    library(&app, GONE_WITH_A_COPY);
    let item = json!({ "kind": "clearBestFile", "a": 1, "b": null });

    // The item carries the switch it offers, as codes.
    let comparison = invoke(&app, "review_item", json!({ "id": item })).unwrap();
    assert_eq!(
        comparison["switch"],
        json!({ "fileId": 2, "effect": "notInRekordbox", "refusal": null })
    );
    // What "Accept all" would switch: its button counts these.
    let clear = invoke(&app, "review_accept_all_items", json!({})).unwrap();
    assert_eq!(clear, json!([{ "recordingId": 1, "fileId": 2 }]));

    // A switch that no longer means what the item showed is refused with
    // its own kind, and nothing changes.
    let stale = invoke(
        &app,
        "review_switch_linked_file",
        json!({ "recordingId": 1, "fileId": 2, "effect": "keepsEntry" }),
    );
    assert_eq!(stale.unwrap_err()["kind"], json!("reviewSwitchChanged"));
    assert_eq!(counts(&app)["clearBestFile"], json!(1));

    let switched = invoke(
        &app,
        "review_switch_linked_file",
        json!({ "recordingId": 1, "fileId": 2, "effect": "notInRekordbox" }),
    )
    .unwrap();
    assert_eq!(switched["track"]["fromFileId"], json!(1));
    assert_eq!(switched["track"]["toFileId"], json!(2));
    assert_eq!(switched["track"]["effect"], json!("notInRekordbox"));
    assert_eq!(counts(&app)["total"], json!(0));
    assert_eq!(
        invoke(&app, "review_item", json!({ "id": item })),
        Ok(json!(null))
    );
    // The Undo button names it by its kind.
    let next = invoke(&app, "next_undo_operation", json!({})).unwrap();
    assert_eq!(next["operation"]["kind"], json!("switch_linked_file"));

    // Undo, by the operation the switch returned: the item is back.
    let undone = invoke(
        &app,
        "undo_last_operation",
        json!({ "operationId": switched["operationId"] }),
    )
    .unwrap();
    assert_eq!(undone["status"], json!("undone"));
    assert_eq!(counts(&app)["clearBestFile"], json!(1));

    // "Accept all": the same switch, and what it switched.
    // "Accept all" takes the items the screen showed, and only those.
    let none = invoke(
        &app,
        "review_accept_clear_best_files",
        json!({ "items": [] }),
    )
    .unwrap();
    assert_eq!(none["switched"], json!([]));
    assert_eq!(counts(&app)["clearBestFile"], json!(1));
    let accepted = invoke(
        &app,
        "review_accept_clear_best_files",
        json!({ "items": clear }),
    )
    .unwrap();
    assert_eq!(
        accepted["switched"],
        json!([{
            "recordingId": 1, "libraryTrackId": 1, "fromFileId": 1, "toFileId": 2,
            "effect": "notInRekordbox"
        }])
    );
    assert_eq!(accepted["skipped"], json!([]));
    assert_eq!(counts(&app)["total"], json!(0));
    let next = invoke(&app, "next_undo_operation", json!({})).unwrap();
    assert_eq!(next["operation"]["kind"], json!("accept_clear_best_files"));
    assert_eq!(next["operation"]["details"]["tracks"], json!(1));

    // A switch for a track whose linked file is on disk is refused with
    // its reason.
    let again = invoke(
        &app,
        "review_switch_linked_file",
        json!({ "recordingId": 1, "fileId": 1, "effect": "notInRekordbox" }),
    );
    assert_eq!(
        again.unwrap_err()["kind"],
        json!("reviewSwitchLinkedOnDisk")
    );
}

#[test]
fn the_remembered_decisions_and_their_undo_are_called_the_way_the_frontend_calls_them() {
    let (_data, app) = app();
    // Track 1: linked to an mp3 with a better FLAC. Track 2: linked to a
    // file the quality rules flag (128 kbps is a low bitrate).
    library(
        &app,
        "INSERT INTO volume (identity, kind, last_mount_path)
             VALUES ('serial=NTFS-1A2B3C4D', 'external', 'Q:\\');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
             VALUES (1, 'Music', 'Music');
         INSERT INTO recording (title) VALUES ('Synthetic Tune'), ('Synthetic Other');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key, sniffed_format, codec,
                           bitrate, audio_hash)
             VALUES (1, 'a.mp3', 'a.mp3', 'mp3', 'mp3', 320, x'0101'),
                    (1, 'a.flac', 'a.flac', 'flac', 'flac', NULL, x'0202'),
                    (1, 'b.mp3', 'b.mp3', 'mp3', 'mp3', 128, x'0303');
         INSERT INTO recording_file (recording_id, file_id) VALUES (1, 1), (1, 2), (2, 3);
         INSERT INTO library_track (recording_id, kind, linked_file_id)
             VALUES (1, 'linked', 1), (2, 'linked', 3);",
    );
    assert_eq!(counts(&app)["qualityProblems"], json!(2));

    let dismissed = invoke(
        &app,
        "review_dismiss_better_file",
        json!({ "recordingId": 1, "fileId": 2 }),
    )
    .unwrap();
    assert!(dismissed["operationId"].is_i64());
    assert_eq!(counts(&app)["qualityProblems"], json!(1));
    // Answered already: its own error kind, never text.
    let again = invoke(
        &app,
        "review_dismiss_better_file",
        json!({ "recordingId": 1, "fileId": 2 }),
    );
    assert_eq!(
        again,
        Err(json!({ "kind": "reviewItemGone", "params": {} }))
    );

    let kept = invoke(&app, "review_keep_anyway", json!({ "recordingId": 2 })).unwrap();
    assert_eq!(counts(&app)["total"], json!(0));
    let undone = invoke(
        &app,
        "undo_last_operation",
        json!({ "operationId": kept["operationId"] }),
    )
    .unwrap();
    assert_eq!(undone["status"], json!("undone"));
    assert_eq!(undone["operation"]["kind"], json!("keep_anyway"));
    assert_eq!(counts(&app)["qualityProblems"], json!(1));

    invoke(&app, "review_want_better_file", json!({ "recordingId": 2 })).unwrap();
    assert_eq!(counts(&app)["total"], json!(0));
    let next = invoke(&app, "next_undo_operation", json!({})).unwrap();
    assert_eq!(next["operation"]["kind"], json!("want_better_file"));
}
