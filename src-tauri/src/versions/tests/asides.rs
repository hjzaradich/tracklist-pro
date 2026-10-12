//! Harmless brackets (1bD-4): a copy counter, a year, a key, a tempo, a
//! catalogue number. Each is an aside only when nothing in it can be a
//! version; anything doubtful stays among the unrecognized brackets.

use super::super::{
    compare, parse_file_name, parse_title, Aside, AsideKind, MarkerKind, Outcome, ParsedName,
};

fn kinds(name: &ParsedName) -> Vec<AsideKind> {
    name.asides.iter().map(|aside| aside.kind).collect()
}

fn values(name: &ParsedName) -> Vec<&str> {
    name.asides
        .iter()
        .map(|aside| aside.value.as_str())
        .collect()
}

// ---- What is harmless ---------------------------------------------------

#[test]
fn each_kind_of_harmless_bracket_is_set_apart_and_leaves_the_title_alone() {
    for (input, kind, value) in [
        ("Velo (1)", AsideKind::CopyCounter, "1"),
        ("Velo (12)", AsideKind::CopyCounter, "12"),
        ("Velo [3]", AsideKind::CopyCounter, "3"),
        ("Velo (2011)", AsideKind::Year, "2011"),
        ("Velo (1998)", AsideKind::Year, "1998"),
        ("Velo (8A)", AsideKind::Key, "8a"),
        ("Velo (12b)", AsideKind::Key, "12b"),
        ("Velo (124)", AsideKind::Tempo, "124"),
        ("Velo [124 BPM]", AsideKind::Tempo, "124"),
        ("Velo (124bpm)", AsideKind::Tempo, "124"),
        ("Velo (98.5 bpm)", AsideKind::Tempo, "98.5"),
        ("Velo [ABC123]", AsideKind::CatalogueNumber, "ABC123"),
        ("Velo (cat-0451)", AsideKind::CatalogueNumber, "CAT0451"),
        ("Velo {ABC-123}", AsideKind::CatalogueNumber, "ABC123"),
    ] {
        for name in [parse_title(input), parse_file_name(&format!("{input}.mp3"))] {
            assert_eq!(kinds(&name), [kind], "{input}");
            assert_eq!(values(&name), [value], "{input}");
            assert_eq!(name.base_title, "Velo", "{input}");
            assert!(name.unrecognized.is_empty(), "{input}");
            assert!(name.markers.is_empty(), "{input}");
        }
    }
}

#[test]
fn a_run_of_harmless_brackets_is_peeled_from_the_end() {
    let name = parse_title("Velo (8A) (124) (1)");
    assert_eq!(
        kinds(&name),
        [AsideKind::Key, AsideKind::Tempo, AsideKind::CopyCounter]
    );
    assert!(name.unrecognized.is_empty());
    // Junk and a featuring credit in between say nothing either way.
    let name = parse_title("Velo (feat. Halden Rook) (320) (1)");
    assert_eq!(kinds(&name), [AsideKind::CopyCounter]);
}

#[test]
fn the_first_bracket_that_is_not_harmless_stops_the_peeling() {
    // A label at the end: nothing in front of it is an aside.
    let name = parse_title("Velo (1) (Dub)");
    assert!(name.asides.is_empty());
    assert_eq!(name.unrecognized, ["(1)"]);
    let name = parse_title("Velo (8A) (Dirty)");
    assert!(name.asides.is_empty());
    assert_eq!(name.unrecognized, ["(8A)"]);
    // A bracket nobody understands in the middle.
    let name = parse_title("Velo (8A) (Acoustic Take) [124 BPM]");
    assert_eq!(kinds(&name), [AsideKind::Tempo]);
    assert_eq!(name.unrecognized, ["(8A)", "(Acoustic Take)"]);
    // ... and a bare number after one may be its number.
    let name = parse_title("Velo (Acoustic Take) (124)");
    assert!(name.asides.is_empty());
    assert_eq!(name.unrecognized, ["(Acoustic Take)", "(124)"]);
    // A bare number after a label is perhaps its number, and stops it.
    let name = parse_title("Velo (8A) (Dub) (124)");
    assert!(name.asides.is_empty());
    assert_eq!(name.unrecognized, ["(8A)", "(124)"]);
    // What comes after the stop is still peeled.
    let name = parse_title("Velo (Pt. 2) (8A)");
    assert_eq!(kinds(&name), [AsideKind::Key]);
    assert_eq!(name.unrecognized, ["(Pt. 2)"]);
    let name = parse_title("Velo (Extended Mix) [124 BPM]");
    assert_eq!(kinds(&name), [AsideKind::Tempo]);
    assert_eq!(name.markers[0].kind, MarkerKind::Extended);
}

// ---- What is not harmless -------------------------------------------------

