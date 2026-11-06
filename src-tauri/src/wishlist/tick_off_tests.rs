//! The auto tick-off (ROADMAP 2.5) over synthetic rows: made-up names, no
//! audio. Grouping is stood in for by a `grouping_file` row.

use super::*;
use crate::inbox::actions::InboxRefusal;
use crate::search::tests::support::Lib;
use crate::settings::{write_feature_enabled, Feature};
use crate::wishlist::create_from_paste;
use crate::wishlist::matching::keys;

fn lib_on() -> Lib {
    let lib = Lib::new();
    switch(&lib, true);
    lib
}

fn switch(lib: &Lib, on: bool) {
    lib.writer
        .call(move |c| write_feature_enabled(c, Feature::Wishlist, on))
        .unwrap();
}

/// A new Inbox file of a track of its own, decided by grouping: its
/// track, file and item. `kind` is its batch's.
fn arrived_as(lib: &Lib, artist: &str, title: &str, kind: &'static str) -> (i64, i64, i64) {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (track, file) = lib.track(Some(title), Some(artist), &format!("dl {n}.mp3"));
    lib.insert(
        "INSERT INTO grouping_file (file_id, decided_as) VALUES (?1, 1)",
        [file],
    );
    let batch = lib.insert(
        "INSERT INTO inbox_batch (music_folder_id, kind) VALUES (1, ?1)",
        [kind],
    );
    let item = lib.insert(
        "INSERT INTO inbox_item (file_id, batch_id) VALUES (?1, ?2)",
        (file, batch),
    );
    (track, file, item)
}

fn arrived(lib: &Lib, artist: &str, title: &str) -> (i64, i64, i64) {
    arrived_as(lib, artist, title, "loose")
}

fn wishlist(lib: &Lib, name: &'static str, pasted: &'static str) -> i64 {
    let outcome = lib
        .writer
        .call(move |c| Ok(create_from_paste(c, name, pasted)))
        .unwrap()
        .unwrap();
    match outcome {
        WishlistOutcome::Done {
            wishlist_id: Some(id),
            ..
        } => id,
        other => panic!("{other:?}"),
    }
}

fn pass(lib: &Lib) -> usize {
    lib.writer.call(run).unwrap()
}

fn status(lib: &Lib, item: i64) -> String {
    lib.writer
        .call(move |c| {
            c.query_row("SELECT status FROM inbox_item WHERE id = ?1", [item], |r| {
                r.get(0)
            })
        })
        .unwrap()
}

/// Each wish item of `list`: its acquired file, and whether it has a time.
fn acquired(lib: &Lib, list: i64) -> Vec<(Option<i64>, bool)> {
    lib.writer
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT acquired_file_id, acquired_at IS NOT NULL FROM wish_item
                 WHERE wishlist_id = ?1 ORDER BY position",
            )?;
            let rows = stmt.query_map([list], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap()
}

fn in_library(lib: &Lib, track: i64) -> bool {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT EXISTS (SELECT 1 FROM library_track WHERE recording_id = ?1)",
                [track],
                |r| r.get(0),
            )
        })
        .unwrap()
}

fn count(lib: &Lib, sql: &'static str) -> i64 {
    lib.writer
        .call(move |c| c.query_row(sql, [], |r| r.get(0)))
        .unwrap()
}

fn undo(lib: &Lib) -> ops::UndoOutcome {
    lib.writer
        .call(|c| Ok(ops::undo_for_user(c, None)))
        .unwrap()
        .unwrap()
}

fn wish_of(lib: &Lib, file: i64) -> Option<WishFor> {
    lib.writer
        .call(move |c| crate::wishlist::matching_wish_for(c, file))
        .unwrap()
}

fn entries(lib: &Lib) -> Vec<DoneForYou> {
    lib.writer.call(|c| done_for_you(c)).unwrap()
}

const NOT: (Option<i64>, bool) = (None, false);

