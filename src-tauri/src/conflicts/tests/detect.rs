//! Keeping the `conflict` table in step with the sides: what is opened,
//! updated and withdrawn, and what is never touched.

use super::*;
use crate::library;

/// A `Genre` all three sides differ on.
fn genre(app: &str, rekordbox: &str, base: &str) -> FieldSides {
    apart("Genre", app, rekordbox, base)
}

/// The usual conflict: the app says Deep House, rekordbox Tech House, and
/// House was sent.
fn usual() -> FieldSides {
    genre("Deep House", "Tech House", "House")
}

/// A track with the usual conflict open on it. Returns the conflict's id.
fn with_open(lib: &Lib) -> (LibraryTrackId, ConflictId) {
    let id = lib.bare();
    let detected = lib.detect(&[track(id, vec![usual()])]);
    assert_eq!(detected.opened.len(), 1, "{detected:?}");
    (id, detected.opened[0])
}

fn none() -> Vec<ConflictId> {
    Vec::new()
}

fn own(field: &str, value: &str, replaced: Option<&str>) -> AppValue {
    AppValue {
        field: field.to_owned(),
        value: value.to_owned(),
        replaced: replaced.map(str::to_owned),
    }
}

// ---- Opening --------------------------------------------------------------

#[test]
fn a_field_both_sides_changed_differently_opens_a_conflict_holding_all_three_values() {
    let lib = Lib::new();
    let id = lib.bare();

    let detected = lib.detect(&[track(id, vec![usual()])]);

    let open = lib.only_open();
    assert_eq!(open.library_track, id);
    assert_eq!(open.field, "Genre");
    assert_eq!(
        values(&open),
        (Some("Deep House"), Some("Tech House"), Some("House"))
    );
    assert_eq!(open.status, Status::Open);
    assert_eq!(open.resolved_value, None);
    assert_eq!(open.resolved_at, None);
    assert!(!open.detected_at.is_empty());
    assert_eq!(detected.opened, [open.id]);
    assert_eq!(detected.updated, none());
    assert_eq!(detected.withdrawn, none());
    assert_eq!(
        detected.fields,
        [FieldOutcome {
            library_track: id,
            field: "Genre".into(),
            decision: Decision {
                verdict: Verdict::Conflict,
                reason: Reason::BothChanged,
            },
            outcome: Outcome::Open { conflict: open.id },
        }]
    );
}

#[test]
fn a_conflict_on_a_field_that_was_never_sent_is_recorded_without_a_base() {
    let lib = Lib::new();
    let id = lib.bare();

    lib.detect(&[track(
        id,
        vec![field("Genre", Some("Deep House"), Some("Tech House"), None)],
    )]);

    assert_eq!(
        values(&lib.only_open()),
        (Some("Deep House"), Some("Tech House"), None)
    );
}

#[test]
fn an_attribute_rekordboxs_entry_does_not_have_is_recorded_as_an_empty_value() {
    let lib = Lib::new();
    let id = lib.bare();

    lib.detect(&[track(
        id,
        vec![field("Mix", Some("Dub"), None, Some("Vocal"))],
    )]);

    assert_eq!(
        values(&lib.only_open()),
        (Some("Dub"), Some(""), Some("Vocal"))
    );
}

#[test]
fn each_field_of_each_track_gets_its_own_conflict() {
    let lib = Lib::new();
    let (one, two) = (lib.bare(), lib.bare());

    lib.detect(&[
        track(one, vec![usual(), apart("Rating", "204", "153", "0")]),
        track(two, vec![usual()]),
    ]);

    let open: Vec<(LibraryTrackId, String)> = lib
        .open(&Filter::default())
        .into_iter()
        .map(|c| (c.library_track, c.field))
        .collect();
    assert_eq!(
        open,
        [
            (one, "Genre".to_owned()),
            (one, "Rating".to_owned()),
            (two, "Genre".to_owned())
        ]
    );
}

// ---- A rerun --------------------------------------------------------------

#[test]
fn a_rerun_on_unchanged_inputs_writes_nothing() {
    let lib = Lib::new();
    let (busy, stale, settled) = (lib.bare(), lib.bare(), lib.bare());
    let sides = vec![
        track(
            busy,
            vec![
                usual(),
                apart("Rating", "204", "153", "0"),
                // Unchanged, only the app, only rekordbox, both alike.
                field("Name", Some("Velo"), Some("Velo"), Some("Velo")),
                field("Comments", Some("warm up"), Some(""), Some("")),
                field(
                    "Label",
                    Some("Kestrel"),
                    Some("Kestrel Nine"),
                    Some("Kestrel"),
                ),
                field("Mix", Some("Dub"), Some("Dub"), Some("")),
                // The app holds no value of its own.
                field("Album", None, Some("Lantern Quay"), Some("Harbor Quay")),
            ],
        ),
        TrackSides {
            entry: RekordboxEntry::Stale,
            ..track(stale, vec![usual()])
        },
        track(settled, vec![usual()]),
    ];
    lib.detect(&sides);
    let decided = lib
        .open(&Filter {
            tracks: Some(vec![settled]),
            ..Filter::default()
        })
        .remove(0);
    lib.settle_by_hand(decided.id, Status::KeptRekordbox, None);
    let first = lib.detect(&sides);
    let before = lib.rows();
    assert_eq!(before.len(), 3, "{before:?}");

    let again = sides.clone();
    let (second, written) = lib.written_by(move |c| detect(c, &again));

    assert_eq!(written, Vec::<String>::new());
    assert_eq!(lib.rows(), before);
    assert_eq!(second.opened, none());
    assert_eq!(second.updated, none());
    assert_eq!(second.withdrawn, none());
    assert_eq!(second.fields, first.fields);
}

