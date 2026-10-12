//! A cut-off copy inside the whole, same-named file is a worse duplicate,
//! not a cut (module docs, "A cut-off copy"; TASKS 1bC-6): the one row it
//! changes, and everything it must leave alone.

use super::shape::{every_file, kind, outcome_of};
use super::*;

/// One alignment, starting `in_a` seconds into A and `in_b` into B.
fn starting(coverage_a: f32, coverage_b: f32, in_a: f32, in_b: f32) -> Audio {
    Audio::Compared(Measured {
        main_starts: Some((in_a, in_b)),
        ..measured(coverage_a, coverage_b, 1.5)
    })
}

/// All of A is the beginning of B, which is much longer.
fn a_is_the_start_of_b() -> Audio {
    starting(0.99, 0.41, 0.0, 0.0)
}

/// All of B is the beginning of A, which is much longer.
fn b_is_the_start_of_a() -> Audio {
    starting(0.41, 0.99, 0.0, 0.0)
}

/// The decision for `a` and `b`, each known to be cut short or not, after
/// checking that asking about `b` and `a` gives the same one with the
/// sides swapped.
fn decided_with(audio: Audio, a: &FileFacts, b: &FileFacts, cut_short: (bool, bool)) -> Decision {
    let pair = Pair::new(audio, a, b).truncated(cut_short.0, cut_short.1);
    let decision = decide(&pair);
    assert_eq!(
        decide(&pair.swapped()),
        decision.clone().swapped(),
        "the answer changed with the order of the files"
    );
    decision
}

const A_CUT_SHORT: (bool, bool) = (true, false);
const B_CUT_SHORT: (bool, bool) = (false, true);
const BOTH_CUT_SHORT: (bool, bool) = (true, true);
const NEITHER: (bool, bool) = (false, false);

/// The cut link a file inside a same-named file gets by the audio alone.
fn inside_cut_link(inner: Side, found_a: f32, found_b: f32) -> Decision {
    Decision {
        outcome: cut_link(&[]),
        reasons: vec![Reason::AudioInsideOther {
            inner,
            found_a,
            found_b,
        }],
    }
}

// ---- The row ------------------------------------------------------------

#[test]
fn a_truncated_copy_inside_a_same_named_file_is_a_duplicate() {
    let (cut_off, whole) = (titled("Velo"), titled("Velo"));
    let got = decided_with(a_is_the_start_of_b(), &cut_off, &whole, A_CUT_SHORT);
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(
        got.reasons,
        [Reason::TruncatedCopy {
            inner: Side::A,
            found_a: 0.99,
            found_b: 0.41,
        }]
    );
    // With the artist tags agreeing too, and with only file names.
    let (cut_off, whole) = (by("Artist A", "Velo"), by("Artist A", "Velo"));
    let got = decided_with(a_is_the_start_of_b(), &cut_off, &whole, A_CUT_SHORT);
    assert_eq!(codes(&got), ["truncatedCopy"]);
    let (cut_off, whole) = (named("Artist A - Velo.mp3"), named("Artist A - Velo.flac"));
    let got = decided_with(a_is_the_start_of_b(), &cut_off, &whole, A_CUT_SHORT);
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["truncatedCopy"]);
}

#[test]
fn the_answer_is_the_same_whichever_file_comes_first() {
    let (whole, cut_off) = (titled("Velo"), titled("Velo"));
    let got = decided_with(b_is_the_start_of_a(), &whole, &cut_off, B_CUT_SHORT);
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(
        got.reasons,
        [Reason::TruncatedCopy {
            inner: Side::B,
            found_a: 0.41,
            found_b: 0.99,
        }]
    );
}

#[test]
fn what_a_cut_off_copy_becomes_is_said_in_one_place() {
    // The owner may make this row "needs review": that is `CUT_OFF_COPY`,
    // and nothing else decides it.
    assert_eq!(CUT_OFF_COPY, Outcome::Duplicate);
    let got = decided_with(
        a_is_the_start_of_b(),
        &titled("Velo"),
        &titled("Velo"),
        A_CUT_SHORT,
    );
    assert_eq!(got.outcome, CUT_OFF_COPY);
}

// ---- Whose verdict ------------------------------------------------------

#[test]
fn an_inner_file_with_no_verdict_keeps_its_cut_link() {
    let (a, b) = (titled("Velo"), titled("Velo"));
    let got = decided_with(a_is_the_start_of_b(), &a, &b, NEITHER);
    assert_eq!(got, inside_cut_link(Side::A, 0.99, 0.41));
    // `Pair::new` alone says the same: not known is not truncated.
    assert_eq!(decided(a_is_the_start_of_b(), &a, &b), got);
}

