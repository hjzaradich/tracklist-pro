//! Tests for the fit function and the order it gives. Pure: tables of
//! made-up keys and tempos.

use super::*;

fn key(text: &str) -> Camelot {
    Camelot::parse(text).unwrap_or_else(|| panic!("{text} is a key"))
}

fn facts(bpm: Option<f64>, k: Option<&str>) -> Facts {
    Facts {
        bpm,
        key: k.map(key),
    }
}

fn relation(target: &str, track: &str) -> KeyFit {
    key_fit(Some(key(target)), Some(key(track))).1
}

#[test]
fn a_key_is_read_from_its_name_and_from_its_place_on_the_wheel() {
    for (text, number, major) in [
        ("1A", 1, false),
        ("8B", 8, true),
        ("08A", 8, false),
        ("12b", 12, true),
        (" 5a ", 5, false),
    ] {
        assert_eq!(
            Camelot::parse(text),
            Some(Camelot { number, major }),
            "{text}"
        );
    }
    for text in [
        "", "A", "0A", "13A", "8", "8C", "+8A", "-8A", "8 A", "1m", "A minor",
    ] {
        assert_eq!(Camelot::parse(text), None, "{text:?}");
    }
    // The table stores 1A as 2, 1B as 3, up to 12B as 25.
    for (order, name) in [
        (2, "1A"),
        (3, "1B"),
        (16, "8A"),
        (17, "8B"),
        (24, "12A"),
        (25, "12B"),
    ] {
        assert_eq!(
            Camelot::from_order(order).map(Camelot::name).as_deref(),
            Some(name)
        );
    }
    for order in [-1, 0, 1, 26, 100] {
        assert_eq!(Camelot::from_order(order), None, "{order}");
    }
}

#[test]
fn keys_fit_when_they_are_the_same_or_neighbors_on_the_wheel_with_12_next_to_1() {
    // (selected track's key, the other's, what the relation is)
    let neighbor = |from: &str, to: &str| KeyFit::Neighbor {
        from: from.to_owned(),
        to: to.to_owned(),
    };
    let far = |from: &str, to: &str| KeyFit::Far {
        from: from.to_owned(),
        to: to.to_owned(),
    };
    let same = |k: &str| KeyFit::Same { key: k.to_owned() };
    for (target, track, expected) in [
        ("8A", "8A", same("8A")),
        ("8B", "8B", same("8B")),
        // One up or down, the same letter.
        ("8A", "9A", neighbor("8A", "9A")),
        ("8A", "7A", neighbor("8A", "7A")),
        ("8B", "9B", neighbor("8B", "9B")),
        // The same number, the other letter, both ways.
        ("8A", "8B", neighbor("8A", "8B")),
        ("8B", "8A", neighbor("8B", "8A")),
        // The wheel wraps, both ways, both letters.
        ("12A", "1A", neighbor("12A", "1A")),
        ("1A", "12A", neighbor("1A", "12A")),
        ("12B", "1B", neighbor("12B", "1B")),
        ("1B", "12B", neighbor("1B", "12B")),
        // Not neighbors: two steps, the other letter one step over, across
        // the wheel, and the ends of the wheel with the letter switched.
        ("8A", "10A", far("8A", "10A")),
        ("8A", "9B", far("8A", "9B")),
        ("8A", "7B", far("8A", "7B")),
        ("8A", "2A", far("8A", "2A")),
        ("12A", "2A", far("12A", "2A")),
        ("12A", "1B", far("12A", "1B")),
        ("1A", "12B", far("1A", "12B")),
    ] {
        assert_eq!(relation(target, track), expected, "{target} then {track}");
    }
}

#[test]
fn every_key_has_exactly_three_neighbors_and_the_relation_goes_both_ways() {
    let wheel: Vec<Camelot> = (2..=25).map(|o| Camelot::from_order(o).unwrap()).collect();
    assert_eq!(wheel.len(), 24);
    for &a in &wheel {
        let near: Vec<String> = wheel
            .iter()
            .filter(|&&b| a.neighbors(b))
            .map(|b| b.name())
            .collect();
        assert_eq!(near.len(), 3, "{} has {near:?}", a.name());
        for &b in &wheel {
            assert_eq!(
                a.neighbors(b),
                b.neighbors(a),
                "{} and {}",
                a.name(),
                b.name()
            );
        }
        assert!(!a.neighbors(a));
    }
}

#[test]
fn a_missing_key_is_said_plainly_on_whichever_side_it_is_missing() {
    for (target, track, expected) in [
        (Some("8A"), None, KeyFit::NoKey),
        (None, Some("8A"), KeyFit::NoTargetKey),
        // The selected track's missing key is what is said when both are.
        (None, None, KeyFit::NoTargetKey),
    ] {
        let (points, said) = key_fit(target.map(key), track.map(key));
        assert_eq!(said, expected, "{target:?} then {track:?}");
        assert_eq!(points, 0);
    }
}

