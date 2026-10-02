//! The generated music folder and rekordbox collection both runs use, and
//! the checks they share on a file read back.

use std::path::Path;

use tracklist_pro_lib::rekordbox::{self, EntryTarget, RekordboxXml};

use super::harness::{Song, World};
use super::rekordbox_side::{key_of, Node, Rekordbox, Track, ANALYSIS};

const fn song(rel: &'static str, title: &'static str, genre: &'static str, audio: u32) -> Song {
    Song {
        rel,
        title,
        artist: "Generated Artist",
        genre,
        audio,
    }
}

/// Files rekordbox has, with the characters a `Location` treats specially:
/// spaces, `#`, brackets, a comma, `+`, `!` and a non-ASCII letter.
pub const KNOWN: [Song; 6] = [
    song("House/Generated One.wav", "Generated One", "House", 1),
    song(
        "House/Generated Two #2 (Extended Mix).wav",
        "Generated Two",
        "House",
        2,
    ),
    song(
        "Techno/Generated Three é.wav",
        "Generated Three",
        "Techno",
        3,
    ),
    song(
        "Techno/Deep/Generated Four, Part 1.wav",
        "Generated Four",
        "Techno",
        4,
    ),
    song(
        "Generated Five + Friends!.wav",
        "Generated Five",
        "Disco",
        5,
    ),
    song("Disco/Generated Six.wav", "Generated Six", "Disco", 6),
];

/// Files rekordbox has never seen.
pub const UNKNOWN: [Song; 3] = [
    song("New/Generated Seven.wav", "Generated Seven", "Garage", 7),
    song(
        "New/Generated Eight (Dub) & More.wav",
        "Generated Eight",
        "Garage",
        8,
    ),
    song("New/Generated Nine.wav", "Generated Nine", "Garage", 9),
];

/// One track's audio under two names: duplicates. rekordbox has neither
/// until a test says so.
pub const TWINS: [Song; 2] = [
    song("Copies/Generated Ten.wav", "Generated Ten", "House", 10),
    song(
        "Copies/Backup/Generated Ten copy.wav",
        "Generated Ten",
        "House",
        10,
    ),
];

/// A track rekordbox has whose file was never in the music folder.
pub const NEVER_ON_DISK: Song = song("Gone/Generated Lost.wav", "Generated Lost", "House", 99);

/// Every file in the music folder.
pub fn songs() -> Vec<Song> {
    KNOWN
        .iter()
        .chain(&UNKNOWN)
        .chain(&TWINS)
        .copied()
        .collect()
}

/// How many tracks All music shows: the twins are one track.
pub const TRACKS_IN_ALL_MUSIC: u32 = (KNOWN.len() + UNKNOWN.len() + 1) as u32;

/// rekordbox's collection: every [`KNOWN`] file, analysed, played and with
/// cues, the track whose file is gone (it has the highest TrackID), and
/// playlists of the user's own, some in folders.
pub fn collection(world: &World) -> Rekordbox {
    let mut rb = Rekordbox::new();
    let mut ids = Vec::new();
    for (i, song) in KNOWN.iter().enumerate() {
        let bpm = format!("{}.00", 120 + i);
        let key = ["8A", "9A", "10A", "11A", "12A", "1A"][i];
        ids.push(rb.has(&world.path_of(song), song, 3 + i as u32, &bpm, key));
    }
    let lost = rb.has(
        &world.path_of(&NEVER_ON_DISK),
        &NEVER_ON_DISK,
        40,
        "128.00",
        "4A",
    );
    rb.playlists = vec![
        Node::Folder(
            "Sets".into(),
            vec![
                Node::Playlist("Opening".into(), vec![ids[0], ids[1], lost]),
                Node::Folder(
                    "Late".into(),
                    vec![Node::Playlist("Peak".into(), vec![ids[2], ids[3]])],
                ),
            ],
        ),
        Node::Playlist("Everything I own".into(), ids),
    ];
    rb
}

/// The track of a read-back file at `path`.
pub fn track_at<'a>(sent: &'a RekordboxXml, path: &Path) -> Option<&'a rekordbox::Track> {
    let key = key_of(path);
    sent.tracks
        .iter()
        .find(|t| t.location.as_ref().is_ok_and(|l| l.match_key() == key))
}

/// The `Location` keys of the entries of the crate or playlist at `path`
/// (from `Crates` or `Playlists` down), in order. Panics if an entry
/// resolves to no track of the file.
pub fn entries(sent: &RekordboxXml, path: &[&str]) -> Vec<String> {
    let (folders, name) = path.split_at(path.len() - 1);
    let playlists = sent.playlists.playlists();
    let (_, playlist) = playlists
        .iter()
        .find(|(at, playlist)| at == folders && playlist.name == name[0])
        .unwrap_or_else(|| panic!("the file has no {path:?}"));
    playlist
        .entries
        .iter()
        .map(|entry| match entry.target {
            EntryTarget::Track(i) => sent.tracks[i].location.as_ref().unwrap().match_key(),
            other => panic!("an entry of {path:?} names no track of the file: {other:?}"),
        })
        .collect()
}

/// The names of the folders and playlists directly inside the folder at
/// `path` (below ROOT).
pub fn children(sent: &RekordboxXml, path: &[&str]) -> Vec<String> {
    let mut folder = &sent.playlists;
    for name in path {
        folder = folder
            .children
            .iter()
            .find_map(|n| match n {
                rekordbox::Node::Folder(f) if f.name == *name => Some(f),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the file has no folder {path:?}"));
    }
    folder
        .children
        .iter()
        .map(|n| n.name().to_owned())
        .collect()
}

/// Fails unless `sent` is `known`, rekordbox's own entry, sent back: its
/// TrackID, every attribute rekordbox wrote, value for value and in
/// rekordbox's order, and nothing else. The analysis fields are left out
/// (no BPM, no key, no grid, no cues), which is what keeps rekordbox's own
/// on import. `Location` names the same path; its spelling is checked by
/// the caller where it matters.
pub fn assert_is_rekordboxs_own_entry(sent: &rekordbox::Track, known: &Track) {
    let name = known.get("Name");
    assert_eq!(sent.track_id, Some(known.id), "{name}: TrackID");
    for analysis in ANALYSIS {
        assert_eq!(sent.attrs.get(analysis), None, "{name}: {analysis} is sent");
    }
    assert!(sent.tempos.is_empty(), "{name}: a beat grid is sent");
    assert!(sent.cues.is_empty(), "{name}: cues are sent");

    let id = known.id.to_string();
    let expected: Vec<(&str, &str)> = std::iter::once(("TrackID", id.as_str()))
        .chain(
            known
                .attrs
                .iter()
                .filter(|(n, _)| !ANALYSIS.contains(&n.as_str()))
                .map(|(n, v)| (n.as_str(), v.as_str())),
        )
        .collect();
    let actual: Vec<(&str, &str)> = sent.attrs.iter().collect();
    let names = |list: &[(&str, &str)]| list.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>();
    assert_eq!(names(&actual), names(&expected), "{name}: attributes");
    for ((attribute, value), (_, expected)) in actual.iter().zip(&expected) {
        if *attribute == "Location" {
            assert_eq!(
                sent.location.as_ref().unwrap().match_key(),
                known.location_key(),
                "{name}: Location"
            );
        } else {
            assert_eq!(value, expected, "{name}: {attribute}");
        }
    }
}
