//! Tests for a crate's notes and summary line. Everything is synthetic:
//! made-up tracks in a migrated database in a temp dir.

use super::*;
use crate::crates::{add_recordings_on, create_folder_on, create_on, remove_on};
use crate::ops::{next_undo, undo_last_via, UndoOutcome};
use crate::search::tests::support::{Lib, Mount};

/// A Library track with a file, tagged with `genre` if it has one; its
/// track id and its Library track id.
fn track(lib: &Lib, n: usize, genre: Option<&str>) -> (i64, i64) {
    let name = format!("Music/tune {n}.mp3");
    let (recording, file) = lib.track(Some(&format!("Tune {n}")), Some("Kit"), &name);
    if let Some(genre) = genre {
        lib.tag(
            file,
            &[
                ("Name", &format!("Tune {n}")),
                ("Artist", "Kit"),
                ("Genre", genre),
            ],
        );
    }
    let library = lib.library(recording, file);
    (recording, library)
}

fn rekordbox(lib: &Lib, recording: i64, bpm: Option<f64>, key: Option<&'static str>) {
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, 'rekordbox', ?2, ?3)",
        (recording, bpm, key),
    );
}

fn energy(lib: &Lib, recording: i64, source: &'static str, energy: i64) {
    lib.insert(
        "INSERT INTO analysis (recording_id, source, energy) VALUES (?1, ?2, ?3)",
        (recording, source, energy),
    );
}

fn crate_of(lib: &Lib, name: &'static str, recordings: &[i64]) -> CrateId {
    let id = lib
        .writer
        .call(move |c| Ok(create_on(c, name)))
        .unwrap()
        .unwrap()
        .unwrap();
    add(lib, id, recordings);
    id
}

fn add(lib: &Lib, id: CrateId, recordings: &[i64]) {
    let recordings = recordings.to_vec();
    lib.writer
        .call(move |c| Ok(add_recordings_on(c, id, &recordings)))
        .unwrap()
        .unwrap()
        .unwrap();
}

/// The summary once the index is current.
fn summary(lib: &Lib, id: CrateId) -> CrateSummary {
    lib.settle();
    lib.writer
        .call(move |c| summarize(c, &Mount, id))
        .unwrap()
        .expect("the crate exists")
}

fn set_notes(lib: &Lib, id: CrateId, notes: &str) -> Done<Option<i64>> {
    let notes = notes.to_owned();
    lib.writer
        .call(move |c| Ok(set_notes_on(c, id, &notes)))
        .unwrap()
        .unwrap()
}

fn notes(lib: &Lib, id: CrateId) -> Option<String> {
    lib.writer
        .call(move |c| {
            c.query_row("SELECT notes FROM crate WHERE id = ?1", [id.0], |r| {
                r.get(0)
            })
        })
        .unwrap()
}

// ---- notes ---------------------------------------------------------------

#[test]
fn a_note_is_saved_and_the_summary_gives_it_back() {
    let lib = Lib::new();
    let id = crate_of(&lib, "Warmup", &[]);
    assert_eq!(summary(&lib, id).notes, None);
    assert!(set_notes(&lib, id, "Opens slow, builds to 124")
        .unwrap()
        .is_some());
    assert_eq!(
        summary(&lib, id).notes.as_deref(),
        Some("Opens slow, builds to 124")
    );
}

#[test]
fn a_note_is_trimmed_and_kept_to_one_line() {
    let lib = Lib::new();
    let id = crate_of(&lib, "Warmup", &[]);
    set_notes(&lib, id, "  first\nsecond\r\n\tthird  ").unwrap();
    assert_eq!(notes(&lib, id).as_deref(), Some("first second   third"));
}

#[test]
fn an_empty_note_clears_the_note() {
    let lib = Lib::new();
    let id = crate_of(&lib, "Warmup", &[]);
    set_notes(&lib, id, "Opens slow").unwrap();
    for empty in ["", "   ", "\n\t"] {
        set_notes(&lib, id, "Opens slow").unwrap();
        assert!(set_notes(&lib, id, empty).unwrap().is_some(), "{empty:?}");
        // Cleared is no note, not a note of nothing.
        assert_eq!(notes(&lib, id), None, "{empty:?}");
        assert_eq!(summary(&lib, id).notes, None);
    }
}