#[test]
fn tempo_points_step_down_at_2_4_and_8_bpm_either_side() {
    for (track, points) in [
        (120.0, 3),
        (122.0, 3),
        (118.0, 3),
        (122.1, 2),
        (124.0, 2),
        (116.0, 2),
        (124.1, 1),
        (128.0, 1),
        (112.0, 1),
        (128.1, 0),
        (111.9, 0),
    ] {
        let (got, spread, _) = bpm_fit(Some(120.0), Some(track));
        assert_eq!(got, points, "{track} after 120");
        assert!((spread.unwrap() - (track - 120.0).abs()).abs() < 1e-9);
    }
}

#[test]
fn the_tempo_difference_is_signed_track_minus_selected_track() {
    let delta = |track: f64| match bpm_fit(Some(120.0), Some(track)).2 {
        BpmFit::Delta { delta, time } => (delta, time),
        other => panic!("{other:?}"),
    };
    assert_eq!(delta(121.5), (1.5, TimeFeel::Straight));
    assert_eq!(delta(118.0), (-2.0, TimeFeel::Straight));
    assert_eq!(delta(120.0), (0.0, TimeFeel::Straight));
}

#[test]
fn half_and_double_time_count_and_are_said_so() {
    let read = |target: f64, track: f64| match bpm_fit(Some(target), Some(track)) {
        (points, _, BpmFit::Delta { delta, time }) => (points, delta, time),
        other => panic!("{other:?}"),
    };
    // 70 follows 140 at half time, and 140 follows 70 at double time.
    assert_eq!(read(140.0, 70.0), (3, 0.0, TimeFeel::Half));
    assert_eq!(read(70.0, 140.0), (3, 0.0, TimeFeel::Double));
    assert_eq!(read(140.0, 71.5), (3, 1.5, TimeFeel::Half));
    assert_eq!(read(70.0, 138.0), (3, -2.0, TimeFeel::Double));
    // The plain reading wins when it is as close (or closer).
    assert_eq!(read(128.0, 128.0), (3, 0.0, TimeFeel::Straight));
    assert_eq!(read(128.0, 130.0), (3, 2.0, TimeFeel::Straight));
    // Too far either way is no points, with the nearest reading named.
    assert_eq!(read(128.0, 100.0), (0, -28.0, TimeFeel::Straight));
}

#[test]
fn a_missing_bpm_is_said_plainly_on_whichever_side_it_is_missing() {
    assert_eq!(bpm_fit(Some(120.0), None), (0, None, BpmFit::NoBpm));
    assert_eq!(bpm_fit(None, Some(120.0)), (0, None, BpmFit::NoTargetBpm));
    assert_eq!(bpm_fit(None, None), (0, None, BpmFit::NoTargetBpm));
}

#[test]
fn points_add_the_key_and_the_tempo_and_the_dots_follow_the_points() {
    let target = facts(Some(124.0), Some("8A"));
    // (the other track, its points, its dots)
    for (track, points, dots) in [
        // Same key, same tempo: the most there is.
        (facts(Some(124.0), Some("8A")), 6, 3),
        (facts(Some(126.0), Some("8A")), 6, 3),
        // A neighbor key and a close tempo.
        (facts(Some(125.5), Some("9A")), 5, 3),
        (facts(Some(124.0), Some("8B")), 5, 3),
        // Same key, tempo within 4 / within 8.
        (facts(Some(127.0), Some("8A")), 5, 3),
        (facts(Some(131.0), Some("8A")), 4, 2),
        // A neighbor key within 4 BPM.
        (facts(Some(127.0), Some("7A")), 4, 2),
        // A neighbor key, tempo within 8; same key and no tempo to compare.
        (facts(Some(131.0), Some("7A")), 3, 1),
        (facts(None, Some("8A")), 3, 1),
        // A neighbor key at any tempo; another key at a close tempo.
        (facts(Some(140.0), Some("9A")), 2, 1),
        (facts(Some(124.0), Some("3B")), 3, 1),
        (facts(Some(127.0), Some("3B")), 2, 1),
        // Another key within 8 BPM, or nothing to compare.
        (facts(Some(131.0), Some("3B")), 1, 0),
        (facts(None, None), 0, 0),
        (facts(Some(150.0), Some("3B")), 0, 0),
    ] {
        let got = fit(target, track);
        assert_eq!((got.points, got.dots()), (points, dots), "{track:?}");
    }
}

fn candidate(id: i64, bpm: Option<f64>, k: Option<&str>) -> Candidate {
    Candidate {
        id,
        facts: facts(bpm, k),
    }
}

fn ids(arranged: &[(i64, RowFit)]) -> Vec<i64> {
    arranged.iter().map(|(id, _)| *id).collect()
}

