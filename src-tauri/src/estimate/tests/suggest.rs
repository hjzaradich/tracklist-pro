//! The energy suggestion: the percentile batch, when it is worked out
//! again, what has none, and the basis it is given with (ROADMAP 3.7; 3C-E1b).
//! That it never becomes a track's energy is tested where the energy readers
//! are (`track_table` and `conflicts` tests).

use crate::estimate::suggest::{
    needed, refresh, refresh_paged, suggestion_for, Basis, BasisCode, EnergySuggestion, BATCH,
};
use crate::estimate::support::plant_features;
use crate::search::tests::support::Lib;

/// A Library track named `name`: its recording and file.
fn track(lib: &Lib, name: &str) -> (i64, i64) {
    let (recording, file) = lib.track(Some(name), Some("Someone"), &format!("{name}.mp3"));
    lib.library(recording, file);
    (recording, file)
}

/// `n` tracks whose every feature rises with their number.
fn rising(lib: &Lib, n: usize) -> Vec<(i64, i64)> {
    (0..n).map(|i| rising_track(lib, i)).collect()
}

fn rising_track(lib: &Lib, i: usize) -> (i64, i64) {
    let (recording, file) = track(lib, &format!("t{i}"));
    let x = i as f64;
    lib.writer
        .call(move |c| {
            plant_features(
                c,
                file,
                Some(100.0 + x),
                Some(-30.0 + x / 10.0),
                1.0 + x,
                2.0 + x,
            )
        })
        .unwrap();
    (recording, file)
}

fn suggestion(lib: &Lib, recording: i64) -> Option<EnergySuggestion> {
    lib.writer
        .call(move |c| suggestion_for(c, recording))
        .unwrap()
}

fn suggested(lib: &Lib, recording: i64) -> Option<u8> {
    suggestion(lib, recording).map(|s| s.value)
}

fn needs_work(lib: &Lib) -> bool {
    lib.writer.call(|c| needed(c)).unwrap()
}

fn batch_count(lib: &Lib) -> Option<i64> {
    lib.writer
        .call(|c| {
            c.query_row(
                "SELECT json_extract(value, '$.count') FROM setting WHERE key = ?1",
                [BATCH],
                |r| r.get(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
        })
        .unwrap()
}

#[test]
fn the_suggestion_is_the_tenth_of_the_library_a_track_ranks_in_and_says_why() {
    let lib = Lib::new();
    let tracks = rising(&lib, 10);
    assert_eq!(refresh(&lib.writer).unwrap(), 10);

    let values: Vec<Option<u8>> = tracks.iter().map(|t| suggested(&lib, t.0)).collect();
    assert_eq!(values, (1..=10).map(Some).collect::<Vec<_>>());

    // The basis is codes and numbers: where each feature stands in the
    // Library, 0 at the bottom and 100 at the top.
    let all = |percentile| {
        [
            BasisCode::Loudness,
            BasisCode::Onsets,
            BasisCode::Flux,
            BasisCode::Tempo,
        ]
        .map(|code| Basis { code, percentile })
    };
    assert_eq!(suggestion(&lib, tracks[9].0).unwrap().basis, all(100));
    assert_eq!(suggestion(&lib, tracks[0].0).unwrap().basis, all(0));
    // Four ninths of the way up, five ninths.
    assert_eq!(suggestion(&lib, tracks[4].0).unwrap().basis, all(44));
    assert_eq!(suggestion(&lib, tracks[5].0).unwrap().basis, all(56));
}

#[test]
fn a_track_with_no_features_has_no_suggestion_and_missing_features_are_left_out_of_the_basis() {
    let lib = Lib::new();
    let (never, _) = track(&lib, "never analysed");
    let (silent, silent_file) = track(&lib, "silent");
    let (bare, bare_file) = track(&lib, "no tempo and no loudness");
    assert!(!needs_work(&lib), "nothing is measured");

    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE file SET audio_hash = zeroblob(34) WHERE id = ?1",
                [silent_file],
            )?;
            c.execute(
                "INSERT INTO file_estimate (file_id, audio_hash, method_version, skipped)
                 SELECT id, audio_hash, ?2, 'silent' FROM file WHERE id = ?1",
                [silent_file, i64::from(crate::estimate::VERSION)],
            )?;
            plant_features(c, bare_file, None, None, 2.0, 5.0)
        })
        .unwrap();
    // The skipped file has no features, so only the other one counts.
    assert_eq!(refresh(&lib.writer).unwrap(), 1);

    let tracks = rising(&lib, 3);
    assert!(refresh(&lib.writer).unwrap() >= 3);
    assert_eq!(suggested(&lib, never), None);
    assert_eq!(suggested(&lib, silent), None);
    assert!(suggested(&lib, tracks[0].0).is_some());
    let codes: Vec<BasisCode> = suggestion(&lib, bare)
        .unwrap()
        .basis
        .iter()
        .map(|b| b.code)
        .collect();
    assert_eq!(codes, [BasisCode::Onsets, BasisCode::Flux]);
    // A recording that isn't a Library track has none either.
    assert_eq!(suggested(&lib, i64::MAX), None);
}

