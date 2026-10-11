//! One test (or a few) per rule under "Rules from the E3 labels" in
//! ROADMAP §1.5, each named after its rule.

use super::*;
use crate::matching::{Comparison, Segment};

// ---- Length decides nothing --------------------------------------------

/// A comparison where one stretch of `matched` items is shared, for files of the
/// given lengths in fingerprint items.
fn one_stretch(items_a: usize, items_b: usize, matched: usize) -> Comparison {
    Comparison {
        items_a,
        items_b,
        coverage_a: matched as f32 / items_a as f32,
        coverage_b: matched as f32 / items_b as f32,
        score: 1.0,
        segments: vec![Segment {
            offset_a: 0,
            offset_b: 0,
            items: matched,
            score: 1.0,
            alignment: 0,
        }],
    }
}

#[test]
fn length_decides_nothing_a_short_pair_and_a_long_pair_with_the_same_shares_get_the_same_decision()
{
    let (a, b) = (titled("Glasswing"), titled("Glasswing"));
    // About one minute and about ten minutes, each matched to 95%.
    let short = Measured::try_from(&one_stretch(500, 500, 475)).unwrap();
    let long = Measured::try_from(&one_stretch(5000, 5000, 4750)).unwrap();
    let (short, long) = (
        decided(Audio::Compared(short), &a, &b),
        decided(Audio::Compared(long), &a, &b),
    );
    assert_eq!(short.outcome, Outcome::Duplicate);
    assert_eq!(short, long);
}

#[test]
fn length_decides_nothing_a_running_time_in_the_name_changes_no_decision() {
    for (what, audio) in every_audio() {
        let timed = decided(
            audio,
            &titled("Glasswing (2:58)"),
            &titled("Glasswing (9:41)"),
        );
        let plain = decided(audio, &titled("Glasswing"), &titled("Glasswing"));
        assert_eq!(timed, plain, "{what}");
    }
}

#[test]
fn length_decides_nothing_the_file_called_extended_may_be_the_one_inside_the_other() {
    let (radio, extended) = (
        titled("Glasswing (Radio Edit)"),
        titled("Glasswing (Extended Mix)"),
    );
    let labels = [MarkerKind::Extended, MarkerKind::RadioEdit];
    // The usual way round, and the "wrong" one: the same cut link.
    let usual = decided(a_inside_b(), &radio, &extended);
    let unusual = decided(b_inside_a(), &radio, &extended);
    assert_eq!(usual.outcome, cut_link(&labels));
    assert_eq!(unusual.outcome, cut_link(&labels));
    assert_eq!(codes(&usual), codes(&unusual));
}

// ---- Version words can be the real title -------------------------------

#[test]
fn version_words_can_be_the_real_title_two_files_both_titled_with_the_word_are_duplicates() {
    let got = decided(full(), &titled("Velo (dirty)"), &titled("Velo (dirty)"));
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn version_words_can_be_the_real_title_the_other_file_settles_whether_the_word_is_a_cut() {
    // Beside the plain title, "(dirty)" is the Dirty cut: never merged.
    let beside_plain = decided(full(), &titled("Velo (dirty)"), &titled("Velo"));
    assert_eq!(beside_plain.outcome, cut_link(&[MarkerKind::Dirty]));
    // Beside the name that carries it twice, it is the title, and the
    // other file is that title's Dirty cut.
    let beside_twice = decided(
        full(),
        &titled("Velo (dirty)"),
        &titled("Velo (dirty) (dirty)"),
    );
    assert_eq!(beside_twice.outcome, cut_link(&[MarkerKind::Dirty]));
    assert_eq!(
        beside_twice.reasons,
        [Reason::VersionVeto {
            only_a: vec![],
            only_b: vec![MarkerKind::Dirty],
        }]
    );
}

#[test]
fn version_words_can_be_the_real_title_a_bare_word_is_never_stripped_to_make_a_merge_or_a_link() {
    let (a, b) = (titled("Come Clean"), titled("Come"));
    // The same audio: "Clean" may be the title or the cut, so the user
    // decides. Not merged, and not vetoed either.
    let same_audio = decided(full(), &a, &b);
    assert_eq!(same_audio.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&same_audio),
        ["sameAudioNamesDisagree", "namesUnclear"]
    );
    // No audio shared: two different titles, as far as anyone can tell.
    let no_audio = decided(nothing_shared(), &a, &b);
    assert_eq!(no_audio.outcome, Outcome::Different);
    assert_eq!(codes(&no_audio), ["namesUnclear"]);
}

