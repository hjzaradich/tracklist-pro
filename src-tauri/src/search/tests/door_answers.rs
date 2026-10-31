//! The doors' two stored answers (1cD-R1a; ROADMAP 1.13 "Every number is a
//! door"): a Library track's "low-quality file" bit and the energy it shows.
//! Every writer of what they read must reach the track's row once the index
//! has run, and only the tracks whose answer can change are made due. Writers
//! that need a bigger world are tested beside it: a merge in
//! `review/tests/same.rs`, a settled star conflict in
//! `conflicts/tests/energy_stars.rs`, a relink's new link in
//! `relink/tests/follow.rs`, the walk that hands a track's analysis over in
//! `grouping/tests`, an upgrade in `db/migrations/schema/track_row_doors.rs`.

use super::support::{Lib, Mount};
use crate::library::LibraryTrackId;
use crate::ops::{undo_for_user, UndoOutcome};
use crate::overview::{door_answers_of, DoorAnswer};
use crate::quality::store;
use crate::search::{flag, rebuild};

#[derive(Debug, Clone, Copy)]
struct Track {
    recording: i64,
    file: i64,
    library: i64,
}

/// A track whose file reads as a 128 kbps MP3, which the rules flag once a
/// verdict is made, with no verdict yet.
fn outside(lib: &Lib, name: &str) -> Track {
    let (recording, file) = lib.track(Some(name), Some("Someone"), name);
    lib.insert(
        "UPDATE file SET audio_hash = randomblob(16), codec = 'mp3', bitrate = 128,
                         sample_rate = 44100
         WHERE id = ?1",
        (file,),
    );
    Track {
        recording,
        file,
        library: 0,
    }
}

/// [`outside`], in the Library, with its row written.
fn in_library(lib: &Lib, name: &str) -> Track {
    let track = outside(lib, name);
    let library = lib.library(track.recording, track.file);
    lib.settle();
    Track { library, ..track }
}

/// [`in_library`], with the current verdict that flags it, and its row
/// holding that.
fn flagged(lib: &Lib, name: &str) -> Track {
    let track = in_library(lib, name);
    judge(lib);
    assert_eq!(row(lib, track), (true, 0), "the world of the test");
    track
}

/// Makes every stale verdict again, as the quality job does first.
fn judge(lib: &Lib) {
    lib.writer.call(store::settle).unwrap();
}

/// What the track's row holds, with the index caught up: the "low-quality
/// file" bit and the energy (0 for none).
fn row(lib: &Lib, track: Track) -> (bool, i64) {
    lib.settle();
    let flags: i64 = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT flags FROM track_row WHERE recording_id = ?1",
                [track.recording],
                |r| r.get(0),
            )
        })
        .unwrap();
    (flags & flag::LOW_QUALITY != 0, flag::energy_of(flags))
}

/// What the Overview's own definitions say of the Library track now.
fn definition(lib: &Lib, track: Track) -> DoorAnswer {
    lib.writer
        .call(move |c| door_answers_of(c, &[LibraryTrackId(track.library)]))
        .unwrap()
        .remove(&track.library)
        .unwrap()
}

/// The track is waiting for the index.
fn due(lib: &Lib, track: Track) -> bool {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT EXISTS (SELECT 1 FROM search_due WHERE recording_id = ?1)",
                [track.recording],
                |r| r.get(0),
            )
        })
        .unwrap()
}

fn undo(lib: &Lib) {
    let outcome = lib
        .writer
        .call(|c| Ok(undo_for_user(c, None)))
        .unwrap()
        .unwrap();
    assert!(
        matches!(outcome, UndoOutcome::Undone { .. }),
        "{outcome:?}"
    );
}

fn set_energy(lib: &Lib, tracks: &[Track], energy: i64) {
    let recordings: Vec<i64> = tracks.iter().map(|t| t.recording).collect();
    lib.writer
        .call(move |c| Ok(crate::energy::set_energy_on(c, &recordings, energy)))
        .unwrap()
        .unwrap();
}

// ---- low quality: the verdict ----------------------------------------------

#[test]
fn a_verdict_the_quality_job_makes_reaches_the_library_tracks_row() {
    let lib = Lib::new();
    let track = in_library(&lib, "low.mp3");
    assert_eq!(row(&lib, track), (false, 0), "no verdict yet is not flagged");
    judge(&lib);
    assert!(due(&lib, track), "a flagged verdict marks the track");
    assert_eq!(row(&lib, track), (true, 0));
}

