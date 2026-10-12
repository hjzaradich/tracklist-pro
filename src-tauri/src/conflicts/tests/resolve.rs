//! Settling conflicts: one or many, each undoable as one step.

use rusqlite::types::Value;

use super::*;
use crate::ops::{OpsError, UndoRefusal};

fn usual() -> FieldSides {
    apart("Genre", "Deep House", "Tech House", "House")
}

fn rating() -> FieldSides {
    apart("Rating", "204", "153", "0")
}

/// A track with the usual conflict open on it.
fn with_open(lib: &Lib) -> (LibraryTrackId, Conflict) {
    let id = lib.bare();
    lib.detect(&[track(id, vec![usual()])]);
    (id, lib.only_open())
}

fn edit(value: &str) -> Choice {
    Choice::Edit(value.to_owned())
}

fn all() -> Filter {
    Filter::default()
}

fn of_field(field: &str) -> Filter {
    Filter {
        field: Some(field.to_owned()),
        ..Filter::default()
    }
}

// ---- One conflict ---------------------------------------------------------

#[test]
fn each_choice_settles_the_conflict_and_keeps_all_three_values() {
    for (choice, status, resolved_value, chosen) in [
        (Choice::KeepApp, Status::KeptApp, None, "Deep House"),
        (
            Choice::KeepRekordbox,
            Status::KeptRekordbox,
            None,
            "Tech House",
        ),
        (
            edit("Dub Techno"),
            Status::Edited,
            Some("Dub Techno"),
            "Dub Techno",
        ),
    ] {
        let lib = Lib::new();
        let (id, open) = with_open(&lib);

        let recorded = lib.resolve(open.id, choice.clone()).unwrap();

        let resolved = recorded.value;
        assert!(recorded.operation_id.is_some(), "{choice:?}");
        assert_eq!(resolved.was, open.id, "{choice:?}");
        assert_eq!(resolved.library_track, id, "{choice:?}");
        assert_eq!(resolved.field, "Genre", "{choice:?}");
        assert_eq!(resolved.status, status, "{choice:?}");
        assert_eq!(resolved.value, chosen, "{choice:?}");

        let rows = lib.rows();
        assert_eq!(rows.len(), 1, "{choice:?}: {rows:?}");
        let settled = &rows[0];
        assert_eq!(settled.id, resolved.conflict, "{choice:?}");
        assert_eq!(settled.status, status, "{choice:?}");
        assert_eq!(
            settled.resolved_value.as_deref(),
            resolved_value,
            "{choice:?}"
        );
        assert!(settled.resolved_at.is_some(), "{choice:?}");
        assert_eq!(settled.chosen(), Some(chosen), "{choice:?}");
        // Whatever was chosen, every side's value is still on record.
        assert_eq!(
            values(settled),
            (Some("Deep House"), Some("Tech House"), Some("House")),
            "{choice:?}"
        );
        assert_eq!(settled.library_track, id, "{choice:?}");
        assert_eq!(settled.field, "Genre", "{choice:?}");
        assert_eq!(settled.detected_at, open.detected_at, "{choice:?}");
        assert_eq!(lib.open(&all()), [], "{choice:?}");
    }
}

#[test]
fn a_settled_conflict_is_a_new_row_whose_id_was_never_used() {
    let lib = Lib::new();
    let (_id, open) = with_open(&lib);

    let resolved = lib.resolve(open.id, Choice::KeepApp).unwrap().value;

    assert!(resolved.conflict > open.id);
    assert_eq!(resolved.was, open.id);
}

#[test]
fn an_edit_that_is_one_sides_value_is_recorded_as_keeping_that_side() {
    for (given, status, chosen) in [
        // Zero stars, as the app's side spells it and otherwise.
        ("0", Status::KeptApp, ""),
        ("", Status::KeptApp, ""),
        ("255", Status::KeptRekordbox, "0255"),
        ("153", Status::Edited, "153"),
    ] {
        let lib = Lib::new();
        let id = lib.bare();
        lib.detect(&[track(id, vec![apart("Rating", "", "0255", "51")])]);
        let open = lib.only_open();

        let resolved = lib.resolve(open.id, edit(given)).unwrap().value;

        assert_eq!(resolved.status, status, "{given:?}");
        assert_eq!(resolved.value, chosen, "{given:?}");
        let settled = &lib.rows()[0];
        assert_eq!(
            settled.resolved_value.as_deref(),
            (status == Status::Edited).then_some(given),
            "{given:?}"
        );
    }
}