#[test]
fn a_truncated_outer_file_changes_nothing() {
    let (a, b) = (titled("Velo"), titled("Velo"));
    let got = decided_with(a_is_the_start_of_b(), &a, &b, B_CUT_SHORT);
    assert_eq!(got, inside_cut_link(Side::A, 0.99, 0.41));
    let got = decided_with(b_is_the_start_of_a(), &a, &b, A_CUT_SHORT);
    assert_eq!(got, inside_cut_link(Side::B, 0.41, 0.99));
}

#[test]
fn two_truncated_files_one_inside_the_other_are_duplicates() {
    let (a, b) = (titled("Velo"), titled("Velo"));
    let got = decided_with(a_is_the_start_of_b(), &a, &b, BOTH_CUT_SHORT);
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["truncatedCopy"]);
}

// ---- Where the inner file sits -------------------------------------------

#[test]
fn a_truncated_file_that_starts_later_in_the_other_stays_a_cut_link() {
    let (a, b) = (titled("Velo"), titled("Velo"));
    // A radio edit without the intro: its start is 30 s into the long one.
    let got = decided_with(starting(0.99, 0.41, 0.0, 30.0), &a, &b, A_CUT_SHORT);
    assert_eq!(got, inside_cut_link(Side::A, 0.99, 0.41));
    // The long one's start is 30 s into the short one: not its beginning
    // either.
    let got = decided_with(starting(0.99, 0.41, 30.0, 0.0), &a, &b, A_CUT_SHORT);
    assert_eq!(got, inside_cut_link(Side::A, 0.99, 0.41));
}

#[test]
fn the_start_may_be_two_seconds_off_and_no_more() {
    let (a, b) = (titled("Velo"), titled("Velo"));
    for (in_a, in_b) in [(2.0, 0.0), (0.0, 2.0), (2.0, 2.0)] {
        let got = decided_with(starting(0.99, 0.41, in_a, in_b), &a, &b, A_CUT_SHORT);
        assert_eq!(got.outcome, Outcome::Duplicate, "{in_a} / {in_b}");
    }
    for (in_a, in_b) in [(2.01, 0.0), (0.0, 2.01)] {
        let got = decided_with(starting(0.99, 0.41, in_a, in_b), &a, &b, A_CUT_SHORT);
        assert_eq!(got.outcome, cut_link(&[]), "{in_a} / {in_b}");
    }
}

#[test]
fn a_truncated_file_whose_start_in_the_other_is_not_known_stays_a_cut_link() {
    let (a, b) = (titled("Velo"), titled("Velo"));
    // Measurements given by hand say nothing about where the audio sits.
    let by_hand = Measured::new(0.99, 0.41, 1.5, 0.99, 0.41, 1.5, 2000).unwrap();
    assert_eq!(by_hand.main_starts(), None);
    let got = decided_with(Audio::Compared(by_hand), &a, &b, A_CUT_SHORT);
    assert_eq!(got, inside_cut_link(Side::A, 0.99, 0.41));
}

#[test]
fn a_truncated_file_found_in_the_other_in_pieces_stays_a_cut_link() {
    let (a, b) = (titled("Velo"), titled("Velo"));
    // An edit put together from parts of the long version: all of it is
    // found, but the main alignment holds only 60% of it.
    let in_pieces = Measured {
        found_a: 0.99,
        main_starts: Some((0.0, 0.0)),
        ..measured(0.6, 0.25, 1.5)
    };
    let got = decided_with(Audio::Compared(in_pieces), &a, &b, A_CUT_SHORT);
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther"]);
    // Just under 90% in one piece is not enough; 90% is.
    let under = Measured {
        found_a: 0.99,
        main_starts: Some((0.0, 0.0)),
        ..measured(0.899, 0.4, 1.5)
    };
    let got = decided_with(Audio::Compared(under), &a, &b, A_CUT_SHORT);
    assert_eq!(got.outcome, cut_link(&[]));
    let got = decided_with(starting(0.9, 0.4, 0.0, 0.0), &a, &b, A_CUT_SHORT);
    assert_eq!(got.outcome, Outcome::Duplicate);
}

