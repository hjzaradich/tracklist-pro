//! One test per row of the decision table (module docs).

use super::*;

// ---- Identical audio ---------------------------------------------------

#[test]
fn identical_audio_with_the_same_names_is_a_duplicate() {
    let got = decided(
        Audio::Identical,
        &titled("Paper Harbor"),
        &titled("Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(got.reasons, [Reason::IdenticalAudio]);
}

#[test]
fn identical_audio_is_a_duplicate_even_when_the_names_say_clean_and_dirty_and_says_the_names_differ(
) {
    let got = decided(
        Audio::Identical,
        &titled("Velo (Clean)"),
        &titled("Velo (Dirty)"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(got.reasons, [Reason::IdenticalAudio, Reason::NamesDiffer]);
}

#[test]
fn identical_audio_is_a_duplicate_whatever_the_names_say_and_says_when_they_differ() {
    for (a, b) in [
        ("Paper Harbor", "Neon Moth"),
        ("Paper Harbor (Quill Ashby Remix)", "Paper Harbor"),
        ("Glasswing (Skyline Session)", "Glasswing"),
        ("Duskline x Emberlow (Mashup)", "Duskline"),
    ] {
        let got = decided(Audio::Identical, &titled(a), &titled(b));
        assert_eq!(got.outcome, Outcome::Duplicate, "{a} / {b}");
        assert_eq!(
            got.reasons,
            [Reason::IdenticalAudio, Reason::NamesDiffer],
            "{a} / {b}"
        );
    }
}

#[test]
fn identical_audio_under_different_artists_is_a_duplicate_and_says_the_artists_differ() {
    let got = decided(
        Audio::Identical,
        &by("Kestrel Nine", "Paper Harbor"),
        &by("The Marrow Choir", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(got.reasons, [Reason::IdenticalAudio, Reason::ArtistsDiffer]);
}

// ---- Full coverage both ways -------------------------------------------

#[test]
fn full_coverage_both_ways_with_the_same_names_is_a_duplicate_and_gives_the_measurements() {
    let got = decided(full(), &titled("Paper Harbor"), &titled("Paper Harbor"));
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(
        got.reasons,
        [Reason::SameAudio {
            coverage_a: 0.98,
            coverage_b: 0.97,
            score: 1.5,
        }]
    );
}

#[test]
fn full_coverage_under_another_title_is_a_duplicate_and_says_the_names_differ() {
    let got = decided(full(), &titled("Paper Harbor"), &titled("Neon Moth"));
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio", "namesDiffer"]);
}

#[test]
fn full_coverage_under_the_same_title_and_different_artists_is_a_duplicate_and_says_the_artists_differ(
) {
    let got = decided(
        full(),
        &by("Kestrel Nine", "Paper Harbor"),
        &by("The Marrow Choir", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio", "artistsDiffer"]);
}

#[test]
fn the_version_check_vetoes_a_merge_clean_and_dirty_with_full_coverage_get_a_cut_link() {
    let got = decided(full(), &titled("Velo (Clean)"), &titled("Velo (Dirty)"));
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(
        got.reasons,
        [Reason::VersionVeto {
            only_a: vec![MarkerKind::Clean],
            only_b: vec![MarkerKind::Dirty],
        }]
    );
}

#[test]
fn the_version_check_vetoes_a_merge_for_every_pair_of_cuts_the_names_tell_apart() {
    let cases: [(&str, &str, &[MarkerKind]); 4] = [
        ("Velo (Dirty)", "Velo", &[MarkerKind::Dirty]),
        (
            "Velo (Extended Mix)",
            "Velo (Radio Edit)",
            &[MarkerKind::Extended, MarkerKind::RadioEdit],
        ),
        ("Velo (Intro)", "Velo (Original Mix)", &[MarkerKind::Intro]),
        ("Velo (2019 Remaster)", "Velo", &[MarkerKind::Remaster]),
    ];
    for (a, b, labels) in cases {
        let got = decided(full(), &titled(a), &titled(b));
        assert_eq!(got.outcome, cut_link(labels), "{a} / {b}");
        assert_eq!(codes(&got), ["versionVeto"], "{a} / {b}");
    }
}

#[test]
fn full_coverage_between_a_remix_and_the_plain_title_goes_to_review_with_what_the_names_say() {
    let got = decided(
        full(),
        &titled("Paper Harbor (Quill Ashby Remix)"),
        &titled("Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [
            Reason::SameAudioNamesDisagree,
            Reason::NamesSayRework {
                only_a: vec![Rework {
                    kind: MarkerKind::Remix,
                    whose: Some("Quill Ashby".to_string()),
                }],
                only_b: vec![],
            },
        ]
    );
}

#[test]
fn full_coverage_with_names_that_cant_be_told_goes_to_review_with_the_doubt() {
    let got = decided(
        full(),
        &titled("Glasswing (Skyline Session)"),
        &titled("Glasswing"),
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
fn full_coverage_between_a_mashup_and_the_part_it_names_goes_to_review() {
    let got = decided(
        full(),
        &titled("Duskline x Emberlow (Mashup)"),
        &titled("Duskline"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [
            Reason::SameAudioNamesDisagree,
            Reason::MashupPart {
                mashup: Side::A,
                part: "Duskline".to_string(),
            },
        ]
    );
}

// ---- One file inside the other -----------------------------------------

#[test]
fn one_sided_coverage_with_the_same_names_is_a_cut_link_never_a_merge() {
    let got = decided(
        a_inside_b(),
        &titled("Paper Harbor"),
        &titled("Paper Harbor"),
    );
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(
        got.reasons,
        [Reason::AudioInsideOther {
            inner: Side::A,
            found_a: 0.99,
            found_b: 0.41,
        }]
    );
}

#[test]
fn one_sided_coverage_names_the_file_that_sits_inside_the_other() {
    let got = decided(
        b_inside_a(),
        &titled("Paper Harbor"),
        &titled("Paper Harbor"),
    );
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(
        got.reasons,
        [Reason::AudioInsideOther {
            inner: Side::B,
            found_a: 0.41,
            found_b: 0.99,
        }]
    );
}

#[test]
fn one_sided_coverage_takes_the_cut_links_labels_from_the_names() {
    let got = decided(
        a_inside_b(),
        &titled("Paper Harbor (Radio Edit)"),
        &titled("Paper Harbor (Extended Mix)"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Extended, MarkerKind::RadioEdit])
    );
    assert_eq!(codes(&got), ["audioInsideOther", "namesSayCut"]);
    assert_eq!(
        got.reasons[1],
        Reason::NamesSayCut {
            only_a: vec![MarkerKind::RadioEdit],
            only_b: vec![MarkerKind::Extended],
        }
    );
}

#[test]
fn one_sided_coverage_with_names_that_cant_be_told_is_still_a_cut_link_and_gives_the_doubt() {
    let got = decided(
        a_inside_b(),
        &titled("Glasswing (Skyline Session)"),
        &titled("Glasswing"),
    );
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther", "namesUnclear"]);
}

#[test]
fn one_sided_coverage_under_the_same_title_and_different_artists_is_a_cut_link_and_says_the_artists_differ(
) {
    let got = decided(
        a_inside_b(),
        &by("Kestrel Nine", "Paper Harbor"),
        &by("The Marrow Choir", "Paper Harbor"),
    );
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther", "artistsDiffer"]);
}

#[test]
fn one_sided_coverage_between_a_remix_and_the_plain_title_goes_to_review() {
    let got = decided(
        a_inside_b(),
        &titled("Paper Harbor"),
        &titled("Paper Harbor (Quill Ashby Remix)"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["audioInsideOtherNamesDisagree", "namesSayRework"]
    );
    assert_eq!(
        got.reasons[0],
        Reason::AudioInsideOtherNamesDisagree {
            inner: Side::A,
            found_a: 0.99,
            found_b: 0.41,
        }
    );
}

#[test]
fn one_sided_coverage_under_another_title_goes_to_review() {
    let got = decided(a_inside_b(), &titled("Paper Harbor"), &titled("Neon Moth"));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["audioInsideOtherNamesDisagree", "titlesDiffer"]
    );
}

// ---- Mashups -----------------------------------------------------------

#[test]
fn a_mashup_sharing_audio_with_the_part_it_names_is_a_rework_link_labeled_mashup() {
    for (what, audio) in [
        ("part inside", b_inside_a()),
        ("mashup inside", a_inside_b()),
        ("partial", partial()),
    ] {
        let got = decided(
            audio,
            &titled("Duskline x Emberlow (Mashup)"),
            &titled("Duskline"),
        );
        assert_eq!(got.outcome, rework_link(&[MarkerKind::Mashup]), "{what}");
        assert_eq!(
            got.reasons,
            [Reason::MashupContains {
                mashup: Side::A,
                part: "Duskline".to_string(),
            }],
            "{what}"
        );
    }
}

#[test]
fn a_mashup_sharing_no_audio_with_the_part_it_names_is_different_the_name_is_only_a_hint() {
    for audio in [nothing_shared(), Audio::NotCompared, Audio::Unknown] {
        let got = decided(
            audio,
            &titled("Duskline"),
            &titled("Duskline x Emberlow (Mashup)"),
        );
        assert_eq!(got.outcome, Outcome::Different);
        assert_eq!(
            got.reasons,
            [Reason::MashupPart {
                mashup: Side::B,
                part: "Duskline".to_string(),
            }]
        );
    }
}

#[test]
fn two_mashups_sharing_a_part_are_not_each_others_source() {
    let (a, b) = (
        titled("Duskline x Emberlow (Mashup)"),
        titled("Duskline x Korvex (Mashup)"),
    );
    let with_audio = decided(partial(), &a, &b);
    assert_eq!(with_audio.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&with_audio),
        ["partialAudioNamesUnclear", "namesUnclear"]
    );
    let without = decided(nothing_shared(), &a, &b);
    assert_eq!(without.outcome, Outcome::Different);
    assert_eq!(codes(&without), ["namesUnclear"]);
}

// ---- Partial audio -----------------------------------------------------

#[test]
fn a_partial_match_between_files_with_the_same_names_goes_to_review() {
    let got = decided(partial(), &titled("Paper Harbor"), &titled("Paper Harbor"));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [Reason::SameTagsPartialAudio {
            found_a: 0.55,
            found_b: 0.48,
        }]
    );
}

#[test]
fn a_partial_match_with_names_that_cant_be_told_goes_to_review_with_the_doubt() {
    let got = decided(
        partial(),
        &titled("Paper Harbor (Remix)"),
        &titled("Paper Harbor (Quill Ashby Remix)"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        got.reasons,
        [
            Reason::PartialAudioNamesUnclear {
                found_a: 0.55,
                found_b: 0.48,
            },
            Reason::NamesUnclear {
                doubts: vec!["reworkDetailMissing".to_string()],
            },
        ]
    );
}

#[test]
fn a_partial_match_under_the_same_title_and_different_artists_goes_to_review() {
    let got = decided(
        partial(),
        &by("Kestrel Nine", "Paper Harbor"),
        &by("The Marrow Choir", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["partialAudioNamesUnclear", "artistsDiffer"]);
}

#[test]
fn a_partial_match_between_a_remix_and_the_title_under_different_artists_goes_to_review_not_to_a_link(
) {
    let got = decided(
        partial(),
        &by("Kestrel Nine", "Paper Harbor (Quill Ashby Remix)"),
        &by("The Marrow Choir", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        [
            "partialAudioNamesUnclear",
            "namesSayRework",
            "artistsDiffer"
        ]
    );
}

#[test]
fn a_partial_match_under_another_title_is_different() {
    let got = decided(partial(), &titled("Paper Harbor"), &titled("Neon Moth"));
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::TitlesDiffer]);
}

// ---- Low or no coverage: the names decide ------------------------------

#[test]
fn with_low_or_no_coverage_the_names_decide_a_cut_link() {
    for (what, audio) in [
        ("partial", partial()),
        ("nothing shared", nothing_shared()),
        ("not compared", Audio::NotCompared),
        ("unknown", Audio::Unknown),
    ] {
        let got = decided(
            audio,
            &titled("Paper Harbor (Extended Mix)"),
            &titled("Paper Harbor"),
        );
        assert_eq!(got.outcome, cut_link(&[MarkerKind::Extended]), "{what}");
        assert_eq!(
            got.reasons,
            [Reason::NamesSayCut {
                only_a: vec![MarkerKind::Extended],
                only_b: vec![],
            }],
            "{what}"
        );
    }
}

#[test]
fn with_low_or_no_coverage_the_names_decide_a_rework_link_and_give_its_kind() {
    for (what, audio) in [
        ("partial", partial()),
        ("nothing shared", nothing_shared()),
        ("not compared", Audio::NotCompared),
        ("unknown", Audio::Unknown),
    ] {
        let got = decided(
            audio,
            &titled("Paper Harbor"),
            &titled("Paper Harbor (Quill Ashby Remix)"),
        );
        assert_eq!(got.outcome, rework_link(&[MarkerKind::Remix]), "{what}");
        assert_eq!(
            got.reasons,
            [Reason::NamesSayRework {
                only_a: vec![],
                only_b: vec![Rework {
                    kind: MarkerKind::Remix,
                    whose: Some("Quill Ashby".to_string()),
                }],
            }],
            "{what}"
        );
    }
}

#[test]
fn a_rework_link_is_labeled_with_every_kind_the_names_tell_apart_each_once() {
    let cases: [(&str, &str, &[MarkerKind]); 4] = [
        (
            "Korvex (Rivetta Remix)",
            "Korvex (Flint Remix)",
            &[MarkerKind::Remix],
        ),
        (
            "Korvex (Rivetta Remix)",
            "Korvex (Dub)",
            &[MarkerKind::Remix, MarkerKind::Dub],
        ),
        (
            "Korvex (Instrumental)",
            "Korvex",
            &[MarkerKind::Instrumental],
        ),
        (
            "Korvex (VIP)",
            "Korvex (Acapella)",
            &[MarkerKind::Vip, MarkerKind::Acapella],
        ),
    ];
    for (a, b, labels) in cases {
        let got = decided(nothing_shared(), &titled(a), &titled(b));
        assert_eq!(got.outcome, rework_link(labels), "{a} / {b}");
        assert_eq!(codes(&got), ["namesSayRework"], "{a} / {b}");
    }
}

#[test]
fn a_rework_that_is_also_another_cut_is_a_rework_link() {
    let got = decided(
        nothing_shared(),
        &titled("Korvex (Rivetta Remix) (Extended Mix)"),
        &titled("Korvex"),
    );
    assert_eq!(got.outcome, rework_link(&[MarkerKind::Remix]));
}

#[test]
fn a_link_made_by_names_alone_needs_the_artists_to_agree() {
    for title in ["Paper Harbor (Extended Mix)", "Paper Harbor (Dub)"] {
        for audio in [nothing_shared(), Audio::NotCompared, Audio::Unknown] {
            let got = decided(
                audio,
                &by("Kestrel Nine", title),
                &by("The Marrow Choir", "Paper Harbor"),
            );
            assert_eq!(got.outcome, Outcome::Different, "{title}");
            assert_eq!(got.reasons, [Reason::SameTitleDifferentArtists], "{title}");
        }
    }
}

#[test]
fn a_link_made_by_names_alone_holds_when_the_artists_agree_or_one_file_names_none() {
    for (a, b) in [
        (
            by("Kestrel Nine", "Paper Harbor (Dub)"),
            by("Kestrel Nine", "Paper Harbor"),
        ),
        (
            by("Kestrel Nine & Halden Rook", "Paper Harbor (Dub)"),
            by("Halden Rook", "Paper Harbor"),
        ),
        (
            by("Kestrel Nine", "Paper Harbor (Dub)"),
            titled("Paper Harbor"),
        ),
    ] {
        let got = decided(nothing_shared(), &a, &b);
        assert_eq!(got.outcome, rework_link(&[MarkerKind::Dub]));
        assert_eq!(codes(&got), ["namesSayRework"]);
    }
}

// ---- The same names, little or no audio --------------------------------

#[test]
fn the_same_names_with_no_shared_audio_are_different_the_fingerprint_vetoes_the_merge() {
    let got = decided(
        nothing_shared(),
        &by("Kestrel Nine", "Paper Harbor"),
        &by("Kestrel Nine", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::FingerprintVeto]);
}

#[test]
fn the_same_names_never_compared_go_to_review_never_to_different() {
    let got = decided(
        Audio::NotCompared,
        &by("Kestrel Nine", "Paper Harbor"),
        &by("Kestrel Nine", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(got.reasons, [Reason::NotCompared]);
}

#[test]
fn the_same_names_with_no_fingerprint_to_check_them_go_to_review() {
    let got = decided(
        Audio::Unknown,
        &by("Kestrel Nine", "Paper Harbor"),
        &by("Kestrel Nine", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(got.reasons, [Reason::NoFingerprint]);
}

#[test]
fn the_same_title_under_different_artists_with_no_audio_to_go_on_is_different() {
    for audio in [nothing_shared(), Audio::NotCompared, Audio::Unknown] {
        let got = decided(
            audio,
            &by("Kestrel Nine", "Paper Harbor"),
            &by("The Marrow Choir", "Paper Harbor"),
        );
        assert_eq!(got.outcome, Outcome::Different);
        assert_eq!(got.reasons, [Reason::SameTitleDifferentArtists]);
    }
}

// ---- The rest ----------------------------------------------------------

#[test]
fn another_title_with_no_audio_to_go_on_is_different() {
    for audio in [nothing_shared(), Audio::NotCompared, Audio::Unknown] {
        let got = decided(audio, &titled("Paper Harbor"), &titled("Neon Moth"));
        assert_eq!(got.outcome, Outcome::Different);
        assert_eq!(got.reasons, [Reason::TitlesDiffer]);
    }
}

#[test]
fn names_that_cant_be_told_with_no_audio_to_go_on_are_different_and_give_the_doubt() {
    for audio in [nothing_shared(), Audio::NotCompared, Audio::Unknown] {
        let got = decided(
            audio,
            &titled("Paper Harbor Extended Mix"),
            &titled("Paper Harbor"),
        );
        assert_eq!(got.outcome, Outcome::Different);
        assert_eq!(
            got.reasons,
            [Reason::NamesUnclear {
                doubts: vec!["cutDiffers".to_string(), "titleReadAsMarker".to_string()],
            }]
        );
    }
}

#[test]
fn names_that_cant_be_told_under_different_artists_with_no_audio_are_different_for_the_names_first()
{
    let got = decided(
        nothing_shared(),
        &by("Kestrel Nine", "Paper Harbor (Skyline Session)"),
        &by("The Marrow Choir", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(codes(&got), ["namesUnclear", "artistsDiffer"]);
}

// ---- The inputs --------------------------------------------------------

#[test]
fn an_artist_tag_is_split_into_its_artists_so_one_shared_artist_is_agreement() {
    let got = decided(
        nothing_shared(),
        &by(
            "Kestrel Nine, Halden Rook & Vey Sun feat. Nox",
            "Paper Harbor (Dub)",
        ),
        &by("Vey Sun", "Paper Harbor"),
    );
    assert_eq!(got.outcome, rework_link(&[MarkerKind::Dub]));
    // A featured artist is no main artist: sharing only that one isn't
    // agreement.
    let featured_only = decided(
        nothing_shared(),
        &by("Kestrel Nine feat. Nox", "Paper Harbor (Dub)"),
        &by("Nox", "Paper Harbor"),
    );
    assert_eq!(featured_only.outcome, Outcome::Different);
}

#[test]
fn a_file_names_artist_counts_like_an_artist_tag() {
    let got = decided(
        nothing_shared(),
        &named("Kestrel Nine - Paper Harbor (Dub).mp3"),
        &by("The Marrow Choir", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::SameTitleDifferentArtists]);
    let agreeing = decided(
        nothing_shared(),
        &named("Kestrel Nine - Paper Harbor (Dub).mp3"),
        &by("Kestrel Nine", "Paper Harbor"),
    );
    assert_eq!(agreeing.outcome, rework_link(&[MarkerKind::Dub]));
}

#[test]
fn measurements_come_from_a_comparison_with_found_shares_and_score_over_every_alignment() {
    use crate::matching::{Comparison, Segment};
    let comparison = Comparison {
        items_a: 1000,
        items_b: 2000,
        coverage_a: 0.6,
        coverage_b: 0.3,
        score: 1.0,
        segments: vec![
            Segment {
                offset_a: 0,
                offset_b: 100,
                items: 600,
                score: 1.0,
                alignment: 0,
            },
            Segment {
                offset_a: 600,
                offset_b: 1200,
                items: 300,
                score: 4.0,
                alignment: 1,
            },
        ],
    };
    assert_eq!(
        Measured::try_from(&comparison).unwrap(),
        Measured {
            coverage_a: 0.6,
            coverage_b: 0.3,
            score: 1.0,
            found_a: 0.9,
            found_b: 0.45,
            found_score: 2.0,
            matched_items: 900,
        }
    );
}

#[test]
fn measurements_from_a_comparison_where_nothing_matched_share_nothing() {
    use crate::matching::Comparison;
    let comparison = Comparison {
        items_a: 1000,
        items_b: 2000,
        coverage_a: 0.0,
        coverage_b: 0.0,
        score: matching::compare::NO_MATCH_SCORE,
        segments: Vec::new(),
    };
    let measured = Measured::try_from(&comparison).unwrap();
    assert_eq!(measured.matched_items, 0);
    assert_eq!(measured.found_score, matching::compare::NO_MATCH_SCORE);
    let got = decided(
        Audio::Compared(measured),
        &titled("Paper Harbor"),
        &titled("Paper Harbor"),
    );
    assert_eq!(got.reasons, [Reason::FingerprintVeto]);
}

#[test]
fn swapping_measurements_swaps_the_two_files_shares_and_nothing_else() {
    let measured = Measured {
        coverage_a: 0.1,
        coverage_b: 0.2,
        score: 3.0,
        found_a: 0.4,
        found_b: 0.5,
        found_score: 3.5,
        matched_items: 77,
    };
    assert_eq!(
        measured.swapped(),
        Measured {
            coverage_a: 0.2,
            coverage_b: 0.1,
            score: 3.0,
            found_a: 0.5,
            found_b: 0.4,
            found_score: 3.5,
            matched_items: 77,
        }
    );
}
