//! The genre bars as doors (1cD-R1b; ROADMAP 1.13 "Every number is a door"):
//! a bar's count and the rows behind its chip are one definition, the genre
//! the index job keeps in `track_row.genre_group`. Genre names are free text,
//! so the Library here holds the awkward ones: capitals only apart, accents,
//! other scripts, blanks inside, and signs that mean something in an address
//! or in a query.

use super::*;
use crate::library::LibraryTrackId;
use crate::overview::{door_answers_of, overview_at, Overview};

/// Genre names a DJ's tags really hold. Each is a Library track of its own,
/// except the ones that are the same genre in other capitals.
const GENRES: &[&str] = &[
    "House",
    "house",
    "HOUSE",
    " House ",
    "Deep House",
    "Drum & Bass",
    "drum & bass",
    "R&B / Soul",
    "100% Pure",
    "50/50",
    "a+b",
    "a_b",
    "axb",
    "#1 Hits",
    "Hip Hop",
    "Hip  Hop",
    "Caf\u{e9}",
    "CAF\u{c9}",
    "Cafe",
    "\u{30c6}\u{30af}\u{30ce}",
    "\u{41f}\u{43e}\u{43f}",
    "\u{1f3b5} Mix",
    "Stra\u{df}e",
    "STRASSE",
    "Rock?filter=lowQuality",
    "x' OR '1'='1",
    "'; DROP TABLE track_row; --",
    "\"quoted\"",
    "back\\slash",
    "Other",
    "Techno",
];

fn world() -> Lib {
    let lib = Lib::new();
    for (i, genre) in GENRES.iter().enumerate() {
        let (recording, file) = lib.track(Some(&format!("T{i}")), Some("A"), &format!("t{i}.mp3"));
        lib.tag(
            file,
            &[
                ("Name", &format!("T{i}")),
                ("Artist", "A"),
                ("Genre", genre),
            ],
        );
        lib.library(recording, file);
    }
    // No genre in the tag, and a blank one.
    for (i, genre) in [None, Some("   ")].into_iter().enumerate() {
        let name = format!("None {i}");
        let (recording, file) = lib.track(Some(&name), Some("A"), &format!("none{i}.mp3"));
        if let Some(genre) = genre {
            lib.tag(file, &[("Name", &name), ("Genre", genre)]);
        }
        lib.library(recording, file);
    }
    // A track rekordbox knows has rekordbox's genre, whatever its tag says;
    // one with a blank genre there has none.
    for (name, rekordbox) in [("Known", "Techno"), ("Known blank", "")] {
        let (recording, file) = lib.track(Some(name), Some("A"), &format!("{name}.mp3"));
        lib.tag(file, &[("Name", name), ("Genre", "Tag Only")]);
        lib.library(recording, file);
        lib.trusted_entry(file, rekordbox);
    }
    // A track outside the Library with a genre of its own is no bar's.
    let (_, file) = lib.track(Some("Outside"), Some("A"), "outside.mp3");
    lib.tag(file, &[("Name", "Outside"), ("Genre", "House")]);
    lib
}

fn overview(lib: &Lib) -> Overview {
    lib.settle();
    lib.writer
        .call(|c| overview_at(c, 1_800_000_000_000))
        .unwrap()
}

fn genre_filter(name: &str) -> Vec<TableFilter> {
    vec![TableFilter::Genre { name: name.into() }]
}

/// The count the table says and the recording ids of the rows, in `source`.
fn rows(lib: &Lib, source: TableSource, filters: Vec<TableFilter>) -> (u32, Vec<i64>) {
    let page = run(lib, filtered(source, filters));
    let mut found = ids(&page);
    found.sort_unstable();
    let total = page.total.expect("the first window counts");
    assert_eq!(total as usize, found.len(), "the count is the rows");
    (total, found)
}

