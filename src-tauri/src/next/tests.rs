//! "What's next" (3.4): the scoring on its own, then the settings, the
//! query and the rows against a migrated database. Made-up tracks only.

use std::collections::HashMap;

use super::score::{
    rank, Ask, Candidate, ExtraPart, ExtraSignal, Facts, Reason, ReasonCode, Scored,
};
use super::*;
use crate::search::tests::support::{Lib, Mount};

// ---- the scoring ---------------------------------------------------------------

fn key(text: &str) -> Option<Camelot> {
    Some(Camelot::parse(text).unwrap())
}

fn facts(bpm: Option<f64>, k: Option<&str>, energy: Option<u8>) -> Facts {
    Facts {
        bpm,
        key: k.and_then(Camelot::parse),
        energy,
    }
}

fn cand(id: i64, bpm: Option<f64>, k: Option<&str>, energy: Option<u8>) -> Candidate {
    Candidate {
        id,
        facts: facts(bpm, k, energy),
    }
}

fn run(
    target: Facts,
    direction: Direction,
    settings: Settings,
    played: &HashMap<i64, u32>,
    candidates: &[Candidate],
) -> score::Ranking {
    let ask = Ask {
        target_id: 0,
        target,
        direction,
        settings: &settings,
        played_after: played,
        extras: &[],
    };
    rank(&ask, candidates, SHOWN)
}

fn ids(results: &[Scored]) -> Vec<i64> {
    results.iter().map(|s| s.id).collect()
}

fn codes(s: &Scored) -> Vec<ReasonCode> {
    s.reasons.iter().map(|r| r.code).collect()
}

fn only(weights: Weights) -> Settings {
    Settings {
        weights,
        ..Settings::default()
    }
}

const KEY_ONLY: Weights = Weights {
    key: 5,
    bpm: 0,
    energy: 0,
    played: 0,
};

#[test]
fn keys_rank_same_then_neighbors_and_relative_then_the_rest_with_their_reasons() {
    let target = facts(None, Some("8A"), None);
    let all = [
        cand(1, None, Some("3B"), None),
        cand(2, None, Some("9A"), None),
        cand(3, None, Some("8A"), None),
        cand(4, None, Some("8B"), None),
        cand(5, None, None, None),
        cand(6, None, Some("7A"), None),
    ];
    let r = run(
        target,
        Direction::Same,
        only(KEY_ONLY),
        &HashMap::new(),
        &all,
    );
    // Neighbors tie, so the lower id first; a track without a key is last of all.
    assert_eq!(ids(&r.results), [3, 2, 4, 6, 1, 5]);
    let first = |id| r.results.iter().find(|s| s.id == id).unwrap().reasons[0].code;
    assert_eq!(first(3), ReasonCode::KeySame);
    assert_eq!(first(2), ReasonCode::KeyAdjacent);
    assert_eq!(first(4), ReasonCode::KeyRelative);
    assert_eq!(first(1), ReasonCode::KeyFar);
    assert_eq!(first(5), ReasonCode::KeyNone);
    assert_eq!(r.results[0].score, 100);
}

#[test]
fn the_wheel_wraps_from_12_to_1() {
    let target = facts(None, Some("12B"), None);
    let all = [
        cand(1, None, Some("1B"), None),
        cand(2, None, Some("4B"), None),
    ];
    let r = run(
        target,
        Direction::Same,
        only(KEY_ONLY),
        &HashMap::new(),
        &all,
    );
    assert_eq!(codes(&r.results[0]), [ReasonCode::KeyAdjacent]);
    assert_eq!(codes(&r.results[1]), [ReasonCode::KeyFar]);
}

