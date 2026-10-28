//! The one rule for what All music shows (ROADMAP 1.11; PR 116's review,
//! item 2): a track whose files are all hidden is left out of every list,
//! count and position, by a condition worked out when the query runs. Each
//! test sets a file's `hidden_reason` and compares the lists with what is
//! left.

use super::*;

/// Five tracks by title: Able, Baker (its only file hidden), Charlie,
/// Dog (its best file hidden, an extra copy shown) and Easy (in the Library,
/// its only file hidden). Returns the library and the tracks' ids.
fn five_tracks() -> (Lib, [i64; 5]) {
    let lib = Lib::new();
    let (able, _) = track(&lib, "Able", "Ann", "able.mp3");
    let (baker, baker_file) = track(&lib, "Baker", "Ann", "baker.flac");
    let (charlie, _) = track(&lib, "Charlie", "Bo", "charlie.mp3");
    let (dog, dog_best) = track(&lib, "Dog", "Bo", "dog.flac");
    lib.file(dog, "dog (copy).mp3", "extra");
    let (easy, easy_file) = track(&lib, "Easy", "Bo", "easy.mp3");
    lib.library(easy, easy_file);
    for file in [baker_file, dog_best, easy_file] {
        lib.insert(
            "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
            (file,),
        );
    }
    (lib, [able, baker, charlie, dog, easy])
}

fn position_in(lib: &Lib, q: &TableQuery, recording_id: i64) -> Option<u32> {
    let asked = TablePositionQuery {
        source: q.source.clone(),
        search: q.search.clone(),
        filters: q.filters.clone(),
        sort: q.sort,
        recording_id,
    };
    lib.writer
        .call(move |c| position(c, &asked))
        .unwrap()
        .expect("All music exists")
}

#[test]
fn all_music_leaves_out_a_track_whose_files_are_all_hidden_and_the_library_keeps_it() {
    let (lib, [able, baker, charlie, dog, easy]) = five_tracks();

    let all = run(&lib, query(TableSource::AllMusic));
    assert_eq!(all.total, Some(3));
    assert_eq!(ids(&all), vec![able, charlie, dog]);
    // Dog still has a file that isn't hidden, its extra copy.
    assert!(!ids(&all).contains(&baker) && !ids(&all).contains(&easy));

    let library = run(&lib, query(TableSource::Library));
    assert_eq!(ids(&library), vec![easy]);
}

#[test]
fn the_position_of_a_track_counts_the_same_rows_as_the_window_after_a_hidden_only_track() {
    let (lib, [able, baker, charlie, dog, easy]) = five_tracks();
    for descending in [false, true] {
        let q = sorted(TableSource::AllMusic, SortColumn::Title, descending);
        let listed = ids(&run(&lib, q.clone()));
        for (index, id) in listed.iter().enumerate() {
            assert_eq!(position_in(&lib, &q, *id), Some(index as u32), "{id}");
        }
        // The hidden-only tracks are not in the list, so have no position.
        for hidden in [baker, easy] {
            assert_eq!(position_in(&lib, &q, hidden), None);
        }
        assert_eq!(listed.len(), 3);
    }
    // Charlie sits after Baker in the title order, yet is the second row.
    let q = query(TableSource::AllMusic);
    assert_eq!(position_in(&lib, &q, charlie), Some(1));
    let _ = (able, dog);
}

#[test]
fn a_search_and_the_count_leave_out_a_hidden_only_track_whose_text_matches() {
    let (lib, [able, _, _, _, _]) = five_tracks();
    // Baker and Able are both by Ann; only Able is listed.
    let found = run(
        &lib,
        TableQuery {
            search: Some("ann".into()),
            ..query(TableSource::AllMusic)
        },
    );
    assert_eq!(ids(&found), vec![able]);
    assert_eq!(found.total, Some(1));
    // The track named by its hidden file's name is not found either.
    let by_file = run(
        &lib,
        TableQuery {
            search: Some("baker".into()),
            ..query(TableSource::AllMusic)
        },
    );
    assert_eq!((by_file.total, by_file.rows.len()), (Some(0), 0));
}

#[test]
fn the_filters_and_the_sorts_work_on_the_tracks_that_are_left() {
    let (lib, [able, _, charlie, dog, _]) = five_tracks();
    lib.insert(
        "UPDATE file SET sniffed_format = 'flac' WHERE rel_path LIKE '%.flac'",
        (),
    );
    lib.insert(
        "UPDATE file SET sniffed_format = 'mp3' WHERE rel_path LIKE '%.mp3'",
        (),
    );
    let flac = run(
        &lib,
        filtered(
            TableSource::AllMusic,
            vec![TableFilter::Format {
                format: "flac".into(),
            }],
        ),
    );
    // Baker is FLAC but hidden: not listed. Dog is listed by its best file,
    // which is the hidden FLAC (PR 116's review, finding 1: awaiting the
    // owner's answer on a hidden best file with a shown extra copy; this
    // pins what the rule gives today).
    assert_eq!(ids(&flac), vec![dog]);
    let not_in_library = run(
        &lib,
        filtered(TableSource::AllMusic, vec![TableFilter::NotInLibrary]),
    );
    assert_eq!(ids(&not_in_library), vec![able, charlie, dog]);
    let in_library = run(
        &lib,
        filtered(TableSource::AllMusic, vec![TableFilter::InLibrary]),
    );
    // Easy is in the Library, and All music leaves it out while it has
    // nothing to show.
    assert_eq!(in_library.total, Some(0));
    for descending in [false, true] {
        let by_artist = run(
            &lib,
            sorted(TableSource::AllMusic, SortColumn::Artist, descending),
        );
        assert_eq!(by_artist.total, Some(3), "{descending}");
    }
    let formats = lib
        .writer
        .call(|c| formats(c, &TableSource::AllMusic))
        .unwrap()
        .unwrap();
    // Baker and Easy are not listed, so their files' formats aren't either.
    assert_eq!(formats, vec!["flac".to_owned(), "mp3".to_owned()]);
}

#[test]
fn showing_a_file_again_puts_its_track_back_in_the_list_at_once() {
    let (lib, [able, baker, charlie, dog, _]) = five_tracks();
    let before = ids(&run(&lib, query(TableSource::AllMusic)));
    assert_eq!(before, vec![able, charlie, dog]);
    let file = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT file_id FROM recording_file rf WHERE rf.recording_id = ?1",
                [baker],
                |r| r.get::<_, i64>(0),
            )
        })
        .unwrap();
    // Nothing is indexed again: the rule is worked out when the query runs.
    lib.writer
        .call(move |c| Ok(crate::hidden::show_again(c, file)))
        .unwrap()
        .unwrap();
    let after = run_as_is(&lib, query(TableSource::AllMusic)).unwrap();
    assert_eq!(ids(&after), vec![able, baker, charlie, dog]);
}
