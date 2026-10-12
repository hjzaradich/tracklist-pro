//! The comparison of each kind of field, and what "no value" is for each.

use proptest::prelude::*;

use super::*;

/// Asserts that every value in `values` is the same value of `field`.
fn all_same(field: &str, values: &[Option<&str>]) {
    for a in values {
        for b in values {
            assert!(same(field, *a, *b), "{field}: {a:?} and {b:?} differ");
        }
    }
}

/// Asserts that no two values in `values` are the same value of `field`.
fn all_differ(field: &str, values: &[Option<&str>]) {
    for (i, a) in values.iter().enumerate() {
        for b in &values[i + 1..] {
            assert!(
                !same(field, *a, *b),
                "{field}: {a:?} and {b:?} are the same"
            );
            assert!(
                !same(field, *b, *a),
                "{field}: {b:?} and {a:?} are the same"
            );
        }
    }
}

#[test]
fn every_field_has_the_kind_the_module_says() {
    for (kind, fields) in [
        (
            Kind::Analysis,
            &["AverageBpm", "Tonality", "TEMPO", "POSITION_MARK"][..],
        ),
        (Kind::TrackId, &["TrackID"]),
        (Kind::Location, &["Location"]),
        (Kind::Rating, &["Rating"]),
        (Kind::Colour, &["Colour"]),
        (Kind::Date, &["DateAdded", "DateModified", "LastPlayed"]),
        (
            Kind::Number,
            &[
                "Size",
                "TotalTime",
                "DiscNumber",
                "TrackNumber",
                "Year",
                "BitRate",
                "SampleRate",
                "PlayCount",
            ],
        ),
        (
            Kind::Text,
            &[
                "Name",
                "Artist",
                "Composer",
                "Album",
                "Grouping",
                "Genre",
                "Kind",
                "Comments",
                "Remixer",
                "Label",
                "Mix",
                "FutureField",
                "rating",
                "",
            ],
        ),
    ] {
        for field in fields {
            assert_eq!(kind_of(field), kind, "{field}");
        }
    }
}

// ---- Location -----------------------------------------------------------

#[test]
fn a_respelled_location_is_not_a_change() {
    all_same(
        "Location",
        &[
            // As rekordbox writes it: lowercase escapes, some characters raw.
            Some("file://localhost/D:/Sets/Caf%c3%a9 (Night)/Bus #2.mp3"),
            // As a send writes it: everything escaped, uppercase.
            Some("file://localhost/D:/Sets/Caf%C3%A9%20%28Night%29/Bus%20%232.mp3"),
            // Unescaped.
            Some("file://localhost/D:/Sets/Caf\u{e9} (Night)/Bus #2.mp3"),
            // Decomposed, and in another letter case (NTFS ignores it).
            Some("file://localhost/d:/sets/Cafe\u{301} (night)/BUS #2.MP3"),
        ],
    );
}

#[test]
fn locations_of_different_files_differ() {
    all_differ(
        "Location",
        &[
            Some("file://localhost/D:/Sets/Bus.mp3"),
            Some("file://localhost/D:/Sets/Bus.flac"),
            Some("file://localhost/E:/Sets/Bus.mp3"),
            Some("file://localhost/D:/Sets/Night/Bus.mp3"),
            // A macOS path keeps its letter case.
            Some("file://localhost/Users/quill/Bus.mp3"),
            Some("file://localhost/Users/quill/bus.mp3"),
            None,
        ],
    );
}

#[test]
fn a_location_that_does_not_decode_is_equal_only_to_the_same_text() {
    all_same(
        "Location",
        &[Some("D:/Sets/Bus.mp3"), Some("D:/Sets/Bus.mp3")],
    );
    all_differ(
        "Location",
        &[
            Some("D:/Sets/Bus.mp3"),
            Some("d:/sets/bus.mp3"),
            Some("file://localhost/D:/Sets/Bus%2.mp3"),
            Some("file://localhost/D:/Sets/Bus.mp3"),
            None,
        ],
    );
}

// ---- Rating -------------------------------------------------------------