// ---- Remixes are often credited only to the remixer --------------------

#[test]
fn remixes_are_often_credited_only_to_the_remixer_the_remix_still_links_to_the_artists_original() {
    let remix = by("Quill Ashby", "Paper Harbor (Quill Ashby Remix)");
    let original = by("Kestrel Nine", "Paper Harbor");
    for audio in [
        partial(),
        nothing_shared(),
        Audio::NotCompared,
        Audio::Unknown,
    ] {
        let got = decided(audio, &remix, &original);
        assert_eq!(got.outcome, rework_link(&[MarkerKind::Remix]));
        assert_eq!(
            got.reasons,
            [
                Reason::NamesSayRework {
                    only_a: vec![Rework {
                        kind: MarkerKind::Remix,
                        whose: Some("Quill Ashby".to_string()),
                    }],
                    only_b: vec![],
                },
                Reason::CreditedToRemixer {
                    sides: vec![Side::A],
                },
            ]
        );
    }
}

#[test]
fn remixes_are_often_credited_only_to_the_remixer_two_files_of_the_remix_are_still_duplicates() {
    let got = decided(
        full(),
        &by("Kestrel Nine", "Paper Harbor (Quill Ashby Remix)"),
        &by("Quill Ashby", "Paper Harbor (Quill Ashby Remix)"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(
        got.reasons[1..],
        [Reason::CreditedToRemixer {
            sides: vec![Side::B],
        }]
    );
    assert_eq!(codes(&got), ["sameAudio", "creditedToRemixer"]);
}

#[test]
fn remixes_are_often_credited_only_to_the_remixer_any_other_artist_is_still_a_different_song() {
    // Credited to someone who is neither the artist nor the remixer.
    let got = decided(
        nothing_shared(),
        &by("The Marrow Choir", "Paper Harbor (Quill Ashby Remix)"),
        &by("Kestrel Nine", "Paper Harbor"),
    );
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::SameTitleDifferentArtists]);
}

#[test]
fn remixes_are_often_credited_only_to_the_remixer_two_remixes_each_under_its_remixer_are_linked() {
    // The names can't say whether both remix the same "Korvex": with each
    // file credited to its own remixer, nothing speaks against it.
    let got = decided(
        nothing_shared(),
        &by("Rivetta", "Korvex (Rivetta Remix)"),
        &by("Flint", "Korvex (Flint Remix)"),
    );
    assert_eq!(got.outcome, rework_link(&[MarkerKind::Remix]));
    assert_eq!(codes(&got), ["namesSayRework", "creditedToRemixer"]);
    assert_eq!(
        got.reasons[1],
        Reason::CreditedToRemixer {
            sides: vec![Side::A, Side::B],
        }
    );
}

// ---- The same title can be different songs, even as remixes ------------

#[test]
fn the_same_title_can_be_different_songs_under_different_artists_nothing_links_them() {
    let got = decided(
        nothing_shared(),
        &by("Kestrel Nine", "Korvex"),
        &by("The Marrow Choir", "Korvex"),
    );
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::SameTitleDifferentArtists]);
}

#[test]
fn the_same_title_can_be_different_songs_even_as_remixes_nothing_links_them() {
    let got = decided(
        nothing_shared(),
        &by("Kestrel Nine", "Korvex (Rivetta Remix)"),
        &by("The Marrow Choir", "Korvex (Flint Bootleg)"),
    );
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::SameTitleDifferentArtists]);
}

