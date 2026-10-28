//! The commands Review's Conflicts group calls, called the way the frontend
//! calls them. Nothing in production opens a conflict yet (1cB-3), so each
//! test writes its conflicts by SQL. Every name here is invented.

use serde_json::{json, Value};
use tauri::Manager;

use crate::db::Writer;
use crate::ipc::testing::{app, invoke};
use crate::ops::UndoOutcome;

type App = tauri::App<tauri::test::MockRuntime>;

/// Three Library tracks (track 1 linked to a file, the others missing) and
/// open conflicts: tracks 1 to 3 differ on `Rating` (track 2's was never
/// sent), track 1 on `Genre`.
const LIBRARY: &str = "
    INSERT INTO volume (identity, kind, last_mount_path)
        VALUES ('serial=NTFS-1A2B3C4D', 'external', 'Q:\\');
    INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music');
    INSERT INTO recording (title, artist) VALUES
        ('Synthetic Tune', 'Test Artist'), ('Second Tune', NULL), ('Third Tune', 'Third Artist');
    INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, sniffed_format, codec,
                      bitrate, audio_hash)
        VALUES (1, 'sets/synthetic tune.mp3', 'sets/synthetic tune.mp3', 1, 'mp3', 'mp3', 320,
                x'0101');
    INSERT INTO library_track (recording_id, kind, linked_file_id, source_status) VALUES
        (1, 'linked', 1, 'ok'), (2, 'linked', NULL, 'missing'), (3, 'linked', NULL, 'missing');
    INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value) VALUES
        (1, 'Rating', '153', '204', '102'),
        (2, 'Rating', '51', '255', NULL),
        (3, 'Rating', '0', '102', '255'),
        (1, 'Genre', 'House', 'Techno', 'Disco');";

fn library() -> (tempfile::TempDir, App) {
    let (data, app) = app();
    app.state::<Writer>()
        .call(|c| c.execute_batch(LIBRARY))
        .unwrap();
    (data, app)
}

fn page(app: &App, field: &str) -> Value {
    invoke(
        app,
        "review_conflicts",
        json!({ "field": field, "offset": 0, "limit": 100 }),
    )
    .unwrap()
}

fn id_of(page: &Value, n: usize) -> Value {
    page["items"][n]["id"].clone()
}

/// What the `conflict` table holds for one open or settled row.
fn row(app: &App, sql: &'static str) -> (String, Option<String>) {
    app.state::<Writer>()
        .call(move |c| c.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?))))
        .unwrap()
}

fn open_count(app: &App) -> i64 {
    app.state::<Writer>()
        .call(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM conflict WHERE status = 'open'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap()
}

fn undo(app: &App) -> UndoOutcome {
    app.state::<Writer>()
        .call(|c| Ok(crate::ops::undo_last(c)))
        .unwrap()
        .unwrap()
}

#[test]
fn a_fresh_library_has_no_conflicts_to_list() {
    let (_data, app) = app();
    assert_eq!(
        invoke(&app, "review_conflict_fields", json!({})),
        Ok(json!([]))
    );
    let page = page(&app, "Rating");
    assert_eq!(page["total"], json!(0));
    assert_eq!(page["items"], json!([]));
}

#[test]
fn conflicts_are_listed_by_field_with_how_many_and_the_kind_of_value() {
    let (_data, app) = library();
    assert_eq!(
        invoke(&app, "review_conflict_fields", json!({})),
        Ok(json!([
            { "field": "Genre", "kind": "text", "count": 1, "crateName": null },
            { "field": "Rating", "kind": "rating", "count": 3, "crateName": null },
        ]))
    );
}

#[test]
fn a_conflict_lists_the_track_and_the_three_values_with_no_base_when_never_sent() {
    let (_data, app) = library();
    let page = page(&app, "Rating");
    assert_eq!(page["total"], json!(3));
    let first = &page["items"][0];
    assert_eq!(first["track"]["title"], json!("Synthetic Tune"));
    assert_eq!(first["track"]["artist"], json!("Test Artist"));
    assert_eq!(first["kind"], json!("rating"));
    assert_eq!(
        (&first["app"], &first["rekordbox"], &first["base"]),
        (&json!("153"), &json!("204"), &json!("102"))
    );
    // Never sent: no base at all, which is not the same as an empty one.
    let never_sent = &page["items"][1];
    assert_eq!(never_sent["track"]["title"], json!("Second Tune"));
    assert_eq!(never_sent["track"]["artist"], json!(null));
    assert_eq!(never_sent["base"], json!(null));
}

#[test]
fn a_page_holds_the_conflicts_after_the_offset_and_says_how_many_there_are_in_all() {
    let (_data, app) = library();
    let page = invoke(
        &app,
        "review_conflicts",
        json!({ "field": "Rating", "offset": 2, "limit": 5 }),
    )
    .unwrap();
    assert_eq!(page["total"], json!(3));
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["track"]["title"], json!("Third Tune"));
}