#[test]
fn ratings_compare_by_stars() {
    // Each of rekordbox's six values, however the number is written.
    for written in [
        &["0", "00", "000"][..],
        &["51", "051"],
        &["102", "0102"],
        &["153", "0153"],
        &["204", "0204"],
        &["255", "0255"],
    ] {
        let values: Vec<Option<&str>> = written.iter().map(|w| Some(*w)).collect();
        all_same("Rating", &values);
    }
    all_differ(
        "Rating",
        &[
            Some("0"),
            Some("51"),
            Some("102"),
            Some("153"),
            Some("204"),
            Some("255"),
        ],
    );
}

#[test]
fn no_rating_is_zero_stars() {
    all_same("Rating", &[None, Some(""), Some("0")]);
    assert!(!same("Rating", None, Some("51")));
}

#[test]
fn a_rating_that_is_not_one_of_rekordboxs_six_is_compared_by_its_number() {
    all_same("Rating", &[Some("100"), Some("0100")]);
    all_differ("Rating", &[Some("100"), Some("102"), Some("101"), None]);
}

// ---- Colour -------------------------------------------------------------

#[test]
fn colours_compare_by_the_color_not_by_how_it_is_written() {
    all_same(
        "Colour",
        &[Some("0xFF8C00"), Some("0xff8c00"), Some("0XFF8C00")],
    );
    all_differ(
        "Colour",
        &[Some("0xFF8C00"), Some("0x0000FF"), Some("0x000000"), None],
    );
}

#[test]
fn no_colour_is_the_absent_or_empty_attribute_and_black_is_a_colour() {
    all_same("Colour", &[None, Some("")]);
    assert!(!same("Colour", None, Some("0x000000")));
}

// ---- Dates --------------------------------------------------------------

#[test]
fn dates_compare_as_dates_and_no_date_is_absent_or_empty() {
    for field in ["DateAdded", "DateModified", "LastPlayed"] {
        all_same(field, &[Some("2026-10-12"), Some("2026-10-12")]);
        all_same(field, &[None, Some("")]);
        all_differ(
            field,
            &[
                Some("2026-10-12"),
                Some("2026-10-11"),
                Some("2025-10-12"),
                // Not dates the reader reads: only equal to themselves.
                Some("2026-10-12 21:04"),
                Some("12/10/2026"),
                None,
            ],
        );
    }
}

// ---- Numbers ------------------------------------------------------------

#[test]
fn numbers_compare_by_value_and_no_number_is_absent_or_empty() {
    for field in [
        "Size",
        "TotalTime",
        "DiscNumber",
        "TrackNumber",
        "Year",
        "BitRate",
        "SampleRate",
        "PlayCount",
    ] {
        all_same(field, &[Some("7"), Some("07"), Some("007")]);
        all_same(field, &[None, Some("")]);
        all_differ(field, &[Some("7"), Some("8"), Some("70"), None]);
    }
}

#[test]
fn zero_is_rekordboxs_none_for_disc_track_and_year_but_a_real_play_count() {
    for field in ["DiscNumber", "TrackNumber", "Year"] {
        all_same(field, &[None, Some(""), Some("0"), Some("00")]);
    }
    for field in ["PlayCount", "Size", "TotalTime", "BitRate", "SampleRate"] {
        assert!(!same(field, Some("0"), None), "{field}");
        all_same(field, &[Some("0"), Some("00")]);
    }
}

// ---- Text ---------------------------------------------------------------

#[test]
fn text_differing_only_in_letter_case_or_spacing_is_a_change() {
    for field in ["Name", "Artist", "Genre", "Comments", "FutureField"] {
        all_differ(
            field,
            &[
                Some("Paper Harbor"),
                Some("paper harbor"),
                Some("Paper Harbor "),
                Some(" Paper Harbor"),
                Some("Paper  Harbor"),
                Some("PaperHarbor"),
                None,
            ],
        );
    }
}

#[test]
fn text_in_another_unicode_form_is_the_same_text() {
    // Composed and decomposed.
    all_same(
        "Artist",
        &[
            Some("Halden R\u{f6}\u{f6}k"),
            Some("Halden Ro\u{308}o\u{308}k"),
        ],
    );
    assert!(!same(
        "Artist",
        Some("Halden R\u{f6}\u{f6}k"),
        Some("Halden Rook")
    ));
}

#[test]
fn blank_text_is_no_value() {
    for field in ["Name", "Comments", "Mix", "FutureField"] {
        all_same(
            field,
            &[None, Some(""), Some(" "), Some("   "), Some("\t\n")],
        );
        assert!(!same(field, None, Some(" x ")), "{field}");
    }
}