#[test]
fn brackets_that_could_be_a_version_stay_unrecognized() {
    for input in [
        // Words: a part, a take, a volume.
        "Velo (Part 1)",
        "Velo (Pt 2)",
        "Velo (Take 2)",
        "Velo (Vol. 3)",
        "Velo (2011 Version)",
        "Velo (Dub 12)",
        // Not the shape of a counter, a year, a tempo, a key or a code.
        "Velo (01)",
        "Velo (0)",
        "Velo (808)",
        "Velo (300)",
        "Velo (1850)",
        "Velo (2100)",
        "Velo (13A)",
        "Velo (0A)",
        "Velo (8C)",
        "Velo (ABC 123)",
        "Velo (A1)",
        "Velo (ABC12)",
        "Velo (Vol001)",
        "Velo (Remix007)",
        "Velo (Alt123)",
        "Velo (30 BPM)",
        "Velo (124 BPM",
        "Velo (1",
    ] {
        let name = parse_title(input);
        assert!(name.asides.is_empty(), "{input}: {:?}", name.asides);
    }
}

#[test]
fn a_bare_number_directly_after_a_label_or_a_counting_word_is_not_harmless() {
    for input in [
        "Velo (Dub) (2)",
        "Velo Dub (2)",
        "Velo (Remix) (2011)",
        "Velo Remaster (2011)",
        "Velo (Remastered) (2011)",
        "Velo (Dub) (124)",
        "Velo Pt. (2)",
        "Velo Part (2)",
        "Velo Vol. (2011)",
        "Velo No. (124)",
        "Velo (Acoustic Take) (2)",
        "Velo (Dub) (1) (2)",
    ] {
        for name in [parse_title(input), parse_file_name(&format!("{input}.mp3"))] {
            assert!(name.asides.is_empty(), "{input}: {:?}", name.asides);
            assert!(
                name.unrecognized
                    .iter()
                    .any(|text| text.contains(['(', '['])),
                "{input} must keep its bracket as unrecognized: {:?}",
                name.unrecognized
            );
        }
    }
}

#[test]
fn a_key_a_tempo_with_its_unit_and_a_code_say_what_they_are_after_a_label_too() {
    for input in [
        "Velo (Extended Mix) (8A)",
        "Velo (Extended Mix) [124 BPM]",
        "Velo (Dub) [ABC123]",
    ] {
        assert_eq!(parse_title(input).asides.len(), 1, "{input}");
    }
}

#[test]
fn a_name_that_is_nothing_but_a_harmless_bracket_is_its_own_title() {
    for input in ["(1)", "(2011)", "(8A)"] {
        let name = parse_title(input);
        assert!(name.asides.is_empty(), "{input}");
        assert_eq!(name.base_title, input);
    }
    let name = parse_file_name("(1).mp3");
    assert!(name.asides.is_empty());
    assert_eq!(name.base_title, "(1)");
}

#[test]
fn a_dash_label_after_the_brackets_means_none_is_at_the_end() {
    let name = parse_title("Velo (1) - Dirty");
    assert!(name.asides.is_empty());
}

#[test]
fn the_title_itself_keeps_its_numbers() {
    for (input, base) in [
        ("124 (1)", "124"),
        ("1999 (2)", "1999"),
        ("Velo 2 (1)", "Velo 2"),
    ] {
        let name = parse_title(input);
        assert_eq!(name.base_title, base, "{input}");
        assert_eq!(kinds(&name), [AsideKind::CopyCounter], "{input}");
    }
}

// ---- What two names say -------------------------------------------------------------------

fn outcome(a: &str, b: &str) -> Outcome {
    let (a, b) = (parse_title(a), parse_title(b));
    let there = compare(&a, &b).outcome;
    assert_eq!(compare(&b, &a).outcome, there, "{a:?} / {b:?}");
    there
}

#[test]
fn a_harmless_bracket_on_one_name_changes_nothing() {
    for bracket in [
        "(1)",
        "(12)",
        "(2011)",
        "(8A)",
        "(124)",
        "[124 BPM]",
        "[ABC123]",
    ] {
        for base in [
            "Velo",
            "Velo (Dirty)",
            "Velo (Halden Rook Remix)",
            "Velo - Radio Edit",
        ] {
            let with = format!("{base} {bracket}");
            let bare_number = matches!(bracket, "(1)" | "(12)" | "(2011)" | "(124)");
            let after_a_label = base != "Velo";
            let got = outcome(&with, base);
            if bare_number && after_a_label {
                // A bare number after a label is perhaps its number.
                assert_eq!(got, Outcome::CantTell, "{with}");
            } else {
                assert_eq!(got, outcome(base, base), "{with}");
            }
        }
    }
}

