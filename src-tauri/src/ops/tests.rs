use super::*;
use crate::db::migrations;
use crate::write_guard;
use serde_json::json;

/// A fully migrated database plus test-only STRICT tables:
/// - `sample`: one column of each type, for checking values come back
///   exactly, and a generated column the log must leave alone;
/// - `parent` with children whose foreign keys cascade, set null or
///   restrict, for checking the log never lets a write silently change rows
///   it can't record.
fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = write_guard::test_path(dir.path(), "ops.db")
        .open_database()
        .unwrap();
    migrations::run(&mut conn, migrations::MIGRATIONS).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    conn.execute_batch(
        "CREATE TABLE sample (
             id INTEGER PRIMARY KEY,
             i  INTEGER,
             r  REAL,
             t  TEXT,
             b  BLOB,
             d  TEXT NOT NULL DEFAULT 'filled in',
             g  INTEGER GENERATED ALWAYS AS (length(t)) VIRTUAL
         ) STRICT;
         CREATE TABLE parent (
             id   INTEGER PRIMARY KEY,
             code TEXT UNIQUE,
             name TEXT
         ) STRICT;
         CREATE TABLE child_cascade (
             id        INTEGER PRIMARY KEY,
             parent_id INTEGER REFERENCES parent (id) ON DELETE CASCADE
         ) STRICT;
         CREATE TABLE child_set_null (
             id        INTEGER PRIMARY KEY,
             parent_id INTEGER REFERENCES parent (id) ON DELETE SET NULL
         ) STRICT;
         CREATE TABLE child_restrict (
             id        INTEGER PRIMARY KEY,
             parent_id INTEGER REFERENCES parent (id) ON DELETE RESTRICT
         ) STRICT;
         CREATE TABLE child_by_code (
             id          INTEGER PRIMARY KEY,
             parent_code TEXT REFERENCES parent (code) ON UPDATE CASCADE
         ) STRICT;",
    )
    .unwrap();
    (dir, conn)
}

/// The database's one writer, as the app opens it.
fn writer() -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&write_guard::test_path(dir.path(), "ops.db")).unwrap();
    (dir, writer)
}

fn no_details() -> serde_json::Value {
    json!({})
}

/// Inserts a static crate directly (not through the log) and returns its id.
fn crate_row(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO crate (kind, name, notes) VALUES ('static', ?1, 'old notes')",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn crate_name(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT name FROM crate WHERE id = ?1", [id], |r| r.get(0))
        .optional()
        .unwrap()
}

fn crate_notes(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT notes FROM crate WHERE id = ?1", [id], |r| r.get(0))
        .unwrap()
}

/// Every column of a `sample` row, typed, for exact comparisons.
fn sample_row(conn: &Connection, id: i64) -> Option<Vec<Value>> {
    conn.query_row(
        "SELECT i, r, t, b, d FROM sample WHERE id = ?1",
        [id],
        |r| (0..5).map(|i| r.get::<_, Value>(i)).collect(),
    )
    .optional()
    .unwrap()
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn set_notes(conn: &mut Connection, id: i64, notes: &str) -> Option<i64> {
    let notes = notes.to_owned();
    record(conn, "edit_crate", &no_details(), |rec| {
        rec.set("crate", id, "notes", notes)
    })
    .unwrap()
    .operation_id
}

fn undone(outcome: UndoOutcome) -> OperationInfo {
    match outcome {
        UndoOutcome::Undone { operation } => operation,
        other => panic!("expected Undone, got {other:?}"),
    }
}

fn refused(outcome: UndoOutcome) -> Vec<UndoConflict> {
    match outcome {
        UndoOutcome::Refused { conflicts, .. } => conflicts,
        other => panic!("expected Refused, got {other:?}"),
    }
}

// --- Recording ---------------------------------------------------------

#[test]
fn a_set_writes_the_value_and_logs_it_with_its_before_and_after() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let op = set_notes(&mut conn, id, "new notes").unwrap();

    assert_eq!(crate_notes(&conn, id).as_deref(), Some("new notes"));
    let logged: (String, i64, String, String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT entity, entity_id, action, field, before, after FROM change
             WHERE operation_id = ?1",
            [op],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        logged,
        (
            "crate".into(),
            id,
            "set".into(),
            "notes".into(),
            Some("old notes".into()),
            Some("new notes".into())
        )
    );
}

#[test]
fn an_operation_stores_its_kind_and_details_and_no_text() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let op = record(
        &mut conn,
        "edit_crate",
        &json!({ "crateCount": 1 }),
        |rec| rec.set("crate", id, "notes", "x"),
    )
    .unwrap()
    .operation_id
    .unwrap();
    let (kind, details): (String, String) = conn
        .query_row(
            "SELECT kind, details FROM operation WHERE id = ?1",
            [op],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "edit_crate");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&details).unwrap(),
        json!({ "crateCount": 1 })
    );
}

