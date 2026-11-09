//! "What's next" (3.4) with "sounds similar" (3.8): the audio model's opinion
//! as a signal. The scoring on its own, then the weight's default and the
//! queries against a migrated database with made-up fingerprints. Made-up
//! tracks only.

use std::collections::HashMap;

use super::score::{
    rank, Ask, Candidate, ExtraPart, ExtraSignal, Facts, Ranking, Reason, ReasonCode, Vocab,
};
use super::*;
use crate::audio_model::ledger::{self, Outcome};
use crate::audio_model::similar::{likeness_of, percent_of, Neighbours, Resemblance};
use crate::audio_model::subset::SUBSET_LEN;
use crate::audio_model::EMBEDDING_LEN;
use crate::crates::fit::Camelot;
use crate::search::tests::support::Lib;
use crate::settings::{write_feature_enabled, Feature};

// ---- the scoring ---------------------------------------------------------------

const NO_WEIGHTS: Weights = Weights {
    key: 0,
    bpm: 0,
    energy: 0,
    played: 0,
    genre: 0,
    tags: 0,
    similar: 0,
};

/// Only the key and sounds similar count.
const KEY_AND_SOUND: Weights = Weights {
    key: 4,
    similar: 1,
    ..NO_WEIGHTS
};

fn on_8a() -> Facts {
    Facts {
        key: Camelot::parse("8A"),
        ..Facts::default()
    }
}

fn in_key(id: i64, key: &str) -> Candidate {
    Candidate {
        id,
        facts: Facts {
            key: Camelot::parse(key),
            ..Facts::default()
        },
    }
}

fn run(weights: Weights, resemblance: &Resemblance, candidates: &[Candidate]) -> Ranking {
    run_with(weights, resemblance, &[], candidates)
}

fn run_with(
    weights: Weights,
    resemblance: &Resemblance,
    extras: &[&dyn ExtraSignal],
    candidates: &[Candidate],
) -> Ranking {
    let settings = Settings {
        weights,
        ..Settings::default()
    };
    let ask = Ask {
        target_id: 0,
        target: on_8a(),
        direction: Direction::Same,
        settings: &settings,
        played_after: &HashMap::new(),
        extras,
        vocab: &Vocab::default(),
        resemblance,
    };
    rank(&ask, candidates, SHOWN)
}

fn cosines(pairs: &[(i64, f32)]) -> Resemblance {
    Resemblance::To(pairs.iter().copied().collect())
}

fn order(r: &Ranking) -> Vec<i64> {
    r.results.iter().map(|s| s.id).collect()
}

fn codes(r: &Ranking, id: i64) -> Vec<ReasonCode> {
    r.results
        .iter()
        .find(|s| s.id == id)
        .expect("ranked")
        .reasons
        .iter()
        .map(|reason| reason.code)
        .collect()
}

fn score(r: &Ranking, id: i64) -> u8 {
    r.results.iter().find(|s| s.id == id).expect("ranked").score
}

#[test]
fn tracks_that_sound_alike_rank_above_ones_that_do_not_with_all_else_equal() {
    // The same key everywhere: only the sound tells them apart. The lower id
    // would win a tie, so the order below is the sound's doing.
    let all = [in_key(1, "8A"), in_key(2, "8A"), in_key(3, "8A")];
    let heard = cosines(&[(1, 0.2), (2, 0.9), (3, 0.5)]);
    let r = run(KEY_AND_SOUND, &heard, &all);
    assert_eq!(order(&r), [2, 3, 1]);
    assert!(score(&r, 2) > score(&r, 3) && score(&r, 3) > score(&r, 1));
    assert!(codes(&r, 2).contains(&ReasonCode::Similar));
    // Without the signal the tie goes to the lower id.
    let r = run(NO_WEIGHTS_KEY, &heard, &all);
    assert_eq!(order(&r), [1, 2, 3]);
}

/// The key alone.
const NO_WEIGHTS_KEY: Weights = Weights {
    key: 4,
    ..NO_WEIGHTS
};

#[test]
fn a_sounds_similar_weight_of_zero_removes_it_and_says_nothing() {
    let all = [in_key(1, "8A"), in_key(2, "8A")];
    let heard = cosines(&[(1, 0.1), (2, 0.9)]);
    let off = Weights {
        similar: 0,
        ..KEY_AND_SOUND
    };
    let r = run(off, &heard, &all);
    assert_eq!(order(&r), [1, 2]);
    assert!(r
        .results
        .iter()
        .all(|s| !s.reasons.iter().any(|x| x.code == ReasonCode::Similar)));
    assert!(r.ignored.is_empty());
    // A weight of 0 is off, not unused: not reported even if the model is off.
    assert!(run(off, &Resemblance::ModelOff, &all).ignored.is_empty());
    assert!(run(off, &Resemblance::NoFingerprint, &all)
        .ignored
        .is_empty());
}

