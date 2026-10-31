//! Setting energy (1cB-8). Everything is synthetic: made-up tracks in a
//! migrated database in a temp dir.

use super::*;
use crate::search::tests::support::Lib;

/// Three tracks outside the Library; their ids.
fn three() -> (Lib, [i64; 3]) {
    let lib = Lib::new();
    let ids = [("One", "a.mp3"), ("Two", "b.mp3"), ("Three", "c.mp3")]
        .map(|(title, name)| lib.track(Some(title), Some("Ann"), name).0);
    (lib, ids)
}

fn set(lib: &Lib, tracks: &[i64], energy: i64) -> Result<EnergySet, Refused> {
    let tracks = tracks.to_vec();
    lib.writer
        .call(move |c| Ok(set_energy_on(c, &tracks, energy)))
        .unwrap()
}

/// The energy a track shows, and the source it comes from.
fn shown(lib: &Lib, track: i64) -> Option<(i64, String)> {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT energy, energy_source FROM recording_display WHERE recording_id = ?1",
                [track],
                |r| {
                    Ok(r.get::<_, Option<i64>>(0)?
                        .zip(r.get::<_, Option<String>>(1)?))
                },
            )
        })
        .unwrap()
}

fn operations(lib: &Lib) -> i64 {
    lib.writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM operation WHERE kind = ?1",
                [SET_ENERGY_OPERATION],
                |r| r.get(0),
            )
        })
        .unwrap()
}

fn undo(lib: &Lib) {
    let undone = lib
        .writer
        .call(|c| Ok(ops::undo_for_user(c, None)))
        .unwrap()
        .unwrap();
    assert!(
        matches!(undone, ops::UndoOutcome::Undone { .. }),
        "{undone:?}"
    );
}

fn energies(lib: &Lib, tracks: &[i64]) -> Vec<Option<i64>> {
    tracks
        .iter()
        .map(|&t| shown(lib, t).map(|(energy, _)| energy))
        .collect()
}

#[test]
fn every_track_given_gets_the_energy_as_one_operation() {
    let (lib, tracks) = three();
    let done = set(&lib, &tracks, 7).unwrap();
    assert_eq!((done.changed, done.skipped), (3, 0));
    assert!(done.operation_id.is_some());
    assert_eq!(operations(&lib), 1);
    for track in tracks {
        assert_eq!(shown(&lib, track), Some((7, "user".to_owned())));
    }
}

#[test]
fn one_undo_puts_every_track_back_as_it_was() {
    let (lib, tracks) = three();
    // One track already has an energy of its own, one has Mixed In Key's.
    set(&lib, &tracks[..1], 3).unwrap();
    lib.insert(
        "INSERT INTO analysis (recording_id, source, energy) VALUES (2, 'mik', 5)",
        (),
    );
    assert_eq!(energies(&lib, &tracks), [Some(3), Some(5), None]);

    set(&lib, &tracks, 9).unwrap();
    assert_eq!(energies(&lib, &tracks), [Some(9), Some(9), Some(9)]);

    undo(&lib);
    assert_eq!(energies(&lib, &tracks), [Some(3), Some(5), None]);
    assert_eq!(shown(&lib, tracks[1]), Some((5, "mik".to_owned())));
    // The earlier operation is the next one to undo, and it is whole too.
    undo(&lib);
    assert_eq!(energies(&lib, &tracks), [None, Some(5), None]);
}

#[test]
fn the_users_energy_is_shown_before_every_other_sources() {
    let (lib, tracks) = three();
    lib.insert(
        "INSERT INTO analysis (recording_id, source, energy) VALUES (1, 'mik', 4)",
        (),
    );
    lib.insert(
        "INSERT INTO analysis (recording_id, source, energy) VALUES (1, 'local', 6)",
        (),
    );
    set(&lib, &tracks[..1], 2).unwrap();
    assert_eq!(shown(&lib, tracks[0]), Some((2, "user".to_owned())));
}

#[test]
fn a_track_that_has_that_energy_is_left_alone_and_a_track_listed_twice_counts_once() {
    let (lib, tracks) = three();
    set(&lib, &tracks[..1], 6).unwrap();
    let done = set(&lib, &[tracks[0], tracks[1], tracks[1], tracks[2]], 6).unwrap();
    assert_eq!((done.changed, done.skipped), (2, 1));
    assert_eq!(operations(&lib), 2);
    // It changed the others only: Undo takes back those two, not the first track's.
    undo(&lib);
    assert_eq!(energies(&lib, &tracks), [Some(6), None, None]);
}

#[test]
fn when_nothing_changes_nothing_is_recorded() {
    let (lib, tracks) = three();
    set(&lib, &tracks, 4).unwrap();
    let again = set(&lib, &tracks, 4).unwrap();
    assert_eq!(
        (again.changed, again.skipped, again.operation_id),
        (0, 3, None)
    );
    assert_eq!(operations(&lib), 1);
    let none = set(&lib, &[], 4).unwrap();
    assert_eq!(
        (none.changed, none.skipped, none.operation_id),
        (0, 0, None)
    );
}

#[test]
fn a_track_that_is_gone_is_skipped_and_the_rest_are_set() {
    let (lib, tracks) = three();
    let done = set(&lib, &[tracks[0], 9_999, tracks[2]], 8).unwrap();
    assert_eq!((done.changed, done.skipped), (2, 1));
    assert_eq!(energies(&lib, &tracks), [Some(8), None, Some(8)]);
}