#[test]
fn the_joining_point_names_the_item_a_file_answers_and_nothing_while_wishlists_are_off() {
    let lib = lib_on();
    let list = wishlist(
        &lib,
        "Friday",
        "Quill Ashby - Paper Harbor\nNova Reed - Lantern",
    );
    let (_, file, _) = arrived(&lib, "Nova Reed", "Lantern");
    let wish = wish_of(&lib, file).expect("a match");
    assert_eq!(wish.wishlist_id, list);
    assert_eq!(wish.result.reason.key(), keys::OWNED_ALL_MUSIC);

    switch(&lib, false);
    assert_eq!(wish_of(&lib, file), None);
    assert_eq!(pass(&lib), 0);
    assert_eq!(entries(&lib), Vec::new());
    assert_eq!(
        count(
            &lib,
            "SELECT count(*) FROM operation WHERE kind = 'wishlist_tick_off'"
        ),
        0
    );
}

#[test]
fn an_owned_match_is_added_and_ticked_off_as_one_operation_and_undo_puts_the_file_back_as_new() {
    let lib = lib_on();
    let list = wishlist(
        &lib,
        "Friday",
        "Quill Ashby - Paper Harbor\nNova Reed - Lantern",
    );
    let (track, file, item) = arrived(&lib, "Nova Reed", "Lantern");
    let ops_before = count(&lib, "SELECT count(*) FROM operation");

    assert_eq!(pass(&lib), 1);
    assert!(in_library(&lib, track));
    assert_eq!(status(&lib, item), "added");
    assert_eq!(acquired(&lib, list), vec![NOT, (Some(file), true)]);
    assert_eq!(
        count(&lib, "SELECT count(*) FROM operation"),
        ops_before + 1
    );

    let shown = entries(&lib);
    assert_eq!(shown.len(), 1);
    assert!(shown[0].added);
    assert_eq!(
        (
            shown[0].inbox_item_id,
            shown[0].wishlist_name.as_str(),
            shown[0].wish_title.as_str()
        ),
        (item, "Friday", "Lantern")
    );
    assert_eq!(shown[0].reason.key(), keys::OWNED_LIBRARY);

    match undo(&lib) {
        ops::UndoOutcome::Undone { operation } => assert_eq!(operation.kind, OPERATION),
        other => panic!("{other:?}"),
    }
    assert!(!in_library(&lib, track));
    assert_eq!(status(&lib, item), "new");
    assert_eq!(acquired(&lib, list), vec![NOT, NOT]);

    // Undone stays undone: the next pass leaves it, and shows the match.
    assert_eq!(pass(&lib), 0);
    assert_eq!(status(&lib, item), "new");
    let shown = entries(&lib);
    assert_eq!(shown.len(), 1);
    assert!(!shown[0].added);
    assert_eq!(shown[0].refusal, None);
}

#[test]
fn another_cut_or_a_rework_is_never_ticked_off() {
    let lib = lib_on();
    let list = wishlist(
        &lib,
        "Friday",
        "Quill Ashby - Paper Harbor (Extended Mix)\nNova Reed - Lantern",
    );
    let (_, cut, cut_item) = arrived(&lib, "Quill Ashby", "Paper Harbor (Radio Edit)");
    let (_, rework, rework_item) = arrived(&lib, "Nova Reed", "Lantern (Dune Moth Remix)");
    assert_eq!(pass(&lib), 0);
    assert_eq!(
        (status(&lib, cut_item), status(&lib, rework_item)),
        ("new".into(), "new".into())
    );
    assert_eq!(acquired(&lib, list), vec![NOT, NOT]);
    assert_eq!((wish_of(&lib, cut), wish_of(&lib, rework)), (None, None));
    assert_eq!(entries(&lib), Vec::new());
}

#[test]
fn an_ambiguous_match_does_nothing_automatic() {
    // One file answers two items (the track is on two lists).
    let lib = lib_on();
    let first = wishlist(&lib, "Friday", "Nova Reed - Lantern");
    let second = wishlist(&lib, "Saturday", "Nova Reed - Lantern");
    let (_, file, item) = arrived(&lib, "Nova Reed", "Lantern");
    assert_eq!(pass(&lib), 0);
    assert_eq!(status(&lib, item), "new");
    assert_eq!(
        (acquired(&lib, first), acquired(&lib, second)),
        (vec![NOT], vec![NOT])
    );
    assert_eq!(wish_of(&lib, file), None);

    // Two files answer one item.
    let lib = lib_on();
    let list = wishlist(&lib, "Friday", "Quill Ashby - Paper Harbor");
    let (_, one, one_item) = arrived(&lib, "Quill Ashby", "Paper Harbor");
    let (_, two, two_item) = arrived(&lib, "Quill Ashby", "Paper Harbor");
    assert_eq!(pass(&lib), 0);
    assert_eq!(
        (status(&lib, one_item), status(&lib, two_item)),
        ("new".into(), "new".into())
    );
    assert_eq!(acquired(&lib, list), vec![NOT]);
    assert_eq!((wish_of(&lib, one), wish_of(&lib, two)), (None, None));
}

