//! "What's next" (3.4): the genre (3.2) and tag (3.1) signals. The scoring on
//! its own, then the weights and the queries against a migrated database.
//! Made-up tracks only.

use std::collections::HashMap;

use super::score::{
    rank, Ask, Candidate, Facts, GenreRef, Ranking, Reason, ReasonCode, Scored, Vocab,
};
use super::*;
use crate::audio_model::similar::Resemblance;
use crate::search::tests::support::Lib;

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

fn settings(weights: Weights) -> Settings {
    Settings {
        weights,
        ..Settings::default()
    }
}

fn genre(node: i64, parent: Option<i64>) -> Option<GenreRef> {
    Some(GenreRef { node, parent })
}

fn with_genre(id: i64, genre: Option<GenreRef>) -> Candidate {
    Candidate {
        id,
        facts: Facts {
            genre,
            ..Facts::default()
        },
    }
}

fn names() -> HashMap<i64, String> {
    [
        (1, "House"),
        (2, "Deep House"),
        (3, "Tech House"),
        (9, "Dub"),
    ]
    .into_iter()
    .map(|(id, name)| (id, name.to_owned()))
    .collect()
}

fn run(target: Facts, vocab: &Vocab, weights: Weights, candidates: &[Candidate]) -> Ranking {
    let settings = settings(weights);
    let ask = Ask {
        target_id: 0,
        target,
        direction: Direction::Same,
        settings: &settings,
        played_after: &HashMap::new(),
        extras: &[],
        vocab,
        resemblance: &Resemblance::NotWeighed,
    };
    rank(&ask, candidates, SHOWN)
}

fn ids(results: &[Scored]) -> Vec<i64> {
    results.iter().map(|s| s.id).collect()
}

fn scored(r: &Ranking, id: i64) -> &Scored {
    r.results.iter().find(|s| s.id == id).unwrap()
}

#[test]
fn the_same_genre_beats_a_shared_parent_beats_nothing() {
    let target = Facts {
        genre: genre(2, Some(1)),
        ..Facts::default()
    };
    let vocab = Vocab {
        genre_names: names(),
        ..Vocab::default()
    };
    let all = [
        with_genre(1, None),
        with_genre(2, genre(9, None)),
        with_genre(3, genre(3, Some(1))),
        with_genre(4, genre(2, Some(1))),
        // The chosen track's parent and a child of it count as beside it.
        with_genre(5, genre(1, None)),
        with_genre(6, genre(7, Some(2))),
    ];
    let weights = Weights {
        genre: 5,
        ..NO_WEIGHTS
    };
    let r = run(target, &vocab, weights, &all);
    assert_eq!(ids(&r.results), [4, 3, 5, 6, 1, 2]);
    assert_eq!(scored(&r, 4).score, 100);
    assert_eq!(scored(&r, 3).score, 50);
    assert_eq!(scored(&r, 5).score, 50);
    assert_eq!(scored(&r, 6).score, 50);
    assert_eq!(scored(&r, 2).score, 0);
    assert!(r.ignored.is_empty());

    let same = &scored(&r, 4).reasons;
    assert_eq!(same.len(), 1);
    assert_eq!(same[0].code, ReasonCode::GenreSame);
    assert_eq!(same[0].from.as_deref(), Some("Deep House"));
    assert!(same[0].good);
    let beside = &scored(&r, 3).reasons;
    assert_eq!(beside[0].code, ReasonCode::GenreRelated);
    assert_eq!(beside[0].from.as_deref(), Some("Deep House"));
    assert_eq!(beside[0].to.as_deref(), Some("Tech House"));
}