// ---- A side changes again -------------------------------------------------

#[test]
fn a_side_changing_again_updates_the_open_conflict() {
    let lib = Lib::new();
    let (id, conflict) = with_open(&lib);
    // So that a new `detected_at` would show.
    lib.run("UPDATE conflict SET detected_at = '2026-01-01T00:00:00.000Z'");

    for (changed, sides) in [
        ("the app's", genre("Dub Techno", "Tech House", "House")),
        ("rekordbox's", genre("Dub Techno", "Minimal", "House")),
        ("the base", genre("Dub Techno", "Minimal", "Electro")),
    ] {
        let expected = (
            sides.app.clone().unwrap(),
            sides.rekordbox.clone().unwrap(),
            sides.base.clone().unwrap(),
        );
        let detected = lib.detect(&[track(id, vec![sides])]);

        let open = lib.only_open();
        assert_eq!(open.id, conflict, "{changed}: it is the same conflict");
        assert_eq!(open.detected_at, "2026-01-01T00:00:00.000Z", "{changed}");
        assert_eq!(
            values(&open),
            (
                Some(expected.0.as_str()),
                Some(expected.1.as_str()),
                Some(expected.2.as_str())
            ),
            "{changed}"
        );
        assert_eq!(detected.updated, [conflict], "{changed}");
        assert_eq!(detected.opened, none(), "{changed}");
        assert_eq!(detected.withdrawn, none(), "{changed}");
        assert_eq!(lib.rows().len(), 1, "{changed}");
    }
}

#[test]
fn a_base_that_goes_away_or_arrives_updates_the_open_conflict_too() {
    let lib = Lib::new();
    let (id, conflict) = with_open(&lib);

    lib.detect(&[track(
        id,
        vec![field("Genre", Some("Deep House"), Some("Tech House"), None)],
    )]);
    let open = lib.only_open();
    assert_eq!(open.id, conflict);
    assert_eq!(values(&open).2, None);

    lib.detect(&[track(id, vec![usual()])]);
    let open = lib.only_open();
    assert_eq!(open.id, conflict);
    assert_eq!(values(&open).2, Some("House"));
}

// ---- The sides agree again ------------------------------------------------

#[test]
fn sides_that_come_to_agree_withdraw_it() {
    let agreeing = [
        (
            "both now hold the same value",
            field("Genre", Some("Minimal"), Some("Minimal"), Some("House")),
        ),
        (
            "the app is back at the base",
            field("Genre", Some("House"), Some("Tech House"), Some("House")),
        ),
        (
            "rekordbox is back at the base",
            field("Genre", Some("Deep House"), Some("House"), Some("House")),
        ),
        (
            "everything is the base again",
            field("Genre", Some("House"), Some("House"), Some("House")),
        ),
        (
            "a send carried the app's value and rekordbox took it",
            field(
                "Genre",
                Some("Deep House"),
                Some("Deep House"),
                Some("Deep House"),
            ),
        ),
        (
            "the app no longer holds a value of its own",
            field("Genre", None, Some("Tech House"), Some("House")),
        ),
        (
            "nothing holds the field any more",
            field("Genre", None, None, None),
        ),
    ];
    for (why, sides) in agreeing {
        let lib = Lib::new();
        let (id, conflict) = with_open(&lib);

        let detected = lib.detect(&[track(id, vec![sides])]);

        assert_eq!(lib.rows(), [], "{why}");
        assert_eq!(detected.withdrawn, [conflict], "{why}");
        assert_eq!(detected.opened, none(), "{why}");
        assert_eq!(detected.updated, none(), "{why}");
    }
}

// ---- A track rekordbox no longer has -------------------------------------

/// The sides of a track rekordbox has no trusted entry for.
fn not_there(id: LibraryTrackId, fields: Vec<FieldSides>) -> TrackSides {
    TrackSides {
        entry: RekordboxEntry::NotThere,
        ..track(id, fields)
    }
}

#[test]
fn a_track_rekordbox_no_longer_has_keeps_its_open_conflicts_untouched() {
    let lib = Lib::new();
    let (id, conflict) = with_open(&lib);
    let before = lib.rows();

    // Whatever the app's side holds now, and with a second field of the
    // app's own that has no conflict.
    for app in ["Deep House", "Breaks", "House"] {
        let gone = vec![not_there(
            id,
            vec![
                genre(app, "Tech House", "House"),
                field("Label", Some("Kestrel"), None, Some("Kestrel Nine")),
            ],
        )];
        let (detected, written) = lib.written_by(move |c| detect(c, &gone));

        assert_eq!(written, Vec::<String>::new(), "{app}");
        // The conflict holds the app's only copy of rekordbox's value.
        assert_eq!(lib.rows(), before, "{app}");
        assert_eq!(detected.withdrawn, none(), "{app}");
        let listed: Vec<(&str, &Outcome)> = detected
            .fields
            .iter()
            .map(|f| (f.field.as_str(), &f.outcome))
            .collect();
        assert_eq!(
            listed,
            [
                ("Genre", &Outcome::Open { conflict }),
                // Nothing of rekordbox's to merge: it goes out as new.
                ("Label", &Outcome::SendApp),
            ],
            "{app}"
        );
    }
}

