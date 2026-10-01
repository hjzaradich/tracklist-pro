//! What the writer puts in the file, checked by reading it back with the
//! app's reader. Every fixture is synthetic: made-up titles, artists and
//! paths only.

use proptest::prelude::*;

use super::*;
use crate::rekordbox::{EntryTarget, Folder, KeyType, Playlist, Track};

// ---- helpers ---------------------------------------------------------------

fn id(n: i64) -> LibraryTrackId {
    LibraryTrackId(n)
}

fn attrs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect()
}

/// A track rekordbox has, with these attributes.
fn known(n: i64, pairs: &[(&str, &str)]) -> TrackInput {
    TrackInput {
        library_track: id(n),
        values: Values::Ready {
            in_rekordbox: true,
            attributes: attrs(pairs),
            rekordbox_holds_other_file: None,
        },
    }
}

/// A plain track rekordbox has: `TrackID` `1000 + n`, at `C:/Kit/<n>.mp3`.
fn known_plain(n: i64) -> TrackInput {
    known(
        n,
        &[
            ("TrackID", &(1000 + n).to_string()),
            ("Name", &format!("Synthetic {n}")),
            ("Location", &format!("file://localhost/C:/Kit/{n}.mp3")),
        ],
    )
}

/// A track rekordbox doesn't know, with these attributes.
fn new(n: i64, pairs: &[(&str, &str)]) -> TrackInput {
    TrackInput {
        library_track: id(n),
        values: Values::Ready {
            in_rekordbox: false,
            attributes: attrs(pairs),
            rekordbox_holds_other_file: None,
        },
    }
}

/// A plain new track at `C:\New\<n>.flac`.
fn new_plain(n: i64) -> TrackInput {
    new(
        n,
        &[
            ("Name", &format!("New {n}")),
            ("Location", &format!(r"C:\New\{n}.flac")),
        ],
    )
}

fn cannot_send(n: i64, why: CannotSend) -> TrackInput {
    TrackInput {
        library_track: id(n),
        values: Values::CannotSend(why),
    }
}

fn playlist(name: &str, entries: &[i64]) -> Node {
    Node::Playlist {
        name: name.to_owned(),
        entries: entries.iter().map(|&n| id(n)).collect(),
    }
}

fn folder(name: &str, children: Vec<Node>) -> Node {
    Node::Folder {
        name: name.to_owned(),
        children,
    }
}

fn path(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

/// Builds, and reads the file back with the reader.
fn send(input: &SendInput) -> (Outgoing, RekordboxXml) {
    let out = build(input).unwrap_or_else(|e| panic!("{e}"));
    let read = RekordboxXml::parse(out.xml()).unwrap_or_else(|e| panic!("{e}"));
    (out, read)
}

fn text(out: &Outgoing) -> &str {
    std::str::from_utf8(out.xml()).unwrap()
}

fn read_attrs(track: &Track) -> Vec<(String, String)> {
    track
        .attrs
        .iter()
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect()
}

/// The track read back for a Library track.
fn read_track<'a>(out: &Outgoing, read: &'a RekordboxXml, n: i64) -> &'a Track {
    let sent = out
        .sent
        .iter()
        .find(|t| t.library_track == id(n))
        .unwrap_or_else(|| panic!("track {n} wasn't sent"));
    read.tracks
        .iter()
        .find(|t| t.track_id == Some(sent.track_id))
        .unwrap_or_else(|| panic!("track {n} isn't in the file"))
}

fn sent_ids(out: &Outgoing) -> Vec<i64> {
    out.sent.iter().map(|t| t.library_track.0).collect()
}

fn top<'a>(read: &'a RekordboxXml, name: &str) -> &'a Folder {
    read.playlists
        .children
        .iter()
        .find_map(|n| match n {
            rekordbox::Node::Folder(f) if f.name == name => Some(f),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no top-level folder {name}"))
}

fn playlist_at<'a>(read: &'a RekordboxXml, names: &[&str]) -> &'a Playlist {
    read.playlists
        .playlists()
        .into_iter()
        .find(|(folders, p)| {
            folders
                .iter()
                .copied()
                .chain([p.name.as_str()])
                .eq(names.iter().copied())
        })
        .unwrap_or_else(|| panic!("no playlist at {names:?}"))
        .1
}

/// A playlist's entries as the Library tracks they resolve to.
fn entries(out: &Outgoing, read: &RekordboxXml, names: &[&str]) -> Vec<i64> {
    playlist_at(read, names)
        .entries
        .iter()
        .map(|e| match e.target {
            EntryTarget::Track(i) => {
                let track_id = read.tracks[i].track_id.unwrap();
                out.sent
                    .iter()
                    .find(|t| t.track_id == track_id)
                    .unwrap()
                    .library_track
                    .0
            }
            other => panic!("an entry of {names:?} doesn't resolve: {other:?}"),
        })
        .collect()
}

// ---- rule 1: every attribute, value for value -------------------------------

