//! The store of the app's own values (`app_values`, 1cB-18): setting,
//! clearing, Undo, a Library track's removal, and the engine reading it.

use super::*;
use crate::conflicts::app_values::{self, AppValueError, CLEAR_OPERATION, SET_OPERATION};
use crate::library;
use crate::ops::UndoRefusal;

/// One `app_value` row: id, track, field, value, replaced.
type Row = (i64, i64, String, String, Option<String>);

impl Lib {
    pub(super) fn set(
        &self,
        id: LibraryTrackId,
        field: &str,
        value: &str,
    ) -> Result<Recorded<Option<AppValue>>, AppValueError> {
        let (field, value) = (field.to_owned(), value.to_owned());
        self.writer
            .call(move |c| Ok(app_values::set(c, &Mount, id, &field, &value)))
            .unwrap()
    }

    fn clear(&self, id: LibraryTrackId, field: &str) -> Result<Recorded<bool>, AppValueError> {
        let field = field.to_owned();
        self.writer
            .call(move |c| Ok(app_values::clear(c, id, &field)))
            .unwrap()
    }

    fn app_rows(&self) -> Vec<Row> {
        self.writer
            .call(|c| {
                c.prepare(
                    "SELECT id, library_track_id, field, value, replaced FROM app_value
                     ORDER BY id",
                )?
                .query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect()
            })
            .unwrap()
    }

    fn stored(&self, ids: &[LibraryTrackId]) -> AppValues {
        let ids = ids.to_vec();
        self.writer
            .call(move |c| app_values::read(c, &ids))
            .unwrap()
    }

    pub(super) fn detect_stored(&self, ids: &[LibraryTrackId]) -> Detected {
        let ids = ids.to_vec();
        self.writer
            .call(move |c| detect_from_store(c, &Mount, &ids))
            .unwrap()
    }

    /// A new read of rekordbox's export in which this track's entry holds
    /// these attributes.
    fn rekordbox_now(&self, id: LibraryTrackId, attributes: &[(&str, &str)]) {
        let file: i64 = self
            .writer
            .call(move |c| {
                let file = c.query_row(
                    "SELECT linked_file_id FROM library_track WHERE id = ?1",
                    [id.0],
                    |r| r.get(0),
                )?;
                c.execute("DELETE FROM rekordbox_track WHERE file_id = ?1", [file])?;
                Ok(file)
            })
            .unwrap();
        self.entry(file, THIS_READ, false, attributes);
    }
}

fn own(field: &str, value: &str, replaced: Option<&str>) -> AppValue {
    AppValue {
        field: field.to_owned(),
        value: value.to_owned(),
        replaced: replaced.map(str::to_owned),
    }
}

fn row(id: i64, track: LibraryTrackId, field: &str, value: &str, replaced: Option<&str>) -> Row {
    (
        id,
        track.0,
        field.to_owned(),
        value.to_owned(),
        replaced.map(str::to_owned),
    )
}

// ---- Setting and clearing ----------------------------------------------------

#[test]
fn a_value_set_and_cleared_round_trips_through_the_store() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House")]);

    let set = lib.set(id, "Genre", "Deep House").unwrap();
    assert!(set.operation_id.is_some());
    assert_eq!(set.value, Some(own("Genre", "Deep House", Some("House"))));
    let rows = lib.app_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0],
        row(rows[0].0, id, "Genre", "Deep House", Some("House"))
    );
    assert_eq!(
        lib.stored(&[id]).of(id),
        [own("Genre", "Deep House", Some("House"))]
    );

    let cleared = lib.clear(id, "Genre").unwrap();
    assert!(cleared.value);
    assert!(cleared.operation_id.is_some());
    assert_eq!(lib.app_rows(), []);
    assert_eq!(lib.stored(&[id]), AppValues::default());

    // Nothing left to clear: nothing logged.
    let again = lib.clear(id, "Genre").unwrap();
    assert!(!again.value);
    assert_eq!(again.operation_id, None);
    let kinds: Vec<String> = lib.operations().into_iter().map(|o| o.0).collect();
    assert_eq!(kinds, [SET_OPERATION, CLEAR_OPERATION]);
}

