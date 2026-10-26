//! Tests for the track table's data. Everything is synthetic: made-up
//! volume, folders, files, titles and artists in a migrated database in a
//! temp dir.

mod folders;
mod order;

use super::*;
use crate::search::flag;
use crate::search::tests::support::{Lib, Mount};

fn dirs() -> FragileDirs {
    FragileDirs {
        downloads: None,
        temp: Vec::new(),
    }
}

fn query(source: TableSource) -> TableQuery {
    TableQuery {
        source,
        search: None,
        sort: None,
        offset: 0,
        limit: 100,
        with_total: true,
    }
}

fn sorted(source: TableSource, column: SortColumn, descending: bool) -> TableQuery {
    TableQuery {
        sort: Some(TableSort { column, descending }),
        ..query(source)
    }
}

/// The page `q` asks for, once the index is current.
fn run(lib: &Lib, q: TableQuery) -> TablePage {
    lib.settle();
    run_as_is(lib, q).expect("the crate exists")
}

/// The page `q` asks for, whether the index is current or not.
fn run_as_is(lib: &Lib, q: TableQuery) -> Option<TablePage> {
    lib.writer
        .call(move |c| Ok(read(c, &Mount, &q)?.map(|found| page(found, q.offset, &Mount, &dirs()))))
        .unwrap()
}

/// The titles of a page's rows, in order (the file's name where a track has
/// no title of its own).
fn titles(page: &TablePage) -> Vec<String> {
    page.rows
        .iter()
        .map(|r| {
            r.title
                .clone()
                .or_else(|| r.file.as_ref().map(|f| f.name.clone()))
                .unwrap_or_default()
        })
        .collect()
}

fn ids(page: &TablePage) -> Vec<i64> {
    page.rows.iter().map(|r| r.recording_id).collect()
}

/// A named track with one file; the Library track and the fields come after.
fn track(lib: &Lib, title: &str, artist: &str, name: &'static str) -> (i64, i64) {
    lib.track(Some(title), Some(artist), name)
}

fn analysis(lib: &Lib, recording: i64, bpm: f64, key: &'static str) {
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, 'rekordbox', ?2, ?3)",
        (recording, bpm, key),
    );
}

fn duration(lib: &Lib, file: i64, ms: i64) {
    lib.insert("UPDATE file SET duration_ms = ?1 WHERE id = ?2", (ms, file));
}

fn genre(lib: &Lib, file: i64, title: &str, artist: &str, genre: &str) {
    lib.tag(
        file,
        &[("Name", title), ("Artist", artist), ("Genre", genre)],
    );
}

/// Six tracks. Library: Delta Drive (1), alpha wave (2) and Cafe (4); not in
/// the Library: Alpha Wave (3, the same title as 2 by another artist), a
/// track named by its file only (5) and one with a file that is gone (6).
struct Fixture {
    lib: Lib,
    recs: [i64; 6],
    files: [i64; 6],
}

fn fixture() -> Fixture {
    let lib = Lib::new();
    let (r1, f1) = track(&lib, "Delta Drive", "Moss", "House/delta.mp3");
    let (r2, f2) = track(&lib, "alpha wave", "Zed", "alpha.flac");
    let (r3, f3) = track(&lib, "Alpha Wave", "Able", "alpha b.mp3");
    let (r4, f4) = track(&lib, "Caf\u{e9}", "Moss", "cafe.mp3");
    let (r5, f5) = lib.track(None, None, "zzz.wav");
    let (r6, f6) = track(&lib, "Gone Girl", "Zed", "gone.mp3");
    lib.library(r1, f1);
    lib.library(r2, f2);
    lib.library(r4, f4);
    genre(&lib, f1, "Delta Drive", "Moss", "House");
    genre(&lib, f2, "alpha wave", "Zed", "Techno");
    genre(&lib, f3, "Alpha Wave", "Able", "house");
    analysis(&lib, r1, 124.0, "8A");
    analysis(&lib, r2, 128.0, "12A");
    analysis(&lib, r3, 124.0, "1B");
    analysis(&lib, r4, 90.5, "10A");
    duration(&lib, f1, 300_000);
    duration(&lib, f2, 200_000);
    duration(&lib, f4, 250_000);
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (f6,));
    lib.insert(
        "UPDATE file SET sniffed_format = 'mp3' WHERE id IN (?1, ?2, ?3, ?4)",
        (f1, f3, f4, f6),
    );
    lib.insert(
        "UPDATE file SET sniffed_format = 'flac' WHERE id = ?1",
        (f2,),
    );
    lib.insert(
        "UPDATE file SET sniffed_format = 'wav' WHERE id = ?1",
        (f5,),
    );
    Fixture {
        lib,
        recs: [r1, r2, r3, r4, r5, r6],
        files: [f1, f2, f3, f4, f5, f6],
    }
}