#[test]
fn an_operation_kind_must_be_a_code_not_text() {
    let (_dir, mut conn) = db();
    for bad in ["", "Edited 3 fields", "Edit", "edit-crate", "1edit"] {
        let result = record(&mut conn, bad, &no_details(), |_| Ok(()));
        assert!(matches!(result, Err(OpsError::BadKind(_))), "{bad:?}");
    }
    assert_eq!(count(&conn, "operation"), 0);
}

#[test]
fn operation_details_must_be_a_json_object() {
    let (_dir, mut conn) = db();
    for bad in [json!("Edited 3 fields"), json!([1]), json!(null), json!(3)] {
        let result = record(&mut conn, "edit_crate", &bad, |_| Ok(()));
        assert!(matches!(result, Err(OpsError::DetailsNotObject)), "{bad}");
    }
    assert_eq!(count(&conn, "operation"), 0);
}

#[test]
fn setting_a_field_to_the_value_it_already_holds_logs_nothing() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    assert_eq!(set_notes(&mut conn, id, "old notes"), None);
    assert_eq!(count(&conn, "operation"), 0);
    assert_eq!(count(&conn, "change"), 0);
}

#[test]
fn a_write_that_fails_keeps_nothing_and_logs_nothing() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let result = record(&mut conn, "edit_crate", &no_details(), |rec| {
        rec.set("crate", id, "notes", "changed")?;
        // Refused by the schema: a crate's name can't be empty.
        rec.set("crate", id, "name", "")
    });
    assert!(matches!(result, Err(OpsError::Db(_))), "{result:?}");
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("old notes"));
    assert_eq!(count(&conn, "operation"), 0);
    assert_eq!(count(&conn, "change"), 0);
}

#[test]
fn an_error_returned_by_the_write_itself_rolls_everything_back() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let result: Result<Recorded<()>, _> = record(&mut conn, "edit_crate", &no_details(), |rec| {
        rec.set("crate", id, "notes", "changed")?;
        Err(OpsError::DetailsNotObject)
    });
    assert!(result.is_err());
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("old notes"));
    assert_eq!(count(&conn, "operation"), 0);
}

#[test]
fn setting_a_field_of_a_missing_row_is_refused() {
    let (_dir, mut conn) = db();
    let result = record(&mut conn, "edit_crate", &no_details(), |rec| {
        rec.set("crate", 999, "notes", "x")
    });
    assert!(
        matches!(result, Err(OpsError::RowNotFound { id: 999, .. })),
        "{result:?}"
    );
    assert_eq!(count(&conn, "operation"), 0);
}

#[test]
fn the_log_refuses_its_own_tables_and_tables_it_cannot_address() {
    let (_dir, mut conn) = db();
    for entity in [
        "operation",
        "change",
        "schema_migration",
        "sqlite_schema",
        // WITHOUT ROWID: `entity_id` can't name one of their rows.
        "setting",
        "service_optin",
        "relink",
        // Not a table.
        "crate_ancestor",
        "no_such_table",
        "crate; DROP TABLE crate",
    ] {
        let result = record(&mut conn, "edit_crate", &no_details(), |rec| {
            rec.insert(entity, &[])
        });
        assert!(
            matches!(result, Err(OpsError::NotRecordable(ref e)) if e == entity),
            "{entity}: {result:?}"
        );
    }
    assert_eq!(count(&conn, "crate"), 0, "the crate table is still there");
}

#[test]
fn the_log_refuses_unknown_fields_and_the_row_id() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    for field in ["id", "rowid", "nope", "notes = 'x', name"] {
        let result = record(&mut conn, "edit_crate", &no_details(), |rec| {
            rec.set("crate", id, field, "x")
        });
        assert!(
            matches!(result, Err(OpsError::NotAField { .. })),
            "{field}: {result:?}"
        );
    }
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("old notes"));
}

#[test]
fn an_insert_logs_every_field_including_defaults_the_database_filled_in() {
    let (_dir, mut conn) = db();
    let recorded = record(&mut conn, "add_sample", &no_details(), |rec| {
        rec.insert("sample", &[("t", Value::Text("hi".into()))])
    })
    .unwrap();
    let id = recorded.value;
    let mut stmt = conn
        .prepare("SELECT field, before, after FROM change WHERE entity_id = ?1 ORDER BY id")
        .unwrap();
    let rows: Vec<(String, Option<String>, Option<String>)> = stmt
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("i".into(), None, None),
            ("r".into(), None, None),
            ("t".into(), None, Some("hi".into())),
            ("b".into(), None, None),
            ("d".into(), None, Some("filled in".into())),
        ]
    );
}