#[test]
fn a_track_that_comes_back_in_a_later_export_still_has_its_conflict_once() {
    let lib = Lib::new();
    let (id, conflict) = with_open(&lib);
    let before = lib.rows();
    lib.detect(&[not_there(id, vec![usual()])]);

    // Back, with the values it had: the very same conflict, and only it.
    let back = vec![track(id, vec![usual()])];
    let (detected, written) = lib.written_by(move |c| detect(c, &back));

    assert_eq!(written, Vec::<String>::new());
    assert_eq!(lib.rows(), before);
    assert_eq!(detected.opened, none());
    assert_eq!(detected.fields[0].outcome, Outcome::Open { conflict });
    assert_eq!(
        values(&lib.only_open()),
        (Some("Deep House"), Some("Tech House"), Some("House"))
    );
}

#[test]
fn a_track_that_comes_back_changed_has_its_conflict_decided_again_by_the_usual_rules() {
    // Back with another value on rekordbox's side: the same conflict, now
    // holding the values as they are.
    let lib = Lib::new();
    let (id, conflict) = with_open(&lib);
    lib.detect(&[not_there(id, vec![usual()])]);
    let detected = lib.detect(&[track(id, vec![genre("Deep House", "Minimal", "House")])]);
    let open = lib.only_open();
    assert_eq!(open.id, conflict);
    assert_eq!(
        values(&open),
        (Some("Deep House"), Some("Minimal"), Some("House"))
    );
    assert_eq!(detected.updated, [conflict]);

    // Back with rekordbox holding the app's value: nothing left to decide.
    let lib = Lib::new();
    let (id, conflict) = with_open(&lib);
    lib.detect(&[not_there(id, vec![usual()])]);
    let detected = lib.detect(&[track(id, vec![genre("Deep House", "Deep House", "House")])]);
    assert_eq!(lib.rows(), []);
    assert_eq!(detected.withdrawn, [conflict]);
}

#[test]
fn a_withdrawn_conflict_that_comes_back_is_a_new_conflict() {
    let lib = Lib::new();
    let (id, first) = with_open(&lib);
    lib.detect(&[track(
        id,
        vec![field("Genre", Some("House"), Some("House"), Some("House"))],
    )]);

    let detected = lib.detect(&[track(id, vec![usual()])]);

    assert!(detected.opened[0] > first, "{detected:?}");
    assert_eq!(lib.only_open().id, detected.opened[0]);
}

// ---- A stale rekordbox row ------------------------------------------------

#[test]
fn a_stale_rekordbox_row_leaves_everything_alone() {
    let lib = Lib::new();
    let (id, _conflict) = with_open(&lib);
    let untouched = lib.bare();
    let before = lib.rows();

    for (why, sides) in [
        (
            "the sides would agree",
            field("Genre", Some("House"), Some("House"), Some("House")),
        ),
        (
            "the sides would differ in another way",
            genre("Dub Techno", "Minimal", "Electro"),
        ),
        ("the sides are the same", usual()),
    ] {
        let stale = vec![
            TrackSides {
                entry: RekordboxEntry::Stale,
                ..track(id, vec![sides])
            },
            // No conflict is opened from a stale row either.
            TrackSides {
                entry: RekordboxEntry::Stale,
                ..track(untouched, vec![usual()])
            },
        ];
        let (detected, written) = lib.written_by(move |c| detect(c, &stale));

        assert_eq!(written, Vec::<String>::new(), "{why}");
        assert_eq!(lib.rows(), before, "{why}");
        assert_eq!(
            detected
                .fields
                .iter()
                .map(|f| (&f.outcome, f.decision.verdict))
                .collect::<Vec<_>>(),
            [
                (&Outcome::Wait, Verdict::Wait),
                (&Outcome::Wait, Verdict::Wait)
            ],
            "{why}"
        );
    }
}

// ---- Settled conflicts ----------------------------------------------------

/// Every way a conflict is settled, with the value chosen for the usual
/// conflict.
fn settlements() -> [(Status, Option<&'static str>, &'static str); 3] {
    [
        (Status::KeptApp, None, "Deep House"),
        (Status::KeptRekordbox, None, "Tech House"),
        (Status::Edited, Some("Dub Techno"), "Dub Techno"),
    ]
}

