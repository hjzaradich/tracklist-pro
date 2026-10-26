//! Tests for how the table is ordered and what the index job writes for it:
//! descending sorts read the ascending index backwards, a deleted track's
//! row goes, and a row that came out the same is not written again.

use super::*;

/// The ids of the window of `q`, as the plain `ORDER BY` of the query gives
/// them (it sorts the rows and reads no index backwards).
fn plain_window(lib: &Lib, q: &TableQuery) -> Vec<i64> {
    let q = q.clone();
    lib.writer
        .call(move |c| {
            let built = build(&q, None);
            select_ids(
                c,
                &built.from_where,
                &built.order_by,
                &built.params,
                q.limit,
                q.offset,
            )
        })
        .unwrap()
}

/// Seventeen tracks whose titles, artists, BPMs, keys and lengths tie and
/// are blank in turns.
fn tied_and_blank_tracks() -> Lib {
    let lib = Lib::new();
    for n in 0..17_usize {
        let name: &'static str = Box::leak(format!("{n}.mp3").into_boxed_str());
        let title = format!("Tune {}", n % 4);
        let artist = if n % 5 == 0 { "" } else { ["Ann", "Bo"][n % 2] };
        let (rec, file) = lib.track(Some(&title), Some(artist), name);
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
    }
    lib
}

#[test]
fn a_descending_sort_gives_the_order_of_a_plain_sort_in_every_window_whether_it_starts_among_the_values_or_the_blanks(
) {
    let lib = tied_and_blank_tracks();
    lib.settle();
    for column in [
        SortColumn::Artist,
        SortColumn::Genre,
        SortColumn::Bpm,
        SortColumn::Key,
        SortColumn::Time,
        SortColumn::Rating,
        SortColumn::Title,
    ] {
        for limit in [1, 3, 7, 40] {
            for offset in 0..19 {
                let q = TableQuery {
                    offset,
                    limit,
                    ..sorted(TableSource::AllMusic, column, true)
                };
                let planned = lib
                    .writer
                    .call({
                        let q = q.clone();
                        move |c| Ok(window_ids(c, &q)?.0)
                    })
                    .unwrap();
                assert_eq!(
                    planned,
                    plain_window(&lib, &q),
                    "{column:?} offset {offset} limit {limit}"
                );
            }
        }
    }
}

#[test]
fn a_descending_sort_is_the_ascending_one_backwards_with_the_blanks_moved_to_the_end() {
    let lib = tied_and_blank_tracks();
    for column in [
        SortColumn::Artist,
        SortColumn::Bpm,
        SortColumn::Time,
        SortColumn::Title,
    ] {
        let up = ids(&run(&lib, sorted(TableSource::AllMusic, column, false)));
        let down = ids(&run(&lib, sorted(TableSource::AllMusic, column, true)));
        assert_eq!(up.len(), 17);
        // The blank rows are the ones that sort last going up.
        let blank = lib
            .writer
            .call(move |c| {
                let (name, nullable, _) = column.column();
                if !nullable {
                    return Ok(0);
                }
                c.query_row(
                    &format!("SELECT count(*) FROM track_row WHERE {name} IS NULL"),
                    [],
                    |r| r.get::<_, i64>(0).map(|n| n as usize),
                )
            })
            .unwrap();
        if column != SortColumn::Title {
            assert!(blank > 0, "{column:?} has blanks in this fixture");
        }
        let (up_filled, up_blank) = up.split_at(up.len() - blank);
        let (down_filled, down_blank) = down.split_at(down.len() - blank);
        let reversed = |ids: &[i64]| ids.iter().rev().copied().collect::<Vec<_>>();
        assert_eq!(down_filled, reversed(up_filled), "{column:?}");
        assert_eq!(down_blank, reversed(up_blank), "{column:?}");
    }
}

