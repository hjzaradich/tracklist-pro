//! Tests for the hidden files. Everything is synthetic: made-up volume,
//! folders, files, titles and artists in a migrated database in a temp dir.

use super::*;
use crate::search::tests::support::{Lib, Mount};
use crate::track_table::{TableQuery, TableSource};

fn query() -> HiddenQuery {
    HiddenQuery {
        search: None,
        sort: None,
        offset: 0,
        limit: 100,
        with_total: true,
    }
}

fn listed(lib: &Lib, q: HiddenQuery) -> HiddenPage {
    lib.settle();
    lib.writer.call(move |c| page(c, &Mount, &q)).unwrap()
}

fn hide(lib: &Lib, file: i64, reason: &'static str) {
    lib.insert(
        "UPDATE file SET hidden_reason = ?1 WHERE id = ?2",
        (reason, file),
    );
}

fn reason_of(lib: &Lib, file: i64) -> Option<String> {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT hidden_reason FROM file WHERE id = ?1",
                [file],
                |r| r.get::<_, Option<String>>(0),
            )
        })
        .unwrap()
}

#[test]
fn every_reason_is_listed_with_its_own_way_back() {
    let lib = Lib::new();
    let (_, hidden_by_user) = lib.track(Some("Alpha"), Some("Artist A"), "alpha.mp3");
    let (_, cleared) = lib.track(Some("Bravo"), None, "bravo.mp3");
    let (copy_of, best) = lib.track(Some("Charlie"), None, "charlie.flac");
    let extra = lib.file(copy_of, "charlie (copy).mp3", "extra");
    lib.track(Some("Delta"), None, "delta.mp3");
    hide(&lib, hidden_by_user, "user");
    hide(&lib, cleared, "inbox_cleared");

    let page = listed(&lib, query());
    assert_eq!(page.total, Some(3));
    let rows: Vec<_> = page
        .rows
        .iter()
        .map(|r| (r.file_id, r.why, r.way_back))
        .collect();
    assert_eq!(
        rows,
        vec![
            (hidden_by_user, Why::UserHid, Some(WayBack::ShowAgain)),
            (cleared, Why::ClearedInbox, None),
            // "Use this file instead" is not built (see the module docs).
            (extra, Why::ExtraFile, None),
        ]
    );
    // An extra copy names its track and the file chosen as best.
    let extra_row = &page.rows[2];
    assert_eq!(extra_row.track, "Charlie");
    assert_eq!(extra_row.file, "charlie (copy).mp3");
    assert_eq!(extra_row.best_file.as_deref(), Some("charlie.flac"));
    let _ = best;
    // A file hidden by the user names its track and artist.
    assert_eq!(page.rows[0].track, "Alpha");
    assert_eq!(page.rows[0].artist.as_deref(), Some("Artist A"));
    assert_eq!(page.rows[0].best_file, None);
}

#[test]
fn a_hidden_extra_copy_is_listed_once_with_the_reason_the_user_gave() {
    let lib = Lib::new();
    let (track, _) = lib.track(Some("Echo"), None, "echo.flac");
    let extra = lib.file(track, "echo (copy).mp3", "extra");
    hide(&lib, extra, "user");

    let page = listed(&lib, query());
    assert_eq!(page.total, Some(1));
    assert_eq!(page.rows[0].why, Why::UserHid);
    assert_eq!(page.rows[0].way_back, Some(WayBack::ShowAgain));
}

#[test]
fn nothing_hidden_lists_nothing() {
    let lib = Lib::new();
    lib.track(Some("Foxtrot"), None, "foxtrot.mp3");
    let page = listed(&lib, query());
    assert_eq!(page.total, Some(0));
    assert!(page.rows.is_empty());
}

#[test]
fn show_again_clears_the_reason_and_undo_hides_the_file_again_exactly() {
    let lib = Lib::new();
    let (_, file) = lib.track(Some("Golf"), None, "golf.mp3");
    hide(&lib, file, "user");

    let shown = lib
        .writer
        .call(move |c| Ok(show_again(c, file)))
        .unwrap()
        .unwrap();
    assert!(shown.operation_id.is_some());
    assert_eq!(reason_of(&lib, file), None);
    assert_eq!(listed(&lib, query()).total, Some(0));

    let undone = lib.writer.call(|c| Ok(ops::undo_last(c))).unwrap().unwrap();
    assert!(matches!(undone, ops::UndoOutcome::Undone { .. }));
    assert_eq!(reason_of(&lib, file).as_deref(), Some("user"));
    let page = listed(&lib, query());
    assert_eq!(page.total, Some(1));
    assert_eq!(page.rows[0].way_back, Some(WayBack::ShowAgain));
}

#[test]
fn show_again_leaves_a_cleared_inbox_file_and_a_shown_file_alone() {
    let lib = Lib::new();
    let (_, cleared) = lib.track(Some("Hotel"), None, "hotel.mp3");
    let (_, shown) = lib.track(Some("India"), None, "india.mp3");
    hide(&lib, cleared, "inbox_cleared");

    for file in [cleared, shown, 9999] {
        let outcome = lib
            .writer
            .call(move |c| Ok(show_again(c, file)))
            .unwrap()
            .unwrap();
        assert_eq!(outcome.operation_id, None);
    }
    assert_eq!(reason_of(&lib, cleared).as_deref(), Some("inbox_cleared"));
}