#[test]
fn saving_the_note_a_crate_has_records_nothing() {
    let lib = Lib::new();
    let id = crate_of(&lib, "Warmup", &[]);
    assert!(
        set_notes(&lib, id, "").unwrap().is_none(),
        "no note to none"
    );
    set_notes(&lib, id, "Opens slow").unwrap();
    assert!(set_notes(&lib, id, " Opens slow ").unwrap().is_none());
    // The earlier edit is still the one Undo would take back.
    let next = lib.writer.call(|c| Ok(next_undo(c))).unwrap().unwrap();
    assert_eq!(next.operation.unwrap().kind, NOTES_OPERATION);
}

#[test]
fn undo_of_a_notes_edit_puts_the_old_note_back_and_names_the_crate() {
    let lib = Lib::new();
    let id = crate_of(&lib, "Warmup", &[]);
    set_notes(&lib, id, "Opens slow").unwrap();
    set_notes(&lib, id, "Opens slow, builds").unwrap();

    let next = lib.writer.call(|c| Ok(next_undo(c))).unwrap().unwrap();
    let operation = next.operation.expect("an edit to undo");
    assert_eq!(operation.kind, NOTES_OPERATION);
    assert_eq!(operation.details.name.as_deref(), Some("Warmup"));

    match undo_last_via(&lib.writer).unwrap() {
        UndoOutcome::Undone { .. } => {}
        other => panic!("expected the undo to work, got {other:?}"),
    }
    assert_eq!(notes(&lib, id).as_deref(), Some("Opens slow"));
    // And the first edit: back to no note at all.
    undo_last_via(&lib.writer).unwrap();
    assert_eq!(notes(&lib, id), None);
}

#[test]
fn undo_of_clearing_a_note_brings_the_note_back() {
    let lib = Lib::new();
    let id = crate_of(&lib, "Warmup", &[]);
    set_notes(&lib, id, "Opens slow").unwrap();
    set_notes(&lib, id, "").unwrap();
    assert_eq!(notes(&lib, id), None);
    undo_last_via(&lib.writer).unwrap();
    assert_eq!(notes(&lib, id).as_deref(), Some("Opens slow"));
}

#[test]
fn a_note_over_the_limit_is_refused_and_changes_nothing() {
    let lib = Lib::new();
    let id = crate_of(&lib, "Warmup", &[]);
    set_notes(&lib, id, "Opens slow").unwrap();
    let longest = "n".repeat(MAX_NOTES_CHARS);
    assert!(set_notes(&lib, id, &longest).is_ok());
    let too_long = "n".repeat(MAX_NOTES_CHARS + 1);
    assert!(matches!(
        set_notes(&lib, id, &too_long),
        Err(CratesError::NotesTooLong)
    ));
    assert_eq!(notes(&lib, id).as_deref(), Some(longest.as_str()));
}

#[test]
fn a_folder_and_a_crate_that_is_not_there_take_no_note() {
    let lib = Lib::new();
    let folder = lib
        .writer
        .call(|c| Ok(create_folder_on(c, None, "Friday")))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        set_notes(&lib, folder, "Hello"),
        Err(CratesError::NotFound)
    ));
    assert!(matches!(
        set_notes(&lib, CrateId(9999), "Hello"),
        Err(CratesError::NotFound)
    ));
    // Neither has a summary.
    for id in [folder, CrateId(9999)] {
        let found = lib.writer.call(move |c| summarize(c, &Mount, id)).unwrap();
        assert_eq!(found, None);
    }
}

// ---- the summary line ----------------------------------------------------

#[test]
fn an_empty_crate_has_a_summary_that_says_it_has_no_tracks_and_nothing_else() {
    let lib = Lib::new();
    let id = crate_of(&lib, "Warmup", &[]);
    assert_eq!(
        summary(&lib, id),
        CrateSummary {
            notes: None,
            tracks: 0,
            bpm: None,
            energy: None,
            keys: Vec::new(),
            genres: Vec::new(),
        }
    );
}