#[test]
fn detect_never_touches_a_settled_conflict() {
    for (status, edited, _chosen) in settlements() {
        let lib = Lib::new();
        let (id, conflict) = with_open(&lib);
        lib.settle_by_hand(conflict, status, edited);
        let before = lib.settled();
        assert_eq!(before.len(), 1);

        let current = |fields| track(id, fields);
        for (why, sides) in [
            ("the same sides", current(vec![usual()])),
            (
                "sides that agree",
                current(vec![field(
                    "Genre",
                    Some("House"),
                    Some("House"),
                    Some("House"),
                )]),
            ),
            (
                "sides that differ in another way",
                current(vec![genre("Electro", "Minimal", "Breaks")]),
            ),
            (
                "no sides at all",
                current(vec![field("Genre", None, None, None)]),
            ),
            (
                "another field only",
                current(vec![apart("Rating", "204", "153", "0")]),
            ),
            (
                "a stale row",
                TrackSides {
                    entry: RekordboxEntry::Stale,
                    ..current(vec![usual()])
                },
            ),
            (
                "a track rekordbox no longer has",
                TrackSides {
                    entry: RekordboxEntry::NotThere,
                    ..current(vec![usual()])
                },
            ),
        ] {
            lib.detect(&[sides]);
            assert_eq!(lib.settled(), before, "{status:?}, {why}");
        }
    }
}

#[test]
fn a_resolution_stands_until_a_side_moves() {
    for (status, edited, chosen) in settlements() {
        let lib = Lib::new();
        let (id, conflict) = with_open(&lib);
        lib.settle_by_hand(conflict, status, edited);
        let before = lib.rows();

        // The same sides: no conflict again, and the value chosen goes out.
        let detected = lib.detect(&[track(id, vec![usual()])]);
        assert_eq!(lib.rows(), before, "{status:?}");
        assert_eq!(detected.opened, none(), "{status:?}");
        assert_eq!(
            detected.fields[0].outcome,
            Outcome::Decided {
                conflict,
                value: chosen.to_owned()
            },
            "{status:?}"
        );

        // The app has taken the value chosen as its own: still settled,
        // and the value chosen is still what goes out.
        let adopted = genre(chosen, "Tech House", "House");
        let detected = lib.detect(&[track(id, vec![adopted])]);
        assert_eq!(lib.rows(), before, "{status:?}, adopted");
        assert_eq!(detected.opened, none(), "{status:?}, adopted");
        assert_eq!(
            detected.fields[0].outcome,
            Outcome::Decided {
                conflict,
                value: chosen.to_owned()
            },
            "{status:?}, adopted"
        );
    }
}

#[test]
fn a_resolved_conflict_whose_side_moves_again_opens_a_new_conflict_and_keeps_the_old_one() {
    for (status, edited, _chosen) in settlements() {
        for (moved, sides) in [
            ("rekordbox", genre("Deep House", "Minimal", "House")),
            ("the base", genre("Deep House", "Tech House", "Electro")),
            ("the app", genre("Breaks", "Tech House", "House")),
            (
                "the base, which is gone",
                field("Genre", Some("Deep House"), Some("Tech House"), None),
            ),
        ] {
            let lib = Lib::new();
            let (id, old) = with_open(&lib);
            lib.settle_by_hand(old, status, edited);
            let settled = lib.settled();
            let expected = (
                sides.app.clone(),
                sides.rekordbox.clone(),
                sides.base.clone(),
            );

            let detected = lib.detect(&[track(id, vec![sides])]);

            let why = format!("{status:?}, {moved} moved");
            assert_eq!(lib.settled(), settled, "{why}");
            let new = lib.only_open();
            assert!(new.id > old, "{why}");
            assert_eq!(detected.opened, [new.id], "{why}");
            assert_eq!(
                (
                    new.app_value.clone(),
                    new.rekordbox_value.clone(),
                    new.base_value.clone()
                ),
                expected,
                "{why}"
            );
            assert_eq!(
                detected.fields[0].outcome,
                Outcome::Open { conflict: new.id },
                "{why}"
            );
        }
    }
}

#[test]
fn a_resolution_that_stands_again_withdraws_the_conflict_opened_meanwhile() {
    let lib = Lib::new();
    let (id, old) = with_open(&lib);
    lib.settle_by_hand(old, Status::KeptApp, None);
    // The app's value moves: a new conflict.
    let moved = lib.detect(&[track(id, vec![genre("Breaks", "Tech House", "House")])]);
    let new = moved.opened[0];

    // And moves back to the value the decision was made on.
    let detected = lib.detect(&[track(id, vec![usual()])]);

    assert_eq!(detected.withdrawn, [new]);
    assert_eq!(lib.open(&Filter::default()), []);
    assert_eq!(
        detected.fields[0].outcome,
        Outcome::Decided {
            conflict: old,
            value: "Deep House".into()
        }
    );
}

#[test]
fn only_the_newest_settled_conflict_answers_its_field() {
    let lib = Lib::new();
    let (id, first) = with_open(&lib);
    lib.settle_by_hand(first, Status::KeptRekordbox, None);
    // rekordbox moves on; the new conflict is settled the other way.
    let moved = genre("Deep House", "Minimal", "House");
    let second = lib.detect(&[track(id, vec![moved.clone()])]).opened[0];
    lib.settle_by_hand_at(second, Status::KeptApp, None, "2026-10-12T11:00:00.000Z");

    let detected = lib.detect(&[track(id, vec![moved])]);
    assert_eq!(
        detected.fields[0].outcome,
        Outcome::Decided {
            conflict: second,
            value: "Deep House".into()
        }
    );

    // Back on the first one's sides, the older decision doesn't answer:
    // the user has decided otherwise since, so they're asked.
    let detected = lib.detect(&[track(id, vec![usual()])]);
    assert_eq!(detected.opened.len(), 1);
    assert_eq!(lib.settled().len(), 2);
}

