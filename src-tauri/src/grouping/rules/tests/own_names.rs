//! A file's own two names are evidence about that file: its title tag and
//! its file name are set side by side, whatever the other file is called
//! (module docs, "A file's own two names").

use super::*;

/// A file with a title tag and a file name.
fn file(title: &str, file_name: &str) -> FileFacts {
    tagged_and_named(title, file_name)
}

/// A plainly tagged file whose file name says nothing about the title.
fn plain(title: &str) -> FileFacts {
    file(title, "nightdrive promo.mp3")
}

fn own_names_disagree() -> Reason {
    Reason::NamesUnclear {
        doubts: vec![OWN_NAMES_DISAGREE.to_string()],
    }
}

// ---- Label words without brackets in the file name ---------------------

#[test]
fn full_audio_goes_to_review_when_a_file_name_ends_in_a_bare_label_word_its_tag_lacks() {
    let got = decided(
        full(),
        &file("Velo", "Halden Rook - Velo Dirty.mp3"),
        &plain("Velo"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [Reason::SameAudioNamesDisagree, own_names_disagree()]
    );
}

#[test]
fn full_audio_goes_to_review_when_the_second_files_name_ends_in_a_bare_label_word_its_tag_lacks() {
    let got = decided(
        full(),
        &plain("Velo"),
        &file("Velo", "Halden Rook - Velo Dirty.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [Reason::SameAudioNamesDisagree, own_names_disagree()]
    );
}

#[test]
fn full_audio_never_merges_on_bare_label_words_in_one_file_name_whatever_the_other_is_named() {
    for name_a in [
        "Velo Clean.mp3",
        "Velo Extended Mix.mp3",
        "halden_rook_-_velo_clean.mp3",
        "Velo Dub.mp3",
    ] {
        for name_b in ["nightdrive promo.mp3", "other promo.flac"] {
            let got = decided(full(), &file("Velo", name_a), &file("Velo", name_b));
            assert_eq!(got.outcome, Outcome::NeedsReview, "{name_a} / {name_b}");
            assert_eq!(
                codes(&got),
                ["sameAudioNamesDisagree", "namesUnclear"],
                "{name_a} / {name_b}"
            );
        }
    }
}

#[test]
fn bare_label_words_in_the_tag_that_the_file_name_lacks_send_full_audio_to_review_too() {
    let got = decided(
        full(),
        &file("Velo Dirty", "Velo.mp3"),
        &plain("Velo Dirty"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [Reason::SameAudioNamesDisagree, own_names_disagree()]
    );
}

#[test]
fn a_file_name_that_adds_a_name_and_a_bare_label_word_to_its_tags_title_sends_full_audio_to_review()
{
    // The parser reads "Velo Quill Ashby" as another title; beside the
    // file's own tag "Velo" it may as well be Quill Ashby's remix.
    for (a, b) in [
        (file("Velo", "Velo Quill Ashby Remix.mp3"), plain("Velo")),
        (plain("Velo"), file("Velo", "Velo Quill Ashby Remix.mp3")),
        (
            file("Velo Quill Ashby Remix", "Velo.mp3"),
            plain("Velo Quill Ashby Remix"),
        ),
    ] {
        let got = decided(full(), &a, &b);
        assert_eq!(got.outcome, Outcome::NeedsReview);
        assert_eq!(
            got.reasons,
            [Reason::SameAudioNamesDisagree, own_names_disagree()]
        );
    }
}

#[test]
fn a_file_name_that_adds_words_but_no_label_word_to_its_tags_title_is_just_another_title() {
    let got = decided(full(), &file("Velo", "Velo Nights.mp3"), &plain("Velo"));
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn a_file_name_whose_title_only_starts_with_the_letters_of_its_tags_title_raises_no_doubt_of_its_own(
) {
    // "Velocity Clean" doesn't hold the title "Velo": no doubt from the
    // file's own names. Its "Clean" is still a version word the other
    // file lacks, so the final guard asks.
    let got = decided(full(), &file("Velo", "Velocity Clean.mp3"), &plain("Velo"));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["labelWordsDiffer", "sameAudio"]);
}

#[test]
fn a_file_name_with_its_tags_title_in_the_middle_and_a_label_word_sends_full_audio_to_review() {
    let got = decided(
        full(),
        &file("Velo", "Night Velo Quill Ashby Remix.mp3"),
        &plain("Velo"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [Reason::SameAudioNamesDisagree, own_names_disagree()]
    );
}

// ---- The net: a label word anywhere, behind any separator --------------

#[test]
fn full_audio_goes_to_review_when_a_file_name_holds_a_label_word_that_is_not_its_last_word() {
    for file_name in [
        "Velo Dirty 128.mp3",
        "Velo Dirty copy.mp3",
        "Velo-Dirty.mp3",
        "Dirty Velo.mp3",
        "Velo.Dirty.mp3",
        "Velo Dirty 2019.mp3",
        "Halden Rook - Velo Dirty copy.mp3",
        "velo_radio_edit_final.mp3",
    ] {
        // Both ways round: the doubted file first, then second.
        for (a, b) in [
            (file("Velo", file_name), plain("Velo")),
            (plain("Velo"), file("Velo", file_name)),
        ] {
            let got = decided(full(), &a, &b);
            assert_eq!(got.outcome, Outcome::NeedsReview, "{file_name}");
            assert_eq!(
                got.reasons,
                [Reason::SameAudioNamesDisagree, own_names_disagree()],
                "{file_name}"
            );
        }
    }
}

#[test]
fn full_audio_goes_to_review_when_the_tag_holds_a_label_word_that_is_not_its_last_word() {
    for title in ["Velo Dirty 320", "Dirty Velo"] {
        let got = decided(full(), &file(title, "Velo.mp3"), &plain(title));
        assert_eq!(got.outcome, Outcome::NeedsReview, "{title}");
        assert_eq!(
            got.reasons,
            [Reason::SameAudioNamesDisagree, own_names_disagree()],
            "{title}"
        );
    }
}

#[test]
fn full_audio_goes_to_review_when_a_bracketed_label_is_not_at_the_end_of_the_file_names_title() {
    // A title-first file name: the parser reads "Velo (Clean)" as the
    // artist. The file's tag says where the title is.
    for artist_tag in [None, Some("Halden Rook")] {
        let labeled = FileFacts::new(Some("Velo"), artist_tag, "Velo (Clean) - Halden Rook.mp3");
        for (a, b) in [(&labeled, &plain("Velo")), (&plain("Velo"), &labeled)] {
            let got = decided(full(), a, b);
            assert_eq!(got.outcome, Outcome::NeedsReview);
            assert_eq!(
                got.reasons,
                [Reason::SameAudioNamesDisagree, own_names_disagree()]
            );
        }
    }
}

#[test]
fn a_label_word_in_the_artist_part_of_a_file_name_raises_no_doubt() {
    // "Dub Kestrel" is the artist: "Dub" is no label of this file.
    for file_name in [
        "Dub Kestrel - Velo copy.mp3",
        "Dub Kestrel - Velo 128.mp3",
        "Dub Kestrel - Velo.mp3",
    ] {
        let got = decided(full(), &file("Velo", file_name), &plain("Velo"));
        assert_eq!(got.outcome, Outcome::Duplicate, "{file_name}");
        assert_eq!(codes(&got), ["sameAudio"], "{file_name}");
    }
}

#[test]
fn a_label_word_that_belongs_to_the_artist_tag_raises_no_doubt() {
    // No " - " in the file name, but the artist tag says who "Dub
    // Kestrel" is, alone or beside another artist.
    for artist_tag in ["Dub Kestrel", "Halden Rook & Dub Kestrel"] {
        let got = decided(
            full(),
            &FileFacts::new(Some("Velo"), Some(artist_tag), "Dub Kestrel Velo copy.mp3"),
            &FileFacts::new(Some("Velo"), Some(artist_tag), "nightdrive promo.mp3"),
        );
        assert_eq!(got.outcome, Outcome::Duplicate, "{artist_tag}");
        assert_eq!(codes(&got), ["sameAudio"], "{artist_tag}");
    }
}

#[test]
fn an_artists_label_word_counts_once_so_the_same_word_again_is_still_a_doubt() {
    // By "Dub Kestrel", and a dub: the second "Dub" is not the artist's.
    for (artist_tag, file_name) in [
        (None, "Dub Kestrel - Velo Dub copy.mp3"),
        (Some("Dub Kestrel"), "Dub Kestrel - Velo Dub copy.mp3"),
        (Some("Dub Kestrel"), "Dub Kestrel Velo Dub copy.mp3"),
    ] {
        let got = decided(
            full(),
            &FileFacts::new(Some("Velo"), artist_tag, file_name),
            &FileFacts::new(Some("Velo"), artist_tag, "nightdrive promo.mp3"),
        );
        assert_eq!(got.outcome, Outcome::NeedsReview, "{file_name}");
        assert_eq!(codes(&got)[1], "namesUnclear", "{file_name}");
    }
}

#[test]
fn with_no_artist_tag_and_no_dash_an_artists_label_word_cant_be_told_from_a_label() {
    // Known and cautious: review, never a wrong link.
    let got = decided(
        full(),
        &file("Velo", "Dub Kestrel Velo copy.mp3"),
        &plain("Velo"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [Reason::SameAudioNamesDisagree, own_names_disagree()]
    );
}

#[test]
fn a_label_the_parser_read_in_the_name_is_not_counted_again() {
    // "(Dirty)" is read as this file's label; "copy" keeps the titles
    // apart for the parser, and nothing else in the name is a label word.
    let got = decided(
        full(),
        &file("Velo (Dirty)", "Velo copy (Dirty).mp3"),
        &plain("Velo (Dirty)"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn original_among_the_other_words_is_no_label() {
    let got = decided(
        full(),
        &file("Velo", "Velo original copy.mp3"),
        &plain("Velo"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn two_files_both_named_with_the_same_label_word_in_the_middle_are_duplicates() {
    let got = decided(
        full(),
        &file("Velo", "Velo Dirty 128.mp3"),
        &file("Velo", "Velo Dirty 128.flac"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn a_label_word_in_the_middle_changes_nothing_without_matched_audio_and_keeps_a_cut_link_one_sided()
{
    let (a, b) = (file("Velo", "Velo Dirty 128.mp3"), plain("Velo"));
    assert_eq!(codes(&decided(partial(), &a, &b)), ["sameTagsPartialAudio"]);
    assert_eq!(codes(&decided(Audio::Unknown, &a, &b)), ["noFingerprint"]);
    let inside = decided(a_inside_b(), &a, &b);
    assert_eq!(inside.outcome, cut_link(&[]));
    assert_eq!(codes(&inside), ["audioInsideOther", "namesUnclear"]);
}

// ---- A title that ends in the label word -------------------------------

#[test]
fn a_file_tagged_come_clean_and_named_come_clean_clean_is_the_clean_cut_whatever_the_other_is_named(
) {
    for name_b in ["nightdrive promo.mp3", "Come Clean.mp3"] {
        let got = decided(
            full(),
            &file("Come Clean", "Come Clean (Clean).mp3"),
            &file("Come Clean", name_b),
        );
        assert_eq!(got.outcome, cut_link(&[MarkerKind::Clean]), "{name_b}");
        assert_eq!(
            got.reasons,
            [Reason::VersionVeto {
                only_a: vec![MarkerKind::Clean],
                only_b: vec![],
            }],
            "{name_b}"
        );
    }
}

#[test]
fn the_clean_cut_of_come_clean_is_never_merged_when_it_is_the_second_file() {
    let got = decided(
        full(),
        &plain("Come Clean"),
        &file("Come Clean", "Come Clean (Clean).mp3"),
    );
    assert_eq!(got.outcome, cut_link(&[MarkerKind::Clean]));
    assert_eq!(
        got.reasons,
        [Reason::VersionVeto {
            only_a: vec![],
            only_b: vec![MarkerKind::Clean],
        }]
    );
}

#[test]
fn two_files_both_tagged_come_clean_and_both_named_clean_are_duplicates() {
    let got = decided(
        full(),
        &file("Come Clean", "Come Clean (Clean).mp3"),
        &file("Come Clean", "Halden Rook - Come Clean (Clean).flac"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn a_title_that_ends_in_a_label_word_on_both_names_of_both_files_is_a_duplicate() {
    let got = decided(
        full(),
        &file("Come Clean", "Come Clean.mp3"),
        &plain("Come Clean"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

// ---- A mashup's "x" in one name only -----------------------------------

#[test]
fn full_audio_goes_to_review_when_only_a_file_name_makes_the_file_a_mashup() {
    let got = decided(
        full(),
        &file("Velo", "Velo x Nightdrive.mp3"),
        &plain("Velo"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [Reason::SameAudioNamesDisagree, own_names_disagree()]
    );
}

#[test]
fn full_audio_goes_to_review_when_only_the_second_files_name_makes_it_a_mashup() {
    let got = decided(
        full(),
        &plain("Velo"),
        &file("Velo", "Velo x Nightdrive.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [Reason::SameAudioNamesDisagree, own_names_disagree()]
    );
}

// ---- Own names that say the same, or nothing about each other ----------

#[test]
fn a_file_whose_tag_and_file_name_differ_only_by_original_mix_still_merges() {
    for (title, file_name) in [
        ("Velo", "Halden Rook - Velo (Original Mix).mp3"),
        ("Velo (Original Mix)", "Velo.mp3"),
    ] {
        let got = decided(full(), &file(title, file_name), &plain("Velo"));
        assert_eq!(got.outcome, Outcome::Duplicate, "{title} / {file_name}");
        assert_eq!(codes(&got), ["sameAudio"], "{title} / {file_name}");
    }
}

#[test]
fn a_file_whose_tag_and_file_name_are_the_same_still_merges() {
    for file_name in ["Velo.mp3", "Halden Rook - Velo.mp3", "01 Velo (320).mp3"] {
        let got = decided(full(), &file("Velo", file_name), &plain("Velo"));
        assert_eq!(got.outcome, Outcome::Duplicate, "{file_name}");
        assert_eq!(codes(&got), ["sameAudio"], "{file_name}");
    }
}

#[test]
fn a_file_whose_file_name_is_unrelated_to_its_tag_still_merges() {
    let got = decided(full(), &plain("Velo"), &file("Velo", "other promo.flac"));
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn a_bracketed_label_in_the_file_name_is_the_files_label_not_a_doubt() {
    // The same remix on both files, one saying so only in its file name.
    let got = decided(
        full(),
        &file("Velo", "Velo (Quill Ashby Remix).mp3"),
        &plain("Velo (Quill Ashby Remix)"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

// ---- A doubt both files carry is no difference -------------------------

#[test]
fn two_files_whose_own_names_disagree_in_the_same_way_are_duplicates() {
    let got = decided(
        full(),
        &file("Velo", "Halden Rook - Velo Dirty.mp3"),
        &file("Velo", "Velo Dirty.flac"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn two_files_whose_own_names_disagree_in_different_ways_go_to_review() {
    let got = decided(
        full(),
        &file("Velo", "Velo Dirty.mp3"),
        &file("Velo", "Velo Clean.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got)[0], "sameAudioNamesDisagree");
}

// ---- The other audio cases ---------------------------------------------

#[test]
fn full_audio_under_two_titles_goes_to_review_when_one_files_own_names_disagree() {
    let got = decided(
        full(),
        &file("Velo", "Velo Dirty.mp3"),
        &file("Nightdrive", "nightdrive promo.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [
            Reason::SameAudioNamesDisagree,
            own_names_disagree(),
            Reason::TitlesDiffer,
        ]
    );
}

#[test]
fn one_sided_coverage_stays_a_cut_link_and_gives_the_doubt_when_one_files_own_names_disagree() {
    let got = decided(
        a_inside_b(),
        &file("Velo", "Velo Dirty.mp3"),
        &plain("Velo"),
    );
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther", "namesUnclear"]);
    assert_eq!(got.reasons[1], own_names_disagree());
}

#[test]
fn identical_audio_is_a_duplicate_and_says_the_names_differ_when_one_files_own_names_disagree() {
    let got = decided(
        Audio::Identical,
        &file("Velo", "Velo Dirty.mp3"),
        &plain("Velo"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(got.reasons, [Reason::IdenticalAudio, Reason::NamesDiffer]);
}

#[test]
fn without_matched_audio_a_files_own_names_decide_nothing() {
    let (a, b) = (file("Velo", "Velo Dirty.mp3"), plain("Velo"));
    for (audio, code) in [
        (partial(), "sameTagsPartialAudio"),
        (nothing_shared(), "fingerprintVeto"),
        (Audio::NotCompared, "notCompared"),
        (Audio::Unknown, "noFingerprint"),
    ] {
        assert_eq!(codes(&decided(audio, &a, &b)), [code]);
    }
}

// ---- Known, and safe ---------------------------------------------------

#[test]
fn a_bare_label_word_beside_the_same_label_in_brackets_is_a_cut_link_not_a_merge() {
    // Both files are probably the Dirty cut; the bare word can't be
    // relied on, so they are linked, never merged.
    let got = decided(
        full(),
        &file("Velo", "Velo Dirty.mp3"),
        &file("Velo", "Velo (Dirty).mp3"),
    );
    assert_eq!(got.outcome, cut_link(&[MarkerKind::Dirty]));
    assert_eq!(codes(&got), ["versionVeto"]);
}

#[test]
fn two_files_under_two_titles_whose_own_names_each_disagree_go_to_review() {
    // Each file's own names leave its version in doubt, and not the same
    // doubt: neither settles the other.
    let got = decided(
        full(),
        &file("Velo", "Velo Dirty.mp3"),
        &file("Nightdrive", "Nightdrive Clean.mp3"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [
            Reason::SameAudioNamesDisagree,
            own_names_disagree(),
            Reason::TitlesDiffer,
        ]
    );
}