#[test]
fn the_reason_says_the_percent_and_a_negative_cosine_counts_for_nothing() {
    let all = [in_key(1, "8A"), in_key(2, "8A")];
    let r = run(KEY_AND_SOUND, &cosines(&[(1, 0.824), (2, -0.4)]), &all);
    let similar = |id| {
        r.results
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .reasons
            .iter()
            .find(|x| x.code == ReasonCode::Similar)
            .cloned()
    };
    let reason = similar(1).expect("a reason");
    assert_eq!(reason.percent, Some(82));
    assert!(reason.good);
    // Opposite sound: a part that counts 0, which has nothing to say.
    assert_eq!(similar(2), None);
    assert!(score(&r, 1) > score(&r, 2));
    assert_eq!(order(&r), [1, 2]);
}

#[test]
fn whats_next_and_the_panel_show_the_same_percent_for_cosines_at_every_rounding_boundary() {
    // The f32 cosines on and around every x.5 boundary: widening to f64 first
    // used to round some of them the other way (a percent off by one).
    let mut checked = 0;
    let mut differed = 0;
    for step in 0..100_i32 {
        let boundary = (step as f32 + 0.5) / 100.0;
        for ulps in -4_i32..=4 {
            let cosine = f32::from_bits(boundary.to_bits().wrapping_add_signed(ulps));
            let all = [in_key(1, "8A")];
            let r = run(KEY_AND_SOUND, &cosines(&[(1, cosine)]), &all);
            let shown = r.results[0]
                .reasons
                .iter()
                .find(|x| x.code == ReasonCode::Similar)
                .and_then(|x| x.percent);
            let panel = percent_of(cosine);
            // A cosine under half a percent has a reason that says 0%.
            assert_eq!(shown.unwrap_or(0), panel, "cosine {cosine}");
            // What the f64 rounding used to say.
            let before = (f64::from(likeness_of(cosine)) * 100.0).round() as u8;
            differed += usize::from(before != panel);
            checked += 1;
        }
    }
    assert!(
        differed > 0,
        "the scan reaches a cosine the two roundings split on"
    );
    assert_eq!(checked, 900);
}

#[test]
fn a_candidate_whose_only_reason_would_be_the_sound_gets_no_similarity_part() {
    // 1 is in a key far from 8A (a fact that counts 0) but sounds just like
    // the chosen track; 2 is the same key and sounds nothing like it.
    let all = [in_key(1, "3B"), in_key(2, "8A")];
    let heard = cosines(&[(1, 0.99), (2, 0.05)]);
    let with = run(KEY_AND_SOUND, &heard, &all);
    let without = run(
        Weights {
            similar: 0,
            ..KEY_AND_SOUND
        },
        &heard,
        &all,
    );
    // 1 is scored and shown exactly as if there were no sound signal at all.
    assert_eq!(score(&with, 1), 0);
    assert_eq!(score(&with, 1), score(&without, 1));
    assert_eq!(codes(&with, 1), codes(&without, 1));
    assert!(!codes(&with, 1).contains(&ReasonCode::Similar));
    // And it is not lifted above the candidate that has a fact going for it.
    assert_eq!(order(&with), [2, 1]);
    // The one with a fact does get the sound.
    assert!(codes(&with, 2).contains(&ReasonCode::Similar));
}

#[test]
fn a_candidate_with_no_fact_at_all_going_for_it_is_never_suggested_for_its_sound() {
    // No key, no BPM, no energy, nothing played: nothing but the sound.
    let bare = Candidate {
        id: 1,
        facts: Facts::default(),
    };
    let weights = Weights {
        key: 4,
        bpm: 4,
        energy: 2,
        played: 2,
        similar: 5,
        ..NO_WEIGHTS
    };
    let r = run(weights, &cosines(&[(1, 1.0)]), &[bare]);
    assert_eq!(score(&r, 1), 0);
    assert!(!codes(&r, 1).contains(&ReasonCode::Similar));
}

struct Loud(i64);

