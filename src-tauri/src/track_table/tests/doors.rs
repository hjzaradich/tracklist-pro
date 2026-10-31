//! "Every number is a door" (1cC-4; ROADMAP 1.13): a number on the Overview
//! that opens the Library with a chip counts exactly the rows behind the
//! chip. The two are one definition (`overview::low_quality_sql`, `ANALYZED`,
//! `IN_A_CRATE`, `shown_energy!`), and these tests count both on a Library
//! made of the edge cases: a linked file that is gone, a track with no linked
//! file, a stale verdict, a track rekordbox knows but never analyzed, a
//! hidden file, a smart crate's rule, an energy from two sources.

use super::quality::current_verdict;
use super::*;
use crate::overview::{overview_at, Overview};

/// The Library of edge cases and its tracks by name.
struct Doors {
    lib: Lib,
    /// Library tracks, by name, as recording ids.
    tracks: Vec<(&'static str, i64)>,
    /// A track outside the Library that is not analyzed and has no energy:
    /// the one an unscoped "not analyzed" or "no energy" would wrongly list.
    plain_outside: i64,
}

impl Doors {
    fn named(&self, names: &[&str]) -> Vec<i64> {
        let mut ids: Vec<i64> = names
            .iter()
            .map(|name| {
                self.tracks
                    .iter()
                    .find(|(n, _)| n == name)
                    .unwrap_or_else(|| panic!("no track {name}"))
                    .1
            })
            .collect();
        ids.sort_unstable();
        ids
    }

    fn overview(&self) -> Overview {
        self.lib.settle();
        self.lib
            .writer
            .call(|c| overview_at(c, 1_800_000_000_000))
            .unwrap()
    }

    /// What the chips find in the Library: the count the table says, and the
    /// ids of the rows, which must be as many.
    fn rows(&self, filters: Vec<TableFilter>) -> (u32, Vec<i64>) {
        let page = run(&self.lib, filtered(TableSource::Library, filters));
        let mut found = ids(&page);
        found.sort_unstable();
        let total = page.total.expect("the first window counts");
        assert_eq!(total as usize, found.len(), "the count is the rows");
        (total, found)
    }

    /// The chip's rows are the Overview's number, and the tracks `expected`.
    fn agree(&self, filter: TableFilter, number: impl Fn(&Overview) -> u32, expected: &[&str]) {
        let (total, found) = self.rows(vec![filter.clone()]);
        assert_eq!(
            total,
            number(&self.overview()),
            "{filter:?}: rows == the Overview's number"
        );
        assert_eq!(found, self.named(expected), "{filter:?}: the tracks");
    }
}

fn verdicted(lib: &Lib, name: &'static str, verdict: Option<&str>) -> (i64, i64) {
    let (track, file) = lib.track(Some(name), Some("Someone"), name);
    lib.library(track, file);
    if let Some(verdict) = verdict {
        current_verdict(lib, file, verdict);
    }
    (track, file)
}

fn crate_with(lib: &Lib, name: &'static str, kind: &'static str) -> i64 {
    lib.insert(
        "INSERT INTO crate (name, kind, rules)
         VALUES (?1, ?2, CASE ?2 WHEN 'smart' THEN '{}' END)",
        (name, kind),
    )
}

fn put(lib: &Lib, crate_id: i64, recording: i64, kind: &'static str) {
    lib.insert(
        "INSERT INTO crate_entry (crate_id, library_track_id, kind)
         SELECT ?1, id, ?3 FROM library_track WHERE recording_id = ?2",
        (crate_id, recording, kind),
    );
}

fn energy(lib: &Lib, recording: i64, source: &'static str, level: i64) {
    lib.insert(
        "INSERT INTO analysis (recording_id, source, energy) VALUES (?1, ?2, ?3)",
        (recording, source, level),
    );
}

fn world() -> Doors {
    let lib = Lib::new();
    let mut tracks: Vec<(&'static str, i64)> = Vec::new();

    // Quality: a clean file, a low-bitrate file, a broken file that is gone
    // from disk, a verdict from old rules, a file with no verdict yet, and a
    // Library track with no linked file at all.
    let mut add = |name: &'static str, made: (i64, i64)| {
        tracks.push((name, made.0));
        made.1
    };
    add("clean", verdicted(&lib, "clean", Some("ok")));
    add("low", verdicted(&lib, "low", Some("low_bitrate")));
    let gone_file = add("gone", verdicted(&lib, "gone", Some("broken")));
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (gone_file,));
    let old_file = add("old", verdicted(&lib, "old", Some("suspect_transcode")));
    lib.insert(
        "UPDATE file_verdict SET rules_version = rules_version + 1 WHERE file_id = ?1",
        (old_file,),
    );
    add("unjudged", verdicted(&lib, "unjudged", None));
    let (nofile, _) = lib.track(Some("nofile"), None, "nofile.mp3");
    lib.insert(
        "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
         VALUES (?1, 'linked', NULL, 'missing')",
        (nofile,),
    );
    tracks.push(("nofile", nofile));
    // A track outside the Library whose file is flagged and analyzed: no
    // number counts it, and no chip of the Library lists it.
    let (outside, outside_file) = lib.track(Some("outside"), Some("Someone"), "outside.mp3");
    current_verdict(&lib, outside_file, "broken");
    analysis(&lib, outside, 128.0, "1A");
    let (plain_outside, _) = lib.track(Some("plain outside"), Some("Someone"), "plain.mp3");