/// A synthetic export: every attribute rekordbox 7 writes, awkward values,
/// grids and cues, an attribute the app doesn't know, nested playlists.
const EXPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="4">
    <TRACK TrackID="21899570" Name="Synthetic One" Artist="TLP Check" Composer="Comp"
           Album="Alb" Grouping="Red" Genre="TLP Genre" Kind="MP3 File" Size="6181152"
           TotalTime="255" DiscNumber="2" TrackNumber="7" Year="1999" AverageBpm="174.00"
           DateModified="2026-09-20" DateAdded="2026-09-25" BitRate="256" SampleRate="44100"
           Comments="Kit &amp; Kin&apos;s &lt;b&gt; &quot;q&quot; /* Tag A / Tag B */" PlayCount="3"
           LastPlayed="2026-09-27" Rating="204"
           Location="file://localhost/C:/Kit/tracks/04%20Kit%20%26%20Kin%20-%20Low%20Tide%20#1%20(100%25%20Flip).mp3"
           Remixer="Remx" Tonality="4A" Label="Lbl" Mix="Extended Mix" Colour="0xFF0000"
           FutureField="kept">
      <TEMPO Inizio="0.057" Bpm="174.00" Metro="4/4" Battito="1"/>
      <POSITION_MARK Name="Drop" Type="0" Start="44.195" Num="0" Red="255" Green="55" Blue="111"/>
    </TRACK>
    <TRACK TrackID="42749078" Name="Line one&#10;line two&#9;tabbed&#13;" Artist="" Composer="" Album=""
           Grouping="" Genre="" Kind="M4A File" Size="100" TotalTime="200"
           DiscNumber="0" TrackNumber="0" Year="0" AverageBpm="0.00" DateAdded="2026-09-25"
           BitRate="0" SampleRate="44100" Comments="" PlayCount="0" Rating="0"
           Location="file://localhost/C:/Kit/Sub%20Folder/06%20North%20Wind,%20Late%20Train%20%f0%9f%9a%80%20Caf%c3%a9%20%e6%97%a5%e6%9c%ac%20A%20+%20B%20%5bx%5d.m4a"
           Remixer="" Tonality="" Label="" Mix=""/>
    <TRACK TrackID="7" Name="Café 日本 🚀" Artist="  padded  " Year="notayear" Rating="999"
           Location="file://localhost//nas/Share%20One/a.flac" Tonality="Fm" AverageBpm="128.00">
      <TEMPO Inizio="0.1" Bpm="128.00" Metro="4/4" Battito="1"/>
    </TRACK>
    <TRACK TrackID="8" Name="Not in a playlist" Location="file://localhost/C:/Kit/unlisted.mp3"/>
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="2">
      <NODE Name="All &amp; sundry" Type="1" KeyType="0" Entries="4">
        <TRACK Key="7"/>
        <TRACK Key="21899570"/>
        <TRACK Key="42749078"/>
        <TRACK Key="8"/>
      </NODE>
      <NODE Type="0" Name="Nested" Count="1">
        <NODE Name="Deep" Type="1" KeyType="0" Entries="2">
          <TRACK Key="42749078"/>
          <TRACK Key="42749078"/>
        </NODE>
      </NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#;

/// The reader's playlist tree as writer input, with Library track ids
/// `index + 1`.
fn nodes_from(folder: &Folder) -> Vec<Node> {
    folder
        .children
        .iter()
        .map(|child| match child {
            rekordbox::Node::Folder(f) => Node::Folder {
                name: f.name.clone(),
                children: nodes_from(f),
            },
            rekordbox::Node::Playlist(p) => Node::Playlist {
                name: p.name.clone(),
                entries: p
                    .entries
                    .iter()
                    .map(|e| match e.target {
                        EntryTarget::Track(i) => id(i as i64 + 1),
                        other => panic!("{other:?}"),
                    })
                    .collect(),
            },
        })
        .collect()
}

/// Every track of an export as a known track, exactly as read.
fn input_from(export: &RekordboxXml) -> SendInput {
    SendInput {
        tracks: export
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| TrackInput {
                library_track: id(i as i64 + 1),
                values: Values::Ready {
                    in_rekordbox: true,
                    attributes: read_attrs(t),
                    rekordbox_holds_other_file: None,
                },
            })
            .collect(),
        crates: Vec::new(),
        playlists: nodes_from(&export.playlists),
        highest_rekordbox_track_id: export
            .tracks
            .iter()
            .filter_map(|t| t.track_id)
            .max()
            .unwrap_or(0),
    }
}

#[test]
fn an_export_sent_back_reads_identical_except_the_analysis_fields_and_the_locations_spelling() {
    let export = RekordboxXml::parse(EXPORT.as_bytes()).unwrap();
    let (out, read) = send(&input_from(&export));
    assert_eq!(out.left_out, []);
    assert_eq!(read.tracks.len(), export.tracks.len());

    for (before, after) in export.tracks.iter().zip(&read.tracks) {
        // Same attributes in the same order, minus the two analysis ones.
        let expected: Vec<(String, String)> = read_attrs(before)
            .into_iter()
            .filter(|(n, _)| n != "AverageBpm" && n != "Tonality")
            .collect();
        let got = read_attrs(after);
        assert_eq!(
            got.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            expected.iter().map(|(n, _)| n).collect::<Vec<_>>()
        );
        for ((name, value), (_, sent)) in expected.iter().zip(&got) {
            if name == "Location" {
                // Rule 5 respells it; it names the same path.
                assert_eq!(after.location, before.location, "{value}");
                assert_eq!(
                    after.location.as_ref().unwrap().match_key(),
                    before.location.as_ref().unwrap().match_key()
                );
            } else {
                assert_eq!(sent, value, "{name}");
            }
        }
        assert!(after.tempos.is_empty() && after.cues.is_empty());
    }
    // The playlists hold the same tracks in the same order.
    assert_eq!(top(&read, PLAYLISTS_FOLDER).children.len(), 2);
    assert_eq!(
        entries(&out, &read, &["Playlists", "All & sundry"]),
        [3, 1, 2, 4]
    );
    assert_eq!(
        entries(&out, &read, &["Playlists", "Nested", "Deep"]),
        [2, 2]
    );
}