impl ExtraSignal for Loud {
    fn part(&self, _target: i64, candidate: i64) -> Option<ExtraPart> {
        (candidate == 1).then_some(ExtraPart {
            weight: 10,
            fitness: 1.0,
            reason: Reason {
                code: ReasonCode::PlayedAfter,
                good: true,
                from: None,
                to: None,
                delta: None,
                step: None,
                count: Some(self.0 as u32),
                tags: None,
                percent: None,
            },
        })
    }
}

#[test]
fn the_sound_is_never_shown_without_a_fact_reason_beside_it() {
    // Two loud extra signals fill the top of the list, and the sound
    // contributes more than the key does: the key must still be beside it.
    let weights = Weights {
        key: 1,
        similar: 1,
        ..NO_WEIGHTS
    };
    let (a, b) = (Loud(2), Loud(3));
    let r = run_with(
        weights,
        &cosines(&[(1, 0.95)]),
        &[&a, &b],
        &[in_key(1, "8A")],
    );
    let shown = codes(&r, 1);
    assert_eq!(shown.len(), 3);
    assert!(shown.contains(&ReasonCode::KeySame), "{shown:?}");
}

#[test]
fn the_sound_is_not_used_while_the_model_is_off_or_the_track_has_no_fingerprint() {
    let all = [in_key(1, "8A"), in_key(2, "8A")];
    let baseline = run(NO_WEIGHTS_KEY, &Resemblance::NotWeighed, &all);
    for missing in [Resemblance::ModelOff, Resemblance::NoFingerprint] {
        let r = run(KEY_AND_SOUND, &missing, &all);
        assert_eq!(r.ignored, [Signal::Similar]);
        // It counts for nothing: the same scores, order and reasons.
        assert_eq!(r.results, baseline.results);
    }
    assert!(run(KEY_AND_SOUND, &cosines(&[(2, 0.5)]), &all)
        .ignored
        .is_empty());
}

#[test]
fn a_candidate_without_a_fingerprint_counts_as_dissimilar_and_never_gains_from_it() {
    // Same facts throughout. 1 sounds alike, 2 has no fingerprint (it is not
    // in the map), 3 is fingerprinted and sounds nothing like it.
    let all = [in_key(1, "8A"), in_key(2, "8A"), in_key(3, "8A")];
    let r = run(KEY_AND_SOUND, &cosines(&[(1, 0.9), (3, -0.3)]), &all);
    assert!(r.ignored.is_empty());
    // It says nothing about the sound, and is scored as a 0 on it.
    assert_eq!(codes(&r, 2), [ReasonCode::KeySame]);
    assert!(score(&r, 1) > score(&r, 2));
    assert!(score(&r, 2) < 100);
    // Not ahead of the dissimilar one for the missing data alone.
    assert_eq!(score(&r, 2), score(&r, 3));
    // And not above it in the order: the lower id wins a tie, so give the
    // unfingerprinted one the higher id.
    let all = [in_key(1, "8A"), in_key(2, "8A")];
    let r = run(KEY_AND_SOUND, &cosines(&[(1, -0.3)]), &all);
    assert_eq!(order(&r), [1, 2]);
    assert_eq!(score(&r, 1), score(&r, 2));
}

// ---- the weight's default ---------------------------------------------------------

fn call<T: Send + 'static>(
    lib: &Lib,
    job: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
) -> T {
    lib.writer.call(move |c| job(c)).unwrap()
}

fn turn(lib: &Lib, on: bool) {
    call(lib, move |c| {
        write_feature_enabled(c, Feature::AudioModel, on)
    });
}

fn similar_weight(lib: &Lib) -> u8 {
    call(lib, read_settings).weights.similar
}