#[test]
fn the_newest_settled_conflict_is_the_one_settled_last_whatever_the_clock_said() {
    let lib = Lib::new();
    let (id, first) = with_open(&lib);
    lib.settle_by_hand_at(
        first,
        Status::KeptRekordbox,
        None,
        "2026-10-12T10:00:00.000Z",
    );
    let moved = genre("Deep House", "Minimal", "House");
    let second = lib.detect(&[track(id, vec![moved.clone()])]).opened[0];
    // The clock was set back in between: the later decision has the
    // earlier time.
    lib.settle_by_hand_at(second, Status::KeptApp, None, "2026-10-12T09:00:00.000Z");
    let before = lib.rows();

    let detected = lib.detect(&[track(id, vec![moved])]);

    // The user isn't asked again about what they just settled.
    assert_eq!(detected.opened, none());
    assert_eq!(lib.rows(), before);
    assert_eq!(
        detected.fields[0].outcome,
        Outcome::Decided {
            conflict: second,
            value: "Deep House".into()
        }
    );
}

// ---- A decision is not reversed once the app holds the value chosen --------

/// Opens a conflict on `sides`, settles it with `choice`, lets the app
/// take the value chosen as its own, and detects. Returns the settled
/// conflict's id, the value chosen and what detect said.
fn settled_and_adopted(
    lib: &Lib,
    sides: &FieldSides,
    choice: Choice,
) -> (ConflictId, String, Detected) {
    let id = lib.bare();
    lib.detect(&[track(id, vec![sides.clone()])]);
    let resolved = lib.resolve(lib.only_open().id, choice).unwrap().value;
    let adopted = FieldSides {
        app: Some(resolved.value.clone()),
        ..sides.clone()
    };
    let settled = lib.rows();

    let detected = lib.detect(&[track(id, vec![adopted.clone()])]);

    // Nothing is opened, the decision is as it was, and a rerun writes
    // nothing.
    assert_eq!(lib.rows(), settled, "{sides:?}");
    let again = vec![track(id, vec![adopted])];
    let (second, written) = lib.written_by(move |c| detect(c, &again));
    assert_eq!(written, Vec::<String>::new(), "{sides:?}");
    assert_eq!(lib.rows(), settled, "{sides:?}");
    assert_eq!(second, detected, "{sides:?}");
    (resolved.conflict, resolved.value, detected)
}

#[test]
fn a_conflict_settled_by_hand_to_the_value_last_sent_is_not_reversed_once_the_app_holds_it() {
    // The app said 4 stars, rekordbox 5, and 3 were sent. The user says:
    // neither, put it back to 3. Once the app holds 3 again, the sides read
    // "only rekordbox changed"; rekordbox's 5 must not come in.
    let lib = Lib::new();
    let sides = apart("Rating", "204", "255", "153");

    let (conflict, chosen, detected) =
        settled_and_adopted(&lib, &sides, Choice::Edit("153".into()));

    assert_eq!(chosen, "153");
    assert_eq!(detected.opened, none());
    assert_eq!(detected.fields.len(), 1, "{detected:?}");
    assert_eq!(
        detected.fields[0].outcome,
        Outcome::Decided {
            conflict,
            value: "153".into()
        }
    );
}

#[test]
fn a_decision_is_honoured_whichever_value_was_chosen_once_the_app_holds_it() {
    for (which, choice, chosen) in [
        ("the app's value", Choice::KeepApp, "204"),
        ("rekordbox's value", Choice::KeepRekordbox, "255"),
        ("the base's value", Choice::Edit("153".into()), "153"),
        ("a fourth value", Choice::Edit("51".into()), "51"),
        // The same choices, given by hand.
        ("the app's value, typed", Choice::Edit("204".into()), "204"),
        (
            "rekordbox's value, typed",
            Choice::Edit("255".into()),
            "255",
        ),
    ] {
        let lib = Lib::new();
        let sides = apart("Rating", "204", "255", "153");

        let (conflict, value, detected) = settled_and_adopted(&lib, &sides, choice);

        assert_eq!(value, chosen, "{which}");
        assert_eq!(detected.opened, none(), "{which}");
        assert_eq!(detected.withdrawn, none(), "{which}");
        assert_eq!(detected.fields.len(), 1, "{which}: {detected:?}");
        assert_eq!(
            detected.fields[0].outcome,
            Outcome::Decided {
                conflict,
                value: chosen.to_owned()
            },
            "{which}"
        );
    }
}