#[test]
fn the_value_is_stored_as_typed_and_setting_it_again_logs_nothing() {
    let lib = Lib::new();
    let id = lib.known(&[("TrackNumber", "7"), ("Name", "Velo")]);

    lib.set(id, "TrackNumber", "008").unwrap();
    lib.set(id, "Name", "  velo ").unwrap();
    let values: Vec<String> = lib.app_rows().into_iter().map(|r| r.3).collect();
    assert_eq!(values, ["008", "  velo "]);

    let again = lib.set(id, "TrackNumber", "008").unwrap();
    assert_eq!(again.operation_id, None);
    assert_eq!(lib.operations().len(), 2);
}

#[test]
fn editing_a_field_again_keeps_the_value_the_first_edit_replaced() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    let first = lib.app_rows();
    // rekordbox moves on; the app's value replaced House all the same.
    lib.rekordbox_now(id, &[("Genre", "Tech House")]);

    let again = lib.set(id, "Genre", "Afro House").unwrap();

    assert_eq!(again.value, Some(own("Genre", "Afro House", Some("House"))));
    assert_eq!(
        lib.app_rows(),
        [row(first[0].0, id, "Genre", "Afro House", Some("House"))]
    );
}

#[test]
fn setting_rekordboxs_own_value_clears_the_apps_own() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House"), ("TrackNumber", "7")]);
    lib.set(id, "Genre", "Deep House").unwrap();

    let back = lib.set(id, "Genre", "House").unwrap();

    assert_eq!(back.value, None);
    assert!(back.operation_id.is_some());
    assert_eq!(lib.app_rows(), []);
    // Equal by the field's kind: rekordbox's 7 is 007. Nothing to store,
    // nothing logged.
    let same = lib.set(id, "TrackNumber", "007").unwrap();
    assert_eq!(same.value, None);
    assert_eq!(same.operation_id, None);
    assert_eq!(lib.app_rows(), []);
    // Letter case is a change for text (1.10).
    assert!(lib.set(id, "Genre", "house").unwrap().value.is_some());
}

#[test]
fn what_an_edit_replaced_is_empty_for_an_attribute_rekordbox_lacks_and_unknown_without_an_entry() {
    let lib = Lib::new();
    let known = lib.known(&[("Genre", "House")]);
    let (not_in_rekordbox, _) = lib.linked();
    let (stale, file) = lib.linked();
    lib.entry(file, EARLIER_READ, false, &[("Genre", "House")]);
    let (probable, file) = lib.linked();
    lib.entry(file, THIS_READ, true, &[("Genre", "House")]);

    let edit = |id| lib.set(id, "Genre", "House").unwrap().value;
    assert_eq!(
        lib.set(known, "Comments", "warm-up").unwrap().value,
        Some(own("Comments", "warm-up", Some("")))
    );
    // Without a trusted entry of the newest read nothing is known of
    // rekordbox's value, so a value is kept even when it looks the same.
    for id in [not_in_rekordbox, stale, probable] {
        assert_eq!(edit(id), Some(own("Genre", "House", None)), "{id:?}");
    }
}

// ---- Refused -----------------------------------------------------------------

#[test]
fn analysis_fields_track_id_and_location_are_never_the_apps() {
    let lib = Lib::new();
    let id = lib.known(&[("AverageBpm", "124.00"), ("Tonality", "Am")]);
    let never = ANALYSIS_FIELDS
        .into_iter()
        .chain(["TrackID", "Location", ""]);
    for field in never {
        assert!(!app_values::editable(field), "{field}");
        assert!(
            matches!(
                lib.set(id, field, "1"),
                Err(AppValueError::NotEditable { .. })
            ),
            "{field}"
        );
        assert!(
            matches!(lib.clear(id, field), Err(AppValueError::NotEditable { .. })),
            "{field}"
        );
    }
    assert_eq!(lib.app_rows(), []);
    assert_eq!(lib.operations(), []);
    assert!(app_values::editable("Genre") && app_values::editable("Rating"));
}

