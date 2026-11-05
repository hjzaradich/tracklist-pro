//! The Inbox's actions (ROADMAP 2.1): each is one operation, and one Undo
//! puts back everything it changed. Synthetic files: rows, no audio.

use rusqlite::OptionalExtension;

use super::*;
use crate::hidden::{show_again, use_this_file};
use crate::jobs::JobKind;
use crate::search::tests::support::Lib;

/// An Inbox item for `file`, in a batch of its own unless `batch` is given.
fn item(lib: &Lib, file: i64, batch: Option<i64>) -> i64 {
    let batch = batch.unwrap_or_else(|| new_batch(lib));
    lib.insert(
        "INSERT INTO inbox_item (file_id, batch_id) VALUES (?1, ?2)",
        (file, batch),
    )
}

fn new_batch(lib: &Lib) -> i64 {
    lib.insert(
        "INSERT INTO inbox_batch (music_folder_id, kind) VALUES (1, 'loose')",
        (),
    )
}

/// An item's status, and whether it has a decision time.
fn state(lib: &Lib, item: i64) -> (String, bool) {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT status, decided_at IS NOT NULL FROM inbox_item WHERE id = ?1",
                [item],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap()
}

fn new() -> (String, bool) {
    ("new".to_owned(), false)
}

fn decided(status: &str) -> (String, bool) {
    (status.to_owned(), true)
}

/// The linked file of `track`'s Library track, if it has one.
fn linked(lib: &Lib, track: i64) -> Option<i64> {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT linked_file_id FROM library_track WHERE recording_id = ?1",
                [track],
                |r| r.get(0),
            )
            .optional()
        })
        .unwrap()
}

fn reason_of(lib: &Lib, file: i64) -> Option<String> {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT hidden_reason FROM file WHERE id = ?1",
                [file],
                |r| r.get(0),
            )
        })
        .unwrap()
}

fn choices(lib: &Lib) -> i64 {
    lib.writer
        .call(|c| c.query_row("SELECT COUNT(*) FROM file_choice", [], |r| r.get(0)))
        .unwrap()
}

fn operations(lib: &Lib) -> i64 {
    lib.writer
        .call(|c| c.query_row("SELECT COUNT(*) FROM operation", [], |r| r.get(0)))
        .unwrap()
}

fn run(
    lib: &Lib,
    action: fn(&mut Connection, &[i64]) -> Result<InboxOutcome, OpsError>,
    items: &[i64],
) -> InboxOutcome {
    let items = items.to_vec();
    lib.writer
        .call(move |c| Ok(action(c, &items)))
        .unwrap()
        .unwrap()
}

fn add_items(lib: &Lib, items: &[i64]) -> InboxOutcome {
    let items = items.to_vec();
    lib.writer
        .call(move |c| Ok(add(c, &items, false)))
        .unwrap()
        .unwrap()
}

fn add_one(c: &mut Connection, items: &[i64]) -> Result<InboxOutcome, OpsError> {
    add(c, items, false)
}