fn stored_similar(lib: &Lib) -> Option<i64> {
    call(lib, |c| {
        c.query_row(
            "SELECT value ->> '$.similar' FROM setting WHERE key = 'next_weights'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map(Option::flatten)
    })
}

#[test]
fn the_sounds_similar_weight_defaults_to_one_only_while_the_audio_model_is_on() {
    let lib = Lib::new();
    assert_eq!(similar_weight(&lib), 0);
    turn(&lib, true);
    assert_eq!(similar_weight(&lib), 1);
    assert_eq!(similar_weight(&lib), Weights::default().similar);
    turn(&lib, false);
    assert_eq!(similar_weight(&lib), 0);
}

#[test]
fn saving_another_weight_does_not_pin_sounds_similar_at_the_default_of_the_moment() {
    let lib = Lib::new();
    // The model is off, so the screen shows 0; the user changes the key.
    let shown = call(&lib, read_settings);
    let changed = Settings {
        weights: Weights {
            key: 9,
            ..shown.weights
        },
        ..shown
    };
    call(&lib, move |c| write_settings(c, changed));
    assert_eq!(stored_similar(&lib), None);
    // Turning the model on later still brings it in at 1, with the key kept.
    turn(&lib, true);
    let now = call(&lib, read_settings);
    assert_eq!((now.weights.key, now.weights.similar), (9, 1));
}

#[test]
fn a_sounds_similar_weight_the_user_set_is_kept_whether_the_model_is_on_or_off() {
    let lib = Lib::new();
    turn(&lib, true);
    // On: the user turns it off (0) and later raises another weight.
    let set = |lib: &Lib, similar: u8, key: u8| {
        let shown = call(lib, read_settings);
        let wanted = Settings {
            weights: Weights {
                key,
                similar,
                ..shown.weights
            },
            ..shown
        };
        call(lib, move |c| write_settings(c, wanted));
    };
    set(&lib, 0, 4);
    assert_eq!(stored_similar(&lib), Some(0));
    turn(&lib, false);
    set(&lib, 0, 5);
    turn(&lib, true);
    assert_eq!(similar_weight(&lib), 0);
    // Above the most is cut, like the others.
    set(&lib, 200, 5);
    assert_eq!(similar_weight(&lib), MOST_WEIGHT);
}

// ---- the queries -------------------------------------------------------------------

/// A Library track whose file is hashed, shown in the track table (key 8A,
/// at `bpm`), and, if `fingerprint` is given, listened to by the model.
fn library_track(lib: &Lib, name: &str, bpm: f64, fingerprint: Option<&[f32]>) -> i64 {
    let (recording, file) = lib.track(Some(name), None, &format!("{name}.wav"));
    lib.library(recording, file);
    let version = i64::from(crate::hash::DEFINITION);
    lib.insert(
        "UPDATE file SET size = 100, mtime = 7, audio_hash = ?1 WHERE id = ?2",
        (vec![file as u8; 34], file),
    );
    lib.insert(
        "INSERT INTO file_stage (file_id, stage, version, size, mtime, status)
         VALUES (?1, 'hash', ?2, 100, 7, 'done')",
        (file, version),
    );
    lib.insert(
        "INSERT INTO track_row (recording_id, flags, title_key, bpm, key_order)
         VALUES (?1, ?2, 'x', ?3, 16)",
        (recording, flag::IN_LIBRARY, bpm),
    );
    if let Some(head) = fingerprint {
        let mut fingerprint = vec![0.0; EMBEDDING_LEN];
        fingerprint[..head.len()].copy_from_slice(head);
        lib.writer
            .call(move |c| {
                let due = ledger::due(c, file)?.expect("the file is due");
                let outcome = Outcome::Heard {
                    fingerprint,
                    scores: vec![-10.0; SUBSET_LEN],
                };
                assert!(ledger::record(c, &due, &outcome)?);
                Ok(())
            })
            .unwrap();
    }
    recording
}

fn rank_db(lib: &Lib, target: i64, weights: Weights) -> Found {
    call(lib, move |c| {
        write_settings(
            c,
            Settings {
                weights,
                ..Settings::default()
            },
        )?;
        rank_with(c, &Neighbours::default(), target, Direction::Same, &[])
    })
}

/// Five tracks and what each sounds like: the chosen one, one near it, one
/// far from it, one near it at a tempo that does not fit, and one with no
/// fingerprint yet.
struct Library {
    lib: Lib,
    target: i64,
    near: i64,
    far: i64,
    near_but_off_tempo: i64,
    unheard: i64,
}

fn library() -> Library {
    let lib = Lib::new();
    let target = library_track(&lib, "target", 124.0, Some(&[1.0, 0.0, 0.1]));
    // The far one first: it has the lower id, so it would win a tie.
    let far = library_track(&lib, "far", 124.0, Some(&[0.0, 1.0, 0.0]));
    let near = library_track(&lib, "near", 124.0, Some(&[1.0, 0.1, 0.2]));
    let near_but_off_tempo = library_track(&lib, "off", 100.0, Some(&[1.0, 0.0, 0.15]));
    let unheard = library_track(&lib, "unheard", 124.0, None);
    // One more far-off sound, so the average fingerprint is not the target's.
    library_track(&lib, "other", 124.0, Some(&[0.0, 0.0, 1.0]));
    Library {
        lib,
        target,
        near,
        far,
        near_but_off_tempo,
        unheard,
    }
}

const BPM_AND_SOUND: Weights = Weights {
    bpm: 4,
    similar: 1,
    ..NO_WEIGHTS
};

#[test]
fn a_library_ranks_the_track_that_sounds_alike_above_the_one_that_does_not() {
    let l = library();
    turn(&l.lib, true);
    let found = rank_db(&l.lib, l.target, BPM_AND_SOUND);
    let position = |id| found.ranked.iter().position(|s| s.id == id).unwrap();
    assert!(position(l.near) < position(l.far), "{:?}", found.ranked);
    let near = &found.ranked[position(l.near)];
    assert!(near
        .reasons
        .iter()
        .any(|r| r.code == ReasonCode::Similar && r.percent.is_some_and(|p| p > 50)));
    assert!(found.ignored.is_empty());
    assert_eq!(found.similar_gap, None);
}

#[test]
fn a_library_track_whose_only_reason_would_be_its_sound_is_not_suggested_for_it() {
    let l = library();
    turn(&l.lib, true);
    let found = rank_db(&l.lib, l.target, BPM_AND_SOUND);
    let off = found
        .ranked
        .iter()
        .find(|s| s.id == l.near_but_off_tempo)
        .expect("ranked, as any track is");
    // It sounds alike, but its tempo does not fit: nothing else to say, so
    // no sound part and no score from it.
    assert_eq!(off.score, 0);
    assert!(off.reasons.iter().all(|r| r.code != ReasonCode::Similar));
    // The same as with the signal off.
    let without = rank_db(
        &l.lib,
        l.target,
        Weights {
            similar: 0,
            ..BPM_AND_SOUND
        },
    );
    let was = without
        .ranked
        .iter()
        .find(|s| s.id == l.near_but_off_tempo)
        .unwrap();
    assert_eq!((off.score, &off.reasons), (was.score, &was.reasons));
}

#[test]
fn a_library_track_without_a_fingerprint_counts_as_dissimilar_not_better() {
    let l = library();
    turn(&l.lib, true);
    let found = rank_db(&l.lib, l.target, BPM_AND_SOUND);
    let unheard = found.ranked.iter().find(|s| s.id == l.unheard).unwrap();
    assert!(unheard
        .reasons
        .iter()
        .all(|r| r.code != ReasonCode::Similar));
    // Counted as 0 on the sound, like the dissimilar track: no gain.
    assert!(unheard.score < 100);
    let far = found.ranked.iter().find(|s| s.id == l.far).unwrap();
    assert_eq!(unheard.score, far.score);
    let near = found.ranked.iter().find(|s| s.id == l.near).unwrap();
    assert!(near.score > unheard.score);
}

#[test]
fn with_the_model_off_a_set_weight_is_reported_not_used_and_changes_nothing() {
    let l = library();
    // Off by default.
    let with = rank_db(&l.lib, l.target, BPM_AND_SOUND);
    assert_eq!(with.ignored, [Signal::Similar]);
    assert_eq!(with.similar_gap, Some(SimilarGap::ModelOff));
    let without = rank_db(
        &l.lib,
        l.target,
        Weights {
            similar: 0,
            ..BPM_AND_SOUND
        },
    );
    assert_eq!(with.ranked, without.ranked);
    assert!(without.ignored.is_empty());
    assert_eq!(without.similar_gap, None);
}

#[test]
fn a_chosen_track_with_no_fingerprint_leaves_sounds_similar_unused() {
    let l = library();
    turn(&l.lib, true);
    let found = rank_db(&l.lib, l.unheard, BPM_AND_SOUND);
    assert_eq!(found.ignored, [Signal::Similar]);
    assert_eq!(found.similar_gap, Some(SimilarGap::NoFingerprint));
    assert!(found
        .ranked
        .iter()
        .all(|s| s.reasons.iter().all(|r| r.code != ReasonCode::Similar)));
}

#[test]
fn at_the_default_settings_a_listened_to_library_uses_the_sound_and_the_reply_says_so() {
    let l = library();
    turn(&l.lib, true);
    let found = call(&l.lib, {
        let target = l.target;
        move |c| rank_with(c, &Neighbours::default(), target, Direction::Same, &[])
    });
    assert_eq!(found.settings.weights.similar, 1);
    assert!(found
        .ranked
        .iter()
        .any(|s| s.reasons.iter().any(|r| r.code == ReasonCode::Similar)));
}
