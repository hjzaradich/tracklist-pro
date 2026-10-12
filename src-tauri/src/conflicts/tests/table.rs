//! One test per row of the decision table (module docs).

use proptest::prelude::*;

use super::*;

fn decided(field: &str, sides: Sides<'_>) -> (Verdict, Reason) {
    let decision = decide(field, &sides);
    (decision.verdict, decision.reason)
}

/// Sides that would be a conflict on any field that is merged.
fn apart() -> Sides<'static> {
    sides("Deep House", "Tech House", "House")
}

// ---- Row 1 --------------------------------------------------------------

#[test]
fn an_analysis_field_is_never_merged_whatever_its_sides_hold() {
    for field in ANALYSIS_FIELDS {
        for sides in [
            sides("8A", "9A", "10A"),
            sides("8A", "8A", "10A"),
            never_sent("8A", "9A"),
            Sides {
                rekordbox: Rekordbox::Stale,
                ..sides("8A", "9A", "10A")
            },
            Sides {
                rekordbox: Rekordbox::NotThere,
                ..sides("8A", "9A", "10A")
            },
        ] {
            assert_eq!(
                decided(field, sides),
                (Verdict::Never, Reason::AnalysisField),
                "{field} {sides:?}"
            );
        }
    }
    assert_eq!(
        ANALYSIS_FIELDS,
        ["AverageBpm", "Tonality", "TEMPO", "POSITION_MARK"]
    );
}

#[test]
fn track_id_is_not_a_field() {
    assert_eq!(
        decided("TrackID", sides("7", "8", "9")),
        (Verdict::Never, Reason::NotAField)
    );
}

// ---- Row 2 --------------------------------------------------------------

#[test]
fn a_rekordbox_row_kept_from_an_older_read_decides_nothing() {
    for sides in [apart(), never_sent("Deep House", "Tech House")] {
        let stale = Sides {
            rekordbox: Rekordbox::Stale,
            ..sides
        };
        assert_eq!(
            decided("Genre", stale),
            (Verdict::Wait, Reason::StaleRead),
            "{stale:?}"
        );
    }
    // Even when the app has no value of its own to lose.
    let stale = Sides {
        app: None,
        rekordbox: Rekordbox::Stale,
        ..apart()
    };
    assert_eq!(decided("Genre", stale).0, Verdict::Wait);
}

// ---- Row 3 --------------------------------------------------------------

#[test]
fn a_track_rekordbox_does_not_have_goes_out_with_the_apps_values() {
    for app in [Some("Deep House"), None] {
        for base in [Some("House"), None] {
            let sides = Sides {
                app,
                rekordbox: Rekordbox::NotThere,
                base,
                replaced: None,
            };
            assert_eq!(
                decided("Genre", sides),
                (Verdict::SendApp, Reason::NotInRekordbox),
                "{sides:?}"
            );
        }
    }
}

// ---- Row 4 --------------------------------------------------------------

#[test]
fn a_field_the_app_holds_no_value_of_its_own_for_takes_rekordboxs() {
    for base in [Some("House"), Some("Tech House"), None] {
        let sides = Sides {
            app: None,
            rekordbox: Rekordbox::Value(Some("Tech House")),
            base,
            replaced: None,
        };
        assert_eq!(
            decided("Genre", sides),
            (Verdict::TakeRekordbox, Reason::AppFollows),
            "{sides:?}"
        );
    }
}

#[test]
fn an_empty_value_of_the_apps_own_is_a_value_not_the_lack_of_one() {
    // The app emptied the comment; rekordbox still has the one sent.
    assert_eq!(
        decided("Comments", sides("", "warm up", "warm up")),
        (Verdict::SendApp, Reason::OnlyAppChanged)
    );
}

// ---- Rows 5 to 10: the field was sent before ----------------------------

#[test]
fn a_field_neither_side_changed_is_unchanged() {
    assert_eq!(
        decided("Genre", sides("House", "House", "House")),
        (Verdict::Unchanged, Reason::NeitherChanged)
    );
}

#[test]
fn only_the_app_changed_is_sent() {
    assert_eq!(
        decided("Genre", sides("Deep House", "House", "House")),
        (Verdict::SendApp, Reason::OnlyAppChanged)
    );
}