// The sources.

#[test]
fn the_library_lists_its_tracks_in_the_library_lists_order() {
    let f = fixture();
    let page = run(&f.lib, query(TableSource::Library));
    // Title, then artist, then id; capitals and accents don't matter.
    assert_eq!(titles(&page), ["alpha wave", "Caf\u{e9}", "Delta Drive"]);
    assert_eq!(ids(&page), [f.recs[1], f.recs[3], f.recs[0]]);
    assert_eq!(page.total, Some(3));
    assert!(page.rows.iter().all(|r| r.in_library));
}

#[test]
fn all_music_lists_every_track_with_a_file_and_says_which_are_in_the_library() {
    let f = fixture();
    // A track with no file isn't in All music.
    let ghost = f.lib.insert("INSERT INTO recording DEFAULT VALUES", ());
    let page = run(&f.lib, query(TableSource::AllMusic));
    assert_eq!(
        titles(&page),
        [
            // Alpha Wave by Able sorts before alpha wave by Zed: the artist
            // breaks the tie.
            "Alpha Wave",
            "alpha wave",
            "Caf\u{e9}",
            "Delta Drive",
            "Gone Girl",
            "zzz.wav"
        ]
    );
    assert!(!ids(&page).contains(&ghost));
    assert_eq!(page.total, Some(6));
    let in_library: Vec<bool> = page.rows.iter().map(|r| r.in_library).collect();
    assert_eq!(in_library, [false, true, true, true, false, false]);
}

#[test]
fn a_crate_lists_its_tracks_in_the_order_they_were_added() {
    let f = fixture();
    let lt = |recording: i64| -> i64 {
        f.lib
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT id FROM library_track WHERE recording_id = ?1",
                    [recording],
                    |r| r.get(0),
                )
            })
            .unwrap()
    };
    f.lib.insert(
        "INSERT INTO crate (name, kind) VALUES ('Warm up', 'static')",
        (),
    );
    for (n, recording) in [f.recs[0], f.recs[3], f.recs[1]].into_iter().enumerate() {
        f.lib.insert(
            "INSERT INTO crate_entry (crate_id, library_track_id, kind, added_at)
             VALUES (1, ?1, 'member', ?2)",
            (lt(recording), format!("2026-10-0{}T10:00:00.000Z", n + 1)),
        );
    }
    let source = TableSource::Crate { id: CrateId(1) };
    let page = run(&f.lib, query(source.clone()));
    assert_eq!(titles(&page), ["Delta Drive", "Caf\u{e9}", "alpha wave"]);
    assert_eq!(page.total, Some(3));
    // Sorting leaves the crate's own order.
    let by_title = run(&f.lib, sorted(source, SortColumn::Title, false));
    assert_eq!(
        titles(&by_title),
        ["alpha wave", "Caf\u{e9}", "Delta Drive"]
    );
}

#[test]
fn a_crate_that_does_not_exist_is_not_a_page() {
    let f = fixture();
    f.lib.settle();
    assert!(run_as_is(&f.lib, query(TableSource::Crate { id: CrateId(99) })).is_none());
    // Nor is a folder, which holds no tracks.
    f.lib.insert(
        "INSERT INTO crate (name, kind) VALUES ('Folder', 'folder')",
        (),
    );
    assert!(run_as_is(&f.lib, query(TableSource::Crate { id: CrateId(1) })).is_none());
}

// What a row shows.

#[test]
fn a_row_shows_the_values_the_lists_show() {
    let f = fixture();
    let page = run(&f.lib, query(TableSource::AllMusic));
    let delta = page
        .rows
        .iter()
        .find(|r| r.recording_id == f.recs[0])
        .unwrap();
    assert_eq!(delta.title.as_deref(), Some("Delta Drive"));
    assert_eq!(delta.artist.as_deref(), Some("Moss"));
    assert_eq!(delta.bpm, Some(124.0));
    assert_eq!(delta.key.as_deref(), Some("8A"));
    assert_eq!(delta.time_ms, Some(300_000));
    assert_eq!(delta.genre.as_deref(), Some("House"));
    assert_eq!(delta.rating, None);
    assert!(delta.library_track_id.is_some());
    let file = delta.file.as_ref().unwrap();
    assert_eq!(file.name, "delta.mp3");
    assert_eq!(file.path, r"E:\Music\House\delta.mp3");
    assert!(file.present);
    // A track with no tags shows its file's name for a title, and nothing
    // for the rest.
    let bare = page
        .rows
        .iter()
        .find(|r| r.recording_id == f.recs[4])
        .unwrap();
    assert_eq!(bare.title.as_deref(), Some("zzz.wav"));
    assert_eq!(bare.artist, None);
    assert_eq!(
        (bare.bpm, bare.key.as_deref(), bare.time_ms),
        (None, None, None)
    );
    assert_eq!(bare.genre, None);
    assert!(bare.library_track_id.is_none());
}