#[test]
fn the_list_is_searched_by_track_and_sorted_by_title_artist_or_file() {
    let lib = Lib::new();
    let (_, a) = lib.track(Some("Juliet one"), Some("Zed"), "b.mp3");
    let (_, b) = lib.track(Some("Juliet two"), Some("Amy"), "a.mp3");
    let (_, c) = lib.track(Some("Kilo"), None, "c.mp3");
    for file in [a, b, c] {
        hide(&lib, file, "user");
    }
    let files = |page: &HiddenPage| page.rows.iter().map(|r| r.file_id).collect::<Vec<_>>();
    let sorted = |column, descending| HiddenQuery {
        sort: Some(TableSort { column, descending }),
        ..query()
    };

    assert_eq!(files(&listed(&lib, query())), vec![a, b, c]);
    let searched = listed(
        &lib,
        HiddenQuery {
            search: Some("juliet".into()),
            ..query()
        },
    );
    assert_eq!((searched.total, files(&searched)), (Some(2), vec![a, b]));
    // Blank artists last, whichever way.
    assert_eq!(
        files(&listed(&lib, sorted(SortColumn::Artist, false))),
        vec![b, a, c]
    );
    assert_eq!(
        files(&listed(&lib, sorted(SortColumn::Artist, true))),
        vec![a, b, c]
    );
    assert_eq!(
        files(&listed(&lib, sorted(SortColumn::File, false))),
        vec![b, a, c]
    );
}

#[test]
fn a_window_of_the_list_is_asked_for_by_offset_and_limit() {
    let lib = Lib::new();
    let mut files = Vec::new();
    for name in ["a", "b", "c", "d", "e"] {
        let (_, file) = lib.track(Some(name), None, &format!("{name}.mp3"));
        hide(&lib, file, "user");
        files.push(file);
    }
    let window = listed(
        &lib,
        HiddenQuery {
            offset: 2,
            limit: 2,
            with_total: false,
            ..query()
        },
    );
    assert_eq!(window.total, None);
    assert_eq!(
        window.rows.iter().map(|r| r.file_id).collect::<Vec<_>>(),
        files[2..4]
    );
}

#[test]
fn a_search_counts_only_hidden_files_whose_track_is_not_in_the_results_already() {
    let lib = Lib::new();
    // Its only file is hidden: not in the results.
    let (_, lone) = lib.track(Some("Lima solo"), None, "lima.mp3");
    hide(&lib, lone, "user");
    // A hidden file next to a shown one: the track is in the results.
    let (both, _) = lib.track(Some("Lima both"), None, "lima-both.flac");
    let extra = lib.file(both, "lima-both (copy).mp3", "extra");
    hide(&lib, extra, "user");
    // An extra copy of a shown track: in the results.
    let (copies, _) = lib.track(Some("Lima copies"), None, "lima-copies.flac");
    lib.file(copies, "lima-copies (copy).mp3", "extra");
    lib.track(Some("Mike"), None, "mike.mp3");
    lib.settle();

    let count = |text: &'static str| lib.writer.call(move |c| matches(c, text)).unwrap().count;
    assert_eq!(count("lima"), 1);
    assert_eq!(count("mike"), 0);
    // Text with no word to look for filters nothing, so it matches nothing.
    assert_eq!(count("  "), 0);
}

#[test]
fn the_all_music_table_and_the_overview_count_leave_out_hidden_files_the_same_way() {
    let lib = Lib::new();
    lib.track(Some("November"), None, "november.mp3");
    // Only hidden files: in neither.
    let (_, only) = lib.track(Some("Oscar"), None, "oscar.mp3");
    hide(&lib, only, "user");
    // One hidden file and one shown: in both.
    let (mixed, mixed_file) = lib.track(Some("Papa"), None, "papa.mp3");
    lib.file(mixed, "papa (copy).mp3", "extra");
    hide(&lib, mixed_file, "inbox_cleared");
    // A Library track whose file is hidden stays in the Library.
    let (quebec, quebec_file) = lib.track(Some("Quebec"), None, "quebec.mp3");
    lib.library(quebec, quebec_file);
    hide(&lib, quebec_file, "user");
    lib.settle();

    let in_table = |lib: &Lib| {
        let q = TableQuery {
            source: TableSource::AllMusic,
            search: None,
            filters: Vec::new(),
            sort: None,
            offset: 0,
            limit: 100,
            with_total: true,
        };
        lib.writer
            .call(move |c| {
                let found = crate::track_table::read(c, &Mount, &q)?.expect("All music exists");
                let page = crate::track_table::page(
                    found,
                    0,
                    &Mount,
                    &crate::fragile::FragileDirs {
                        downloads: None,
                        temp: Vec::new(),
                    },
                );
                Ok((page.total, page.rows.len()))
            })
            .unwrap()
    };
    let overview = |lib: &Lib| {
        lib.writer
            .call(|c| crate::overview::overview_at(c, &Mount, 0).map(|o| o.all_music))
            .unwrap()
    };

    assert_eq!(overview(&lib), 2);
    assert_eq!(in_table(&lib), (Some(2), 2));
    // The same after a file is shown again.
    lib.writer
        .call(move |c| Ok(show_again(c, only)))
        .unwrap()
        .unwrap();
    assert_eq!(overview(&lib), 3);
    assert_eq!(in_table(&lib), (Some(3), 3));
}