#[test]
fn the_bpm_range_runs_from_the_slowest_to_the_fastest_track() {
    let lib = Lib::new();
    let tracks: Vec<(i64, i64)> = (0..4).map(|n| track(&lib, n, None)).collect();
    rekordbox(&lib, tracks[0].0, Some(126.0), None);
    rekordbox(&lib, tracks[1].0, Some(122.5), None);
    rekordbox(&lib, tracks[2].0, Some(128.0), None);
    // The fourth has none: it is in the count and nowhere else.
    let ids: Vec<i64> = tracks.iter().map(|t| t.0).collect();
    let id = crate_of(&lib, "Peak", &ids);
    let got = summary(&lib, id);
    assert_eq!(got.tracks, 4);
    assert_eq!(
        got.bpm,
        Some(BpmRange {
            min: 122.5,
            max: 128.0
        })
    );
}

#[test]
fn the_bpm_is_the_one_the_table_shows_rekordbox_first_then_the_tag_then_mixed_in_key() {
    let lib = Lib::new();
    let (a, _) = track(&lib, 0, None);
    let (b, _) = track(&lib, 1, None);
    let (c, _) = track(&lib, 2, None);
    rekordbox(&lib, a, Some(100.0), None);
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (?1, 'tag', 110)",
        (a,),
    );
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (?1, 'tag', 120)",
        (b,),
    );
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (?1, 'mik', 130)",
        (b,),
    );
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (?1, 'mik', 140)",
        (c,),
    );
    let id = crate_of(&lib, "Mixed", &[a, b, c]);
    // a shows 100, b shows 120, c shows 140.
    assert_eq!(
        summary(&lib, id).bpm,
        Some(BpmRange {
            min: 100.0,
            max: 140.0
        })
    );
    let table_shows = |recording: i64| {
        lib.writer
            .call(move |conn| {
                conn.query_row(
                    "SELECT bpm FROM track_row WHERE recording_id = ?1",
                    [recording],
                    |r| r.get::<_, f64>(0),
                )
            })
            .unwrap()
    };
    assert_eq!([a, b, c].map(table_shows), [100.0, 120.0, 140.0]);
}

#[test]
fn the_energy_is_the_one_the_overview_uses_user_then_mixed_in_key_then_local_then_the_audio_model()
{
    let lib = Lib::new();
    let tracks: Vec<i64> = (0..5).map(|n| track(&lib, n, None).0).collect();
    // 0: every source; the user's wins.
    energy(&lib, tracks[0], "ml", 9);
    energy(&lib, tracks[0], "local", 8);
    energy(&lib, tracks[0], "mik", 7);
    energy(&lib, tracks[0], "user", 2);
    // 1: no user; Mixed In Key's wins.
    energy(&lib, tracks[1], "ml", 9);
    energy(&lib, tracks[1], "local", 8);
    energy(&lib, tracks[1], "mik", 4);
    // 2: local over the audio model.
    energy(&lib, tracks[2], "ml", 10);
    energy(&lib, tracks[2], "local", 6);
    // 3: only the audio model.
    energy(&lib, tracks[3], "ml", 5);
    // 4: none.
    let id = crate_of(&lib, "Energy", &tracks);
    // The shown energies are 2, 4, 6 and 5.
    assert_eq!(
        summary(&lib, id).energy,
        Some(EnergyRange { min: 2, max: 6 })
    );
    // And the track whose energy the user sets last shows the user's.
    energy(&lib, tracks[4], "user", 10);
    assert_eq!(
        summary(&lib, id).energy,
        Some(EnergyRange { min: 2, max: 10 })
    );
}

#[test]
fn keys_are_each_listed_once_in_camelot_order() {
    let lib = Lib::new();
    let keys = ["12A", "8A", "1B", "8A", "1A", "12B", "8B"];
    let tracks: Vec<i64> = keys
        .iter()
        .enumerate()
        .map(|(n, key)| {
            let (recording, _) = track(&lib, n, None);
            rekordbox(&lib, recording, None, Some(key));
            recording
        })
        .collect();
    let id = crate_of(&lib, "Keys", &tracks);
    // 1A, 1B, 8A, 8B, 12A, 12B: by number, A before B, not by the text.
    assert_eq!(
        summary(&lib, id).keys,
        ["1A", "1B", "8A", "8B", "12A", "12B"]
    );
}