    let rec = |name: &str| tracks.iter().find(|(n, _)| *n == name).unwrap().1;

    // rekordbox: "low" has a BPM and a key from it, "clean" a key only; "old"
    // is known to rekordbox (an entry) but has no analysis from it; "gone"
    // has a BPM from a file tag, which is not rekordbox analyzing it.
    analysis(&lib, rec("low"), 124.0, "8A");
    lib.insert(
        "INSERT INTO analysis (recording_id, source, key) VALUES (?1, 'rekordbox', '5A')",
        (rec("clean"),),
    );
    lib.insert(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES (?1, 'tag', 120.0)",
        (rec("gone"),),
    );
    lib.rekordbox_entry(old_file, &[("Name", "old")]);

    // Crates: "clean" in two, "low" in one, "gone" only in a smart crate's
    // rule (which is not membership).
    let warmup = crate_with(&lib, "Warmup", "static");
    let peak = crate_with(&lib, "Peak", "static");
    let smart = crate_with(&lib, "Fast", "smart");
    put(&lib, warmup, rec("clean"), "member");
    put(&lib, peak, rec("clean"), "member");
    put(&lib, warmup, rec("low"), "member");
    put(&lib, smart, rec("gone"), "always_include");

    // Energy: the user's wins over Mixed In Key's, which wins over the local
    // estimate's, which wins over the audio model's.
    energy(&lib, rec("clean"), "user", 7);
    energy(&lib, rec("clean"), "mik", 3);
    energy(&lib, rec("low"), "mik", 3);
    energy(&lib, rec("old"), "ml", 9);
    energy(&lib, rec("old"), "local", 4);
    energy(&lib, rec("unjudged"), "local", 7);
    energy(&lib, rec("gone"), "user", 3);

    // A hidden file doesn't take its Library track out of the Library.
    lib.insert(
        "UPDATE file SET hidden_reason = 'user'
         WHERE id = (SELECT linked_file_id FROM library_track WHERE recording_id = ?1)",
        (rec("unjudged"),),
    );
    Doors {
        lib,
        tracks,
        plain_outside,
    }
}

#[test]
fn the_low_quality_chip_opens_the_tracks_the_overview_counts_as_made_from_low_quality_files() {
    let w = world();
    // A flagged file that is gone still counts; a stale verdict, no verdict
    // and no linked file don't; neither does a track outside the Library.
    w.agree(TableFilter::LowQuality, |o| o.low_quality, &["low", "gone"]);
}

