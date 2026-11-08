//! Your own genre model (5B-B3a): the learner on made-up fingerprints, then
//! the stored model, the signature, the switch and the jobs on a made-up
//! Library. Nothing here listens to audio.

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::super::genre_model::{
    self, fit, learn, learn_job, logit, request, your_genres, GenreLearner, Interrupted, Learned,
    Sample, Stopped, MIN_TRACKS, PRIORITY,
};
use super::super::ledger::{self, Outcome};
use super::super::subset::{self, SUBSET_LEN};
use super::super::EMBEDDING_LEN;
use crate::db::DbError;
use crate::jobs::{JobContext, JobError, JobId, JobKind, JobQueue, JobStatus, NewJob};
use crate::search::tests::support::Lib;
use crate::settings::{write_feature_enabled, Feature};

// ---- Made-up fingerprints ---------------------------------------------------------

/// A small deterministic source of numbers in -0.5..0.5.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) - 0.5
    }
}

/// A track of genre number `genre`: noise on every number, a different level
/// on every number (so the numbers are not on one scale), and a push of
/// 1.5 on the eight numbers that belong to the genre.
fn sound_of(genre: usize, noise: &mut Noise) -> Vec<f32> {
    (0..EMBEDDING_LEN)
        .map(|i| {
            let level = 0.3 + (i % 7) as f32;
            let push = if (8 * genre..8 * genre + 8).contains(&i) {
                1.5
            } else {
                0.0
            };
            level + push + 2.0 * noise.next()
        })
        .collect()
}

/// `per_genre[g]` tracks of each genre `g`, one genre after the other.
fn samples(per_genre: &[usize], seed: u64) -> Vec<Sample> {
    let mut noise = Noise(seed);
    let mut out = Vec::new();
    for (g, &n) in per_genre.iter().enumerate() {
        for _ in 0..n {
            out.push(Sample {
                node: 100 + g as i64,
                fingerprint: sound_of(g, &mut noise),
            });
        }
    }
    out
}

fn no_report(_: f64) -> Result<(), Stopped> {
    Ok(())
}

// ---- The learner ------------------------------------------------------------------

#[test]
fn the_learner_names_the_right_genre_for_tracks_it_has_not_seen() {
    let learned = fit(&samples(&[12, 12, 12], 1), &mut no_report)
        .unwrap()
        .expect("three genres of 12 are enough");
    assert_eq!(learned.nodes.len(), 3);
    let mut noise = Noise(99);
    let mut right = 0;
    for genre in 0..3 {
        for _ in 0..10 {
            let heard = sound_of(genre, &mut noise);
            let best = learned
                .nodes
                .iter()
                .max_by(|a, b| {
                    let (a, b) = (
                        logit(&learned.mean, &learned.scale, &a.weights, a.bias, &heard),
                        logit(&learned.mean, &learned.scale, &b.weights, b.bias, &heard),
                    );
                    a.total_cmp(&b)
                })
                .unwrap();
            right += usize::from(best.node == 100 + genre as i64);
        }
    }
    assert!(right >= 28, "{right} of 30 unseen tracks named right");
}

#[test]
fn a_genre_with_fewer_than_ten_tracks_is_left_out() {
    let learned = fit(&samples(&[12, 9, 15], 2), &mut no_report)
        .unwrap()
        .unwrap();
    let nodes: Vec<i64> = learned.nodes.iter().map(|n| n.node).collect();
    assert_eq!(nodes, vec![100, 102], "the genre of 9 has no weights");
    assert_eq!(learned.tracks, 27, "and its tracks are not learned from");
    assert_eq!(
        learned
            .nodes
            .iter()
            .map(|n| n.trained_on)
            .collect::<Vec<_>>(),
        vec![12, 15]
    );
    // Exactly ten is enough.
    let ten = fit(&samples(&[MIN_TRACKS, MIN_TRACKS], 2), &mut no_report).unwrap();
    assert_eq!(ten.unwrap().nodes.len(), 2);
}