#[test]
fn half_and_double_time_count_and_the_reason_says_which() {
    let weights = Weights {
        key: 0,
        bpm: 5,
        energy: 0,
        played: 0,
    };
    let target = facts(Some(140.0), None, None);
    let all = [
        cand(1, Some(70.0), None, None),
        cand(2, Some(280.0), None, None),
        cand(3, Some(141.0), None, None),
        cand(4, Some(100.0), None, None),
        cand(5, None, None, None),
    ];
    let r = run(
        target,
        Direction::Same,
        only(weights),
        &HashMap::new(),
        &all,
    );
    assert_eq!(ids(&r.results), [1, 2, 3, 4, 5]);
    assert_eq!(codes(&r.results[0]), [ReasonCode::BpmHalf]);
    assert_eq!(codes(&r.results[1]), [ReasonCode::BpmDouble]);
    assert_eq!(r.results[2].reasons[0].delta, Some(1.0));
    assert!(!r.results[3].reasons[0].good);
    assert_eq!(codes(&r.results[4]), [ReasonCode::BpmNone]);
}

#[test]
fn a_tight_tempo_scores_as_sort_by_fit_does() {
    let weights = Weights {
        key: 0,
        bpm: 5,
        energy: 0,
        played: 0,
    };
    let target = facts(Some(120.0), None, None);
    let all = [
        cand(1, Some(122.0), None, None),
        cand(2, Some(124.0), None, None),
        cand(3, Some(128.0), None, None),
        cand(4, Some(129.0), None, None),
    ];
    let r = run(
        target,
        Direction::Same,
        only(weights),
        &HashMap::new(),
        &all,
    );
    // 3, 2 and 1 points of 3; nine BPM away is none.
    let scores: Vec<u8> = r.results.iter().map(|s| s.score).collect();
    assert_eq!(scores, [100, 67, 33, 0]);
}

#[test]
fn the_energy_step_follows_the_direction() {
    let weights = Weights {
        key: 0,
        bpm: 0,
        energy: 5,
        played: 0,
    };
    let target = facts(None, None, Some(5));
    let all = [
        cand(1, None, None, Some(4)),
        cand(2, None, None, Some(5)),
        cand(3, None, None, Some(6)),
        cand(4, None, None, Some(9)),
    ];
    let go = |d| ids(&run(target, d, only(weights), &HashMap::new(), &all).results);
    assert_eq!(go(Direction::Up), [3, 2, 1, 4]);
    assert_eq!(go(Direction::Same), [2, 1, 3, 4]);
    assert_eq!(go(Direction::Down), [1, 2, 3, 4]);
    let r = run(target, Direction::Up, only(weights), &HashMap::new(), &all);
    assert_eq!(r.results[0].reasons[0].step, Some(1));
}

#[test]
fn a_track_with_no_energy_is_last_and_says_why() {
    let target = facts(Some(120.0), Some("8A"), Some(5));
    let all = [
        // A perfect fit but no energy.
        cand(1, Some(120.0), Some("8A"), None),
        cand(2, Some(160.0), Some("3B"), Some(5)),
    ];
    let r = run(
        target,
        Direction::Same,
        Settings::default(),
        &HashMap::new(),
        &all,
    );
    assert_eq!(ids(&r.results), [2, 1]);
    assert_eq!(r.results[1].reasons[0].code, ReasonCode::EnergyNone);
    assert!(r.results[1].reasons.len() <= score::MOST_REASONS);
    assert!(r.results[1]
        .reasons
        .iter()
        .any(|x| x.code == ReasonCode::KeySame));

    // Not weighing energy: no energy is no reason to rank last.
    let off = only(Weights {
        energy: 0,
        ..Weights::default()
    });
    let r = run(target, Direction::Same, off, &HashMap::new(), &all);
    assert_eq!(ids(&r.results), [1, 2]);
}

#[test]
fn what_the_chosen_track_lacks_is_ignored_and_reported() {
    let target = facts(None, None, None);
    let all = [cand(1, Some(120.0), Some("8A"), Some(5))];
    let r = run(
        target,
        Direction::Same,
        Settings::default(),
        &HashMap::new(),
        &all,
    );
    assert_eq!(
        r.ignored,
        [Signal::Key, Signal::Bpm, Signal::Energy, Signal::Played]
    );
    assert!(r.results[0].reasons.is_empty());

    // A weight of 0 is off, not ignored.
    let off = only(Weights {
        key: 0,
        bpm: 0,
        energy: 0,
        played: 0,
    });
    assert!(run(target, Direction::Same, off, &HashMap::new(), &all)
        .ignored
        .is_empty());
}