#[test]
fn a_truncated_file_whose_one_piece_matches_too_loosely_stays_a_cut_link() {
    let (a, b) = (titled("Velo"), titled("Velo"));
    // Everything found scores under the line, the main alignment over it.
    let loose = Measured {
        score: 4.5,
        found_score: 3.9,
        main_starts: Some((0.0, 0.0)),
        ..measured(0.99, 0.41, 1.5)
    };
    let got = decided_with(Audio::Compared(loose), &a, &b, A_CUT_SHORT);
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther"]);
}

// ---- No false merge: what the names say stands ----------------------------

/// Asserts that the cut-short `a` inside `b` is decided exactly as without
/// its verdict, and returns that decision.
fn unchanged_by_the_verdict(a: &FileFacts, b: &FileFacts) -> Decision {
    let without = decided_with(a_is_the_start_of_b(), a, b, NEITHER);
    for cut_short in [A_CUT_SHORT, B_CUT_SHORT, BOTH_CUT_SHORT] {
        let with = decided_with(a_is_the_start_of_b(), a, b, cut_short);
        assert_eq!(with, without, "{cut_short:?}");
    }
    assert_ne!(without.outcome, Outcome::Duplicate);
    without
}

#[test]
fn a_truncated_clean_inside_a_dirty_stays_a_cut_link() {
    let got = unchanged_by_the_verdict(&titled("Velo (Clean)"), &titled("Velo (Dirty)"));
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
    assert_eq!(codes(&got), ["audioInsideOther", "namesSayCut"]);
    // Wherever the label is written: only in the file names, under the
    // same title tag.
    let got = unchanged_by_the_verdict(
        &tagged_and_named("Velo", "Velo (Clean).mp3"),
        &tagged_and_named("Velo", "Velo (Dirty).mp3"),
    );
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Clean, MarkerKind::Dirty])
    );
}

#[test]
fn a_truncated_radio_edit_named_as_one_stays_a_cut_link() {
    let got = unchanged_by_the_verdict(&titled("Velo (Radio Edit)"), &titled("Velo"));
    assert_eq!(got.outcome, cut_link(&[MarkerKind::RadioEdit]));
    let got =
        unchanged_by_the_verdict(&titled("Velo (Radio Edit)"), &titled("Velo (Extended Mix)"));
    assert_eq!(
        got.outcome,
        cut_link(&[MarkerKind::Extended, MarkerKind::RadioEdit])
    );
    // The long one names itself, the short one doesn't.
    let got = unchanged_by_the_verdict(&titled("Velo"), &titled("Velo (Extended Mix)"));
    assert_eq!(got.outcome, cut_link(&[MarkerKind::Extended]));
}

#[test]
fn a_truncated_file_with_unclear_names_stays_a_cut_link() {
    // The file's own two names disagree about its version.
    let got = unchanged_by_the_verdict(
        &tagged_and_named("Velo", "Velo Dirty 128.mp3"),
        &tagged_and_named("Velo", "Velo.mp3"),
    );
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther", "namesUnclear"]);
    // A bracket the parser doesn't know.
    let got = unchanged_by_the_verdict(&titled("Velo (Skyline Session)"), &titled("Velo"));
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther", "namesUnclear"]);
}

#[test]
fn a_truncated_file_by_another_artist_stays_a_cut_link() {
    let got = unchanged_by_the_verdict(&by("Artist A", "Velo"), &by("Artist B", "Velo"));
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther", "artistsDiffer"]);
}

#[test]
fn a_truncated_file_credited_to_its_remixer_stays_a_cut_link() {
    let got = unchanged_by_the_verdict(
        &by("Artist B", "Velo (Artist B Remix)"),
        &by("Artist A", "Velo (Artist B Remix)"),
    );
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther", "creditedToRemixer"]);
}

#[test]
fn a_truncated_file_under_another_title_still_needs_review() {
    let got = unchanged_by_the_verdict(&titled("Velo"), &titled("Nightdrive"));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(
        codes(&got),
        ["audioInsideOtherNamesDisagree", "titlesDiffer"]
    );
}

#[test]
fn a_truncated_remix_inside_the_original_still_needs_review() {
    let got = unchanged_by_the_verdict(&titled("Velo (Artist B Remix)"), &titled("Velo"));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got)[0], "audioInsideOtherNamesDisagree");
}

#[test]
fn a_truncated_part_of_a_mashup_stays_a_rework_link() {
    let got = unchanged_by_the_verdict(&titled("Velo"), &titled("Velo x Nightdrive (Mashup)"));
    assert_eq!(got.outcome, rework_link(&[MarkerKind::Mashup]));
    assert_eq!(codes(&got), ["mashupContains"]);
}

// ---- The final guard ------------------------------------------------------