fn is_done(outcome: &InboxOutcome) -> bool {
    matches!(
        outcome,
        InboxOutcome::Done {
            operation_id: Some(_),
            ..
        }
    )
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

/// The file's hashes: its audio, and its whole bytes.
fn hashes(lib: &Lib, file: i64, audio: u8, bytes: u8) {
    lib.insert(
        "UPDATE file SET audio_hash = ?1, blake3 = ?2 WHERE id = ?3",
        (vec![audio; 16], vec![bytes; 32], file),
    );
}

fn rank(lib: &Lib) {
    lib.writer
        .call(|c| crate::file_roles::settle_batch(c, 500))
        .unwrap();
}

#[test]
fn add_links_the_items_own_file_even_when_another_file_ranks_higher_and_undo_takes_it_back() {
    let lib = Lib::new();
    let (track, best) = lib.track(Some("Alpha"), None, "alpha.flac");
    let download = lib.file(track, "alpha (download).mp3", "extra");
    let it = item(&lib, download, None);
    assert_ne!(best, download);

    assert!(is_done(&add_items(&lib, &[it])));
    // ROADMAP 1.12: ranking again does not move a Library link to the best file.
    rank(&lib);
    assert_eq!(linked(&lib, track), Some(download));
    assert_eq!(state(&lib, it), decided("added"));

    undo(&lib);
    assert_eq!(linked(&lib, track), None);
    assert_eq!(state(&lib, it), new());
}

#[test]
fn add_is_refused_for_a_track_with_a_library_track_and_records_nothing() {
    let lib = Lib::new();
    let (track, linked_file) = lib.track(Some("Bravo"), None, "bravo.mp3");
    lib.library(track, linked_file);
    let download = lib.file(track, "bravo (download).flac", "extra");
    let it = item(&lib, download, None);
    let before = operations(&lib);

    assert_eq!(
        add_items(&lib, &[it]),
        InboxOutcome::Refused {
            item_id: it,
            reason: InboxRefusal::HasLibraryTrack
        }
    );
    assert_eq!(linked(&lib, track), Some(linked_file));
    assert_eq!(state(&lib, it), new());
    assert_eq!(operations(&lib), before);
}

#[test]
fn add_is_refused_when_rekordbox_has_the_track_as_another_file() {
    let lib = Lib::new();
    let (track, known) = lib.track(Some("Charlie"), None, "charlie.mp3");
    lib.trusted_entry(known, "House");
    let download = lib.file(track, "charlie (download).flac", "extra");
    let it = item(&lib, download, None);

    assert_eq!(
        add_items(&lib, &[it]),
        InboxOutcome::Refused {
            item_id: it,
            reason: InboxRefusal::InRekordboxAsAnotherFile
        }
    );
    assert_eq!(linked(&lib, track), None);
}

#[test]
fn add_all_adds_a_batch_as_one_operation_one_file_per_track() {
    let lib = Lib::new();
    let batch = new_batch(&lib);
    let (delta, delta_file) = lib.track(Some("Delta"), None, "delta.mp3");
    let delta_copy = lib.file(delta, "delta.wav", "extra");
    let (echo, echo_file) = lib.track(Some("Echo"), None, "echo.mp3");
    let (known, known_file) = lib.track(Some("Foxtrot"), None, "foxtrot.mp3");
    lib.library(known, known_file);
    let have = lib.file(known, "foxtrot (copy).mp3", "extra");
    let items: Vec<i64> = [delta_file, delta_copy, echo_file, have]
        .iter()
        .map(|f| item(&lib, *f, Some(batch)))
        .collect();

    let wanted = lib.writer.call(move |c| batch_to_add(c, batch)).unwrap();
    assert_eq!(wanted, vec![items[0], items[2]]);
    let before = operations(&lib);
    assert!(is_done(&add_items(&lib, &wanted)));
    assert_eq!(operations(&lib), before + 1);
    assert_eq!(linked(&lib, delta), Some(delta_file));
    assert_eq!(linked(&lib, echo), Some(echo_file));
    assert_eq!(state(&lib, items[1]), new());
    assert_eq!(state(&lib, items[3]), new());

    undo(&lib);
    assert_eq!((linked(&lib, delta), linked(&lib, echo)), (None, None));
    for it in [items[0], items[2]] {
        assert_eq!(state(&lib, it), new());
    }
}

#[test]
fn a_refused_item_leaves_the_whole_add_unwritten_and_names_that_item() {
    let lib = Lib::new();
    let (golf, golf_file) = lib.track(Some("Golf"), None, "golf.mp3");
    let (_, gone_file) = lib.track(Some("Hotel"), None, "hotel.mp3");
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (gone_file,));
    let ok = item(&lib, golf_file, None);
    let gone = item(&lib, gone_file, None);
    let before = operations(&lib);

    assert_eq!(
        add_items(&lib, &[ok, gone]),
        InboxOutcome::Refused {
            item_id: gone,
            reason: InboxRefusal::FileMissing
        }
    );
    assert_eq!(linked(&lib, golf), None);
    assert_eq!(state(&lib, ok), new());
    assert_eq!(operations(&lib), before);
    // A file of no track yet, and an item that isn't there.
    let loose = lib.insert(
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'x.mp3', 'x.mp3')",
        (),
    );
    let loose = item(&lib, loose, None);
    assert_eq!(
        run(&lib, add_one, &[loose]),
        InboxOutcome::refused(loose, InboxRefusal::NotIdentified)
    );
    assert_eq!(
        run(&lib, hide, &[9999]),
        InboxOutcome::refused(9999, InboxRefusal::ItemGone)
    );
    assert_eq!(operations(&lib), before);
}

