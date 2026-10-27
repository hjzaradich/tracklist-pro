//! The columns each track table shows are remembered per screen, and are no
//! part of the Library (1cA-5).

use serde_json::{json, Value};

use super::*;
use crate::ipc::testing::{app, invoke};

/// A migrated database in a temp dir, with its writer and read pool.
fn db() -> (tempfile::TempDir, Writer, ReadPool) {
    let dir = tempfile::tempdir().unwrap();
    let db_file = crate::db::db_path(dir.path());
    let db_file = db_file.file_name().unwrap().to_str().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(dir.path(), db_file)).unwrap();
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    (dir, writer, reads)
}

fn columns(shown: &[&str], known: &[&str]) -> TableColumns {
    TableColumns {
        shown: shown.iter().map(|s| s.to_string()).collect(),
        known: known.iter().map(|s| s.to_string()).collect(),
    }
}

fn count(writer: &Writer, table: &'static str) -> i64 {
    writer
        .call(move |c| c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
        .unwrap()
}

#[test]
fn a_screen_with_no_choice_reads_as_none_so_it_shows_its_default() {
    let (_dir, _writer, reads) = db();
    for screen in [
        TableScreen::Library,
        TableScreen::AllMusic,
        TableScreen::Crate,
    ] {
        assert_eq!(reads.read(|c| read_table_columns(c, screen)).unwrap(), None);
    }
}

#[test]
fn each_screen_keeps_its_own_columns_in_the_order_they_were_given() {
    let (_dir, writer, reads) = db();
    let library = columns(
        &["title", "bpm", "artist"],
        &["title", "artist", "bpm", "key"],
    );
    let all_music = columns(&["file", "title"], &["title", "file"]);
    let crates = columns(&["title"], &["title"]);
    writer
        .call({
            let (l, a, c) = (library.clone(), all_music.clone(), crates.clone());
            move |conn| {
                write_table_columns(conn, TableScreen::Library, Some(&l))?;
                write_table_columns(conn, TableScreen::AllMusic, Some(&a))?;
                write_table_columns(conn, TableScreen::Crate, Some(&c))
            }
        })
        .unwrap();
    let read = |screen| reads.read(move |c| read_table_columns(c, screen)).unwrap();
    assert_eq!(read(TableScreen::Library), Some(library));
    assert_eq!(read(TableScreen::AllMusic), Some(all_music));
    assert_eq!(read(TableScreen::Crate), Some(crates));
    assert_eq!(count(&writer, "ui_setting"), 3);
}

#[test]
fn changing_a_choice_keeps_one_row_for_the_screen_and_updates_its_time() {
    let (_dir, writer, reads) = db();
    writer
        .call(|c| {
            write_table_columns(
                c,
                TableScreen::Crate,
                Some(&columns(&["title"], &["title"])),
            )?;
            c.execute(
                "UPDATE ui_setting SET updated_at = '2000-01-01T00:00:00.000Z'",
                [],
            )?;
            write_table_columns(
                c,
                TableScreen::Crate,
                Some(&columns(&["artist", "title"], &["title", "artist"])),
            )
        })
        .unwrap();
    assert_eq!(count(&writer, "ui_setting"), 1);
    let updated: String = writer
        .call(|c| c.query_row("SELECT updated_at FROM ui_setting", [], |r| r.get(0)))
        .unwrap();
    assert!(updated.as_str() > "2000-01-01T00:00:00.000Z", "{updated}");
    let now = reads
        .read(|c| read_table_columns(c, TableScreen::Crate))
        .unwrap()
        .unwrap();
    assert_eq!(now.shown, ["artist", "title"]);
}

#[test]
fn going_back_to_the_default_forgets_the_choice_of_that_screen_only() {
    let (_dir, writer, reads) = db();
    writer
        .call(|c| {
            let one = columns(&["title"], &["title"]);
            write_table_columns(c, TableScreen::Library, Some(&one))?;
            write_table_columns(c, TableScreen::AllMusic, Some(&one))?;
            write_table_columns(c, TableScreen::Library, None)?;
            // Forgetting what was never chosen is fine too.
            write_table_columns(c, TableScreen::Crate, None)
        })
        .unwrap();
    let read = |screen| reads.read(move |c| read_table_columns(c, screen)).unwrap();
    assert_eq!(read(TableScreen::Library), None);
    assert!(read(TableScreen::AllMusic).is_some());
}

#[test]
fn a_stored_value_this_version_cannot_read_is_no_choice_instead_of_an_error() {
    let (_dir, writer, reads) = db();
    for value in [
        "42",
        "null",
        r#""title""#,
        r#"["title"]"#,
        r#"{"shown":"title","known":[]}"#,
        r#"{"shown":[1,2],"known":[]}"#,
        r#"{"shown":["title"]}"#,
    ] {
        writer
            .call(move |c| {
                c.execute(
                    "INSERT OR REPLACE INTO ui_setting (key, value)
                     VALUES ('table_columns.library', ?1)",
                    [value],
                )
            })
            .unwrap();
        assert_eq!(
            reads
                .read(|c| read_table_columns(c, TableScreen::Library))
                .unwrap(),
            None,
            "{value}"
        );
    }
}

#[test]
fn ids_this_version_does_not_know_are_kept_as_text_for_the_screen_to_ignore() {
    // The backend does not know the columns: a later version's id, or a
    // removed column's, comes back as it was stored.
    let (_dir, writer, reads) = db();
    let odd = columns(&["title", "tempo_curve"], &["title", "tempo_curve"]);
    writer
        .call({
            let odd = odd.clone();
            move |c| write_table_columns(c, TableScreen::Library, Some(&odd))
        })
        .unwrap();
    assert_eq!(
        reads
            .read(|c| read_table_columns(c, TableScreen::Library))
            .unwrap(),
        Some(odd)
    );
}

#[test]
fn blank_repeated_and_runaway_ids_are_tidied_away_when_stored() {
    let (_dir, writer, reads) = db();
    let long = "x".repeat(LONGEST_ID + 1);
    let many: Vec<String> = (0..MOST_COLUMNS + 10).map(|n| format!("c{n}")).collect();
    let messy = TableColumns {
        shown: vec![
            "title".into(),
            "".into(),
            "artist".into(),
            "title".into(),
            long,
        ],
        known: many,
    };
    writer
        .call(move |c| write_table_columns(c, TableScreen::AllMusic, Some(&messy)))
        .unwrap();
    let stored = reads
        .read(|c| read_table_columns(c, TableScreen::AllMusic))
        .unwrap()
        .unwrap();
    assert_eq!(stored.shown, ["title", "artist"]);
    assert_eq!(stored.known.len(), MOST_COLUMNS);
}

#[test]
fn a_choice_is_no_operation_so_nothing_is_logged_and_undo_leaves_it_alone() {
    let (_dir, writer, reads) = db();
    writer
        .call(|c| {
            write_table_columns(
                c,
                TableScreen::Library,
                Some(&columns(&["title"], &["title"])),
            )
        })
        .unwrap();
    assert_eq!(count(&writer, "operation"), 0);
    assert_eq!(count(&writer, "change"), 0);

    // A real operation, and its undo, leave the choice as it was.
    writer
        .call(|c| Ok(crate::crates::create_on(c, "Warmup").map(|_| ())))
        .unwrap()
        .unwrap();
    assert_eq!(count(&writer, "operation"), 1);
    writer
        .call(|c| Ok(crate::ops::undo_last(c).map(|_| ())))
        .unwrap()
        .unwrap();
    assert_eq!(count(&writer, "crate"), 0);
    assert!(reads
        .read(|c| read_table_columns(c, TableScreen::Library))
        .unwrap()
        .is_some());
}

#[test]
fn the_frontend_reads_and_sets_a_screens_columns_and_goes_back_to_the_default() {
    let (_data, app) = app();
    assert_eq!(
        invoke(&app, "table_columns", json!({ "screen": "allMusic" })),
        Ok(Value::Null)
    );
    let chosen = json!({ "shown": ["title", "file"], "known": ["title", "file", "folder"] });
    assert_eq!(
        invoke(
            &app,
            "set_table_columns",
            json!({ "screen": "allMusic", "columns": chosen })
        ),
        Ok(Value::Null)
    );
    assert_eq!(
        invoke(&app, "table_columns", json!({ "screen": "allMusic" })),
        Ok(chosen)
    );
    // The Library's choice is its own.
    assert_eq!(
        invoke(&app, "table_columns", json!({ "screen": "library" })),
        Ok(Value::Null)
    );
    assert_eq!(
        invoke(
            &app,
            "set_table_columns",
            json!({ "screen": "allMusic", "columns": null })
        ),
        Ok(Value::Null)
    );
    assert_eq!(
        invoke(&app, "table_columns", json!({ "screen": "allMusic" })),
        Ok(Value::Null)
    );
}

#[test]
fn a_screen_the_app_does_not_have_is_refused() {
    let (_data, app) = app();
    assert!(invoke(&app, "table_columns", json!({ "screen": "settings" })).is_err());
    assert!(invoke(
        &app,
        "set_table_columns",
        json!({ "screen": "settings", "columns": null })
    )
    .is_err());
}