#[test]
fn a_library_track_rekordbox_knows_shows_rekordboxs_values() {
    let f = fixture();
    f.lib.rekordbox_entry(
        f.files[0],
        &[
            ("Name", "Delta Drive (rekordbox)"),
            ("Genre", "Deep House"),
            ("TotalTime", "301"),
            ("Rating", "153"),
        ],
    );
    let page = run(&f.lib, query(TableSource::Library));
    let delta = page
        .rows
        .iter()
        .find(|r| r.recording_id == f.recs[0])
        .unwrap();
    assert_eq!(delta.title.as_deref(), Some("Delta Drive (rekordbox)"));
    assert_eq!(delta.genre.as_deref(), Some("Deep House"));
    assert_eq!(delta.time_ms, Some(301_000));
    assert_eq!(delta.rating, Some(3));
    // The tags of the others stand.
    let alpha = page
        .rows
        .iter()
        .find(|r| r.recording_id == f.recs[1])
        .unwrap();
    assert_eq!(
        (alpha.genre.as_deref(), alpha.rating),
        (Some("Techno"), None)
    );
    // What the table sorts by is rekordbox's too.
    let by_genre = run(
        &f.lib,
        sorted(TableSource::Library, SortColumn::Genre, false),
    );
    assert_eq!(by_genre.rows[0].genre.as_deref(), Some("Deep House"));
    let by_time = run(&f.lib, sorted(TableSource::Library, SortColumn::Time, true));
    assert_eq!(by_time.rows[0].recording_id, f.recs[0]);
    let by_rating = run(
        &f.lib,
        sorted(TableSource::Library, SortColumn::Rating, true),
    );
    assert_eq!(by_rating.rows[0].recording_id, f.recs[0]);
}

#[test]
fn a_missing_file_and_a_probable_match_are_on_the_row() {
    let f = fixture();
    f.lib.library(f.recs[5], f.files[5]);
    f.lib.insert(
        "UPDATE library_track SET source_status = 'missing' WHERE recording_id = ?1",
        (f.recs[5],),
    );
    let page = run(&f.lib, query(TableSource::Library));
    let gone = page
        .rows
        .iter()
        .find(|r| r.recording_id == f.recs[5])
        .unwrap();
    assert!(gone.source_missing);
    assert!(!gone.file.as_ref().unwrap().present);
    // A track outside the Library whose file is the probable match of a
    // rekordbox entry can't be added yet.
    f.lib.rekordbox_entry(f.files[2], &[]);
    f.lib
        .insert("UPDATE rekordbox_track SET relink_probable = 1", ());
    let all = run(&f.lib, query(TableSource::AllMusic));
    let flagged: Vec<i64> = all
        .rows
        .iter()
        .filter(|r| r.match_not_confirmed)
        .map(|r| r.recording_id)
        .collect();
    assert_eq!(flagged, [f.recs[2]]);
}

// Sorting.

#[test]
fn every_column_sorts_both_ways_and_descending_is_ascending_backwards_with_the_blanks_last() {
    let f = fixture();
    let all = TableSource::AllMusic;
    let order = |column: SortColumn, descending: bool| {
        ids(&run(&f.lib, sorted(all.clone(), column, descending)))
    };
    let [r1, r2, r3, r4, r5, r6] = f.recs;
    // BPM: 90.5, 124 (two: ties by id), 128, and the blanks last. Backwards,
    // the tied tracks and the blanks come in the reverse of their order.
    assert_eq!(order(SortColumn::Bpm, false), [r4, r1, r3, r2, r5, r6]);
    assert_eq!(order(SortColumn::Bpm, true), [r2, r3, r1, r4, r6, r5]);
    // Key: around the Camelot wheel (1B, 8A, 10A, 12A), blanks last.
    assert_eq!(order(SortColumn::Key, false), [r3, r1, r4, r2, r5, r6]);
    assert_eq!(order(SortColumn::Key, true), [r2, r4, r1, r3, r6, r5]);
    // Time: only three tracks have one.
    assert_eq!(order(SortColumn::Time, false), [r2, r4, r1, r3, r5, r6]);
    assert_eq!(order(SortColumn::Time, true), [r1, r4, r2, r6, r5, r3]);
    // Title: the file's name stands in for a missing one; no blanks.
    assert_eq!(order(SortColumn::Title, false), [r3, r2, r4, r1, r6, r5]);
    assert_eq!(order(SortColumn::Title, true), [r5, r6, r1, r4, r2, r3]);
    // Artist: capitals don't matter; the track with none is last.
    assert_eq!(order(SortColumn::Artist, false), [r3, r4, r1, r2, r6, r5]);
    assert_eq!(order(SortColumn::Artist, true), [r6, r2, r1, r4, r3, r5]);
    // Genre: case folded ("house" and "House" are one genre), blanks last.
    assert_eq!(order(SortColumn::Genre, false), [r3, r1, r2, r4, r6, r5]);
    assert_eq!(order(SortColumn::Genre, true), [r2, r1, r3, r5, r6, r4]);
    // File: by the file's name.
    assert_eq!(order(SortColumn::File, false), [r3, r2, r4, r1, r6, r5]);
    // Folder: the music folder, then the folder inside it.
    assert_eq!(order(SortColumn::Folder, true), [r1, r6, r5, r4, r3, r2]);
}