#[test]
fn a_value_not_of_the_fields_kind_or_an_unknown_track_is_refused() {
    let lib = Lib::new();
    let id = lib.known(&[("Rating", "0")]);
    for (field, value) in [("Rating", "lots"), ("Rating", "100"), ("Colour", "red")] {
        assert!(
            matches!(
                lib.set(id, field, value),
                Err(AppValueError::BadValue { .. })
            ),
            "{field} {value}"
        );
    }
    assert!(matches!(
        lib.set(LibraryTrackId(999), "Genre", "House"),
        Err(AppValueError::NoTrack(LibraryTrackId(999)))
    ));
    assert!(matches!(
        lib.clear(LibraryTrackId(999), "Genre"),
        Err(AppValueError::NoTrack(_))
    ));
    assert_eq!(lib.app_rows(), []);
    assert_eq!(lib.operations(), []);
}

// ---- Undo --------------------------------------------------------------------

#[test]
fn undo_of_a_set_and_of_a_clear_restores_exactly() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House"), ("Rating", "51")]);
    lib.set(id, "Rating", "255").unwrap();
    let one = lib.app_rows();
    lib.set(id, "Genre", "Deep House").unwrap();
    let two = lib.app_rows();
    lib.set(id, "Genre", "Afro House").unwrap();
    let edited = lib.app_rows();
    lib.clear(id, "Rating").unwrap();
    assert_eq!(lib.app_rows().len(), 1);

    assert_eq!(lib.undone(), CLEAR_OPERATION);
    assert_eq!(lib.app_rows(), edited, "the row is back under its own id");
    assert_eq!(lib.undone(), SET_OPERATION);
    assert_eq!(lib.app_rows(), two);
    assert_eq!(lib.undone(), SET_OPERATION);
    assert_eq!(lib.app_rows(), one);
    assert_eq!(lib.undone(), SET_OPERATION);
    assert_eq!(lib.app_rows(), []);
    assert_eq!(lib.undo(), UndoOutcome::NothingToUndo);
}

#[test]
fn undo_of_a_set_that_cleared_puts_the_apps_value_back() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    let before = lib.app_rows();
    lib.set(id, "Genre", "House").unwrap();

    assert_eq!(lib.undone(), SET_OPERATION);
    assert_eq!(lib.app_rows(), before);
}

#[test]
fn undo_is_refused_when_the_value_changed_since() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    lib.set(id, "Genre", "Afro House").unwrap();
    // Changed outside the log, as the send and the merge before it may.
    lib.run("UPDATE app_value SET value = 'Tech House'");
    let before = lib.app_rows();

    match lib.undo() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("expected the undo to be refused, got {other:?}"),
    }
    assert_eq!(lib.app_rows(), before);
}

#[test]
fn removing_a_library_track_removes_its_own_values_and_undo_brings_them_back() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House"), ("Rating", "51")]);
    let other = lib.known(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    lib.set(id, "Rating", "255").unwrap();
    lib.set(other, "Genre", "Disco").unwrap();
    let before = lib.app_rows();

    library::remove(&lib.writer, id).unwrap();

    let left: Vec<i64> = lib.app_rows().into_iter().map(|r| r.1).collect();
    assert_eq!(left, [other.0]);
    assert_eq!(lib.undone(), library::REMOVE_OPERATION);
    assert_eq!(lib.app_rows(), before);
}

// ---- The engine reads the store ----------------------------------------------