#[test]
fn a_sent_value_rekordbox_still_shows_the_old_value_for_was_not_taken_up() {
    // The app changed House to Deep House and sent it (the base). rekordbox
    // still shows House: the send was never imported.
    let sides = replacing(sides("Deep House", "House", "Deep House"), "House");
    assert_eq!(
        decided("Genre", sides),
        (Verdict::NotTakenUp, Reason::SendNotTakenUp)
    );
}

#[test]
fn an_edit_whose_send_was_never_imported_is_not_reverted() {
    let sides = replacing(sides("Deep House", "House", "Deep House"), "House");
    let handling = decide("Genre", &sides).verdict.handling();
    // The app's value stands and goes out again; rekordbox's old value
    // isn't taken in over it, and nobody is asked.
    assert_eq!(handling, Handling::SendApp);
    assert_eq!(NOT_TAKEN_UP, Handling::SendApp);
}

#[test]
fn only_rekordbox_changed_is_taken_in() {
    // What the app's value replaced isn't known: ROADMAP 1.10's rule.
    assert_eq!(
        decided("Genre", sides("House", "Tech House", "House")),
        (Verdict::TakeRekordbox, Reason::OnlyRekordboxChanged)
    );
    // Known, and rekordbox moved on from it: a real change in rekordbox.
    let moved_on = replacing(sides("Deep House", "Tech House", "Deep House"), "House");
    assert_eq!(
        decided("Genre", moved_on),
        (Verdict::TakeRekordbox, Reason::OnlyRekordboxChanged)
    );
}

#[test]
fn both_changed_to_the_same_value_is_no_conflict() {
    assert_eq!(
        decided("Genre", sides("Deep House", "Deep House", "House")),
        (Verdict::Same, Reason::BothChangedAlike)
    );
}

#[test]
fn both_changed_to_different_values_is_a_conflict() {
    assert_eq!(
        decided("Genre", apart()),
        (Verdict::Conflict, Reason::BothChanged)
    );
    // Also for an edit made on top of a send that wasn't taken up: asking
    // is the safe answer.
    let on_top = replacing(sides("Dub Techno", "House", "Deep House"), "House");
    assert_eq!(
        decided("Genre", on_top),
        (Verdict::Conflict, Reason::BothChanged)
    );
}

// ---- Rows 11 to 13: the field was never sent ----------------------------

#[test]
fn a_never_sent_field_both_sides_agree_on_is_no_conflict() {
    assert_eq!(
        decided("Genre", never_sent("House", "House")),
        (Verdict::Same, Reason::NeverSentAlike)
    );
}

#[test]
fn a_never_sent_field_whose_rekordbox_value_is_the_one_the_app_replaced_is_sent() {
    let sides = replacing(never_sent("Deep House", "House"), "House");
    assert_eq!(
        decided("Genre", sides),
        (Verdict::SendApp, Reason::NeverSentOnlyAppChanged)
    );
}

#[test]
fn a_never_sent_field_the_sides_differ_on_is_a_conflict_with_no_base() {
    // What the app replaced isn't known.
    assert_eq!(
        decided("Genre", never_sent("Deep House", "House")),
        (Verdict::Conflict, Reason::NeverSentBothDiffer)
    );
    // Known, and rekordbox holds something else now.
    let moved_on = replacing(never_sent("Deep House", "Tech House"), "House");
    assert_eq!(
        decided("Genre", moved_on),
        (Verdict::Conflict, Reason::NeverSentBothDiffer)
    );
}

// ---- The table as a whole -----------------------------------------------

#[test]
fn an_attribute_rekordbox_does_not_write_is_an_empty_value_on_its_side() {
    let missing = |app, base| Sides {
        app: Some(app),
        rekordbox: Rekordbox::Value(None),
        base: Some(base),
        replaced: None,
    };
    assert_eq!(
        decided("Mix", missing("", "")).0,
        Verdict::Unchanged,
        "nothing on any side"
    );
    assert_eq!(decided("Mix", missing("Dub", "")).0, Verdict::SendApp);
    assert_eq!(
        decided("Mix", missing("Dub", "Dub")).0,
        Verdict::TakeRekordbox
    );
    assert_eq!(decided("Mix", missing("Dub", "Vocal")).0, Verdict::Conflict);
}