#[test]
fn a_candidate_with_no_genre_or_an_unrelated_one_gets_nothing_and_says_nothing() {
    let target = Facts {
        genre: genre(2, Some(1)),
        ..Facts::default()
    };
    let vocab = Vocab {
        genre_names: names(),
        ..Vocab::default()
    };
    let all = [with_genre(1, None), with_genre(2, genre(9, None))];
    let weights = Weights {
        genre: 5,
        ..NO_WEIGHTS
    };
    let r = run(target, &vocab, weights, &all);
    for id in [1, 2] {
        assert_eq!(scored(&r, id).score, 0);
        assert!(scored(&r, id).reasons.is_empty(), "{id}");
    }
}

#[test]
fn a_chosen_track_with_no_genre_leaves_genre_unused() {
    let all = [
        with_genre(1, genre(2, Some(1))),
        with_genre(2, None),
        with_genre(3, genre(9, None)),
    ];
    let vocab = Vocab::default();
    let off = run(Facts::default(), &vocab, NO_WEIGHTS, &all);
    let on = run(
        Facts::default(),
        &vocab,
        Weights {
            genre: 10,
            ..NO_WEIGHTS
        },
        &all,
    );
    // Reported as not used, and the order is what it is without it.
    assert_eq!(on.ignored, [Signal::Genre]);
    assert!(off.ignored.is_empty());
    assert_eq!(ids(&on.results), ids(&off.results));
    assert!(on.results.iter().all(|s| s.reasons.is_empty()));
}

#[test]
fn genre_at_weight_zero_changes_nothing_and_turned_up_it_counts() {
    let target = Facts {
        bpm: Some(124.0),
        genre: genre(2, Some(1)),
        ..Facts::default()
    };
    let near = |id, bpm, g| Candidate {
        id,
        facts: Facts {
            bpm: Some(bpm),
            genre: g,
            ..Facts::default()
        },
    };
    let all = [
        near(1, 128.0, genre(2, Some(1))),
        near(2, 124.0, genre(9, None)),
        near(3, 126.0, None),
    ];
    let bpm_only = Weights {
        bpm: 4,
        ..NO_WEIGHTS
    };
    let r = run(target, &Vocab::default(), bpm_only, &all);
    assert_eq!(ids(&r.results), [2, 3, 1]);
    assert!(r.ignored.is_empty());
    let louder = run(
        target,
        &Vocab::default(),
        Weights {
            genre: 10,
            ..bpm_only
        },
        &all,
    );
    // Turned up, the same genre lifts the farther tempo to the top.
    assert_eq!(louder.results[0].id, 1);
}

fn tagged(names: &[&str], shared: &[(i64, &[usize])]) -> Vocab {
    Vocab {
        target_tags: names.iter().map(|n| (*n).to_owned()).collect(),
        shared_tags: shared.iter().map(|(id, n)| (*id, n.to_vec())).collect(),
        ..Vocab::default()
    }
}

fn plain(id: i64) -> Candidate {
    Candidate {
        id,
        facts: Facts::default(),
    }
}

const TAGS_ONLY: Weights = Weights {
    tags: 5,
    ..NO_WEIGHTS
};

fn texts(tags: &[&str]) -> Option<Vec<String>> {
    Some(tags.iter().map(|t| (*t).to_owned()).collect())
}

#[test]
fn more_shared_tags_rank_higher_up_to_three() {
    let vocab = tagged(
        &["dark", "peak", "vocal", "warm"],
        &[(1, &[1]), (2, &[0, 1]), (3, &[0, 1, 3]), (4, &[0, 1, 2, 3])],
    );
    let all = [plain(1), plain(2), plain(3), plain(4), plain(5)];
    let r = run(Facts::default(), &vocab, TAGS_ONLY, &all);
    // Three or four in common are both a full fit (capped), so the lower id.
    assert_eq!(ids(&r.results), [3, 4, 2, 1, 5]);
    let scores: Vec<u8> = r.results.iter().map(|s| s.score).collect();
    assert_eq!(scores, [100, 100, 67, 33, 0]);
    // The one with none in common says nothing.
    assert!(scored(&r, 5).reasons.is_empty());
}

