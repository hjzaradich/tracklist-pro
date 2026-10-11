//! Measurements are checked when they are made: numbers that can't be
//! measurements are refused, never judged.

use super::*;
use crate::matching::{Comparison, Segment};

/// `Measured::new` with every value sound except what `change` alters.
fn with(change: impl FnOnce(&mut [f64; 6])) -> Result<Measured, BadMeasurement> {
    // coverage_a, coverage_b, score, found_a, found_b, found_score
    let mut values = [0.95, 0.9, 1.5, 0.97, 0.92, 1.75];
    change(&mut values);
    let [coverage_a, coverage_b, score, found_a, found_b, found_score] = values;
    Measured::new(
        coverage_a as f32,
        coverage_b as f32,
        score,
        found_a as f32,
        found_b as f32,
        found_score,
        2000,
    )
}

const SHARES: [(usize, &str); 4] = [
    (0, "coverage_a"),
    (1, "coverage_b"),
    (3, "found_a"),
    (4, "found_b"),
];

const SCORES: [(usize, &str); 2] = [(2, "score"), (5, "found_score")];

#[test]
fn sound_measurements_are_taken_as_given() {
    let measured = with(|_| {}).expect("sound measurements");
    assert_eq!(
        measured,
        Measured {
            coverage_a: 0.95,
            coverage_b: 0.9,
            score: 1.5,
            found_a: 0.97,
            found_b: 0.92,
            found_score: 1.75,
            matched_items: 2000,
        }
    );
}

#[test]
fn the_ends_of_each_range_are_measurements() {
    for (at, field) in SHARES {
        assert!(with(|v| v[at] = 0.0).is_ok(), "{field} = 0");
        assert!(with(|v| v[at] = 1.0).is_ok(), "{field} = 1");
    }
    for (at, field) in SCORES {
        assert!(with(|v| v[at] = 0.0).is_ok(), "{field} = 0");
        assert!(with(|v| v[at] = 32.0).is_ok(), "{field} = 32");
    }
}

#[test]
fn a_share_written_as_a_percent_is_refused_not_taken_for_a_full_match() {
    for (at, field) in SHARES {
        assert_eq!(
            with(|v| v[at] = 95.0),
            Err(BadMeasurement::NotAShare(field))
        );
    }
}

#[test]
fn a_share_just_over_1_or_under_0_is_refused() {
    for (at, field) in SHARES {
        for bad in [1.5, 1.0001, -0.0001, -1.0] {
            assert_eq!(
                with(|v| v[at] = bad),
                Err(BadMeasurement::NotAShare(field)),
                "{field} = {bad}"
            );
        }
    }
}

#[test]
fn a_share_that_is_not_a_number_is_refused() {
    for (at, field) in SHARES {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                with(|v| v[at] = bad),
                Err(BadMeasurement::NotAShare(field)),
                "{field} = {bad}"
            );
        }
    }
}

#[test]
fn a_score_under_0_over_32_or_not_a_number_is_refused() {
    for (at, field) in SCORES {
        for bad in [-1.0, -0.0001, 32.0001, f64::NAN, f64::INFINITY] {
            assert_eq!(
                with(|v| v[at] = bad),
                Err(BadMeasurement::NotAScore(field)),
                "{field} = {bad}"
            );
        }
    }
}

#[test]
fn shares_with_no_item_matched_are_refused_not_taken_for_a_full_match() {
    assert_eq!(
        Measured::new(1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0),
        Err(BadMeasurement::SharesWithNothingMatched)
    );
    for at in 0..4 {
        let mut shares = [0.0_f32; 4];
        shares[at] = 0.25;
        assert_eq!(
            Measured::new(shares[0], shares[1], 32.0, shares[2], shares[3], 32.0, 0),
            Err(BadMeasurement::SharesWithNothingMatched),
            "share {at}"
        );
    }
    // Nothing matched and no share claimed is a sound "nothing found".
    assert!(Measured::new(0.0, 0.0, 32.0, 0.0, 0.0, 32.0, 0).is_ok());
}

#[test]
fn a_refusal_names_the_field_in_plain_words() {
    assert_eq!(
        BadMeasurement::NotAShare("coverage_a").to_string(),
        "coverage_a is not a share from 0 to 1"
    );
    assert_eq!(
        BadMeasurement::NotAScore("score").to_string(),
        "score is not a score from 0 to 32"
    );
    assert_eq!(
        BadMeasurement::SharesWithNothingMatched.to_string(),
        "no items matched, but a share of a file is matched"
    );
}

#[test]
fn a_stored_comparison_with_numbers_that_cant_be_measurements_is_refused_too() {
    let stretch = Segment {
        offset_a: 0,
        offset_b: 0,
        items: 950,
        score: 1.0,
        alignment: 0,
    };
    let sound = Comparison {
        items_a: 1000,
        items_b: 1000,
        coverage_a: 0.95,
        coverage_b: 0.95,
        score: 1.0,
        segments: vec![stretch],
    };
    assert!(Measured::try_from(&sound).is_ok());
    let percent = Comparison {
        coverage_a: 95.0,
        ..sound.clone()
    };
    assert_eq!(
        Measured::try_from(&percent),
        Err(BadMeasurement::NotAShare("coverage_a"))
    );
    let not_a_number = Comparison {
        score: f64::NAN,
        ..sound.clone()
    };
    assert_eq!(
        Measured::try_from(&not_a_number),
        Err(BadMeasurement::NotAScore("score"))
    );
    let a_bad_stretch = Comparison {
        segments: vec![Segment {
            score: -3.0,
            ..stretch
        }],
        ..sound.clone()
    };
    assert_eq!(
        Measured::try_from(&a_bad_stretch),
        Err(BadMeasurement::NotAScore("found_score"))
    );
    let nothing_matched_yet_covered = Comparison {
        segments: Vec::new(),
        ..sound
    };
    assert_eq!(
        Measured::try_from(&nothing_matched_yet_covered),
        Err(BadMeasurement::SharesWithNothingMatched)
    );
}