#[test]
fn the_rows_compare_by_the_fields_kind_not_by_text() {
    // rekordbox's spelling of the path, and the send's re-encoded one.
    let location = sides(
        "file://localhost/D:/Sets/Lantern%20Quay.mp3",
        "file://localhost/D:/Sets/Lantern Quay.mp3",
        "file://localhost/D:/Sets/Lantern%20Quay.mp3",
    );
    assert_eq!(decided("Location", location).0, Verdict::Unchanged);
    // Zero stars, written three ways.
    assert_eq!(
        decided("Rating", sides("0", "", "000")).0,
        Verdict::Unchanged
    );
    // The same color in the other letter case.
    assert_eq!(
        decided("Colour", sides("0xff8c00", "0xFF8C00", "0x0000FF")).0,
        Verdict::Same
    );
    // Letter case in text is a change.
    assert_eq!(
        decided("Genre", sides("house", "House", "House")).0,
        Verdict::SendApp
    );
}

#[test]
fn each_verdict_is_handled_in_one_way() {
    for (verdict, handling) in [
        (Verdict::Never, Handling::Nothing),
        (Verdict::Unchanged, Handling::Nothing),
        (Verdict::Same, Handling::Nothing),
        (Verdict::Wait, Handling::Wait),
        (Verdict::SendApp, Handling::SendApp),
        (Verdict::NotTakenUp, Handling::SendApp),
        (Verdict::TakeRekordbox, Handling::TakeRekordbox),
        (Verdict::Conflict, Handling::Review),
    ] {
        assert_eq!(verdict.handling(), handling, "{verdict:?}");
        // The one setting only moves the one verdict.
        let reviewed = verdict.handling_when(Handling::Review);
        if verdict == Verdict::NotTakenUp {
            assert_eq!(reviewed, Handling::Review);
        } else {
            assert_eq!(reviewed, handling, "{verdict:?}");
        }
    }
}

/// A value from a small set, so sides often agree: plain text, text that
/// differs only in case, spacing or Unicode form, numbers and blanks.
fn value() -> impl Strategy<Value = &'static str> {
    proptest::sample::select(vec![
        "",
        " ",
        "0",
        "00",
        "51",
        "051",
        "255",
        "7",
        "House",
        "house",
        "House ",
        "Caf\u{e9}",
        "Cafe\u{301}",
        "0xFF0000",
        "0xff0000",
        "2026-10-12",
        "file://localhost/C:/a%20b.mp3",
        "file://localhost/C:/a b.mp3",
        "file://localhost/c:/A B.MP3",
    ])
}

fn field() -> impl Strategy<Value = &'static str> {
    proptest::sample::select(vec![
        "Name",
        "Genre",
        "Comments",
        "Rating",
        "Colour",
        "PlayCount",
        "DiscNumber",
        "Year",
        "DateAdded",
        "Location",
        "FutureField",
        "Tonality",
        "TrackID",
    ])
}

proptest! {
    /// The `conflict` table wants three different texts (its CHECK), with
    /// an attribute rekordbox doesn't write stored as empty text.
    #[test]
    fn a_conflict_always_has_three_values_that_differ_as_text(
        field in field(),
        app in value(),
        rekordbox in proptest::option::of(value()),
        base in proptest::option::of(value()),
        replaced in proptest::option::of(value()),
    ) {
        let sides = Sides {
            app: Some(app),
            rekordbox: Rekordbox::Value(rekordbox),
            base,
            replaced,
        };
        if decide(field, &sides).verdict == Verdict::Conflict {
            let rekordbox = rekordbox.unwrap_or("");
            prop_assert_ne!(app, rekordbox);
            prop_assert_ne!(Some(app), base);
            prop_assert_ne!(Some(rekordbox), base);
        }
    }

    /// Whatever the app's value replaced, knowing it never turns a field
    /// into a conflict, and never makes rekordbox's value come in.
    #[test]
    fn knowing_what_the_app_replaced_only_ever_keeps_the_apps_value(
        field in field(),
        app in value(),
        rekordbox in proptest::option::of(value()),
        base in proptest::option::of(value()),
        replaced in value(),
    ) {
        let unknown = Sides {
            app: Some(app),
            rekordbox: Rekordbox::Value(rekordbox),
            base,
            replaced: None,
        };
        let known = Sides { replaced: Some(replaced), ..unknown };
        let (before, after) = (decide(field, &unknown).verdict, decide(field, &known).verdict);
        if before != after {
            prop_assert!(
                matches!(
                    (before, after),
                    (Verdict::TakeRekordbox, Verdict::NotTakenUp)
                        | (Verdict::Conflict, Verdict::SendApp)
                ),
                "{:?} became {:?}", before, after
            );
        }
    }
}
