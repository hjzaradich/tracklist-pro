//! The ids of a range of rows (`ids`, 1cB-8): what the table asks for to
//! select a range or all of its tracks. They must be the ids the windows
//! give, in the same order, whatever the sort, ties and blanks.

use super::*;

/// Twelve tracks whose titles tie and whose artists and BPMs are blank in
/// turns; every third one is in the Library.
fn tied_and_blank() -> Lib {
    let lib = Lib::new();
    for n in 0..12_usize {
        let name: &'static str = Box::leak(format!("{n}.mp3").into_boxed_str());
        let title = format!("Tune {}", n % 3);
        let artist = if n % 4 == 0 { "" } else { ["Ann", "Bo"][n % 2] };
        let (rec, file) = lib.track(Some(&title), Some(artist), name);
        if n % 2 == 1 {
            analysis(&lib, rec, 100.0 + (n % 5) as f64, "8A");
        }
        if n % 3 == 0 {
            lib.library(rec, file);
        }
    }
    lib
}

fn ids_of(lib: &Lib, q: &TableQuery) -> Option<Vec<i64>> {
    lib.settle();
    let q = q.clone();
    lib.writer
        .call(move |c| crate::track_table::ids(c, &q))
        .unwrap()
}

const SORTS: [SortColumn; 5] = [
    SortColumn::Title,
    SortColumn::Artist,
    SortColumn::Bpm,
    SortColumn::File,
    SortColumn::DateAdded,
];

#[test]
fn the_ids_of_a_range_are_the_ids_of_the_same_rows_in_a_window_in_every_sort_both_ways() {
    let lib = tied_and_blank();
    for source in [TableSource::AllMusic, TableSource::Library] {
        let listed = run(&lib, query(source.clone()));
        assert!(listed.rows.len() >= 4, "the comparison has rows to compare");
        for column in SORTS {
            for descending in [false, true] {
                let q = sorted(source.clone(), column, descending);
                let whole = ids(&run(&lib, q.clone()));
                assert_eq!(ids_of(&lib, &q), Some(whole.clone()), "{q:?}");
                // A range in the middle is the same rows.
                let middle = TableQuery {
                    offset: 2,
                    limit: 3,
                    with_total: false,
                    ..q.clone()
                };
                assert_eq!(ids_of(&lib, &middle), Some(whole[2..5].to_vec()), "{q:?}");
            }
        }
    }
}

#[test]
fn the_ids_follow_the_search_and_the_filters_the_table_has() {
    let lib = tied_and_blank();
    let q = TableQuery {
        search: Some("Tune 1".to_owned()),
        ..query(TableSource::AllMusic)
    };
    let found = ids(&run(&lib, q.clone()));
    assert!(!found.is_empty() && found.len() < 12);
    assert_eq!(ids_of(&lib, &q), Some(found));

    let q = filtered(TableSource::AllMusic, vec![TableFilter::InLibrary]);
    let in_library = ids(&run(&lib, q.clone()));
    assert_eq!(in_library.len(), 4);
    assert_eq!(ids_of(&lib, &q), Some(in_library));
}

#[test]
fn a_list_longer_than_a_window_comes_whole_in_one_answer() {
    let lib = Lib::new();
    let count = MAX_WINDOW as usize + 20;
    for n in 0..count {
        let name: &'static str = Box::leak(format!("{n}.mp3").into_boxed_str());
        lib.track(Some("Same title"), None, name);
    }
    let q = TableQuery {
        limit: MAX_IDS,
        ..query(TableSource::AllMusic)
    };
    let all = ids_of(&lib, &q).unwrap();
    assert_eq!(all.len(), count);
    // A window stops at its own limit; the ids don't, and are the same order.
    let first_window = ids(&run(&lib, TableQuery { limit: MAX_WINDOW, ..q.clone() }));
    assert_eq!(first_window.len(), MAX_WINDOW as usize);
    assert_eq!(all[..first_window.len()], first_window[..]);
    // A range beyond the first window is the rest.
    let rest = TableQuery {
        offset: MAX_WINDOW,
        ..q
    };
    assert_eq!(ids_of(&lib, &rest).unwrap(), all[MAX_WINDOW as usize..]);
}

#[test]
fn asking_for_ids_does_not_count_the_rows_or_need_them_counted() {
    let lib = tied_and_blank();
    let counted = TableQuery {
        with_total: true,
        ..query(TableSource::AllMusic)
    };
    let uncounted = TableQuery {
        with_total: false,
        ..counted.clone()
    };
    assert_eq!(ids_of(&lib, &counted), ids_of(&lib, &uncounted));
}

#[test]
fn a_crate_that_does_not_exist_has_no_ids() {
    let lib = tied_and_blank();
    assert_eq!(
        ids_of(&lib, &query(TableSource::Crate { id: CrateId(99) })),
        None
    );
}