#[test]
fn not_now_keeps_the_file_shown_and_undo_makes_the_item_new_again() {
    let lib = Lib::new();
    let (_, file) = lib.track(Some("India"), None, "india.mp3");
    let it = item(&lib, file, None);

    assert!(is_done(&run(&lib, not_now, &[it])));
    assert_eq!(state(&lib, it), decided("not_now"));
    assert_eq!(reason_of(&lib, file), None);
    // Asked again, nothing changes and nothing is recorded.
    assert_eq!(
        run(&lib, not_now, &[it]),
        InboxOutcome::Done {
            operation_id: None,
            items: Vec::new(),
            skipped: Vec::new()
        }
    );

    undo(&lib);
    assert_eq!(state(&lib, it), new());
}

#[test]
fn hide_of_a_chosen_file_ends_the_choice_and_undo_brings_both_back() {
    let lib = Lib::new();
    let (track, a) = lib.track(Some("Juliet"), None, "juliet.flac");
    let b = lib.file(track, "juliet (download).mp3", "extra");
    hashes(&lib, a, 1, 1);
    hashes(&lib, b, 2, 2);
    rank(&lib);
    lib.writer
        .call(move |c| Ok(use_this_file(c, b)))
        .unwrap()
        .unwrap();
    assert_eq!(choices(&lib), 1);
    let it = item(&lib, b, None);

    assert!(is_done(&run(&lib, hide, &[it])));
    assert_eq!(reason_of(&lib, b).as_deref(), Some("user"));
    assert_eq!(choices(&lib), 0);
    assert_eq!(state(&lib, it), decided("hidden"));

    undo(&lib);
    assert_eq!(reason_of(&lib, b), None);
    assert_eq!(choices(&lib), 1);
    assert_eq!(state(&lib, it), new());
}

#[test]
fn hide_leaves_the_choice_of_another_copy_alone_and_show_again_puts_the_item_back() {
    let lib = Lib::new();
    let (track, a) = lib.track(Some("Kilo"), None, "kilo.flac");
    let b = lib.file(track, "kilo (copy).flac", "extra");
    hashes(&lib, a, 1, 1);
    hashes(&lib, b, 1, 2);
    rank(&lib);
    lib.writer
        .call(move |c| Ok(use_this_file(c, b)))
        .unwrap()
        .unwrap();
    // `a` holds the same audio, other bytes: the choice names `b`.
    let it = item(&lib, a, None);

    assert!(is_done(&run(&lib, hide, &[it])));
    assert_eq!(choices(&lib), 1);
    lib.writer
        .call(move |c| Ok(show_again(c, a)))
        .unwrap()
        .unwrap();
    assert_eq!(reason_of(&lib, a), None);
    assert_eq!(state(&lib, it), new());
}

#[test]
fn clear_and_put_back_and_undo_of_each() {
    let lib = Lib::new();
    let (track, linked_file) = lib.track(Some("Lima"), None, "lima.mp3");
    lib.library(track, linked_file);
    let have = lib.file(track, "lima (again).mp3", "extra");
    let it = item(&lib, have, None);

    assert!(is_done(&run(&lib, clear, &[it])));
    assert_eq!(reason_of(&lib, have).as_deref(), Some("inbox_cleared"));
    assert_eq!(state(&lib, it), decided("hidden"));

    let put_back = lib
        .writer
        .call(move |c| Ok(show_again(c, have)))
        .unwrap()
        .unwrap();
    assert!(put_back.operation_id.is_some());
    assert_eq!(reason_of(&lib, have), None);
    assert_eq!(state(&lib, it), new());

    undo(&lib);
    assert_eq!(reason_of(&lib, have).as_deref(), Some("inbox_cleared"));
    assert_eq!(state(&lib, it), decided("hidden"));
    undo(&lib);
    assert_eq!(reason_of(&lib, have), None);
    assert_eq!(state(&lib, it), new());
}

#[test]
fn clear_is_refused_for_a_track_outside_the_library() {
    let lib = Lib::new();
    let (_, file) = lib.track(Some("Mike"), None, "mike.mp3");
    let it = item(&lib, file, None);
    assert_eq!(
        run(&lib, clear, &[it]),
        InboxOutcome::refused(it, InboxRefusal::NoLibraryTrack)
    );
    assert_eq!(reason_of(&lib, file), None);
    assert_eq!(state(&lib, it), new());
}

