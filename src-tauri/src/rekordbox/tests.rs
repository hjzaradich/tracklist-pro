//! Whole-file tests. Every fixture is synthetic: made-up titles, artists
//! and paths only.

use proptest::prelude::*;

use super::*;
use crate::tags::key::{self, MusicalKey};

fn parse(xml: &str) -> RekordboxXml {
    RekordboxXml::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

fn parse_err(xml: &str) -> XmlError {
    match RekordboxXml::parse(xml.as_bytes()) {
        Ok(read) => panic!("expected an error, read {} tracks", read.tracks.len()),
        Err(e) => e,
    }
}

/// A whole export around these COLLECTION tracks and ROOT children.
/// ROOT has no `Count`, so tests needn't count nested nodes.
fn export(tracks: &str, root_children: &str) -> String {
    let count = tracks.matches("<TRACK ").count();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="{count}">
{tracks}
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT">
{root_children}
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#
    )
}

/// A plain track with an id and a path under `C:/Kit/`.
fn track(id: u64, file: &str) -> String {
    format!(
        r#"<TRACK TrackID="{id}" Name="Synthetic {id}" Location="file://localhost/C:/Kit/{file}"/>"#
    )
}

fn warnings_about(read: &RekordboxXml, attribute: &str) -> Vec<Warning> {
    read.warnings
        .iter()
        .filter(|w| matches!(&w.problem, Problem::BadValue { attribute: a, .. } if a == attribute))
        .cloned()
        .collect()
}

/// A track with every attribute rekordbox 7 writes, plus cues and a grid.
const FULL_TRACK: &str = r#"
    <TRACK TrackID="21899570" Name="Synthetic One" Artist="TLP Check" Composer="Comp"
           Album="Alb" Grouping="Red" Genre="TLP Genre" Kind="MP3 File" Size="6181152"
           TotalTime="255" DiscNumber="2" TrackNumber="7" Year="1999" AverageBpm="174.00"
           DateModified="2026-09-20" DateAdded="2026-09-25" BitRate="256" SampleRate="44100"
           Comments="Kit &amp; Kin&apos;s" PlayCount="3" LastPlayed="2026-09-27" Rating="204"
           Location="file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20-%20Low%20Tide%20#1%20(100%25%20Flip).mp3"
           Remixer="Remx" Tonality="4A" Label="Lbl" Mix="Extended Mix" Colour="0xFF0000"
           FutureField="kept">
      <TEMPO Inizio="0.057" Bpm="174.00" Metro="4/4" Battito="1"/>
      <TEMPO Inizio="120.402" Bpm="87.00" Metro="3/4" Battito="2"/>
      <POSITION_MARK Name="Drop" Type="0" Start="44.195" Num="0" Red="255" Green="55" Blue="111"/>
      <POSITION_MARK Name="" Type="0" Start="45.574" Num="-1"/>
      <POSITION_MARK Name="Roll" Type="4" Start="60.000" End="62.500" Num="1" Red="0" Green="0" Blue="255"/>
    </TRACK>"#;

// --- 1aA-8: COLLECTION ---

#[test]
fn every_track_attribute_is_kept_as_read_in_file_order_including_unknown_ones() {
    let read = parse(&export(FULL_TRACK, ""));
    let t = &read.tracks[0];
    let names: Vec<&str> = t.attrs.iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "TrackID",
            "Name",
            "Artist",
            "Composer",
            "Album",
            "Grouping",
            "Genre",
            "Kind",
            "Size",
            "TotalTime",
            "DiscNumber",
            "TrackNumber",
            "Year",
            "AverageBpm",
            "DateModified",
            "DateAdded",
            "BitRate",
            "SampleRate",
            "Comments",
            "PlayCount",
            "LastPlayed",
            "Rating",
            "Location",
            "Remixer",
            "Tonality",
            "Label",
            "Mix",
            "Colour",
            "FutureField",
        ]
    );
    // Values are kept exactly, numbers included ("174.00", not 174).
    assert_eq!(t.attrs.get("AverageBpm"), Some("174.00"));
    assert_eq!(t.attrs.get("FutureField"), Some("kept"));
    assert_eq!(t.attrs.get("Comments"), Some("Kit & Kin's"));
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
}

#[test]
fn known_track_attributes_are_parsed_into_typed_fields() {
    let read = parse(&export(FULL_TRACK, ""));
    let t = &read.tracks[0];
    assert_eq!(t.track_id, Some(21899570));
    assert_eq!(
        t.location.as_ref().unwrap().as_file().unwrap().as_str(),
        "C:/Kit/tracks/04 Kit & Kin - Low Tide #1 (100% Flip).mp3"
    );
    assert_eq!(t.size, Some(6181152));
    assert_eq!(t.total_time, Some(255));
    assert_eq!(t.disc_number, Some(2));
    assert_eq!(t.track_number, Some(7));
    assert_eq!(t.year, Some(1999));
    assert_eq!(t.average_bpm, Some(174.0));
    assert_eq!(
        t.date_modified,
        Some(Date {
            year: 2026,
            month: 9,
            day: 20
        })
    );
    assert_eq!(
        t.date_added,
        Some(Date {
            year: 2026,
            month: 9,
            day: 25
        })
    );
    assert_eq!(t.bit_rate, Some(256));
    assert_eq!(t.sample_rate, Some(44100));
    assert_eq!(t.play_count, Some(3));
    assert_eq!(
        t.last_played,
        Some(Date {
            year: 2026,
            month: 9,
            day: 27
        })
    );
    assert_eq!(t.rating, Some(204));
    assert_eq!(t.stars(), Some(4));
    assert_eq!(t.colour, Some(Rgb { r: 255, g: 0, b: 0 }));
    assert_eq!(t.key, key::parse("4A"));
    assert_eq!(
        (
            t.name(),
            t.artist(),
            t.composer(),
            t.album(),
            t.grouping(),
            t.genre(),
            t.kind()
        ),
        (
            "Synthetic One",
            "TLP Check",
            "Comp",
            "Alb",
            "Red",
            "TLP Genre",
            "MP3 File"
        )
    );
    assert_eq!(
        (t.remixer(), t.tonality(), t.label(), t.mix(), t.comments()),
        ("Remx", "4A", "Lbl", "Extended Mix", "Kit & Kin's")
    );
}