/// The genre the Overview's own definition gives each of `recordings`.
fn defined(lib: &Lib, recordings: &[i64]) -> Vec<Option<String>> {
    let recordings = recordings.to_vec();
    lib.writer
        .call(move |c| {
            let mut out = Vec::new();
            for recording in recordings {
                let library: i64 = c.query_row(
                    "SELECT id FROM library_track WHERE recording_id = ?1",
                    [recording],
                    |r| r.get(0),
                )?;
                out.push(
                    door_answers_of(c, &[LibraryTrackId(library)])?
                        .remove(&library)
                        .and_then(|answer| answer.genre),
                );
            }
            Ok(out)
        })
        .unwrap()
}

#[test]
fn every_genre_bar_opens_exactly_the_tracks_it_counts_whatever_the_name() {
    let lib = world();
    let o = overview(&lib);
    assert!(
        o.genres.len() > 25,
        "a varied set of bars: {:?}",
        o.genres.iter().map(|g| &g.name).collect::<Vec<_>>()
    );
    let mut counted = 0;
    for bar in &o.genres {
        let (total, found) = rows(&lib, TableSource::Library, genre_filter(&bar.name));
        assert_eq!(
            total, bar.tracks,
            "the rows of {:?} are its count",
            bar.name
        );
        // Each row has the bar's genre, however it is spelled.
        for genre in defined(&lib, &found) {
            assert_eq!(
                genre.map(|g| g.to_lowercase()),
                Some(bar.name.to_lowercase()),
                "{:?}",
                bar.name
            );
        }
        // And All music lists the same Library tracks, and none outside it.
        assert_eq!(
            rows(&lib, TableSource::AllMusic, genre_filter(&bar.name)).1,
            found,
            "{:?} in All music",
            bar.name
        );
        counted += bar.tracks;
    }
    // Every Library track is behind one bar, or has no genre.
    assert_eq!(counted + o.no_genre, o.tracks);
    assert_eq!(
        o.no_genre, 3,
        "no tag, a blank tag and a blank in rekordbox"
    );
}

#[test]
fn a_bar_that_merges_spellings_counts_and_opens_all_of_them() {
    let lib = world();
    let o = overview(&lib);
    let house = o
        .genres
        .iter()
        .find(|g| g.name == "House")
        .expect("House is a bar");
    assert_eq!(house.tracks, 4, "House, house, HOUSE and ' House '");
    for name in ["house", "HOUSE", "  House  ", "hOuSe"] {
        assert_eq!(
            rows(&lib, TableSource::Library, genre_filter(name)).0,
            4,
            "{name:?}"
        );
    }
    // A track rekordbox knows takes rekordbox's word: Techno twice.
    let techno = o.genres.iter().find(|g| g.name == "Techno").unwrap();
    assert_eq!(techno.tracks, 2);
}

#[test]
fn a_name_that_stands_for_nothing_lists_nothing_and_a_name_is_a_value_not_a_query() {
    let lib = world();
    let total = overview(&lib).tracks;
    for name in [
        "",
        "   ",
        "No such genre",
        "Tag Only",
        "%",
        "_",
        "House%",
        "H_use",
        "x' OR '1'='1 ",
        "\0",
    ] {
        let (found, _) = rows(&lib, TableSource::Library, genre_filter(name));
        // Only a name that really is a genre lists rows ("x' OR '1'='1" is
        // one, with its blank trimmed).
        let is_a_genre = GENRES
            .iter()
            .any(|g| g.trim().to_lowercase() == name.trim().to_lowercase());
        assert_eq!(found > 0, is_a_genre, "{name:?}");
        assert!(found < total, "{name:?} never lists every track");
    }
    // The table is intact after the names that look like statements.
    let (found, _) = rows(
        &lib,
        TableSource::Library,
        genre_filter("'; DROP TABLE track_row; --"),
    );
    assert_eq!(found, 1);
    let (found, _) = rows(&lib, TableSource::Library, genre_filter("a_b"));
    assert_eq!(found, 1, "an underscore is not a wildcard");
    let (found, _) = rows(&lib, TableSource::Library, genre_filter("100% Pure"));
    assert_eq!(found, 1, "a percent sign is not a wildcard");
}