#[test]
fn keeping_the_apps_value_settles_the_conflict_with_it() {
    let (_data, app) = library();
    let id = id_of(&page(&app, "Rating"), 0);
    let settled = invoke(
        &app,
        "review_resolve_conflict",
        json!({ "id": id, "choice": { "choice": "keepApp" } }),
    )
    .unwrap();
    assert_eq!(settled["status"], json!("keptApp"));
    assert_eq!(settled["value"], json!("153"));
    assert_eq!(page(&app, "Rating")["total"], json!(2));
}

#[test]
fn keeping_rekordboxs_value_settles_the_conflict_with_it() {
    let (_data, app) = library();
    let id = id_of(&page(&app, "Rating"), 0);
    let settled = invoke(
        &app,
        "review_resolve_conflict",
        json!({ "id": id, "choice": { "choice": "keepRekordbox" } }),
    )
    .unwrap();
    assert_eq!(settled["status"], json!("keptRekordbox"));
    assert_eq!(settled["value"], json!("204"));
}

#[test]
fn a_value_given_by_hand_settles_the_conflict_as_edited() {
    let (_data, app) = library();
    let id = id_of(&page(&app, "Rating"), 0);
    let settled = invoke(
        &app,
        "review_resolve_conflict",
        json!({ "id": id, "choice": { "choice": "edit", "value": "255" } }),
    )
    .unwrap();
    assert_eq!(settled["status"], json!("edited"));
    assert_eq!(settled["value"], json!("255"));
    assert_eq!(
        row(
            &app,
            "SELECT status, resolved_value FROM conflict WHERE library_track_id = 1 AND field = 'Rating' AND status <> 'open'"
        ),
        ("edited".to_owned(), Some("255".to_owned()))
    );
}

#[test]
fn a_value_given_by_hand_of_the_wrong_kind_is_refused_and_nothing_changes() {
    let (_data, app) = library();
    let id = id_of(&page(&app, "Rating"), 0);
    // 100 isn't one of rekordbox's six ratings.
    let refused = invoke(
        &app,
        "review_resolve_conflict",
        json!({ "id": id, "choice": { "choice": "edit", "value": "100" } }),
    )
    .unwrap_err();
    assert_eq!(refused["kind"], json!("conflictBadValue"));
    assert_eq!(open_count(&app), 4);
    assert_eq!(page(&app, "Rating")["total"], json!(3));
}

#[test]
fn settling_a_conflict_that_is_not_open_any_more_is_refused() {
    let (_data, app) = library();
    let id = id_of(&page(&app, "Rating"), 0);
    let keep = json!({ "id": id, "choice": { "choice": "keepApp" } });
    invoke(&app, "review_resolve_conflict", keep.clone()).unwrap();
    let refused = invoke(&app, "review_resolve_conflict", keep).unwrap_err();
    assert_eq!(refused["kind"], json!("conflictGone"));
}