#[test]
fn a_match_the_add_rules_refuse_is_left_and_shown_with_the_reason() {
    let lib = lib_on();
    let list = wishlist(&lib, "Friday", "Nova Reed - Lantern");
    let (track, _, item) = arrived(&lib, "Nova Reed", "Lantern");
    let older = lib.file(track, "lantern old.mp3", "extra");
    lib.library(track, older);
    assert_eq!(pass(&lib), 0);
    assert_eq!(status(&lib, item), "new");
    assert_eq!(acquired(&lib, list), vec![NOT]);
    let shown = entries(&lib);
    assert_eq!(shown.len(), 1);
    assert!(!shown[0].added);
    assert_eq!(shown[0].refusal, Some(InboxRefusal::HasLibraryTrack));
}

#[test]
fn a_low_bitrate_file_is_never_ticked_off_automatically() {
    let lib = lib_on();
    let list = wishlist(&lib, "Friday", "Nova Reed - Lantern");
    let (_, file, item) = arrived(&lib, "Nova Reed", "Lantern");
    lib.insert(
        "UPDATE file SET codec = 'mp3', bitrate = 128 WHERE id = ?1",
        [file],
    );
    let before = count(&lib, "SELECT count(*) FROM operation");
    assert_eq!(pass(&lib), 0);
    assert_eq!(status(&lib, item), "new");
    assert_eq!(acquired(&lib, list), vec![NOT]);
    assert_eq!(count(&lib, "SELECT count(*) FROM operation"), before);
    let shown = entries(&lib);
    assert_eq!(shown.len(), 1);
    assert!(!shown[0].added);
    assert_eq!(shown[0].refusal, None);
}

#[test]
fn many_files_are_asked_about_in_one_batch_with_the_same_answers_as_one_at_a_time() {
    let lib = lib_on();
    wishlist(
        &lib,
        "Friday",
        "Nova Reed - Lantern
Quill Ashby - Paper Harbor",
    );
    let (_, one, _) = arrived(&lib, "Nova Reed", "Lantern");
    let (_, two, _) = arrived(&lib, "Quill Ashby", "Paper Harbor");
    let (_, three, _) = arrived(&lib, "Mara Voss", "Unlisted");
    let batch = lib
        .writer
        .call(move |c| crate::wishlist::matching_wishes_for(c, &[one, two, three]))
        .unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch.get(&one), wish_of(&lib, one).as_ref());
    assert_eq!(batch.get(&two), wish_of(&lib, two).as_ref());
    assert_eq!(batch.get(&three), None);
}

#[test]
fn only_new_arrivals_in_a_decided_track_are_looked_at() {
    let lib = lib_on();
    let list = wishlist(
        &lib,
        "Friday",
        "Nova Reed - Lantern\nQuill Ashby - Paper Harbor",
    );
    // Already in the folder at its first look.
    let (_, _, baseline) = arrived_as(&lib, "Nova Reed", "Lantern", "already_there");
    // Not decided by grouping yet.
    let (_, file, item) = arrived(&lib, "Quill Ashby", "Paper Harbor");
    lib.insert(
        "UPDATE grouping_file SET decided_as = NULL WHERE file_id = ?1",
        [file],
    );
    assert_eq!(pass(&lib), 0);
    assert_eq!(
        (status(&lib, baseline), status(&lib, item)),
        ("new".into(), "new".into())
    );
    lib.insert(
        "UPDATE grouping_file SET decided_as = 1 WHERE file_id = ?1",
        [file],
    );
    assert_eq!(pass(&lib), 1);
    assert_eq!(status(&lib, item), "added");
    assert_eq!(acquired(&lib, list), vec![NOT, (Some(file), true)]);
}

