//! The local key through the analyze job: kept in the ledger always, shown
//! only behind the owner's test switch, and the comparison with rekordbox.

use super::audio;
use super::job::{analyze_all, local_row, stored, track_of, walk_and_hash};
use super::key::{progression, third_octave, RATE};
use super::support::Library;
use crate::estimate::compare::{self, agreement, Agreement, KeyAgreement};
use crate::settings::{read_feature_enabled, switch_feature, Feature};
use crate::tags::key::{Mode, MusicalKey};

/// A WAV of a chord progression in a key: about 16 s.
fn tonal(pitch_class: u8, mode: Mode) -> Vec<u8> {
    let samples = progression(third_octave(pitch_class), mode, 4.0)
        .into_iter()
        .map(|x| (x * 32_767.0).round() as i16)
        .collect();
    audio::wav(&audio::Pcm {
        rate: RATE,
        channels: 1,
        samples,
    })
}

fn switch(library: &Library, on: bool) {
    library
        .writer
        .call(move |c| switch_feature(c, Feature::LocalKey, on))
        .unwrap();
}

/// How many `'local'` rows hold a key.
fn local_keys(library: &Library) -> i64 {
    library
        .writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM analysis WHERE source = 'local' AND key IS NOT NULL",
                [],
                |r| r.get(0),
            )
        })
        .unwrap()
}

/// The key the track shows and its source, by the view.
fn shown_key(library: &Library, track: i64) -> (Option<String>, Option<String>) {
    library
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT key, key_source FROM recording_display WHERE recording_id = ?1",
                [track],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap()
}

#[test]
fn the_key_is_kept_in_the_ledger_but_with_the_switch_off_no_local_row_holds_one() {
    let library = Library::new();
    library.put("a.wav", &tonal(2, Mode::Major));
    let ids = walk_and_hash(&library);
    assert!(!library
        .writer
        .call(|c| read_feature_enabled(c, Feature::LocalKey))
        .unwrap());
    assert_eq!(analyze_all(&library), 1);
    let row = stored(&library, ids["a.wav"]).unwrap();
    let d_major = MusicalKey {
        pitch_class: 2,
        mode: Mode::Major,
    };
    assert_eq!(row.key, Some(d_major), "{row:?}");
    assert!(row.key_confidence.is_some_and(|c| c > 0.0 && c <= 1.0));
    assert_eq!(local_keys(&library), 0);
    let track = track_of(&library, ids["a.wav"]);
    assert_eq!(shown_key(&library, track).0, None);
    // And the comparison, which reads the local key, says nothing.
    assert_eq!(library.writer.call(|c| compare::compare(c)).unwrap(), None);
}

#[test]
fn turning_the_switch_on_shows_the_key_in_camelot_and_off_takes_it_away() {
    let library = Library::new();
    library.put("a.wav", &tonal(11, Mode::Minor));
    let ids = walk_and_hash(&library);
    analyze_all(&library);
    let track = track_of(&library, ids["a.wav"]);
    let before = local_row(&library, track);

    switch(&library, true);
    let (id, bpm, key, energy) = local_row(&library, track).unwrap();
    // B minor is 10A.
    assert_eq!((key.as_deref(), energy), (Some("10A"), None));
    assert_eq!(
        shown_key(&library, track),
        (Some("10A".into()), Some("local".into()))
    );
    if let Some((was, had_bpm, ..)) = before {
        assert_eq!((id, bpm), (was, had_bpm), "the same row, its BPM kept");
    }
    // A rerun changes nothing.
    assert_eq!(analyze_all(&library), 0);
    assert_eq!(
        local_row(&library, track).unwrap().2.as_deref(),
        Some("10A")
    );

    switch(&library, false);
    assert_eq!(local_keys(&library), 0);
    assert_eq!(shown_key(&library, track).0, None);
    assert_eq!(local_row(&library, track).map(|r| r.1), before.map(|r| r.1));
}

#[test]
fn rekordboxs_key_still_wins_over_the_local_one_which_stays() {
    let library = Library::new();
    library.put("a.wav", &tonal(2, Mode::Major));
    let ids = walk_and_hash(&library);
    let track = track_of(&library, ids["a.wav"]);
    library
        .writer
        .call(move |c| {
            c.execute(
                "INSERT INTO analysis (recording_id, source, key) VALUES (?1, 'rekordbox', '8A')",
                [track],
            )
        })
        .unwrap();
    switch(&library, true);
    analyze_all(&library);
    // D major is 10B.
    assert_eq!(
        local_row(&library, track).unwrap().2.as_deref(),
        Some("10B")
    );
    assert_eq!(
        shown_key(&library, track),
        (Some("8A".into()), Some("rekordbox".into()))
    );
}