#[test]
fn record_via_runs_on_the_one_writer_and_undo_last_via_undoes_it() {
    let (_dir, writer) = writer();
    let recorded = record_via(&writer, "add_crate", no_details(), |rec| {
        rec.insert(
            "crate",
            &[
                ("kind", Value::Text("static".into())),
                ("name", Value::Text("Peak".into())),
            ],
        )
    })
    .unwrap();
    assert!(recorded.operation_id.is_some());
    let id = recorded.value;
    let name = move |w: &Writer| w.call(move |c| Ok(crate_name(c, id))).unwrap();
    assert_eq!(name(&writer).as_deref(), Some("Peak"));

    undone(undo_last_via(&writer).unwrap());
    assert_eq!(name(&writer), None);
}

fn parent_row(conn: &Connection, code: &str) -> i64 {
    conn.execute("INSERT INTO parent (code, name) VALUES (?1, 'P')", [code])
        .unwrap();
    conn.last_insert_rowid()
}

/// A static crate holding one Library track, as the real schema has it:
/// `crate_entry` rows are deleted with their crate (ON DELETE CASCADE).
fn crate_with_a_track(conn: &Connection) -> i64 {
    conn.execute_batch(
        "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'DJ Music', 'DJ Music');
         INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
         INSERT INTO recording (title) VALUES ('Track 1');
         INSERT INTO recording_file (recording_id, file_id) VALUES (1, 1);
         INSERT INTO library_track (recording_id, linked_file_id) VALUES (1, 1);",
    )
    .unwrap();
    let id = crate_row(conn, "Peak");
    conn.execute(
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (?1, 1)",
        [id],
    )
    .unwrap();
    id
}

#[test]
fn deleting_a_crate_that_holds_tracks_is_refused_so_its_entries_are_not_lost() {
    // crate_entry rows would go with the crate (ON DELETE CASCADE), and the
    // log can't record them, so undo would bring the crate back empty.
    let (_dir, mut conn) = db();
    let id = crate_with_a_track(&conn);
    let result = record(&mut conn, "delete_crate", &no_details(), |rec| {
        rec.delete("crate", id)
    });
    assert!(
        matches!(result, Err(OpsError::Referenced { ref by, .. }) if by == "crate_entry"),
        "{result:?}"
    );
    assert_eq!(crate_name(&conn, id).as_deref(), Some("Peak"));
    assert_eq!(count(&conn, "crate_entry"), 1);
    assert_eq!(count(&conn, "operation"), 0);
}

#[test]
fn deleting_a_row_is_refused_while_a_set_null_key_references_it() {
    let (_dir, mut conn) = db();
    let id = parent_row(&conn, "a");
    conn.execute("INSERT INTO child_set_null (parent_id) VALUES (?1)", [id])
        .unwrap();
    let result = record(&mut conn, "delete_parent", &no_details(), |rec| {
        rec.delete("parent", id)
    });
    assert!(
        matches!(result, Err(OpsError::Referenced { ref by, .. }) if by == "child_set_null"),
        "{result:?}"
    );
    let parent_id: Option<i64> = conn
        .query_row("SELECT parent_id FROM child_set_null", [], |r| r.get(0))
        .unwrap();
    assert_eq!(parent_id, Some(id), "the child was silently changed");
}

#[test]
fn deleting_a_row_nothing_references_is_still_allowed() {
    let (_dir, mut conn) = db();
    let id = parent_row(&conn, "a");
    let other = parent_row(&conn, "b");
    conn.execute("INSERT INTO child_cascade (parent_id) VALUES (?1)", [other])
        .unwrap();
    record(&mut conn, "delete_parent", &no_details(), |rec| {
        rec.delete("parent", id)
    })
    .unwrap();
    assert_eq!(count(&conn, "parent"), 1);
    assert_eq!(count(&conn, "child_cascade"), 1);
}

#[test]
fn changing_a_key_that_cascades_to_other_rows_is_refused() {
    let (_dir, mut conn) = db();
    let id = parent_row(&conn, "a");
    conn.execute("INSERT INTO child_by_code (parent_code) VALUES ('a')", [])
        .unwrap();
    let result = record(&mut conn, "edit_parent", &no_details(), |rec| {
        rec.set("parent", id, "code", "b")
    });
    assert!(
        matches!(result, Err(OpsError::Referenced { ref by, .. }) if by == "child_by_code"),
        "{result:?}"
    );
    let code: String = conn
        .query_row("SELECT parent_code FROM child_by_code", [], |r| r.get(0))
        .unwrap();
    assert_eq!(code, "a", "the child was silently changed");

    // Another field of the same row is fine.
    record(&mut conn, "edit_parent", &no_details(), |rec| {
        rec.set("parent", id, "name", "Q")
    })
    .unwrap();
}