#[test]
fn the_reason_names_the_shared_tags() {
    let vocab = tagged(&["dark", "peak", "vocal"], &[(1, &[1, 2]), (2, &[0])]);
    let r = run(Facts::default(), &vocab, TAGS_ONLY, &[plain(1), plain(2)]);
    let reason: &Reason = &scored(&r, 1).reasons[0];
    assert_eq!(reason.code, ReasonCode::TagsShared);
    assert_eq!(reason.tags, texts(&["peak", "vocal"]));
    assert_eq!(reason.count, Some(2));
    assert!(reason.good);
    assert_eq!(scored(&r, 2).reasons[0].tags, texts(&["dark"]));
    assert!(!scored(&r, 2).reasons[0].good);
}

#[test]
fn a_chosen_track_with_one_tag_is_fully_matched_by_sharing_it() {
    let vocab = tagged(&["peak"], &[(1, &[0])]);
    let r = run(Facts::default(), &vocab, TAGS_ONLY, &[plain(1), plain(2)]);
    assert_eq!(scored(&r, 1).score, 100);
    assert_eq!(scored(&r, 2).score, 0);
}

#[test]
fn a_chosen_track_with_no_tags_leaves_tags_unused_and_says_so_to_a_user_who_has_tags() {
    let vocab = Vocab {
        any_tags: true,
        ..Vocab::default()
    };
    let r = run(Facts::default(), &vocab, TAGS_ONLY, &[plain(1), plain(2)]);
    assert_eq!(r.ignored, [Signal::Tags]);
    assert!(r.results.iter().all(|s| s.reasons.is_empty()));
}

#[test]
fn a_user_with_no_tags_at_all_is_not_told_tags_are_unused() {
    let r = run(
        Facts::default(),
        &Vocab::default(),
        TAGS_ONLY,
        &[plain(1), plain(2)],
    );
    assert!(r.ignored.is_empty());
    assert!(r.results.iter().all(|s| s.reasons.is_empty()));
}

#[test]
fn tags_decide_between_candidates_that_tie_on_every_other_signal() {
    let same = Facts {
        bpm: Some(124.0),
        key: Camelot::parse("8A"),
        energy: Some(5),
        genre: None,
    };
    let twin = |id| Candidate { id, facts: same };
    let vocab = tagged(&["peak", "vocal"], &[(2, &[0]), (3, &[0, 1])]);
    let all = [twin(1), twin(2), twin(3)];
    // At the default weights; without tags the lower id would come first.
    let r = run(same, &vocab, Weights::default(), &all);
    assert_eq!(ids(&r.results), [3, 2, 1]);
    let r = run(same, &Vocab::default(), Weights::default(), &all);
    assert_eq!(ids(&r.results), [1, 2, 3]);
}

#[test]
fn default_weights_rank_exactly_as_before_for_a_user_without_tags() {
    let defaults = Weights::default();
    assert_eq!((defaults.genre, defaults.tags), (0, 1));
    let target = Facts {
        bpm: Some(124.0),
        key: Camelot::parse("8A"),
        energy: Some(5),
        genre: genre(2, Some(1)),
    };
    let mix = |id, bpm, key: &str, energy, g| Candidate {
        id,
        facts: Facts {
            bpm: Some(bpm),
            key: Camelot::parse(key),
            energy: Some(energy),
            genre: g,
        },
    };
    let all = [
        mix(1, 124.0, "8A", 5, genre(9, None)),
        mix(2, 126.0, "9A", 6, genre(2, Some(1))),
        mix(3, 90.0, "3B", 2, genre(2, Some(1))),
        mix(4, 128.0, "8B", 5, None),
    ];
    let before = Weights {
        genre: 0,
        tags: 0,
        ..defaults
    };
    let a = run(target, &Vocab::default(), defaults, &all);
    let b = run(target, &Vocab::default(), before, &all);
    assert_eq!(a.results, b.results);
}

// ---- the queries -----------------------------------------------------------------