#[test]
fn the_ties_of_a_sort_fall_back_to_the_columns_tie_break_and_descending_reverses_them() {
    let lib = Lib::new();
    // Five tracks with the same BPM and genre, some with the same title.
    let mut recs = Vec::new();
    for (title, artist, name) in [
        ("Same", "B", "1.mp3"),
        ("Same", "A", "2.mp3"),
        ("Same", "A", "3.mp3"),
        ("Other", "C", "4.mp3"),
    ] {
        let (rec, file) = track(&lib, title, artist, name);
        genre(&lib, file, title, artist, "House");
        analysis(&lib, rec, 120.0, "8A");
        recs.push(rec);
    }
    // The same BPM: the ties are the track's id, backwards when descending.
    let page = run(&lib, sorted(TableSource::AllMusic, SortColumn::Bpm, false));
    assert_eq!(ids(&page), recs);
    let page = run(&lib, sorted(TableSource::AllMusic, SortColumn::Bpm, true));
    assert_eq!(ids(&page), [recs[3], recs[2], recs[1], recs[0]]);
    // The same genre: the title first, then the id; both reversed.
    let page = run(
        &lib,
        sorted(TableSource::AllMusic, SortColumn::Genre, false),
    );
    assert_eq!(ids(&page), [recs[3], recs[0], recs[1], recs[2]]);
    let page = run(&lib, sorted(TableSource::AllMusic, SortColumn::Genre, true));
    assert_eq!(ids(&page), [recs[2], recs[1], recs[0], recs[3]]);
    // The default order ties on the artist, then the id.
    let page = run(&lib, query(TableSource::AllMusic));
    assert_eq!(ids(&page), [recs[3], recs[1], recs[2], recs[0]]);
}

#[test]
fn blank_values_sort_last_both_ways_and_descending_reads_them_backwards_too() {
    let lib = Lib::new();
    let mut recs = Vec::new();
    for (n, bpm) in [Some(100.0), None, Some(110.0), None]
        .into_iter()
        .enumerate()
    {
        let (rec, _) = track(
            &lib,
            &format!("T{n}"),
            "A",
            Box::leak(format!("{n}.mp3").into_boxed_str()),
        );
        if let Some(bpm) = bpm {
            analysis(&lib, rec, bpm, "8A");
        }
        recs.push(rec);
    }
    let up = run(&lib, sorted(TableSource::AllMusic, SortColumn::Bpm, false));
    assert_eq!(ids(&up), [recs[0], recs[2], recs[1], recs[3]]);
    let down = run(&lib, sorted(TableSource::AllMusic, SortColumn::Bpm, true));
    assert_eq!(ids(&down), [recs[2], recs[0], recs[3], recs[1]]);
}

// Windows.

#[test]
fn windows_laid_end_to_end_are_the_whole_list_with_no_row_twice_or_skipped() {
    let lib = Lib::new();
    for n in 0..23 {
        let (rec, file) = track(
            &lib,
            &format!("Tune {:02}", n % 7),
            "A",
            Box::leak(format!("{n}.mp3").into_boxed_str()),
        );
        genre(&lib, file, &format!("Tune {:02}", n % 7), "A", "House");
        analysis(&lib, rec, 100.0 + f64::from(n % 5), "8A");
    }
    for column in [SortColumn::Title, SortColumn::Bpm, SortColumn::Genre] {
        for descending in [false, true] {
            let whole = run(
                &lib,
                TableQuery {
                    limit: 100,
                    ..sorted(TableSource::AllMusic, column, descending)
                },
            );
            assert_eq!(whole.rows.len(), 23);
            let mut stitched = Vec::new();
            for offset in (0..30).step_by(5) {
                let part = run(
                    &lib,
                    TableQuery {
                        offset,
                        limit: 5,
                        with_total: offset == 0,
                        ..sorted(TableSource::AllMusic, column, descending)
                    },
                );
                assert_eq!(part.offset, offset);
                assert_eq!(part.total.is_some(), offset == 0);
                stitched.extend(ids(&part));
            }
            assert_eq!(stitched, ids(&whole), "{column:?} {descending}");
        }
    }
}