#[test]
fn fewer_than_two_genres_of_ten_make_no_model() {
    assert_eq!(fit(&[], &mut no_report), Ok(None));
    assert_eq!(fit(&samples(&[30], 3), &mut no_report), Ok(None));
    assert_eq!(fit(&samples(&[30, 9, 4], 3), &mut no_report), Ok(None));
}

#[test]
fn the_same_tracks_give_exactly_the_same_weights() {
    let tracks = samples(&[11, 13, 12], 4);
    let first = fit(&tracks, &mut no_report).unwrap().unwrap();
    let second = fit(&tracks, &mut no_report).unwrap().unwrap();
    assert_eq!(first, second);
    assert!(first
        .nodes
        .iter()
        .all(|n| n.weights.iter().any(|w| *w != 0.0)));
}

#[test]
fn the_learner_stops_when_asked_to_and_says_so() {
    let mut calls = 0;
    let stopped = fit(&samples(&[10, 10], 5), &mut |_| {
        calls += 1;
        if calls > 3 {
            Err(Stopped)
        } else {
            Ok(())
        }
    });
    assert_eq!(stopped, Err(Stopped));
}

#[test]
fn a_number_that_is_the_same_in_every_track_is_ignored_not_divided_by_zero() {
    let mut tracks = samples(&[10, 10], 6);
    for t in &mut tracks {
        t.fingerprint[191] = 4.0;
    }
    let learned = fit(&tracks, &mut no_report).unwrap().unwrap();
    assert_eq!(learned.scale[191], 0.0);
    assert!(learned
        .nodes
        .iter()
        .all(|n| n.weights.iter().all(|w| w.is_finite()) && n.bias.is_finite()));
}

// ---- A made-up Library ------------------------------------------------------------

fn turn(lib: &Lib, on: bool) {
    lib.writer
        .call(move |c| write_feature_enabled(c, Feature::AudioModel, on))
        .unwrap();
}

/// A genre node in the tree, by name.
fn node(lib: &Lib, name: &'static str) -> i64 {
    lib.insert(
        "INSERT INTO genre (name, name_key) VALUES (?1, lower(?1))",
        (name,),
    )
}