#[test]
fn a_delete_the_database_refuses_leaves_no_logged_delete_even_if_the_error_is_ignored() {
    let (_dir, mut conn) = db();
    let id = parent_row(&conn, "a");
    conn.execute("INSERT INTO child_restrict (parent_id) VALUES (?1)", [id])
        .unwrap();
    let crate_id = crate_row(&conn, "Warmup");

    let recorded = record(&mut conn, "tidy", &no_details(), |rec| {
        // RESTRICT refuses the delete; the caller carries on regardless.
        assert!(rec.delete("parent", id).is_err());
        rec.set("crate", crate_id, "notes", "new notes")
    })
    .unwrap();
    assert!(recorded.operation_id.is_some());
    let deletes: i64 = conn
        .query_row(
            "SELECT count(*) FROM change WHERE action = 'delete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(deletes, 0, "a delete that never happened was logged");

    // So undo works: the parent was never gone.
    undone(undo_last(&mut conn).unwrap());
    assert_eq!(crate_notes(&conn, crate_id).as_deref(), Some("old notes"));
    assert_eq!(count(&conn, "parent"), 1);
}

#[test]
fn an_operation_whose_only_write_failed_is_not_logged() {
    let (_dir, mut conn) = db();
    let id = parent_row(&conn, "a");
    conn.execute("INSERT INTO child_restrict (parent_id) VALUES (?1)", [id])
        .unwrap();
    let recorded = record(&mut conn, "delete_parent", &no_details(), |rec| {
        let _ = rec.delete("parent", id);
        Ok(())
    })
    .unwrap();
    assert_eq!(recorded.operation_id, None);
    assert_eq!(count(&conn, "operation"), 0);
    assert_eq!(count(&conn, "change"), 0);
}

#[test]
fn a_temp_table_with_the_same_name_cannot_stand_in_for_the_real_one() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    conn.execute_batch(
        "CREATE TEMP TABLE crate (id INTEGER PRIMARY KEY, notes TEXT) STRICT;
         INSERT INTO temp.crate (id, notes) VALUES (1, 'temp notes');",
    )
    .unwrap();
    record(&mut conn, "edit_crate", &no_details(), |rec| {
        rec.set("crate", id, "notes", "new notes")
    })
    .unwrap();
    let main_notes: String = conn
        .query_row("SELECT notes FROM main.crate WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    let temp_notes: String = conn
        .query_row("SELECT notes FROM temp.crate WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(main_notes, "new notes");
    assert_eq!(temp_notes, "temp notes");

    undone(undo_last(&mut conn).unwrap());
    let main_notes: String = conn
        .query_row("SELECT notes FROM main.crate WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(main_notes, "old notes");
}

#[test]
fn a_number_that_is_not_finite_is_refused_not_stored_as_null() {
    let (_dir, mut conn) = db();
    conn.execute("INSERT INTO sample (r) VALUES (1.5)", [])
        .unwrap();
    let id = conn.last_insert_rowid();
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let set = record(&mut conn, "edit_sample", &no_details(), |rec| {
            rec.set("sample", id, "r", n)
        });
        assert!(
            matches!(set, Err(OpsError::NotFinite(ref f)) if f == "r"),
            "{set:?}"
        );
        let insert = record(&mut conn, "add_sample", &no_details(), |rec| {
            rec.insert("sample", &[("r", Value::Real(n))])
        });
        assert!(matches!(insert, Err(OpsError::NotFinite(_))), "{insert:?}");
    }
    let r: f64 = conn
        .query_row("SELECT r FROM sample WHERE id = ?1", [id], |r| r.get(0))
        .unwrap();
    assert_eq!(r, 1.5);
    assert_eq!(count(&conn, "sample"), 1);
    assert_eq!(count(&conn, "operation"), 0);
}

// --- Undo: exact reversal ----------------------------------------------

#[test]
fn with_an_empty_log_there_is_nothing_to_undo() {
    let (_dir, mut conn) = db();
    assert_eq!(undo_last(&mut conn).unwrap(), UndoOutcome::NothingToUndo);
}

#[test]
fn undoing_a_set_restores_the_value_before() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let op = set_notes(&mut conn, id, "new notes").unwrap();
    let info = undone(undo_last(&mut conn).unwrap());
    assert_eq!(info.id, op);
    assert_eq!(info.kind, "edit_crate");
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("old notes"));
}

#[test]
fn undo_restores_null_and_empty_text_as_different_values() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    conn.execute("UPDATE crate SET notes = NULL WHERE id = ?1", [id])
        .unwrap();

    // NULL → "" → back to NULL.
    set_notes(&mut conn, id, "").unwrap();
    assert_eq!(crate_notes(&conn, id).as_deref(), Some(""));
    undone(undo_last(&mut conn).unwrap());
    assert_eq!(crate_notes(&conn, id), None);

    // "" → NULL → back to "".
    conn.execute("UPDATE crate SET notes = '' WHERE id = ?1", [id])
        .unwrap();
    record(&mut conn, "edit_crate", &no_details(), |rec| {
        rec.set("crate", id, "notes", Value::Null)
    })
    .unwrap();
    assert_eq!(crate_notes(&conn, id), None);
    undone(undo_last(&mut conn).unwrap());
    assert_eq!(crate_notes(&conn, id).as_deref(), Some(""));
}