#[test]
fn played_transitions_lift_a_track_and_the_reason_counts_them() {
    let target = facts(Some(120.0), Some("8A"), None);
    let all = [
        cand(1, Some(120.0), Some("8A"), None),
        cand(2, Some(128.0), Some("11B"), None),
    ];
    let mut played = HashMap::new();
    played.insert(2, 2);
    let heavy = only(Weights {
        key: 1,
        bpm: 1,
        energy: 0,
        played: 10,
    });
    let r = run(target, Direction::Same, heavy, &played, &all);
    assert_eq!(ids(&r.results), [2, 1]);
    let reason = &r.results[0].reasons[0];
    assert_eq!(
        (reason.code, reason.count),
        (ReasonCode::PlayedAfter, Some(2))
    );
    // Track 1 was never played after it: no played reason.
    assert!(!codes(&r.results[1]).contains(&ReasonCode::PlayedAfter));
}

#[test]
fn wide_counts_keys_two_steps_away_and_bigger_tempo_gaps() {
    let target = facts(Some(120.0), Some("8A"), None);
    let all = [cand(1, Some(130.0), Some("10A"), None)];
    let weights = Weights {
        key: 5,
        bpm: 5,
        energy: 0,
        played: 0,
    };
    let tight = run(
        target,
        Direction::Same,
        only(weights),
        &HashMap::new(),
        &all,
    );
    let wide = run(
        target,
        Direction::Same,
        Settings {
            weights,
            tolerance: Tolerance::Wide,
        },
        &HashMap::new(),
        &all,
    );
    assert_eq!(tight.results[0].score, 0);
    assert!(wide.results[0].score > 30);
    assert!(codes(&wide.results[0]).contains(&ReasonCode::KeyTwoSteps));
}

#[test]
fn only_the_best_few_are_kept_with_at_most_three_reasons_and_never_the_chosen_track() {
    let target = facts(Some(120.0), Some("8A"), Some(5));
    let all: Vec<Candidate> = (0..40)
        .map(|n| cand(n, Some(120.0 + n as f64), Some("8A"), Some(5)))
        .collect();
    let r = run(
        target,
        Direction::Same,
        Settings::default(),
        &HashMap::new(),
        &all,
    );
    assert_eq!(r.results.len(), SHOWN);
    // Id 0 is the chosen track (the run's target id), left out.
    assert_eq!(ids(&r.results)[0], 1);
    assert!(!ids(&r.results).contains(&0));
    assert!(r
        .results
        .iter()
        .all(|s| s.reasons.len() <= score::MOST_REASONS));
}

struct Always;

impl ExtraSignal for Always {
    fn part(&self, _target: i64, candidate: i64) -> Option<ExtraPart> {
        (candidate == 2).then_some(ExtraPart {
            weight: 10,
            fitness: 1.0,
            reason: Reason {
                code: ReasonCode::PlayedAfter,
                good: true,
                from: None,
                to: None,
                delta: None,
                step: None,
                count: Some(9),
            },
        })
    }
}

#[test]
fn an_extra_signal_is_weighed_with_the_rest_and_can_give_a_reason() {
    let target = facts(Some(120.0), Some("8A"), None);
    // Track 1 is the better fit on its own; the extra signal lifts track 2.
    let all = [
        cand(1, Some(124.0), Some("8A"), None),
        cand(2, Some(124.0), Some("9A"), None),
    ];
    let settings = Settings::default();
    let none = HashMap::new();
    let extras: [&dyn ExtraSignal; 1] = [&Always];
    let ask = Ask {
        target_id: 0,
        target,
        direction: Direction::Same,
        settings: &settings,
        played_after: &none,
        extras: &extras,
    };
    let r = rank(&ask, &all, SHOWN);
    assert_eq!(r.results[0].id, 2);
    assert_eq!(r.results[0].reasons[0].count, Some(9));
    // Without it, track 1 wins.
    let plain = run(target, Direction::Same, settings, &none, &all);
    assert_eq!(plain.results[0].id, 1);
}