fn index(lib: &Lib, id: i64, genre_node: Option<i64>) {
    lib.writer
        .call(move |c| {
            c.execute(
                "INSERT INTO track_row (recording_id, flags, title_key, genre_node)
                 VALUES (?1, ?2, 'x', ?3)",
                (id, flag::IN_LIBRARY, genre_node),
            )
        })
        .unwrap();
}

fn recording(lib: &Lib) -> i64 {
    lib.insert("INSERT INTO recording DEFAULT VALUES", ())
}

fn call<T: Send + 'static>(
    lib: &Lib,
    job: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
) -> T {
    lib.writer.call(move |c| job(c)).unwrap()
}

fn rank_db(lib: &Lib, target: i64, weights: Weights) -> Found {
    call(lib, move |c| {
        write_settings(c, settings(weights))?;
        rank_for(c, target, Direction::Same, &[])
    })
}

fn ranked_ids(found: &Found) -> Vec<i64> {
    found.ranked.iter().map(|s| s.id).collect()
}

#[test]
fn genre_nodes_are_read_with_their_parents_and_named_in_the_reason() {
    let lib = Lib::new();
    let house = lib.insert(
        "INSERT INTO genre (name, name_key) VALUES ('House', 'house')",
        (),
    );
    let deep = lib.insert(
        "INSERT INTO genre (name, name_key, parent_id) VALUES ('Deep House', 'deep house', ?1)",
        (house,),
    );
    let tech = lib.insert(
        "INSERT INTO genre (name, name_key, parent_id) VALUES ('Tech House', 'tech house', ?1)",
        (house,),
    );
    let dub = lib.insert(
        "INSERT INTO genre (name, name_key) VALUES ('Dub', 'dub')",
        (),
    );
    let (target, same, beside, other, none, stale) = (
        recording(&lib),
        recording(&lib),
        recording(&lib),
        recording(&lib),
        recording(&lib),
        recording(&lib),
    );
    index(&lib, target, Some(deep));
    index(&lib, same, Some(deep));
    index(&lib, beside, Some(tech));
    index(&lib, other, Some(dub));
    index(&lib, none, None);
    // A node that is gone reads as no genre.
    index(&lib, stale, Some(9999));

    let genre_only = Weights {
        genre: 5,
        ..NO_WEIGHTS
    };
    let found = rank_db(&lib, target, genre_only);
    assert_eq!(ranked_ids(&found), [same, beside, other, none, stale]);
    assert_eq!(found.ranked[0].reasons[0].code, ReasonCode::GenreSame);
    assert_eq!(
        found.ranked[0].reasons[0].from.as_deref(),
        Some("Deep House")
    );
    assert_eq!(found.ranked[1].reasons[0].code, ReasonCode::GenreRelated);
    assert_eq!(found.ranked[1].reasons[0].to.as_deref(), Some("Tech House"));
    assert!(found.ranked[2..].iter().all(|s| s.reasons.is_empty()));
    assert!(found.ignored.is_empty());

    // The chosen track without a genre: not used.
    let found = rank_db(&lib, none, genre_only);
    assert_eq!(found.ignored, [Signal::Genre]);
}

fn tag(lib: &Lib, name: &'static str) -> i64 {
    lib.insert("INSERT INTO tag (group_id, name) VALUES (1, ?1)", (name,))
}

fn apply(lib: &Lib, recording: i64, tag: i64) {
    lib.writer
        .call(move |c| {
            c.execute(
                "INSERT INTO tag_applied (recording_id, tag_id) VALUES (?1, ?2)",
                (recording, tag),
            )
        })
        .unwrap();
}