/// Pairs whose names the table reads as the same, with no doubt, and that
/// don't carry the same version words: the final guard's cases.
fn same_to_the_table_with_other_version_words() -> Vec<(FileFacts, FileFacts)> {
    vec![
        (
            tagged_and_named("Velo", "Velo Clean - Artist A.mp3"),
            tagged_and_named("Velo", "Velo Dirty - Artist A.mp3"),
        ),
        (
            tagged_and_named("Velo", "Velo Radio Edit - Artist A.mp3"),
            tagged_and_named("Velo", "Velo Extended Mix - Artist A.mp3"),
        ),
        (
            tagged_and_named("Velo", "Clean - Velo Artist A.mp3"),
            tagged_and_named("Velo", "Dirty - Velo Artist A.mp3"),
        ),
    ]
}

#[test]
fn a_truncated_copy_merge_goes_through_the_final_guard() {
    for (a, b) in same_to_the_table_with_other_version_words() {
        assert_ne!(a.label_words, b.label_words);
        let pair = Pair::new(a_is_the_start_of_b(), &a, &b).truncated(true, false);
        // The table alone would merge them…
        let table = by_table(&pair);
        assert_eq!(table.outcome, Outcome::Duplicate, "{a:?} / {b:?}");
        assert_eq!(codes(&table), ["truncatedCopy"]);
        // …and the guard stands in front of it, as on a full match.
        assert_ne!(decide(&pair).outcome, Outcome::Duplicate, "{a:?} / {b:?}");
        let full_match = decided(full(), &a, &b);
        assert_eq!(codes(&full_match)[0], "labelWordsDiffer");
    }
}

#[test]
fn a_truncated_copy_with_other_version_words_stays_a_cut_link() {
    for (a, b) in same_to_the_table_with_other_version_words() {
        let without = decided_with(a_is_the_start_of_b(), &a, &b, NEITHER);
        assert_eq!(without, inside_cut_link(Side::A, 0.99, 0.41));
        for cut_short in [A_CUT_SHORT, BOTH_CUT_SHORT] {
            let with = decided_with(a_is_the_start_of_b(), &a, &b, cut_short);
            assert_eq!(with, without, "{a:?} / {b:?}");
        }
    }
}

#[test]
fn the_guard_still_sends_a_full_match_to_review_whatever_the_verdicts() {
    for (a, b) in same_to_the_table_with_other_version_words() {
        let without = decided(full(), &a, &b);
        assert_eq!(without.outcome, Outcome::NeedsReview);
        for cut_short in [A_CUT_SHORT, B_CUT_SHORT, BOTH_CUT_SHORT] {
            assert_eq!(decided_with(full(), &a, &b, cut_short), without);
        }
    }
}

// ---- Every other row ------------------------------------------------------

/// Every kind of audio, with one file being the beginning of the other
/// besides.
fn every_audio_and_the_starts() -> Vec<(&'static str, Audio)> {
    let mut audio = every_audio();
    audio.push(("a is the start of b", a_is_the_start_of_b()));
    audio.push(("b is the start of a", b_is_the_start_of_a()));
    audio
}

#[test]
fn truncation_changes_no_other_row_of_the_table() {
    let files = every_file();
    let mut merged = 0;
    for a in &files {
        for b in &files {
            for (what, audio) in every_audio_and_the_starts() {
                let without = decided_with(audio, a, b, NEITHER);
                for cut_short in [A_CUT_SHORT, B_CUT_SHORT, BOTH_CUT_SHORT] {
                    let with = decided_with(audio, a, b, cut_short);
                    if with == without {
                        continue;
                    }
                    // The one row: a cut link made by the audio alone, with
                    // nothing from the names beside it, whose inner file is
                    // the one cut short and the other's beginning.
                    let context = format!("{what}, {cut_short:?}: {a:?} / {b:?}");
                    assert_eq!(without.outcome, cut_link(&[]), "{context}");
                    let [Reason::AudioInsideOther {
                        inner,
                        found_a,
                        found_b,
                    }] = without.reasons[..]
                    else {
                        panic!("{context}: {without:?}");
                    };
                    assert!(what.contains("is the start of"), "{context}");
                    let inner_cut_short = match inner {
                        Side::A => cut_short.0,
                        Side::B => cut_short.1,
                    };
                    assert!(inner_cut_short, "{context}");
                    assert_eq!(a.label_words, b.label_words, "{context}");
                    assert_eq!(with.outcome, Outcome::Duplicate, "{context}");
                    assert_eq!(
                        with.reasons,
                        [Reason::TruncatedCopy {
                            inner,
                            found_a,
                            found_b,
                        }],
                        "{context}"
                    );
                    merged += 1;
                }
            }
        }
    }
    assert!(merged > 0, "the row was never reached");
}