#[test]
fn facts_that_are_not_sensible_read_as_missing() {
    for bad in [f64::NAN, f64::INFINITY, -3.0, 0.0, 1000.0, 5000.0] {
        assert_eq!(facts_of(Some(bad), None, 0).bpm, None, "{bad}");
    }
    assert_eq!(facts_of(Some(128.5), None, 0).bpm, Some(128.5));
    assert_eq!(facts_of(None, Some(99), 0).key, None);
    assert_eq!(facts_of(None, Some(-1), 0).key, None);
    assert_eq!(facts_of(None, Some(16), 0).key, key("8A"));
    // Four bits can hold 11 to 15, which are not an energy.
    assert_eq!(facts_of(None, None, flag::energy(15)).energy, None);
    assert_eq!(facts_of(None, None, flag::energy(7)).energy, Some(7));
    assert_eq!(facts_of(None, None, 0).energy, None);
}

// ---- the settings --------------------------------------------------------------

fn call<T: Send + 'static>(
    lib: &Lib,
    job: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
) -> T {
    lib.writer.call(move |c| job(c)).unwrap()
}

#[test]
fn settings_default_to_genre_neutral_weights_and_tight_and_read_back_what_was_stored() {
    let lib = Lib::new();
    let defaults = call(&lib, read_settings);
    assert_eq!(defaults, Settings::default());
    assert_eq!(defaults.tolerance, Tolerance::Tight);
    assert!(defaults.weights.key > 0 && defaults.weights.bpm > 0);

    let wanted = Settings {
        weights: Weights {
            key: 7,
            bpm: 0,
            energy: 3,
            played: 10,
        },
        tolerance: Tolerance::Wide,
    };
    let operations_before: i64 = call(&lib, |c| {
        c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0))
    });
    assert_eq!(call(&lib, move |c| write_settings(c, wanted)), wanted);
    assert_eq!(call(&lib, read_settings), wanted);
    // Not an operation: Undo has nothing to take back.
    let operations_after: i64 = call(&lib, |c| {
        c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0))
    });
    assert_eq!(operations_before, operations_after);

    // Over the top is cut, and writing again keeps one row each.
    let loud = Settings {
        weights: Weights {
            key: 200,
            ..wanted.weights
        },
        ..wanted
    };
    assert_eq!(
        call(&lib, move |c| write_settings(c, loud)).weights.key,
        MOST_WEIGHT
    );
    let rows: i64 = call(&lib, |c| {
        c.query_row(
            "SELECT count(*) FROM setting WHERE key LIKE 'next_%'",
            [],
            |r| r.get(0),
        )
    });
    assert_eq!(rows, 2);
}

#[test]
fn a_stored_value_that_cannot_be_read_is_the_default_one_value_at_a_time() {
    let lib = Lib::new();
    lib.exec(
        "INSERT INTO setting (key, value) VALUES
            ('next_weights', '{\"key\": 9, \"bpm\": \"loud\", \"energy\": -4, \"played\": 99.7}'),
            ('next_tolerance', '12')",
    );
    let s = call(&lib, read_settings);
    assert_eq!(s.weights.key, 9);
    assert_eq!(s.weights.bpm, Weights::default().bpm);
    assert_eq!(s.weights.energy, 0);
    assert_eq!(s.weights.played, MOST_WEIGHT);
    assert_eq!(s.tolerance, Tolerance::Tight);

    lib.exec("UPDATE setting SET value = '[1, 2]' WHERE key = 'next_weights'");
    assert_eq!(call(&lib, read_settings).weights, Weights::default());
}

// ---- the query -----------------------------------------------------------------

/// A Library track's index row: BPM, wheel position and flags.
fn index(lib: &Lib, id: i64, bpm: Option<f64>, key_order: Option<i64>, flags: i64) {
    lib.writer
        .call(move |c| {
            c.execute(
                "INSERT INTO track_row (recording_id, flags, title_key, bpm, key_order)
                 VALUES (?1, ?2, 'x', ?3, ?4)",
                (id, flags, bpm, key_order),
            )
        })
        .unwrap();
}

fn recording(lib: &Lib) -> i64 {
    lib.insert("INSERT INTO recording DEFAULT VALUES", ())
}