#[test]
fn an_edit_that_does_not_fit_the_field_is_refused_and_changes_nothing() {
    let lib = Lib::new();
    let id = lib.bare();
    lib.detect(&[track(id, vec![rating()])]);
    let before = lib.rows();

    for bad in ["100", "five", "5 stars", "-51"] {
        let refused = lib.resolve(before[0].id, edit(bad));
        assert!(
            matches!(
                &refused,
                Err(ResolveError::BadValue { field, value }) if field == "Rating" && value == bad
            ),
            "{bad}: {refused:?}"
        );
    }

    assert_eq!(lib.rows(), before);
    assert_eq!(lib.operations(), []);
}

#[test]
fn a_conflict_that_is_not_open_cannot_be_resolved() {
    let lib = Lib::new();
    let (_id, open) = with_open(&lib);
    let settled = lib.resolve(open.id, Choice::KeepApp).unwrap().value;
    let before = lib.rows();
    let logged = lib.operations();

    for id in [open.id, settled.conflict, ConflictId(999)] {
        let refused = lib.resolve(id, Choice::KeepRekordbox);
        assert!(
            matches!(&refused, Err(ResolveError::NotOpen { conflict }) if *conflict == id),
            "{id:?}: {refused:?}"
        );
    }

    assert_eq!(lib.rows(), before);
    assert_eq!(lib.operations(), logged);
}

#[test]
fn a_resolve_is_one_operation_that_says_what_was_chosen_for_which_field() {
    let lib = Lib::new();
    let (_id, open) = with_open(&lib);

    let recorded = lib.resolve(open.id, Choice::KeepRekordbox).unwrap();

    assert_eq!(
        lib.operations(),
        [(
            RESOLVE_OPERATION.to_owned(),
            serde_json::json!({ "conflicts": 1, "choice": "keepRekordbox", "field": "Genre" }),
            false
        )]
    );
    assert!(recorded.operation_id.is_some());
}

// ---- Undo -----------------------------------------------------------------

#[test]
fn undoing_a_resolve_reopens_it_with_all_three_values() {
    for choice in [Choice::KeepApp, Choice::KeepRekordbox, edit("Dub Techno")] {
        let lib = Lib::new();
        let (_id, open) = with_open(&lib);
        lib.resolve(open.id, choice.clone()).unwrap();

        assert_eq!(lib.undone(), RESOLVE_OPERATION, "{choice:?}");

        // The very row that was open: its id, its values, when it was found.
        assert_eq!(lib.rows(), [open], "{choice:?}");
    }
}

#[test]
fn a_reopened_conflict_can_be_resolved_again_another_way() {
    let lib = Lib::new();
    let (_id, open) = with_open(&lib);
    let first = lib.resolve(open.id, Choice::KeepApp).unwrap().value;
    lib.undone();

    let second = lib.resolve(open.id, Choice::KeepRekordbox).unwrap().value;

    assert!(second.conflict > first.conflict, "an id was reused");
    let rows = lib.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, Status::KeptRekordbox);
}

#[test]
fn a_detect_after_a_resolve_does_not_stand_in_the_way_of_undoing_it() {
    let lib = Lib::new();
    let (id, open) = with_open(&lib);
    lib.resolve(open.id, Choice::KeepRekordbox).unwrap();
    // The decision stands: detect writes nothing.
    lib.detect(&[track(id, vec![usual()])]);

    assert_eq!(lib.undone(), RESOLVE_OPERATION);

    assert_eq!(lib.rows(), [open.clone()]);
    // And detect finds the conflict open, as it is.
    let detected = lib.detect(&[track(id, vec![usual()])]);
    assert_eq!(detected.opened, []);
    assert_eq!(lib.rows(), [open]);
}

#[test]
fn undoing_a_resolve_is_refused_once_the_field_has_a_newer_open_conflict() {
    let lib = Lib::new();
    let (id, open) = with_open(&lib);
    lib.resolve(open.id, Choice::KeepApp).unwrap();
    // rekordbox moves on: a new conflict on the same field.
    lib.detect(&[track(
        id,
        vec![apart("Genre", "Deep House", "Minimal", "House")],
    )]);
    let before = lib.rows();
    assert_eq!(before.len(), 2);

    match lib.undo() {
        UndoOutcome::Refused {
            operation, reason, ..
        } => {
            assert_eq!(operation.kind, RESOLVE_OPERATION);
            assert_eq!(reason, UndoRefusal::ChangedSince);
        }
        other => panic!("expected the undo to be refused, got {other:?}"),
    }

    // Nothing was changed: the decision and the new conflict are both there.
    assert_eq!(lib.rows(), before);
}

// ---- Many at once ---------------------------------------------------------

/// Three tracks, each with a `Genre` and a `Rating` conflict open.
fn six_open(lib: &Lib) -> Vec<LibraryTrackId> {
    let ids = vec![lib.bare(), lib.bare(), lib.bare()];
    let sides: Vec<TrackSides> = ids
        .iter()
        .map(|id| track(*id, vec![usual(), rating()]))
        .collect();
    lib.detect(&sides);
    assert_eq!(lib.open(&all()).len(), 6);
    ids
}

