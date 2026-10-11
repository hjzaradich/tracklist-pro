//! One file's two names taken together ([`ParsedName::with_labels_of`]),
//! labels compared whatever the titles ([`compare_labels`]), and artist
//! tags ([`parse_artists`]).

use super::super::{
    compare, compare_labels, parse_artists, parse_file_name, parse_title, CreditRole, MarkerKind,
    Outcome, Reason, Side,
};
use super::{show_credits, show_markers};

// ---- A file's version is what either of its names says ----------------

#[test]
fn a_label_only_the_file_name_carries_is_added_to_the_title_tags_name() {
    let tag = parse_title("Velo");
    let file = parse_file_name("nightdrive promo (Dirty).mp3");
    let merged = tag.with_labels_of(&file);
    assert_eq!(merged.base_title, "Velo");
    assert_eq!(show_markers(&merged.markers), "dirty");
    assert_eq!(merged.original, "Velo");
    // Beside the plain title it is now the Dirty cut, as if tagged so.
    assert_eq!(
        compare(&merged, &parse_title("Velo")).outcome,
        Outcome::DifferentCut
    );
}

#[test]
fn a_label_only_the_title_tag_carries_is_added_to_the_file_names_name() {
    let tag = parse_title("Velo (Quill Ashby Remix)");
    let file = parse_file_name("Halden Rook - Velo.mp3");
    let merged = file.with_labels_of(&tag);
    assert_eq!(merged.base_title, "Velo");
    assert_eq!(show_markers(&merged.markers), "remix=Quill Ashby");
    // The file name's own credits stay, and none are taken.
    assert_eq!(show_credits(&merged.credits), "Halden Rook");
}

#[test]
fn a_label_both_names_carry_is_there_once_however_each_writes_it() {
    for (title, file_name) in [
        ("Velo (Clean)", "Velo [CLEAN].mp3"),
        ("Velo (Extended Mix)", "Velo - Extended.mp3"),
        ("Velo (Quill Ashby Remix)", "Velo (Quill Ashby's Remix).mp3"),
        ("Velo (2019 Remaster)", "Velo (Remastered 2019).mp3"),
    ] {
        let tag = parse_title(title);
        let merged = tag.with_labels_of(&parse_file_name(file_name));
        assert_eq!(merged.markers, tag.markers, "{title} / {file_name}");
    }
}

#[test]
fn two_files_that_carry_the_same_label_in_different_names_are_the_same_version() {
    // One file says "(Dirty)" in its file name only, the other in its tag
    // only: each one's names together say the same.
    let a = parse_title("Velo").with_labels_of(&parse_file_name("Velo (Dirty).mp3"));
    let b = parse_title("Velo (Dirty)").with_labels_of(&parse_file_name("nightdrive promo.mp3"));
    assert_eq!(compare(&a, &b).outcome, Outcome::SameVersion);
}

#[test]
fn labels_of_both_names_add_up() {
    let merged =
        parse_title("Velo (Clean)").with_labels_of(&parse_file_name("Velo (Intro) (Dub).mp3"));
    assert_eq!(show_markers(&merged.markers), "clean | intro | dub");
}

#[test]
fn only_the_missing_label_of_a_bracket_with_several_is_added() {
    let merged = parse_title("Velo (Clean)")
        .with_labels_of(&parse_file_name("Velo (2019 Remaster - Clean).mp3"));
    assert_eq!(show_markers(&merged.markers), "clean | remaster");
}

#[test]
fn a_rework_that_doesnt_say_whose_learns_it_from_the_other_name() {
    let merged = parse_title("Velo (Remix)")
        .with_labels_of(&parse_file_name("Velo (Quill Ashby Remix).mp3"));
    assert_eq!(show_markers(&merged.markers), "remix=Quill Ashby");
    // And the other way round, the unnamed one adds nothing.
    let merged = parse_title("Velo (Quill Ashby Remix)")
        .with_labels_of(&parse_file_name("Velo (Remix).mp3"));
    assert_eq!(show_markers(&merged.markers), "remix=Quill Ashby");
}

#[test]
fn two_names_that_give_different_remixers_keep_both() {
    let merged = parse_title("Velo (Quill Ashby Remix)")
        .with_labels_of(&parse_file_name("Velo (Vey Sun Remix).mp3"));
    assert_eq!(
        show_markers(&merged.markers),
        "remix=Quill Ashby | remix=Vey Sun"
    );
}