#[test]
fn only_energies_from_one_to_ten_are_accepted_and_nothing_is_changed_by_a_refusal() {
    let (lib, tracks) = three();
    for energy in [0, 11, -1, 100] {
        assert!(
            matches!(set(&lib, &tracks, energy), Err(Refused::NotAnEnergy(e)) if e == energy),
            "{energy}"
        );
    }
    assert_eq!(operations(&lib), 0);
    assert_eq!(energies(&lib, &tracks), [None, None, None]);
    for energy in [1, 10] {
        assert!(set(&lib, &tracks[..1], energy).is_ok(), "{energy}");
    }
}

#[test]
fn what_a_send_would_write_for_a_library_track_is_the_same_before_and_after_its_energy_is_set() {
    use crate::library::LibraryTrackId;
    use crate::search::tests::support::Mount;
    use crate::send_values::{send_values_one, Outcome};

    let lib = Lib::new();
    let (track, file) = lib.track(Some("One"), Some("Ann"), "a.mp3");
    let library_track = lib.library(track, file);
    let values = |lib: &Lib| {
        lib.writer
            .call(move |c| send_values_one(c, &Mount, LibraryTrackId(library_track)))
            .unwrap()
    };
    let before = values(&lib);
    let Outcome::Ready(ready) = &before.outcome else {
        panic!("the track can be sent: {before:?}");
    };
    // The send has values to write, and energy is not among them.
    assert!(!ready.values.is_empty());
    assert!(ready
        .values
        .iter()
        .all(|v| !v.attribute.to_lowercase().contains("energy")));

    set(&lib, &[track], 5).unwrap();
    assert_eq!(shown(&lib, track), Some((5, "user".to_owned())));
    assert_eq!(values(&lib), before);
}

#[test]
fn an_energy_outside_the_library_does_not_make_the_search_index_work_again() {
    // (A Library track's does: its row holds the energy it shows,
    // `search/tests/door_answers.rs`.)
    let (lib, tracks) = three();
    lib.settle();
    assert!(!lib.pending());
    set(&lib, &tracks, 5).unwrap();
    assert!(!lib.pending());
    undo(&lib);
    assert!(!lib.pending());
}

/// 1cC-M1: setting an energy on 100,000 selected tracks (one operation), what
/// asking "what would Undo do" costs after it (`next_undo`) and its Undo.
/// Half the tracks are in the Library and a quarter already have an energy of
/// their own, so both the insert and the update paths run. Prints; asserts
/// nothing about time. Run alone, in a release build:
/// `cargo test --release --lib setting_energy_on_100000 -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn setting_energy_on_100000_tracks_and_undoing_it_cost() {
    use std::time::Instant;
    let mib = crate::ops::history_tests::peak_working_set_mib;
    let lib = Lib::new();
    lib.writer
        .call(|c| {
            c.execute_batch(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000)
                 INSERT INTO recording (title) SELECT 'Tune ' || i FROM n;
                 INSERT INTO library_track (recording_id, source_status)
                     SELECT id, 'missing' FROM recording WHERE id % 2 = 0;
                 INSERT INTO analysis (recording_id, source, energy)
                     SELECT id, 'user', 1 + id % 10 FROM recording WHERE id % 4 = 0;",
            )
        })
        .unwrap();
    println!("peak memory after building the library: {} MiB", mib());
    let all: Vec<i64> = (1..=100_000).collect();
    // The first index run, so what is timed below is the catching up. The
    // pages are the job's.
    let catch_up = || {
        let started = Instant::now();
        lib.writer
            .call(|c| crate::search::settle(c, &crate::search::tests::support::Mount, 500))
            .unwrap();
        started.elapsed()
    };
    println!("the first index run: {:?}", catch_up());
    let started = Instant::now();
    let done = set(&lib, &all, 7).unwrap();
    println!(
        "set energy on 100000 tracks: {:?} (changed {}, skipped {}); peak {} MiB",
        started.elapsed(),
        done.changed,
        done.skipped,
        mib()
    );
    let due: i64 = lib
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM search_due", [], |r| r.get(0)))
        .unwrap();
    println!(
        "tracks marked for the index: {due}, caught up in {:?}",
        catch_up()
    );
    let started = Instant::now();
    let next = lib.writer.call(|c| Ok(ops::next_undo(c))).unwrap().unwrap();
    println!(
        "next_undo after it: {:?} (refusal {:?})",
        started.elapsed(),
        next.refusal
    );
    let started = Instant::now();
    undo(&lib);
    println!("Undo of it: {:?}; peak {} MiB", started.elapsed(), mib());
    // 1cD-R1a: the Library's half is marked for the index job, which brings
    // the stored energy of their rows up to date.
    println!("the index catching up after the Undo: {:?}", catch_up());
    // The same again from a clean slate for the second figure: setting an
    // energy on the tracks that all have it already changes nothing.
    set(&lib, &all, 7).unwrap();
    let started = Instant::now();
    let again = set(&lib, &all, 7).unwrap();
    println!(
        "set the same energy again: {:?} (changed {})",
        started.elapsed(),
        again.changed
    );
}
