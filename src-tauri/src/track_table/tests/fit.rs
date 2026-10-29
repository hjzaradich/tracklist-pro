//! A crate sorted by fit (`TableSource::CrateByFit`, 1cB-16): the order, the
//! reasons on the rows, and that a window, the count, a track's position and
//! the ids of a range all agree with that one order. The fit itself is
//! tested on its own (`crates::fit`).

use super::*;
use crate::crates::fit::{BpmFit, FitReason, KeyFit, TimeFeel};

/// What a track in the fixture is: a title, a BPM and a key (either may be
/// missing).
type Spec = (&'static str, Option<f64>, Option<&'static str>);

/// A Library crate (id 1) of these tracks, added in this order, and the
/// tracks' ids in the same order.
fn crate_of(lib: &Lib, specs: &[Spec]) -> Vec<i64> {
    lib.insert(
        "INSERT INTO crate (name, kind) VALUES ('Peak time', 'static')",
        (),
    );
    specs
        .iter()
        .enumerate()
        .map(|(n, (title, bpm, key))| {
            let name: &'static str = Box::leak(format!("{n}.mp3").into_boxed_str());
            let (rec, file) = lib.track(Some(title), Some("Kit"), name);
            if bpm.is_some() || key.is_some() {
                lib.insert(
                    "INSERT INTO analysis (recording_id, source, bpm, key)
                     VALUES (?1, 'rekordbox', ?2, ?3)",
                    (rec, *bpm, *key),
                );
            }
            let library_track = lib.library(rec, file);
            lib.insert(
                "INSERT INTO crate_entry (crate_id, library_track_id, kind, added_at)
                 VALUES (1, ?1, 'member', ?2)",
                (library_track, format!("2026-10-01T10:{n:02}:00.000Z")),
            );
            rec
        })
        .collect()
}

fn by_fit(after: i64) -> TableQuery {
    query(TableSource::CrateByFit {
        id: CrateId(1),
        after,
    })
}

fn position_in(lib: &Lib, q: &TableQuery, track: i64) -> Option<Option<u32>> {
    lib.settle();
    let asked = TablePositionQuery {
        source: q.source.clone(),
        search: q.search.clone(),
        filters: q.filters.clone(),
        sort: q.sort,
        recording_id: track,
    };
    lib.writer.call(move |c| position(c, &asked)).unwrap()
}

fn all_ids(lib: &Lib, q: &TableQuery) -> Option<Vec<i64>> {
    lib.settle();
    let q = q.clone();
    lib.writer
        .call(move |c| crate::track_table::ids(c, &q))
        .unwrap()
}

const TRACKS: [Spec; 6] = [
    ("Far key", Some(124.0), Some("3B")),
    ("Selected", Some(124.0), Some("8A")),
    ("Neighbor close", Some(125.5), Some("9A")),
    ("Same key far tempo", Some(131.0), Some("8A")),
    ("No key", Some(124.0), None),
    ("Nothing", None, None),
];

#[test]
fn a_crate_sorted_by_fit_lists_the_selected_track_first_then_the_best_fits() {
    let lib = Lib::new();
    let t = crate_of(&lib, &TRACKS);
    let page = run(&lib, by_fit(t[1]));
    assert_eq!(page.total, Some(6));
    // Selected; 9A at +1.5 (5 points); 8A at +7 (4); then the 124 BPM tracks
    // with no key and a far key (3 points each, in the order they were added
    // since their tempo difference is the same); then nothing to compare.
    assert_eq!(ids(&page), [t[1], t[2], t[3], t[0], t[4], t[5]]);
    assert_eq!(titles(&page)[..2], ["Selected", "Neighbor close"]);
}

#[test]
fn each_row_of_a_fit_sorted_crate_carries_its_dots_and_its_reason() {
    let lib = Lib::new();
    let t = crate_of(&lib, &TRACKS);
    let page = run(&lib, by_fit(t[1]));
    let fit = |track: i64| {
        page.rows
            .iter()
            .find(|r| r.recording_id == track)
            .and_then(|r| r.fit.clone())
            .expect("the row has a fit")
    };
    // The selected track has no fit to itself.
    assert_eq!(fit(t[1]).reason, None);
    let neighbor = fit(t[2]);
    assert_eq!(neighbor.dots, 3);
    assert_eq!(
        neighbor.reason,
        Some(FitReason {
            key: KeyFit::Neighbor {
                from: "8A".to_owned(),
                to: "9A".to_owned()
            },
            bpm: BpmFit::Delta {
                delta: 1.5,
                time: TimeFeel::Straight
            },
        })
    );
    assert_eq!(fit(t[3]).dots, 2);
    assert_eq!(
        fit(t[4]).reason.map(|r| r.key),
        Some(KeyFit::NoKey),
        "a track with no key says so"
    );
    let nothing = fit(t[5]).reason.unwrap();
    assert_eq!((nothing.key, nothing.bpm), (KeyFit::NoKey, BpmFit::NoBpm));
}

#[test]
fn rows_of_a_table_that_is_not_sorted_by_fit_carry_no_fit() {
    let lib = Lib::new();
    let t = crate_of(&lib, &TRACKS);
    for source in [
        TableSource::Crate { id: CrateId(1) },
        TableSource::Library,
        TableSource::AllMusic,
    ] {
        let page = run(&lib, query(source));
        assert!(page.rows.iter().all(|r| r.fit.is_none()));
    }
    // And the crate in its own order is the order the tracks were added.
    let normal = run(&lib, query(TableSource::Crate { id: CrateId(1) }));
    assert_eq!(ids(&normal), t);
}