#[test]
fn a_stored_edit_only_the_app_made_is_sent() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House"), ("Name", "Velo")]);
    lib.sent(id, &[("Genre", "House"), ("Name", "Velo")]);
    lib.set(id, "Genre", "Deep House").unwrap();

    let detected = lib.detect_stored(&[id]);

    assert_eq!(lib.rows(), []);
    assert_eq!(detected.fields.len(), 1, "{detected:?}");
    let genre = &detected.fields[0];
    assert_eq!(genre.field, "Genre");
    assert_eq!(genre.decision.reason, Reason::OnlyAppChanged);
    assert_eq!(genre.outcome, Outcome::SendApp);
}

#[test]
fn a_stored_edit_rekordbox_also_changed_is_a_conflict() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House")]);
    lib.sent(id, &[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    lib.rekordbox_now(id, &[("Genre", "Tech House")]);

    let detected = lib.detect_stored(&[id]);

    let open = lib.only_open();
    assert_eq!(
        values(&open),
        (Some("Deep House"), Some("Tech House"), Some("House"))
    );
    assert_eq!(detected.opened, [open.id]);
}

#[test]
fn a_stored_edit_whose_send_was_never_imported_stands() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    // Sent over House, and rekordbox still shows House: the XML wasn't
    // imported. A field sent counts what the send replaced (1cB-19).
    lib.sent(id, &[("Genre", "Deep House")]);
    lib.run("UPDATE sync_base SET replaced = 'House'");

    let detected = lib.detect_stored(&[id]);

    assert_eq!(lib.rows(), []);
    assert_eq!(detected.fields.len(), 1, "{detected:?}");
    assert_eq!(detected.fields[0].decision.verdict, Verdict::NotTakenUp);
    assert_eq!(detected.fields[0].outcome, Outcome::SendApp);
}

#[test]
fn a_stored_edit_never_sent_goes_out_while_rekordbox_shows_what_it_replaced() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();

    let detected = lib.detect_stored(&[id]);
    assert_eq!(
        detected.fields[0].decision.reason,
        Reason::NeverSentOnlyAppChanged
    );
    assert_eq!(lib.rows(), []);

    // Once rekordbox shows something else, neither side is proved unchanged.
    lib.rekordbox_now(id, &[("Genre", "Tech House")]);
    lib.detect_stored(&[id]);
    assert_eq!(
        values(&lib.only_open()),
        (Some("Deep House"), Some("Tech House"), None)
    );
}

#[test]
fn a_field_without_a_stored_value_follows_rekordbox() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "Tech House")]);
    lib.sent(id, &[("Genre", "House")]);
    let other = lib.known(&[("Genre", "House")]);
    lib.set(other, "Genre", "Deep House").unwrap();

    let detected = lib.detect_stored(&[id]);

    assert_eq!(detected, Detected::default());
    assert_eq!(lib.rows(), []);
}

// ---- A resolve takes the value chosen into the store (1cB-3) -----------------