#[test]
fn an_add_asks_for_the_roles_and_the_waveform_jobs() {
    let lib = Lib::new();
    let (_, file) = lib.track(Some("November"), None, "november.mp3");
    hashes(&lib, file, 3, 3);
    lib.insert(
        "INSERT INTO file_stage (file_id, stage, version, status) VALUES (?1, 'hash', ?2, 'done')",
        (file, i64::from(crate::hash::DEFINITION)),
    );
    let it = item(&lib, file, None);
    assert!(is_done(&add_items(&lib, &[it])));

    let mut asked: Vec<JobKind> = Vec::new();
    ask_after_add(&lib.writer, None, |job| {
        asked.push(job.kind);
        Ok::<_, DbError>(JobId(asked.len() as i64))
    });
    assert!(asked.contains(&JobKind::Group), "{asked:?}");
    assert!(asked.contains(&JobKind::Waveform), "{asked:?}");
}

fn add_batch_of(lib: &Lib, batch: i64) -> InboxOutcome {
    lib.writer
        .call(move |c| Ok(add_batch(c, batch, false)))
        .unwrap()
        .unwrap()
}

fn library_tracks(lib: &Lib) -> i64 {
    lib.writer
        .call(|c| c.query_row("SELECT COUNT(*) FROM library_track", [], |r| r.get(0)))
        .unwrap()
}

#[test]
fn add_all_adds_what_can_be_added_and_lists_each_item_it_skipped_with_why() {
    let lib = Lib::new();
    let batch = new_batch(&lib);
    let (ok, ok_file) = lib.track(Some("Oscar"), None, "oscar.mp3");
    let (_, gone_file) = lib.track(Some("Papa"), None, "papa.mp3");
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (gone_file,));
    let loose_file = lib.insert(
        "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'q.mp3', 'q.mp3')",
        (),
    );
    let good = item(&lib, ok_file, Some(batch));
    let gone = item(&lib, gone_file, Some(batch));
    let loose = item(&lib, loose_file, Some(batch));
    let before = operations(&lib);

    let outcome = add_batch_of(&lib, batch);
    let InboxOutcome::Done {
        operation_id,
        items,
        skipped,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert!(operation_id.is_some());
    assert_eq!(items, vec![good]);
    assert_eq!(operations(&lib), before + 1);
    assert_eq!(linked(&lib, ok), Some(ok_file));
    let mut skipped: Vec<(i64, InboxRefusal)> =
        skipped.iter().map(|s| (s.item_id, s.reason)).collect();
    skipped.sort_by_key(|s| s.0);
    assert_eq!(
        skipped,
        vec![
            (gone, InboxRefusal::FileMissing),
            (loose, InboxRefusal::NotIdentified)
        ]
    );
    assert_eq!(state(&lib, gone), new());
    assert_eq!(state(&lib, loose), new());

    undo(&lib);
    assert_eq!(linked(&lib, ok), None);
    assert_eq!(state(&lib, good), new());
}

#[test]
fn add_all_with_nothing_addable_records_no_operation_and_lists_the_skipped() {
    let lib = Lib::new();
    let batch = new_batch(&lib);
    let (_, gone_file) = lib.track(Some("Quebec"), None, "quebec.mp3");
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (gone_file,));
    let gone = item(&lib, gone_file, Some(batch));
    let before = operations(&lib);

    assert_eq!(
        add_batch_of(&lib, batch),
        InboxOutcome::Done {
            operation_id: None,
            items: Vec::new(),
            skipped: vec![Skipped {
                item_id: gone,
                reason: InboxRefusal::FileMissing
            }]
        }
    );
    assert_eq!(operations(&lib), before);
}

#[test]
fn add_is_refused_while_the_rekordbox_match_cannot_be_confirmed() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Romeo"), None, "romeo.mp3");
    let entry = lib.rekordbox_entry(file, &[]);
    lib.insert(
        "UPDATE rekordbox_track SET relink_probable = 1 WHERE id = ?1",
        (entry,),
    );
    let it = item(&lib, file, None);
    assert_eq!(
        add_items(&lib, &[it]),
        InboxOutcome::refused(it, InboxRefusal::MatchNotConfirmed)
    );
    assert_eq!(linked(&lib, track), None);
}