#[test]
fn rekordbox_zero_for_none_reads_as_none_but_a_zero_play_count_or_rating_stays() {
    let read = parse(&export(
        r#"<TRACK TrackID="1" Location="file://localhost/C:/a.mp3" Year="0" DiscNumber="0"
                  TrackNumber="0" AverageBpm="0.00" PlayCount="0" Rating="0" LastPlayed=""/>"#,
        "",
    ));
    let t = &read.tracks[0];
    assert_eq!(
        (t.year, t.disc_number, t.track_number, t.average_bpm),
        (None, None, None, None)
    );
    assert_eq!(t.play_count, Some(0), "PlayCount=0 is a real count (§5.2)");
    assert_eq!(t.rating, Some(0));
    assert_eq!(t.last_played, None);
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
}

#[test]
fn missing_attributes_leave_fields_empty_without_warnings() {
    let read = parse(&export(
        r#"<TRACK TrackID="5" Location="file://localhost/C:/a.mp3"/>"#,
        "",
    ));
    let t = &read.tracks[0];
    assert_eq!(
        (t.size, t.rating, t.key, t.colour),
        (None, None, None, None)
    );
    assert_eq!(t.name(), "");
    assert!(read.warnings.is_empty());
}

#[test]
fn cues_keep_their_type_start_end_number_color_and_name_in_order() {
    let read = parse(&export(FULL_TRACK, ""));
    let cues = &read.tracks[0].cues;
    assert_eq!(cues.len(), 3);
    assert_eq!(cues[0].kind, Some(CueKind::Cue));
    assert_eq!(cues[0].start, Some(44.195));
    assert_eq!(cues[0].end, None);
    assert_eq!(cues[0].slot(), Some(CueSlot::Hot(0)));
    assert_eq!(
        cues[0].color,
        Some(Rgb {
            r: 255,
            g: 55,
            b: 111
        })
    );
    assert_eq!(cues[0].name(), "Drop");
    assert_eq!(cues[1].slot(), Some(CueSlot::Memory));
    assert_eq!(cues[1].color, None);
    assert_eq!(cues[2].kind, Some(CueKind::Loop));
    assert_eq!((cues[2].start, cues[2].end), (Some(60.0), Some(62.5)));
    assert_eq!(cues[2].slot(), Some(CueSlot::Hot(1)));
    // Their attributes are kept as read, for the writer.
    assert_eq!(cues[0].attrs.get("Start"), Some("44.195"));
}