/// A track sent with Genre House, edited to Deep House in the app and to
/// Tech House in rekordbox since: an open conflict.
fn edited_on_both_sides(lib: &Lib) -> LibraryTrackId {
    let id = lib.known(&[("Genre", "House")]);
    lib.sent(id, &[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    lib.rekordbox_now(id, &[("Genre", "Tech House")]);
    lib.detect_stored(&[id]);
    id
}

#[test]
fn keeping_rekordboxs_value_clears_the_apps_own_and_undo_puts_it_back() {
    let lib = Lib::new();
    let id = edited_on_both_sides(&lib);
    let open = lib.only_open();
    let before = lib.app_rows();

    lib.resolve(open.id, Choice::KeepRekordbox).unwrap();

    assert_eq!(lib.app_rows(), []);
    // The field follows rekordbox again: nothing of the app's goes out.
    assert_eq!(lib.detect_stored(&[id]).fields, []);
    assert_eq!(lib.operations().len(), 2, "the edit, then the resolve");
    assert_eq!(lib.undone(), RESOLVE_OPERATION);
    assert_eq!(lib.app_rows(), before);
    assert_eq!(lib.only_open().id, open.id);
}

#[test]
fn keeping_the_apps_value_leaves_its_own_and_logs_nothing_more() {
    let lib = Lib::new();
    let id = edited_on_both_sides(&lib);
    let open = lib.only_open();
    let before = lib.app_rows();

    lib.resolve(open.id, Choice::KeepApp).unwrap();

    assert_eq!(lib.app_rows(), before);
    let detected = lib.detect_stored(&[id]);
    assert_eq!(detected.fields.len(), 1, "{detected:?}");
    assert!(matches!(
        &detected.fields[0].outcome,
        Outcome::Decided { value, .. } if value == "Deep House"
    ));
}

#[test]
fn a_value_given_by_hand_becomes_the_apps_own_keeping_what_the_edit_replaced() {
    let lib = Lib::new();
    let id = edited_on_both_sides(&lib);
    let open = lib.only_open();
    let edit_id = lib.app_rows()[0].0;

    lib.resolve(open.id, Choice::Edit("Disco".into())).unwrap();

    assert_eq!(
        lib.app_rows(),
        [row(edit_id, id, "Genre", "Disco", Some("House"))]
    );
    assert_eq!(lib.undone(), RESOLVE_OPERATION);
    assert_eq!(
        lib.app_rows(),
        [row(edit_id, id, "Genre", "Deep House", Some("House"))]
    );
}

#[test]
fn a_value_chosen_for_a_field_the_store_has_none_of_is_stored_over_rekordboxs() {
    // A conflict the store holds no value for (the app's value came from
    // elsewhere): keeping the app's stores it, with rekordbox's value as
    // the one it replaced.
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "Tech House")]);
    lib.detect(&[track(
        id,
        vec![apart("Genre", "Deep House", "Tech House", "House")],
    )]);
    let open = lib.only_open();

    lib.resolve(open.id, Choice::KeepApp).unwrap();

    let rows = lib.app_rows();
    assert_eq!(
        rows,
        [row(
            rows[0].0,
            id,
            "Genre",
            "Deep House",
            Some("Tech House")
        )]
    );
}

#[test]
fn a_bulk_resolve_takes_every_value_chosen_into_the_store_in_one_step() {
    let lib = Lib::new();
    let one = edited_on_both_sides(&lib);
    let two = edited_on_both_sides(&lib);
    let before = lib.app_rows();
    assert_eq!(before.len(), 2);

    lib.resolve_all(&Filter::default(), Choice::KeepRekordbox)
        .unwrap();

    assert_eq!(lib.app_rows(), []);
    assert_eq!(lib.detect_stored(&[one, two]).fields, []);
    assert_eq!(lib.undone(), RESOLVE_ALL_OPERATION);
    assert_eq!(lib.app_rows(), before);
}

// ---- Taking rekordbox's value in (1cB-3) -------------------------------------

#[test]
fn undo_of_an_edit_is_refused_once_the_merge_took_rekordboxs_value_in() {
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House")]);
    lib.set(id, "Genre", "Deep House").unwrap();
    // Sent, imported, then changed in rekordbox: only rekordbox changed.
    lib.sent(id, &[("Genre", "Deep House")]);
    lib.rekordbox_now(id, &[("Genre", "Tech House")]);

    let merged = lib
        .writer
        .call(move |c| merge_before_send(c, &Mount, &[id]))
        .unwrap();
    assert_eq!(merged.taken_in, [(id, "Genre".to_owned())]);
    assert_eq!(merged.chosen, std::collections::HashMap::new());
    assert_eq!(lib.app_rows(), []);
    let operations = lib.operations();

    match lib.undo() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("expected the undo to be refused, got {other:?}"),
    }
    assert_eq!(lib.app_rows(), []);
    assert_eq!(lib.operations(), operations, "nothing undone or logged");
}