#[test]
fn a_remasters_year_from_the_other_name_counts() {
    let merged =
        parse_title("Velo (Remaster)").with_labels_of(&parse_file_name("Velo (2019 Remaster).mp3"));
    let other = parse_title("Velo (2011 Remaster)");
    assert_eq!(compare(&merged, &other).outcome, Outcome::DifferentCut);
    // A year the name already gives isn't doubled by a yearless one.
    let merged =
        parse_title("Velo (2019 Remaster)").with_labels_of(&parse_file_name("Velo (Remaster).mp3"));
    assert_eq!(merged.markers.len(), 1);
}

#[test]
fn a_label_this_name_holds_in_its_title_counts_as_had() {
    // "Paper Harbor Extended Mix" keeps the words in its title; the file
    // name's bracket says the same thing and adds nothing.
    let tag = parse_title("Paper Harbor Extended Mix");
    let merged = tag.with_labels_of(&parse_file_name("Paper Harbor (Extended Mix).mp3"));
    assert_eq!(merged.base_title, "Paper Harbor Extended Mix");
    assert!(merged.markers.is_empty());
    assert_eq!(merged.title_markers, tag.title_markers);
}

#[test]
fn label_words_the_other_name_keeps_in_its_title_are_not_taken() {
    let merged = parse_title("Come").with_labels_of(&parse_file_name("Come Clean.mp3"));
    assert!(merged.markers.is_empty());
    assert_eq!(merged.base_title, "Come");
}

#[test]
fn a_mashups_x_is_not_taken_but_its_label_is() {
    let merged =
        parse_title("Duskline").with_labels_of(&parse_file_name("Duskline x Emberlow.mp3"));
    assert!(merged.markers.is_empty());
    let merged = parse_title("Duskline")
        .with_labels_of(&parse_file_name("Duskline x Emberlow (Mashup).mp3"));
    assert_eq!(show_markers(&merged.markers), "mashup");
    assert!(merged.mashup_parts.is_empty());
}

#[test]
fn a_bracket_nobody_understands_is_carried_over_once() {
    let merged = parse_title("Glasswing")
        .with_labels_of(&parse_file_name("Glasswing (Skyline Session).mp3"));
    assert_eq!(merged.unrecognized, ["(Skyline Session)"]);
    assert_eq!(
        compare(&merged, &parse_title("Glasswing")).outcome,
        Outcome::CantTell
    );
    let merged = parse_title("Glasswing (Skyline Session)")
        .with_labels_of(&parse_file_name("Glasswing (skyline session).mp3"));
    assert_eq!(merged.unrecognized, ["(Skyline Session)"]);
}

#[test]
fn a_name_with_nothing_to_take_from_the_other_is_unchanged() {
    let tag = parse_title("Velo (Clean)");
    assert_eq!(
        tag.with_labels_of(&parse_file_name("Halden Rook - Velo.mp3")),
        tag
    );
}

// ---- Labels compared whatever the titles -------------------------------

#[test]
fn labels_under_two_different_titles_still_compare() {
    let cases = [
        ("Velo (Clean)", "Nightdrive (Dirty)", Outcome::DifferentCut),
        ("Velo (Clean)", "Nightdrive (Clean)", Outcome::SameVersion),
        ("Velo", "Nightdrive", Outcome::SameVersion),
        (
            "Velo (Quill Ashby Remix)",
            "Nightdrive",
            Outcome::DifferentRework,
        ),
        (
            "Velo (Quill Ashby Remix)",
            "Nightdrive (Remix)",
            Outcome::CantTell,
        ),
        ("Velo (Skyline Session)", "Nightdrive", Outcome::CantTell),
        (
            "Velo (2019 Remaster)",
            "Nightdrive (2011 Remaster)",
            Outcome::DifferentCut,
        ),
    ];
    for (a, b, outcome) in cases {
        let got = compare_labels(&parse_title(a), &parse_title(b));
        assert_eq!(got.outcome, outcome, "{a} / {b}");
        // The titles are not this comparison's business.
        assert!(
            !got.reasons.contains(&Reason::BaseTitlesMatch)
                && !got.reasons.contains(&Reason::BaseTitlesDiffer),
            "{a} / {b}"
        );
    }
}

#[test]
fn comparing_labels_names_the_cuts_only_one_name_has_and_doesnt_depend_on_the_order() {
    let (clean, dirty) = (
        parse_title("Velo (Clean)"),
        parse_title("Nightdrive (Dirty)"),
    );
    assert_eq!(
        compare_labels(&clean, &dirty).reasons,
        [Reason::CutDiffers {
            only_a: vec![MarkerKind::Clean],
            only_b: vec![MarkerKind::Dirty],
        }]
    );
    assert_eq!(
        compare_labels(&dirty, &clean).reasons,
        [Reason::CutDiffers {
            only_a: vec![MarkerKind::Dirty],
            only_b: vec![MarkerKind::Clean],
        }]
    );
}