#[test]
fn copy_counters_never_make_two_names_unclear_whatever_their_numbers() {
    assert_eq!(outcome("Velo (1)", "Velo (2)"), Outcome::SameVersion);
    assert_eq!(outcome("Velo (1)", "Velo"), Outcome::SameVersion);
    assert_eq!(outcome("Velo (12)", "Velo (3)"), Outcome::SameVersion);
}

#[test]
fn two_names_whose_brackets_of_one_kind_disagree_are_unclear() {
    for (a, b) in [
        ("Velo (2011)", "Velo (2012)"),
        ("Velo (8A)", "Velo (9A)"),
        ("Velo [124 BPM]", "Velo [126 BPM]"),
        ("Velo [ABC123]", "Velo [ABC124]"),
        ("Velo (124)", "Velo (126)"),
    ] {
        assert_eq!(outcome(a, b), Outcome::CantTell, "{a} / {b}");
    }
}

#[test]
fn two_names_whose_brackets_agree_or_are_of_other_kinds_are_the_same() {
    for (a, b) in [
        ("Velo (2011)", "Velo (2011)"),
        ("Velo (8A)", "Velo (8a)"),
        ("Velo (124)", "Velo [124 BPM]"),
        ("Velo (8A)", "Velo [124 BPM]"),
        ("Velo (2011) (8A)", "Velo (124) (1)"),
    ] {
        assert_eq!(outcome(a, b), Outcome::SameVersion, "{a} / {b}");
    }
}

#[test]
fn a_harmless_bracket_beside_a_real_difference_leaves_the_difference() {
    assert_eq!(
        outcome("Velo (Dirty) [124 BPM]", "Velo (Clean)"),
        Outcome::DifferentCut
    );
    assert_eq!(
        outcome("Velo (Halden Rook Remix) (8A)", "Velo (1)"),
        Outcome::DifferentRework
    );
}

#[test]
fn the_asides_of_a_files_two_names_are_put_together() {
    let tag = parse_title("Velo");
    let file_name = parse_file_name("Velo (2011).mp3");
    let merged = tag.with_labels_of(&file_name);
    assert_eq!(values(&merged), ["2011"]);
    // Not added twice.
    assert_eq!(values(&merged.with_labels_of(&file_name)), ["2011"]);
}

#[test]
fn years_can_be_put_back_among_the_brackets_nobody_understands() {
    let name = parse_title("Velo (2011) (8A)");
    assert_eq!(kinds(&name), [AsideKind::Year, AsideKind::Key]);
    let unsettled = name.with_years_unsettled();
    assert_eq!(kinds(&unsettled), [AsideKind::Key]);
    assert_eq!(unsettled.unrecognized, ["(2011)"]);
    assert_eq!(
        compare(&unsettled, &parse_title("Velo")).outcome,
        Outcome::CantTell
    );
}

#[test]
fn an_aside_is_what_it_was_written_as() {
    let name = parse_title("Velo [124 BPM]");
    assert_eq!(
        name.asides,
        [Aside {
            kind: AsideKind::Tempo,
            value: "124".to_string(),
            text: "[124 BPM]".to_string(),
        }]
    );
}

// ---- A file name whose second part is only a label --------------------------------

use super::super::parse_file_name_title_first;

#[test]
fn a_file_name_whose_second_part_is_only_labels_can_be_read_title_first() {
    for (input, kind) in [
        ("Velo - Radio Edit.mp3", MarkerKind::RadioEdit),
        ("Velo - Dirty.mp3", MarkerKind::Dirty),
        ("Velo - Extended Mix.mp3", MarkerKind::Extended),
        ("Velo - Halden Rook Remix.mp3", MarkerKind::Remix),
    ] {
        let name = parse_file_name_title_first(input).expect(input);
        assert_eq!(name.base_title, "Velo", "{input}");
        assert!(name.credits.is_empty(), "{input}");
        assert_eq!(name.markers[0].kind, kind, "{input}");
        // The default reading is unchanged: an artist and a title.
        let default = parse_file_name(input);
        assert_eq!(default.credits[0].name, "Velo", "{input}");
    }
}

#[test]
fn a_harmless_bracket_after_the_label_is_set_apart_and_the_label_still_read() {
    let name = parse_file_name_title_first("Velo - Radio Edit [124 BPM].mp3").unwrap();
    assert_eq!(name.base_title, "Velo");
    assert_eq!(name.markers[0].kind, MarkerKind::RadioEdit);
    assert_eq!(kinds(&name), [AsideKind::Tempo]);
}

#[test]
fn a_file_name_has_no_other_reading_when_its_second_part_is_not_only_labels() {
    for input in [
        "Halden Rook - Velo.mp3",
        "Velo.mp3",
        "Velo (Radio Edit).mp3",
        "Halden Rook - Velo - Radio Edit.mp3",
        "Radio Edit.mp3",
    ] {
        assert!(parse_file_name_title_first(input).is_none(), "{input}");
    }
}