#[test]
fn sending_the_writers_own_output_again_gives_the_same_bytes() {
    let export = RekordboxXml::parse(EXPORT.as_bytes()).unwrap();
    let (first, read) = send(&input_from(&export));
    // The second time round the tree already sits under `Playlists`.
    let mut input = input_from(&read);
    input.playlists = nodes_from(top(&read, PLAYLISTS_FOLDER));
    let (second, _) = send(&input);
    assert_eq!(text(&second), text(&first));
}

#[test]
fn control_characters_and_xml_special_characters_survive_in_every_value() {
    let awkward = "tab\there\nnew line\r\ncr-lf & <tag> \"double\" 'single' &amp; &#9; ]]> é 🚀";
    let input = SendInput {
        tracks: vec![
            known(
                1,
                &[
                    ("TrackID", "5"),
                    ("Name", awkward),
                    ("Comments", "  leading and trailing  "),
                    ("Location", "file://localhost/C:/Kit/a.mp3"),
                    ("Mix", ""),
                ],
            ),
            new(2, &[("Name", awkward), ("Location", r"C:\New\b & c's.mp3")]),
        ],
        playlists: vec![playlist("P \"quoted\" & <odd>\tname", &[1, 2])],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    for n in [1, 2] {
        assert_eq!(read_track(&out, &read, n).name(), awkward);
    }
    let first = read_track(&out, &read, 1);
    assert_eq!(first.comments(), "  leading and trailing  ");
    assert_eq!(first.attrs.get("Mix"), Some(""));
    assert_eq!(
        entries(&out, &read, &["Playlists", "P \"quoted\" & <odd>\tname"]),
        [1, 2]
    );
    // No raw tab, CR or LF inside any attribute: every line is one element.
    for line in text(&out).lines() {
        assert!(!line.contains(['\t', '\r']), "{line:?}");
        assert!(
            line.trim_start().starts_with('<') && line.ends_with('>'),
            "{line:?}"
        );
    }
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
}

#[test]
fn nothing_is_added_defaulted_or_reordered() {
    let given = [
        ("Zeta", "last name first"),
        ("TrackID", "12"),
        ("PlayCount", "0"),
        ("Rating", ""),
        ("Location", "file://localhost/C:/Kit/a.mp3"),
        ("Name", "Synthetic"),
    ];
    let input = SendInput {
        tracks: vec![known(1, &given)],
        playlists: vec![playlist("P", &[1])],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    assert_eq!(read_attrs(read_track(&out, &read, 1)), attrs(&given));
    assert_eq!(out.sent[0].attributes, attrs(&given));

    // A new track gets its file-local TrackID first and nothing else.
    let input = SendInput {
        tracks: vec![new(1, &[("Name", "N"), ("Location", r"C:\New\n.mp3")])],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    assert_eq!(
        read_attrs(read_track(&out, &read, 1)),
        attrs(&[
            ("TrackID", "1"),
            ("Name", "N"),
            ("Location", "file://localhost/C:/New/n.mp3")
        ])
    );
}

/// Any text XML can carry, heavy on the characters that need escaping.
fn carriable() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            3 => any::<char>(),
            1 => proptest::sample::select(vec!['&', '<', '>', '"', '\'', '\t', '\n', '\r', ' ', ';', '#']),
        ],
        0..40,
    )
    .prop_map(|chars| {
        chars
            .into_iter()
            .filter(|c| xml::uncarriable(c.encode_utf8(&mut [0; 4])).is_none())
            .collect()
    })
}

proptest! {
    #[test]
    fn any_text_xml_can_carry_reads_back_exactly(
        name in carriable(),
        comments in carriable(),
        playlist_name in carriable().prop_filter("a name", |n| !n.is_empty()),
    ) {
        let input = SendInput {
            tracks: vec![known(1, &[
                ("TrackID", "3"),
                ("Name", &name),
                ("Comments", &comments),
                ("Location", "file://localhost/C:/Kit/a.mp3"),
            ])],
            crates: vec![playlist(&playlist_name, &[1])],
            ..SendInput::default()
        };
        let (out, read) = send(&input);
        let track = read_track(&out, &read, 1);
        prop_assert_eq!(track.name(), name.as_str());
        prop_assert_eq!(track.comments(), comments.as_str());
        prop_assert_eq!(top(&read, CRATES_FOLDER).children[0].name(), playlist_name.as_str());
    }
}

// ---- rule 3: analysis fields ----------------------------------------------

const KNOWN_FULL: &[(&str, &str)] = &[
    ("TrackID", "21"),
    ("Name", "Synthetic"),
    ("Artist", "TLP Check"),
    ("AverageBpm", "174.00"),
    ("Rating", "204"),
    ("Location", "file://localhost/C:/Kit/a.mp3"),
    ("Tonality", "4A"),
    ("Colour", "0xFF0000"),
];