#[test]
fn the_low_quality_chip_follows_a_verdict_made_again_at_once() {
    let w = world();
    w.agree(TableFilter::LowQuality, |o| o.low_quality, &["low", "gone"]);
    // The quality job judges "old" again with the rules of now.
    let old = w.named(&["old"])[0];
    let file: i64 = w
        .lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT linked_file_id FROM library_track WHERE recording_id = ?1",
                [old],
                |r| r.get(0),
            )
        })
        .unwrap();
    current_verdict(&w.lib, file, "truncated");
    w.agree(
        TableFilter::LowQuality,
        |o| o.low_quality,
        &["low", "gone", "old"],
    );
}

#[test]
fn the_analyzed_chips_split_the_library_as_the_overview_does() {
    let w = world();
    // rekordbox supplied a BPM or a key for "clean" and "low". Knowing a
    // track (an entry) is not analyzing it, and a tag's BPM is not rekordbox's.
    w.agree(
        TableFilter::AnalyzedByRekordbox,
        |o| o.analyzed_by_rekordbox,
        &["clean", "low"],
    );
    w.agree(
        TableFilter::NotAnalyzedByRekordbox,
        |o| o.not_analyzed,
        &["gone", "old", "unjudged", "nofile"],
    );
    // A Library track is one or the other. A track outside the Library is
    // neither, in All music too.
    let (analyzed, _) = w.rows(vec![TableFilter::AnalyzedByRekordbox]);
    let (not, _) = w.rows(vec![TableFilter::NotAnalyzedByRekordbox]);
    assert_eq!(analyzed + not, w.overview().tracks);
    let all_music = run(
        &w.lib,
        filtered(
            TableSource::AllMusic,
            vec![TableFilter::NotAnalyzedByRekordbox],
        ),
    );
    // (All music leaves out "unjudged", whose only file is hidden.)
    assert_eq!(all_music.total, Some(not - 1));
}

#[test]
fn a_track_rekordbox_knows_is_not_analyzed_until_rekordbox_supplied_a_bpm_or_a_key() {
    let w = world();
    // "old" is in rekordbox (the Library's own chip says so) and not analyzed.
    let old = w.named(&["old"])[0];
    let (_, in_rekordbox) = w.rows(vec![TableFilter::InRekordbox]);
    assert!(in_rekordbox.contains(&old));
    // Both chips at once leave the track rekordbox has and didn't analyze.
    let (_, both) = w.rows(vec![
        TableFilter::InRekordbox,
        TableFilter::NotAnalyzedByRekordbox,
    ]);
    assert_eq!(both, [old]);
}

#[test]
fn the_crate_chips_open_the_tracks_the_overview_counts_in_crates_and_in_no_crate() {
    let w = world();
    // A smart crate's rule is not membership.
    w.agree(TableFilter::InCrate, |o| o.in_crates, &["clean", "low"]);
    w.agree(
        TableFilter::InNoCrate,
        |o| o.in_no_crate,
        &["gone", "old", "unjudged", "nofile"],
    );
}

#[test]
fn the_energy_chips_open_the_tracks_each_bar_counts() {
    let w = world();
    let o = w.overview();
    // 3: "low" (Mixed In Key's) and "gone" (the user's); 4: "old" (the local
    // estimate beats the model's 9); 7: "clean" (the user's beats Mixed In
    // Key's) and "unjudged"; 9: nobody.
    let levels: [(u8, &[&str]); 4] = [
        (3, &["low", "gone"]),
        (4, &["old"]),
        (7, &["clean", "unjudged"]),
        (9, &[]),
    ];
    for (level, names) in levels {
        let (total, found) = w.rows(vec![TableFilter::Energy { level }]);
        assert_eq!(total, o.energy[usize::from(level) - 1], "energy {level}");
        assert_eq!(found, w.named(names), "energy {level}: the tracks");
    }
    for level in 1..=10u8 {
        let (total, _) = w.rows(vec![TableFilter::Energy { level }]);
        assert_eq!(total, o.energy[usize::from(level) - 1], "energy {level}");
    }
    w.agree(TableFilter::NoEnergy, |o| o.no_energy, &["nofile"]);
}