#[test]
fn a_verdict_made_again_that_no_longer_flags_takes_the_bit_away() {
    let lib = Lib::new();
    let track = flagged(&lib, "low.mp3");
    // A better re-read: the verdict goes stale, then comes out `ok`.
    lib.insert("UPDATE file SET bitrate = 320 WHERE id = ?1", (track.file,));
    assert_eq!(row(&lib, track), (false, 0), "stale is no verdict");
    judge(&lib);
    assert_eq!(row(&lib, track), (false, 0));
    assert!(!definition(&lib, track).low_quality);
}

#[test]
fn a_verdict_dropped_takes_the_bit_away() {
    let lib = Lib::new();
    let track = flagged(&lib, "low.mp3");
    lib.insert("DELETE FROM file_verdict WHERE file_id = ?1", (track.file,));
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (false, 0));
}

// ---- low quality: what makes a verdict stale --------------------------------

#[test]
fn every_fact_a_verdict_reads_makes_it_stale_and_the_next_settling_brings_the_bit_back() {
    let changes: [(&str, &'static str); 5] = [
        ("bitrate", "UPDATE file SET bitrate = 129 WHERE id = ?1"),
        ("sample rate", "UPDATE file SET sample_rate = 48000 WHERE id = ?1"),
        ("codec", "UPDATE file SET codec = 'aac' WHERE id = ?1"),
        ("audio hash", "UPDATE file SET audio_hash = randomblob(16) WHERE id = ?1"),
        ("read verdict", "UPDATE file SET quality_verdict = 'truncated' WHERE id = ?1"),
    ];
    for (what, change) in changes {
        let lib = Lib::new();
        let track = flagged(&lib, "low.mp3");
        lib.insert(change, (track.file,));
        assert!(due(&lib, track), "{what}: marks the track");
        assert_eq!(row(&lib, track), (false, 0), "{what}: stale is no verdict");
        judge(&lib);
        let answer = definition(&lib, track);
        assert_eq!(row(&lib, track), (answer.low_quality, 0), "{what}: settled");
    }
}

#[test]
fn a_measurement_taken_replaced_or_dropped_makes_the_verdict_stale_and_the_settling_follows() {
    let lib = Lib::new();
    let track = flagged(&lib, "low.mp3");
    let version = i64::from(crate::quality::VERSION);
    // The ledger's two writes, and the row going with its file.
    lib.insert(
        "INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_hz, decoded_ms,
                                   header_ms, ended)
         SELECT id, audio_hash, ?2, 16000, 1000, 1000, 'complete' FROM file WHERE id = ?1",
        (track.file, version),
    );
    assert!(due(&lib, track), "taken");
    assert_eq!(row(&lib, track), (false, 0), "stale is no verdict");
    judge(&lib);
    assert_eq!(row(&lib, track), (true, 0), "settled, flagged again");

    lib.insert(
        "UPDATE file_quality SET cutoff_hz = 15000 WHERE file_id = ?1",
        (track.file,),
    );
    assert!(due(&lib, track), "replaced");
    assert_eq!(row(&lib, track), (false, 0));
    judge(&lib);
    assert_eq!(row(&lib, track), (true, 0));

    lib.insert("DELETE FROM file_quality WHERE file_id = ?1", (track.file,));
    assert!(due(&lib, track), "dropped");
    assert_eq!(row(&lib, track), (false, 0));
}

#[test]
fn a_new_rules_version_needs_no_mark_of_its_own_because_the_quality_job_makes_every_verdict_again()
{
    // A build with new rules writes no row: the verdicts of the old rules are
    // stale, and the stored bit still says what they said.
    let lib = Lib::new();
    let track = flagged(&lib, "low.mp3");
    lib.insert(
        "UPDATE file_verdict SET rules_version = rules_version + 1 WHERE file_id = ?1",
        (track.file,),
    );
    lib.exec("DELETE FROM search_due");
    assert_eq!(row(&lib, track), (true, 0), "nothing has told the row yet");
    // So the quality job is asked for, with nothing to measure ...
    assert!(lib
        .writer
        .call(|c| crate::quality::due(c, None))
        .unwrap());
    // ... and the verdict it makes again reaches the row, flagged or not.
    lib.insert("UPDATE file SET bitrate = 320 WHERE id = ?1", (track.file,));
    lib.exec("DELETE FROM search_due");
    judge(&lib);
    assert!(due(&lib, track), "the verdict that stopped flagging marks it");
    assert_eq!(row(&lib, track), (false, 0));
    assert!(
        !lib.writer
            .call(|c| crate::quality::due(c, None))
            .unwrap(),
        "nothing is asked for once the verdicts are made"
    );
}

