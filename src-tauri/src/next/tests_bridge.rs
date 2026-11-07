//! "Fill the gap" scoring (3.5) on its own: pure, made-up tracks.

use std::collections::HashSet;

use super::bridge::{bridge, energy_jump, key_fit, BridgeReason, Bridged, Gap};
use super::score::{Candidate, Facts};
use super::{Settings, Tolerance, Weights};
use crate::crates::fit::Camelot;

fn facts(bpm: Option<f64>, key: Option<&str>, energy: Option<u8>) -> Facts {
    Facts {
        bpm,
        key: key.and_then(Camelot::parse),
        energy,
        genre: None,
    }
}

fn cand(id: i64, bpm: Option<f64>, key: Option<&str>, energy: Option<u8>) -> Candidate {
    Candidate {
        id,
        facts: facts(bpm, key, energy),
    }
}

const ALL: Weights = Weights {
    key: 4,
    bpm: 4,
    energy: 2,
    played: 0,
    genre: 0,
    tags: 0,
};

fn only(key: u8, bpm: u8, energy: u8, tolerance: Tolerance) -> Settings {
    Settings {
        weights: Weights {
            key,
            bpm,
            energy,
            ..ALL
        },
        tolerance,
    }
}

fn run(
    before: Option<Facts>,
    after: Option<Facts>,
    target: Option<f64>,
    settings: &Settings,
    exclude: &[i64],
    candidates: &[Candidate],
) -> Vec<Bridged> {
    let exclude: HashSet<i64> = exclude.iter().copied().collect();
    bridge(
        &Gap {
            before,
            after,
            target,
            settings,
            exclude: &exclude,
        },
        candidates,
        10,
    )
}

fn ids(results: &[Bridged]) -> Vec<i64> {
    results.iter().map(|b| b.id).collect()
}

#[test]
fn a_true_key_bridge_ranks_above_a_track_that_fits_one_side_and_says_what_gap_it_fills() {
    let a = Some(facts(None, Some("8A"), None));
    let b = Some(facts(None, Some("10A"), None));
    let all = [
        cand(1, None, Some("8A"), None),  // fits A, clashes with B
        cand(2, None, Some("10A"), None), // fits B, clashes with A
        cand(3, None, Some("9A"), None),  // 8A -> 9A -> 10A
        cand(4, None, Some("3B"), None),  // fits neither
    ];
    let settings = only(5, 0, 0, Tolerance::Tight);
    let r = run(a, b, None, &settings, &[], &all);
    // The one-sided tracks are as weak as their worse leg: no credit, no reason.
    assert_eq!(ids(&r), [3]);
    assert_eq!(r[0].score, 67);
    assert_eq!(
        r[0].reasons,
        [BridgeReason::KeyBridge {
            from: "8A".into(),
            via: "9A".into(),
            to: "10A".into()
        }]
    );
}

#[test]
fn when_both_sides_already_fit_the_key_reason_is_a_fit_not_a_bridge() {
    let a = Some(facts(None, Some("8A"), None));
    let b = Some(facts(None, Some("9A"), None));
    let all = [
        cand(7, None, Some("8A"), None),
        cand(2, None, Some("9A"), None),
    ];
    let r = run(a, b, None, &only(5, 0, 0, Tolerance::Tight), &[], &all);
    assert_eq!(ids(&r), [2, 7]);
    assert_eq!(
        r[0].reasons[0],
        BridgeReason::KeyFits {
            from: Some("8A".into()),
            via: "9A".into(),
            to: Some("9A".into())
        }
    );
}

#[test]
fn tolerance_decides_which_tempo_gaps_count() {
    // 120 and 122: a candidate at 130 is 10 and 8 away.
    let a = Some(facts(Some(120.0), None, None));
    let b = Some(facts(Some(122.0), None, None));
    let all = [
        cand(1, Some(130.0), None, None),
        cand(2, Some(121.0), None, None),
    ];
    let tight = run(a, b, None, &only(0, 5, 0, Tolerance::Tight), &[], &all);
    assert_eq!(ids(&tight), [2]);
    let wide = run(a, b, None, &only(0, 5, 0, Tolerance::Wide), &[], &all);
    assert_eq!(ids(&wide), [2, 1]);
}

#[test]
fn a_tempo_between_two_far_tempos_is_a_bpm_step_and_half_and_double_time_count() {
    let a = Some(facts(Some(100.0), None, None));
    let b = Some(facts(Some(116.0), None, None));
    // 108 is 8 from each side (tight reaches 8), the ends are 16 apart.
    let r = run(
        a,
        b,
        None,
        &only(0, 5, 0, Tolerance::Tight),
        &[],
        &[cand(1, Some(108.0), None, None)],
    );
    assert_eq!(
        r[0].reasons,
        [BridgeReason::BpmStep {
            from: 100.0,
            via: 108.0,
            to: 116.0
        }]
    );
    // 60 and 62 read at double time are 120 and 124: a candidate at 122 fits both.
    let a = Some(facts(Some(60.0), None, None));
    let b = Some(facts(Some(62.0), None, None));
    let r = run(
        a,
        b,
        None,
        &only(0, 5, 0, Tolerance::Tight),
        &[],
        &[
            cand(1, Some(61.0), None, None),
            cand(2, Some(122.0), None, None),
        ],
    );
    assert_eq!(ids(&r), [1, 2]);
}

