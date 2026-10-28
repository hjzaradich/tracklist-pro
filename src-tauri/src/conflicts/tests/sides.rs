//! Reading the three sides from the database: rekordbox's entry, the bases
//! a send recorded, and the app's own values.

use super::*;

fn own(field: &str, value: &str, replaced: Option<&str>) -> AppValue {
    AppValue {
        field: field.to_owned(),
        value: value.to_owned(),
        replaced: replaced.map(str::to_owned),
    }
}

/// One track's sides.
fn one(lib: &Lib, id: LibraryTrackId, app: &AppValues) -> TrackSides {
    let mut sides = lib.sides(&[id], app);
    assert_eq!(sides.len(), 1, "{sides:?}");
    sides.remove(0)
}

fn named<'a>(track: &'a TrackSides, name: &str) -> &'a FieldSides {
    track
        .fields
        .iter()
        .find(|f| f.field == name)
        .unwrap_or_else(|| panic!("no {name} in {track:?}"))
}

#[test]
fn the_three_sides_are_rekordboxs_entry_the_last_send_and_the_apps_own_values() {
    let lib = Lib::new();
    let id = lib.known(&[
        ("Name", "Paper Harbor"),
        ("Genre", "Tech House"),
        ("Rating", "153"),
        ("Location", "file://localhost/E:/Music/Paper Harbor.mp3"),
    ]);
    lib.sent(
        id,
        &[
            ("Name", "Paper Harbor"),
            ("Genre", "House"),
            ("Location", "file://localhost/E:/Music/Paper%20Harbor.mp3"),
        ],
    );
    let mut app = AppValues::default();
    // What the send replaced, as it recorded it.
    lib.run("UPDATE sync_base SET replaced = 'Minimal' WHERE field = 'Genre'");
    app.set(id, own("Genre", "Deep House", Some("House")));
    app.set(id, own("Rating", "204", Some("153")));

    let track = one(&lib, id, &app);

    assert_eq!(track.library_track, id);
    assert_eq!(track.entry, RekordboxEntry::Current);
    assert_eq!(
        named(&track, "Genre"),
        &FieldSides {
            field: "Genre".into(),
            app: Some("Deep House".into()),
            rekordbox: Some("Tech House".into()),
            base: Some("House".into()),
            // For a field sent before, what the send replaced, not the edit.
            replaced: Some("Minimal".into()),
        }
    );
    // A field the app holds no value of its own for.
    assert_eq!(
        named(&track, "Name"),
        &field("Name", None, Some("Paper Harbor"), Some("Paper Harbor"))
    );
    // Never sent: no base, and what the edit replaced.
    assert_eq!(
        named(&track, "Rating"),
        &FieldSides {
            field: "Rating".into(),
            app: Some("204".into()),
            rekordbox: Some("153".into()),
            base: None,
            replaced: Some("153".into()),
        }
    );
    // Each side's own spelling, untouched.
    let location = named(&track, "Location");
    assert_eq!(
        location.rekordbox.as_deref(),
        Some("file://localhost/E:/Music/Paper Harbor.mp3")
    );
    assert_eq!(
        location.base.as_deref(),
        Some("file://localhost/E:/Music/Paper%20Harbor.mp3")
    );
    // rekordbox's attributes first, in its order.
    let names: Vec<&str> = track.fields.iter().map(|f| f.field.as_str()).collect();
    assert_eq!(names, ["Name", "Genre", "Rating", "Location"]);
}

#[test]
fn a_field_only_the_base_or_only_the_app_names_is_there_too() {
    let lib = Lib::new();
    let id = lib.known(&[("Name", "Velo")]);
    // Sent before, and rekordbox's entry no longer has the attribute.
    lib.sent(id, &[("Mix", "Dub")]);
    let mut app = AppValues::default();
    app.set(id, own("Label", "Quill Ashby Recordings", None));

    let track = one(&lib, id, &app);

    assert_eq!(named(&track, "Mix"), &field("Mix", None, None, Some("Dub")));
    assert_eq!(
        named(&track, "Label"),
        &field("Label", Some("Quill Ashby Recordings"), None, None)
    );
}

