//! The "Audio model" panel's backend (5A-A3): the guesses and "Sounds like".
//! Made-up tracks and made-up 192-number fingerprints only.

use super::super::guess::{guesses_for, guesses_in, Guesses, MOST_GENRES};
use super::super::ledger::{self, Outcome};
use super::super::panel::{label_key, PanelState};
use super::super::similar::{sounds_like_of, Neighbours, SoundsLike, LISTED};
use super::super::subset::{self, SUBSET, SUBSET_LEN};
use super::super::EMBEDDING_LEN;
use crate::search::tests::support::{Lib, Mount};
use crate::settings::{write_feature_enabled, Feature};

fn vector(head: &[f32]) -> Vec<f32> {
    let mut v = vec![0.0; EMBEDDING_LEN];
    v[..head.len()].copy_from_slice(head);
    v
}

fn quiet() -> Vec<f32> {
    vec![-10.0; SUBSET_LEN]
}

fn logit(p: f64) -> f32 {
    (p / (1.0 - p)).ln() as f32
}

fn turn(lib: &Lib, on: bool) {
    lib.writer
        .call(move |c| write_feature_enabled(c, Feature::AudioModel, on))
        .unwrap();
}

/// A Library track whose file is hashed and has not been listened to.
fn hashed(lib: &Lib, name: &str) -> (i64, i64) {
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
    (recording, file)
}

/// A Library track that the audio model has listened to (through the
/// ledger, as the job does): the track and its file.
fn listened(lib: &Lib, name: &str, fingerprint: &[f32], scores: &[f32]) -> (i64, i64) {
    let (recording, file) = hashed(lib, name);
    let (fingerprint, scores) = (vector(fingerprint), scores.to_vec());
    lib.writer
        .call(move |c| {
            let due = ledger::due(c, file)?.expect("the file is due");
            let outcome = Outcome::Heard {
                fingerprint,
                scores,
            };
            assert!(ledger::record(c, &due, &outcome)?);
            Ok(())
        })
        .unwrap();
    (recording, file)
}

fn sounds_like(lib: &Lib, cache: &Neighbours, recording: i64) -> SoundsLike {
    let cache = cache.clone();
    lib.writer
        .call(move |c| sounds_like_of(c, &cache, &Mount, recording))
        .unwrap()
}

fn guesses(lib: &Lib, recording: i64) -> Guesses {
    lib.writer.call(move |c| guesses_for(c, recording)).unwrap()
}

fn listed(found: &SoundsLike) -> Vec<i64> {
    found.tracks.iter().map(|t| t.row.recording_id).collect()
}

/// Six tracks whose fingerprints add up to nothing, so taking the mean off
/// changes nothing: two axes, at 0, ~37, 90, 180, ~217 and 270 degrees.
fn six(lib: &Lib) -> Vec<i64> {
    let heads: [[f32; 2]; 6] = [
        [1.0, 0.0],
        [0.8, 0.6],
        [0.0, 1.0],
        [-1.0, 0.0],
        [-0.8, -0.6],
        [0.0, -1.0],
    ];
    heads
        .iter()
        .enumerate()
        .map(|(i, head)| listened(lib, &format!("Synthetic Tune {i}"), head, &quiet()).0)
        .collect()
}

// ---- Sounds like ------------------------------------------------------------------

#[test]
fn sounds_like_lists_the_nearest_first_with_the_cosine_as_a_percent() {
    let lib = Lib::new();
    turn(&lib, true);
    let t = six(&lib);
    let found = sounds_like(&lib, &Neighbours::default(), t[0]);
    assert_eq!(found.state, PanelState::Ready);
    // 0.8, then two at 0 (the lower id first), then -0.8 and -1.
    assert_eq!(listed(&found), vec![t[1], t[2], t[5], t[4], t[3]]);
    let percents: Vec<u8> = found.tracks.iter().map(|t| t.percent).collect();
    assert_eq!(percents, vec![80, 0, 0, 0, 0]);
}