#[test]
fn text_put_back_or_cleared_by_hand_stays_as_the_user_decided() {
    for (sides, typed) in [
        // Put back to what was sent.
        (apart("Genre", "Deep House", "Tech House", "House"), "House"),
        // Cleared, where nothing was sent: rekordbox's comment stays out.
        (apart("Comments", "warm up", "closing /* Peak */", ""), ""),
        // A field that was never sent.
        (
            field("Genre", Some("Deep House"), Some("Tech House"), None),
            "Minimal",
        ),
    ] {
        let lib = Lib::new();

        let (conflict, value, detected) =
            settled_and_adopted(&lib, &sides, Choice::Edit(typed.to_owned()));

        assert_eq!(value, typed, "{sides:?}");
        assert_eq!(
            detected.fields[0].outcome,
            Outcome::Decided {
                conflict,
                value: typed.to_owned()
            },
            "{sides:?}"
        );
    }
}

#[test]
fn a_decision_the_app_holds_stops_standing_exactly_when_a_side_moves() {
    // Settled to the base's value and taken by the app: 3 / 5 / 3.
    let lib = Lib::new();
    let id = lib.bare();
    lib.detect(&[track(id, vec![apart("Rating", "204", "255", "153")])]);
    let settled = lib
        .resolve(lib.only_open().id, Choice::Edit("153".into()))
        .unwrap()
        .value
        .conflict;
    let outcome = |app: &str, rekordbox: &str, base: &str| {
        lib.detect(&[track(
            id,
            vec![field("Rating", Some(app), Some(rekordbox), Some(base))],
        )])
        .fields
        .first()
        .map(|f| f.outcome.clone())
    };
    let decided = Some(Outcome::Decided {
        conflict: settled,
        value: "153".into(),
    });

    assert_eq!(outcome("153", "255", "153"), decided);
    // rekordbox written another way is the same value: it hasn't moved.
    assert_eq!(outcome("153", "0255", "153"), decided);
    // rekordbox moves: its new value is taken in, by the table.
    assert_eq!(outcome("153", "102", "153"), Some(Outcome::TakeRekordbox));
    // The app moves to a value that is neither the old one nor the one
    // chosen: a new conflict.
    assert!(matches!(
        outcome("51", "255", "153"),
        Some(Outcome::Open { conflict }) if conflict > settled
    ));
    // The base moves (a send recorded 4 stars): both changed alike is
    // nothing; the decision no longer answers.
    assert_eq!(outcome("255", "255", "204"), None);
    assert_eq!(lib.settled().len(), 1);
}

// ---- A send that was never imported ---------------------------------------

#[test]
fn an_edit_whose_send_was_never_imported_opens_no_conflict_and_is_sent_again() {
    let lib = Lib::new();
    let id = lib.bare();
    // The app changed House to Deep House and sent it; rekordbox still
    // shows House.
    let sides = FieldSides {
        replaced: Some("House".into()),
        ..field(
            "Genre",
            Some("Deep House"),
            Some("House"),
            Some("Deep House"),
        )
    };

    let detected = lib.detect(&[track(id, vec![sides])]);

    assert_eq!(lib.rows(), []);
    assert_eq!(
        detected.fields,
        [FieldOutcome {
            library_track: id,
            field: "Genre".into(),
            decision: Decision {
                verdict: Verdict::NotTakenUp,
                reason: Reason::SendNotTakenUp,
            },
            outcome: Outcome::SendApp,
        }]
    );
}

// ---- What the send is told ------------------------------------------------

#[test]
fn what_a_send_does_is_listed_for_each_field_the_app_holds_a_value_for_and_no_other() {
    let lib = Lib::new();
    let (current, stale, new) = (lib.bare(), lib.bare(), lib.bare());

    let detected = lib.detect(&[
        track(
            current,
            vec![
                field("Comments", Some("warm up"), Some(""), Some("")),
                field(
                    "Label",
                    Some("Kestrel"),
                    Some("Kestrel Nine"),
                    Some("Kestrel"),
                ),
                usual(),
                // In agreement: nothing to do, so not listed.
                field("Name", Some("Velo"), Some("Velo"), Some("Velo")),
                field("Mix", Some("Dub"), Some("Dub"), Some("")),
                // The app follows rekordbox: not listed.
                field("Album", None, Some("Lantern Quay"), Some("Harbor Quay")),
            ],
        ),
        TrackSides {
            entry: RekordboxEntry::Stale,
            ..track(
                stale,
                vec![usual(), field("Album", None, Some("x"), Some("y"))],
            )
        },
        TrackSides {
            entry: RekordboxEntry::NotThere,
            ..track(new, vec![usual(), field("Album", None, None, Some("y"))])
        },
    ]);

    let open = lib.only_open().id;
    let listed: Vec<(LibraryTrackId, &str, &Outcome)> = detected
        .fields
        .iter()
        .map(|f| (f.library_track, f.field.as_str(), &f.outcome))
        .collect();
    assert_eq!(
        listed,
        [
            (current, "Comments", &Outcome::SendApp),
            (current, "Label", &Outcome::TakeRekordbox),
            (current, "Genre", &Outcome::Open { conflict: open }),
            (stale, "Genre", &Outcome::Wait),
            (new, "Genre", &Outcome::SendApp),
        ]
    );
}

// ---- What is left alone ---------------------------------------------------