#[test]
fn a_healthy_file_and_a_file_outside_the_library_mark_nothing_so_the_quality_job_costs_no_index_work(
) {
    let lib = Lib::new();
    // Healthy: its verdict, a stale one, its measurement.
    let healthy = in_library(&lib, "fine.mp3");
    lib.insert("UPDATE file SET bitrate = 320 WHERE id = ?1", (healthy.file,));
    lib.settle();
    judge(&lib);
    assert!(!lib.pending(), "an `ok` verdict");
    lib.insert("UPDATE file SET bitrate = 256 WHERE id = ?1", (healthy.file,));
    assert!(!lib.pending(), "a stale `ok` verdict");
    lib.insert(
        "INSERT INTO file_quality (file_id, audio_hash, method_version)
         SELECT id, audio_hash, 1 FROM file WHERE id = ?1",
        (healthy.file,),
    );
    assert!(!lib.pending(), "a measurement of a file no verdict flags");

    // Flagged, but no Library track links it.
    let stranger = outside(&lib, "stranger.mp3");
    lib.settle();
    judge(&lib);
    assert!(!lib.pending(), "a flagged verdict outside the Library");
    lib.insert(
        "UPDATE file SET bitrate = 127 WHERE id = ?1",
        (stranger.file,),
    );
    assert!(!lib.pending(), "its facts");

    // And facts written again as they are mark nothing.
    let track = flagged(&lib, "low.mp3");
    lib.insert(
        "UPDATE file SET codec = codec, bitrate = bitrate, sample_rate = sample_rate,
                         audio_hash = audio_hash, quality_verdict = quality_verdict
         WHERE id = ?1",
        (track.file,),
    );
    assert!(!lib.pending(), "a read that found the same");
}

// ---- energy -----------------------------------------------------------------

#[test]
fn an_energy_set_and_changed_reaches_the_row_and_undo_moves_it_back_each_time() {
    let lib = Lib::new();
    let track = in_library(&lib, "a.mp3");
    set_energy(&lib, &[track], 5);
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (false, 5));
    set_energy(&lib, &[track], 8);
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (false, 8));

    undo(&lib);
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (false, 5));
    undo(&lib);
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (false, 0));
}

#[test]
fn a_bulk_energy_reaches_every_row_and_one_undo_moves_them_all_back() {
    let lib = Lib::new();
    let tracks: Vec<Track> = ["a.mp3", "b.mp3", "c.mp3"]
        .map(|name| in_library(&lib, name))
        .to_vec();
    set_energy(&lib, &tracks[..1], 3);
    set_energy(&lib, &tracks, 9);
    assert_eq!(tracks.map_rows(&lib), [(false, 9); 3]);
    undo(&lib);
    assert_eq!(
        tracks.map_rows(&lib),
        [(false, 3), (false, 0), (false, 0)],
        "each back to what it had"
    );
}

trait Rows {
    fn map_rows(&self, lib: &Lib) -> Vec<(bool, i64)>;
}

impl Rows for Vec<Track> {
    fn map_rows(&self, lib: &Lib) -> Vec<(bool, i64)> {
        self.iter().map(|t| row(lib, *t)).collect()
    }
}

#[test]
fn the_row_holds_the_energy_of_the_source_that_wins_and_follows_each_source_row() {
    let lib = Lib::new();
    let track = in_library(&lib, "a.mp3");
    let add = |source: &'static str, level: i64| {
        lib.insert(
            "INSERT INTO analysis (recording_id, source, energy) VALUES (?1, ?2, ?3)",
            (track.recording, source, level),
        );
    };
    add("ml", 2);
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (false, 2));
    add("local", 4);
    assert_eq!(row(&lib, track), (false, 4));
    add("mik", 6);
    assert_eq!(row(&lib, track), (false, 6));
    set_energy(&lib, &[track], 10);
    assert_eq!(row(&lib, track), (false, 10), "the user's wins, up to 10");

    lib.insert(
        "UPDATE analysis SET energy = 7 WHERE recording_id = ?1 AND source = 'mik'",
        (track.recording,),
    );
    assert!(due(&lib, track), "a row that loses still marks: it costs a read");
    assert_eq!(row(&lib, track), (false, 10));
    lib.insert(
        "DELETE FROM analysis WHERE recording_id = ?1 AND source = 'user'",
        (track.recording,),
    );
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (false, 7), "the next source shows");
}