#[test]
fn undo_restores_every_column_type_exactly() {
    let (_dir, mut conn) = db();
    let tricky: Vec<Value> = vec![
        Value::Integer(i64::MAX),
        Value::Real(0.1 + 0.2), // not 0.3: must not be rounded on the way
        Value::Text("Artisté – Song Title (Remixer Edit) 🎧\n\"quoted\"".into()),
        Value::Blob(vec![0, 1, 0xfe, 0xff, b'"']),
        Value::Text("  spaces kept  ".into()),
    ];
    conn.execute(
        "INSERT INTO sample (i, r, t, b, d) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params_from_iter(tricky.iter()),
    )
    .unwrap();
    let id = conn.last_insert_rowid();

    record(&mut conn, "edit_sample", &no_details(), |rec| {
        rec.set("sample", id, "i", i64::MIN)?;
        rec.set("sample", id, "r", 1e-300)?;
        rec.set("sample", id, "t", "other")?;
        rec.set("sample", id, "b", vec![9u8])?;
        rec.set("sample", id, "d", "other")
    })
    .unwrap();
    undone(undo_last(&mut conn).unwrap());
    assert_eq!(sample_row(&conn, id).unwrap(), tricky);
}

#[test]
fn a_whole_number_set_on_a_real_column_is_logged_as_stored_and_undone() {
    let (_dir, mut conn) = db();
    conn.execute("INSERT INTO sample (r) VALUES (128.5)", [])
        .unwrap();
    let id = conn.last_insert_rowid();
    record(&mut conn, "edit_sample", &no_details(), |rec| {
        rec.set("sample", id, "r", 128i64)
    })
    .unwrap();
    undone(undo_last(&mut conn).unwrap());
    let r: f64 = conn
        .query_row("SELECT r FROM sample WHERE id = ?1", [id], |r| r.get(0))
        .unwrap();
    assert_eq!(r, 128.5);
}

#[test]
fn undoing_an_insert_deletes_the_row() {
    let (_dir, mut conn) = db();
    let id = record(&mut conn, "add_crate", &no_details(), |rec| {
        rec.insert(
            "crate",
            &[
                ("kind", Value::Text("static".into())),
                ("name", Value::Text("Peak".into())),
            ],
        )
    })
    .unwrap()
    .value;
    assert_eq!(crate_name(&conn, id).as_deref(), Some("Peak"));
    undone(undo_last(&mut conn).unwrap());
    assert_eq!(crate_name(&conn, id), None);
    assert_eq!(count(&conn, "crate"), 0);
}

#[test]
fn undoing_a_delete_puts_the_row_back_with_its_id_and_every_field() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let other = crate_row(&conn, "Peak");
    let full_row = |c: &Connection| -> Vec<Value> {
        c.query_row("SELECT * FROM crate WHERE id = ?1", [id], |r| {
            (0..8).map(|i| r.get::<_, Value>(i)).collect()
        })
        .unwrap()
    };
    let before = full_row(&conn);

    record(&mut conn, "delete_crate", &no_details(), |rec| {
        rec.delete("crate", id)
    })
    .unwrap();
    assert_eq!(crate_name(&conn, id), None);

    undone(undo_last(&mut conn).unwrap());
    assert_eq!(
        full_row(&conn),
        before,
        "the row came back different (created_at, notes, position…)"
    );
    assert_eq!(crate_name(&conn, other).as_deref(), Some("Peak"));
}

#[test]
fn undoing_a_delete_restores_blobs_reals_and_nulls_exactly() {
    let (_dir, mut conn) = db();
    conn.execute(
        "INSERT INTO sample (i, r, t, b) VALUES (NULL, 0.1, NULL, x'00ff10')",
        [],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    let before = sample_row(&conn, id);
    record(&mut conn, "delete_sample", &no_details(), |rec| {
        rec.delete("sample", id)
    })
    .unwrap();
    assert_eq!(sample_row(&conn, id), None);
    undone(undo_last(&mut conn).unwrap());
    assert_eq!(sample_row(&conn, id), before);
}

#[test]
fn changes_made_in_sequence_within_one_operation_unwind_in_reverse() {
    let (_dir, mut conn) = db();
    let existing = crate_row(&conn, "Warmup");
    let made = record(&mut conn, "tidy_crates", &no_details(), |rec| {
        // Set the same field twice, make a row then edit it, delete a row.
        rec.set("crate", existing, "notes", "first")?;
        rec.set("crate", existing, "notes", "second")?;
        let id = rec.insert(
            "crate",
            &[
                ("kind", Value::Text("static".into())),
                ("name", Value::Text("Draft".into())),
            ],
        )?;
        rec.set("crate", id, "name", "Final")?;
        let doomed = rec.insert(
            "crate",
            &[
                ("kind", Value::Text("static".into())),
                ("name", Value::Text("Doomed".into())),
            ],
        )?;
        rec.delete("crate", doomed)?;
        Ok(id)
    })
    .unwrap()
    .value;
    assert_eq!(crate_name(&conn, made).as_deref(), Some("Final"));

    undone(undo_last(&mut conn).unwrap());
    assert_eq!(crate_notes(&conn, existing).as_deref(), Some("old notes"));
    assert_eq!(count(&conn, "crate"), 1, "made rows were left behind");
}

#[test]
fn an_undone_operation_is_marked_not_deleted_and_is_not_undone_twice() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let op = set_notes(&mut conn, id, "new notes").unwrap();
    undone(undo_last(&mut conn).unwrap());

    let undone_at: Option<String> = conn
        .query_row("SELECT undone_at FROM operation WHERE id = ?1", [op], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(undone_at.is_some());
    assert_eq!(count(&conn, "change"), 1, "the history was thrown away");

    // Someone sets the notes by hand; a second undo mustn't touch them.
    conn.execute("UPDATE crate SET notes = 'by hand' WHERE id = ?1", [id])
        .unwrap();
    assert_eq!(undo_last(&mut conn).unwrap(), UndoOutcome::NothingToUndo);
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("by hand"));
}