#[test]
fn fields_and_tracks_that_are_not_given_are_left_alone() {
    let lib = Lib::new();
    let (one, two) = (lib.bare(), lib.bare());
    let both = vec![usual(), apart("Rating", "204", "153", "0")];
    lib.detect(&[track(one, both.clone()), track(two, both)]);
    assert_eq!(lib.rows().len(), 4);

    // Only the first track's `Genre`, which now agrees.
    lib.detect(&[track(
        one,
        vec![field("Genre", Some("House"), Some("House"), Some("House"))],
    )]);

    let left: Vec<(LibraryTrackId, String)> = lib
        .rows()
        .into_iter()
        .map(|c| (c.library_track, c.field))
        .collect();
    assert_eq!(
        left,
        [
            (one, "Rating".to_owned()),
            (two, "Genre".to_owned()),
            (two, "Rating".to_owned())
        ]
    );
}

#[test]
fn a_run_that_fails_keeps_nothing_of_itself() {
    let lib = Lib::new();
    let (id, _conflict) = with_open(&lib);
    let other = lib.bare();
    let before = lib.rows();

    // The last track doesn't exist: its conflict can't be recorded.
    let failed = lib.try_detect(&[
        track(
            id,
            vec![field("Genre", Some("House"), Some("House"), Some("House"))],
        ),
        track(other, vec![usual()]),
        track(LibraryTrackId(999), vec![usual()]),
    ]);

    assert!(failed.is_err(), "{failed:?}");
    assert_eq!(lib.rows(), before);
}

// ---- Analysis fields ------------------------------------------------------

#[test]
fn no_path_makes_an_analysis_conflict() {
    let lib = Lib::new();
    let id = lib.bare();
    let mut fields: Vec<FieldSides> = ANALYSIS_FIELDS
        .iter()
        .map(|name| apart(name, "8A", "9A", "10A"))
        .collect();
    fields.push(apart("TrackID", "7", "8", "9"));
    fields.extend(
        ANALYSIS_FIELDS
            .iter()
            .map(|name| field(name, Some("8A"), Some("9A"), None)),
    );

    let (detected, written) = {
        let sides = vec![track(id, fields)];
        lib.written_by(move |c| detect(c, &sides))
    };

    assert_eq!(written, Vec::<String>::new());
    assert_eq!(lib.rows(), []);
    assert_eq!(detected, Detected::default());
}

#[test]
fn the_fields_the_engine_never_merges_are_exactly_the_ones_the_database_refuses() {
    // The names in migration 0014's triggers, as the database holds them.
    let lib = Lib::new();
    let triggers: Vec<String> = lib
        .writer
        .call(|c| {
            c.prepare(
                "SELECT sql FROM sqlite_master
                 WHERE type = 'trigger' AND tbl_name = 'conflict' ORDER BY name",
            )?
            .query_map([], |r| r.get(0))?
            .collect()
        })
        .unwrap();
    assert_eq!(triggers.len(), 2, "{triggers:?}");
    for sql in triggers {
        let list = sql
            .split_once("new.field IN (")
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(list, _)| list)
            .unwrap_or_else(|| panic!("no field list in {sql}"));
        let mut refused: Vec<&str> = list
            .split(',')
            .map(|n| n.trim().trim_matches('\''))
            .collect();
        refused.sort_unstable();
        let mut never: Vec<&str> = ANALYSIS_FIELDS.to_vec();
        never.sort_unstable();
        assert_eq!(refused, never);
    }
}

// ---- Not an operation -----------------------------------------------------

#[test]
fn detecting_is_not_an_operation_so_undo_never_takes_a_conflict_away() {
    let lib = Lib::new();
    let (id, conflict) = with_open(&lib);
    lib.detect(&[track(id, vec![genre("Breaks", "Tech House", "House")])]);

    assert_eq!(lib.operations(), []);
    assert_eq!(lib.undo(), UndoOutcome::NothingToUndo);
    assert_eq!(lib.only_open().id, conflict);
}

#[test]
fn undoing_a_removal_after_a_detect_restores_its_conflicts() {
    let lib = Lib::new();
    let (removed, kept) = (lib.bare(), lib.bare());
    let opened = lib
        .detect(&[track(
            removed,
            vec![usual(), apart("Rating", "204", "153", "0")],
        )])
        .opened;
    lib.settle_by_hand(opened[1], Status::KeptApp, None);
    let before = lib.rows();
    assert_eq!(before.len(), 2);

    library::remove(&lib.writer, removed).unwrap();
    assert_eq!(lib.rows(), []);
    // Conflicts found since then must not take the removed ones' ids.
    let since = lib.detect(&[track(kept, vec![usual()])]).opened[0];
    assert!(since > opened[1], "{since:?} reused an id of {opened:?}");

    assert_eq!(lib.undone(), library::REMOVE_OPERATION);

    let mut after = lib.rows();
    let others = after.split_off(2);
    assert_eq!(after, before);
    assert_eq!(others.len(), 1);
    assert_eq!(others[0].id, since);
}

// ---- From the database ----------------------------------------------------