fn rank_ids(lib: &Lib, target: i64, direction: Direction) -> Vec<(i64, Vec<ReasonCode>)> {
    call(lib, move |c| {
        let found = rank_for(c, target, direction, &[])?;
        Ok(found
            .ranked
            .iter()
            .map(|s| (s.id, s.reasons.iter().map(|r| r.code).collect()))
            .collect())
    })
}

#[test]
fn the_library_is_ranked_from_the_index_and_the_plays_and_a_bad_row_stops_nothing() {
    let lib = Lib::new();
    let lib_flag = flag::IN_LIBRARY;
    let target = recording(&lib);
    let same = recording(&lib);
    let played = recording(&lib);
    let outside = recording(&lib);
    let junk = recording(&lib);
    index(
        &lib,
        target,
        Some(124.0),
        Some(16),
        lib_flag | flag::energy(5),
    );
    index(
        &lib,
        same,
        Some(124.0),
        Some(16),
        lib_flag | flag::energy(5),
    );
    index(
        &lib,
        played,
        Some(150.0),
        Some(5),
        lib_flag | flag::energy(5),
    );
    // Not in the Library: never suggested.
    index(
        &lib,
        outside,
        Some(124.0),
        Some(16),
        flag::HAS_FILE | flag::energy(5),
    );
    // Nonsense in every fact; still ranked, with nothing to say.
    index(
        &lib,
        junk,
        Some(f64::INFINITY),
        Some(9999),
        lib_flag | flag::energy(15),
    );
    // A set that played `played` after `target`, twice.
    lib.exec("INSERT INTO session (source, name, played_on) VALUES ('m3u8', 'a', '2026-10-01'), ('m3u8', 'b', '2026-10-02')");
    for session in [1, 2] {
        for (position, rec) in [(0, target), (1, played)] {
            lib.writer
                .call(move |c| {
                    c.execute(
                        "INSERT INTO play_event (session_id, position, recording_id, raw_path)
                         VALUES (?1, ?2, ?3, 'p')",
                        (session, position, rec),
                    )
                })
                .unwrap();
        }
    }

    let heavy_play = Settings {
        weights: Weights {
            key: 1,
            bpm: 1,
            energy: 0,
            played: 10,
        },
        ..Settings::default()
    };
    call(&lib, move |c| write_settings(c, heavy_play));
    let ranked = rank_ids(&lib, target, Direction::Same);
    assert_eq!(
        ranked.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [played, same, junk]
    );
    assert_eq!(ranked[0].1[0], ReasonCode::PlayedAfter);

    // With defaults, energy is weighed: the junk row has none, so it is last.
    call(&lib, |c| write_settings(c, Settings::default()));
    let ranked = rank_ids(&lib, target, Direction::Same);
    assert_eq!(ranked.last().unwrap().0, junk);
    assert_eq!(ranked.last().unwrap().1[0], ReasonCode::EnergyNone);
}

#[test]
fn a_track_the_index_does_not_know_still_ranks_the_others_with_nothing_to_compare() {
    let lib = Lib::new();
    let a = recording(&lib);
    index(&lib, a, Some(120.0), Some(16), flag::IN_LIBRARY);
    let ranked = rank_ids(&lib, 999_999, Direction::Up);
    assert_eq!(ranked.len(), 1);
    assert!(ranked[0].1.is_empty());
}

#[test]
fn a_hundred_thousand_tracks_are_one_query_and_the_best_ten_are_kept() {
    let lib = Lib::new();
    lib.exec(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000)
         INSERT INTO track_row (recording_id, flags, title_key, bpm, key_order)
         SELECT i, 1 | ((i % 10 + 1) << 8), 'x', 90 + (i % 80), 2 + (i % 24) FROM n",
    );
    let (found, count) = call(&lib, |c| {
        let found = rank_for(c, 50_000, Direction::Up, &[])?;
        let count: i64 = c.query_row("SELECT count(*) FROM track_row", [], |r| r.get(0))?;
        Ok((found, count))
    });
    assert_eq!(count, 100_000);
    // Everyone but the chosen track was weighed, in memory; ten kept.
    assert_eq!(found.considered, 99_999);
    assert_eq!(found.ranked.len(), SHOWN);
    assert!(found.ranked.iter().all(|s| s.id != 50_000));
    assert!(found.ranked.windows(2).all(|w| w[0].score >= w[1].score));
    // Close in tempo and key, and a step up in energy: a high score, short of
    // perfect since the best fit in this fixture isn't exact on all three.
    assert!(found.ranked[0].score >= 80, "{}", found.ranked[0].score);
}