#[test]
fn undo_is_single_step_it_undoes_only_the_most_recent_operation() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    set_notes(&mut conn, id, "one").unwrap();
    let second = set_notes(&mut conn, id, "two").unwrap();

    assert_eq!(undone(undo_last(&mut conn).unwrap()).id, second);
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("one"));
    // Multi-step undo is 1bA-12: the first operation stays done.
    assert_eq!(undo_last(&mut conn).unwrap(), UndoOutcome::NothingToUndo);
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("one"));

    // A new operation can be undone again.
    let third = set_notes(&mut conn, id, "three").unwrap();
    assert_eq!(undone(undo_last(&mut conn).unwrap()).id, third);
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("one"));
}

// --- Undo: refused when something changed since -----------------------

#[test]
fn undo_is_refused_when_the_field_was_changed_since() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let op = set_notes(&mut conn, id, "new notes").unwrap();
    // A later change outside this operation, e.g. a rekordbox read-back.
    conn.execute("UPDATE crate SET notes = 'later' WHERE id = ?1", [id])
        .unwrap();

    let conflicts = refused(undo_last(&mut conn).unwrap());
    assert_eq!(
        conflicts,
        vec![UndoConflict {
            entity: "crate".into(),
            entity_id: id,
            field: Some("notes".into()),
            problem: ConflictProblem::ChangedSince,
        }]
    );
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("later"));
    let undone_at: Option<String> = conn
        .query_row("SELECT undone_at FROM operation WHERE id = ?1", [op], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(undone_at, None, "a refused undo was marked as done");
}

#[test]
fn a_later_change_to_a_different_field_does_not_block_undo() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    set_notes(&mut conn, id, "new notes").unwrap();
    conn.execute("UPDATE crate SET name = 'Renamed' WHERE id = ?1", [id])
        .unwrap();
    undone(undo_last(&mut conn).unwrap());
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("old notes"));
    assert_eq!(crate_name(&conn, id).as_deref(), Some("Renamed"));
}

#[test]
fn a_refused_undo_changes_nothing_even_where_it_could_have() {
    let (_dir, mut conn) = db();
    let a = crate_row(&conn, "A");
    let b = crate_row(&conn, "B");
    record(&mut conn, "edit_crate", &no_details(), |rec| {
        rec.set("crate", a, "notes", "a new")?;
        rec.set("crate", b, "notes", "b new")
    })
    .unwrap();
    // Only b's notes change later; a's could be undone on their own.
    conn.execute("UPDATE crate SET notes = 'b later' WHERE id = ?1", [b])
        .unwrap();

    let conflicts = refused(undo_last(&mut conn).unwrap());
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].entity_id, b);
    assert_eq!(crate_notes(&conn, a).as_deref(), Some("a new"));
    assert_eq!(crate_notes(&conn, b).as_deref(), Some("b later"));
}

#[test]
fn undo_is_refused_when_a_changed_row_was_deleted_since() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    set_notes(&mut conn, id, "new notes").unwrap();
    conn.execute("DELETE FROM crate WHERE id = ?1", [id])
        .unwrap();
    let conflicts = refused(undo_last(&mut conn).unwrap());
    assert_eq!(conflicts[0].problem, ConflictProblem::RowGone);
    assert_eq!(conflicts[0].field, None);
    assert_eq!(crate_name(&conn, id), None);
}

#[test]
fn undoing_an_insert_is_refused_when_the_new_row_was_edited_since() {
    let (_dir, mut conn) = db();
    let id = record(&mut conn, "add_crate", &no_details(), |rec| {
        rec.insert(
            "crate",
            &[
                ("kind", Value::Text("static".into())),
                ("name", Value::Text("Peak".into())),
            ],
        )
    })
    .unwrap()
    .value;
    conn.execute("UPDATE crate SET notes = 'my notes' WHERE id = ?1", [id])
        .unwrap();

    let conflicts = refused(undo_last(&mut conn).unwrap());
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].field.as_deref(), Some("notes"));
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("my notes"));
}