#[test]
fn analysis_fields_and_the_track_id_are_never_among_the_sides() {
    let lib = Lib::new();
    let id = lib.known(&[
        ("Name", "Velo"),
        ("AverageBpm", "124.00"),
        ("Tonality", "8A"),
    ]);
    // Whatever a base or the app's values might hold for them.
    lib.sent(id, &[("Tonality", "9A"), ("AverageBpm", "125.00")]);
    let mut app = AppValues::default();
    app.set(id, own("Tonality", "10A", None));
    app.set(id, own("TEMPO", "x", None));
    app.set(id, own("TrackID", "99", None));

    let track = one(&lib, id, &app);

    let names: Vec<&str> = track.fields.iter().map(|f| f.field.as_str()).collect();
    assert_eq!(names, ["Name", "Location"]);
}

#[test]
fn a_track_rekordbox_does_not_have_has_no_rekordbox_side() {
    let lib = Lib::new();
    let (id, _file) = lib.linked();
    lib.sent(id, &[("Genre", "House")]);
    let mut app = AppValues::default();
    app.set(id, own("Genre", "Deep House", None));

    let track = one(&lib, id, &app);

    assert_eq!(track.entry, RekordboxEntry::NotThere);
    assert_eq!(
        track.fields,
        [field("Genre", Some("Deep House"), None, Some("House"))]
    );
}

#[test]
fn a_track_rekordbox_has_only_by_a_probable_match_has_no_rekordbox_side() {
    let lib = Lib::new();
    let (id, file) = lib.linked();
    lib.entry(file, THIS_READ, true, &[("Genre", "Tech House")]);

    let track = one(&lib, id, &AppValues::default());

    // Nothing of rekordbox's is attached until the match is confirmed.
    assert_eq!(track.entry, RekordboxEntry::NotThere);
    assert_eq!(track.fields, []);
}

#[test]
fn an_entry_kept_from_an_older_read_is_stale_and_one_from_the_newest_read_is_not() {
    let lib = Lib::new();
    let (old, old_file) = lib.linked();
    lib.entry(old_file, EARLIER_READ, false, &[("Genre", "House")]);
    let new = lib.known(&[("Genre", "House")]);

    let sides = lib.sides(&[old, new], &AppValues::default());

    assert_eq!(sides[0].entry, RekordboxEntry::Stale);
    assert_eq!(sides[1].entry, RekordboxEntry::Current);
}

#[test]
fn when_every_entry_is_from_the_same_read_none_is_stale_however_old_that_read_is() {
    let lib = Lib::new();
    let (id, file) = lib.linked();
    lib.entry(file, EARLIER_READ, false, &[("Genre", "House")]);

    assert_eq!(
        one(&lib, id, &AppValues::default()).entry,
        RekordboxEntry::Current
    );
}

#[test]
fn a_track_that_cannot_be_sent_is_left_out() {
    let lib = Lib::new();
    // No linked file: nothing to send for it.
    let bare = lib.bare();
    let known = lib.known(&[("Genre", "House")]);

    let sides = lib.sides(&[bare, known, LibraryTrackId(999)], &AppValues::default());

    let tracks: Vec<LibraryTrackId> = sides.iter().map(|t| t.library_track).collect();
    assert_eq!(tracks, [known]);
}

#[test]
fn a_field_with_an_open_conflict_is_among_the_sides_even_when_no_side_names_it_any_more() {
    let lib = Lib::new();
    let id = lib.known(&[("Name", "Velo")]);
    lib.detect(&[track(id, vec![apart("Mix", "Dub", "Vocal", "Club")])]);
    assert_eq!(lib.only_open().field, "Mix");

    // Now nothing holds a `Mix`: not rekordbox, not a base, not the app.
    let sides = one(&lib, id, &AppValues::default());

    assert_eq!(named(&sides, "Mix"), &field("Mix", None, None, None));
}

#[test]
fn the_apps_value_for_a_field_is_the_last_one_set() {
    let id = LibraryTrackId(7);
    let mut app = AppValues::default();
    assert_eq!(app.of(id), []);
    app.set(id, own("Genre", "House", None));
    app.set(id, own("Rating", "204", None));
    app.set(id, own("Genre", "Deep House", Some("House")));
    assert_eq!(
        app.of(id),
        [
            own("Rating", "204", None),
            own("Genre", "Deep House", Some("House"))
        ]
    );
    assert_eq!(app.of(LibraryTrackId(8)), []);
}