#[test]
fn accents_and_letters_in_other_scripts_are_their_own_genres_but_capitals_are_not() {
    let lib = world();
    let o = overview(&lib);
    let tracks_of = |name: &str| o.genres.iter().find(|g| g.name == name).map(|g| g.tracks);
    // The two spellings are one bar, named by the smaller spelling.
    assert_eq!(
        tracks_of("CAF\u{c9}"),
        Some(2),
        "Caf\u{e9} and CAF\u{c9} have one bar"
    );
    assert_eq!(tracks_of("Caf\u{e9}"), None);
    assert_eq!(
        rows(&lib, TableSource::Library, genre_filter("Caf\u{e9}")).0,
        2
    );
    assert_eq!(tracks_of("Cafe"), Some(1), "an accent makes another genre");
    assert_eq!(tracks_of("Hip Hop"), Some(1));
    assert_eq!(
        tracks_of("Hip  Hop"),
        Some(1),
        "a blank inside is part of the name"
    );
    for name in [
        "\u{30c6}\u{30af}\u{30ce}",
        "\u{41f}\u{43e}\u{43f}",
        "\u{1f3b5} Mix",
    ] {
        assert_eq!(
            rows(&lib, TableSource::Library, genre_filter(name)).0,
            1,
            "{name:?}"
        );
    }
}

#[test]
fn a_genre_chip_works_with_the_other_filters_a_search_and_a_sort() {
    let lib = world();
    let house = genre_filter("House");
    let (_, all) = rows(&lib, TableSource::Library, house.clone());
    assert_eq!(all.len(), 4);

    // With another filter: the tracks that have both.
    let both = vec![
        TableFilter::Genre {
            name: "House".into(),
        },
        TableFilter::InNoCrate,
    ];
    assert_eq!(rows(&lib, TableSource::Library, both).1, all);
    let none = vec![
        TableFilter::Genre {
            name: "House".into(),
        },
        TableFilter::InCrate,
    ];
    assert_eq!(rows(&lib, TableSource::Library, none).0, 0);

    // With a search: only the track with that title.
    let mut q = filtered(TableSource::Library, house.clone());
    q.search = Some("T1".into());
    let page = run(&lib, q);
    assert_eq!(titles(&page), ["T1"]);

    // A sort keeps the chip, and reversing it reverses the rows.
    let mut q = sorted(TableSource::Library, SortColumn::Title, false);
    q.filters = house;
    let up = ids(&run(&lib, q.clone()));
    q.sort = Some(TableSort {
        column: SortColumn::Title,
        descending: true,
    });
    let down = ids(&run(&lib, q));
    assert_eq!(up.len(), 4);
    assert_eq!(down, up.iter().rev().copied().collect::<Vec<_>>());
}

#[test]
fn a_change_to_a_tag_moves_the_track_between_genre_chips_once_the_index_has_run() {
    let lib = world();
    let before = overview(&lib);
    let house = before
        .genres
        .iter()
        .find(|g| g.name == "House")
        .unwrap()
        .tracks;
    let moved = rows(&lib, TableSource::Library, genre_filter("House")).1[0];
    let file: i64 = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT file_id FROM recording_file WHERE recording_id = ?1",
                [moved],
                |r| r.get(0),
            )
        })
        .unwrap();
    // The tag editor or a scan gives the file another genre.
    lib.tag(
        file,
        &[("Name", "Moved"), ("Artist", "A"), ("Genre", "Brand New")],
    );
    assert!(lib.pending(), "the change marks the track due");
    assert_eq!(
        rows(&lib, TableSource::Library, genre_filter("House")).0,
        house - 1
    );
    assert_eq!(
        rows(&lib, TableSource::Library, genre_filter("brand new")).1,
        [moved]
    );
    let after = overview(&lib);
    assert_eq!(
        after
            .genres
            .iter()
            .find(|g| g.name == "Brand New")
            .map(|g| g.tracks),
        Some(1)
    );
}