#[test]
fn undoing_an_insert_is_refused_when_the_new_row_is_already_gone() {
    let (_dir, mut conn) = db();
    let id = record(&mut conn, "add_crate", &no_details(), |rec| {
        rec.insert(
            "crate",
            &[
                ("kind", Value::Text("static".into())),
                ("name", Value::Text("Peak".into())),
            ],
        )
    })
    .unwrap()
    .value;
    conn.execute("DELETE FROM crate WHERE id = ?1", [id])
        .unwrap();
    let conflicts = refused(undo_last(&mut conn).unwrap());
    assert_eq!(conflicts[0].problem, ConflictProblem::RowGone);
}

#[test]
fn undoing_an_insert_is_refused_when_rows_now_reference_the_new_row() {
    // Undoing would delete the row, and the cascade would take the later
    // child rows with it, unrecorded.
    let (_dir, mut conn) = db();
    let id = record(&mut conn, "add_parent", &no_details(), |rec| {
        rec.insert("parent", &[("code", Value::Text("a".into()))])
    })
    .unwrap()
    .value;
    conn.execute("INSERT INTO child_cascade (parent_id) VALUES (?1)", [id])
        .unwrap();

    let conflicts = refused(undo_last(&mut conn).unwrap());
    assert_eq!(
        conflicts,
        vec![UndoConflict {
            entity: "parent".into(),
            entity_id: id,
            field: None,
            problem: ConflictProblem::Referenced,
        }]
    );
    assert_eq!(count(&conn, "parent"), 1);
    assert_eq!(
        count(&conn, "child_cascade"),
        1,
        "the later child was deleted"
    );
}

#[test]
fn undoing_a_key_change_is_refused_when_rows_now_reference_the_new_key() {
    let (_dir, mut conn) = db();
    let id = parent_row(&conn, "a");
    record(&mut conn, "edit_parent", &no_details(), |rec| {
        rec.set("parent", id, "code", "b")
    })
    .unwrap();
    conn.execute("INSERT INTO child_by_code (parent_code) VALUES ('b')", [])
        .unwrap();

    let conflicts = refused(undo_last(&mut conn).unwrap());
    assert_eq!(conflicts[0].problem, ConflictProblem::Referenced);
    assert_eq!(conflicts[0].field.as_deref(), Some("code"));
    let code: String = conn
        .query_row("SELECT parent_code FROM child_by_code", [], |r| r.get(0))
        .unwrap();
    assert_eq!(code, "b", "the later child was silently changed");
}

#[test]
fn undoing_a_delete_is_refused_when_the_row_is_back() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    record(&mut conn, "delete_crate", &no_details(), |rec| {
        rec.delete("crate", id)
    })
    .unwrap();
    conn.execute(
        "INSERT INTO crate (id, kind, name) VALUES (?1, 'static', 'Someone else')",
        [id],
    )
    .unwrap();

    let conflicts = refused(undo_last(&mut conn).unwrap());
    assert_eq!(conflicts[0].problem, ConflictProblem::RowBack);
    assert_eq!(crate_name(&conn, id).as_deref(), Some("Someone else"));
}

// --- The command the frontend calls ------------------------------------

#[test]
fn undo_outcomes_serialize_for_the_ui_with_operation_and_row_ids() {
    let operation = OperationInfo {
        id: 41,
        kind: "edit_crate".into(),
    };
    assert_eq!(
        serde_json::to_value(UndoOutcome::NothingToUndo).unwrap(),
        json!({ "status": "nothingToUndo" })
    );
    assert_eq!(
        serde_json::to_value(UndoOutcome::Undone {
            operation: operation.clone()
        })
        .unwrap(),
        json!({ "status": "undone", "operation": { "id": 41, "kind": "edit_crate" } })
    );
    assert_eq!(
        serde_json::to_value(ConflictProblem::Referenced).unwrap(),
        json!("referenced")
    );
    assert_eq!(
        serde_json::to_value(UndoOutcome::Refused {
            operation,
            conflicts: vec![UndoConflict {
                entity: "crate".into(),
                entity_id: 7,
                field: Some("notes".into()),
                problem: ConflictProblem::ChangedSince,
            }],
        })
        .unwrap(),
        json!({
            "status": "refused",
            "operation": { "id": 41, "kind": "edit_crate" },
            "conflicts": [{
                "entity": "crate",
                "entityId": 7,
                "field": "notes",
                "problem": "changedSince",
            }],
        })
    );
}