#[test]
fn an_edit_made_on_both_sides_since_the_last_send_is_found_from_the_database() {
    let lib = Lib::new();
    let id = lib.known(&[("Name", "Velo"), ("Genre", "Tech House"), ("Rating", "153")]);
    lib.sent(
        id,
        &[("Name", "Velo"), ("Genre", "House"), ("Rating", "153")],
    );
    let mut app = AppValues::default();
    app.set(id, own("Genre", "Deep House", None));

    let detected = lib.detect_for(&[id], &app);

    let open = lib.only_open();
    assert_eq!(open.library_track, id);
    assert_eq!(open.field, "Genre");
    assert_eq!(
        values(&open),
        (Some("Deep House"), Some("Tech House"), Some("House"))
    );
    assert_eq!(detected.opened, [open.id]);
}

#[test]
fn with_no_values_of_the_apps_own_nothing_conflicts_whatever_rekordbox_changed() {
    // Today's app: every field of a known track follows rekordbox.
    let lib = Lib::new();
    let id = lib.known(&[
        ("Name", "Velo (Late Mix)"),
        ("Genre", "Tech House"),
        ("Rating", "255"),
        ("Location", "file://localhost/E:/Music/Lantern Quay.mp3"),
    ]);
    lib.sent(
        id,
        &[
            ("Name", "Velo"),
            ("Genre", "House"),
            ("Rating", "0"),
            ("Location", "file://localhost/E:/Music/Lantern%20Quay.mp3"),
        ],
    );

    let detected = lib.detect_for(&[id], &AppValues::default());

    assert_eq!(lib.rows(), []);
    assert_eq!(detected, Detected::default());
}

#[test]
fn an_edit_whose_send_was_never_imported_is_not_reverted() {
    let lib = Lib::new();
    // rekordbox still shows House; Deep House was sent.
    let id = lib.known(&[("Genre", "House")]);
    lib.sent(id, &[("Genre", "Deep House")]);
    let mut app = AppValues::default();
    app.set(id, own("Genre", "Deep House", Some("House")));

    let detected = lib.detect_for(&[id], &app);

    assert_eq!(lib.rows(), []);
    assert_eq!(detected.fields.len(), 1, "{detected:?}");
    assert_eq!(detected.fields[0].decision.verdict, Verdict::NotTakenUp);
    assert_eq!(detected.fields[0].outcome, Outcome::SendApp);
}

#[test]
fn without_knowing_what_the_app_replaced_a_send_never_imported_reads_as_rekordboxs_change() {
    // The gap `replaced` closes: the same state, with nothing to tell it
    // from rekordbox changing the value back.
    let lib = Lib::new();
    let id = lib.known(&[("Genre", "House")]);
    lib.sent(id, &[("Genre", "Deep House")]);
    let mut app = AppValues::default();
    app.set(id, own("Genre", "Deep House", None));

    let detected = lib.detect_for(&[id], &app);

    assert_eq!(lib.rows(), []);
    assert_eq!(detected.fields[0].outcome, Outcome::TakeRekordbox);
}

#[test]
fn a_track_that_cannot_be_sent_keeps_its_conflicts_as_they_are() {
    let lib = Lib::new();
    // No linked file, so no send values, so nothing is decided about it.
    let (id, _conflict) = with_open(&lib);
    let before = lib.rows();

    let detected = lib.detect_for(&[id], &AppValues::default());

    assert_eq!(lib.rows(), before);
    assert_eq!(detected, Detected::default());
}

#[test]
fn a_large_library_is_detected_in_batches_with_the_same_answer_and_a_rerun_writes_nothing() {
    // More tracks than one batch holds, so the answer crosses a boundary.
    const TRACKS: i64 = 2_300;
    let lib = Lib::new();
    let ids = lib.many_known(TRACKS);
    assert_eq!(ids.len() as i64, TRACKS);
    assert!(ids.len() > 2 * store::TRACKS_AT_A_TIME);
    // Every tenth track has an edit of the app's own that rekordbox's
    // differs from.
    let mut app = AppValues::default();
    let edited: Vec<LibraryTrackId> = ids.iter().copied().step_by(10).collect();
    for id in &edited {
        app.set(*id, own("Genre", "Deep House", None));
    }

    let detected = lib.detect_for(&ids, &app);

    let open = lib.open(&Filter::default());
    let tracks: Vec<LibraryTrackId> = open.iter().map(|c| c.library_track).collect();
    assert_eq!(tracks, edited);
    for conflict in &open {
        assert_eq!(
            values(conflict),
            (Some("Deep House"), Some("Tech House"), Some("House"))
        );
    }
    assert_eq!(
        detected.opened,
        open.iter().map(|c| c.id).collect::<Vec<_>>()
    );
    // In the order asked, across the batches.
    assert_eq!(
        detected
            .fields
            .iter()
            .map(|f| f.library_track)
            .collect::<Vec<_>>(),
        edited
    );

    let before = lib.rows();
    let (again, written) = {
        let (ids, app) = (ids.clone(), app.clone());
        lib.written_by(move |c| detect_for(c, &Mount, &ids, &app))
    };
    assert_eq!(written, Vec::<String>::new());
    assert_eq!(lib.rows(), before);
    assert_eq!(again.fields, detected.fields);
    assert_eq!(again.opened, none());
}