#[test]
fn taking_rekordbox_for_all_ratings_settles_every_open_rating_and_nothing_else() {
    let lib = Lib::new();
    let ids = six_open(&lib);
    let genres = lib.open(&of_field("Genre"));

    let recorded = lib
        .resolve_all(&of_field("Rating"), Choice::KeepRekordbox)
        .unwrap();

    let resolved = recorded.value;
    assert_eq!(
        resolved.iter().map(|r| r.library_track).collect::<Vec<_>>(),
        ids
    );
    for one in &resolved {
        assert_eq!(one.field, "Rating");
        assert_eq!(one.status, Status::KeptRekordbox);
        assert_eq!(one.value, "153");
    }
    let settled = lib.settled();
    assert_eq!(settled.len(), 3);
    for row in &settled {
        assert_eq!(row.field, "Rating");
        assert_eq!(values(row), (Some("204"), Some("153"), Some("0")));
    }
    // The other fields' conflicts are as they were.
    assert_eq!(lib.open(&all()), genres);
}

#[test]
fn a_bulk_resolve_is_one_operation_that_says_how_many_were_settled() {
    let lib = Lib::new();
    six_open(&lib);

    let recorded = lib
        .resolve_all(&of_field("Rating"), Choice::KeepRekordbox)
        .unwrap();

    assert!(recorded.operation_id.is_some());
    assert_eq!(
        lib.operations(),
        [(
            RESOLVE_ALL_OPERATION.to_owned(),
            serde_json::json!({ "choice": "keepRekordbox", "field": "Rating", "conflicts": 3 }),
            false
        )]
    );
}

#[test]
fn one_undo_takes_back_a_bulk_resolve() {
    let lib = Lib::new();
    six_open(&lib);
    let before = lib.rows();
    lib.resolve_all(&all(), Choice::KeepApp).unwrap();
    assert_eq!(lib.open(&all()), []);

    assert_eq!(lib.undone(), RESOLVE_ALL_OPERATION);

    assert_eq!(lib.rows(), before);
    assert_eq!(lib.undo(), UndoOutcome::NothingToUndo);
}

#[test]
fn a_bulk_resolve_can_be_narrowed_to_some_tracks_or_to_a_field_of_some_tracks() {
    let lib = Lib::new();
    let ids = six_open(&lib);

    let some = Filter {
        field: None,
        tracks: Some(vec![ids[0], ids[2]]),
    };
    let resolved = lib.resolve_all(&some, Choice::KeepApp).unwrap().value;
    assert_eq!(
        resolved
            .iter()
            .map(|r| (r.library_track, r.field.as_str()))
            .collect::<Vec<_>>(),
        [
            (ids[0], "Genre"),
            (ids[0], "Rating"),
            (ids[2], "Genre"),
            (ids[2], "Rating")
        ]
    );

    let one = Filter {
        field: Some("Rating".into()),
        tracks: Some(vec![ids[1]]),
    };
    lib.resolve_all(&one, Choice::KeepRekordbox).unwrap();
    let left: Vec<(LibraryTrackId, String)> = lib
        .open(&all())
        .into_iter()
        .map(|c| (c.library_track, c.field))
        .collect();
    assert_eq!(left, [(ids[1], "Genre".to_owned())]);
}

#[test]
fn a_bulk_resolve_with_nothing_to_settle_changes_and_logs_nothing() {
    let lib = Lib::new();
    six_open(&lib);
    let before = lib.rows();

    for nothing in [
        of_field("Comments"),
        Filter {
            field: None,
            tracks: Some(vec![]),
        },
        Filter {
            field: None,
            tracks: Some(vec![LibraryTrackId(999)]),
        },
    ] {
        let recorded = lib.resolve_all(&nothing, Choice::KeepApp).unwrap();
        assert_eq!(recorded.operation_id, None, "{nothing:?}");
        assert_eq!(recorded.value, [], "{nothing:?}");
    }

    assert_eq!(lib.rows(), before);
    assert_eq!(lib.operations(), []);
}

#[test]
fn a_bulk_edit_gives_every_matched_conflict_the_value() {
    let lib = Lib::new();
    six_open(&lib);

    let resolved = lib
        .resolve_all(&of_field("Rating"), edit("102"))
        .unwrap()
        .value;

    assert_eq!(resolved.len(), 3);
    for row in lib.settled() {
        assert_eq!(row.status, Status::Edited);
        assert_eq!(row.chosen(), Some("102"));
        assert_eq!(values(&row), (Some("204"), Some("153"), Some("0")));
    }
}