#[test]
fn a_door_chip_and_a_search_narrow_the_same_rows() {
    let w = world();
    let both = run(
        &w.lib,
        TableQuery {
            search: Some("low".to_owned()),
            filters: vec![TableFilter::LowQuality],
            ..query(TableSource::Library)
        },
    );
    assert_eq!(ids(&both), w.named(&["low"]));
    assert_eq!(both.total, Some(1));
    // Every track but one has the artist "Someone": the chip alone finds two,
    // the search alone finds more, together they are the ones in both.
    let by_artist = run(
        &w.lib,
        TableQuery {
            search: Some("Someone".to_owned()),
            filters: vec![TableFilter::LowQuality],
            ..query(TableSource::Library)
        },
    );
    let mut found = ids(&by_artist);
    found.sort_unstable();
    assert_eq!(found, w.named(&["low", "gone"]));
    assert_eq!(by_artist.total, Some(2));
}

#[test]
fn door_chips_combine_with_the_other_filters_and_a_sort_keeps_them() {
    let w = world();
    // Low quality and in no crate: "gone" only ("low" is in Warmup).
    let (_, found) = w.rows(vec![TableFilter::LowQuality, TableFilter::InNoCrate]);
    assert_eq!(found, w.named(&["gone"]));
    // Sorting changes the order, not the rows.
    let sorted_page = run(
        &w.lib,
        TableQuery {
            filters: vec![TableFilter::LowQuality],
            sort: Some(TableSort {
                column: SortColumn::Title,
                descending: true,
            }),
            ..query(TableSource::Library)
        },
    );
    assert_eq!(titles(&sorted_page), ["low", "gone"]);
    assert_eq!(sorted_page.total, Some(2));
}

#[test]
fn the_windows_of_a_door_chip_lay_end_to_end_into_its_rows() {
    let w = world();
    let filters = vec![TableFilter::NotAnalyzedByRekordbox];
    let whole = run(&w.lib, filtered(TableSource::Library, filters.clone()));
    let mut pieces = Vec::new();
    for offset in 0..whole.total.unwrap() {
        let piece = run(
            &w.lib,
            TableQuery {
                offset,
                limit: 1,
                filters: filters.clone(),
                sort: Some(TableSort {
                    column: SortColumn::Title,
                    descending: false,
                }),
                ..query(TableSource::Library)
            },
        );
        pieces.extend(ids(&piece));
    }
    let mut all = ids(&whole);
    all.sort_unstable();
    pieces.sort_unstable();
    assert_eq!(pieces, all);
}

#[test]
fn the_door_filters_list_only_library_tracks_whatever_the_source() {
    let w = world();
    // A track outside the Library is neither analyzed nor not analyzed, and
    // its energy is neither a level nor "none": a door never lists it.
    for filter in [
        TableFilter::NotAnalyzedByRekordbox,
        TableFilter::AnalyzedByRekordbox,
        TableFilter::NoEnergy,
        TableFilter::LowQuality,
    ] {
        let (_, in_library) = w.rows(vec![filter.clone()]);
        assert!(!in_library.contains(&w.plain_outside), "{filter:?}");
        let mut in_all_music = ids(&run(
            &w.lib,
            filtered(TableSource::AllMusic, vec![filter.clone()]),
        ));
        in_all_music.sort_unstable();
        assert!(
            !in_all_music.contains(&w.plain_outside),
            "{filter:?} in All music"
        );
        let library_ids: Vec<i64> = w.tracks.iter().map(|(_, id)| *id).collect();
        assert!(
            in_all_music.iter().all(|id| library_ids.contains(id)),
            "{filter:?} in All music lists a track outside the Library"
        );
    }
    let mut not_analyzed = ids(&run(
        &w.lib,
        filtered(
            TableSource::AllMusic,
            vec![TableFilter::NotAnalyzedByRekordbox],
        ),
    ));
    not_analyzed.sort_unstable();
    assert_eq!(not_analyzed, w.named(&["gone", "old", "nofile"]));
}