#[test]
fn comparing_labels_leaves_label_words_in_a_title_alone() {
    // With no shared title to settle it, "Clean" stays part of "Come
    // Clean": nothing says these two differ in version.
    let got = compare_labels(&parse_title("Come Clean"), &parse_title("Nightdrive"));
    assert_eq!(got.outcome, Outcome::SameVersion);
}

#[test]
fn comparing_labels_still_weighs_the_artists() {
    let got = compare_labels(
        &parse_file_name("Quill Ashby - Velo (Quill Ashby Remix).mp3"),
        &parse_file_name("Kestrel Nine - Nightdrive.mp3"),
    );
    assert_eq!(got.outcome, Outcome::DifferentRework);
    assert!(got
        .reasons
        .contains(&Reason::CreditedToRemixer { side: Side::A }));
}

// ---- Artist tags -------------------------------------------------------

#[test]
fn an_artist_tag_splits_into_main_and_featured_artists() {
    let credits = parse_artists("Kestrel Nine, Halden Rook & Vey Sun feat. Nox");
    assert_eq!(
        show_credits(&credits),
        "Kestrel Nine | Halden Rook | Vey Sun | +Nox"
    );
    assert_eq!(credits[3].role, CreditRole::Featured);
}

#[test]
fn an_artist_tag_splits_on_x_and_vs_between_acts() {
    assert_eq!(
        show_credits(&parse_artists("Kestrel Nine x Halden Rook")),
        "Kestrel Nine | Halden Rook"
    );
    assert_eq!(
        show_credits(&parse_artists("Kestrel Nine vs Halden Rook")),
        "Kestrel Nine | Halden Rook"
    );
}

#[test]
fn an_artist_tag_with_one_artist_gives_that_artist_as_written() {
    let credits = parse_artists("  The Marrow   Choir ");
    assert_eq!(show_credits(&credits), "The Marrow Choir");
    assert_eq!(credits[0].role, CreditRole::Main);
}

#[test]
fn an_empty_artist_tag_names_nobody() {
    assert!(parse_artists("").is_empty());
    assert!(parse_artists("   ").is_empty());
}

#[test]
fn an_artist_tag_is_not_split_on_a_slash_or_a_semicolon() {
    // Either can be part of one act's name, and a tag doesn't say which
    // it is. Pinned: two artists written this way stay one name.
    for tag in [
        "Nox/Vey",
        "Kestrel Nine; Halden Rook",
        "Kestrel Nine / Halden Rook",
    ] {
        assert_eq!(parse_artists(tag).len(), 1, "{tag}");
    }
    assert_eq!(parse_artists("Nox/Vey")[0].name, "Nox/Vey");
}

#[test]
fn an_artist_tag_is_taken_as_written_with_no_junk_looked_for() {
    // A file name's artist part is cleaned of underscores and site names
    // first; a tag isn't.
    assert_eq!(parse_artists("Kestrel_Nine")[0].name, "Kestrel_Nine");
    assert_eq!(
        parse_file_name("Kestrel_Nine_-_Velo.mp3").credits[0].name,
        "Kestrel Nine"
    );
}

// ---- A label one name holds in its title and the other brackets -------

#[test]
fn every_label_of_the_other_name_is_added_even_one_this_name_holds_in_its_title() {
    let tag = parse_title("Come Clean");
    let file = parse_file_name("Come Clean (Clean).mp3");
    // Taken as had, the two names of the file still differ by a cut…
    let had = tag.with_labels_of(&file);
    assert!(had.markers.is_empty());
    assert_eq!(compare(&had, &file).outcome, Outcome::DifferentCut);
    // …so the bracket is added all the same, and they say the same.
    let merged = tag.with_every_label_of(&file);
    assert_eq!(merged.base_title, "Come Clean");
    assert_eq!(show_markers(&merged.markers), "clean");
    assert_eq!(compare(&merged, &file).outcome, Outcome::SameVersion);
    assert_eq!(
        compare(&merged, &parse_title("Come Clean")).outcome,
        Outcome::DifferentCut
    );
}

#[test]
fn every_label_of_the_other_name_adds_nothing_this_name_already_reads_as_a_label() {
    for (title, file_name) in [
        ("Velo (Clean)", "Velo [CLEAN].mp3"),
        ("Velo (Quill Ashby Remix)", "Velo (Remix).mp3"),
        ("Velo", "Halden Rook - Velo.mp3"),
    ] {
        let (tag, file) = (parse_title(title), parse_file_name(file_name));
        assert_eq!(
            tag.with_every_label_of(&file),
            tag.with_labels_of(&file),
            "{title} / {file_name}"
        );
    }
}