#[test]
fn a_window_is_at_most_the_most_a_window_holds_and_a_window_past_the_end_is_empty() {
    let f = fixture();
    let page = run(
        &f.lib,
        TableQuery {
            offset: 50,
            ..query(TableSource::AllMusic)
        },
    );
    assert!(page.rows.is_empty());
    assert_eq!(page.total, Some(6));
    let page = run(
        &f.lib,
        TableQuery {
            limit: u32::MAX,
            ..query(TableSource::AllMusic)
        },
    );
    assert_eq!(page.rows.len(), 6);
    let none = run(
        &f.lib,
        TableQuery {
            limit: 0,
            ..query(TableSource::AllMusic)
        },
    );
    assert!(none.rows.is_empty());
    assert_eq!(none.total, Some(6));
}

// The flags the rows keep for the filters (1cA-4).

/// The tracks (ids, sorted) whose row has `bit` set, once the index is current.
fn with_flag(f: &Fixture, bit: i64) -> Vec<i64> {
    f.lib.settle();
    f.lib
        .writer
        .call(move |c| {
            c.prepare("SELECT recording_id FROM track_row WHERE flags & ?1 <> 0 ORDER BY 1")?
                .query_map([bit], |r| r.get(0))?
                .collect()
        })
        .unwrap()
}

/// A stored quality verdict for `file`.
fn verdict(lib: &Lib, file: i64, verdict: Option<&'static str>) {
    lib.insert(
        "INSERT OR REPLACE INTO file_verdict (file_id, rules_version, inputs, verdict)
         VALUES (?1, 1, '[]', ?2)",
        (file, verdict),
    );
}