#[test]
fn a_track_already_in_rekordbox_is_sent_without_exactly_the_four_analysis_fields() {
    let input = SendInput {
        tracks: vec![known(1, KNOWN_FULL)],
        playlists: vec![playlist("P", &[1])],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    let track = read_track(&out, &read, 1);
    // The two attributes are gone, and every other one is there.
    let expected: Vec<(String, String)> = attrs(KNOWN_FULL)
        .into_iter()
        .filter(|(n, _)| n != "AverageBpm" && n != "Tonality")
        .collect();
    assert_eq!(expected.len(), KNOWN_FULL.len() - 2);
    assert_eq!(read_attrs(track), expected);
    // No grid, no cues.
    assert!(track.tempos.is_empty() && track.cues.is_empty());
    for word in ["TEMPO", "POSITION_MARK", "Tonality", "AverageBpm"] {
        assert!(!text(&out).contains(word), "{word} is in the file");
    }
    // What's recorded as sent matches the file.
    assert_eq!(out.sent[0].attributes, expected);
    assert!(out.sent[0]
        .fields()
        .all(|(n, _)| !ANALYSIS_ATTRIBUTES.contains(&n) && n != "TrackID"));
}

#[test]
fn bpm_and_key_are_never_sent_for_a_new_track_either() {
    // Values that carried the app's own estimate would still not reach
    // the file.
    let input = SendInput {
        tracks: vec![new(
            1,
            &[
                ("Name", "N"),
                ("AverageBpm", "128.00"),
                ("Tonality", "8A"),
                ("Location", r"C:\New\n.mp3"),
            ],
        )],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    let track = read_track(&out, &read, 1);
    assert_eq!(track.attrs.get("AverageBpm"), None);
    assert_eq!(track.attrs.get("Tonality"), None);
    assert_eq!(track.average_bpm, None);
    assert_eq!(track.key, None);
    for word in [
        "TEMPO",
        "POSITION_MARK",
        "Tonality",
        "AverageBpm",
        "128.00",
        "8A",
    ] {
        assert!(!text(&out).contains(word), "{word} is in the file");
    }
}

proptest! {
    #[test]
    fn no_input_puts_an_analysis_field_in_the_file(
        in_rekordbox in any::<bool>(),
        bpm in "[0-9]{2,3}\\.[0-9]{2}",
        key in "[0-9]{1,2}[AB]",
        bpm_first in any::<bool>(),
    ) {
        let location = if in_rekordbox { "file://localhost/C:/Kit/a.mp3" } else { r"C:\Kit\a.mp3" };
        let mut pairs = vec![("Name", "N"), ("Location", location)];
        if in_rekordbox {
            pairs.insert(0, ("TrackID", "4"));
        }
        let at = if bpm_first { 0 } else { pairs.len() };
        pairs.insert(at, ("AverageBpm", bpm.as_str()));
        pairs.push(("Tonality", key.as_str()));
        let track = TrackInput {
            library_track: id(1),
            values: Values::Ready { in_rekordbox, attributes: attrs(&pairs), rekordbox_holds_other_file: None },
        };
        let input = SendInput {
            tracks: vec![track],
            playlists: vec![playlist("P", &[1])],
            ..SendInput::default()
        };
        let (out, read) = send(&input);
        prop_assert_eq!(read.tracks.len(), 1);
        prop_assert_eq!(read.tracks[0].attrs.get("AverageBpm"), None);
        prop_assert_eq!(read.tracks[0].attrs.get("Tonality"), None);
        prop_assert!(!text(&out).contains("AverageBpm") && !text(&out).contains("Tonality"));
        prop_assert!(!text(&out).contains("TEMPO") && !text(&out).contains("POSITION_MARK"));
    }
}

// ---- rule 4: what COLLECTION holds ------------------------------------------

#[test]
fn collection_holds_every_track_an_entry_names_and_every_new_track_and_no_other() {
    let input = SendInput {
        tracks: vec![
            known_plain(1), // in a crate
            known_plain(2), // in no crate or playlist: not sent
            new_plain(3),   // new, in a playlist
            new_plain(4),   // new, in nothing: still sent
            known_plain(5), // in a nested playlist
        ],
        crates: vec![playlist("Warm up", &[1])],
        playlists: vec![
            playlist("Friday", &[3, 1]),
            folder("Old", vec![playlist("Deep", &[5])]),
        ],
        highest_rekordbox_track_id: 2000,
    };
    let (out, read) = send(&input);
    assert_eq!(sent_ids(&out), [1, 3, 4, 5]);
    assert_eq!(out.not_needed, [id(2)]);
    assert_eq!(out.left_out, []);
    assert_eq!(read.tracks.len(), 4);
    assert_eq!(read.declared_entries, Some(4));
    assert!(read.is_complete());
    // Every entry resolves against this file's own COLLECTION (§5.2, T1).
    assert_eq!(read.entries_without_track, 0);
    assert_eq!(entries(&out, &read, &["Crates", "Warm up"]), [1]);
    assert_eq!(entries(&out, &read, &["Playlists", "Friday"]), [3, 1]);
    assert_eq!(entries(&out, &read, &["Playlists", "Old", "Deep"]), [5]);
    // Each sent track carries every attribute it was given.
    assert_eq!(
        read_attrs(read_track(&out, &read, 5)),
        attrs(&[
            ("TrackID", "1005"),
            ("Name", "Synthetic 5"),
            ("Location", "file://localhost/C:/Kit/5.mp3")
        ])
    );
}

#[test]
fn a_send_with_no_playlists_still_carries_its_new_tracks_and_never_playlists_alone() {
    let input = SendInput {
        tracks: vec![known_plain(1), new_plain(2)],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    assert_eq!(sent_ids(&out), [2]);
    assert_eq!(out.not_needed, [id(1)]);
    assert!(read.playlists.playlists().is_empty());
}

#[test]
fn an_entry_that_repeats_a_track_stays_repeated_in_order() {
    let input = SendInput {
        tracks: vec![known_plain(1), known_plain(2)],
        crates: vec![playlist("Loop", &[2, 1, 2, 2, 1])],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    assert_eq!(entries(&out, &read, &["Crates", "Loop"]), [2, 1, 2, 2, 1]);
    // COLLECTION holds each once.
    assert_eq!(sent_ids(&out), [1, 2]);
}

#[test]
fn entries_are_keyed_by_track_id_as_rekordbox_exports_them() {
    let input = SendInput {
        tracks: vec![known_plain(1), new_plain(2)],
        crates: vec![playlist("C", &[1, 2])],
        highest_rekordbox_track_id: 5000,
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    let crate_ = playlist_at(&read, &["Crates", "C"]);
    assert_eq!(crate_.key_type, KeyType::TrackId);
    assert_eq!(
        crate_
            .entries
            .iter()
            .map(|e| e.key.as_str())
            .collect::<Vec<_>>(),
        ["1001", "5001"]
    );
    assert!(text(&out).contains(r#"KeyType="0""#));
}

// ---- TrackIDs -------------------------------------------------------------

#[test]
fn new_tracks_are_numbered_above_every_track_id_in_the_last_rekordbox_read() {
    // rekordbox's highest id (9000) belongs to a track that isn't sent.
    let input = SendInput {
        tracks: vec![new_plain(1), known_plain(2), new_plain(3)],
        crates: vec![playlist("C", &[2])],
        highest_rekordbox_track_id: 9000,
        ..SendInput::default()
    };
    let (out, _) = send(&input);
    let ids: Vec<(i64, u64)> = out
        .sent
        .iter()
        .map(|t| (t.library_track.0, t.track_id))
        .collect();
    assert_eq!(ids, [(1, 9001), (2, 1002), (3, 9002)]);
}

#[test]
fn new_track_ids_also_clear_the_ids_in_the_file_when_the_given_highest_is_stale() {
    let input = SendInput {
        tracks: vec![known_plain(7), new_plain(1)],
        crates: vec![playlist("C", &[7])],
        highest_rekordbox_track_id: 3,
        ..SendInput::default()
    };
    let (out, _) = send(&input);
    assert_eq!(out.sent[1].track_id, 1008);
}

#[test]
fn a_known_track_keeps_rekordboxs_track_id() {
    let input = SendInput {
        tracks: vec![known_plain(1)],
        crates: vec![playlist("C", &[1])],
        highest_rekordbox_track_id: 9000,
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    assert_eq!(out.sent[0].track_id, 1001);
    assert_eq!(read.tracks[0].track_id, Some(1001));
}

#[test]
fn running_out_of_track_ids_refuses_the_send() {
    let input = SendInput {
        tracks: vec![new_plain(1)],
        highest_rekordbox_track_id: u64::MAX,
        ..SendInput::default()
    };
    assert_eq!(build(&input), Err(BuildError::TrackIdsExhausted));
}

// ---- tracks that can't be sent ----------------------------------------------

#[test]
fn a_track_that_cannot_be_sent_is_left_out_and_reported_with_its_entries() {
    let input = SendInput {
        tracks: vec![
            known_plain(1),
            cannot_send(2, CannotSend::FileMissing { file_id: 40 }),
            known_plain(3),
            cannot_send(4, CannotSend::NoLinkedFile),
        ],
        crates: vec![
            playlist("Mixed", &[1, 2, 3, 2]),
            folder("F", vec![playlist("Only gone", &[2])]),
            playlist("Never had any", &[]),
        ],
        playlists: vec![playlist("P", &[3, 2])],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    assert_eq!(sent_ids(&out), [1, 3]);
    assert_eq!(read.tracks.len(), 2);
    assert_eq!(
        out.left_out,
        [
            LeftOut {
                library_track: id(2),
                reason: Reason::NoValues(CannotSend::FileMissing { file_id: 40 }),
                entries: vec![
                    DroppedEntry {
                        path: path(&["Crates", "Mixed"]),
                        position: 1
                    },
                    DroppedEntry {
                        path: path(&["Crates", "Mixed"]),
                        position: 3
                    },
                    DroppedEntry {
                        path: path(&["Crates", "F", "Only gone"]),
                        position: 0
                    },
                    DroppedEntry {
                        path: path(&["Playlists", "P"]),
                        position: 1
                    },
                ],
            },
            // In no crate or playlist, and still reported.
            LeftOut {
                library_track: id(4),
                reason: Reason::NoValues(CannotSend::NoLinkedFile),
                entries: vec![],
            },
        ]
    );
    // The other entries keep their order, and nothing points at a track
    // the file doesn't hold.
    assert_eq!(entries(&out, &read, &["Crates", "Mixed"]), [1, 3]);
    assert_eq!(entries(&out, &read, &["Playlists", "P"]), [3]);
    assert_eq!(read.entries_without_track, 0);
    // A crate that lost every entry is still written, empty, and listed;
    // one that never had any isn't listed.
    assert!(playlist_at(&read, &["Crates", "F", "Only gone"])
        .entries
        .is_empty());
    assert!(playlist_at(&read, &["Crates", "Never had any"])
        .entries
        .is_empty());
    assert_eq!(out.emptied, [path(&["Crates", "F", "Only gone"])]);
}

#[test]
fn an_entry_naming_a_track_the_input_does_not_hold_is_dropped_and_reported() {
    let input = SendInput {
        tracks: vec![known_plain(1)],
        crates: vec![playlist("C", &[9, 1])],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    assert_eq!(entries(&out, &read, &["Crates", "C"]), [1]);
    assert_eq!(
        out.left_out,
        [LeftOut {
            library_track: id(9),
            reason: Reason::NotGiven,
            entries: vec![DroppedEntry {
                path: path(&["Crates", "C"]),
                position: 0
            }],
        }]
    );
}

/// The reason one track, in a crate with a good one, is left out for.
fn refusal(track: TrackInput) -> Reason {
    let bad = track.library_track;
    let input = SendInput {
        tracks: vec![known_plain(100), track],
        crates: vec![playlist("C", &[100, bad.0])],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    // Only that track is affected.
    assert_eq!(sent_ids(&out), [100]);
    assert_eq!(entries(&out, &read, &["Crates", "C"]), [100]);
    assert_eq!(out.left_out.len(), 1);
    assert_eq!(out.left_out[0].library_track, bad);
    assert_eq!(
        out.left_out[0].entries,
        [DroppedEntry {
            path: path(&["Crates", "C"]),
            position: 1
        }]
    );
    out.left_out[0].reason.clone()
}

#[test]
fn a_value_with_a_character_xml_cannot_carry_keeps_its_track_out_whatever_its_source() {
    // rekordbox's own value (the reader accepts `&#1;`).
    let export = RekordboxXml::parse(
        r#"<DJ_PLAYLISTS Version="1.0.0"><COLLECTION Entries="1">
           <TRACK TrackID="5" Name="ok" Comments="bell&#7;here" Location="file://localhost/C:/Kit/a.mp3"/>
           </COLLECTION></DJ_PLAYLISTS>"#
            .as_bytes(),
    )
    .unwrap();
    let from_rekordbox = TrackInput {
        library_track: id(1),
        values: Values::Ready {
            in_rekordbox: true,
            attributes: read_attrs(&export.tracks[0]),
            rekordbox_holds_other_file: None,
        },
    };
    assert_eq!(
        refusal(from_rekordbox),
        Reason::UncarriableCharacter {
            attribute: "Comments".into()
        }
    );
    // A value from a file's tag.
    let from_tag = new(
        1,
        &[("Name", "form\u{c}feed"), ("Location", r"C:\New\a.mp3")],
    );
    assert_eq!(
        refusal(from_tag),
        Reason::UncarriableCharacter {
            attribute: "Name".into()
        }
    );
    for c in [
        '\u{0}', '\u{1}', '\u{8}', '\u{b}', '\u{1f}', '\u{fffe}', '\u{ffff}',
    ] {
        let track = known(
            1,
            &[
                ("TrackID", "5"),
                ("Location", "file://localhost/C:/Kit/a.mp3"),
                ("Genre", &format!("a{c}b")),
            ],
        );
        assert_eq!(
            refusal(track),
            Reason::UncarriableCharacter {
                attribute: "Genre".into()
            },
            "{c:?}"
        );
    }
}

#[test]
fn a_track_whose_values_cannot_be_written_safely_is_left_out_with_a_reason_code() {
    let location = ("Location", "file://localhost/C:/Kit/a.mp3");
    let cases: Vec<(TrackInput, Reason)> = vec![
        (
            known(1, &[("TrackID", "5"), ("Name", "no location")]),
            Reason::NoLocation,
        ),
        (new(1, &[("Name", "no location")]), Reason::NoLocation),
        (
            known(1, &[("TrackID", "5"), ("Location", "soundcloud:tracks:1")]),
            Reason::Location(LocationProblem::NotAFile),
        ),
        (
            known(
                1,
                &[
                    ("TrackID", "5"),
                    ("Location", "file://localhost/C:/a%zz.mp3"),
                ],
            ),
            Reason::Location(LocationProblem::Undecodable),
        ),
        (
            new(1, &[("Location", r"Music\relative.mp3")]),
            Reason::Location(LocationProblem::NotAFullPath),
        ),
        // A new track's Location is a path, never an address.
        (
            new(1, &[("Location", "file://localhost/C:/Kit/a.mp3")]),
            Reason::Location(LocationProblem::NotAFullPath),
        ),
        (known(1, &[("Name", "no id"), location]), Reason::BadTrackId),
        (known(1, &[("TrackID", ""), location]), Reason::BadTrackId),
        (
            known(1, &[("TrackID", "12x"), location]),
            Reason::BadTrackId,
        ),
        (known(1, &[("TrackID", "-3"), location]), Reason::BadTrackId),
        (
            known(1, &[("TrackID", "007"), location]),
            Reason::BadTrackId,
        ),
        (
            new(1, &[("TrackID", "5"), ("Location", r"C:\New\a.mp3")]),
            Reason::TrackIdOnNewTrack,
        ),
        (
            known(1, &[("TrackID", "5"), location, ("Bad Name", "x")]),
            Reason::BadAttributeName {
                attribute: "Bad Name".into(),
            },
        ),
        (
            known(1, &[("TrackID", "5"), location, ("x\" y=\"z", "x")]),
            Reason::BadAttributeName {
                attribute: "x\" y=\"z".into(),
            },
        ),
        (
            known(1, &[("TrackID", "5"), location, ("", "x")]),
            Reason::BadAttributeName {
                attribute: String::new(),
            },
        ),
        (
            known(
                1,
                &[("TrackID", "5"), location, ("Name", "a"), ("Name", "b")],
            ),
            Reason::RepeatedAttribute {
                attribute: "Name".into(),
            },
        ),
        // Same file as track 100 (Windows ignores letter case).
        (
            known(
                1,
                &[
                    ("TrackID", "5"),
                    ("Location", "file://localhost/c:/KIT/100.MP3"),
                ],
            ),
            Reason::DuplicateLocation,
        ),
        (
            new(1, &[("Location", r"C:\Kit\100.mp3")]),
            Reason::DuplicateLocation,
        ),
        (
            known(
                1,
                &[
                    ("TrackID", "1100"),
                    ("Location", "file://localhost/C:/Kit/other.mp3"),
                ],
            ),
            Reason::DuplicateTrackId,
        ),
    ];
    for (track, expected) in cases {
        let shown = format!("{track:?}");
        assert_eq!(refusal(track), expected, "{shown}");
    }
}

// ---- rule 5: Location ---------------------------------------------------------

#[test]
fn locations_are_written_fully_encoded_and_read_back_as_the_same_path() {
    let paths = [
        r"C:\Music\Rock & Roll #1 (100% live) + more, really!.mp3",
        "D:\\Café\\日本語 トラック 🚀.flac",
        r"e:\50%20 not an escape\a'b.wav",
        r"\\nas\Share One\x y.aiff",
    ];
    let input = SendInput {
        tracks: paths
            .iter()
            .enumerate()
            .map(|(i, p)| new(i as i64 + 1, &[("Location", p)]))
            .collect(),
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    for (i, path) in paths.iter().enumerate() {
        let track = read_track(&out, &read, i as i64 + 1);
        let file = track.location.as_ref().unwrap().as_file().unwrap();
        assert_eq!(file.to_windows().as_deref(), Some(*path));
        let written = track.location_raw();
        assert_eq!(out.sent[i].location(), written);
        assert!(written.starts_with("file://localhost/"));
        assert!(
            written
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~/:%".contains(&b)),
            "{written}"
        );
    }
    assert_eq!(
        out.sent[0].location(),
        "file://localhost/C:/Music/Rock%20%26%20Roll%20%231%20%28100%25%20live%29%20%2B%20more%2C%20really%21.mp3"
    );
}

#[test]
fn a_known_tracks_location_is_respelled_but_names_the_same_path() {
    let rekordbox = "file://localhost/C:/Kit/Low%20Tide%20#1%20(100%25%20Flip),%20caf%c3%a9.mp3";
    let input = SendInput {
        tracks: vec![known(1, &[("TrackID", "5"), ("Location", rekordbox)])],
        crates: vec![playlist("C", &[1])],
        ..SendInput::default()
    };
    let (out, read) = send(&input);
    assert_eq!(
        out.sent[0].location(),
        "file://localhost/C:/Kit/Low%20Tide%20%231%20%28100%25%20Flip%29%2C%20caf%C3%A9.mp3"
    );
    assert_eq!(
        read.tracks[0].location,
        rekordbox::location::decode(rekordbox)
    );
}

// ---- rule 6: the two top-level folders ----------------------------------------

#[test]
fn only_crates_and_playlists_sit_at_the_top_level_and_always_both() {
    let top_names = |read: &RekordboxXml| -> Vec<String> {
        read.playlists
            .children
            .iter()
            .map(|n| n.name().to_owned())
            .collect()
    };
    // Nothing to send at all.
    let (out, read) = send(&SendInput::default());
    assert_eq!(top_names(&read), ["Crates", "Playlists"]);
    assert!(read
        .playlists
        .children
        .iter()
        .all(|n| matches!(n, rekordbox::Node::Folder(f) if f.children.is_empty())));
    assert_eq!(read.playlists.name, "ROOT");
    assert_eq!(read.declared_entries, Some(0));
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
    assert_eq!(out.sent, []);

    // Whatever the trees hold, they stay below the two folders.
    let input = SendInput {
        tracks: vec![known_plain(1)],
        crates: vec![
            playlist("Playlists", &[1]),
            folder(
                "ROOT",
                vec![folder("Empty", vec![]), playlist("Crates", &[1])],
            ),
        ],
        playlists: vec![playlist("Crates", &[1]), playlist("Second", &[])],
        ..SendInput::default()
    };
    let (_, read) = send(&input);
    assert_eq!(top_names(&read), ["Crates", "Playlists"]);
    let names = |folder: &Folder| -> Vec<String> {
        folder
            .children
            .iter()
            .map(|n| n.name().to_owned())
            .collect()
    };
    assert_eq!(names(top(&read, "Crates")), ["Playlists", "ROOT"]);
    assert_eq!(names(top(&read, "Playlists")), ["Crates", "Second"]);
    let paths: Vec<Vec<&str>> = read
        .playlists
        .playlists()
        .into_iter()
        .map(|(folders, p)| folders.into_iter().chain([p.name.as_str()]).collect())
        .collect();
    assert_eq!(
        paths,
        [
            vec!["Crates", "Playlists"],
            vec!["Crates", "ROOT", "Crates"],
            vec!["Playlists", "Crates"],
            vec!["Playlists", "Second"],
        ]
    );
    // Counts match what each node holds: the reader has nothing to warn of.
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
}

#[test]
fn a_tree_rekordbox_would_misread_refuses_the_whole_send() {
    let with = |crates: Vec<Node>| {
        build(&SendInput {
            tracks: vec![known_plain(1)],
            crates,
            ..SendInput::default()
        })
    };
    assert_eq!(
        with(vec![folder("F", vec![playlist("", &[1])])]),
        Err(BuildError::EmptyName {
            path: path(&["Crates", "F"])
        })
    );
    // Same-name siblings would replace each other on import (§5.2).
    assert_eq!(
        with(vec![playlist("Twin", &[1]), folder("Twin", vec![])]),
        Err(BuildError::RepeatedName {
            path: path(&["Crates", "Twin"])
        })
    );
    assert_eq!(
        with(vec![playlist("bell\u{7}", &[1])]),
        Err(BuildError::UncarriableName {
            path: path(&["Crates", "bell\u{7}"])
        })
    );
    // The same name in different folders is fine.
    assert!(with(vec![
        playlist("Twin", &[1]),
        folder("F", vec![playlist("Twin", &[1])])
    ])
    .is_ok());

    let mut deep = playlist("leaf", &[1]);
    for i in 0..MAX_DEPTH {
        deep = folder(&format!("f{i}"), vec![deep]);
    }
    assert!(matches!(with(vec![deep]), Err(BuildError::TooDeep { .. })));

    assert_eq!(
        build(&SendInput {
            tracks: vec![known_plain(1), known_plain(1)],
            ..SendInput::default()
        }),
        Err(BuildError::RepeatedTrack(id(1)))
    );
}

// ---- the file on disk -------------------------------------------------------

/// A guard over `<temp>/data`, and that folder.
fn guarded() -> (tempfile::TempDir, WriteGuard, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let guard = WriteGuard::app_data(&data).unwrap();
    (dir, guard, data)
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn small_send() -> Outgoing {
    build(&SendInput {
        tracks: vec![known_plain(1), new_plain(2)],
        crates: vec![playlist("C", &[1, 2])],
        ..SendInput::default()
    })
    .unwrap()
}

#[test]
fn the_written_file_holds_exactly_the_built_bytes_and_nothing_is_left_beside_it() {
    let (_dir, guard, data) = guarded();
    let out = small_send();
    let dest = data.join("send.xml");
    write_file(&guard, &dest, &out).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), out.xml());
    assert_eq!(names_in(&data), ["send.xml"]);
    let read = RekordboxXml::read_file(&dest).unwrap();
    assert_eq!(read.tracks.len(), 2);

    // A second send replaces it whole.
    let again = build(&SendInput {
        tracks: vec![new_plain(3)],
        ..SendInput::default()
    })
    .unwrap();
    write_file(&guard, &dest, &again).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), again.xml());
    assert_eq!(names_in(&data), ["send.xml"]);
}

#[test]
fn a_failed_write_leaves_no_file() {
    let (_dir, guard, data) = guarded();
    // A folder is in the way of the destination.
    let dest = data.join("send.xml");
    guard.create_dir_all(&dest).unwrap();
    assert!(write_file(&guard, &dest, &small_send()).is_err());
    assert_eq!(names_in(&data), ["send.xml"]);
    assert!(dest.is_dir() && names_in(&dest).is_empty());

    // A destination whose folder doesn't exist.
    let missing = data.join("no such folder").join("send.xml");
    assert!(write_file(&guard, &missing, &small_send()).is_err());
    assert_eq!(names_in(&data), ["send.xml"]);
}

#[test]
fn a_destination_outside_the_apps_folders_is_refused_and_nothing_is_written() {
    let (dir, guard, data) = guarded();
    // Where a music folder, or rekordbox's own, would be.
    let music = dir.path().join("Music");
    std::fs::create_dir(&music).unwrap();
    let refused = write_file(&guard, &music.join("send.xml"), &small_send());
    assert!(
        matches!(refused, Err(GuardError::Outside(_))),
        "{refused:?}"
    );
    assert!(names_in(&music).is_empty());
    assert!(names_in(&data).is_empty());
}

// ---- the read-back check ---------------------------------------------------

#[test]
fn a_file_that_does_not_read_back_as_meant_is_refused() {
    let out = small_send();
    let top = [CRATES_FOLDER, PLAYLISTS_FOLDER].map(|name| xml::NodeElement::Folder {
        name: name.to_owned(),
        children: Vec::new(),
    });
    let own_text = xml::render(
        &out.sent
            .iter()
            .map(|t| xml::TrackElement {
                attributes: &t.attributes,
            })
            .collect::<Vec<_>>(),
        &top,
    );
    assert_eq!(read_back(&own_text, &out.sent, &top), Ok(()));

    // One value differs from what was meant.
    let mut meant = out.sent.clone();
    meant[0].attributes[1].1.push('!');
    assert!(read_back(&own_text, &meant, &top).is_err());
    // A track is missing from the file.
    assert!(read_back(
        &own_text,
        &[out.sent.clone(), out.sent.clone()].concat(),
        &top
    )
    .is_err());
    // An attribute the file holds wasn't meant.
    let mut meant = out.sent.clone();
    meant[1].attributes.pop();
    assert!(read_back(&own_text, &meant, &top).is_err());
    // The playlists differ.
    assert!(read_back(&own_text, &out.sent, &top[..1]).is_err());
    // Not well-formed at all.
    assert!(read_back(&own_text[..own_text.len() - 20], &out.sent, &top).is_err());
}