#[test]
fn sounds_like_takes_the_mean_off_before_comparing() {
    // Raw, P is nearer R than Q (cosine 0.89 against 0.6); after the mean
    // (2.7, 2.7) is taken off it is the other way round.
    let lib = Lib::new();
    turn(&lib, true);
    let p = listened(&lib, "Synthetic P", &[3.0, 1.0], &quiet()).0;
    let q = listened(&lib, "Synthetic Q", &[1.0, 3.0], &quiet()).0;
    let r = listened(&lib, "Synthetic R", &[4.0, 4.0], &quiet()).0;
    let found = sounds_like(&lib, &Neighbours::default(), p);
    assert_eq!(listed(&found), vec![q, r]);
}

#[test]
fn sounds_like_leaves_out_the_track_itself_and_its_other_files() {
    let lib = Lib::new();
    turn(&lib, true);
    let t = six(&lib);
    // Another file of the first track, with the very same sound.
    let other = lib.file(t[0], "Synthetic Tune 0 copy.wav", "extra");
    lib.insert(
        "INSERT INTO embedding (file_id, audio_hash, model_id, fingerprint, label_scores)
         SELECT ?1, ?2, model_id, fingerprint, label_scores FROM embedding WHERE file_id = (
             SELECT linked_file_id FROM library_track WHERE recording_id = ?3)",
        (other, vec![0xEE_u8; 34], t[0]),
    );
    lib.insert(
        "UPDATE file SET audio_hash = ?1 WHERE id = ?2",
        (vec![0xEE_u8; 34], other),
    );
    let found = sounds_like(&lib, &Neighbours::default(), t[0]);
    assert!(!listed(&found).contains(&t[0]));
    assert_eq!(listed(&found).len(), 5);
}

#[test]
fn sounds_like_lists_at_most_25() {
    let lib = Lib::new();
    turn(&lib, true);
    let first = listened(&lib, "Synthetic Tune 0", &[1.0, 0.0], &quiet()).0;
    for i in 1..=30 {
        listened(
            &lib,
            &format!("Synthetic Tune {i}"),
            &[1.0, i as f32 / 10.0],
            &quiet(),
        );
    }
    let found = sounds_like(&lib, &Neighbours::default(), first);
    assert_eq!(found.tracks.len(), LISTED);
}

#[test]
fn sounds_like_ignores_a_result_of_another_model_or_of_audio_the_file_no_longer_has() {
    let lib = Lib::new();
    turn(&lib, true);
    let t = six(&lib);
    // Track 3 was heard by another model; track 4's audio has changed since.
    lib.insert(
        "UPDATE embedding SET model_id = 'some-other-model' WHERE file_id = (
             SELECT linked_file_id FROM library_track WHERE recording_id = ?1)",
        (t[3],),
    );
    lib.insert(
        "UPDATE file SET audio_hash = ?1 WHERE id = (
             SELECT linked_file_id FROM library_track WHERE recording_id = ?2)",
        (vec![0xAB_u8; 34], t[4]),
    );
    let found = sounds_like(&lib, &Neighbours::default(), t[0]);
    let mut got = listed(&found);
    got.sort_unstable();
    assert_eq!(got, vec![t[1], t[2], t[5]]);
}