// Missing tags and missing art (1cC-U1b): the Overview counts two flag bits
// the index job keeps, and the two chips list the same bits.

/// Sets a file's tags to `fields` (as `Lib::tag` does), with a cover picture
/// if `cover` says so.
fn tagged(lib: &Lib, file: i64, fields: &[(&str, &str)], cover: bool) {
    let frame = |field: &str| match field {
        "Name" => "TIT2",
        "Artist" => "TPE1",
        "Genre" => "TCON",
        other => panic!("no frame for {other}"),
    };
    let mut items: Vec<serde_json::Value> = fields
        .iter()
        .map(|(field, text)| {
            serde_json::json!({"key": frame(field), "value": {"type": "text", "text": text}})
        })
        .collect();
    if cover {
        items.push(serde_json::json!({"key": "APIC", "value": {"type": "binary", "len": 50000}}));
    }
    let raw = serde_json::json!({ "id3v2": items }).to_string();
    lib.insert("UPDATE file SET raw_tags = ?1 WHERE id = ?2", (raw, file));
}

/// A Library of the edge cases of both numbers.
fn gaps_world() -> Doors {
    let lib = Lib::new();
    let mut tracks: Vec<(&'static str, i64)> = Vec::new();
    let full = [("Name", "T"), ("Artist", "A"), ("Genre", "House")];
    let mut library = |name: &'static str, file_name: &str| {
        let (track, file) = lib.track(None, None, file_name);
        lib.library(track, file);
        tracks.push((name, track));
        file
    };
    let complete = library("complete", "complete.mp3");
    let no_genre = library("no genre", "no genre.mp3");
    let no_cover = library("no cover", "no cover.mp3");
    let empty_title = library("empty title", "empty title.mp3");
    let known = library("known", "known.mp3");
    let blank_genre = library("known, blank genre", "known blank.mp3");
    let unread = library("unread", "unread.mp3");
    let gone = library("gone", "gone.mp3");
    tagged(&lib, complete, &full, true);
    tagged(&lib, no_genre, &[("Name", "T"), ("Artist", "A")], true);
    tagged(&lib, no_cover, &full, false);
    tagged(
        &lib,
        empty_title,
        &[("Name", ""), ("Artist", "A"), ("Genre", "House")],
        true,
    );
    // rekordbox has the track: its values are the answer, whatever the file
    // says, and the file's picture is still the file's.
    tagged(&lib, known, &[], true);
    lib.rekordbox_entry(
        known,
        &[("TrackID", "8"), ("Name", "N"), ("Artist", "A"), ("Genre", "House")],
    );
    tagged(&lib, blank_genre, &full, true);
    lib.rekordbox_entry(
        blank_genre,
        &[("TrackID", "9"), ("Name", "N"), ("Artist", "A"), ("Genre", "")],
    );
    // The tags were never read: nothing says art is missing.
    lib.rekordbox_entry(
        unread,
        &[("TrackID", "10"), ("Name", "N"), ("Artist", "A"), ("Genre", "House")],
    );
    // (relink sets the track beside the file.)
    lib.exec(
        "UPDATE rekordbox_track
         SET recording_id = (SELECT recording_id FROM recording_file WHERE file_id = rekordbox_track.file_id)",
    );
    // A file that is gone still has the tags that were stored.
    tagged(&lib, gone, &full, true);
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (gone,));
    // No linked file at all.
    let (nofile, _) = lib.track(None, None, "nofile.mp3");
    lib.insert(
        "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
         VALUES (?1, 'linked', NULL, 'missing')",
        (nofile,),
    );
    tracks.push(("nofile", nofile));
    // Outside the Library, with no tags and no picture: no chip lists it.
    let (plain_outside, _) = lib.track(None, None, "plain.mp3");
    Doors {
        lib,
        tracks,
        plain_outside,
    }
}

