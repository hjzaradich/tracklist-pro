//! Setting energy (1cB-8). Everything is synthetic: made-up tracks in a
//! migrated database in a temp dir.

use super::*;
use crate::search::tests::support::Lib;

/// Three tracks outside the Library; their ids.
fn three() -> (Lib, [i64; 3]) {
    let lib = Lib::new();
    let ids = [("One", "a.mp3"), ("Two", "b.mp3"), ("Three", "c.mp3")].map(|(title, name)| {
        lib.track(Some(title), Some("Ann"), name).0
    });
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
                |r| Ok(r.get::<_, Option<i64>>(0)?.zip(r.get::<_, Option<String>>(1)?)),
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
    assert!(matches!(undone, ops::UndoOutcome::Undone { .. }), "{undone:?}");
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
    assert_eq!((again.changed, again.skipped, again.operation_id), (0, 3, None));
    assert_eq!(operations(&lib), 1);
    let none = set(&lib, &[], 4).unwrap();
    assert_eq!((none.changed, none.skipped, none.operation_id), (0, 0, None));
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
fn setting_the_energy_of_a_library_track_writes_nothing_a_send_reads() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("One"), Some("Ann"), "a.mp3");
    lib.library(track, file);
    set(&lib, &[track], 5).unwrap();
    let (bases, values): (i64, i64) = lib
        .writer
        .call(|c| {
            Ok((
                c.query_row("SELECT count(*) FROM sync_base", [], |r| r.get(0))?,
                c.query_row("SELECT count(*) FROM app_value", [], |r| r.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!((bases, values), (0, 0));
    assert_eq!(shown(&lib, track), Some((5, "user".to_owned())));
}

#[test]
fn an_energy_alone_does_not_make_the_search_index_work_again() {
    let (lib, tracks) = three();
    lib.settle();
    assert!(!lib.pending());
    set(&lib, &tracks, 5).unwrap();
    assert!(!lib.pending());
    undo(&lib);
    assert!(!lib.pending());
}