#[test]
fn the_selected_track_stays_first_and_the_rest_follow_best_fit_first() {
    let tracks = [
        candidate(1, Some(140.0), Some("3B")),
        candidate(2, Some(124.0), Some("8A")),
        candidate(3, Some(124.5), Some("9A")),
        candidate(4, Some(124.0), Some("8A")),
        candidate(5, Some(130.0), Some("8A")),
    ];
    // Track 3 is selected: 9A at 124.5.
    let arranged = arrange(3, tracks[2].facts, &tracks);
    assert_eq!(ids(&arranged), [3, 2, 4, 5, 1]);
    // The selected track has no reason; the others do.
    assert_eq!(arranged[0].1.reason, None);
    assert!(arranged[1..].iter().all(|(_, fit)| fit.reason.is_some()));
}

#[test]
fn equal_fits_keep_the_order_the_tracks_came_in() {
    // Four identical tracks, then the selected one, in the middle.
    let same = |id| candidate(id, Some(124.0), Some("8A"));
    for order in [[10, 11, 12, 13], [13, 12, 11, 10], [11, 13, 10, 12]] {
        let mut tracks: Vec<Candidate> = order.iter().map(|&id| same(id)).collect();
        tracks.insert(2, candidate(99, Some(124.0), Some("8A")));
        let arranged = arrange(99, tracks[2].facts, &tracks);
        let mut expected = vec![99];
        expected.extend(order);
        assert_eq!(ids(&arranged), expected, "{order:?}");
    }
}

#[test]
fn among_equal_points_the_closer_tempo_comes_first_and_tracks_with_no_tempo_last() {
    let tracks = [
        candidate(1, Some(100.0), Some("8A")), // selected
        candidate(2, None, Some("8A")),        // same key, no tempo: 3 points
        candidate(3, Some(108.0), Some("8A")), // 3 + 1: 4 points
        candidate(4, Some(104.0), Some("8A")), // 3 + 2: 5 points
        candidate(5, Some(100.0), Some("3B")), // 3 points, spread 0
        candidate(6, Some(107.0), Some("3B")), // 1 point
        candidate(7, Some(101.0), Some("9A")), // 2 + 3: 5 points, spread 1
    ];
    let arranged = arrange(1, tracks[0].facts, &tracks);
    // 7 and 4 both have 5 points; 7's tempo is closer. 3 has 4. 5 has 3 points
    // at no spread, so it is before 2, which has 3 points and no tempo.
    assert_eq!(ids(&arranged), [1, 7, 4, 3, 5, 2, 6]);
}

#[test]
fn tracks_with_a_missing_key_or_tempo_are_ordered_and_say_what_is_missing() {
    let tracks = [
        candidate(1, Some(120.0), Some("8A")), // selected
        candidate(2, None, None),
        candidate(3, Some(121.0), None),
        candidate(4, None, Some("8A")),
    ];
    let arranged = arrange(1, tracks[0].facts, &tracks);
    // 4: same key (3 points); 3: tempo within 2 (3 points, but a spread of 1
    // beats none); 2: nothing.
    assert_eq!(ids(&arranged), [1, 3, 4, 2]);
    let reason = |id: i64| {
        arranged
            .iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, fit)| fit.reason.clone())
            .unwrap()
    };
    assert_eq!(reason(2).key, KeyFit::NoKey);
    assert_eq!(reason(2).bpm, BpmFit::NoBpm);
    assert_eq!(reason(3).key, KeyFit::NoKey);
    assert_eq!(reason(4).bpm, BpmFit::NoBpm);
}

#[test]
fn a_selected_track_with_no_key_or_tempo_leaves_the_rest_in_their_own_order_saying_why() {
    let tracks = [
        candidate(1, None, None), // selected
        candidate(2, Some(120.0), Some("8A")),
        candidate(3, Some(90.0), Some("1B")),
    ];
    let arranged = arrange(1, tracks[0].facts, &tracks);
    assert_eq!(ids(&arranged), [1, 2, 3]);
    for (_, fit) in &arranged[1..] {
        let reason = fit.reason.as_ref().unwrap();
        assert_eq!(reason.key, KeyFit::NoTargetKey);
        assert_eq!(reason.bpm, BpmFit::NoTargetBpm);
        assert_eq!(fit.dots, 0);
    }
}

#[test]
fn the_selected_track_is_not_invented_when_it_is_not_among_the_tracks() {
    let tracks = [
        candidate(2, Some(130.0), Some("8A")),
        candidate(3, Some(124.0), Some("8A")),
    ];
    let arranged = arrange(1, facts(Some(124.0), Some("8A")), &tracks);
    assert_eq!(ids(&arranged), [3, 2]);
}

#[test]
fn the_reason_names_both_keys_and_the_signed_tempo_difference() {
    let tracks = [
        candidate(1, Some(124.0), Some("8A")),
        candidate(2, Some(125.5), Some("9A")),
    ];
    let arranged = arrange(1, tracks[0].facts, &tracks);
    assert_eq!(
        arranged[1].1,
        RowFit {
            dots: 3,
            reason: Some(FitReason {
                key: KeyFit::Neighbor {
                    from: "8A".to_owned(),
                    to: "9A".to_owned()
                },
                bpm: BpmFit::Delta {
                    delta: 1.5,
                    time: TimeFeel::Straight
                },
            }),
        }
    );
}