#[test]
fn an_energy_handed_to_another_track_leaves_one_row_and_reaches_the_other() {
    let lib = Lib::new();
    let (from, to) = (in_library(&lib, "a.mp3"), in_library(&lib, "b.mp3"));
    set_energy(&lib, &[from], 6);
    assert_eq!(row(&lib, from), (false, 6));
    // What the walk that empties a track does with its analysis.
    lib.insert(
        "UPDATE OR IGNORE analysis SET recording_id = ?2
         WHERE recording_id = ?1 AND source <> 'rekordbox'",
        (from.recording, to.recording),
    );
    assert!(due(&lib, from) && due(&lib, to));
    assert_eq!((row(&lib, from), row(&lib, to)), ((false, 0), (false, 6)));
}

#[test]
fn an_energy_outside_the_library_marks_nothing_and_stores_nothing() {
    let lib = Lib::new();
    let track = outside(&lib, "a.mp3");
    lib.settle();
    set_energy(&lib, &[track], 7);
    assert!(!lib.pending());
    assert_eq!(row(&lib, track), (false, 0), "only a Library track's row holds it");
}

// ---- the Library and its links ----------------------------------------------

#[test]
fn joining_and_leaving_the_library_bring_and_take_both_answers_and_undo_moves_them_back() {
    let lib = Lib::new();
    let track = outside(&lib, "a.mp3");
    set_energy(&lib, &[track], 7);
    judge(&lib);
    assert_eq!(row(&lib, track), (false, 0), "outside: none of it is stored");

    let library = lib.library(track.recording, track.file);
    let track = Track { library, ..track };
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (true, 7), "the Library track has them");

    crate::library::remove(&lib.writer, LibraryTrackId(library)).unwrap();
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (false, 0), "out of the Library, gone");

    undo(&lib);
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (true, 7), "the Undo brings them back");
}

#[test]
fn a_new_link_reads_the_new_files_verdict() {
    let lib = Lib::new();
    let track = flagged(&lib, "low.mp3");
    let better = lib.file(track.recording, "better.mp3", "duplicate");
    lib.insert(
        "UPDATE file SET audio_hash = randomblob(16), codec = 'flac', bitrate = 900,
                         sample_rate = 44100
         WHERE id = ?1",
        (better,),
    );
    judge(&lib);
    lib.insert(
        "UPDATE library_track SET linked_file_id = ?2 WHERE id = ?1",
        (track.library, better),
    );
    assert!(due(&lib, track));
    assert_eq!(row(&lib, track), (false, 0));
    lib.insert(
        "UPDATE library_track SET linked_file_id = ?2 WHERE id = ?1",
        (track.library, track.file),
    );
    assert_eq!(row(&lib, track), (true, 0));
}

// ---- what doesn't change them -----------------------------------------------

#[test]
fn a_rekordbox_read_keeps_both_answers_and_adds_its_own_flag() {
    let lib = Lib::new();
    let track = flagged(&lib, "low.mp3");
    set_energy(&lib, &[track], 4);
    assert_eq!(row(&lib, track), (true, 4));
    lib.rekordbox_entry(track.file, &[("Name", "low")]);
    assert!(lib.pending(), "a read marks every Library track");
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, 'rekordbox', 128.0, '8A')",
        (track.recording,),
    );
    assert_eq!(row(&lib, track), (true, 4));
    let flags: i64 = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT flags FROM track_row WHERE recording_id = ?1",
                [track.recording],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_ne!(flags & flag::IN_REKORDBOX, 0);
}

#[test]
fn every_library_row_holds_what_the_definitions_say_and_a_rebuild_comes_out_the_same() {
    let lib = Lib::new();
    let tracks = [
        flagged(&lib, "low.mp3"),
        in_library(&lib, "no verdict.mp3"),
        flagged(&lib, "stale.mp3"),
        flagged(&lib, "energy.mp3"),
    ];
    lib.insert("UPDATE file SET bitrate = 127 WHERE id = ?1", (tracks[2].file,));
    set_energy(&lib, &tracks[1..2], 1);
    set_energy(&lib, &tracks[3..], 10);
    let held = |lib: &Lib| -> Vec<(bool, i64)> {
        tracks.iter().map(|t| row(lib, *t)).collect()
    };
    let rows = held(&lib);
    assert_eq!(
        rows,
        [(true, 0), (false, 1), (false, 0), (true, 10)]
    );
    for (track, (low_quality, energy)) in tracks.iter().zip(&rows) {
        let answer = definition(&lib, *track);
        assert_eq!(answer.low_quality, *low_quality);
        assert_eq!(i64::from(answer.energy.unwrap_or(0)), *energy);
    }
    lib.writer.call(|c| rebuild(c, &Mount, 2)).unwrap();
    assert_eq!(held(&lib), rows, "the rows are derived data");
}