#[test]
fn tracks_that_fit_equally_stay_in_the_order_they_were_added() {
    let lib = Lib::new();
    let same: [Spec; 5] = [
        ("A", Some(120.0), Some("5A")),
        ("Selected", Some(120.0), Some("5A")),
        ("B", Some(120.0), Some("5A")),
        ("C", Some(120.0), Some("5A")),
        ("D", Some(120.0), Some("5A")),
    ];
    let t = crate_of(&lib, &same);
    assert_eq!(ids(&run(&lib, by_fit(t[1]))), [t[1], t[0], t[2], t[3], t[4]]);
    // The same, whichever track is selected.
    assert_eq!(ids(&run(&lib, by_fit(t[3]))), [t[3], t[0], t[1], t[2], t[4]]);
}

#[test]
fn a_window_of_a_fit_sorted_crate_is_a_slice_of_the_whole_order_and_the_count_is_the_whole() {
    let lib = Lib::new();
    let t = crate_of(&lib, &TRACKS);
    let whole = ids(&run(&lib, by_fit(t[1])));
    for limit in [1, 2, 4, 10] {
        for offset in 0..7 {
            let page = run(
                &lib,
                TableQuery {
                    offset,
                    limit,
                    ..by_fit(t[1])
                },
            );
            let expected: Vec<i64> = whole
                .iter()
                .copied()
                .skip(offset as usize)
                .take(limit as usize)
                .collect();
            assert_eq!(ids(&page), expected, "offset {offset} limit {limit}");
            assert_eq!(page.total, Some(6));
            assert_eq!(page.offset, offset);
        }
    }
    // Without the count asked for, none is given.
    let uncounted = run(
        &lib,
        TableQuery {
            with_total: false,
            ..by_fit(t[1])
        },
    );
    assert_eq!(uncounted.total, None);
}

#[test]
fn a_track_s_position_and_the_ids_of_a_range_follow_the_fit_order() {
    let lib = Lib::new();
    let t = crate_of(&lib, &TRACKS);
    let q = by_fit(t[1]);
    let whole = ids(&run(&lib, q.clone()));
    for (index, &track) in whole.iter().enumerate() {
        assert_eq!(position_in(&lib, &q, track), Some(Some(index as u32)));
    }
    // The selected track is at the top.
    assert_eq!(position_in(&lib, &q, t[1]), Some(Some(0)));
    assert_eq!(all_ids(&lib, &q), Some(whole));
    // A track that is not in the crate is nowhere in it.
    let outside = lib.track(Some("Outside"), Some("Kit"), "outside.mp3").0;
    assert_eq!(position_in(&lib, &q, outside), Some(None));
}

#[test]
fn a_search_and_the_filters_narrow_a_fit_sorted_crate_without_changing_its_order() {
    let lib = Lib::new();
    let t = crate_of(&lib, &TRACKS);
    let q = TableQuery {
        search: Some("key".to_owned()),
        ..by_fit(t[1])
    };
    let page = run(&lib, q.clone());
    // "Far key", "Same key far tempo" and "No key" match; the selected track
    // does not, so it is not listed, but the order is still by fit to it.
    assert_eq!(ids(&page), [t[3], t[0], t[4]]);
    assert_eq!(page.total, Some(3));
    assert_eq!(position_in(&lib, &q, t[0]), Some(Some(1)));
    assert_eq!(all_ids(&lib, &q), Some(vec![t[3], t[0], t[4]]));
    assert!(page.rows.iter().all(|r| r.fit.is_some()));
}

#[test]
fn a_selected_track_with_no_key_or_tempo_leaves_the_crate_in_its_own_order_and_says_why() {
    let lib = Lib::new();
    let t = crate_of(&lib, &TRACKS);
    let page = run(&lib, by_fit(t[5]));
    assert_eq!(ids(&page), [t[5], t[0], t[1], t[2], t[3], t[4]]);
    for row in &page.rows[1..] {
        let fit = row.fit.clone().unwrap();
        assert_eq!(fit.dots, 0);
        let reason = fit.reason.unwrap();
        assert_eq!(reason.key, KeyFit::NoTargetKey);
        assert_eq!(reason.bpm, BpmFit::NoTargetBpm);
    }
}

#[test]
fn a_fit_sorted_crate_that_is_not_there_is_nothing_and_so_is_one_that_holds_no_tracks() {
    let lib = Lib::new();
    let t = crate_of(&lib, &TRACKS);
    let missing = TableQuery {
        source: TableSource::CrateByFit {
            id: CrateId(99),
            after: t[1],
        },
        ..by_fit(t[1])
    };
    lib.settle();
    let read_missing = lib
        .writer
        .call(move |c| Ok(read(c, &Mount, &missing)?.is_none()))
        .unwrap();
    assert!(read_missing);
    lib.insert(
        "INSERT INTO crate (name, kind) VALUES ('Empty', 'static')",
        (),
    );
    let empty = run(
        &lib,
        query(TableSource::CrateByFit {
            id: CrateId(2),
            after: t[1],
        }),
    );
    assert_eq!((empty.total, empty.rows.len()), (Some(0), 0));
}
