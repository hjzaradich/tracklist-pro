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
            items: Vec::new()
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