#[test]
fn the_reply_has_each_result_as_the_table_row_with_its_reasons() {
    let lib = Lib::new();
    let (target, _) = lib.track(Some("Chosen"), Some("Kit"), "a.mp3");
    let (next, file) = lib.track(Some("Follower"), Some("Kit"), "b.mp3");
    lib.library(next, file);
    index(&lib, target, Some(120.0), Some(16), flag::IN_LIBRARY);
    index(&lib, next, Some(121.0), Some(16), flag::IN_LIBRARY);
    let reply = call(&lib, move |c| {
        let found = rank_for(c, target, Direction::Same, &[])?;
        reply(c, &Mount, found)
    });
    assert_eq!(reply.results.len(), 1);
    assert_eq!(reply.results[0].row.recording_id, next);
    assert_eq!(reply.results[0].row.title.as_deref(), Some("Follower"));
    assert_eq!(reply.results[0].reasons[0].code, ReasonCode::KeySame);
    assert_eq!(reply.considered, 1);
}

struct Odd {
    weight: u8,
    fitness: f64,
}

impl ExtraSignal for Odd {
    fn part(&self, _target: i64, candidate: i64) -> Option<ExtraPart> {
        (candidate == 2).then(|| ExtraPart {
            weight: self.weight,
            fitness: self.fitness,
            reason: Reason {
                code: ReasonCode::PlayedAfter,
                good: true,
                from: None,
                to: None,
                delta: None,
                step: None,
                count: Some(1),
            },
        })
    }
}

fn with_extra(weight: u8, fitness: f64) -> score::Ranking {
    let target = facts(Some(120.0), Some("8A"), None);
    let all = [
        cand(1, Some(124.0), Some("8A"), None),
        cand(2, Some(124.0), Some("9A"), None),
    ];
    let settings = Settings::default();
    let none = HashMap::new();
    let odd = Odd { weight, fitness };
    let extras: [&dyn ExtraSignal; 1] = [&odd];
    let ask = Ask {
        target_id: 0,
        target,
        direction: Direction::Same,
        settings: &settings,
        played_after: &none,
        extras: &extras,
    };
    rank(&ask, &all, SHOWN)
}

#[test]
fn a_bad_extra_signal_is_not_used_and_its_weight_is_capped() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 2.0, -0.5] {
        let r = with_extra(10, bad);
        assert_eq!(ids(&r.results), vec![1, 2], "{bad}");
        assert!(
            r.results[1].reasons.iter().all(|x| x.count != Some(1)),
            "{bad}"
        );
        let plain = with_extra(0, 1.0);
        assert_eq!(r.results[1].score, plain.results[1].score, "{bad}");
    }
    // 255 counts as 10.
    assert_eq!(
        with_extra(255, 1.0).results[0].score,
        with_extra(10, 1.0).results[0].score
    );
    assert_eq!(
        with_extra(255, 1.0).results[1].score,
        with_extra(10, 1.0).results[1].score
    );
}

#[test]
fn reasons_are_named_only_for_the_results_shown() {
    let target = facts(Some(120.0), Some("8A"), None);
    let all: Vec<Candidate> = (1..=200)
        .map(|n| cand(n, Some(120.0), Some("9A"), None))
        .collect();
    score::NAMED.with(|n| n.set(0));
    let r = run(
        target,
        Direction::Same,
        Settings::default(),
        &HashMap::new(),
        &all,
    );
    assert_eq!(r.results.len(), SHOWN);
    assert_eq!(score::NAMED.with(std::cell::Cell::get), SHOWN);
    assert!(r.results[0].reasons.iter().any(|x| x.from.is_some()));
}
