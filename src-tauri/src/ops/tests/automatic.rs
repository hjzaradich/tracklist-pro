//! Automatic operations (1bE-10): logged, never the one Undo lands on, and
//! taken back together with an earlier user action on the same rows.

use super::*;

/// Sets a crate's notes as the app does on its own: an automatic operation.
fn automatic_notes(conn: &mut Connection, id: i64, notes: &str) -> Option<i64> {
    let notes = notes.to_owned();
    record_automatic(conn, AutomaticKind::FollowFile, &no_details(), |rec| {
        rec.set("crate", id, "notes", notes)
    })
    .unwrap()
    .operation_id
}

fn undone_at(conn: &Connection, operation: i64) -> Option<String> {
    conn.query_row(
        "SELECT undone_at FROM operation WHERE id = ?1",
        [operation],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn an_automatic_operation_is_logged_with_its_kind_and_changes() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let op = automatic_notes(&mut conn, id, "moved").unwrap();
    let kind: String = conn
        .query_row("SELECT kind FROM operation WHERE id = ?1", [op], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(kind, "follow_file");
    assert_eq!(AutomaticKind::of(&kind), Some(AutomaticKind::FollowFile));
    assert_eq!(count(&conn, "change"), 1);
}

#[test]
fn a_users_action_cannot_be_recorded_under_an_automatic_kind() {
    // Or Undo would skip something the user did.
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    for kind in AutomaticKind::ALL {
        let result = record(&mut conn, kind.as_str(), &no_details(), |rec| {
            rec.set("crate", id, "notes", "mine")
        });
        assert!(matches!(result, Err(OpsError::BadKind(_))), "{result:?}");
    }
    assert_eq!(count(&conn, "operation"), 0);
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("old notes"));
}

#[test]
fn with_only_automatic_operations_there_is_nothing_to_undo() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let op = automatic_notes(&mut conn, id, "moved").unwrap();
    assert_eq!(next_undo(&mut conn).unwrap().operation, None);
    assert_eq!(undo_last(&mut conn).unwrap(), UndoOutcome::NothingToUndo);
    assert_eq!(
        undo_only(&mut conn, op).unwrap(),
        UndoOutcome::NothingToUndo
    );
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("moved"));
    assert_eq!(undone_at(&conn, op), None);
}

#[test]
fn undo_takes_back_the_users_last_action_when_automatic_operations_are_newer() {
    let (_dir, mut conn) = db();
    let mine = crate_row(&conn, "Warmup");
    let other = crate_row(&conn, "Peak");
    let user = set_notes(&mut conn, mine, "my notes").unwrap();
    let first = automatic_notes(&mut conn, other, "moved once").unwrap();
    let second = automatic_notes(&mut conn, other, "moved twice").unwrap();

    // What the Undo button names is the user's action.
    let next = next_undo(&mut conn).unwrap();
    assert_eq!(
        next.operation.map(|o| (o.id, o.kind)),
        Some((user, "edit_crate".to_owned()))
    );
    assert_eq!(next.refusal, None);

    assert_eq!(undone(undo_last(&mut conn).unwrap()).id, user);
    assert_eq!(crate_notes(&conn, mine).as_deref(), Some("old notes"));
    // The automatic operations were on another row: they stand.
    assert_eq!(crate_notes(&conn, other).as_deref(), Some("moved twice"));
    assert_eq!(undone_at(&conn, first), None);
    assert_eq!(undone_at(&conn, second), None);
    assert_eq!(undo_last(&mut conn).unwrap(), UndoOutcome::NothingToUndo);
}

#[test]
fn undoing_an_action_first_takes_back_the_automatic_operations_made_since_on_its_rows() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let other = crate_row(&conn, "Peak");
    let user = set_notes(&mut conn, id, "my notes").unwrap();
    let first = automatic_notes(&mut conn, id, "moved once").unwrap();
    let elsewhere = automatic_notes(&mut conn, other, "moved elsewhere").unwrap();
    let second = automatic_notes(&mut conn, id, "moved twice").unwrap();

    assert_eq!(next_undo(&mut conn).unwrap().refusal, None);
    assert_eq!(undone(undo_last(&mut conn).unwrap()).id, user);
    // Back to what the row held before the user's action.
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("old notes"));
    assert!(undone_at(&conn, user).is_some());
    assert!(undone_at(&conn, first).is_some());
    assert!(undone_at(&conn, second).is_some());
    // The one on another row is untouched.
    assert_eq!(
        crate_notes(&conn, other).as_deref(),
        Some("moved elsewhere")
    );
    assert_eq!(undone_at(&conn, elsewhere), None);
}

#[test]
fn an_automatic_operation_older_than_the_action_is_left_alone() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let before = automatic_notes(&mut conn, id, "moved").unwrap();
    let user = set_notes(&mut conn, id, "my notes").unwrap();
    assert_eq!(undone(undo_last(&mut conn).unwrap()).id, user);
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("moved"));
    assert_eq!(undone_at(&conn, before), None);
}

#[test]
fn an_automatic_operation_that_cannot_be_taken_back_refuses_the_whole_undo_as_changed_since() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let user = record(&mut conn, "edit_crate", &no_details(), |rec| {
        rec.set("crate", id, "name", "Renamed")?;
        rec.set("crate", id, "notes", "my notes")
    })
    .unwrap()
    .operation_id
    .unwrap();
    let automatic = automatic_notes(&mut conn, id, "moved").unwrap();
    // The row changed again, outside the log.
    conn.execute("UPDATE crate SET notes = 'later' WHERE id = ?1", [id])
        .unwrap();

    // Known ahead of time, as for any refusal.
    let next = next_undo(&mut conn).unwrap();
    assert_eq!(next.operation.as_ref().map(|o| o.id), Some(user));
    assert_eq!(next.refusal, Some(UndoRefusal::ChangedSince));

    let outcome = undo_last(&mut conn).unwrap();
    let UndoOutcome::Refused {
        operation,
        reason,
        conflicts,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!(operation.id, user);
    assert_eq!(reason, UndoRefusal::ChangedSince);
    assert_eq!(
        conflicts,
        vec![UndoConflict {
            entity: "crate".into(),
            entity_id: id,
            field: Some("notes".into()),
            problem: ConflictProblem::ChangedSince,
        }]
    );
    // Nothing half done: not the name either, which could have gone back.
    assert_eq!(crate_name(&conn, id).as_deref(), Some("Renamed"));
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("later"));
    assert_eq!(undone_at(&conn, user), None);
    assert_eq!(undone_at(&conn, automatic), None);
}

#[test]
fn an_automatic_operation_already_taken_back_is_not_taken_back_twice() {
    let (_dir, mut conn) = db();
    let id = crate_row(&conn, "Warmup");
    let first = set_notes(&mut conn, id, "first").unwrap();
    let automatic = automatic_notes(&mut conn, id, "moved").unwrap();
    assert_eq!(undone(undo_last(&mut conn).unwrap()).id, first);
    let at = undone_at(&conn, automatic);
    assert!(at.is_some());
    // A new action, undone again: the old automatic operation plays no part.
    let second = set_notes(&mut conn, id, "second").unwrap();
    assert_eq!(undone(undo_last(&mut conn).unwrap()).id, second);
    assert_eq!(crate_notes(&conn, id).as_deref(), Some("old notes"));
    assert_eq!(undone_at(&conn, automatic), at);
}