#[test]
fn every_cue_type_rekordbox_writes_is_recognized() {
    let marks: String = (0..5)
        .map(|t| format!(r#"<POSITION_MARK Name="" Type="{t}" Start="1.0" Num="-1"/>"#))
        .collect();
    let read = parse(&export(
        &format!(r#"<TRACK TrackID="1" Location="file://localhost/C:/a.mp3">{marks}</TRACK>"#),
        "",
    ));
    let kinds: Vec<_> = read.tracks[0].cues.iter().map(|c| c.kind).collect();
    use CueKind::*;
    assert_eq!(
        kinds,
        [
            Some(Cue),
            Some(FadeIn),
            Some(FadeOut),
            Some(Load),
            Some(Loop)
        ]
    );
}

#[test]
fn beat_grid_entries_keep_start_bpm_meter_and_beat_in_order() {
    let read = parse(&export(FULL_TRACK, ""));
    let grid = &read.tracks[0].tempos;
    assert_eq!(grid.len(), 2);
    assert_eq!(grid[0].start, Some(0.057));
    assert_eq!(grid[0].bpm, Some(174.0));
    assert_eq!(grid[0].meter, Some(Meter { beats: 4, unit: 4 }));
    assert_eq!(grid[0].beat, Some(1));
    assert_eq!(grid[1].start, Some(120.402));
    assert_eq!(grid[1].meter, Some(Meter { beats: 3, unit: 4 }));
    assert_eq!(grid[1].beat, Some(2));
    assert_eq!(grid[1].attrs.get("Bpm"), Some("87.00"));
}

#[test]
fn a_bad_value_clears_that_field_and_records_a_warning_naming_its_track() {
    let read = parse(&export(
        r#"<TRACK TrackID="7" Location="file://localhost/C:/a.mp3" Size="big" Rating="300"
                  AverageBpm="fast" DateAdded="2026-02-30" Colour="red" Name="Still Read">
             <TEMPO Inizio="x" Bpm="120.00" Metro="four" Battito="1"/>
             <POSITION_MARK Type="9" Start="-1" Num="-5" Red="255"/>
           </TRACK>"#,
        "",
    ));
    let t = &read.tracks[0];
    assert_eq!(
        (t.size, t.rating, t.average_bpm, t.date_added, t.colour),
        (None, None, None, None, None)
    );
    assert_eq!(t.name(), "Still Read", "the rest of the track is read");
    assert_eq!(
        t.attrs.get("Size"),
        Some("big"),
        "the raw value is kept for the writer"
    );
    assert_eq!(t.tempos[0].start, None);
    assert_eq!(t.tempos[0].bpm, Some(120.0));
    assert_eq!(t.tempos[0].meter, None);
    let cue = &t.cues[0];
    assert_eq!(
        (cue.kind, cue.start, cue.num, cue.color),
        (None, None, None, None)
    );

    for attribute in [
        "Size",
        "Rating",
        "AverageBpm",
        "DateAdded",
        "Colour",
        "Inizio",
        "Metro",
        "Type",
        "Start",
        "Num",
    ] {
        let found = warnings_about(&read, attribute);
        assert_eq!(found.len(), 1, "{attribute}: {:?}", read.warnings);
        assert_eq!(found[0].track_id, Some(7), "{attribute}");
    }
    assert!(read
        .warnings
        .iter()
        .any(|w| w.problem == Problem::PartialColor));
}

#[test]
fn a_track_with_bad_values_never_fails_the_file() {
    let tracks = [
        track(1, "one.mp3"),
        // A bad entity, a Location that doesn't decode.
        r#"<TRACK TrackID="2" Artist="A &nbsp; B" Location="file://localhost/C:/bad%zz.mp3" Size="12"/>"#.into(),
        r#"<TRACK TrackID="3" Location="file://localhost/C:/%c3.mp3" Genre="Bad &amp Name"/>"#.into(),
        track(4, "four.mp3"),
    ]
    .join("
");
    let read = parse(&export(&tracks, ""));
    assert_eq!(read.tracks.len(), 4);
    let two = &read.tracks[1];
    assert_eq!(two.track_id, Some(2));
    assert_eq!(two.size, Some(12));
    assert_eq!(two.artist(), "A &nbsp; B", "kept as written");
    assert_eq!(two.location, Err(LocationError::BadEscape { at: 23 }));
    assert_eq!(read.tracks[2].location, Err(LocationError::NotUtf8));
    assert_eq!(read.tracks[3].name(), "Synthetic 4");
    let problems: Vec<&Problem> = read
        .warnings
        .iter()
        .filter(|w| w.track_id == Some(2))
        .map(|w| &w.problem)
        .collect();
    assert!(problems
        .iter()
        .any(|p| matches!(p, Problem::BadEntity { attribute, .. } if attribute == "Artist")));
    assert!(problems
        .iter()
        .any(|p| matches!(p, Problem::BadLocation { .. })));
    assert!(read.warnings.iter().any(|w| w.track_id == Some(3)
        && matches!(&w.problem, Problem::BadEntity { attribute, .. } if attribute == "Genre")));
}

#[test]
fn a_malformed_or_repeated_attribute_refuses_the_file() {
    // Dropping the attribute would make a later send omit it, and an
    // omitted Rating resets rekordbox's (§5.2), so the file is refused.
    for bad in [
        r#"<TRACK TrackID="1" Rating=255 Location="file://localhost/C:/a.mp3"/>"#,
        r#"<TRACK TrackID="1" Rating Location="file://localhost/C:/a.mp3"/>"#,
        r#"<TRACK TrackID="1" Name="First" Name="Second" Location="file://localhost/C:/a.mp3"/>"#,
        r#"<TRACK TrackID="1" Location="file://localhost/C:/a.mp3"><POSITION_MARK Start=1.0/></TRACK>"#,
    ] {
        let tracks = [track(7, "fine.mp3"), bad.to_owned()].join(
            "
",
        );
        let result = RekordboxXml::parse(export(&tracks, "").as_bytes());
        assert!(
            matches!(&result, Err(XmlError::Malformed { detail, .. }) if detail.contains("bad attribute")),
            "{bad}: {:?}",
            result.map(|r| r.tracks.len())
        );
    }
    let bad_entry = r#"<NODE Name="p" Type="1" KeyType="0" Entries="1"><TRACK Key=7/></NODE>"#;
    assert!(matches!(
        parse_err(&export(&track(7, "fine.mp3"), bad_entry)),
        XmlError::Malformed { .. }
    ));
}

#[test]
fn an_escaped_entity_is_decoded_once_not_twice() {
    let read = parse(&export(
        r#"<TRACK TrackID="1" Name="a&amp;amp;amp;b" Location="file://localhost/C:/a.mp3"/>"#,
        "",
    ));
    assert_eq!(read.tracks[0].name(), "a&amp;amp;b");
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
}

#[test]
fn entities_resolve_in_every_attribute() {
    let read = parse(&export(
        r#"<TRACK TrackID="1" Name="&lt;Intro&gt; &quot;Q&quot; &#233;&#x1F680;"
                  Location="file://localhost/C:/Kit/A%20&amp;%20B.mp3"/>"#,
        "",
    ));
    let t = &read.tracks[0];
    assert_eq!(t.name(), "<Intro> \"Q\" é🚀");
    assert_eq!(
        t.location.as_ref().unwrap().as_file().unwrap().as_str(),
        "C:/Kit/A & B.mp3"
    );
}

#[test]
fn a_utf8_byte_order_mark_is_ignored() {
    let xml = format!("\u{feff}{}", export(&track(1, "a.mp3"), ""));
    assert_eq!(parse(&xml).tracks.len(), 1);
}

#[test]
fn utf16_and_non_utf8_encodings_are_refused() {
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("<DJ_PLAYLISTS/>".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    assert!(matches!(
        RekordboxXml::parse(&utf16[..]),
        Err(XmlError::UnsupportedEncoding(_))
    ));
    let latin1 = export("", "").replace("UTF-8", "ISO-8859-1");
    assert!(matches!(parse_err(&latin1), XmlError::UnsupportedEncoding(e) if e == "ISO-8859-1"));
}

#[test]
fn tonality_in_any_notation_gets_a_normalized_key_and_keeps_its_raw_value() {
    let notations = [
        ("8A", "Am"),
        ("08A", "Am"),
        ("Am", "Am"),
        ("F#m", "F#m"),
        ("Gbm", "F#m"),
        ("1m", "Am"),
        ("Db", "Db"),
        ("C#", "Db"),
        ("A minor", "Am"),
    ];
    let tracks: String = notations
        .iter()
        .enumerate()
        .map(|(i, (tonality, _))| {
            format!(r#"<TRACK TrackID="{i}" Location="file://localhost/C:/{i}.mp3" Tonality="{tonality}"/>"#)
        })
        .collect();
    let read = parse(&export(&tracks, ""));
    for (t, (raw, musical)) in read.tracks.iter().zip(notations) {
        assert_eq!(t.tonality(), raw);
        assert_eq!(
            t.key.map(MusicalKey::to_musical).as_deref(),
            Some(musical),
            "{raw:?}"
        );
    }
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
}

#[test]
fn a_tonality_no_notation_reads_keeps_its_raw_value_and_warns() {
    let read = parse(&export(
        r#"<TRACK TrackID="1" Location="file://localhost/C:/a.mp3" Tonality="H-moll"/>"#,
        "",
    ));
    assert_eq!(read.tracks[0].key, None);
    assert_eq!(read.tracks[0].tonality(), "H-moll");
    assert_eq!(
        read.warnings[0].problem,
        Problem::UnknownKey {
            value: "H-moll".into()
        }
    );
}

#[test]
fn a_file_cut_off_anywhere_is_refused_rather_than_read_short() {
    let tracks: String = (1..=3).map(|i| track(i, &format!("{i}.mp3"))).collect();
    let xml = export(
        &tracks,
        r#"<NODE Name="p" Type="1" KeyType="0" Entries="1"><TRACK Key="1"/></NODE>"#,
    );
    let end = xml.find("</DJ_PLAYLISTS>").unwrap() + "</DJ_PLAYLISTS>".len();
    for cut in 0..end {
        let result = RekordboxXml::parse(&xml.as_bytes()[..cut]);
        assert!(
            matches!(
                result,
                Err(XmlError::Truncated | XmlError::Malformed { .. })
            ),
            "cut at {cut}: {:?}",
            result.map(|r| r.tracks.len())
        );
    }
    assert_eq!(
        RekordboxXml::parse(&xml.as_bytes()[..end])
            .unwrap()
            .tracks
            .len(),
        3
    );
}

#[test]
fn bytes_that_arent_utf8_make_the_file_malformed_not_a_garbled_read() {
    let xml = export(&track(1, "a.mp3"), "");
    let at = xml.find("Synthetic 1").unwrap();
    let mut bytes = xml.into_bytes();
    bytes[at] = 0xFF;
    assert!(matches!(
        RekordboxXml::parse(&bytes[..]),
        Err(XmlError::Malformed { .. })
    ));
}

#[test]
fn mismatched_tags_are_refused_with_their_position() {
    let xml = export(&track(1, "a.mp3"), "").replace("</COLLECTION>", "</COLLECTIONS>");
    assert!(matches!(parse_err(&xml), XmlError::Malformed { offset, .. } if offset > 0));
}

#[test]
fn files_that_arent_collection_exports_are_refused() {
    assert!(
        matches!(parse_err("<plist><dict/></plist>"), XmlError::NotRekordboxXml { root } if root == "plist")
    );
    assert!(matches!(
        parse_err("<DJ_PLAYLISTS Version=\"1.0.0\"><PLAYLISTS/></DJ_PLAYLISTS>"),
        XmlError::NoCollection
    ));
    assert!(matches!(parse_err(""), XmlError::Truncated));
    assert!(matches!(
        parse_err("<DJ_PLAYLISTS><COLLECTION/></DJ_PLAYLISTS><DJ_PLAYLISTS/>"),
        XmlError::Malformed { .. }
    ));
}

#[test]
fn an_empty_collection_parses_to_no_tracks() {
    let read = parse(&export("", ""));
    assert!(read.tracks.is_empty());
    assert_eq!(read.declared_entries, Some(0));
    assert_eq!(read.playlists.name, "ROOT");
    assert_eq!(read.version.as_deref(), Some("1.0.0"));
    assert_eq!(
        read.product,
        Some(Product {
            name: "rekordbox".into(),
            version: "7.2.19".into(),
            company: "AlphaTheta".into()
        })
    );
}

#[test]
fn a_collection_count_that_disagrees_with_its_tracks_is_warned() {
    let xml = export(&track(1, "a.mp3"), "").replace(r#"Entries="1""#, r#"Entries="2""#);
    let read = parse(&xml);
    assert!(read.warnings.iter().any(|w| w.problem
        == Problem::CountMismatch {
            element: Element::Collection,
            declared: 2,
            found: 1
        }));
}

#[test]
fn duplicate_track_ids_and_locations_are_warned_and_both_tracks_kept() {
    let tracks = [track(1, "a.mp3"), track(1, "b.mp3"), track(2, "A.MP3")].join("\n");
    let read = parse(&export(&tracks, ""));
    assert_eq!(read.tracks.len(), 3);
    assert!(read
        .warnings
        .iter()
        .any(|w| w.problem == Problem::DuplicateTrackId { track_id: 1 }));
    assert!(read
        .warnings
        .iter()
        .any(|w| w.track_id == Some(2) && matches!(w.problem, Problem::DuplicateLocation { .. })));
}

#[test]
fn unexpected_elements_are_skipped_with_everything_inside_them() {
    let read = parse(&export(
        r#"<TRACK TrackID="1" Location="file://localhost/C:/a.mp3">
             <EXTRA a="1"><TEMPO Inizio="0" Bpm="1" Metro="4/4" Battito="1"/></EXTRA>
             <TEMPO Inizio="0.5" Bpm="120.00" Metro="4/4" Battito="1"/>
           </TRACK>
           <NOT_A_TRACK/>"#,
        "",
    ));
    assert_eq!(read.tracks.len(), 1);
    assert_eq!(
        read.tracks[0].tempos.len(),
        1,
        "the TEMPO inside EXTRA isn't read"
    );
    assert_eq!(read.tracks[0].tempos[0].start, Some(0.5));
    let unexpected: Vec<&str> = read
        .warnings
        .iter()
        .filter_map(|w| match &w.problem {
            Problem::UnexpectedElement { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(unexpected, ["EXTRA", "NOT_A_TRACK"]);
}

#[test]
fn the_rb_kit_export_snippet_parses_with_its_skips_counted() {
    let xml = include_str!("../../../spikes/rb-kit/fixtures/rb-export-snippet.xml");
    let read = parse(xml);
    // 6 rows: a sample, a demo track, two real tracks, a deleted row and
    // a streaming entry.
    assert_eq!(read.tracks.len(), 3);
    assert_eq!(
        read.skips.tracks,
        ReasonCounts {
            deleted: 1,
            demo_tracks: 1,
            samples: 1,
            streaming: 1
        }
    );
    let names: Vec<&str> = read.tracks.iter().map(Track::name).collect();
    assert_eq!(names, ["Synthetic One", "Synthetic Two", "Streamed"]);
    assert_eq!(
        read.tracks[1]
            .location
            .as_ref()
            .unwrap()
            .as_file()
            .unwrap()
            .as_str(),
        "C:/Kit/Sub Folder/Deeper Level/06 North Wind, Late Train 🚀 Café 日本 A + B [x].m4a"
    );
    assert_eq!(read.skips.cue_analysis_playlists, 1);
    let lists = read.playlists.playlists();
    let by_id = lists.iter().find(|(_, p)| p.name == "by-id").unwrap().1;
    assert_eq!(
        by_id.entries.iter().map(|e| e.target).collect::<Vec<_>>(),
        [
            EntryTarget::Track(1),
            EntryTarget::Track(0),
            EntryTarget::NoTrack
        ]
    );
    let by_location = lists.iter().find(|(_, p)| p.name == "by-location").unwrap();
    assert_eq!(by_location.0, ["TLP Check", "nested"]);
    assert_eq!(by_location.1.entries[0].target, EntryTarget::Track(0));
    assert_eq!(read.entries_without_track, 1);
}

#[test]
fn reading_an_export_leaves_the_file_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.xml");
    let xml = export(FULL_TRACK, "");
    std::fs::write(&path, &xml).unwrap();
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    let read = RekordboxXml::read_file(&path).unwrap();
    assert_eq!(read.tracks.len(), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), xml);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
    assert!(matches!(
        RekordboxXml::read_file(&dir.path().join("missing.xml")),
        Err(XmlError::Io(_))
    ));
}

// --- 1aA-9: PLAYLISTS ---

const TREE: &str = r#"
      <NODE Name="Warmup" Type="1" KeyType="0" Entries="2">
        <TRACK Key="3"/>
        <TRACK Key="1"/>
      </NODE>
      <NODE Type="0" Name="Gigs" Count="2">
        <NODE Type="0" Name="2026" Count="1">
          <NODE Name="Club Night" Type="1" KeyType="1" Entries="2">
            <TRACK Key="file://localhost/C:/Kit/two.mp3"/>
            <TRACK Key="file://localhost/C:/Kit/one.mp3"/>
          </NODE>
        </NODE>
        <NODE Name="Empty" Type="1" KeyType="0" Entries="0"/>
      </NODE>
      <NODE Type="0" Name="Gigs" Count="0"/>"#;

fn three_tracks() -> String {
    [
        track(1, "one.mp3"),
        track(2, "two.mp3"),
        track(3, "three.mp3"),
    ]
    .join("\n")
}

#[test]
fn folders_and_playlists_keep_their_nesting_and_order() {
    let read = parse(&export(&three_tracks(), TREE));
    let root = &read.playlists;
    assert_eq!(root.name, "ROOT");
    let names: Vec<&str> = root.children.iter().map(Node::name).collect();
    assert_eq!(
        names,
        ["Warmup", "Gigs", "Gigs"],
        "same-name folders are both kept"
    );
    let Node::Folder(gigs) = &root.children[1] else {
        panic!()
    };
    let Node::Folder(year) = &gigs.children[0] else {
        panic!()
    };
    assert_eq!(year.name, "2026");
    assert!(
        matches!(&gigs.children[1], Node::Playlist(p) if p.name == "Empty" && p.entries.is_empty())
    );
    let paths: Vec<(Vec<&str>, &str)> = root
        .playlists()
        .into_iter()
        .map(|(path, p)| (path, p.name.as_str()))
        .collect();
    assert_eq!(
        paths,
        [
            (vec![], "Warmup"),
            (vec!["Gigs", "2026"], "Club Night"),
            (vec!["Gigs"], "Empty")
        ]
    );
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
}

#[test]
fn entries_resolve_by_track_id_and_by_location_in_playlist_order() {
    let read = parse(&export(&three_tracks(), TREE));
    let lists = read.playlists.playlists();
    let warmup = lists[0].1;
    assert_eq!(warmup.key_type, KeyType::TrackId);
    assert_eq!(
        warmup.entries,
        [
            Entry {
                key: "3".into(),
                target: EntryTarget::Track(2)
            },
            Entry {
                key: "1".into(),
                target: EntryTarget::Track(0)
            },
        ]
    );
    let club = lists[1].1;
    assert_eq!(club.key_type, KeyType::Location);
    let targets: Vec<_> = club.entries.iter().map(|e| e.target).collect();
    assert_eq!(targets, [EntryTarget::Track(1), EntryTarget::Track(0)]);
    assert_eq!(read.entries_without_track, 0);
}

#[test]
fn location_entries_match_any_spelling_of_the_same_path() {
    let tracks = r#"<TRACK TrackID="1" Location="file://localhost/C:/Kit/Low%20Tide%20#1%20(Caf%c3%a9).mp3"/>"#;
    let entries = [
        // Our writer's full encoding, uppercase hex.
        "file://localhost/C:/Kit/Low%20Tide%20%231%20%28Caf%C3%A9%29.mp3",
        // Other letter case (Windows ignores it), NFD é, raw characters.
        "file://localhost/c:/KIT/low tide #1 (cafe\u{301}).MP3",
    ];
    let keys: String = entries
        .iter()
        .map(|k| format!(r#"<TRACK Key="{k}"/>"#))
        .collect();
    let read = parse(&export(
        tracks,
        &format!(r#"<NODE Name="p" Type="1" KeyType="1" Entries="2">{keys}</NODE>"#),
    ));
    let targets: Vec<_> = read.playlists.playlists()[0]
        .1
        .entries
        .iter()
        .map(|e| e.target)
        .collect();
    assert_eq!(targets, [EntryTarget::Track(0), EntryTarget::Track(0)]);
}

#[test]
fn entries_that_point_at_no_track_are_kept_and_counted() {
    let read = parse(&export(
        &track(1, "one.mp3"),
        r#"<NODE Name="p" Type="1" KeyType="0" Entries="4">
             <TRACK Key="1"/><TRACK Key="999"/><TRACK Key="abc"/><TRACK Key=""/>
           </NODE>
           <NODE Name="q" Type="1" KeyType="1" Entries="2">
             <TRACK Key="file://localhost/C:/Kit/elsewhere.mp3"/><TRACK Key="file://localhost/C:/bad%zz"/>
           </NODE>"#,
    ));
    let lists = read.playlists.playlists();
    let targets: Vec<_> = lists[0].1.entries.iter().map(|e| e.target).collect();
    use EntryTarget::*;
    assert_eq!(targets, [Track(0), NoTrack, NoTrack, NoTrack]);
    assert_eq!(lists[0].1.entries[1].key, "999");
    assert_eq!(read.entries_without_track, 5);
}

#[test]
fn an_entry_resolves_to_the_first_of_two_tracks_sharing_a_location() {
    let read = parse(&export(
        &[track(1, "same.mp3"), track(2, "SAME.mp3")].join(
            "
",
        ),
        r#"<NODE Name="p" Type="1" KeyType="1" Entries="1">
             <TRACK Key="file://localhost/C:/Kit/same.mp3"/>
           </NODE>"#,
    ));
    assert_eq!(
        read.playlists.playlists()[0].1.entries[0].target,
        EntryTarget::Track(0)
    );
}

#[test]
fn an_entry_resolves_to_the_live_track_when_a_deleted_row_shares_its_id() {
    let read = parse(&export(
        r#"<TRACK TrackID="1" Location="file://localhost/C:/Kit/old.mp3" rb_local_deleted="1"/>
           <TRACK TrackID="1" Location="file://localhost/C:/Kit/live.mp3"/>"#,
        r#"<NODE Name="p" Type="1" KeyType="0" Entries="1"><TRACK Key="1"/></NODE>"#,
    ));
    assert_eq!(read.tracks.len(), 1);
    assert_eq!(
        read.playlists.playlists()[0].1.entries[0].target,
        EntryTarget::Track(0)
    );
    assert_eq!(read.skips.entries, ReasonCounts::default());
}

#[test]
fn a_track_id_key_with_a_sign_or_spaces_points_at_no_track() {
    let read = parse(&export(
        &track(1, "one.mp3"),
        r#"<NODE Name="p" Type="1" KeyType="0" Entries="3">
             <TRACK Key="+1"/><TRACK Key=" 1"/><TRACK Key="1"/>
           </NODE>"#,
    ));
    let targets: Vec<_> = read.playlists.playlists()[0]
        .1
        .entries
        .iter()
        .map(|e| e.target)
        .collect();
    assert_eq!(
        targets,
        [
            EntryTarget::NoTrack,
            EntryTarget::NoTrack,
            EntryTarget::Track(0)
        ]
    );
}

#[test]
fn playlist_folders_nested_past_the_limit_refuse_the_file_without_crashing() {
    let nested = |depth: usize| {
        let open = r#"<NODE Type="0" Name="f" Count="1">"#.repeat(depth - 1);
        let close = "</NODE>".repeat(depth - 1);
        export(
            "",
            &format!("{open}<NODE Name=\"p\" Type=\"1\" Entries=\"0\"/>{close}"),
        )
    };
    // ROOT plus 255 below it is the limit, and reads.
    let read = parse(&nested(reader::MAX_NODE_DEPTH - 1));
    assert_eq!(
        read.playlists.playlists()[0].0.len(),
        reader::MAX_NODE_DEPTH - 2
    );
    assert!(matches!(
        parse_err(&nested(reader::MAX_NODE_DEPTH)),
        XmlError::Malformed { .. }
    ));
    // Deep enough to overflow the stack if anything recursed that far.
    assert!(matches!(
        parse_err(&nested(20_000)),
        XmlError::Malformed { .. }
    ));
}

#[test]
fn a_collection_short_of_its_declared_count_is_not_complete() {
    let full = export(&track(1, "a.mp3"), "");
    assert!(parse(&full).is_complete());
    let short = full.replace(r#"Entries="1""#, r#"Entries="2""#);
    assert!(!parse(&short).is_complete());
    let undeclared = full.replace(r#"<COLLECTION Entries="1">"#, "<COLLECTION>");
    assert!(parse(&undeclared).is_complete());
}

#[test]
fn playlists_resolve_even_when_they_come_before_the_collection() {
    let xml = r#"<DJ_PLAYLISTS Version="1.0.0">
      <PLAYLISTS><NODE Type="0" Name="ROOT" Count="1">
        <NODE Name="p" Type="1" KeyType="0" Entries="1"><TRACK Key="1"/></NODE>
      </NODE></PLAYLISTS>
      <COLLECTION Entries="1"><TRACK TrackID="1" Location="file://localhost/C:/a.mp3"/></COLLECTION>
    </DJ_PLAYLISTS>"#;
    let read = parse(xml);
    assert_eq!(
        read.playlists.playlists()[0].1.entries[0].target,
        EntryTarget::Track(0)
    );
}

#[test]
fn a_file_without_playlists_has_an_empty_root() {
    let xml = r#"<DJ_PLAYLISTS><COLLECTION Entries="0"/></DJ_PLAYLISTS>"#;
    let read = parse(xml);
    assert_eq!(
        read.playlists,
        Folder {
            name: "ROOT".into(),
            children: vec![]
        }
    );
}

#[test]
fn odd_node_types_key_types_and_counts_are_warned_and_read_sensibly() {
    let read = parse(&export(
        &track(1, "one.mp3"),
        r#"<NODE Name="no type, entries" KeyType="0" Entries="1"><TRACK Key="1"/></NODE>
           <NODE Name="no type, children" Count="1"><NODE Name="x" Type="1" Entries="0"/></NODE>
           <NODE Name="bad keytype" Type="1" KeyType="7" Entries="3"><TRACK Key="1"/></NODE>"#,
    ));
    let kids = &read.playlists.children;
    assert!(matches!(&kids[0], Node::Playlist(p) if p.entries[0].target == EntryTarget::Track(0)));
    assert!(matches!(&kids[1], Node::Folder(f) if f.children.len() == 1));
    assert!(
        matches!(&kids[2], Node::Playlist(p) if p.key_type == KeyType::TrackId
        && p.entries[0].target == EntryTarget::Track(0))
    );
    let problems: Vec<&Problem> = read.warnings.iter().map(|w| &w.problem).collect();
    assert_eq!(
        problems
            .iter()
            .filter(|p| matches!(p, Problem::UnknownNodeType { .. }))
            .count(),
        2
    );
    assert!(problems
        .iter()
        .any(|p| matches!(p, Problem::BadValue { attribute, .. } if attribute == "KeyType")));
    assert!(problems.contains(&&Problem::CountMismatch {
        element: Element::Node,
        declared: 3,
        found: 1
    }));
}

#[test]
fn playlists_without_a_single_root_folder_are_gathered_under_one_root() {
    let xml = r#"<DJ_PLAYLISTS><COLLECTION Entries="0"/><PLAYLISTS>
        <NODE Name="a" Type="1" Entries="0"/><NODE Name="b" Type="0" Count="0"/>
      </PLAYLISTS></DJ_PLAYLISTS>"#;
    let read = parse(xml);
    let names: Vec<&str> = read.playlists.children.iter().map(Node::name).collect();
    assert_eq!(names, ["a", "b"]);
    assert!(read
        .warnings
        .iter()
        .any(|w| w.problem == Problem::UnusualPlaylistRoot { nodes: 2 }));
}

// --- 1aA-10: skip rules ---

const SKIPPABLE: &str = r#"
    <TRACK TrackID="1" Name="Mine" Location="file://localhost/C:/Kit/mine.mp3"/>
    <TRACK TrackID="2" Name="Gone" Location="file://localhost/C:/Kit/gone.mp3" rb_local_deleted="1"/>
    <TRACK TrackID="3" Name="Demo" Location="file://localhost/C:/Users/someone/Music/PioneerDJ/Demo%20Tracks/Demo%20Track%201.mp3"/>
    <TRACK TrackID="4" Name="Sample" Location="file://localhost/C:/Users/Another%20User/Music/rekordbox/Sampler/OSC_SAMPLER(3)/PRESET%20ONESHOT/NOISE.wav"/>
    <TRACK TrackID="5" Name="Cloud" Location="soundcloud:tracks:12345"/>
    <TRACK TrackID="6" Name="Stream" Location="file://localhost/spotify:track:abc"/>
    <TRACK TrackID="7" Name="Kept" Location="file://localhost/C:/Kit/kept.mp3" rb_local_deleted="0"/>"#;

#[test]
fn deleted_rows_demo_tracks_and_samples_are_left_out_and_counted_by_reason() {
    let read = parse(&export(SKIPPABLE, ""));
    let names: Vec<&str> = read.tracks.iter().map(Track::name).collect();
    assert_eq!(names, ["Mine", "Cloud", "Stream", "Kept"]);
    assert_eq!(
        read.skips.tracks,
        ReasonCounts {
            deleted: 1,
            demo_tracks: 1,
            samples: 1,
            streaming: 2
        }
    );
    assert_eq!(read.skips.tracks.total(), 5);
    let skipped: Vec<(Option<u64>, SkipReason)> = read
        .skipped
        .iter()
        .map(|s| (s.track_id, s.reason))
        .collect();
    assert_eq!(
        skipped,
        [
            (Some(2), SkipReason::Deleted),
            (Some(3), SkipReason::DemoTrack),
            (Some(4), SkipReason::Sample)
        ]
    );
}

#[test]
fn streaming_entries_are_kept_as_tracks_without_a_file_and_counted() {
    let read = parse(&export(SKIPPABLE, ""));
    let streaming: Vec<&Track> = read.tracks.iter().filter(|t| t.is_streaming()).collect();
    assert_eq!(streaming.len(), 2);
    assert_eq!(
        streaming[0].location,
        Ok(Location::Streaming(StreamingId {
            scheme: "soundcloud".into(),
            id: "tracks:12345".into()
        }))
    );
    assert_eq!(streaming[1].name(), "Stream");
    assert_eq!(read.skips.tracks.get(SkipReason::Streaming), 2);
}

#[test]
fn a_deleted_row_is_skipped_whatever_its_location() {
    let read = parse(&export(
        r#"<TRACK TrackID="1" Location="soundcloud:tracks:1" rb_local_deleted="1"/>
           <TRACK TrackID="2" Location="not a location" rb_local_deleted="1"/>"#,
        "",
    ));
    assert!(read.tracks.is_empty());
    assert_eq!(
        read.skips.tracks,
        ReasonCounts {
            deleted: 2,
            ..ReasonCounts::default()
        }
    );
}

#[test]
fn the_cue_analysis_playlist_is_left_out_and_counted_with_its_entries() {
    let read = parse(&export(
        SKIPPABLE,
        r#"<NODE Name="CUE Analysis Playlist" Type="1" KeyType="0" Entries="2"><TRACK Key="1"/><TRACK Key="7"/></NODE>
           <NODE Name="Mine" Type="1" KeyType="0" Entries="1"><TRACK Key="1"/></NODE>
           <NODE Type="0" Name="Folder" Count="1">
             <NODE Name="CUE Analysis Playlist" Type="1" KeyType="0" Entries="0"/>
           </NODE>"#,
    ));
    let names: Vec<&str> = read.playlists.children.iter().map(Node::name).collect();
    assert_eq!(names, ["Mine", "Folder"]);
    assert_eq!(read.skips.cue_analysis_playlists, 1);
    assert_eq!(read.skips.cue_analysis_entries, 2);
    // Only rekordbox's own, at the top: a user's playlist of that name in
    // a folder is theirs.
    let Node::Folder(folder) = &read.playlists.children[1] else {
        panic!()
    };
    assert_eq!(folder.children[0].name(), CUE_ANALYSIS);
    // Its entries aren't counted as pointing at skipped tracks.
    assert_eq!(read.skips.entries, ReasonCounts::default());
}

const CUE_ANALYSIS: &str = skip::CUE_ANALYSIS_PLAYLIST;

#[test]
fn playlist_entries_pointing_at_skipped_tracks_are_counted_by_reason() {
    let read = parse(&export(
        SKIPPABLE,
        r#"<NODE Name="by id" Type="1" KeyType="0" Entries="7">
             <TRACK Key="1"/><TRACK Key="2"/><TRACK Key="3"/><TRACK Key="4"/>
             <TRACK Key="5"/><TRACK Key="6"/><TRACK Key="2"/>
           </NODE>
           <NODE Name="by location" Type="1" KeyType="1" Entries="3">
             <TRACK Key="file://localhost/C:/Kit/gone.mp3"/>
             <TRACK Key="file://localhost/c:/users/someone/music/pioneerdj/demo%20tracks/demo%20track%201.mp3"/>
             <TRACK Key="soundcloud:tracks:12345"/>
           </NODE>"#,
    ));
    let lists = read.playlists.playlists();
    use EntryTarget::*;
    let by_id: Vec<_> = lists[0].1.entries.iter().map(|e| e.target).collect();
    assert_eq!(
        by_id,
        [
            Track(0),
            Skipped(SkipReason::Deleted),
            Skipped(SkipReason::DemoTrack),
            Skipped(SkipReason::Sample),
            Track(1),
            Track(2),
            Skipped(SkipReason::Deleted),
        ]
    );
    let by_location: Vec<_> = lists[1].1.entries.iter().map(|e| e.target).collect();
    assert_eq!(
        by_location,
        [
            Skipped(SkipReason::Deleted),
            Skipped(SkipReason::DemoTrack),
            Track(1)
        ]
    );
    assert_eq!(
        read.skips.entries,
        ReasonCounts {
            deleted: 3,
            demo_tracks: 2,
            samples: 1,
            streaming: 3
        }
    );
    assert_eq!(read.entries_without_track, 0);
}

// --- robustness ---

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn corrupting_any_bytes_never_panics_the_parser(
        edits in proptest::collection::vec((any::<proptest::sample::Index>(), any::<u8>()), 1..8),
    ) {
        let mut bytes = export(&format!("{FULL_TRACK}\n{SKIPPABLE}"), TREE).into_bytes();
        for (at, byte) in edits {
            let i = at.index(bytes.len());
            bytes[i] = byte;
        }
        let _ = RekordboxXml::parse(&bytes[..]);
    }
}
