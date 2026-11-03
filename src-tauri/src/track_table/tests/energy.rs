//! The energy a row shows and whose word it is (1cD): the Overview's answer
//! (`overview::shown_energy!`), with the source it came from, for the rows of
//! a window only.

use super::*;
use crate::overview::{overview_at, shown_energy};

/// A Library track named `name`.
fn library_track(lib: &Lib, name: &str) -> i64 {
    let (track, file) = lib.track(Some(name), Some("Someone"), &format!("{name}.mp3"));
    lib.library(track, file);
    track
}

fn give(lib: &Lib, recording: i64, source: &'static str, level: i64) {
    lib.insert(
        "INSERT INTO analysis (recording_id, source, energy) VALUES (?1, ?2, ?3)",
        (recording, source, level),
    );
}

/// An `analysis` row the table's checks would refuse.
fn unchecked(lib: &Lib, recording: i64, source: &'static str, level: i64) {
    lib.writer
        .call(move |c| {
            c.execute_batch("PRAGMA ignore_check_constraints = ON")?;
            let done = c.execute(
                "INSERT INTO analysis (recording_id, source, energy) VALUES (?1, ?2, ?3)",
                (recording, source, level),
            );
            c.execute_batch("PRAGMA ignore_check_constraints = OFF")?;
            done
        })
        .unwrap();
}

/// What the row of `recording` shows.
fn shown(lib: &Lib, recording: i64) -> (Option<u8>, Option<EnergySource>) {
    let page = run(lib, query(TableSource::AllMusic));
    let row = page
        .rows
        .iter()
        .find(|r| r.recording_id == recording)
        .expect("the track is listed");
    (row.energy, row.energy_source)
}

/// The Overview's own answer for a track: the SQL every other reader of the
/// shown energy shares.
fn overviews_answer(lib: &Lib, recording: i64) -> Option<i64> {
    lib.writer
        .call(move |c| {
            c.query_row(
                &format!("SELECT {}", shown_energy!("?1")),
                [recording],
                |r| r.get(0),
            )
        })
        .unwrap()
}

#[test]
fn a_row_shows_the_energy_of_each_source_and_says_whose_word_it_is() {
    let lib = Lib::new();
    let cases = [
        ("user", EnergySource::User, 8),
        ("mik", EnergySource::Mik, 6),
        ("local", EnergySource::Local, 4),
        ("ml", EnergySource::Ml, 2),
    ];
    for (source, expected, level) in cases {
        let track = library_track(&lib, source);
        give(&lib, track, source, level);
        assert_eq!(
            shown(&lib, track),
            (Some(level as u8), Some(expected)),
            "{source}"
        );
        assert_eq!(overviews_answer(&lib, track), Some(level), "{source}");
    }
}

#[test]
fn the_users_word_wins_and_taking_it_away_brings_the_next_source_back() {
    let lib = Lib::new();
    let track = library_track(&lib, "ranked");
    give(&lib, track, "ml", 1);
    give(&lib, track, "local", 2);
    give(&lib, track, "mik", 3);
    assert_eq!(shown(&lib, track), (Some(3), Some(EnergySource::Mik)));
    give(&lib, track, "user", 9);
    assert_eq!(shown(&lib, track), (Some(9), Some(EnergySource::User)));
    for (source, level, now) in [
        ("user", 3, EnergySource::Mik),
        ("mik", 2, EnergySource::Local),
        ("local", 1, EnergySource::Ml),
    ] {
        lib.insert("DELETE FROM analysis WHERE source = ?1", (source,));
        assert_eq!(shown(&lib, track), (Some(level), Some(now)));
        assert_eq!(overviews_answer(&lib, track), Some(i64::from(level)));
    }
    lib.insert("DELETE FROM analysis WHERE source = 'ml'", ());
    assert_eq!(shown(&lib, track), (None, None));
    assert_eq!(overviews_answer(&lib, track), None);
}