#[test]
fn sounds_like_says_which_state_a_track_is_in() {
    let lib = Lib::new();
    let cache = Neighbours::default();
    let (a, a_file) = hashed(&lib, "Synthetic Tune A");
    let (loose, _) = lib.track(Some("Synthetic Tune B"), None, "b.wav");

    assert_eq!(sounds_like(&lib, &cache, a).state, PanelState::Off);

    turn(&lib, true);
    assert_eq!(
        sounds_like(&lib, &cache, a).state,
        PanelState::NotListenedYet
    );
    assert_eq!(
        sounds_like(&lib, &cache, loose).state,
        PanelState::NotInLibrary
    );
    assert_eq!(
        sounds_like(&lib, &cache, 9999).state,
        PanelState::NotInLibrary
    );

    // Failed: not tried again until its audio changes.
    lib.writer
        .call(move |c| {
            let due = ledger::due(c, a_file)?.expect("due");
            let failed = Outcome::Failed(ledger::Reason::Damaged);
            assert!(ledger::record(c, &due, &failed)?);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        sounds_like(&lib, &cache, a).state,
        PanelState::CouldntBeRead
    );

    // A lone listened-to track: ready, and nothing else to list.
    let (b, _) = listened(&lib, "Synthetic Tune C", &[1.0, 0.0], &quiet());
    let alone = sounds_like(&lib, &cache, b);
    assert_eq!(alone.state, PanelState::Ready);
    assert!(alone.tracks.is_empty());
}

#[test]
fn sounds_like_calls_a_stored_fingerprint_with_a_bad_number_unreadable() {
    let lib = Lib::new();
    turn(&lib, true);
    let (a, _) = listened(&lib, "Synthetic Tune A", &[1.0, 0.0], &quiet());
    lib.insert(
        "UPDATE embedding SET fingerprint = ?1",
        (subset::to_blob(&[f32::NAN; EMBEDDING_LEN]),),
    );
    let found = sounds_like(&lib, &Neighbours::default(), a);
    assert_eq!(found.state, PanelState::CouldntBeRead);
}

#[test]
fn a_new_result_is_found_by_the_next_search_without_anyone_clearing_the_cache() {
    let lib = Lib::new();
    turn(&lib, true);
    let cache = Neighbours::default();
    let a = listened(&lib, "Synthetic Tune A", &[1.0, 0.0], &quiet()).0;
    let b = listened(&lib, "Synthetic Tune B", &[0.0, 1.0], &quiet()).0;
    let first = sounds_like(&lib, &cache, a);
    assert_eq!(listed(&first), vec![b]);
    assert!(cache.built());
    let c = listened(&lib, "Synthetic Tune C", &[0.9, 0.1], &quiet()).0;
    let second = sounds_like(&lib, &cache, a);
    assert_eq!(listed(&second), vec![c, b]);
}

#[test]
fn a_track_that_left_the_library_is_not_listed_even_before_the_cache_is_rebuilt() {
    let lib = Lib::new();
    turn(&lib, true);
    let cache = Neighbours::default();
    let a = listened(&lib, "Synthetic Tune A", &[1.0, 0.0], &quiet()).0;
    let b = listened(&lib, "Synthetic Tune B", &[0.9, 0.1], &quiet()).0;
    let c = listened(&lib, "Synthetic Tune C", &[0.0, 1.0], &quiet()).0;
    assert_eq!(listed(&sounds_like(&lib, &cache, a)), vec![b, c]);
    lib.insert("DELETE FROM library_track WHERE recording_id = ?1", (b,));
    assert_eq!(listed(&sounds_like(&lib, &cache, a)), vec![c]);
}

#[test]
fn turning_the_model_off_frees_the_cache_and_hides_the_list() {
    let lib = Lib::new();
    turn(&lib, true);
    let cache = Neighbours::default();
    let a = listened(&lib, "Synthetic Tune A", &[1.0, 0.0], &quiet()).0;
    listened(&lib, "Synthetic Tune B", &[0.0, 1.0], &quiet());
    sounds_like(&lib, &cache, a);
    assert!(cache.built());
    turn(&lib, false);
    let found = sounds_like(&lib, &cache, a);
    assert_eq!(found.state, PanelState::Off);
    assert!(found.tracks.is_empty());
    assert!(!cache.built());
}

// ---- the guesses ------------------------------------------------------------------

fn index_of(name: &str) -> usize {
    SUBSET.iter().position(|(_, n)| *n == name).unwrap()
}

#[test]
fn the_guesses_are_the_best_three_genres_and_the_best_mood_above_the_threshold() {
    let mut scores = quiet();
    scores[index_of("Techno")] = logit(0.84);
    scores[index_of("House music")] = logit(0.60);
    scores[index_of("Trance music")] = logit(0.45);
    scores[index_of("Dubstep")] = logit(0.35);
    scores[index_of("Disco")] = logit(0.29);
    scores[index_of("Happy music")] = logit(0.40);
    scores[index_of("Sad music")] = logit(0.70);
    let (genres, mood) = guesses_in(&scores);
    let shown: Vec<(&str, u8)> = genres.iter().map(|g| (g.key.as_str(), g.percent)).collect();
    assert_eq!(
        shown,
        vec![("techno", 84), ("houseMusic", 60), ("tranceMusic", 45)]
    );
    assert_eq!(genres.len(), MOST_GENRES);
    let mood = mood.expect("a mood passes");
    assert_eq!((mood.key.as_str(), mood.percent), ("sadMusic", 70));
}

#[test]
fn nothing_below_the_threshold_is_guessed_and_a_bad_score_is_skipped() {
    let mut scores = quiet();
    scores[index_of("Disco")] = logit(0.29);
    scores[index_of("Sad music")] = logit(0.29);
    scores[index_of("Techno")] = f32::NAN;
    scores[index_of("Funk")] = f32::INFINITY;
    let (genres, mood) = guesses_in(&scores);
    assert!(genres.is_empty());
    assert!(mood.is_none());
}

#[test]
fn a_mood_is_never_listed_as_a_genre() {
    let mut scores = quiet();
    scores[index_of("Happy music")] = logit(0.9);
    let (genres, mood) = guesses_in(&scores);
    assert!(genres.is_empty());
    assert_eq!(mood.map(|m| m.key), Some("happyMusic".to_owned()));
}

#[test]
fn guesses_come_from_the_stored_scores_of_the_shown_file() {
    let lib = Lib::new();
    turn(&lib, true);
    let mut scores = quiet();
    scores[index_of("Techno")] = logit(0.84);
    let (a, _) = listened(&lib, "Synthetic Tune A", &[1.0, 0.0], &scores);
    let found = guesses(&lib, a);
    assert_eq!(found.state, PanelState::Ready);
    assert_eq!(found.genres.len(), 1);
    assert_eq!(found.genres[0].key, "techno");
    assert!(found.mood.is_none());
}

#[test]
fn guesses_say_which_state_a_track_is_in() {
    let lib = Lib::new();
    let (a, a_file) = hashed(&lib, "Synthetic Tune A");
    let (loose, _) = lib.track(Some("Synthetic Tune B"), None, "b.wav");
    assert_eq!(guesses(&lib, a).state, PanelState::Off);

    turn(&lib, true);
    assert_eq!(guesses(&lib, a).state, PanelState::NotListenedYet);
    assert_eq!(guesses(&lib, loose).state, PanelState::NotInLibrary);

    // Heard by another model: as if not heard.
    lib.insert(
        "INSERT INTO embedding (file_id, audio_hash, model_id, fingerprint, label_scores)
         SELECT id, audio_hash, 'some-other-model', zeroblob(768), zeroblob(228)
         FROM file WHERE id = ?1",
        (a_file,),
    );
    assert_eq!(guesses(&lib, a).state, PanelState::NotListenedYet);

    // The right model but scores of a different length: unreadable.
    lib.insert(
        "UPDATE embedding SET model_id = ?1, label_scores = zeroblob(16) WHERE file_id = ?2",
        (subset::model_id(), a_file),
    );
    assert_eq!(guesses(&lib, a).state, PanelState::CouldntBeRead);

    // Whole scores, all zero: ready, and a logit of 0 is 50 %.
    lib.insert(
        "UPDATE embedding SET label_scores = zeroblob(228) WHERE file_id = ?1",
        (a_file,),
    );
    let ready = guesses(&lib, a);
    assert_eq!(ready.state, PanelState::Ready);
    assert_eq!(ready.genres.len(), MOST_GENRES);
    assert_eq!(ready.genres[0].percent, 50);
}

// ---- the labels -------------------------------------------------------------------

#[test]
fn label_keys_are_the_names_in_camel_case() {
    assert_eq!(label_key("Hip hop music"), "hipHopMusic");
    assert_eq!(label_key("New-age music"), "newAgeMusic");
    assert_eq!(label_key("A capella"), "aCapella");
    assert_eq!(label_key("Techno"), "techno");
}

#[test]
fn every_kept_label_has_its_own_key_and_wording_in_the_locale() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/locales/en/audioModel.json");
    let text = std::fs::read_to_string(path).expect("the locale file");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let labels = json["label"].as_object().expect("a `label` group");
    let mut keys: Vec<String> = SUBSET.iter().map(|(_, n)| label_key(n)).collect();
    let all = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), all, "two labels share a key");
    for key in &keys {
        assert!(labels.contains_key(key), "no wording for {key}");
    }
    assert_eq!(labels.len(), all, "wording for a label that is not kept");
}