#[test]
fn energy_aims_at_the_target_when_there_is_one_else_between_the_two_ends() {
    let a = Some(facts(None, None, Some(4)));
    let b = Some(facts(None, None, Some(8)));
    let all = [
        cand(1, None, None, Some(6)),
        cand(2, None, None, Some(9)),
        cand(3, None, None, Some(4)),
    ];
    let settings = only(0, 0, 5, Tolerance::Tight);
    // No target: halfway, 4 -> 6 -> 8.
    let r = run(a, b, None, &settings, &[], &all);
    assert_eq!(r[0].id, 1);
    assert_eq!(
        r[0].reasons,
        [BridgeReason::EnergyStep {
            from: Some(4),
            via: 6,
            to: Some(8)
        }]
    );
    // A target of 9 here: the 9 comes first, and the reason names the target.
    let r = run(a, b, Some(9.0), &settings, &[], &all);
    assert_eq!(r[0].id, 2);
    assert_eq!(
        r[0].reasons,
        [BridgeReason::EnergyTarget {
            via: 9,
            target: 9.0
        }]
    );
}

#[test]
fn a_candidate_without_energy_goes_last_while_energy_counts() {
    let a = Some(facts(None, Some("8A"), Some(5)));
    let b = Some(facts(None, Some("10A"), Some(7)));
    let all = [
        cand(1, None, Some("9A"), None), // a perfect key bridge, no energy
        cand(2, None, Some("9A"), Some(6)),
        cand(3, None, Some("9A"), Some(5)),
    ];
    let r = run(a, b, None, &only(4, 0, 2, Tolerance::Tight), &[], &all);
    assert_eq!(ids(&r), [2, 3, 1]);
}

#[test]
fn tracks_in_the_exclude_set_are_never_offered() {
    let a = Some(facts(None, Some("8A"), None));
    let b = Some(facts(None, Some("10A"), None));
    let all = [
        cand(3, None, Some("9A"), None),
        cand(7, None, Some("9A"), None),
    ];
    let r = run(a, b, None, &only(5, 0, 0, Tolerance::Tight), &[3], &all);
    assert_eq!(ids(&r), [7]);
}

#[test]
fn every_suggestion_has_a_reason_and_a_track_with_nothing_to_say_is_left_out() {
    let a = Some(facts(Some(120.0), Some("8A"), Some(5)));
    let b = Some(facts(Some(124.0), Some("10A"), Some(7)));
    let all = [
        cand(1, Some(122.0), Some("9A"), Some(6)),
        cand(2, None, None, None),                 // nothing known
        cand(3, Some(190.0), Some("3B"), Some(1)), // fits nothing
        cand(4, Some(122.0), Some("9A"), None),
    ];
    let r = run(a, b, None, &Settings::default(), &[], &all);
    assert_eq!(ids(&r), [1, 4]);
    assert!(r.iter().all(|s| !s.reasons.is_empty()));
    assert!(r.iter().all(|s| s.reasons.len() <= 3));
    // The key bridge leads the reasons: the ends are two steps apart.
    assert!(matches!(r[0].reasons[0], BridgeReason::KeyBridge { .. }));
}

#[test]
fn at_the_end_only_the_leg_from_the_last_track_counts_and_with_nothing_to_compare_nothing_is_offered(
) {
    let a = Some(facts(Some(120.0), Some("8A"), None));
    let r = run(
        a,
        None,
        None,
        &Settings::default(),
        &[],
        &[cand(1, Some(121.0), Some("9A"), None)],
    );
    assert_eq!(ids(&r), [1]);
    assert_eq!(
        r[0].reasons[0],
        BridgeReason::KeyFits {
            from: Some("8A".into()),
            via: "9A".into(),
            to: None
        }
    );
    let r = run(
        None,
        None,
        None,
        &Settings::default(),
        &[],
        &[cand(1, Some(121.0), Some("9A"), Some(5))],
    );
    assert!(r.is_empty());
}

#[test]
fn a_hundred_thousand_candidates_are_ranked_in_memory_and_the_best_few_kept() {
    let a = Some(facts(Some(120.0), Some("8A"), Some(5)));
    let b = Some(facts(Some(124.0), Some("10A"), Some(7)));
    let all: Vec<Candidate> = (1..=100_000_i64)
        .map(|i| {
            cand(
                i,
                Some(90.0 + (i % 80) as f64),
                Some(format!("{}{}", i % 12 + 1, if i % 2 == 0 { 'A' } else { 'B' }).as_str()),
                Some((i % 10) as u8 + 1),
            )
        })
        .collect();
    let r = run(a, b, None, &Settings::default(), &[], &all);
    assert_eq!(r.len(), 10);
    assert!(r.windows(2).all(|w| w[0].score >= w[1].score));
    assert!(r[0].score >= 60, "{}", r[0].score);
}

#[test]
fn the_clash_rules_are_what_s_nexts_unknowns_are_not_clashes() {
    let k = |t: &str| Camelot::parse(t);
    assert_eq!(key_fit(k("8A"), k("9A"), Tolerance::Tight), Some(2.0 / 3.0));
    assert_eq!(key_fit(k("8A"), k("10A"), Tolerance::Tight), Some(0.0));
    assert!(key_fit(k("8A"), k("10A"), Tolerance::Wide).unwrap() > 0.0);
    assert_eq!(key_fit(k("8A"), None, Tolerance::Tight), None);
    // An energy step: two is a climb, three is a jump when tight.
    assert!(!energy_jump(2, Tolerance::Tight));
    assert!(energy_jump(3, Tolerance::Tight));
    assert!(energy_jump(-3, Tolerance::Tight));
    assert!(!energy_jump(4, Tolerance::Wide));
    assert!(energy_jump(5, Tolerance::Wide));
}