#[test]
fn the_same_title_can_be_different_songs_shared_audio_sends_the_pair_to_review() {
    let got = decided(
        partial(),
        &by("Kestrel Nine", "Korvex (Rivetta Remix)"),
        &by("The Marrow Choir", "Korvex (Flint Bootleg)"),
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

// ---- Mashup part names can match unrelated tracks ----------------------

#[test]
fn mashup_part_names_can_match_unrelated_tracks_without_shared_audio_nothing_links_them() {
    let (mashup, namesake) = (titled("Duskline x Emberlow (Mashup)"), titled("Duskline"));
    for audio in [nothing_shared(), Audio::NotCompared, Audio::Unknown] {
        let got = decided(audio, &mashup, &namesake);
        assert_eq!(got.outcome, Outcome::Different);
        assert_eq!(codes(&got), ["mashupPart"]);
    }
    // With shared audio, the part is a source the mashup contains.
    let got = decided(partial(), &mashup, &namesake);
    assert_eq!(got.outcome, rework_link(&[MarkerKind::Mashup]));
    assert_eq!(codes(&got), ["mashupContains"]);
}

// ---- Identical tags can hide a different recording ---------------------

#[test]
fn identical_tags_can_hide_a_different_recording_the_fingerprint_vetoes_a_name_based_merge() {
    let (a, b) = (
        by("Kestrel Nine", "Paper Harbor"),
        by("Kestrel Nine", "Paper Harbor"),
    );
    let got = decided(nothing_shared(), &a, &b);
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(got.reasons, [Reason::FingerprintVeto]);
}

#[test]
fn identical_tags_can_hide_a_different_recording_a_pair_nobody_compared_goes_to_review() {
    // A weak partial match is often never proposed for comparison: no
    // stored comparison is no finding, so it can't keep the pair apart.
    let (a, b) = (
        by("Kestrel Nine", "Paper Harbor"),
        by("Kestrel Nine", "Paper Harbor"),
    );
    let got = decided(Audio::NotCompared, &a, &b);
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(got.reasons, [Reason::NotCompared]);
}

#[test]
fn identical_tags_can_hide_a_different_recording_a_partial_match_goes_to_review() {
    let (a, b) = (
        by("Kestrel Nine", "Paper Harbor"),
        by("Kestrel Nine", "Paper Harbor"),
    );
    let got = decided(partial(), &a, &b);
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
}

#[test]
fn identical_tags_can_hide_a_different_recording_identical_tags_alone_never_merge() {
    let (a, b) = (
        by("Kestrel Nine", "Paper Harbor"),
        by("Kestrel Nine", "Paper Harbor"),
    );
    for (what, audio) in every_audio() {
        let merges = matches!(what, "identical" | "full");
        let got = decided(audio, &a, &b);
        assert_eq!(got.outcome == Outcome::Duplicate, merges, "{what}");
    }
}

// ---- Strip store and rip junk ------------------------------------------

#[test]
fn store_and_rip_junk_is_stripped_a_junk_laden_file_name_counts_as_the_same_name() {
    let junky = named(
        "Downloads/10000001_Halden_Rook_-_Orbit_Line_&#40;Original Mix&#41; [ example-ripper ] (320kbps) - RipSite.example.mp3",
    );
    let clean = by("Halden Rook", "Orbit Line");
    // A duplicate with nothing said about the names: they don't differ.
    let same_audio = decided(full(), &junky, &clean);
    assert_eq!(same_audio.outcome, Outcome::Duplicate);
    assert_eq!(codes(&same_audio), ["sameAudio"]);
    // And the "identical tags" rule applies to them as to any same names.
    let part = decided(partial(), &junky, &clean);
    assert_eq!(part.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&part), ["sameTagsPartialAudio"]);
}

#[test]
fn store_and_rip_junk_is_stripped_before_the_version_check_so_the_veto_still_holds() {
    let got = decided(
        full(),
        &named("10000002_Halden_Rook_-_Orbit_Line_&#40;Clean&#41; [ example-ripper ].mp3"),
        &named("RipSite.example - Halden Rook - Orbit Line (Dirty).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(codes(&got), ["versionVeto"]);
}

// ---- Naming conventions vary across pools and bootlegs -----------------

#[test]
fn naming_conventions_vary_across_pools_and_bootlegs_a_handle_edit_and_a_bracketed_flip_are_rework_links(
) {
    let cases: [(&str, MarkerKind, &str); 2] = [
        (
            "Glasswing (@lumenlark edit)",
            MarkerKind::Bootleg,
            "@lumenlark",
        ),
        ("Glasswing [Vey Sun Flip]", MarkerKind::Flip, "Vey Sun"),
    ];
    for (title, kind, whose) in cases {
        let got = decided(nothing_shared(), &titled(title), &titled("Glasswing"));
        assert_eq!(got.outcome, rework_link(&[kind]), "{title}");
        assert_eq!(
            got.reasons,
            [Reason::NamesSayRework {
                only_a: vec![Rework {
                    kind,
                    whose: Some(whose.to_string()),
                }],
                only_b: vec![],
            }],
            "{title}"
        );
    }
}

#[test]
fn naming_conventions_vary_across_pools_and_bootlegs_two_spellings_of_one_cut_are_one_version() {
    // One pool writes "(Extended Mix)", another " - Extended".
    let got = decided(
        full(),
        &titled("Glasswing (Extended Mix)"),
        &titled("Glasswing - Extended"),
    );
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}
