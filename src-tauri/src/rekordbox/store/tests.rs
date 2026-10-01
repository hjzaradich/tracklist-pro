//! 1aB-10: the `rekordbox_track` snapshot. Every fixture is synthetic:
//! made-up titles, artists and paths only.

use proptest::prelude::*;
use rusqlite::Connection;

use super::*;
use crate::db::Writer;
use crate::rekordbox::RekordboxXml;

/// A migrated database in a temp dir.
fn db() -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    (dir, writer)
}

fn parse(xml: &str) -> RekordboxXml {
    RekordboxXml::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

/// A whole export around these tracks and ROOT children. `Entries` is the
/// number of tracks unless `entries` says otherwise.
fn export_with(tracks: &str, root_children: &str, entries: Option<usize>) -> String {
    let count = entries.unwrap_or_else(|| tracks.matches("<TRACK ").count());
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

fn export(tracks: &str) -> String {
    export_with(tracks, "", None)
}

fn track(id: u64, file: &str) -> String {
    format!(
        r#"<TRACK TrackID="{id}" Name="Synthetic {id}" Location="file://localhost/C:/Kit/{file}"/>"#
    )
}

fn store(writer: &Writer, xml: &str) -> SnapshotSummary {
    let rows = SnapshotRows::from_xml(&parse(xml));
    writer
        .call(move |c| replace_snapshot(c, &rows, |_, _, _| Ok(())))
        .unwrap()
}

/// Every stored row, as (TrackID, location_key, read_at), by TrackID.
fn stored(writer: &Writer) -> Vec<(i64, String, String)> {
    writer
        .call(|c| {
            c.prepare(
                "SELECT track_id, location_key, read_at FROM rekordbox_track ORDER BY track_id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect()
        })
        .unwrap()
}

fn column(writer: &Writer, track_id: i64, column: &'static str) -> String {
    writer
        .call(move |c| {
            c.query_row(
                &format!("SELECT {column} FROM rekordbox_track WHERE track_id = ?1"),
                [track_id],
                |r| r.get(0),
            )
        })
        .unwrap()
}

/// The JSON object text for these pairs, in this order, built without the
/// code under test.
fn object(pairs: &[(&str, &str)]) -> String {
    let parts: Vec<String> = pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}:{}",
                serde_json::to_string(k).unwrap(),
                serde_json::to_string(v).unwrap()
            )
        })
        .collect();
    format!("{{{}}}", parts.join(","))
}

/// Sets every stored row's `read_at` to a time long past, so a later read's
/// rows can be told apart. The snapshot's values are read-only, so this
/// swaps the trigger out for the moment.
fn age_snapshot(writer: &Writer) {
    writer
        .call(|c| {
            let trigger: String = c.query_row(
                "SELECT sql FROM sqlite_schema WHERE name = 'rekordbox_track_values_are_read_only'",
                [],
                |r| r.get(0),
            )?;
            c.execute_batch(&format!(
                "DROP TRIGGER rekordbox_track_values_are_read_only;
                 UPDATE rekordbox_track SET read_at = '2000-01-01T00:00:00.000Z';
                 {trigger};"
            ))
        })
        .unwrap();
}

const OLD: &str = "2000-01-01T00:00:00.000Z";

// --- attributes ---