#[test]
fn two_items_of_one_track_in_one_add_make_one_track_and_the_second_is_reported() {
    let lib = Lib::new();
    let (track, a) = lib.track(Some("Sierra"), None, "sierra.mp3");
    let b = lib.file(track, "sierra (copy).mp3", "extra");
    let first = item(&lib, a, None);
    let second = item(&lib, b, None);
    let before = operations(&lib);
    assert_eq!(
        add_items(&lib, &[first, second]),
        InboxOutcome::refused(second, InboxRefusal::SameTrackTwice)
    );
    assert_eq!(library_tracks(&lib), 0);
    assert_eq!(operations(&lib), before);
}

#[test]
fn a_low_bitrate_file_needs_the_go_ahead_and_is_added_with_it() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Tango"), None, "tango.mp3");
    lib.insert(
        "UPDATE file SET codec = 'mp3', bitrate = 128 WHERE id = ?1",
        (file,),
    );
    let it = item(&lib, file, None);
    let before = operations(&lib);

    assert_eq!(
        add_items(&lib, &[it]),
        InboxOutcome::NeedsConfirmation {
            item_id: it,
            warning: AddWarning::LowBitrate { kbps: 128 }
        }
    );
    assert_eq!(linked(&lib, track), None);
    assert_eq!(operations(&lib), before);

    let confirmed = lib
        .writer
        .call(move |c| Ok(add(c, &[it], true)))
        .unwrap()
        .unwrap();
    assert!(is_done(&confirmed));
    assert_eq!(linked(&lib, track), Some(file));
}

#[test]
fn adding_an_item_twice_is_refused_the_second_time_and_leaves_one_track() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Uniform"), None, "uniform.mp3");
    let it = item(&lib, file, None);
    assert!(is_done(&add_items(&lib, &[it])));
    let before = operations(&lib);
    assert_eq!(
        add_items(&lib, &[it]),
        InboxOutcome::refused(it, InboxRefusal::AlreadyDecided)
    );
    assert_eq!(library_tracks(&lib), 1);
    assert_eq!(linked(&lib, track), Some(file));
    assert_eq!(operations(&lib), before);
}

#[test]
fn an_add_cannot_be_undone_out_of_order_after_a_later_operation() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Victor"), None, "victor.mp3");
    let (_, other) = lib.track(Some("Whiskey"), None, "whiskey.mp3");
    let it = item(&lib, file, None);
    let later = item(&lib, other, None);
    let InboxOutcome::Done {
        operation_id: Some(add_id),
        ..
    } = add_items(&lib, &[it])
    else {
        panic!("not added");
    };
    assert!(is_done(&run(&lib, not_now, &[later])));

    let undone = lib
        .writer
        .call(move |c| Ok(ops::undo_for_user(c, Some(add_id))))
        .unwrap()
        .unwrap();
    assert!(
        matches!(undone, ops::UndoOutcome::NothingToUndo),
        "{undone:?}"
    );
    assert_eq!(linked(&lib, track), Some(file));
    assert_eq!(state(&lib, it), decided("added"));
}

#[test]
fn hide_and_clear_refuse_a_file_that_is_a_library_track_own_linked_file() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Xray"), None, "xray.mp3");
    let it = item(&lib, file, None);
    lib.library(track, file);
    for action in [hide, clear] {
        assert_eq!(
            run(&lib, action, &[it]),
            InboxOutcome::refused(it, InboxRefusal::IsLibraryFile)
        );
    }
    assert_eq!(reason_of(&lib, file), None);
    assert_eq!(state(&lib, it), new());
}

#[test]
fn follow_up_jobs_are_asked_for_only_when_something_was_added() {
    let done_with = |operation_id| InboxOutcome::Done {
        operation_id,
        items: Vec::new(),
        skipped: Vec::new(),
    };
    let outcomes = [
        (InboxOutcome::refused(1, InboxRefusal::FileMissing), 0),
        (
            InboxOutcome::NeedsConfirmation {
                item_id: 1,
                warning: AddWarning::LowBitrate { kbps: 128 },
            },
            0,
        ),
        (done_with(None), 0),
        (done_with(Some(7)), 1),
    ];
    for (outcome, asked) in outcomes {
        let mut calls = 0;
        let back = follow_up_if_added(outcome.clone(), || calls += 1);
        assert_eq!(back, outcome);
        assert_eq!(calls, asked, "{outcome:?}");
    }
}
