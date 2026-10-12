//! Each threshold, on both sides: 90% coverage, a difference score of 4,
//! and the least audio that counts as shared.

use super::*;

/// The largest share under 90%.
fn just_under_90() -> f32 {
    f32::from_bits(DUPLICATE_COVERAGE.to_bits() - 1)
}

/// The smallest score over 4.
fn just_over_4() -> f64 {
    f64::from_bits(DUPLICATE_SCORE.to_bits() + 1)
}

fn same_names(measured: Measured) -> Decision {
    decided(
        Audio::Compared(measured),
        &titled("Paper Harbor"),
        &titled("Paper Harbor"),
    )
}

#[test]
fn the_thresholds_are_the_roadmaps_90_percent_and_score_4() {
    assert_eq!(DUPLICATE_COVERAGE, 0.9);
    assert_eq!(DUPLICATE_SCORE, 4.0);
}

// ---- The duplicate rule ------------------------------------------------

#[test]
fn exactly_90_percent_both_ways_at_exactly_score_4_is_a_duplicate() {
    let got = same_names(measured(0.9, 0.9, 4.0));
    assert_eq!(got.outcome, Outcome::Duplicate);
    assert_eq!(codes(&got), ["sameAudio"]);
}

#[test]
fn just_under_90_percent_of_the_first_file_is_not_a_duplicate() {
    let got = same_names(measured(just_under_90(), 1.0, 0.0));
    // The second file sits inside the first: one-sided, so a cut link.
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(
        got.reasons,
        [Reason::AudioInsideOther {
            inner: Side::B,
            found_a: just_under_90(),
            found_b: 1.0,
        }]
    );
}

#[test]
fn just_under_90_percent_of_the_second_file_is_not_a_duplicate() {
    let got = same_names(measured(1.0, just_under_90(), 0.0));
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(
        got.reasons,
        [Reason::AudioInsideOther {
            inner: Side::A,
            found_a: 1.0,
            found_b: just_under_90(),
        }]
    );
}

#[test]
fn just_under_90_percent_of_both_files_is_a_partial_match_not_a_duplicate() {
    let got = same_names(measured(just_under_90(), just_under_90(), 0.0));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
}

#[test]
fn a_score_just_over_4_is_not_a_duplicate_whatever_the_coverage() {
    let got = same_names(measured(1.0, 1.0, just_over_4()));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
}

#[test]
fn the_duplicate_rule_is_judged_on_the_main_alignment_not_on_what_other_alignments_found() {
    // Both files are found in full, but the first needs a second
    // alignment to be: something was taken out of it.
    let got = same_names(Measured {
        coverage_a: 0.6,
        coverage_b: 0.95,
        score: 1.0,
        found_a: 0.95,
        found_b: 0.95,
        found_score: 1.0,
        matched_items: 2000,
        main_starts: None,
    });
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
}

#[test]
fn the_duplicate_rule_is_judged_on_the_main_alignments_score_not_the_score_over_everything_found() {
    let got = same_names(Measured {
        coverage_a: 0.95,
        coverage_b: 0.95,
        score: 4.0,
        found_a: 1.0,
        found_b: 1.0,
        found_score: 6.0,
        matched_items: 2000,
        main_starts: None,
    });
    assert_eq!(got.outcome, Outcome::Duplicate);
}

// ---- One file inside the other -----------------------------------------

#[test]
fn exactly_90_percent_of_one_file_found_in_the_other_at_exactly_score_4_is_one_sided() {
    let got = same_names(measured(0.9, 0.5, 4.0));
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(
        got.reasons,
        [Reason::AudioInsideOther {
            inner: Side::A,
            found_a: 0.9,
            found_b: 0.5,
        }]
    );
}

#[test]
fn just_under_90_percent_of_one_file_found_in_the_other_is_a_partial_match() {
    for found in [
        measured(just_under_90(), 0.5, 1.0),
        measured(0.5, just_under_90(), 1.0),
    ] {
        let got = same_names(found);
        assert_eq!(got.outcome, Outcome::NeedsReview);
        assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
    }
}

#[test]
fn one_sided_coverage_counts_what_every_alignment_found_not_only_the_main_one() {
    // An edit with a part taken out: the main alignment covers half of
    // it, a second one the rest.
    let got = same_names(Measured {
        coverage_a: 0.5,
        coverage_b: 0.2,
        score: 1.0,
        found_a: 0.95,
        found_b: 0.4,
        found_score: 1.0,
        matched_items: 2000,
        main_starts: None,
    });
    assert_eq!(got.outcome, cut_link(&[]));
    assert_eq!(codes(&got), ["audioInsideOther"]);
}

#[test]
fn one_sided_coverage_with_a_score_just_over_4_over_everything_found_is_a_partial_match() {
    let got = same_names(Measured {
        coverage_a: 0.95,
        coverage_b: 0.4,
        score: 1.0,
        found_a: 0.95,
        found_b: 0.4,
        found_score: just_over_4(),
        matched_items: 2000,
        main_starts: None,
    });
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
}

#[test]
fn one_sided_coverage_is_judged_on_the_score_over_everything_found_not_the_main_alignments() {
    let got = same_names(Measured {
        coverage_a: 0.5,
        coverage_b: 0.2,
        score: just_over_4(),
        found_a: 0.95,
        found_b: 0.4,
        found_score: 4.0,
        matched_items: 2000,
        main_starts: None,
    });
    assert_eq!(got.outcome, cut_link(&[]));
}

#[test]
fn both_files_found_in_full_without_the_duplicate_rule_holding_is_not_one_sided() {
    // 90% of each is found in the other, but the score is over 4: neither
    // a duplicate nor one file inside the other.
    let got = same_names(measured(0.9, 0.9, just_over_4()));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
}

// ---- The least audio that counts as shared -----------------------------

fn a_short_stretch(matched_items: usize) -> Measured {
    // A sliver of each file, or none of it when no item matched.
    let share = match matched_items {
        0 => 0.0,
        _ => 0.001,
    };
    Measured {
        coverage_a: share,
        coverage_b: share,
        score: 2.0,
        found_a: share,
        found_b: share,
        found_score: 2.0,
        matched_items,
        main_starts: None,
    }
}

#[test]
fn the_floor_for_shared_audio_is_the_least_matching_ever_reports() {
    // One item: the main alignment keeps any stretch that isn't empty.
    assert_eq!(MIN_SHARED_ITEMS, 1);
}

#[test]
fn a_matched_stretch_at_the_floor_is_a_partial_match() {
    let got = same_names(a_short_stretch(MIN_SHARED_ITEMS));
    assert_eq!(got.outcome, Outcome::NeedsReview);
    assert_eq!(codes(&got), ["sameTagsPartialAudio"]);
}

#[test]
fn a_matched_stretch_under_the_floor_is_no_shared_audio() {
    let got = same_names(a_short_stretch(MIN_SHARED_ITEMS - 1));
    assert_eq!(got.outcome, Outcome::Different);
    assert_eq!(codes(&got), ["fingerprintVeto"]);
}