#[test]
fn a_window_whose_key_is_silence_shows_no_key() {
    let library = Library::new();
    library.put("a.wav", &tonal(2, Mode::Major));
    let ids = walk_and_hash(&library);
    analyze_all(&library);
    // As if the window had matched no key.
    library
        .writer
        .call(|c| {
            c.execute(
                "UPDATE file_estimate SET key_pitch_class = NULL, key_mode = 'silence',
                     key_confidence = NULL",
                [],
            )
        })
        .unwrap();
    switch(&library, true);
    let row = stored(&library, ids["a.wav"]).unwrap();
    assert_eq!((row.key, row.key_mode.as_deref()), (None, Some("silence")));
    assert_eq!(local_keys(&library), 0);
}

#[test]
fn keys_are_told_apart_as_exact_relative_one_step_parallel_or_other() {
    let key = |pitch_class, mode| MusicalKey { pitch_class, mode };
    let a_minor = key(9, Mode::Minor); // 8A
    assert_eq!(agreement(a_minor, a_minor), Agreement::Exact);
    assert_eq!(agreement(a_minor, key(0, Mode::Major)), Agreement::Relative); // 8B
    assert_eq!(agreement(a_minor, key(4, Mode::Minor)), Agreement::OneStep); // 9A
    assert_eq!(agreement(a_minor, key(2, Mode::Minor)), Agreement::OneStep); // 7A
    assert_eq!(agreement(a_minor, key(9, Mode::Major)), Agreement::Parallel); // 11B
    assert_eq!(agreement(a_minor, key(3, Mode::Minor)), Agreement::Other); // 2A
                                                                           // Round the wheel: 12A and 1A are one step apart.
    assert_eq!(
        agreement(key(1, Mode::Minor), key(8, Mode::Minor)),
        Agreement::OneStep
    );
}

#[test]
fn the_comparison_counts_each_kind_overall_and_per_genre_node() {
    let library = Library::new();
    let names = [
        "a.wav", "b.wav", "c.wav", "d.wav", "e.wav", "f.wav", "g.wav",
    ];
    for name in names {
        library.put(name, &tonal(9, Mode::Minor));
    }
    let ids = walk_and_hash(&library);
    analyze_all(&library);
    switch(&library, true);
    // Every local key is set to A minor (8A); rekordbox says, per file, the
    // key and the genre node. g.wav has no rekordbox key: not counted.
    let cases: [(&str, Option<&str>, Option<i64>); 7] = [
        ("a.wav", Some("8A"), Some(1)),
        ("b.wav", Some("8A"), Some(2)),
        ("c.wav", Some("8B"), Some(1)),
        ("d.wav", Some("9A"), Some(1)),
        ("e.wav", Some("11B"), None),
        ("f.wav", Some("3A"), Some(2)),
        ("g.wav", None, Some(1)),
    ];
    let rows: Vec<(i64, Option<String>, Option<i64>)> = cases
        .iter()
        .map(|(name, key, genre)| {
            (
                track_of(&library, ids[*name]),
                key.map(String::from),
                *genre,
            )
        })
        .collect();
    library
        .writer
        .call(move |c| {
            c.execute_batch(
                "UPDATE file_estimate SET key_pitch_class = 9, key_mode = 'minor', key_confidence = 0.1;
                 INSERT INTO genre (id, name, name_key) VALUES (1, 'House', 'house'), (2, 'Techno', 'techno');",
            )?;
            for (track, key, genre) in &rows {
                if let Some(key) = key {
                    c.execute(
                        "INSERT INTO analysis (recording_id, source, key) VALUES (?1, 'rekordbox', ?2)",
                        rusqlite::params![track, key],
                    )?;
                }
                c.execute(
                    "INSERT INTO track_row (recording_id, title_key, genre_node) VALUES (?1, 't', ?2)
                     ON CONFLICT (recording_id) DO UPDATE SET genre_node = excluded.genre_node",
                    rusqlite::params![track, genre],
                )?;
            }
            Ok(())
        })
        .unwrap();
    let found = library
        .writer
        .call(|c| compare::compare(c))
        .unwrap()
        .expect("the switch is on");
    let counts = |tracks, exact, relative, one_step, parallel, other| KeyAgreement {
        tracks,
        exact,
        relative,
        one_step,
        parallel,
        other,
    };
    assert_eq!(found.overall, counts(6, 2, 1, 1, 1, 1));
    let genres: Vec<(Option<&str>, KeyAgreement)> = found
        .genres
        .iter()
        .map(|g| (g.genre.as_deref(), g.counts))
        .collect();
    assert_eq!(
        genres,
        [
            (Some("House"), counts(3, 1, 1, 1, 0, 0)),
            (Some("Techno"), counts(2, 1, 0, 0, 0, 1)),
            (None, counts(1, 0, 0, 0, 1, 0)),
        ]
    );
}