#[test]
fn the_missing_tags_chip_opens_the_tracks_the_overview_counts() {
    let w = gaps_world();
    w.agree(
        TableFilter::MissingTags,
        |o| o.missing_tags,
        &["no genre", "empty title", "known, blank genre", "nofile"],
    );
}

#[test]
fn the_missing_art_chip_opens_the_tracks_the_overview_counts() {
    let w = gaps_world();
    // Tags never read are unknown, not missing; a track with no linked file
    // has nothing to look at.
    w.agree(TableFilter::MissingArt, |o| o.missing_art, &["no cover"]);
}

#[test]
fn the_chips_list_the_tracks_the_per_track_definitions_find() {
    let w = gaps_world();
    w.lib.settle();
    let tags = w
        .lib
        .writer
        .call(|c| {
            let mut out = Vec::new();
            for id in crate::overview::missing_tag_ids(c, &Mount, true, None)? {
                out.push(c.query_row(
                    "SELECT recording_id FROM library_track WHERE id = ?1",
                    [id],
                    |r| r.get::<_, i64>(0),
                )?);
            }
            out.sort_unstable();
            Ok(out)
        })
        .unwrap();
    assert_eq!(w.rows(vec![TableFilter::MissingTags]).1, tags);
}

#[test]
fn the_gap_chips_work_with_the_other_filters_a_search_and_a_sort() {
    let w = gaps_world();
    // Both at once: the tracks with a tag gap and no picture. "no genre" has
    // a cover, "no cover" has all its tags: nobody is in both.
    let both = w.rows(vec![TableFilter::MissingTags, TableFilter::MissingArt]);
    assert_eq!(both, (0, Vec::new()));
    // Missing tags and not analyzed (nobody was): the same four.
    let (total, _) = w.rows(vec![
        TableFilter::MissingTags,
        TableFilter::NotAnalyzedByRekordbox,
    ]);
    assert_eq!(total, 4);
    // A search narrows the chip's rows.
    let found = run(
        &w.lib,
        TableQuery {
            search: Some("known".to_owned()),
            filters: vec![TableFilter::MissingTags],
            ..query(TableSource::Library)
        },
    );
    assert_eq!(ids(&found), w.named(&["known, blank genre"]));
    // A sort changes the order, not the rows.
    let sorted_page = run(
        &w.lib,
        TableQuery {
            filters: vec![TableFilter::MissingTags],
            sort: Some(TableSort {
                column: SortColumn::File,
                descending: true,
            }),
            ..query(TableSource::Library)
        },
    );
    assert_eq!(sorted_page.total, Some(4));
    // The chips list Library tracks only, in All music too.
    for filter in [TableFilter::MissingTags, TableFilter::MissingArt] {
        let in_all_music = ids(&run(
            &w.lib,
            filtered(TableSource::AllMusic, vec![filter.clone()]),
        ));
        assert!(!in_all_music.contains(&w.plain_outside), "{filter:?}");
        assert!(!w.rows(vec![filter.clone()]).1.contains(&w.plain_outside));
    }
}

#[test]
fn a_change_to_a_tag_moves_the_track_in_or_out_of_the_chip_once_the_index_has_run() {
    let w = gaps_world();
    w.agree(TableFilter::MissingArt, |o| o.missing_art, &["no cover"]);
    let file: i64 = w
        .lib
        .writer
        .call(|c| {
            c.query_row(
                "SELECT id FROM file WHERE rel_path = 'no cover.mp3'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    // The tag editor or a scan gives the file a picture.
    tagged(
        &w.lib,
        file,
        &[("Name", "T"), ("Artist", "A"), ("Genre", "House")],
        true,
    );
    assert!(w.lib.pending(), "the change marks the track due");
    w.agree(TableFilter::MissingArt, |o| o.missing_art, &[]);
}