#[test]
fn every_sort_either_way_round_is_read_from_its_index_with_nothing_to_sort() {
    let lib = tied_and_blank_tracks();
    lib.settle();
    for (column, index) in [
        (SortColumn::Title, "track_row_title"),
        (SortColumn::Artist, "track_row_artist"),
        (SortColumn::Genre, "track_row_genre"),
        (SortColumn::Bpm, "track_row_bpm"),
        (SortColumn::Key, "track_row_key"),
        (SortColumn::Time, "track_row_time"),
        (SortColumn::Rating, "track_row_rating"),
        (SortColumn::DateAdded, "track_row_added"),
        (SortColumn::File, "track_row_file"),
        (SortColumn::Folder, "track_row_folder"),
    ] {
        for source in [TableSource::Library, TableSource::AllMusic] {
            for descending in [false, true] {
                let q = sorted(source.clone(), column, descending);
                let plans = lib
                    .writer
                    .call(move |c| {
                        let built = build(&q, None);
                        let statements = match &built.backwards {
                            None => vec![(built.from_where.clone(), built.order_by.clone())],
                            Some(b) => vec![
                                (
                                    format!("{} AND {}", built.from_where, b.filled),
                                    b.filled_order.clone(),
                                ),
                                (
                                    format!("{} AND {}", built.from_where, b.blank),
                                    b.blank_order.clone(),
                                ),
                            ],
                        };
                        let mut plans = Vec::new();
                        for (from_where, order) in statements {
                            let sql = format!(
                                "EXPLAIN QUERY PLAN SELECT tr.recording_id {from_where}
                                 ORDER BY {order} LIMIT ?{} OFFSET ?{}",
                                built.params.len() + 1,
                                built.params.len() + 2,
                            );
                            let mut params = built.params.clone();
                            params.push(Value::Integer(200));
                            params.push(Value::Integer(0));
                            let details: Vec<String> = c
                                .prepare(&sql)?
                                .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(3))?
                                .collect::<rusqlite::Result<_>>()?;
                            plans.push(details.join(" / "));
                        }
                        Ok(plans)
                    })
                    .unwrap();
                for plan in plans {
                    assert!(
                        plan.split(|c: char| !c.is_alphanumeric() && c != '_')
                            .any(|word| word == index),
                        "{column:?} descending {descending} should read {index}: {plan}"
                    );
                    assert!(
                        !plan.contains("TEMP B-TREE"),
                        "{column:?} descending {descending} sorts: {plan}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_deleted_tracks_row_goes_and_it_leaves_every_list() {
    let f = fixture();
    let [r1, _, _, r4, ..] = f.recs;
    f.lib.settle();
    let rows_of = |lib: &Lib, id: i64| -> i64 {
        lib.writer
            .call(move |c| {
                c.query_row(
                    "SELECT count(*) FROM track_row WHERE recording_id = ?1",
                    [id],
                    |r| r.get(0),
                )
            })
            .unwrap()
    };
    assert_eq!(rows_of(&f.lib, r4), 1);
    // The track goes, as the grouping does it: its Library track, its files'
    // links, then the track.
    f.lib
        .insert("DELETE FROM analysis WHERE recording_id = ?1", (r4,));
    f.lib
        .insert("DELETE FROM library_track WHERE recording_id = ?1", (r4,));
    f.lib
        .insert("DELETE FROM recording_file WHERE recording_id = ?1", (r4,));
    f.lib.insert("DELETE FROM recording WHERE id = ?1", (r4,));
    f.lib.settle();
    assert_eq!(
        rows_of(&f.lib, r4),
        0,
        "no row is left for a track that is gone"
    );
    for source in [TableSource::Library, TableSource::AllMusic] {
        let page = run(&f.lib, query(source));
        assert!(!ids(&page).contains(&r4));
        assert_eq!(page.total, Some(page.rows.len() as u32));
    }
    assert!(ids(&run(&f.lib, query(TableSource::Library))).contains(&r1));
}

/// Settles what is due, and returns every row of the track table and of the
/// search index that was inserted, updated or deleted meanwhile, as
/// (table, track's id).
fn rows_written_by_settling(lib: &Lib) -> Vec<(String, i64)> {
    lib.writer
        .call(|c| {
            let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let seen = std::sync::Arc::clone(&written);
            c.update_hook(Some(
                move |_: rusqlite::hooks::Action, _: &str, table: &str, row: i64| {
                    // The search index keeps a size for each of its rows,
                    // under the track's id.
                    if table == "track_row" || table == "search_track_docsize" {
                        seen.lock().unwrap().push((table.to_owned(), row));
                    }
                },
            ))?;
            let result = crate::search::settle(c, &Mount, 2);
            c.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>)?;
            result?;
            let mut written = written.lock().unwrap().clone();
            written.sort();
            written.dedup();
            Ok(written)
        })
        .unwrap()
}

fn mark_everything_due_as_a_rekordbox_read_does(lib: &Lib) {
    lib.exec("INSERT OR IGNORE INTO search_due (recording_id) VALUES (-1)");
}

#[test]
fn a_rekordbox_read_that_changed_nothing_writes_no_row_of_either_table() {
    let f = fixture();
    f.lib.settle();
    let before = table_rows(&f.lib);
    mark_everything_due_as_a_rekordbox_read_does(&f.lib);
    assert!(f.lib.pending());
    assert_eq!(rows_written_by_settling(&f.lib), Vec::new());
    assert!(!f.lib.pending(), "the marks are cleared all the same");
    assert_eq!(table_rows(&f.lib), before);
    // A hook that sees nothing would prove nothing: it does see a write.
    f.lib.insert(
        "UPDATE analysis SET bpm = 99 WHERE recording_id = ?1",
        (f.recs[0],),
    );
    assert!(!rows_written_by_settling(&f.lib).is_empty());
}

#[test]
fn a_rekordbox_read_that_changed_two_tracks_writes_the_rows_of_those_two_only() {
    let f = fixture();
    let [r1, r2, r3, r4, ..] = f.recs;
    f.lib.settle();
    // r1's BPM changes (the table's row, not its text), r2's genre changes
    // (the table's row and its text).
    f.lib.insert(
        "UPDATE analysis SET bpm = 99 WHERE recording_id = ?1",
        (r1,),
    );
    genre(&f.lib, f.files[1], "alpha wave", "Zed", "Trance");
    mark_everything_due_as_a_rekordbox_read_does(&f.lib);
    let written = rows_written_by_settling(&f.lib);
    let wrote = |table: &str, id: i64| written.contains(&(table.to_owned(), id));
    assert!(wrote("track_row", r1), "{written:?}");
    assert!(
        !wrote("search_track_docsize", r1),
        "r1's text is the same: {written:?}"
    );
    assert!(wrote("track_row", r2), "{written:?}");
    assert!(wrote("search_track_docsize", r2), "{written:?}");
    for same in [r3, r4] {
        assert!(!wrote("track_row", same), "{written:?}");
        assert!(!wrote("search_track_docsize", same), "{written:?}");
    }
    // And the changes are there to be read.
    let bpm = ids(&run(
        &f.lib,
        sorted(TableSource::AllMusic, SortColumn::Bpm, false),
    ));
    assert_eq!(bpm[..2], [r4, r1]);
    assert_eq!(f.lib.finds("trance"), [r2]);
    assert_eq!(f.lib.finds("techno"), Vec::<i64>::new());
}

#[test]
fn the_rows_written_by_skipping_unchanged_ones_are_the_rows_a_rebuild_gives() {
    let f = fixture();
    f.lib.settle();
    f.lib.insert(
        "UPDATE analysis SET bpm = 99 WHERE recording_id = ?1",
        (f.recs[0],),
    );
    genre(&f.lib, f.files[1], "alpha wave", "Zed", "Trance");
    mark_everything_due_as_a_rekordbox_read_does(&f.lib);
    f.lib.settle();
    let incremental = table_rows(&f.lib);
    let search = f.lib.index_contents();
    f.lib
        .writer
        .call(|c| crate::search::rebuild(c, &Mount, 2))
        .unwrap();
    assert_eq!(table_rows(&f.lib), incremental);
    assert_eq!(f.lib.index_contents(), search);
}

/// Every row of the track table, as text.
fn table_rows(lib: &Lib) -> Vec<String> {
    lib.writer
        .call(|c| {
            c.prepare("SELECT * FROM track_row ORDER BY recording_id")?
                .query_map([], |r| {
                    (0..r.as_ref().column_count())
                        .map(|i| {
                            r.get::<_, rusqlite::types::Value>(i)
                                .map(|v| format!("{v:?}"))
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map(|v| v.join("|"))
                })?
                .collect()
        })
        .unwrap()
}

#[test]
fn a_search_and_a_sort_together_list_the_found_tracks_in_the_sorted_order() {
    let f = fixture();
    let [r1, r2, r3, r4, ..] = f.recs;
    let found = |text: &str, column: SortColumn, descending: bool, offset: u32, limit: u32| {
        let page = run(
            &f.lib,
            TableQuery {
                search: Some(text.to_owned()),
                offset,
                limit,
                ..sorted(TableSource::AllMusic, column, descending)
            },
        );
        (ids(&page), page.total)
    };
    // "moss" finds r1 (124 BPM, "Delta Drive") and r4 (90.5 BPM, "Cafe"):
    // by BPM the order is not that of their ids, going up.
    assert_eq!(
        found("moss", SortColumn::Bpm, false, 0, 10),
        (vec![r4, r1], Some(2))
    );
    assert_eq!(
        found("moss", SortColumn::Bpm, true, 0, 10),
        (vec![r1, r4], Some(2))
    );
    // "alph" finds r2 (128 BPM, 12A) and r3 (124 BPM, 1B): the order by BPM
    // or key is not that of their ids, going up, and the other way going down.
    assert_eq!(found("alph", SortColumn::Bpm, false, 0, 10).0, [r3, r2]);
    assert_eq!(found("alph", SortColumn::Bpm, true, 0, 10).0, [r2, r3]);
    assert_eq!(found("alph", SortColumn::Key, false, 0, 10).0, [r3, r2]);
    assert_eq!(found("alph", SortColumn::Title, true, 0, 10).0, [r2, r3]);
    // A window of the sorted, searched list.
    assert_eq!(found("alph", SortColumn::Bpm, false, 1, 10).0, [r2]);
    assert_eq!(found("alph", SortColumn::Bpm, true, 1, 1).0, [r3]);
}

#[test]
fn a_track_without_a_value_sorts_last_whichever_way_with_a_search_too() {
    let lib = Lib::new();
    let mut recs = Vec::new();
    for (n, bpm) in [Some(120.0), None, Some(100.0), None, Some(110.0)]
        .into_iter()
        .enumerate()
    {
        let name: &'static str = Box::leak(format!("{n}.mp3").into_boxed_str());
        let (rec, _) = track(&lib, &format!("Tune {n}"), "A", name);
        if let Some(bpm) = bpm {
            analysis(&lib, rec, bpm, "8A");
        }
        recs.push(rec);
    }
    let found = |descending: bool, offset: u32, limit: u32| {
        ids(&run(
            &lib,
            TableQuery {
                search: Some("tune".to_owned()),
                offset,
                limit,
                ..sorted(TableSource::AllMusic, SortColumn::Bpm, descending)
            },
        ))
    };
    let [a, b, c, d, e] = [recs[0], recs[1], recs[2], recs[3], recs[4]];
    assert_eq!(found(false, 0, 10), [c, e, a, b, d]);
    assert_eq!(found(true, 0, 10), [a, e, c, d, b]);
    assert_eq!(found(true, 2, 2), [c, d]);
}