#[test]
fn a_bulk_resolve_by_field_settles_every_conflict_of_the_field_as_one_operation_and_one_undo_reopens_them_all(
) {
    let (_data, app) = library();
    let before = page(&app, "Rating");

    let settled = invoke(
        &app,
        "review_resolve_conflicts",
        json!({ "field": "Rating", "side": "rekordbox", "expected": 3 }),
    )
    .unwrap();
    assert_eq!(settled, json!(3));
    // The other field is untouched.
    assert_eq!(page(&app, "Rating")["total"], json!(0));
    assert_eq!(page(&app, "Genre")["total"], json!(1));
    assert_eq!(
        row(
            &app,
            "SELECT status, resolved_value FROM conflict WHERE library_track_id = 3 AND field = 'Rating' AND status <> 'open'"
        ),
        ("kept_rekordbox".to_owned(), None)
    );
    let operations: i64 = app
        .state::<Writer>()
        .call(|c| c.query_row("SELECT COUNT(*) FROM operation", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(operations, 1, "a bulk resolve is one operation");

    assert!(matches!(undo(&app), UndoOutcome::Undone { .. }));
    // All three are open again, with their three values.
    assert_eq!(page(&app, "Rating"), before);
}

#[test]
fn a_bulk_resolve_keeping_the_apps_values_keeps_each_conflicts_own_value() {
    let (_data, app) = library();
    invoke(
        &app,
        "review_resolve_conflicts",
        json!({ "field": "Rating", "side": "app", "expected": 3 }),
    )
    .unwrap();
    let values: Vec<(i64, String)> = app
        .state::<Writer>()
        .call(|c| {
            c.prepare(
                "SELECT library_track_id, app_value FROM conflict
                 WHERE field = 'Rating' AND status = 'kept_app' ORDER BY library_track_id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
        })
        .unwrap();
    assert_eq!(
        values,
        vec![(1, "153".into()), (2, "51".into()), (3, "0".into())]
    );
}

#[test]
fn a_bulk_resolve_of_a_field_with_no_open_conflicts_settles_nothing_and_records_nothing() {
    let (_data, app) = library();
    let settled = invoke(
        &app,
        "review_resolve_conflicts",
        json!({ "field": "Comments", "side": "app", "expected": 0 }),
    )
    .unwrap();
    assert_eq!(settled, json!(0));
    let operations: i64 = app
        .state::<Writer>()
        .call(|c| c.query_row("SELECT COUNT(*) FROM operation", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(operations, 0);
}

/// Runs `sql` on the writer.
fn exec(app: &App, sql: &'static str) {
    app.state::<Writer>()
        .call(move |c| c.execute_batch(sql))
        .unwrap();
}

#[test]
fn a_conflict_carries_the_name_of_its_tracks_linked_file_if_it_has_one() {
    let (_data, app) = library();
    let page = page(&app, "Rating");
    assert_eq!(
        page["items"][0]["track"]["fileName"],
        json!("synthetic tune.mp3")
    );
    assert_eq!(page["items"][1]["track"]["fileName"], json!(null));
}

#[test]
fn open_conflicts_count_in_what_needs_a_decision_one_each() {
    let (_data, app) = library();
    let counts = invoke(&app, "review_counts", json!({})).unwrap();
    assert_eq!(counts["conflicts"], json!(4));
    assert_eq!(counts["total"], json!(4));

    let id = id_of(&page(&app, "Rating"), 0);
    invoke(
        &app,
        "review_resolve_conflict",
        json!({ "id": id, "choice": { "choice": "keepApp" } }),
    )
    .unwrap();
    let counts = invoke(&app, "review_counts", json!({})).unwrap();
    assert_eq!(counts["conflicts"], json!(3));
    assert_eq!(counts["total"], json!(3));
    // A bulk resolve takes them all out of the count.
    invoke(
        &app,
        "review_resolve_conflicts",
        json!({ "field": "Rating", "side": "app", "expected": 2 }),
    )
    .unwrap();
    let counts = invoke(&app, "review_counts", json!({})).unwrap();
    assert_eq!(counts["conflicts"], json!(1));
    assert_eq!(counts["total"], json!(1));
}

#[test]
fn a_bulk_resolve_asked_about_another_number_of_conflicts_than_there_are_settles_nothing() {
    let (_data, app) = library();
    let refused = invoke(
        &app,
        "review_resolve_conflicts",
        json!({ "field": "Rating", "side": "app", "expected": 2 }),
    )
    .unwrap_err();
    assert_eq!(refused["kind"], json!("conflictsChanged"));
    assert_eq!(open_count(&app), 4);
    let operations: i64 = app
        .state::<Writer>()
        .call(|c| c.query_row("SELECT COUNT(*) FROM operation", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(operations, 0);
}

#[test]
fn a_file_location_is_never_given_by_hand() {
    let (_data, app) = library();
    exec(
        &app,
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
         VALUES (1, 'Location', 'file://localhost/D:/a.mp3', 'file://localhost/D:/b.mp3', NULL);",
    );
    let id = id_of(&page(&app, "Location"), 0);
    let refused = invoke(
        &app,
        "review_resolve_conflict",
        json!({ "id": id, "choice": { "choice": "edit", "value": "file://localhost/D:/c.mp3" } }),
    )
    .unwrap_err();
    assert_eq!(refused["kind"], json!("conflictBadValue"));
    assert_eq!(page(&app, "Location")["total"], json!(1));
    // Keeping a side is fine.
    let settled = invoke(
        &app,
        "review_resolve_conflict",
        json!({ "id": id, "choice": { "choice": "keepRekordbox" } }),
    )
    .unwrap();
    assert_eq!(settled["value"], json!("file://localhost/D:/b.mp3"));
}

#[test]
fn a_value_given_by_hand_is_accepted_or_refused_by_the_kind_of_its_field() {
    let (_data, app) = library();
    exec(
        &app,
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value) VALUES
            (1, 'Colour', '0xFF0000', '0x0000FF', '0xFFFFFF'),
            (1, 'Year', '2001', '2002', '2003'),
            (1, 'DateAdded', '2026-01-01', '2026-01-02', '2025-12-31'),
            (1, 'PlayCount', '1', '2', '3'),
            (1, 'Name', 'A', 'B', 'C');",
    );
    // In each field a value that doesn't fit comes first: it must leave the
    // conflict open for the one that does.
    let cases = [
        ("Colour", "green", false),
        ("Colour", "0x00FF00", true),
        ("Year", "20x", false),
        ("Year", "0", true),
        ("DateAdded", "2026-02-30", false),
        ("DateAdded", "2026-09-28", true),
        ("PlayCount", "-1", false),
        ("PlayCount", "12", true),
        ("Name", "Any text at all", true),
    ];
    for (field, value, fits) in cases {
        let id = id_of(&page(&app, field), 0);
        let answer = invoke(
            &app,
            "review_resolve_conflict",
            json!({ "id": id, "choice": { "choice": "edit", "value": value } }),
        );
        if fits {
            let settled = answer.unwrap_or_else(|e| panic!("{field} {value}: {e}"));
            assert_eq!(settled["status"], json!("edited"), "{field} {value}");
            assert_eq!(settled["value"], json!(value), "{field} {value}");
            assert_eq!(page(&app, field)["total"], json!(0), "{field} {value}");
        } else {
            let refused = answer.expect_err(&format!("{field} {value} should be refused"));
            assert_eq!(
                refused["kind"],
                json!("conflictBadValue"),
                "{field} {value}"
            );
            assert_eq!(page(&app, field)["total"], json!(1), "{field} {value}");
        }
    }
}

#[test]
fn a_page_never_holds_more_than_the_most_a_caller_may_ask_for() {
    let (_data, app) = library();
    exec(
        &app,
        "INSERT INTO recording (title)
            WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 501)
            SELECT 'Tune ' || i FROM n;
         INSERT INTO library_track (recording_id, kind, source_status)
            SELECT id, 'linked', 'missing' FROM recording WHERE id > 3;
         INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
            SELECT id, 'Genre', 'a', 'b', 'c' FROM library_track WHERE id > 3;",
    );
    let page = invoke(
        &app,
        "review_conflicts",
        json!({ "field": "Genre", "offset": 0, "limit": 100000 }),
    )
    .unwrap();
    // Track 1's conflict and the 501 added.
    assert_eq!(page["total"], json!(502));
    assert_eq!(
        page["items"].as_array().unwrap().len(),
        super::MAX_PAGE as usize
    );
}

// ---- crate membership (1cB-20, PR 119's follow-ups) ---------------------------

/// The field of a membership conflict for the crate `name` inside the folder
/// `Gigs`, as the engine writes it.
fn membership_field(name: &str) -> String {
    use crate::rekordbox_write::{sibling_key, CRATES_FOLDER};
    crate::conflicts::membership::field_of(&[
        sibling_key(CRATES_FOLDER),
        sibling_key("Gigs"),
        sibling_key(name),
    ])
}

#[test]
fn a_crate_membership_conflict_is_named_by_its_crate_and_is_never_given_by_hand() {
    let (_data, app) = library();
    // A name with a space, quotes and an accent: nothing may depend on it.
    let name = "Peak \"time\" café";
    let field = membership_field(name);
    let sql_field = field.clone();
    app.state::<Writer>()
        .call(move |c| {
            c.execute("INSERT INTO crate (kind, name) VALUES ('folder', 'Gigs')", [])?;
            c.execute(
                "INSERT INTO crate (parent_id, kind, name) VALUES (1, 'static', ?1)",
                [name],
            )?;
            c.execute(
                "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value,
                                       base_value)
                 VALUES (1, ?1, 'in', 'out', NULL)",
                [&sql_field],
            )
        })
        .unwrap();

    let fields = invoke(&app, "review_conflict_fields", json!({})).unwrap();
    assert!(
        fields.as_array().unwrap().contains(&json!({
            "field": field,
            "kind": "membership",
            "count": 1,
            "crateName": format!("Gigs / {name}"),
        })),
        "{fields}"
    );
    let id = id_of(&page(&app, &field), 0);
    let refused = invoke(
        &app,
        "review_resolve_conflict",
        json!({ "id": id, "choice": { "choice": "edit", "value": "in" } }),
    )
    .unwrap_err();
    assert_eq!(refused["kind"], json!("conflictBadValue"));
    assert_eq!(page(&app, &field)["total"], json!(1));
    // Keeping a side is fine.
    let settled = invoke(
        &app,
        "review_resolve_conflict",
        json!({ "id": id, "choice": { "choice": "keepRekordbox" } }),
    )
    .unwrap();
    assert_eq!(settled["value"], json!("out"));
}

#[test]
fn a_membership_conflict_whose_crate_is_gone_is_named_by_the_last_name_of_its_key() {
    let (_data, app) = library();
    let field = membership_field("Old Peak");
    let sql_field = field.clone();
    app.state::<Writer>()
        .call(move |c| {
            c.execute(
                "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value,
                                       base_value)
                 VALUES (1, ?1, 'out', 'in', NULL)",
                [&sql_field],
            )
        })
        .unwrap();
    let fields = invoke(&app, "review_conflict_fields", json!({})).unwrap();
    let named = fields
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field"] == json!(field))
        .unwrap();
    assert_eq!(named["crateName"], json!(crate::rekordbox_write::sibling_key("Old Peak")));
}
