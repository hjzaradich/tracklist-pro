//! A file's version is what either of its names says: its labels are those
//! of its title tag and its file name together, and the two files' labels
//! are compared (module docs).

use super::*;

/// A file with a title tag and a file name.
fn file(title: &str, file_name: &str) -> FileFacts {
    tagged_and_named(title, file_name)
}

// ---- Full audio: a label in either name stops the merge ----------------

#[test]
fn full_audio_never_merges_a_file_whose_file_name_alone_says_dirty_whatever_the_other_is_named() {
    let got = decided(
        full(),
        &file("Velo", "Velo (Dirty).mp3"),
        &file("Velo", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, cut_link(&[MarkerKind::Dirty]));
    assert_eq!(
        got.reasons,
        [Reason::VersionVeto {
            only_a: vec![MarkerKind::Dirty],
            only_b: vec![],
        }]
    );
}

#[test]
fn full_audio_never_merges_a_file_whose_file_name_alone_says_dirty_when_it_is_the_second_file() {
    let got = decided(
        full(),
        &file("Velo", "nightdrive promo.mp3"),
        &file("Velo", "Velo (Dirty).mp3"),
    );
    assert_eq!(got.outcome, cut_link(&[MarkerKind::Dirty]));
    assert_eq!(
        got.reasons,
        [Reason::VersionVeto {
            only_a: vec![],
            only_b: vec![MarkerKind::Dirty],
        }]
    );
}

#[test]
fn full_audio_never_merges_on_a_cut_label_only_one_file_name_carries() {
    let cases: [(&str, &str, &[MarkerKind]); 3] = [
        (
            "Velo (Clean).mp3",
            "halden_rook-velo.mp3",
            &[MarkerKind::Clean],
        ),
        (
            "Velo (Extended Mix).mp3",
            "nightdrive promo.mp3",
            &[MarkerKind::Extended],
        ),
        (
            "Halden Rook - Velo - Radio Edit.mp3",
            "nightdrive promo.mp3",
            &[MarkerKind::RadioEdit],
        ),
    ];
    for (name_a, name_b, labels) in cases {
        let got = decided(full(), &file("Velo", name_a), &file("Velo", name_b));
        assert_eq!(got.outcome, cut_link(labels), "{name_a}");
        assert_eq!(codes(&got), ["versionVeto"], "{name_a}");
    }
}

#[test]
fn full_audio_with_a_rework_label_only_one_file_name_carries_goes_to_review() {
    let got = decided(
        full(),
        &file("Velo", "Velo (Quill Ashby Remix).mp3"),
        &file("Velo", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameAudioNamesDisagree", "namesSayRework"]);
}

#[test]
fn full_audio_with_a_bracket_nobody_understands_in_one_file_name_goes_to_review() {
    let got = decided(
        full(),
        &file("Velo", "Velo (Skyline Session).mp3"),
        &file("Velo", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [
            Reason::SameAudioNamesDisagree,
            Reason::NamesUnclear {
                doubts: vec!["unrecognizedDiffers".to_string()],
            },
        ]
    );
}

#[test]
fn a_files_labels_are_those_of_its_tag_and_its_file_name_together() {
    let got = decided(
        full(),
        &file("Velo (Clean)", "Velo (Intro).mp3"),
        &file("Velo (Dirty)", "Velo.mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty, MarkerKind::Intro])
    );
    assert_eq!(
        got.reasons,
        [Reason::VersionVeto {
            only_a: vec![MarkerKind::Clean, MarkerKind::Intro],
            only_b: vec![MarkerKind::Dirty],
        }]
    );
}

#[test]
fn a_file_whose_names_give_both_a_rework_and_a_cut_goes_to_review_on_full_audio() {
    let got = decided(
        full(),
        &file("Velo (Quill Ashby Remix)", "Velo (Clean).mp3"),
        &file("Velo", "Velo (Dirty).mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameAudioNamesDisagree", "namesSayRework"]);
}

// ---- A label both files carry is no difference -------------------------

#[test]
fn two_files_that_both_say_dirty_one_in_its_tag_and_one_in_its_file_name_are_duplicates() {
    let got = decided(
        full(),
        &file("Velo", "Velo (Dirty).mp3"),
        &file("Velo (Dirty)", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn two_files_tagged_the_same_cut_are_duplicates_though_only_one_file_name_says_so() {
    let got = decided(
        full(),
        &file("Velo (Clean)", "Velo (Clean).mp3"),
        &file("Velo (Clean)", "Velo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn two_files_that_both_end_in_the_same_version_word_are_duplicates() {
    let got = decided(
        full(),
        &file("Velo (dirty)", "Velo (dirty).mp3"),
        &file("Velo (dirty)", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn the_same_remix_named_in_one_files_tag_and_the_other_files_file_name_is_a_duplicate() {
    let got = decided(
        full(),
        &file("Velo (Quill Ashby Remix)", "nightdrive promo.mp3"),
        &file("Velo", "Velo (Quill Ashby's Remix).mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn a_remix_one_name_leaves_unnamed_is_the_remix_the_files_other_name_gives() {
    // Tagged "(Remix)", named "(Quill Ashby Remix)": one remix, known by
    // name, and the same as the other file's.
    let got = decided(
        full(),
        &file("Velo (Remix)", "Velo (Quill Ashby Remix).mp3"),
        &file("Velo (Quill Ashby Remix)", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

// ---- Two titles on both readings: the labels still count ---------------

#[test]
fn full_audio_under_two_titles_with_clean_in_one_files_tag_and_dirty_in_the_others_file_name_is_a_cut_link(
) {
    let got = decided(
        full(),
        &file("Velo (Clean)", "other promo.mp3"),
        &file("fourth promo", "Velo (Dirty).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(
        got.reasons,
        [
            Reason::VersionVeto {
                only_a: vec![MarkerKind::Clean],
                only_b: vec![MarkerKind::Dirty],
            },
            Reason::TitlesDiffer,
        ]
    );
}

#[test]
fn full_audio_under_two_titles_with_a_rework_label_on_one_file_goes_to_review() {
    let got = decided(
        full(),
        &file("Velo (Quill Ashby Remix)", "other promo.mp3"),
        &file("fourth promo", "Velo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["sameAudioNamesDisagree", "namesSayRework", "titlesDiffer"]
    );
}

#[test]
fn full_audio_under_two_titles_with_a_bracket_nobody_understands_goes_to_review() {
    let got = decided(
        full(),
        &titled("Velo (Skyline Session)"),
        &titled("Nightdrive"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["sameAudioNamesDisagree", "namesUnclear", "titlesDiffer"]
    );
}

#[test]
fn full_audio_under_two_titles_with_the_same_labels_is_a_duplicate_and_says_the_names_differ() {
    for (a, b) in [
        (
            file("Velo (Clean)", "other promo.mp3"),
            file("fourth promo", "Velo (Clean).mp3"),
        ),
        (titled("Velo (Clean)"), titled("Nightdrive (Clean)")),
        (titled("Velo"), titled("Nightdrive")),
    ] {
        let got = decided(full(), &a, &b);
        assert_eq!(got.outcome, Outcome::Duplicate);
        assert_eq!(codes(&got), ["sameAudio", "namesDiffer"]);
    }
}

#[test]
fn full_audio_under_two_titles_leaves_label_words_in_a_title_alone() {
    // "Come Clean" beside an unrelated title: nothing says "Clean" is a
    // cut here, so it is a duplicate under another name.
    let got = decided(full(), &titled("Come Clean"), &titled("Nightdrive"));
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio", "namesDiffer"]);
}

#[test]
fn without_matched_audio_labels_under_two_titles_decide_nothing() {
    for audio in [
        partial(),
        nothing_shared(),
        Audio::NotCompared,
        Audio::Unknown,
    ] {
        let got = decided(
            audio,
            &titled("Velo (Clean)"),
            &titled("Nightdrive (Dirty)"),
        );
        assert_eq!(got.outcome, Outcome::Different);
        assert_eq!(got.reasons, [Reason::TitlesDiffer]);
    }
}

// ---- One file inside the other -----------------------------------------

#[test]
fn one_sided_coverage_takes_its_labels_from_a_file_name_when_the_tags_carry_none() {
    let got = decided(
        a_inside_b(),
        &file("Velo", "Velo (Radio Edit).mp3"),
        &file("Velo", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, cut_link(&[MarkerKind::RadioEdit]));
    assert_eq!(codes(&got), ["audioInsideOther", "namesSayCut"]);
}

#[test]
fn one_sided_coverage_where_the_files_differ_by_a_cut_and_a_rework_goes_to_review() {
    // Tagged as a remix beside the plain title, named Radio Edit beside
    // Extended Mix: not simply two cuts of one production.
    let got = decided(
        a_inside_b(),
        &file("Velo (Quill Ashby Remix)", "Velo (Radio Edit).mp3"),
        &file("Velo", "Velo (Extended Mix).mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["audioInsideOtherNamesDisagree", "namesSayRework"]
    );
}

#[test]
fn one_sided_coverage_with_a_rework_label_only_one_file_name_carries_goes_to_review() {
    let got = decided(
        a_inside_b(),
        &file("Velo", "nightdrive promo.mp3"),
        &file("Velo", "Velo (Quill Ashby Remix).mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["audioInsideOtherNamesDisagree", "namesSayRework"]
    );
}

#[test]
fn one_sided_coverage_under_two_titles_is_a_cut_link_when_the_labels_give_two_cuts() {
    let got = decided(
        a_inside_b(),
        &file("Velo (Radio Edit)", "other promo.mp3"),
        &file("fourth promo", "Velo (Extended Mix).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Extended, MarkerKind::RadioEdit])
    );
    assert_eq!(
        codes(&got),
        ["audioInsideOther", "namesSayCut", "titlesDiffer"]
    );
}

#[test]
fn one_sided_coverage_under_two_titles_with_a_rework_label_goes_to_review_and_says_which() {
    let got = decided(
        a_inside_b(),
        &file("Velo (Quill Ashby Remix)", "other promo.mp3"),
        &file("fourth promo", "Velo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        [
            "audioInsideOtherNamesDisagree",
            "namesSayRework",
            "titlesDiffer"
        ]
    );
}

#[test]
fn one_sided_coverage_under_two_titles_still_goes_to_review_when_the_labels_cant_be_told_or_are_the_same(
) {
    for (a, b) in [
        ("Velo (Skyline Session)", "Nightdrive"),
        ("Velo (Clean)", "Nightdrive (Clean)"),
        ("Velo", "Nightdrive"),
    ] {
        let got = decided(a_inside_b(), &titled(a), &titled(b));
        assert_eq!(got.outcome, Outcome::NeedsReview, "{a} / {b}");
        assert_eq!(
            codes(&got),
            ["audioInsideOtherNamesDisagree", "titlesDiffer"],
            "{a} / {b}"
        );
    }
}

// ---- Identical audio ---------------------------------------------------

#[test]
fn identical_audio_says_the_names_differ_when_a_label_sits_in_one_file_name_only() {
    let got = decided(
        Audio::Identical,
        &file("Velo", "Velo (Dirty).mp3"),
        &file("Velo", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(got.reasons, [Reason::IdenticalAudio, Reason::NamesDiffer]);
}

// ---- Little or no audio: the names decide ------------------------------

#[test]
fn with_low_or_no_coverage_a_label_only_a_file_name_carries_still_makes_the_link() {
    for audio in [
        partial(),
        nothing_shared(),
        Audio::NotCompared,
        Audio::Unknown,
    ] {
        let cut = decided(
            audio,
            &file("Velo", "Velo (Extended Mix).mp3"),
            &file("Velo", "nightdrive promo.mp3"),
        );
        assert_eq!(cut.outcome, cut_link(&[MarkerKind::Extended]));
        assert_eq!(codes(&cut), ["namesSayCut"]);

        let rework = decided(
            audio,
            &file("Velo", "nightdrive promo.mp3"),
            &file("Velo", "Velo [Vey Sun Flip].mp3"),
        );
        assert_eq!(rework.outcome, rework_link(&[MarkerKind::Flip]));
        assert_eq!(codes(&rework), ["namesSayRework"]);
    }
}

#[test]
fn with_low_or_no_coverage_a_file_whose_names_give_a_rework_and_a_cut_is_a_rework_link() {
    let got = decided(
        nothing_shared(),
        &file("Velo (Quill Ashby Remix)", "Velo (Extended Mix).mp3"),
        &file("Velo", "Velo.mp3"),
    );
    assert_eq!(got.outcome, rework_link(&[MarkerKind::Remix]));
}

#[test]
fn a_bracket_nobody_understands_in_either_name_makes_the_files_names_unclear() {
    let (a, b) = (
        file("Velo", "Velo (Skyline Session).mp3"),
        file("Velo", "Velo.mp3"),
    );
    let part = decided(partial(), &a, &b);
    assert_eq!(part.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&part), ["partialAudioNamesUnclear", "namesUnclear"]);
    let none = decided(nothing_shared(), &a, &b);
    assert_eq!(none.outcome, Outcome::Different);
    assert_eq!(codes(&none), ["namesUnclear"]);
}

// ---- Remaster years ----------------------------------------------------

#[test]
fn full_audio_between_two_remasters_of_different_years_is_a_cut_link() {
    let got = decided(
        full(),
        &titled("Velo (2019 Remaster)"),
        &titled("Velo (2011 Remaster)"),
    );
    assert_eq!(got.outcome, cut_link(&[MarkerKind::Remaster]));
    assert_eq!(
        got.reasons,
        [Reason::VersionVeto {
            only_a: vec![MarkerKind::Remaster],
            only_b: vec![MarkerKind::Remaster],
        }]
    );
}

#[test]
fn full_audio_between_a_remaster_with_a_year_and_one_without_is_a_duplicate() {
    let got = decided(
        full(),
        &titled("Velo (2019 Remaster)"),
        &titled("Velo (Remastered)"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn a_remasters_year_in_the_file_name_counts_for_a_file_tagged_remaster() {
    let got = decided(
        full(),
        &file("Velo (Remaster)", "Velo (2019 Remaster).mp3"),
        &file("Velo (2011 Remaster)", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, cut_link(&[MarkerKind::Remaster]));
}