#[test]
fn comments_are_one_text_with_the_my_tags_block_in_it() {
    all_differ(
        "Comments",
        &[
            Some("warm up /* Peak / Vocal */"),
            Some("warm up /* Peak */"),
            Some("warm up"),
            Some("closing /* Peak / Vocal */"),
        ],
    );
}

// ---- Values that don't parse --------------------------------------------

#[test]
fn a_value_that_does_not_parse_as_its_kind_is_equal_only_to_the_same_text() {
    for (field, unread, near) in [
        ("Rating", "five", "Five"),
        ("Rating", "256", "0256"),
        ("Rating", "-51", "51"),
        ("Colour", "orange", "Orange"),
        ("Colour", "FF8C00", "0xFF8C00"),
        ("DateAdded", "2026-13-01", "2026-13-1"),
        ("PlayCount", "1e3", "1000"),
        ("PlayCount", "12 ", "12"),
        ("Year", "99999", "099999"),
    ] {
        assert!(same(field, Some(unread), Some(unread)), "{field} {unread}");
        assert!(!same(field, Some(unread), Some(near)), "{field} {unread}");
        assert!(!same(field, Some(unread), None), "{field} {unread}");
    }
}

// ---- A value given by hand ----------------------------------------------

#[test]
fn a_value_given_by_hand_must_parse_as_its_fields_kind() {
    for (field, value) in [
        ("Genre", "Deep House"),
        ("Genre", ""),
        ("FutureField", "anything at all"),
        ("Rating", "0"),
        ("Rating", "204"),
        ("Rating", ""),
        ("Colour", "0xFF8C00"),
        ("Colour", ""),
        ("DateAdded", "2026-10-12"),
        ("Year", "1999"),
        ("PlayCount", "0"),
        ("Location", "file://localhost/D:/Sets/Bus.mp3"),
    ] {
        assert!(fits(field, value), "{field} = {value:?}");
    }
    for (field, value) in [
        // Not one of rekordbox's six ratings.
        ("Rating", "100"),
        ("Rating", "5"),
        ("Rating", "five"),
        ("Colour", "orange"),
        ("DateAdded", "yesterday"),
        ("Year", "ninety-nine"),
        ("PlayCount", "-1"),
        ("Location", "D:/Sets/Bus.mp3"),
        // Never by hand: rekordbox's analysis, and its own id.
        ("Tonality", "8A"),
        ("AverageBpm", "124.00"),
        ("TEMPO", ""),
        ("TrackID", "7"),
    ] {
        assert!(!fits(field, value), "{field} = {value:?}");
    }
}

// ---- Any two values -----------------------------------------------------

fn any_field() -> impl Strategy<Value = &'static str> {
    proptest::sample::select(vec![
        "Name",
        "Comments",
        "Rating",
        "Colour",
        "PlayCount",
        "TrackNumber",
        "Year",
        "LastPlayed",
        "Location",
        "FutureField",
    ])
}

fn any_value() -> impl Strategy<Value = Option<String>> {
    proptest::option::of(prop_oneof![
        "[ 0-9]{0,4}",
        "[a-cA-C ]{0,4}",
        "0[xX][0-9a-fA-F]{6}",
        "20[0-9]{2}-[01][0-9]-[0-3][0-9]",
        "file://localhost/[cC]:/[a-bA-B %]{1,4}",
        "\\PC{0,6}",
    ])
}

proptest! {
    #[test]
    fn the_same_text_is_always_the_same_value_and_the_comparison_has_no_direction(
        field in any_field(),
        a in any_value(),
        b in any_value(),
    ) {
        let (a, b) = (a.as_deref(), b.as_deref());
        prop_assert!(same(field, a, a));
        prop_assert_eq!(same(field, a, b), same(field, b, a));
    }

    #[test]
    fn values_equal_to_a_third_are_equal_to_each_other(
        field in any_field(),
        a in any_value(),
        b in any_value(),
        c in any_value(),
    ) {
        let (a, b, c) = (a.as_deref(), b.as_deref(), c.as_deref());
        if same(field, a, b) && same(field, b, c) {
            prop_assert!(same(field, a, c));
        }
    }
}