#[test]
fn a_track_with_analysis_but_no_energy_shows_none() {
    let lib = Lib::new();
    let track = library_track(&lib, "bpm only");
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (?1, 'mik', 126.0)",
        (track,),
    );
    assert_eq!(shown(&lib, track), (None, None));
}

#[test]
fn a_track_outside_the_library_shows_its_energy_and_source_too() {
    let lib = Lib::new();
    let (outside, _) = lib.track(Some("outside"), Some("Someone"), "outside.mp3");
    give(&lib, outside, "local", 5);
    assert_eq!(shown(&lib, outside), (Some(5), Some(EnergySource::Local)));
}

#[test]
fn the_rows_energy_is_what_the_overview_counts_for_each_level() {
    let lib = Lib::new();
    let sources = ["user", "mik", "local", "ml"];
    for n in 0..24_i64 {
        let track = library_track(&lib, &format!("track {n}"));
        // Some tracks have two sources, some one, some none.
        match n % 3 {
            0 => {
                give(&lib, track, sources[(n % 4) as usize], 1 + n % 10);
                give(
                    &lib,
                    track,
                    sources[((n + 1) % 4) as usize],
                    1 + (n * 7) % 10,
                );
            }
            1 => give(&lib, track, sources[(n % 4) as usize], 1 + n % 10),
            _ => {}
        }
    }
    lib.settle();
    let overview = lib
        .writer
        .call(|c| overview_at(c, 1_800_000_000_000))
        .unwrap();
    let page = run(&lib, query(TableSource::Library));
    for level in 1..=10_u8 {
        let rows = page.rows.iter().filter(|r| r.energy == Some(level)).count();
        assert_eq!(
            rows as u32,
            overview.energy[usize::from(level) - 1],
            "energy {level}"
        );
    }
    let none = page.rows.iter().filter(|r| r.energy.is_none()).count();
    assert_eq!(none as u32, overview.no_energy);
    for row in &page.rows {
        assert_eq!(
            row.energy.map(i64::from),
            overviews_answer(&lib, row.recording_id)
        );
        assert_eq!(row.energy.is_some(), row.energy_source.is_some());
    }
}

#[test]
fn a_row_with_unusual_energy_data_still_loads_with_the_rest_of_the_window() {
    let lib = Lib::new();
    let fine = library_track(&lib, "fine");
    give(&lib, fine, "mik", 6);
    let odd = library_track(&lib, "odd");
    // Values the table's checks refuse, as an older or damaged database
    // might hold them.
    unchecked(&lib, odd, "user", 99);
    let odd_source = library_track(&lib, "odd source");
    unchecked(&lib, odd_source, "future", 4);
    assert_eq!(shown(&lib, fine), (Some(6), Some(EnergySource::Mik)));
    assert_eq!(shown(&lib, odd), (None, None));
    assert_eq!(shown(&lib, odd_source), (Some(4), None));
}

#[test]
fn the_energy_lookup_of_a_window_reads_the_windows_tracks_by_index_and_never_scans_the_list() {
    let lib = Lib::new();
    let track = library_track(&lib, "planned");
    give(&lib, track, "mik", 5);
    let plan: Vec<String> = lib
        .writer
        .call(|c| {
            let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {WINDOW_ENERGY}"))?;
            let rows = stmt.query_map(["[1]"], |r| r.get::<_, String>(3))?;
            rows.collect()
        })
        .unwrap();
    assert!(
        plan.iter()
            .any(|d| d.starts_with("SEARCH a USING") && d.contains("recording_id=?")),
        "{plan:?}"
    );
    assert!(
        !plan
            .iter()
            .any(|d| d.starts_with("SCAN a ") || d == "SCAN a" || d.starts_with("SCAN analysis")),
        "{plan:?}"
    );
}