const FULL_TRACK: &str = r#"
    <TRACK TrackID="21899570" Name="Synthetic One" Artist="TLP Check" Composer="Comp"
           Album="Alb" Grouping="Red" Genre="TLP Genre" Kind="MP3 File" Size="6181152"
           TotalTime="255" DiscNumber="0" TrackNumber="7" Year="1999" AverageBpm="174.00"
           DateModified="2026-09-20" DateAdded="2026-09-25" BitRate="256" SampleRate="44100"
           Comments="Kit &amp; Kin&apos;s &quot;dub&quot; &lt;3" PlayCount="0" LastPlayed="" Rating="204"
           Location="file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20#1%20(100%25%20Flip).mp3"
           Remixer="Remx" Tonality="4A" Label="Lbl" Mix="Extended Mix" Colour="0xFF0000"
           FutureField="kept" Zeta="last">
      <TEMPO Inizio="0.057" Bpm="174.00" Metro="4/4" Battito="1"/>
      <TEMPO Inizio="120.402" Bpm="87.00" Metro="3/4" Battito="2" Extra="x"/>
      <POSITION_MARK Name="Drop" Type="0" Start="44.195" Num="0" Red="255" Green="55" Blue="111"/>
      <POSITION_MARK Name="" Type="0" Start="45.574" Num="-1"/>
      <POSITION_MARK Name="Roll" Type="4" Start="60.000" End="62.500" Num="1" Red="0" Green="0" Blue="255"/>
    </TRACK>"#;

#[test]
fn every_track_attribute_is_stored_exactly_as_read_in_file_order_including_unknown_and_empty_ones()
{
    let (_dir, writer) = db();
    store(&writer, &export(FULL_TRACK));
    let expected = object(&[
        ("TrackID", "21899570"),
        ("Name", "Synthetic One"),
        ("Artist", "TLP Check"),
        ("Composer", "Comp"),
        ("Album", "Alb"),
        ("Grouping", "Red"),
        ("Genre", "TLP Genre"),
        ("Kind", "MP3 File"),
        ("Size", "6181152"),
        ("TotalTime", "255"),
        ("DiscNumber", "0"),
        ("TrackNumber", "7"),
        ("Year", "1999"),
        ("AverageBpm", "174.00"),
        ("DateModified", "2026-09-20"),
        ("DateAdded", "2026-09-25"),
        ("BitRate", "256"),
        ("SampleRate", "44100"),
        ("Comments", "Kit & Kin's \"dub\" <3"),
        ("PlayCount", "0"),
        ("LastPlayed", ""),
        ("Rating", "204"),
        (
            "Location",
            "file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20#1%20(100%25%20Flip).mp3",
        ),
        ("Remixer", "Remx"),
        ("Tonality", "4A"),
        ("Label", "Lbl"),
        ("Mix", "Extended Mix"),
        ("Colour", "0xFF0000"),
        ("FutureField", "kept"),
        ("Zeta", "last"),
    ]);
    assert_eq!(column(&writer, 21899570, "attributes"), expected);
}