#[test]
fn the_top_genres_are_the_three_biggest_biggest_first_with_ties_by_name() {
    let lib = Lib::new();
    let genres = [
        ("House", 4),
        ("Techno", 3),
        ("Disco", 2),
        ("Ambient", 2),
        ("Dub", 1),
    ];
    let mut tracks = Vec::new();
    let mut n = 0;
    for (genre, count) in genres {
        for _ in 0..count {
            tracks.push(track(&lib, n, Some(genre)).0);
            n += 1;
        }
    }
    let id = crate_of(&lib, "Genres", &tracks);
    // Disco and Ambient tie on 2; the name decides, and only three are listed.
    assert_eq!(summary(&lib, id).genres, ["House", "Techno", "Ambient"]);
}

#[test]
fn genres_that_differ_only_in_capitals_count_together_spelled_as_the_first_track_has_it() {
    let lib = Lib::new();
    let tracks: Vec<i64> = [Some("deep house"), Some("Deep House"), Some("Techno"), None]
        .into_iter()
        .enumerate()
        .map(|(n, genre)| track(&lib, n, genre).0)
        .collect();
    let id = crate_of(&lib, "Genres", &tracks);
    // Two "deep house" against one "Techno"; the first track's spelling.
    assert_eq!(summary(&lib, id).genres, ["deep house", "Techno"]);
}

#[test]
fn the_genre_is_rekordbox_s_when_it_has_one_and_the_file_tag_otherwise() {
    let lib = Lib::new();
    let (tagged, _) = track(&lib, 0, Some("House"));
    let (known, _) = {
        let (recording, file) = lib.track(Some("Known"), Some("Kit"), "Music/known.mp3");
        lib.tag(
            file,
            &[("Name", "Known"), ("Artist", "Kit"), ("Genre", "Tag genre")],
        );
        let library = lib.library(recording, file);
        lib.rekordbox_entry(file, &[("Name", "Known"), ("Genre", "Rekordbox genre")]);
        (recording, library)
    };
    let id = crate_of(&lib, "Genres", &[tagged, known]);
    assert_eq!(summary(&lib, id).genres, ["House", "Rekordbox genre"]);
}

#[test]
fn a_part_with_no_data_is_left_out_and_the_others_stay() {
    let lib = Lib::new();
    // Tracks with a key and nothing else.
    let tracks: Vec<i64> = (0..3)
        .map(|n| {
            let (recording, _) = track(&lib, n, None);
            rekordbox(&lib, recording, None, Some("8A"));
            recording
        })
        .collect();
    let id = crate_of(&lib, "Keys only", &tracks);
    let got = summary(&lib, id);
    assert_eq!(got.tracks, 3);
    assert_eq!(got.bpm, None);
    assert_eq!(got.energy, None);
    assert_eq!(got.keys, ["8A"]);
    assert!(got.genres.is_empty());

    // Tracks with nothing at all: every part is left out.
    let blank: Vec<i64> = (10..12).map(|n| track(&lib, n, None).0).collect();
    let id = crate_of(&lib, "Blank", &blank);
    assert_eq!(
        summary(&lib, id),
        CrateSummary {
            notes: None,
            tracks: 2,
            bpm: None,
            energy: None,
            keys: Vec::new(),
            genres: Vec::new(),
        }
    );
}

#[test]
fn the_summary_is_of_the_crate_s_own_tracks_only() {
    let lib = Lib::new();
    let (inside, _) = track(&lib, 0, Some("House"));
    let (outside, _) = track(&lib, 1, Some("Techno"));
    rekordbox(&lib, inside, Some(124.0), Some("8A"));
    rekordbox(&lib, outside, Some(150.0), Some("3B"));
    let id = crate_of(&lib, "Mine", &[inside]);
    crate_of(&lib, "Other", &[outside]);
    let got = summary(&lib, id);
    assert_eq!(
        got.bpm,
        Some(BpmRange {
            min: 124.0,
            max: 124.0
        })
    );
    assert_eq!(got.keys, ["8A"]);
    assert_eq!(got.genres, ["House"]);
}

