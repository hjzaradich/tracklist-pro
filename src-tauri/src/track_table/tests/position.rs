//! Where a track is in the table (`position`, 1cA-5): the index the table
//! scrolls to when a sort or a filter changes and a track was selected. It
//! must be the index the track has in the rows a window gives, whatever the
//! sort, ties and blanks, so each test compares the two.

use super::*;

/// Seventeen tracks whose titles, artists, BPMs, keys and lengths tie and
/// are blank in turns; every other one is in the Library, and those are in a
/// crate in an order of their own.
fn library_with_ties_and_blanks() -> Lib {
    let lib = Lib::new();
    lib.insert(
        "INSERT INTO crate (name, kind) VALUES ('Warm up', 'static')",
        (),
    );
    for n in 0..17_usize {
        let name: &'static str = Box::leak(format!("{n}.mp3").into_boxed_str());
        let title = format!("Tune {}", n % 4);
        let artist = if n % 5 == 0 { "" } else { ["Ann", "Bo"][n % 2] };
        let (rec, file) = lib.track(Some(&title), Some(artist), name);
        lib.tag(
            file,
            &[
                ("Name", &title),
                ("Artist", artist),
                ("Genre", ["House", "", "Techno"][n % 3]),
            ],
        );
        if n % 3 != 0 {
            analysis(
                &lib,
                rec,
                100.0 + (n % 4) as f64,
                if n % 2 == 0 { "8A" } else { "9B" },
            );
        }
        if n % 4 != 0 {
            duration(&lib, file, 60_000 + (n as i64 % 3) * 1000);
        }
        lib.insert(
            "UPDATE file SET sniffed_format = ?1 WHERE id = ?2",
            (["mp3", "flac"][n % 2], file),
        );
        if n % 2 == 0 {
            let library_track = lib.library(rec, file);
            // Added in an order that is not the order of the ids.
            lib.insert(
                "INSERT INTO crate_entry (crate_id, library_track_id, kind, added_at)
                 VALUES (1, ?1, 'member', ?2)",
                (
                    library_track,
                    format!("2026-10-01T10:{:02}:00.000Z", (n * 7) % 11),
                ),
            );
        }
    }
    lib
}

const COLUMNS: [SortColumn; 10] = [
    SortColumn::Title,
    SortColumn::Artist,
    SortColumn::Bpm,
    SortColumn::Key,
    SortColumn::Time,
    SortColumn::Genre,
    SortColumn::Rating,
    SortColumn::DateAdded,
    SortColumn::File,
    SortColumn::Folder,
];

fn position_of(lib: &Lib, q: &TableQuery, recording_id: i64) -> Option<Option<u32>> {
    let asked = TablePositionQuery {
        source: q.source.clone(),
        search: q.search.clone(),
        filters: q.filters.clone(),
        sort: q.sort,
        recording_id,
    };
    lib.writer.call(move |c| position(c, &asked)).unwrap()
}

/// Every track of the library, listed by `q` and not: the listed ones are at
/// the index the window gave them, the others have none.
fn assert_agrees_with_the_window(lib: &Lib, q: TableQuery) {
    let q = TableQuery {
        limit: MAX_WINDOW,
        offset: 0,
        ..q
    };
    let listed = ids(&run(lib, q.clone()));
    for (index, &id) in listed.iter().enumerate() {
        assert_eq!(
            position_of(lib, &q, id),
            Some(Some(index as u32)),
            "track {id} of {q:?}"
        );
    }
    let everyone = ids(&run(
        lib,
        TableQuery {
            source: TableSource::AllMusic,
            search: None,
            filters: Vec::new(),
            sort: None,
            ..q.clone()
        },
    ));
    for id in everyone.into_iter().filter(|id| !listed.contains(id)) {
        assert_eq!(position_of(lib, &q, id), Some(None), "track {id} of {q:?}");
    }
}

#[test]
fn a_tracks_position_is_the_index_its_row_has_in_every_sort_both_ways_with_ties_and_blanks() {
    let lib = library_with_ties_and_blanks();
    for source in [
        TableSource::AllMusic,
        TableSource::Library,
        TableSource::Crate { id: CrateId(1) },
    ] {
        // The lists are not empty, so the comparison below has rows to compare.
        let listed = run(&lib, query(source.clone())).total;
        assert_eq!(
            listed,
            Some(if source == TableSource::AllMusic {
                17
            } else {
                9
            })
        );
        assert_agrees_with_the_window(&lib, query(source.clone()));
        for column in COLUMNS {
            for descending in [false, true] {
                assert_agrees_with_the_window(&lib, sorted(source.clone(), column, descending));
            }
        }
    }
}

#[test]
fn a_tracks_position_is_the_index_its_row_has_through_a_search_and_filters_too() {
    let lib = library_with_ties_and_blanks();
    let flac = TableFilter::Format {
        format: "flac".to_owned(),
    };
    for source in [TableSource::AllMusic, TableSource::Crate { id: CrateId(1) }] {
        for (search, filters) in [
            (Some("tune"), vec![]),
            (Some("tune 1"), vec![]),
            (None, vec![flac.clone()]),
            (Some("ann"), vec![flac.clone()]),
            (None, vec![TableFilter::InCrate]),
        ] {
            for sort in [
                None,
                Some(TableSort {
                    column: SortColumn::Bpm,
                    descending: true,
                }),
                Some(TableSort {
                    column: SortColumn::Genre,
                    descending: false,
                }),
                Some(TableSort {
                    column: SortColumn::Artist,
                    descending: true,
                }),
            ] {
                assert_agrees_with_the_window(
                    &lib,
                    TableQuery {
                        search: search.map(str::to_owned),
                        filters: filters.clone(),
                        sort,
                        ..query(source.clone())
                    },
                );
            }
        }
    }
}

#[test]
fn a_track_the_source_does_not_list_has_no_position() {
    let f = fixture();
    f.lib.settle();
    let q = query(TableSource::Library);
    // Alpha Wave (3) is in All music only; a search can leave a track out.
    assert_eq!(position_of(&f.lib, &q, f.recs[2]), Some(None));
    let searched = TableQuery {
        search: Some("delta".to_owned()),
        ..q.clone()
    };
    assert_eq!(position_of(&f.lib, &searched, f.recs[0]), Some(Some(0)));
    assert_eq!(position_of(&f.lib, &searched, f.recs[3]), Some(None));
    // Nor has a track nobody has heard of.
    assert_eq!(position_of(&f.lib, &q, 9_999), Some(None));
}

#[test]
fn a_crate_that_does_not_exist_has_no_positions_at_all() {
    let f = fixture();
    f.lib.settle();
    let q = query(TableSource::Crate { id: CrateId(99) });
    assert_eq!(position_of(&f.lib, &q, f.recs[0]), None);
}