#[test]
fn with_verdicts_every_deciding_reason_still_comes_with_its_one_outcome() {
    let files = every_file();
    let mut reached = false;
    for a in &files {
        for b in &files {
            for (what, audio) in every_audio_and_the_starts() {
                for cut_short in [A_CUT_SHORT, B_CUT_SHORT, BOTH_CUT_SHORT] {
                    let got = decided_with(audio, a, b, cut_short);
                    let deciding = got.reasons[0].code();
                    assert_eq!(outcome_of(deciding), kind(&got.outcome), "{what}: {got:?}");
                    reached |= deciding == "truncatedCopy";
                    // Still nothing merges on names: only identical audio,
                    // a full match, or a cut-off copy.
                    if got.outcome == Outcome::Duplicate {
                        assert!(
                            matches!(deciding, "identicalAudio" | "sameAudio" | "truncatedCopy"),
                            "{what}: {got:?}"
                        );
                    }
                }
            }
        }
    }
    assert!(reached);
}

// ---- Where the audio starts, from a comparison ----------------------------

#[test]
fn a_comparison_says_where_its_main_alignment_starts_in_each_file() {
    use crate::matching::{Comparison, Segment};
    let stretch = |offset_a, offset_b, items, alignment| Segment {
        offset_a,
        offset_b,
        items,
        score: 1.0,
        alignment,
    };
    let comparison = Comparison {
        items_a: 1000,
        items_b: 3000,
        coverage_a: 0.9,
        coverage_b: 0.3,
        score: 1.0,
        // The main alignment in two stretches, the later one listed first,
        // and another alignment that starts earlier than both.
        segments: vec![
            stretch(500, 520, 400, 0),
            stretch(10, 30, 500, 0),
            stretch(0, 2000, 10, 1),
        ],
    };
    let measured = Measured::try_from(&comparison).unwrap();
    let item = matching::item_seconds();
    assert_eq!(measured.main_starts(), Some((10.0 * item, 30.0 * item)));
    // Nothing matched: there is no start.
    let nothing = Comparison {
        coverage_a: 0.0,
        coverage_b: 0.0,
        score: matching::compare::NO_MATCH_SCORE,
        segments: Vec::new(),
        ..comparison
    };
    assert_eq!(Measured::try_from(&nothing).unwrap().main_starts(), None);
}

#[test]
fn a_stored_comparison_of_a_cut_off_copy_and_the_whole_file_makes_them_duplicates() {
    use crate::matching::{Comparison, Segment};
    // 2,000 items of A, all of them the first 2,000 of B's 5,000.
    let comparison = Comparison {
        items_a: 2000,
        items_b: 5000,
        coverage_a: 1.0,
        coverage_b: 0.4,
        score: 0.5,
        segments: vec![Segment {
            offset_a: 0,
            offset_b: 0,
            items: 2000,
            score: 0.5,
            alignment: 0,
        }],
    };
    let audio = Audio::Compared(Measured::try_from(&comparison).unwrap());
    let (a, b) = (titled("Velo"), titled("Velo"));
    let got = decided_with(audio, &a, &b, A_CUT_SHORT);
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["truncatedCopy"]);
    // The same 2,000 items found a minute into B: a cut.
    let later = Comparison {
        segments: vec![Segment {
            offset_b: (60.0 / matching::item_seconds()) as usize,
            ..comparison.segments[0]
        }],
        ..comparison
    };
    let audio = Audio::Compared(Measured::try_from(&later).unwrap());
    let got = decided_with(audio, &a, &b, A_CUT_SHORT);
    assert_eq!(got.outcome, cut_link(&[]));
}

#[test]
fn a_start_that_is_no_number_of_seconds_is_refused() {
    let measured = measured(0.99, 0.41, 1.5);
    for bad in [-0.1, f32::NAN, f32::INFINITY] {
        assert_eq!(
            measured.starting(bad, 0.0),
            Err(BadMeasurement::NotAStart("start in A"))
        );
        assert_eq!(
            measured.starting(0.0, bad),
            Err(BadMeasurement::NotAStart("start in B"))
        );
    }
    let sound = measured.starting(0.0, 12.5).unwrap();
    assert_eq!(sound.main_starts(), Some((0.0, 12.5)));
    assert_eq!(sound.swapped().main_starts(), Some((12.5, 0.0)));
}