#[test]
fn with_nothing_measured_there_is_nothing_to_work_out_and_no_batch_is_recorded() {
    let lib = Lib::new();
    track(&lib, "a");
    assert!(!needs_work(&lib));
    assert_eq!(refresh(&lib.writer).unwrap(), 0);
    assert_eq!(batch_count(&lib), None);
}

#[test]
fn the_batch_is_worked_out_again_only_once_the_library_has_changed_by_more_than_five_percent() {
    let lib = Lib::new();
    let tracks = rising(&lib, 20);
    assert!(needs_work(&lib), "the first batch");
    assert_eq!(refresh(&lib.writer).unwrap(), 20);
    assert_eq!(batch_count(&lib), Some(20));
    assert!(!needs_work(&lib));
    assert_eq!(refresh(&lib.writer).unwrap(), 0, "nothing changed");

    // Make the lowest track's suggestion wrong: only a whole batch puts it
    // right.
    let (lowest, lowest_file) = tracks[0];
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE file_estimate SET energy_suggestion = 10 WHERE file_id = ?1",
                [lowest_file],
            )
        })
        .unwrap();

    // One more track is 5% of 20, not more than 5%: only the newcomer is
    // placed.
    let (newcomer, _) = rising_track(&lib, 20);
    assert!(needs_work(&lib));
    assert_eq!(refresh(&lib.writer).unwrap(), 1);
    assert_eq!(batch_count(&lib), Some(20), "not a new batch");
    assert_eq!(suggested(&lib, lowest), Some(10), "the others were left");
    assert_eq!(suggested(&lib, newcomer), Some(10), "the highest of 21");

    // Two more is 10%: the whole Library again.
    rising_track(&lib, 21);
    assert!(needs_work(&lib));
    assert!(refresh(&lib.writer).unwrap() >= 2);
    assert_eq!(batch_count(&lib), Some(22), "the files with features");
    assert_eq!(suggested(&lib, lowest), Some(1), "put right by the batch");
    assert!(!needs_work(&lib));
}

#[test]
fn a_file_measured_again_is_given_a_suggestion_again_without_a_whole_batch() {
    let lib = Lib::new();
    let tracks = rising(&lib, 20);
    refresh(&lib.writer).unwrap();
    let (recording, file) = tracks[5];
    assert!(suggested(&lib, recording).is_some());
    // What the job's record() leaves: new features and no suggestion.
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE file_estimate SET energy_suggestion = NULL, loudness_lufs = -5
                 WHERE file_id = ?1",
                [file],
            )
        })
        .unwrap();
    assert_eq!(suggested(&lib, recording), None);
    assert!(needs_work(&lib));
    assert_eq!(refresh(&lib.writer).unwrap(), 1);
    assert!(suggested(&lib, recording).is_some());
}