#[test]
fn the_typed_columns_read_from_the_stored_attributes() {
    let (_dir, writer) = db();
    store(&writer, &export(FULL_TRACK));
    let row: (String, f64, String, i64, i64, String, String) = writer
        .call(|c| {
            c.query_row(
                "SELECT location, bpm, tonality, play_count, rating, colour, comments
                 FROM rekordbox_track",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
        })
        .unwrap();
    assert_eq!(
        row,
        (
            "file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20#1%20(100%25%20Flip).mp3".into(),
            174.0,
            "4A".into(),
            0,
            204,
            "0xFF0000".into(),
            "Kit & Kin's \"dub\" <3".into()
        )
    );
}

#[test]
fn every_grid_and_cue_entry_is_stored_with_every_attribute_in_file_order() {
    let (_dir, writer) = db();
    store(&writer, &export(FULL_TRACK));
    let tempo = format!(
        "[{},{}]",
        object(&[
            ("Inizio", "0.057"),
            ("Bpm", "174.00"),
            ("Metro", "4/4"),
            ("Battito", "1")
        ]),
        object(&[
            ("Inizio", "120.402"),
            ("Bpm", "87.00"),
            ("Metro", "3/4"),
            ("Battito", "2"),
            ("Extra", "x")
        ]),
    );
    let marks = format!(
        "[{},{},{}]",
        object(&[
            ("Name", "Drop"),
            ("Type", "0"),
            ("Start", "44.195"),
            ("Num", "0"),
            ("Red", "255"),
            ("Green", "55"),
            ("Blue", "111")
        ]),
        object(&[
            ("Name", ""),
            ("Type", "0"),
            ("Start", "45.574"),
            ("Num", "-1")
        ]),
        object(&[
            ("Name", "Roll"),
            ("Type", "4"),
            ("Start", "60.000"),
            ("End", "62.500"),
            ("Num", "1"),
            ("Red", "0"),
            ("Green", "0"),
            ("Blue", "255")
        ]),
    );
    assert_eq!(column(&writer, 21899570, "tempo"), tempo);
    assert_eq!(column(&writer, 21899570, "position_marks"), marks);
}

#[test]
fn a_track_without_cues_grid_or_playlists_stores_empty_lists_and_no_file_match() {
    let (_dir, writer) = db();
    store(&writer, &export(&track(1, "a.mp3")));
    for col in ["tempo", "position_marks", "playlists", "my_tags"] {
        assert_eq!(column(&writer, 1, col), "[]", "{col}");
    }
    let unmatched: bool = writer
        .call(|c| {
            c.query_row(
                "SELECT file_id IS NULL AND recording_id IS NULL AND relink_method IS NULL
                 FROM rekordbox_track",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert!(unmatched, "relinking is 1aC; nothing is matched here");
}

#[test]
fn my_tags_at_the_end_of_comments_are_stored_while_the_comments_stay_as_read() {
    let (_dir, writer) = db();
    let tagged = r#"<TRACK TrackID="1" Name="A" Location="file://localhost/C:/Kit/a.mp3" Comments="Opener /* Peak */"/>"#;
    let only_block = r#"<TRACK TrackID="2" Name="B" Location="file://localhost/C:/Kit/b.mp3" Comments="/* TLP */"/>"#;
    let not_at_end = r#"<TRACK TrackID="3" Name="C" Location="file://localhost/C:/Kit/c.mp3" Comments="a /* x */ b"/>"#;
    store(
        &writer,
        &export(&[tagged, only_block, not_at_end, &track(4, "d.mp3")].join("\n")),
    );
    assert_eq!(column(&writer, 1, "my_tags"), r#"["Peak"]"#);
    assert_eq!(column(&writer, 1, "comments"), "Opener /* Peak */");
    assert_eq!(column(&writer, 2, "my_tags"), r#"["TLP"]"#);
    assert_eq!(column(&writer, 3, "my_tags"), "[]");
    assert_eq!(column(&writer, 3, "comments"), "a /* x */ b");
    assert_eq!(column(&writer, 4, "my_tags"), "[]");
}

/// XML-escapes an attribute value. Control characters (tab, newline and
/// the rest) go in as `&#N;`, since raw ones aren't kept as written.
fn escape(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            '\'' => "&apos;".to_owned(),
            c if c.is_control() => format!("&#{};", u32::from(c)),
            c => c.to_string(),
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn any_attribute_values_and_names_round_trip_exactly_in_order(
        extra in proptest::collection::vec(("[A-Z][A-Za-z_]{0,8}", "[^\\x00]{0,24}"), 0..8),
    ) {
        // Unknown names only once each, and never a name rekordbox writes.
        let mut seen = std::collections::HashSet::new();
        let extra: Vec<(String, String)> = extra
            .into_iter()
            .filter(|(k, _)| !["TrackID", "Location", "Name", "Type", "Start", "End",
                              "Num", "Red", "Green", "Blue", "Bpm", "Metro"].contains(&k.as_str()))
            .filter(|(k, _)| seen.insert(k.clone()))
            .collect();
        let mut attrs = String::new();
        for (k, v) in &extra {
            attrs.push_str(&format!(r#" {k}="{}""#, escape(v)));
        }
        let xml = export(&format!(
            r#"<TRACK TrackID="5"{attrs} Location="file://localhost/C:/Kit/p.mp3"/>"#
        ));
        let (_dir, writer) = db();
        store(&writer, &xml);
        let mut pairs: Vec<(&str, &str)> = vec![("TrackID", "5")];
        pairs.extend(extra.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        pairs.push(("Location", "file://localhost/C:/Kit/p.mp3"));
        prop_assert_eq!(column(&writer, 5, "attributes"), object(&pairs));
        // And SQLite reads each value back the same.
        for (k, v) in &extra {
            let key = k.clone();
            let got: String = writer
                .call(move |c| c.query_row(
                    "SELECT json_extract(attributes, '$.' || ?1) FROM rekordbox_track",
                    [key], |r| r.get(0)))
                .unwrap();
            prop_assert_eq!(&got, v);
        }
    }
}

// --- keys, playlists, streaming ---

#[test]
fn location_key_is_the_parsers_match_key_so_other_spellings_of_a_path_share_it() {
    let (_dir, writer) = db();
    // NFD "é" and lowercase letters; the key is NFC and case-folded.
    store(
        &writer,
        &export(r#"<TRACK TrackID="1" Location="file://localhost/c:/kit/Cafe%CC%81%20#2.mp3"/>"#),
    );
    let key = column(&writer, 1, "location_key");
    assert_eq!(key, "C:/KIT/CAFÉ #2.MP3");
    let other = crate::rekordbox::location::decode("file://localhost/C:/Kit/Caf%C3%A9%20#2.mp3")
        .unwrap()
        .match_key();
    assert_eq!(key, other);
}

#[test]
fn each_track_lists_the_playlists_holding_it_once_each_with_their_folders_in_tree_order() {
    let (_dir, writer) = db();
    let tracks = [track(1, "a.mp3"), track(2, "b.mp3"), track(3, "c.mp3")].join("\n");
    let playlists = r#"
      <NODE Type="1" Name="Top" KeyType="0" Entries="2">
        <TRACK Key="2"/><TRACK Key="1"/>
      </NODE>
      <NODE Type="0" Name="Gigs" Count="1">
        <NODE Type="0" Name="2026" Count="1">
          <NODE Type="1" Name="Warm / up" KeyType="0" Entries="3">
            <TRACK Key="1"/><TRACK Key="1"/><TRACK Key="99"/>
          </NODE>
        </NODE>
      </NODE>"#;
    store(&writer, &export_with(&tracks, playlists, None));
    assert_eq!(
        column(&writer, 1, "playlists"),
        r#"[["Top"],["Gigs","2026","Warm / up"]]"#
    );
    assert_eq!(column(&writer, 2, "playlists"), r#"[["Top"]]"#);
    assert_eq!(column(&writer, 3, "playlists"), "[]");
}

#[test]
fn streaming_entries_are_stored_keyed_by_their_stream_and_skipped_tracks_are_not() {
    let (_dir, writer) = db();
    let tracks = [
        track(1, "a.mp3"),
        r#"<TRACK TrackID="2" Name="Stream" Location="soundcloud:tracks:12345"/>"#.to_owned(),
        r#"<TRACK TrackID="3" Location="file://localhost/C:/Users/someone/Music/PioneerDJ/Demo Tracks/Demo.mp3"/>"#.to_owned(),
        r#"<TRACK TrackID="4" Location="file://localhost/C:/Kit/gone.mp3" rb_local_deleted="1"/>"#.to_owned(),
    ]
    .join("\n");
    let summary = store(&writer, &export(&tracks));
    let ids: Vec<i64> = stored(&writer).into_iter().map(|r| r.0).collect();
    assert_eq!(ids, [1, 2]);
    assert_eq!(
        column(&writer, 2, "location_key"),
        "soundcloud:tracks:12345"
    );
    assert_eq!((summary.tracks, summary.streaming), (2, 1));
}

#[test]
fn tracks_with_no_usable_track_id_or_location_are_counted_and_not_stored() {
    let (_dir, writer) = db();
    let tracks = [
        track(1, "a.mp3"),
        // No TrackID; leading zero (the table checks it reads back the same);
        // the id an earlier track has; a Location that doesn't decode.
        r#"<TRACK Name="No id" Location="file://localhost/C:/Kit/n.mp3"/>"#.to_owned(),
        r#"<TRACK TrackID="07" Location="file://localhost/C:/Kit/z.mp3"/>"#.to_owned(),
        r#"<TRACK TrackID="1" Location="file://localhost/C:/Kit/dup.mp3"/>"#.to_owned(),
        r#"<TRACK TrackID="9" Location="file://localhost/C:/Kit/bad%zz.mp3"/>"#.to_owned(),
        track(10, "b.mp3"),
    ]
    .join("\n");
    let summary = store(&writer, &export(&tracks));
    let rows = stored(&writer);
    assert_eq!(
        rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        [1, 10],
        "{rows:?}"
    );
    assert_eq!(column(&writer, 1, "location_key"), "C:/KIT/A.MP3");
    assert_eq!(summary.not_stored, 4);
    assert_eq!(summary.tracks, 2);
}

// --- replacing the snapshot ---

#[test]
fn a_complete_read_replaces_the_whole_snapshot_with_one_read_time() {
    let (_dir, writer) = db();
    store(
        &writer,
        &export(&[track(1, "a.mp3"), track(2, "b.mp3")].join("\n")),
    );
    age_snapshot(&writer);
    let summary = store(
        &writer,
        &export(&[track(2, "b.mp3"), track(3, "c.mp3")].join("\n")),
    );
    let rows = stored(&writer);
    assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), [2, 3]);
    assert!(
        rows.iter().all(|r| r.2 != OLD && r.2 == rows[0].2),
        "{rows:?}"
    );
    assert_eq!(
        summary,
        SnapshotSummary {
            tracks: 2,
            streaming: 0,
            kept: 0,
            not_stored: 0,
            complete: true
        }
    );
}

#[test]
fn an_incomplete_export_keeps_earlier_tracks_it_lacks_with_their_earlier_read_time() {
    let (_dir, writer) = db();
    store(
        &writer,
        &export(&[track(1, "a.mp3"), track(2, "b.mp3"), track(3, "c.mp3")].join("\n")),
    );
    age_snapshot(&writer);
    // Says 3 entries but holds 2: track 3 may just be missing from the file.
    let xml = export_with(
        &[track(1, "a.mp3"), track(2, "b-renamed.mp3")].join("\n"),
        "",
        Some(3),
    );
    assert!(!parse(&xml).is_complete());
    let summary = store(&writer, &xml);
    let rows = stored(&writer);
    let by_id = |id: i64| rows.iter().find(|r| r.0 == id).unwrap().clone();
    // Read again: replaced, with this read's time.
    assert_ne!(by_id(1).2, OLD);
    assert_eq!(by_id(2).1, "C:/KIT/B-RENAMED.MP3");
    assert_ne!(by_id(2).2, OLD);
    // Not in this read: kept, untouched.
    assert_eq!(by_id(3), (3, "C:/KIT/C.MP3".into(), OLD.into()));
    assert_eq!(
        (summary.tracks, summary.kept, summary.complete),
        (2, 1, false)
    );
}

#[test]
fn an_incomplete_export_replaces_an_earlier_track_at_the_same_location_whatever_its_id() {
    let (_dir, writer) = db();
    store(&writer, &export(&track(1, "a.mp3")));
    age_snapshot(&writer);
    // rekordbox renumbered the track (an import reassigns TrackIDs).
    let xml = export_with(&track(40, "a.mp3"), "", Some(2));
    let summary = store(&writer, &xml);
    let rows = stored(&writer);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, 40);
    assert_eq!(summary.kept, 0);
}

#[test]
fn an_incomplete_export_drops_a_kept_track_whose_id_it_now_gives_another_track() {
    let (_dir, writer) = db();
    store(
        &writer,
        &export(&[track(1, "a.mp3"), track(2, "b.mp3")].join("\n")),
    );
    age_snapshot(&writer);
    // Track 2's id now belongs to a different file, and b.mp3 isn't in the
    // read. TrackIDs are only valid within one read, so the new one wins.
    let xml = export_with(
        &[track(1, "a.mp3"), track(2, "new.mp3")].join("\n"),
        "",
        Some(5),
    );
    let summary = store(&writer, &xml);
    let rows = stored(&writer);
    assert_eq!(
        rows.iter().map(|r| (r.0, r.1.as_str())).collect::<Vec<_>>(),
        [(1, "C:/KIT/A.MP3"), (2, "C:/KIT/NEW.MP3")]
    );
    assert_eq!(summary.kept, 0);
}

/// Makes the next insert of a track named `name` fail, as a full disk or
/// a crash midway would.
fn fail_on_insert_of(conn: &Connection, name: &str) -> rusqlite::Result<()> {
    conn.execute_batch(&format!(
        "CREATE TEMP TRIGGER fail_midway BEFORE INSERT ON main.rekordbox_track
         WHEN json_extract(new.attributes, '$.Name') = '{name}'
         BEGIN SELECT RAISE(ABORT, 'disk full'); END;"
    ))
}

#[test]
fn a_read_that_fails_midway_leaves_the_previous_snapshot_whole() {
    let (_dir, writer) = db();
    store(
        &writer,
        &export(&[track(1, "a.mp3"), track(2, "b.mp3")].join("\n")),
    );
    age_snapshot(&writer);
    let before = stored(&writer);
    writer
        .call(|c| fail_on_insert_of(c, "Synthetic 4"))
        .unwrap();

    let rows = SnapshotRows::from_xml(&parse(&export(
        &[track(3, "c.mp3"), track(4, "d.mp3"), track(5, "e.mp3")].join("\n"),
    )));
    let recorded = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = recorded.clone();
    let result = writer.call(move |c| {
        replace_snapshot(c, &rows, |_, _, _| {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
    });
    assert!(result.is_err(), "the insert of track 4 should fail");
    assert!(!recorded.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(stored(&writer), before);
}

#[test]
fn a_failure_recording_the_read_undoes_the_new_snapshot_too() {
    let (_dir, writer) = db();
    store(&writer, &export(&track(1, "a.mp3")));
    let before = stored(&writer);
    let rows = SnapshotRows::from_xml(&parse(&export(&track(2, "b.mp3"))));
    let result = writer.call(move |c| {
        replace_snapshot(c, &rows, |_, _, _| {
            Err(rusqlite::Error::InvalidParameterName("boom".into()))
        })
    });
    assert!(result.is_err());
    assert_eq!(stored(&writer), before);
}

#[test]
fn an_incomplete_export_replaces_earlier_tracks_it_now_skips_or_cannot_store() {
    let (_dir, writer) = db();
    store(
        &writer,
        &export(
            &[
                track(1, "a.mp3"),
                track(2, "b.mp3"),
                track(3, "c.mp3"),
                track(4, "d.mp3"),
                track(5, "e.mp3"),
            ]
            .join("\n"),
        ),
    );
    age_snapshot(&writer);
    // Says 9 entries. b.mp3 is now marked deleted, c.mp3's TrackID can't be
    // stored, and track 5 is at a Location that doesn't decode. d.mp3 isn't
    // in the file at all.
    let tracks = [
        track(1, "a.mp3"),
        r#"<TRACK TrackID="2" Location="file://localhost/C:/Kit/b.mp3" rb_local_deleted="1"/>"#
            .to_owned(),
        r#"<TRACK TrackID="03" Location="file://localhost/C:/Kit/c.mp3"/>"#.to_owned(),
        r#"<TRACK TrackID="5" Location="file://localhost/C:/Kit/bad%zz.mp3"/>"#.to_owned(),
    ]
    .join("\n");
    let summary = store(&writer, &export_with(&tracks, "", Some(9)));
    let rows = stored(&writer);
    assert_eq!(
        rows.iter().map(|r| (r.0, r.2 == OLD)).collect::<Vec<_>>(),
        [(1, false), (4, true)],
        "{rows:?}"
    );
    assert_eq!((summary.kept, summary.not_stored), (1, 2));
}