#[test]
fn bad_data_in_one_file_never_stops_the_others() {
    let lib = lib_on();
    let list = wishlist(
        &lib,
        "Friday",
        "Nova Reed - Lantern\nQuill Ashby - Paper Harbor",
    );
    let (_, broken, _) = arrived(&lib, "Nova Reed", "Lantern");
    lib.insert(
        r#"UPDATE file SET raw_tags = '{"id3v2": "oops", "ape": [1, {"key": 5, "value": null}]}'
           WHERE id = ?1"#,
        [broken],
    );
    let (_, file, item) = arrived(&lib, "Quill Ashby", "Paper Harbor");
    assert_eq!(pass(&lib), 1);
    assert_eq!(status(&lib, item), "added");
    assert_eq!(acquired(&lib, list)[1], (Some(file), true));
}

fn crates(lib: &Lib) -> Vec<(i64, String, i64)> {
    lib.writer
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT c.id, c.name, count(e.id) FROM crate c
                 LEFT JOIN crate_entry e ON e.crate_id = c.id GROUP BY c.id ORDER BY c.id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect()
        })
        .unwrap()
}

fn crate_on(lib: &Lib, list: i64, on: bool) {
    let outcome = lib
        .writer
        .call(move |c| set_auto_crate(c, list, on))
        .unwrap();
    assert!(
        matches!(outcome, WishlistOutcome::Done { .. }),
        "{outcome:?}"
    );
    assert_eq!(lib.writer.call(move |c| auto_crate(c, list)).unwrap(), on);
}

#[test]
fn the_wishlists_crate_is_off_by_default_and_when_on_is_made_or_reused_in_the_same_operation() {
    let lib = lib_on();
    let list = wishlist(
        &lib,
        "Friday",
        "Nova Reed - Lantern\nQuill Ashby - Paper Harbor",
    );
    assert!(!lib.writer.call(move |c| auto_crate(c, list)).unwrap());
    arrived(&lib, "Nova Reed", "Lantern");
    assert_eq!(pass(&lib), 1);
    assert_eq!(crates(&lib), Vec::new());

    // On: a crate named after the list is made, in the tick-off itself.
    crate_on(&lib, list, true);
    arrived(&lib, "Quill Ashby", "Paper Harbor");
    assert_eq!(pass(&lib), 1);
    assert_eq!(crates(&lib), vec![(1, "Friday".to_owned(), 1)]);
    assert!(matches!(undo(&lib), ops::UndoOutcome::Undone { .. }));
    assert_eq!(crates(&lib), Vec::new());

    // A crate of that name (by the send's rule) is there: it is reused.
    let lib = lib_on();
    let list = wishlist(&lib, "Friday", "Nova Reed - Lantern");
    crate_on(&lib, list, true);
    lib.writer
        .call(|c| Ok(crate::crates::create_on(c, "friday ")))
        .unwrap()
        .unwrap()
        .unwrap();
    let made = crates(&lib);
    arrived(&lib, "Nova Reed", "Lantern");
    assert_eq!(pass(&lib), 1);
    assert_eq!(crates(&lib), vec![(made[0].0, made[0].1.clone(), 1)]);
    assert!(matches!(undo(&lib), ops::UndoOutcome::Undone { .. }));
    assert_eq!(crates(&lib), vec![(made[0].0, made[0].1.clone(), 0)]);
}

#[test]
fn the_crate_switch_refuses_while_wishlists_are_off_or_for_no_such_list() {
    let lib = lib_on();
    let refused = |lib: &Lib, list: i64| {
        lib.writer
            .call(move |c| set_auto_crate(c, list, true))
            .unwrap()
    };
    assert_eq!(
        refused(&lib, 99),
        WishlistOutcome::Refused {
            reason: WishlistRefusal::NoSuchWishlist
        }
    );
    let list = wishlist(&lib, "Friday", "Nova Reed - Lantern");
    switch(&lib, false);
    assert_eq!(
        refused(&lib, list),
        WishlistOutcome::Refused {
            reason: WishlistRefusal::NotTurnedOn
        }
    );
}