#[test]
fn a_bulk_edit_that_does_not_fit_one_of_the_fields_settles_none() {
    let lib = Lib::new();
    six_open(&lib);
    let before = lib.rows();

    // A rating of rekordbox's fits `Rating`; text doesn't. Both ways round,
    // across both fields, one conflict is refused, so all are.
    let refused = lib.resolve_all(&all(), edit("Dub Techno"));

    assert!(
        matches!(
            &refused,
            Err(ResolveError::BadValue { field, .. }) if field == "Rating"
        ),
        "{refused:?}"
    );
    assert_eq!(lib.rows(), before);
    assert_eq!(lib.operations(), []);
}

// ---- The review queue -----------------------------------------------------

#[test]
fn the_review_queue_is_the_open_conflicts_oldest_first_and_can_be_narrowed() {
    let lib = Lib::new();
    let ids = six_open(&lib);
    lib.resolve_all(
        &Filter {
            field: None,
            tracks: Some(vec![ids[0]]),
        },
        Choice::KeepApp,
    )
    .unwrap();

    let queue = |filter: &Filter| -> Vec<(LibraryTrackId, String)> {
        lib.open(filter)
            .into_iter()
            .map(|c| (c.library_track, c.field))
            .collect()
    };
    let genre = |id| (id, "Genre".to_owned());
    let rating = |id| (id, "Rating".to_owned());
    assert_eq!(
        queue(&all()),
        [genre(ids[1]), rating(ids[1]), genre(ids[2]), rating(ids[2])]
    );
    assert_eq!(queue(&of_field("Rating")), [rating(ids[1]), rating(ids[2])]);
    assert_eq!(
        queue(&Filter {
            field: Some("Genre".into()),
            tracks: Some(vec![ids[2], ids[0]]),
        }),
        [genre(ids[2])]
    );
    let ids_ascend = lib.open(&all()).windows(2).all(|w| w[0].id < w[1].id);
    assert!(ids_ascend);
}

// ---- A store of the app's values joins in ----------------------------------

/// What a store would do: here, a stand-in write to another logged table,
/// named after the value chosen.
fn stand_in(rec: &mut crate::ops::Recorder<'_>, resolved: &Resolved) -> Result<(), OpsError> {
    rec.insert(
        "crate",
        &[
            ("kind", Value::Text("static".into())),
            (
                "name",
                Value::Text(format!("{} = {}", resolved.field, resolved.value)),
            ),
        ],
    )?;
    Ok(())
}

fn crates(lib: &Lib) -> Vec<String> {
    lib.writer
        .call(|c| {
            c.prepare("SELECT name FROM crate ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap()
}

#[test]
fn what_a_store_writes_while_a_conflict_is_settled_is_undone_with_it() {
    let lib = Lib::new();
    let (_id, open) = with_open(&lib);
    let id = open.id;

    lib.writer
        .call(move |c| Ok(resolve_with(c, id, &Choice::KeepRekordbox, stand_in)))
        .unwrap()
        .unwrap();
    assert_eq!(crates(&lib), ["Genre = Tech House"]);
    assert_eq!(lib.operations().len(), 1, "one operation holds both");

    lib.undone();

    assert_eq!(crates(&lib), Vec::<String>::new());
    assert_eq!(lib.rows(), [open]);
}

#[test]
fn a_store_joins_in_once_for_each_conflict_of_a_bulk_resolve_and_one_undo_takes_all_back() {
    let lib = Lib::new();
    six_open(&lib);
    let before = lib.rows();

    lib.writer
        .call(|c| {
            Ok(resolve_all_with(
                c,
                &Filter {
                    field: Some("Rating".into()),
                    tracks: None,
                },
                &Choice::KeepApp,
                |rec, resolved| {
                    // Crate names are unique: tell the three apart.
                    let mut named = resolved.clone();
                    named.value = format!("{} ({})", resolved.value, resolved.library_track.0);
                    stand_in(rec, &named)
                },
            ))
        })
        .unwrap()
        .unwrap();
    assert_eq!(crates(&lib).len(), 3);

    lib.undone();

    assert_eq!(crates(&lib), Vec::<String>::new());
    assert_eq!(lib.rows(), before);
}

#[test]
fn a_store_that_fails_leaves_the_conflict_open_and_nothing_logged() {
    let lib = Lib::new();
    let (_id, open) = with_open(&lib);
    let id = open.id;

    let failed = lib
        .writer
        .call(move |c| {
            Ok(resolve_with(c, id, &Choice::KeepApp, |rec, _| {
                rec.insert("crate", &[("no_such_column", Value::Null)])
                    .map(|_| ())
            }))
        })
        .unwrap();

    assert!(matches!(failed, Err(ResolveError::Ops(_))), "{failed:?}");
    assert_eq!(lib.rows(), [open]);
    assert_eq!(lib.operations(), []);
}