#[test]
fn tags_in_common_are_read_in_one_pass_and_ranked_with_their_names() {
    let lib = Lib::new();
    lib.exec("INSERT INTO tag_group (id, name) VALUES (1, 'Mood')");
    let (peak, vocal, warm) = (tag(&lib, "peak"), tag(&lib, "vocal"), tag(&lib, "warm"));
    let (target, both, one, other_tag, nothing) = (
        recording(&lib),
        recording(&lib),
        recording(&lib),
        recording(&lib),
        recording(&lib),
    );
    for id in [target, both, one, other_tag, nothing] {
        index(&lib, id, None);
    }
    for t in [peak, vocal] {
        apply(&lib, target, t);
        apply(&lib, both, t);
    }
    apply(&lib, one, vocal);
    apply(&lib, one, warm);
    apply(&lib, other_tag, warm);

    let found = rank_db(&lib, target, Weights::default());
    assert_eq!(ranked_ids(&found), [both, one, other_tag, nothing]);
    let first = &found.ranked[0].reasons[0];
    assert_eq!(first.code, ReasonCode::TagsShared);
    assert_eq!(first.tags, texts(&["peak", "vocal"]));
    assert_eq!(found.ranked[1].reasons[0].tags, texts(&["vocal"]));
    assert!(found.ranked[2].reasons.is_empty());
    assert!(!found.ignored.contains(&Signal::Tags));

    // A chosen track with no tags, in a library that has some: not used.
    let found = rank_db(&lib, nothing, Weights::default());
    assert!(found.ignored.contains(&Signal::Tags));
}

#[test]
fn the_default_ranking_for_a_library_with_genres_but_no_tags_is_the_old_one() {
    let lib = Lib::new();
    lib.exec(
        "INSERT INTO genre (id, name, name_key) VALUES (1, 'House', 'house'), (2, 'Dub', 'dub')",
    );
    let ids: Vec<i64> = (0..5).map(|_| recording(&lib)).collect();
    for (n, id) in ids.iter().enumerate() {
        let (id, n) = (*id, n as i64);
        lib.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO track_row (recording_id, flags, title_key, bpm, key_order, genre_node)
                     VALUES (?1, ?2, 'x', ?3, ?4, ?5)",
                    (
                        id,
                        flag::IN_LIBRARY | flag::energy(3 + n as u8),
                        120.0 + 2.0 * n as f64,
                        16 - n,
                        1 + n % 2,
                    ),
                )
            })
            .unwrap();
    }
    let target = ids[0];
    // The audio model is off, where sounds similar defaults to 0.
    let off = Weights {
        similar: 0,
        ..Weights::default()
    };
    let now = rank_db(&lib, target, off);
    let before = rank_db(
        &lib,
        target,
        Weights {
            genre: 0,
            tags: 0,
            ..off
        },
    );
    assert_eq!(now.ranked, before.ranked);
    // A library with no tags at all is not told tags are unused.
    assert_eq!(now.ignored, before.ignored);
    assert_eq!(now.ignored, [Signal::Played]);
}

#[test]
fn the_new_weights_default_read_back_and_older_settings_without_them_still_load() {
    let lib = Lib::new();
    let defaults = call(&lib, read_settings).weights;
    assert_eq!((defaults.genre, defaults.tags), (0, 1));

    // What an earlier version stored has no genre or tags.
    lib.exec(
        r#"INSERT INTO setting (key, value) VALUES
           ('next_weights', json('{"key":1,"bpm":2,"energy":3,"played":4}'))"#,
    );
    let old = call(&lib, read_settings).weights;
    assert_eq!((old.key, old.played), (1, 4));
    assert_eq!((old.genre, old.tags), (0, 1));

    let wanted = Settings {
        weights: Weights {
            genre: 6,
            tags: 0,
            ..old
        },
        ..Settings::default()
    };
    assert_eq!(call(&lib, move |c| write_settings(c, wanted)), wanted);
    assert_eq!(call(&lib, read_settings), wanted);
    let loud = Settings {
        weights: Weights {
            genre: 200,
            tags: 99,
            ..wanted.weights
        },
        ..wanted
    };
    let stored = call(&lib, move |c| write_settings(c, loud));
    assert_eq!(
        (stored.weights.genre, stored.weights.tags),
        (MOST_WEIGHT, MOST_WEIGHT)
    );
}