#[test]
fn a_first_pass_that_was_only_partway_is_batched_again_as_more_files_get_measured() {
    let lib = Lib::new();
    // Ten of twenty Library tracks are measured (the other drive is out).
    let first = rising(&lib, 10);
    for i in 10..20 {
        track(&lib, &format!("unmeasured {i}"));
    }
    assert_eq!(refresh(&lib.writer).unwrap(), 10);
    assert_eq!(
        batch_count(&lib),
        Some(10),
        "ten have features, not twenty tracks"
    );
    assert!(!needs_work(&lib));
    let (lowest, lowest_file) = first[0];
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE file_estimate SET energy_suggestion = 10 WHERE file_id = ?1",
                [lowest_file],
            )
        })
        .unwrap();

    // The other drive comes back and its files are measured: twice as many
    // files with features. Everything is ranked again, not only the new ones.
    for i in 10..20 {
        let (_, file) = track(&lib, &format!("later {i}"));
        lib.writer
            .call(move |c| plant_features(c, file, Some(110.0 + i as f64), Some(-20.0), 5.0, 9.0))
            .unwrap();
    }
    assert!(needs_work(&lib));
    assert!(refresh(&lib.writer).unwrap() > 10);
    assert_eq!(batch_count(&lib), Some(20));
    assert_eq!(suggested(&lib, lowest), Some(1), "ranked among all twenty");
}

#[test]
fn a_library_that_shrank_by_more_than_five_percent_is_batched_again() {
    let lib = Lib::new();
    let tracks = rising(&lib, 20);
    refresh(&lib.writer).unwrap();
    assert_eq!(batch_count(&lib), Some(20));
    // One track out is 5%: left alone.
    let out = |lib: &Lib, recordings: Vec<i64>| {
        lib.writer
            .call(move |c| {
                for r in recordings {
                    c.execute("DELETE FROM library_track WHERE recording_id = ?1", [r])?;
                }
                Ok(())
            })
            .unwrap();
    };
    out(&lib, vec![tracks[19].0]);
    assert!(!needs_work(&lib), "19 of 20 is within 5%");
    // Three out in all is 15%.
    out(&lib, vec![tracks[18].0, tracks[17].0]);
    assert!(needs_work(&lib));
    refresh(&lib.writer).unwrap();
    assert_eq!(batch_count(&lib), Some(17));
    assert!(!needs_work(&lib));
}

#[test]
fn the_work_goes_a_page_of_files_at_a_time_and_gives_the_same_answers() {
    let one = Lib::new();
    let paged = Lib::new();
    let (a, b) = (rising(&one, 20), rising(&paged, 20));
    assert_eq!(refresh(&one.writer).unwrap(), 20);
    // Three files to a call: pages of 3, to read and to write.
    assert_eq!(refresh_paged(&paged.writer, 3).unwrap(), 20);
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(suggestion(&one, x.0), suggestion(&paged, y.0));
        assert!(suggestion(&paged, y.0).is_some());
    }
    assert_eq!(batch_count(&paged), Some(20));
    // Newcomers go a page at a time too.
    rising_track(&paged, 20);
    assert_eq!(refresh_paged(&paged.writer, 3).unwrap(), 1);
}

#[test]
fn a_suggestion_is_read_from_its_own_row_and_the_batchs_table_not_from_the_library() {
    let lib = Lib::new();
    let tracks = rising(&lib, 10);
    refresh(&lib.writer).unwrap();
    let before = suggestion(&lib, tracks[4].0).unwrap();
    // Every other file's features change: only a reading of the whole
    // Library would notice.
    let own = tracks[4].1;
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE file_estimate SET onset_density = 500, spectral_flux = 500,
                                          loudness_lufs = -1, bpm = 190
                 WHERE file_id <> ?1",
                [own],
            )
        })
        .unwrap();
    assert_eq!(suggestion(&lib, tracks[4].0), Some(before));
}
