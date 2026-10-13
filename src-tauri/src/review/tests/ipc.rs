//! The three commands, called the way the frontend calls them.

use serde_json::json;
use tauri::Manager;

use crate::db::Writer;
use crate::ipc::testing::{app, invoke};

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
    app.state::<Writer>()
        .call(|c| {
            c.execute_batch(
                "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES ('serial=NTFS-1A2B3C4D', 'external', 'Q:\\');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');
                 INSERT INTO recording (title) VALUES ('Synthetic Tune');
                 INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, sniffed_format)
                     VALUES (1, 'gone.mp3', 'gone.mp3', 0, 'mp3'),
                            (1, 'copy.mp3', 'copy.mp3', 1, 'mp3');
                 INSERT INTO recording_file (recording_id, file_id)
                     VALUES (1, 1), (1, 2);
                 INSERT INTO library_track (recording_id, kind, linked_file_id)
                     VALUES (1, 'linked', 1);
                 INSERT INTO better_file (recording_id, file_id, step) VALUES (1, 2, 'onDisk');",
            )
        })
        .unwrap();

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