#[test]
fn a_window_shows_energy_for_its_own_rows_only() {
    let lib = Lib::new();
    let mut tracks = Vec::new();
    for n in 0..6_i64 {
        let track = library_track(&lib, &format!("t{n}"));
        give(&lib, track, "user", 1 + n);
        tracks.push(track);
    }
    let window = TableQuery {
        offset: 2,
        limit: 2,
        ..query(TableSource::Library)
    };
    let page = run(&lib, window);
    assert_eq!(page.rows.len(), 2);
    for row in &page.rows {
        let n = tracks.iter().position(|t| *t == row.recording_id).unwrap();
        assert_eq!(row.energy, Some(1 + n as u8));
    }
}

// ---- Sorting by energy (1cD-XE) ---------------------------------------------

/// Library tracks "a" to "f" whose shown energies are: a 7 (user), b 3 (Mixed
/// In Key), c 7 (estimate), d none, e 10 (user), f none.
fn energies_to_sort() -> (Lib, Vec<i64>) {
    let lib = Lib::new();
    let tracks: Vec<i64> = ["a", "b", "c", "d", "e", "f"]
        .iter()
        .map(|name| library_track(&lib, name))
        .collect();
    give(&lib, tracks[0], "user", 7);
    give(&lib, tracks[1], "mik", 3);
    give(&lib, tracks[2], "local", 7);
    give(&lib, tracks[4], "user", 10);
    (lib, tracks)
}

#[test]
fn sorting_by_energy_goes_lowest_first_and_highest_first_with_the_tracks_with_none_last_either_way()
{
    let (lib, t) = energies_to_sort();
    for source in [TableSource::Library, TableSource::AllMusic] {
        let up = ids(&run(
            &lib,
            sorted(source.clone(), SortColumn::Energy, false),
        ));
        assert_eq!(
            up,
            [t[1], t[0], t[2], t[4], t[3], t[5]],
            "ascending, ties by id"
        );
        let down = ids(&run(&lib, sorted(source, SortColumn::Energy, true)));
        assert_eq!(
            down,
            [t[4], t[2], t[0], t[1], t[5], t[3]],
            "descending is ascending backwards, none still last"
        );
    }
}

#[test]
fn sorting_by_energy_follows_a_new_energy_once_the_index_has_caught_up() {
    let (lib, t) = energies_to_sort();
    let order = |lib: &Lib| {
        ids(&run(
            lib,
            sorted(TableSource::Library, SortColumn::Energy, false),
        ))
    };
    assert_eq!(order(&lib)[0], t[1]);
    give(&lib, t[3], "user", 1);
    assert_eq!(order(&lib)[0], t[3], "the new energy 1 comes first");
    lib.insert(
        "DELETE FROM analysis WHERE recording_id = ?1 AND source = 'user'",
        (t[3],),
    );
    assert_eq!(order(&lib).last(), Some(&t[5]));
    assert!(
        order(&lib).ends_with(&[t[3], t[5]]),
        "and with none it goes last again"
    );
}

#[test]
fn a_window_of_an_energy_sort_is_the_same_rows_as_the_plain_sort_whichever_way_it_starts() {
    let (lib, _) = energies_to_sort();
    lib.settle();
    for descending in [false, true] {
        for limit in [1, 2, 4, 9] {
            for offset in 0..7 {
                let q = TableQuery {
                    offset,
                    limit,
                    ..sorted(TableSource::Library, SortColumn::Energy, descending)
                };
                let planned = lib
                    .writer
                    .call({
                        let q = q.clone();
                        move |c| Ok(window_ids(c, &q)?.0)
                    })
                    .unwrap();
                let plain = lib
                    .writer
                    .call({
                        let q = q.clone();
                        move |c| {
                            let built = build(&q, None);
                            select_ids(
                                c,
                                &built.from_where,
                                &built.order_by,
                                &built.params,
                                q.limit,
                                q.offset,
                            )
                        }
                    })
                    .unwrap();
                assert_eq!(
                    planned, plain,
                    "descending {descending} offset {offset} limit {limit}"
                );
            }
        }
    }
}