/// A Library track tagged with `genre` (or none) that the audio model has
/// listened to, with this fingerprint: its recording and its file.
fn track(lib: &Lib, name: &str, genre: Option<&str>, fingerprint: &[f32]) -> (i64, i64) {
    let (recording, file) = lib.track(Some(name), None, &format!("{name}.mp3"));
    if let Some(genre) = genre {
        lib.tag(file, &[("Name", name), ("Genre", genre)]);
    }
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
    let (fingerprint, scores) = (fingerprint.to_vec(), vec![-10.0; SUBSET_LEN]);
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

/// Two or three genres of `per_genre` listened tracks each, named
/// `Techno`, `House`, `Trance`, indexed. The tracks are in order.
struct Crate {
    lib: Lib,
    nodes: Vec<i64>,
    tracks: Vec<(i64, i64)>,
    noise: Noise,
}

const NAMES: [&str; 3] = ["Techno", "House", "Trance"];

fn crate_of(per_genre: &[usize]) -> Crate {
    let lib = Lib::new();
    turn(&lib, true);
    let nodes: Vec<i64> = NAMES.iter().map(|n| node(&lib, n)).collect();
    let mut noise = Noise(7);
    let mut tracks = Vec::new();
    for (g, &n) in per_genre.iter().enumerate() {
        for i in 0..n {
            let heard = sound_of(g, &mut noise);
            tracks.push(track(
                &lib,
                &format!("{}{i}", NAMES[g]),
                Some(NAMES[g]),
                &heard,
            ));
        }
    }
    lib.settle();
    Crate {
        lib,
        nodes,
        tracks,
        noise,
    }
}

impl Crate {
    /// A listened track with no genre, of genre number `genre`'s sound.
    fn untagged(&mut self, name: &str, genre: usize) -> i64 {
        let heard = sound_of(genre, &mut self.noise);
        let made = track(&self.lib, name, None, &heard);
        self.lib.settle();
        made.0
    }

    fn learn(&self) -> Learned {
        learn(&self.lib.writer, &mut no_report).unwrap()
    }

    fn due(&self) -> bool {
        self.lib.writer.call(|c| genre_model::due(c)).unwrap()
    }

    fn guesses(&self, recording: i64) -> Vec<(String, u8)> {
        self.lib
            .writer
            .call(move |c| Ok(your_genres(c, recording)))
            .unwrap()
            .into_iter()
            .map(|g| (g.name, g.percent))
            .collect()
    }

    fn names(&self, recording: i64) -> Vec<String> {
        self.guesses(recording).into_iter().map(|g| g.0).collect()
    }

    fn count(&self, table: &'static str) -> i64 {
        self.lib
            .writer
            .call(move |c| c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
            .unwrap()
    }

    /// The model's state: genres, tracks.
    fn state(&self) -> Option<(i64, i64)> {
        self.lib
            .writer
            .call(|c| {
                use rusqlite::OptionalExtension;
                c.query_row("SELECT genres, tracks FROM genre_model", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()
            })
            .unwrap()
    }

    fn trained_on(&self) -> Vec<(i64, i64)> {
        self.lib
            .writer
            .call(|c| {
                c.prepare("SELECT genre_id, trained_on FROM genre_model_node ORDER BY genre_id")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .unwrap()
    }
}

#[test]
fn a_model_learned_from_the_library_names_the_genre_of_a_track_with_none() {
    let mut c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    assert_eq!(c.state(), Some((2, 24)));
    assert_eq!(c.trained_on(), vec![(c.nodes[0], 12), (c.nodes[1], 12)]);
    let techno = c.untagged("Unknown One", 0);
    let house = c.untagged("Unknown Two", 1);
    assert_eq!(c.names(techno).first().map(String::as_str), Some("Techno"));
    assert_eq!(c.names(house).first().map(String::as_str), Some("House"));
    for recording in [techno, house] {
        let guesses = c.guesses(recording);
        assert!(guesses.len() <= 3 && guesses.iter().all(|g| g.1 >= 30 && g.1 <= 100));
        assert!(guesses.windows(2).all(|w| w[0].1 >= w[1].1), "best first");
    }
}

#[test]
fn a_genre_of_nine_library_tracks_gets_no_weights_and_is_never_named() {
    let mut c = crate_of(&[12, 12, 9]);
    assert_eq!(c.learn(), Learned::Genres(2));
    assert_eq!(c.trained_on(), vec![(c.nodes[0], 12), (c.nodes[1], 12)]);
    let trance = c.untagged("Unknown Three", 2);
    assert!(!c.names(trance).contains(&"Trance".to_string()));
}

#[test]
fn with_fewer_than_two_genres_of_ten_there_is_no_model_and_no_guesses() {
    let mut c = crate_of(&[14, 6]);
    assert_eq!(c.learn(), Learned::TooLittle);
    // "No model" is stored, so the answer isn't worked out again.
    assert_eq!(c.state(), Some((0, 0)));
    assert_eq!(c.count("genre_model_node"), 0);
    assert!(!c.due());
    assert_eq!(c.learn(), Learned::Unchanged);
    let heard = c.untagged("Unknown", 0);
    assert!(c.guesses(heard).is_empty());
    assert!(c.guesses(c.tracks[0].0).is_empty());
}

#[test]
fn a_model_is_learned_again_only_when_what_it_learned_from_changed() {
    let mut c = crate_of(&[12, 12]);
    assert!(c.due(), "nothing learned yet");
    assert_eq!(c.learn(), Learned::Genres(2));
    assert!(!c.due());
    // A mark that a new learning would overwrite.
    c.lib.exec("UPDATE genre_model SET tracks = 999");
    assert_eq!(c.learn(), Learned::Unchanged);
    assert_eq!(c.state(), Some((2, 999)), "not learned again");
    // A track in no genre, or in a genre too small to count, changes nothing.
    c.untagged("Unknown", 0);
    assert!(!c.due());
    for i in 0..3 {
        let heard = sound_of(2, &mut c.noise);
        track(&c.lib, &format!("Trance{i}"), Some("Trance"), &heard);
    }
    c.lib.settle();
    assert!(
        !c.due(),
        "three tracks of a third genre are too few to count"
    );
    // Another track in a learned genre does.
    let heard = sound_of(0, &mut c.noise);
    track(&c.lib, "Techno New", Some("Techno"), &heard);
    c.lib.settle();
    assert!(c.due());
    assert_eq!(c.learn(), Learned::Genres(2));
    assert_eq!(c.state(), Some((2, 25)));
}

#[test]
fn a_track_whose_genre_changes_in_the_index_is_learned_again_with_its_new_genre() {
    let c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    assert!(!c.due());
    // The track's tag now says House; the index (settle) writes its node.
    let (recording, file) = c.tracks[0];
    c.lib.tag(file, &[("Name", "Techno0"), ("Genre", "House")]);
    c.lib.settle();
    let node: i64 = c
        .lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT genre_node FROM track_row WHERE recording_id = ?1",
                [recording],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(node, c.nodes[1]);
    assert!(c.due());
    assert_eq!(c.learn(), Learned::Genres(2));
    assert_eq!(c.trained_on(), vec![(c.nodes[0], 11), (c.nodes[1], 13)]);
}

#[test]
fn a_genre_tree_edit_that_changes_a_tracks_node_is_learned_again_once_the_index_has_run() {
    let c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    // An alias sends the spelling "Hard Techno" to the Techno node.
    let (_, file) = c.tracks[12];
    c.lib
        .tag(file, &[("Name", "House0"), ("Genre", "Hard Techno")]);
    c.lib.settle();
    assert!(c.due(), "the track has no node now");
    assert_eq!(c.learn(), Learned::Genres(2));
    assert_eq!(c.trained_on(), vec![(c.nodes[0], 12), (c.nodes[1], 11)]);
    c.lib.insert(
        "INSERT INTO genre_alias (genre_id, alias, alias_key, kind) VALUES (?1, 'Hard Techno', 'hard techno', 'exact')",
        (c.nodes[0],),
    );
    // The edit only marks the tracks due; nothing is learned from it until
    // the index has run.
    assert!(!c.due());
    c.lib.settle();
    assert!(c.due());
    assert_eq!(c.learn(), Learned::Genres(2));
    assert_eq!(c.trained_on(), vec![(c.nodes[0], 13), (c.nodes[1], 11)]);
}

#[test]
fn with_the_switch_off_nothing_is_queued_nothing_learned_and_the_model_is_kept_but_silent() {
    let mut c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    let heard = c.untagged("Unknown", 0);
    assert!(!c.guesses(heard).is_empty());
    turn(&c.lib, false);
    assert!(!c.due(), "off: nothing is due");
    request(&c.lib.writer, |job| -> Result<JobId, DbError> {
        panic!("queued {job:?} while off")
    })
    .unwrap();
    assert_eq!(c.learn(), Learned::Off);
    assert_eq!(c.count("genre_model_node"), 2, "the model is kept");
    assert_eq!(c.state(), Some((2, 24)));
    assert!(c.guesses(heard).is_empty(), "and says nothing");
    // A change meanwhile is learned when it is turned on again.
    turn(&c.lib, true);
    assert!(!c.guesses(heard).is_empty());
    assert!(!c.due());
}

#[test]
fn asking_queues_one_job_at_the_lowest_priority_when_the_model_is_out_of_date() {
    let c = crate_of(&[12, 12]);
    let asked = Mutex::new(Vec::new());
    request(&c.lib.writer, |job: NewJob| -> Result<JobId, DbError> {
        asked.lock().unwrap().push(job);
        Ok(JobId(1))
    })
    .unwrap();
    let asked = asked.into_inner().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].kind, JobKind::GenreModel);
    assert_eq!(asked[0].priority, PRIORITY);
    assert!(PRIORITY < super::super::PRIORITY, "under the embed job");
    // Up to date: nothing is asked.
    assert_eq!(c.learn(), Learned::Genres(2));
    request(&c.lib.writer, |job| -> Result<JobId, DbError> {
        panic!("queued {job:?} though current")
    })
    .unwrap();
}

#[test]
fn a_model_of_another_embedding_model_is_not_used_and_is_replaced() {
    let mut c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    let heard = c.untagged("Unknown", 0);
    assert!(!c.guesses(heard).is_empty());
    // As if it came in a bundle made with an older model.
    c.lib.exec(
        "UPDATE genre_model SET model_id = 'ced-tiny@00000000/labels0', signature = 'old';
         UPDATE genre_model_node SET model_id = 'ced-tiny@00000000/labels0';",
    );
    assert!(c.guesses(heard).is_empty(), "the old model is not used");
    assert!(c.due(), "and a new one is wanted");
    assert_eq!(c.learn(), Learned::Genres(2));
    assert!(!c.guesses(heard).is_empty());
    let model: String = c
        .lib
        .writer
        .call(|c| c.query_row("SELECT model_id FROM genre_model", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(model, subset::model_id());
}

#[test]
fn fingerprints_of_another_embedding_model_are_not_learned_from() {
    let c = crate_of(&[12, 12]);
    c.lib
        .exec("UPDATE embedding SET model_id = 'ced-tiny@00000000/labels0'");
    assert_eq!(c.learn(), Learned::TooLittle);
    assert!(c.guesses(c.tracks[0].0).is_empty());
}

#[test]
fn deleting_a_genre_node_takes_its_weights_with_it_and_it_is_no_longer_named() {
    let mut c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    let heard = c.untagged("Unknown", 1);
    assert_eq!(c.names(heard).first().map(String::as_str), Some("House"));
    let house = c.nodes[1];
    c.lib.insert("DELETE FROM genre WHERE id = ?1", (house,));
    assert_eq!(c.trained_on(), vec![(c.nodes[0], 12)]);
    assert!(!c.names(heard).contains(&"House".to_string()));
    // The tracks of the deleted genre have no node once the index has run,
    // and the model is learned again (now of too few genres).
    c.lib.settle();
    assert!(c.due());
    assert_eq!(c.learn(), Learned::TooLittle);
    assert_eq!(c.count("genre_model_node"), 0);
}

#[test]
fn a_short_or_unreadable_fingerprint_is_left_out_and_the_rest_is_learned() {
    let c = crate_of(&[12, 12]);
    // The table refuses these (its length check); a damaged file might not.
    let (short, garbled) = (c.tracks[0].1, c.tracks[1].1);
    c.lib
        .writer
        .call(move |conn| {
            conn.execute_batch("PRAGMA ignore_check_constraints = ON")?;
            conn.execute(
                "UPDATE embedding SET fingerprint = zeroblob(10) WHERE file_id = ?1",
                [short],
            )?;
            let nan: Vec<u8> = [f32::NAN; EMBEDDING_LEN]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            conn.execute(
                "UPDATE embedding SET fingerprint = ?2 WHERE file_id = ?1",
                (garbled, nan),
            )?;
            conn.execute_batch("PRAGMA ignore_check_constraints = OFF")
        })
        .unwrap();
    assert_eq!(c.learn(), Learned::Genres(2));
    assert_eq!(c.state(), Some((2, 22)), "two tracks were left out");
    assert!(!c.due(), "and it is not tried again every time");
    assert!(
        c.guesses(c.tracks[0].0).is_empty(),
        "the short one has no fingerprint to name"
    );
    assert!(!c.guesses(c.tracks[5].0).is_empty());
}

#[test]
fn learning_that_is_stopped_keeps_the_model_it_had() {
    let c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    let heard = sound_of(0, &mut Noise(5));
    let (recording, _) = track(&c.lib, "Another", Some("Techno"), &heard);
    c.lib.settle();
    let stopped = learn(&c.lib.writer, &mut |f| {
        if f > 0.0 {
            Err(Stopped)
        } else {
            Ok(())
        }
    });
    assert!(matches!(stopped, Err(Interrupted::Stopped)));
    assert_eq!(c.state(), Some((2, 24)), "still the earlier model");
    assert!(c.due());
    assert!(!c.guesses(recording).is_empty());
}

#[test]
fn a_training_file_whose_audio_changed_makes_the_model_due_again() {
    let c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    assert!(!c.due());
    // The file's audio is new and its fingerprint was made again for it:
    // same track, same file, same genre, but not what was learned from.
    let file = c.tracks[3].1;
    c.lib
        .writer
        .call(move |conn| {
            conn.execute(
                "UPDATE file SET audio_hash = ?1 WHERE id = ?2",
                (vec![0xEEu8; 34], file),
            )?;
            conn.execute(
                "UPDATE embedding SET audio_hash = ?1 WHERE file_id = ?2",
                (vec![0xEEu8; 34], file),
            )
        })
        .unwrap();
    assert!(c.due());
    assert_eq!(c.learn(), Learned::Genres(2));
    assert!(!c.due());
}

#[test]
fn at_most_three_genres_are_named_and_only_from_thirty_percent() {
    let c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    // Replace the learned weights with ones that give each genre a fixed
    // probability whatever the track: no weights, only a bias.
    let ones: Vec<u8> = [1.0f32; EMBEDDING_LEN]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let model = subset::model_id();
    let chances = [
        ("Ninety five", 0.95),
        ("Eighty", 0.80),
        ("Sixty", 0.60),
        ("Forty", 0.40),
        ("Just in", 0.301),
        ("Just out", 0.29),
    ];
    c.lib
        .writer
        .call(move |conn| {
            conn.execute(
                "UPDATE genre_model SET mean = zeroblob(768), scale = ?1",
                [ones],
            )?;
            conn.execute("DELETE FROM genre_model_node", [])?;
            for (name, p) in chances {
                conn.execute(
                    "INSERT INTO genre (name, name_key) VALUES (?1, lower(?1))",
                    [name],
                )?;
                let bias = (p / (1.0 - p) as f64).ln();
                conn.execute(
                    "INSERT INTO genre_model_node (genre_id, weights, bias, trained_on, model_id)
                     VALUES (last_insert_rowid(), zeroblob(768), ?1, 10, ?2)",
                    (bias, &model),
                )?;
            }
            Ok(())
        })
        .unwrap();
    let track = c.tracks[0].0;
    assert_eq!(
        c.guesses(track),
        vec![
            ("Ninety five".to_string(), 95),
            ("Eighty".to_string(), 80),
            ("Sixty".to_string(), 60),
        ],
        "four pass, three are named, best first"
    );
    c.lib
        .exec("DELETE FROM genre WHERE name IN ('Ninety five', 'Eighty', 'Sixty', 'Forty')");
    assert_eq!(
        c.guesses(track),
        vec![("Just in".to_string(), 30)],
        "30% is in, 29% is out"
    );
}

// ---- The jobs ---------------------------------------------------------------------

fn finished(lib: &Lib, id: JobId) -> JobStatus {
    let start = Instant::now();
    loop {
        let job = lib
            .writer
            .call(move |c| crate::jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            return job.status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "job {id} never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn queue_with_learner(lib: &Lib) -> JobQueue {
    JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(
            JobKind::GenreModel,
            crate::scan::chain::after_genre_model(GenreLearner),
        )
        .start()
        .unwrap()
}

#[test]
fn the_job_learns_the_model_and_a_second_run_changes_nothing() {
    let mut c = crate_of(&[12, 12]);
    let queue = queue_with_learner(&c.lib);
    let id = queue.enqueue(learn_job()).unwrap();
    assert_eq!(finished(&c.lib, id), JobStatus::Done);
    assert_eq!(c.state(), Some((2, 24)));
    let heard = c.untagged("Unknown", 0);
    assert!(!c.guesses(heard).is_empty());
    c.lib.exec("UPDATE genre_model SET tracks = 999");
    let again = queue.enqueue(learn_job()).unwrap();
    assert_eq!(finished(&c.lib, again), JobStatus::Done);
    assert_eq!(c.state(), Some((2, 999)));
    queue.shutdown();
}

#[test]
fn the_job_with_the_switch_off_ends_at_once_and_learns_nothing() {
    let c = crate_of(&[12, 12]);
    turn(&c.lib, false);
    let queue = queue_with_learner(&c.lib);
    let id = queue.enqueue(learn_job()).unwrap();
    assert_eq!(finished(&c.lib, id), JobStatus::Done);
    assert_eq!(c.state(), None);
    queue.shutdown();
}

#[test]
fn switching_off_cancels_a_running_genre_job_and_keeps_the_model() {
    let c = crate_of(&[12, 12]);
    assert_eq!(c.learn(), Learned::Genres(2));
    let (reached, reached_rx) = mpsc::channel();
    let reached = Mutex::new(reached);
    let queue = JobQueue::builder(c.lib.writer.clone())
        .workers(1)
        .handler(
            JobKind::GenreModel,
            move |ctx: &JobContext| -> Result<(), JobError> {
                reached.lock().unwrap().send(()).unwrap();
                loop {
                    ctx.check_cancelled()?;
                    std::thread::sleep(Duration::from_millis(5));
                }
            },
        )
        .start()
        .unwrap();
    let id = queue.enqueue(learn_job()).unwrap();
    reached_rx.recv_timeout(Duration::from_secs(60)).unwrap();
    super::super::switch(&c.lib.writer, &queue, false).unwrap();
    assert_eq!(finished(&c.lib, id), JobStatus::Cancelled);
    assert_eq!(c.count("genre_model_node"), 2, "the model is kept");
    queue.shutdown();
}

/// Index and embed jobs that do nothing, each followed by what the chain
/// asks for after it.
fn queue_of_the_chain(lib: &Lib) -> JobQueue {
    let nothing = |_: &JobContext| -> Result<(), JobError> { Ok(()) };
    JobQueue::builder(lib.writer.clone())
        .workers(1)
        .handler(JobKind::Index, crate::scan::chain::after_index(nothing))
        .handler(JobKind::Embed, crate::scan::chain::after_embed(nothing))
        .start()
        .unwrap()
}

fn genre_jobs(lib: &Lib) -> i64 {
    lib.writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM job WHERE kind = 'genre_model'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap()
}

#[test]
fn the_embed_job_and_the_index_job_each_ask_for_the_model_when_they_end() {
    for kind in [JobKind::Embed, JobKind::Index] {
        let c = crate_of(&[12, 12]);
        let queue = queue_of_the_chain(&c.lib);
        assert_eq!(genre_jobs(&c.lib), 0);
        let id = queue.enqueue(NewJob::new(kind)).unwrap();
        assert_eq!(finished(&c.lib, id), JobStatus::Done);
        assert_eq!(genre_jobs(&c.lib), 1, "after the {kind:?} job");
        queue.shutdown();
    }
}

#[test]
fn the_embed_and_index_jobs_ask_for_nothing_with_the_switch_off_or_nothing_new_to_learn() {
    for kind in [JobKind::Embed, JobKind::Index] {
        let c = crate_of(&[12, 12]);
        turn(&c.lib, false);
        let queue = queue_of_the_chain(&c.lib);
        let id = queue.enqueue(NewJob::new(kind)).unwrap();
        assert_eq!(finished(&c.lib, id), JobStatus::Done);
        assert_eq!(genre_jobs(&c.lib), 0, "off, after the {kind:?} job");
        turn(&c.lib, true);
        assert_eq!(c.learn(), Learned::Genres(2));
        let id = queue.enqueue(NewJob::new(kind)).unwrap();
        assert_eq!(finished(&c.lib, id), JobStatus::Done);
        assert_eq!(genre_jobs(&c.lib), 0, "current, after the {kind:?} job");
        queue.shutdown();
    }
}