#[test]
fn the_summary_updates_after_tracks_are_added_and_removed() {
    let lib = Lib::new();
    let (a, a_library) = track(&lib, 0, Some("House"));
    let (b, _) = track(&lib, 1, Some("Techno"));
    rekordbox(&lib, a, Some(124.0), Some("8A"));
    rekordbox(&lib, b, Some(132.0), Some("9A"));
    energy(&lib, a, "user", 4);
    energy(&lib, b, "user", 9);
    let id = crate_of(&lib, "Growing", &[a]);

    let before = summary(&lib, id);
    assert_eq!(before.tracks, 1);
    assert_eq!(
        before.bpm,
        Some(BpmRange {
            min: 124.0,
            max: 124.0
        })
    );
    assert_eq!(before.energy, Some(EnergyRange { min: 4, max: 4 }));
    assert_eq!(before.keys, ["8A"]);
    assert_eq!(before.genres, ["House"]);

    add(&lib, id, &[b]);
    let after = summary(&lib, id);
    assert_eq!(after.tracks, 2);
    assert_eq!(
        after.bpm,
        Some(BpmRange {
            min: 124.0,
            max: 132.0
        })
    );
    assert_eq!(after.energy, Some(EnergyRange { min: 4, max: 9 }));
    assert_eq!(after.keys, ["8A", "9A"]);
    assert_eq!(after.genres, ["House", "Techno"]);

    lib.writer
        .call(move |c| Ok(remove_on(c, id, &[LibraryTrackId(a_library)])))
        .unwrap()
        .unwrap()
        .unwrap();
    let removed = summary(&lib, id);
    assert_eq!(removed.tracks, 1);
    assert_eq!(
        removed.bpm,
        Some(BpmRange {
            min: 132.0,
            max: 132.0
        })
    );
    assert_eq!(removed.genres, ["Techno"]);
}

#[test]
fn the_summary_updates_after_a_track_s_values_change() {
    let lib = Lib::new();
    let (a, _) = track(&lib, 0, Some("House"));
    rekordbox(&lib, a, Some(124.0), Some("8A"));
    energy(&lib, a, "mik", 5);
    let id = crate_of(&lib, "Changing", &[a]);
    assert_eq!(
        summary(&lib, id).bpm,
        Some(BpmRange {
            min: 124.0,
            max: 124.0
        })
    );

    // rekordbox says something else after the next read; the user sets an
    // energy; the file's genre tag is retagged.
    lib.insert(
        "UPDATE analysis SET bpm = 126, key = '9A' WHERE recording_id = ?1 AND source = 'rekordbox'",
        (a,),
    );
    energy(&lib, a, "user", 8);
    lib.insert(
        "UPDATE file SET raw_tags = replace(raw_tags, 'House', 'Disco')
         WHERE id IN (SELECT file_id FROM recording_file WHERE recording_id = ?1)",
        (a,),
    );
    lib.insert(
        "INSERT OR IGNORE INTO search_due (recording_id) VALUES (?1)",
        (a,),
    );
    let got = summary(&lib, id);
    assert_eq!(
        got.bpm,
        Some(BpmRange {
            min: 126.0,
            max: 126.0
        })
    );
    assert_eq!(got.keys, ["9A"]);
    assert_eq!(got.energy, Some(EnergyRange { min: 8, max: 8 }));
    assert_eq!(got.genres, ["Disco"]);
}

/// How long the summary takes at 10,000 tracks; run on its own to read the
/// time: `cargo test summary_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn summary_cost_at_10000_tracks() {
    let lib = Lib::new();
    lib.exec(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10000)
         INSERT INTO recording (title) SELECT 'Tune ' || i FROM n;
         INSERT INTO library_track (recording_id, source_status)
             SELECT id, 'missing' FROM recording;
         INSERT INTO analysis (recording_id, source, bpm, key)
             SELECT id, 'rekordbox', 90 + id % 60, (1 + id % 12) || (CASE id % 2 WHEN 0 THEN 'A' ELSE 'B' END)
             FROM recording;
         INSERT INTO analysis (recording_id, source, energy)
             SELECT id, 'user', 1 + id % 10 FROM recording;
         INSERT INTO crate (kind, name, position) VALUES ('static', 'Big', 0);
         INSERT INTO crate_entry (crate_id, library_track_id)
             SELECT 1, id FROM library_track;",
    );
    lib.settle();
    let started = std::time::Instant::now();
    let got = lib
        .writer
        .call(|c| summarize(c, &Mount, CrateId(1)))
        .unwrap()
        .unwrap();
    println!(
        "summary of {} tracks took {:?}",
        got.tracks,
        started.elapsed()
    );
    assert_eq!(got.tracks, 10_000);
}