#[test]
fn the_undo_command_answers_over_ipc() {
    use tauri::Manager;

    let (_dir, writer) = writer();
    let app = tauri::test::mock_builder()
        .invoke_handler(crate::ipc::specta_builder().invoke_handler())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(writer.clone());
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let undo_result = || {
        let request = tauri::webview::InvokeRequest {
            cmd: "undo_last_operation".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: if cfg!(windows) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        };
        tauri::test::get_ipc_response(&webview, request)
            .map(|body| body.deserialize::<serde_json::Value>().unwrap())
    };
    let undo = || undo_result().unwrap();

    assert_eq!(undo(), json!({ "status": "nothingToUndo" }));
    let added = record_via(&writer, "add_crate", no_details(), |rec| {
        rec.insert(
            "crate",
            &[
                ("kind", Value::Text("static".into())),
                ("name", Value::Text("Peak".into())),
            ],
        )
    })
    .unwrap();
    assert_eq!(
        undo(),
        json!({
            "status": "undone",
            "operation": { "id": added.operation_id.unwrap(), "kind": "add_crate" },
        })
    );
    assert_eq!(undo(), json!({ "status": "nothingToUndo" }));

    // A database error comes back as an error kind, never its text.
    writer
        .call(|c| c.execute_batch("ALTER TABLE operation RENAME TO gone"))
        .unwrap();
    let failed = undo_result().unwrap_err();
    assert_eq!(failed, json!({ "kind": "database", "params": {} }));
}

#[test]
fn a_refused_undo_names_the_operation_and_each_conflicting_row_by_id_over_ipc() {
    use tauri::Manager;

    let (_data, app) = crate::ipc::testing::app();
    let writer = app.state::<Writer>().inner().clone();
    // Ids past 2^32, so a narrowing on the way would show.
    let (a, b) = (5_000_000_001_i64, 5_000_000_002_i64);
    writer
        .call(move |c| {
            c.execute(
                "INSERT INTO crate (id, kind, name) VALUES (?1, 'static', 'A'), (?2, 'static', 'B')",
                [a, b],
            )?;
            c.execute("INSERT INTO operation (id, kind, details) VALUES (7000000000, 'seed', '{}')", [])
        })
        .unwrap();
    let op = record_via(&writer, "edit_crate", no_details(), move |rec| {
        rec.set("crate", a, "notes", "a new")?;
        rec.set("crate", b, "notes", "b new")
    })
    .unwrap()
    .operation_id
    .unwrap();
    assert!(op > 7_000_000_000, "{op}");
    writer
        .call(move |c| {
            c.execute(
                "UPDATE crate SET notes = 'later' WHERE id IN (?1, ?2)",
                [a, b],
            )
        })
        .unwrap();

    let outcome = crate::ipc::testing::invoke(&app, "undo_last_operation", json!({})).unwrap();
    assert_eq!(
        outcome,
        json!({
            "status": "refused",
            "operation": { "id": op, "kind": "edit_crate" },
            "conflicts": [
                { "entity": "crate", "entityId": b, "field": "notes", "problem": "changedSince" },
                { "entity": "crate", "entityId": a, "field": "notes", "problem": "changedSince" },
            ],
        })
    );
    // The ids come back exactly: they name the same rows in the database.
    let sent = outcome["conflicts"][0]["entityId"].as_i64().unwrap();
    let name: String = writer
        .call(move |c| c.query_row("SELECT name FROM crate WHERE id = ?1", [sent], |r| r.get(0)))
        .unwrap();
    assert_eq!(name, "B");
}

#[test]
fn bindings_declare_the_undo_command_and_its_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bindings.ts");
    crate::ipc::export_bindings(&path).unwrap();
    let ts = std::fs::read_to_string(&path).unwrap();
    assert!(
        ts.contains(
            r#"undoLastOperation: () => typedError<UndoOutcome, IpcError>(__TAURI_INVOKE("undo_last_operation"))"#
        ),
        "{ts}"
    );
    assert!(ts.contains("export type UndoOutcome ="), "{ts}");
    for expected in [
        "export type OperationInfo = {",
        "id: number,",
        "entityId: number,",
    ] {
        assert!(
            ts.contains(expected),
            "missing `{expected}` in:
{ts}"
        );
    }
    assert!(ts.contains("export type ConflictProblem ="), "{ts}");
}

#[test]
fn a_write_whose_log_row_fails_is_undone_even_if_the_error_is_ignored() {
    // Each recorder method is one unit: if its log row can't be written, its
    // write goes too, so nothing unlogged (and so un-undoable) is kept.
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let doomed = crate_row(&conn, "Doomed");
    conn.execute_batch(
        "CREATE TEMP TRIGGER refuse_notes_log BEFORE INSERT ON main.change
         WHEN new.field = 'notes' BEGIN SELECT RAISE(ABORT, 'log refused'); END;",
    )
    .unwrap();

    let recorded = record(&mut conn, "tidy", &no_details(), |rec| {
        assert!(rec.set("crate", id, "notes", "new notes").is_err());
        assert!(rec.delete("crate", doomed).is_err());
        rec.set("crate", id, "name", "Renamed")
    })
    .unwrap();
    assert!(recorded.operation_id.is_some());
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("old notes"));
    assert_eq!(crate_name(&conn, doomed).as_deref(), Some("Doomed"));
    assert_eq!(crate_name(&conn, id).as_deref(), Some("Renamed"));
}