#[test]
fn a_row_keeps_what_the_library_and_all_music_are_made_of() {
    let f = fixture();
    let [r1, r2, r3, r4, r5, r6] = f.recs;
    assert_eq!(with_flag(&f, flag::IN_LIBRARY), [r1, r2, r4]);
    assert_eq!(with_flag(&f, flag::HAS_FILE), [r1, r2, r3, r4, r5, r6]);
    // A format is the shown file's.
    let formats: Vec<Option<String>> = f
        .lib
        .writer
        .call(|c| {
            c.prepare("SELECT format FROM track_row ORDER BY recording_id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    let formats: Vec<_> = formats.into_iter().map(Option::unwrap).collect();
    assert_eq!(formats, ["mp3", "flac", "mp3", "mp3", "wav", "mp3"]);
}

#[test]
fn the_flags_follow_the_changes_they_depend_on() {
    let f = fixture();
    let [r1, r2, r3, r4, r5, r6] = f.recs;
    // Settle once so every change below is one the triggers must catch.
    f.lib.settle();
    // Missing: a file coming back, another going, and the scan's mark on a
    // Library track.
    assert_eq!(with_flag(&f, flag::MISSING), [r6]);
    f.lib
        .insert("UPDATE file SET present = 1 WHERE id = ?1", (f.files[5],));
    f.lib
        .insert("UPDATE file SET present = 0 WHERE id = ?1", (f.files[0],));
    assert_eq!(with_flag(&f, flag::MISSING), [r1]);
    f.lib
        .insert("UPDATE file SET present = 1 WHERE id = ?1", (f.files[0],));
    f.lib.insert(
        "UPDATE library_track SET source_status = 'missing' WHERE recording_id = ?1",
        (r4,),
    );
    assert_eq!(with_flag(&f, flag::MISSING), [r4]);
    // And the scan finding it again.
    f.lib.insert(
        "UPDATE library_track SET source_status = 'ok' WHERE recording_id = ?1",
        (r4,),
    );
    assert!(with_flag(&f, flag::MISSING).is_empty());
    // Quality: a stored verdict that flags the file, and one that doesn't.
    verdict(&f.lib, f.files[0], Some("ok"));
    verdict(&f.lib, f.files[1], Some("low_bitrate"));
    verdict(&f.lib, f.files[2], Some("broken"));
    verdict(&f.lib, f.files[3], None);
    assert_eq!(with_flag(&f, flag::QUALITY_PROBLEM), [r2, r3]);
    verdict(&f.lib, f.files[1], Some("ok"));
    assert_eq!(with_flag(&f, flag::QUALITY_PROBLEM), [r3]);
    f.lib
        .insert("DELETE FROM file_verdict WHERE file_id = ?1", (f.files[2],));
    assert!(with_flag(&f, flag::QUALITY_PROBLEM).is_empty());
    // A format worked out again.
    f.lib.insert(
        "UPDATE file SET sniffed_format = 'flac' WHERE id = ?1",
        (f.files[0],),
    );
    f.lib.settle();
    let flac: Vec<i64> = f
        .lib
        .writer
        .call(|c| {
            c.prepare("SELECT recording_id FROM track_row WHERE format = 'flac' ORDER BY 1")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    assert_eq!(flac, [r1, r2]);
    // Crates: a track joining one, moving to another, and leaving with its crate.
    f.lib.insert(
        "INSERT INTO crate (name, kind) VALUES ('One', 'static')",
        (),
    );
    f.lib.insert(
        "INSERT INTO crate (name, kind) VALUES ('Two', 'static')",
        (),
    );
    f.lib.insert(
        "INSERT INTO crate_entry (crate_id, library_track_id, kind)
         SELECT 1, id, 'member' FROM library_track WHERE recording_id = ?1",
        (r2,),
    );
    assert_eq!(with_flag(&f, flag::IN_CRATE), [r2]);
    f.lib.insert("UPDATE crate_entry SET crate_id = 2", ());
    assert_eq!(with_flag(&f, flag::IN_CRATE), [r2]);
    f.lib.insert("DELETE FROM crate WHERE id = 2", ());
    assert!(with_flag(&f, flag::IN_CRATE).is_empty());
    // rekordbox: a read adds an entry; a relink moves it; a read drops it.
    f.lib.rekordbox_entry(f.files[4], &[]);
    assert_eq!(with_flag(&f, flag::IN_REKORDBOX), [r5]);
    f.lib
        .insert("UPDATE rekordbox_track SET file_id = ?1", (f.files[2],));
    assert_eq!(with_flag(&f, flag::IN_REKORDBOX), [r3]);
    // A probable match isn't the track being in rekordbox.
    f.lib
        .insert("UPDATE rekordbox_track SET relink_probable = 1", ());
    assert!(with_flag(&f, flag::IN_REKORDBOX).is_empty());
    f.lib
        .insert("UPDATE rekordbox_track SET relink_probable = 0", ());
    assert_eq!(with_flag(&f, flag::IN_REKORDBOX), [r3]);
    f.lib.insert("DELETE FROM rekordbox_track", ());
    assert!(with_flag(&f, flag::IN_REKORDBOX).is_empty());
}

// Search.

#[test]
fn search_narrows_the_rows_of_every_source() {
    let f = fixture();
    let [r1, r2, r3, r4, ..] = f.recs;
    let search = |text: &str, source: TableSource| {
        let mut found = ids(&run(
            &f.lib,
            TableQuery {
                search: Some(text.to_owned()),
                ..query(source)
            },
        ));
        found.sort_unstable();
        found
    };
    // Capitals and accents don't matter, and a word matches from its start.
    assert_eq!(search("alph", TableSource::AllMusic), [r2, r3]);
    assert_eq!(search("CAFE", TableSource::AllMusic), [r4]);
    // The artist, the genre and the file's name are searched too.
    assert_eq!(search("moss", TableSource::AllMusic), [r1, r4]);
    assert_eq!(search("techno", TableSource::AllMusic), [r2]);
    assert_eq!(search("delta", TableSource::AllMusic), [r1]);
    // The Library only has its own.
    assert_eq!(search("alph", TableSource::Library), [r2]);
    // Text with no word to look for filters nothing.
    assert_eq!(search("  \" ", TableSource::AllMusic).len(), 6);
    // The total counts what is left, and a sort keeps to it.
    let page = run(
        &f.lib,
        TableQuery {
            search: Some("moss".to_owned()),
            ..sorted(TableSource::AllMusic, SortColumn::Bpm, true)
        },
    );
    assert_eq!(page.total, Some(2));
    assert_eq!(ids(&page), [r1, r4]);
}

#[test]
fn search_is_not_passed_to_the_index_as_it_is() {
    let f = fixture();
    // Operators and quotes are only text.
    for text in [
        "alpha OR delta",
        "title:alpha",
        "\"alpha",
        "alpha*",
        "NEAR(a b)",
    ] {
        let page = run(
            &f.lib,
            TableQuery {
                search: Some(text.to_owned()),
                ..query(TableSource::AllMusic)
            },
        );
        assert!(page.rows.len() <= 6, "{text}");
    }
}

// Staying current.

#[test]
fn the_page_says_when_the_index_is_behind_and_the_rows_shown_are_never_stale() {
    let f = fixture();
    f.lib.settle();
    let page = run_as_is(&f.lib, query(TableSource::Library)).unwrap();
    assert!(!page.behind);
    // A retag: the row shows the new title at once, while its place waits
    // for the index job.
    f.lib
        .tag(f.files[3], &[("Name", "Zebra"), ("Artist", "Moss")]);
    let page = run_as_is(&f.lib, query(TableSource::Library)).unwrap();
    assert!(page.behind);
    assert!(titles(&page).contains(&"Zebra".to_owned()));
    assert_eq!(titles(&page), ["alpha wave", "Zebra", "Delta Drive"]);
    let page = run(&f.lib, query(TableSource::Library));
    assert!(!page.behind);
    assert_eq!(titles(&page), ["alpha wave", "Delta Drive", "Zebra"]);
}

#[test]
fn a_track_added_to_the_library_appears_in_it_and_one_removed_leaves() {
    let f = fixture();
    let [_, _, r3, ..] = f.recs;
    assert_eq!(run(&f.lib, query(TableSource::Library)).total, Some(3));
    let lt = f.lib.library(r3, f.files[2]);
    let page = run(&f.lib, query(TableSource::Library));
    assert_eq!(page.total, Some(4));
    assert!(ids(&page).contains(&r3));
    f.lib
        .insert("DELETE FROM library_track WHERE id = ?1", (lt,));
    let page = run(&f.lib, query(TableSource::Library));
    assert_eq!(page.total, Some(3));
    assert!(!ids(&page).contains(&r3));
    // It is still in All music.
    assert!(ids(&run(&f.lib, query(TableSource::AllMusic))).contains(&r3));
}

#[test]
fn a_new_bpm_moves_the_track_in_the_order_once_the_index_has_caught_up() {
    let f = fixture();
    let [r1, r2, r3, r4, ..] = f.recs;
    let order = || {
        ids(&run(
            &f.lib,
            sorted(TableSource::AllMusic, SortColumn::Bpm, false),
        ))
    };
    assert_eq!(&order()[..4], [r4, r1, r3, r2]);
    f.lib.insert(
        "UPDATE analysis SET bpm = 150 WHERE recording_id = ?1",
        (r4,),
    );
    assert_eq!(&order()[..4], [r1, r3, r2, r4]);
}

#[test]
fn rebuilding_the_rows_gives_the_rows_the_incremental_path_does() {
    let f = fixture();
    f.lib.settle();
    let rows = |lib: &Lib| -> Vec<String> {
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
    };
    let before = rows(&f.lib);
    assert_eq!(before.len(), 6);
    f.lib
        .writer
        .call(|c| crate::search::rebuild(c, &Mount, 2))
        .unwrap();
    assert_eq!(rows(&f.lib), before);
}

#[test]
fn a_track_with_no_file_and_no_library_track_has_no_row() {
    let lib = Lib::new();
    lib.insert("INSERT INTO recording DEFAULT VALUES", ());
    lib.settle();
    let count: i64 = lib
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM track_row", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn a_library_track_whose_file_is_gone_from_the_database_still_has_a_row() {
    let lib = Lib::new();
    let (rec, file) = track(&lib, "Lonely", "A", "lonely.mp3");
    lib.library(rec, file);
    lib.insert("DELETE FROM recording_file WHERE file_id = ?1", (file,));
    let page = run(&lib, query(TableSource::Library));
    assert_eq!(ids(&page), [rec]);
    assert!(page.rows[0].file.is_some(), "its link still names the file");
    // It has no file among the track's files, so it is not in All music.
    assert!(run(&lib, query(TableSource::AllMusic)).rows.is_empty());
}

#[test]
fn the_camelot_order_runs_round_the_wheel() {
    use crate::search::camelot_order;
    assert_eq!(camelot_order("1A"), Some(2));
    assert_eq!(camelot_order("1B"), Some(3));
    assert_eq!(camelot_order("12B"), Some(25));
    assert!(camelot_order("8A") < camelot_order("8B"));
    assert!(camelot_order("8B") < camelot_order("9A"));
    for bad in ["", "A", "13A", "0A", "8C", "x"] {
        assert_eq!(camelot_order(bad), None, "{bad}");
    }
}

#[test]
fn a_length_is_rekordboxs_else_the_files_and_never_zero() {
    use crate::search::time_ms;
    assert_eq!(time_ms(Some(301), Some(300_400)), Some(301_000));
    assert_eq!(time_ms(None, Some(300_400)), Some(300_400));
    assert_eq!(time_ms(Some(0), Some(300_400)), Some(300_400));
    assert_eq!(time_ms(None, Some(0)), None);
    assert_eq!(time_ms(None, None), None);
}

mod over_ipc {
    use serde_json::{json, Value};
    use tauri::Manager;

    use crate::db::Writer;
    use crate::ipc::testing::{app, invoke};

    fn library_page(app: &tauri::App<tauri::test::MockRuntime>) -> Value {
        invoke(
            app,
            "track_table_page",
            json!({ "query": {
                "source": { "kind": "library" },
                "search": null,
                "sort": null,
                "offset": 0,
                "limit": 50,
                "withTotal": true,
            }}),
        )
        .unwrap()
    }

    #[test]
    fn a_track_added_or_removed_over_ipc_is_in_or_out_of_the_table_at_once() {
        let (_data, app) = app();
        app.state::<Writer>()
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                         VALUES ('serial=NTFS-1A2B3C4D', 'external', 'Q:\');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                         VALUES (1, 'Music', 'Music');
                     INSERT INTO file (music_folder_id, rel_path, rel_path_key)
                         VALUES (1, 'a.mp3', 'a.mp3');
                     INSERT INTO recording (title) VALUES ('Synthetic Tune');
                     INSERT INTO recording_file (recording_id, file_id, role)
                         VALUES (1, 1, 'best');",
                )?;
                // The index has been through the track, as it is by the
                // time anyone adds it.
                crate::search::settle(c, &crate::scan::system_volumes(), 100)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(library_page(&app)["total"], json!(0));

        let added = invoke(
            &app,
            "promote_track",
            json!({ "recordingId": 1, "confirmLowBitrate": false }),
        )
        .unwrap();
        assert_eq!(added["added"], json!(true));
        // At once: no waiting for the index job.
        let page = library_page(&app);
        assert_eq!(page["total"], json!(1));
        assert_eq!(page["behind"], json!(false));
        assert_eq!(page["rows"][0]["title"], json!("Synthetic Tune"));
        assert_eq!(page["rows"][0]["inLibrary"], json!(true));
        assert_eq!(
            page["rows"][0]["libraryTrackId"],
            added["libraryTrack"]["id"]
        );
        assert_eq!(page["rows"][0]["file"]["name"], json!("a.mp3"));

        let id = added["libraryTrack"]["id"].clone();
        invoke(&app, "remove_library_track", json!({ "id": id })).unwrap();
        let page = library_page(&app);
        assert_eq!(page["total"], json!(0));
        assert_eq!(page["behind"], json!(false));
    }

    #[test]
    fn a_crate_that_is_not_there_is_an_error_the_screen_can_name() {
        let (_data, app) = app();
        let refused = invoke(
            &app,
            "track_table_page",
            json!({ "query": {
                "source": { "kind": "crate", "id": 99 },
                "search": null,
                "sort": null,
                "offset": 0,
                "limit": 50,
                "withTotal": true,
            }}),
        )
        .unwrap_err();
        assert_eq!(refused["kind"], json!("crateNotFound"));
    }
}

#[test]
fn the_bpm_and_key_the_table_reads_are_the_ones_the_display_view_gives() {
    let lib = Lib::new();
    // Each source alone and several together, in every order of arrival.
    type Source<'a> = (&'a str, Option<f64>, Option<&'a str>);
    type Pair = (Option<f64>, Option<String>);
    let cases: [&[Source]; 5] = [
        &[("rekordbox", Some(124.0), Some("8A"))],
        &[
            ("local", Some(100.0), Some("1B")),
            ("tag", Some(110.0), None),
        ],
        &[("mik", None, Some("5A")), ("local", Some(99.0), Some("6A"))],
        &[
            ("local", Some(90.0), None),
            ("mik", Some(95.0), Some("2B")),
            ("rekordbox", None, Some("3A")),
        ],
        &[],
    ];
    for (n, rows) in cases.into_iter().enumerate() {
        let (rec, _) = lib.track(None, None, Box::leak(format!("t{n}.mp3").into_boxed_str()));
        for (source, bpm, key) in rows {
            lib.insert(
                "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, ?2, ?3, ?4)",
                (rec, source.to_string(), *bpm, key.map(str::to_owned)),
            );
        }
        let (view, direct): (Pair, Pair) = lib
            .writer
            .call(move |c| {
                let view = c.query_row(
                    "SELECT bpm, key FROM recording_display WHERE recording_id = ?1",
                    [rec],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let direct = c.query_row(
                    &format!(
                        "SELECT {} FROM recording r WHERE r.id = ?1",
                        crate::search::BPM_AND_KEY
                    ),
                    [rec],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                Ok((view, direct))
            })
            .unwrap();
        assert_eq!(direct, view, "{rows:?}");
    }
}
