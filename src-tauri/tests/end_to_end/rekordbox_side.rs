//! rekordbox's side of a run: a generated collection that can be saved as
//! an export, and that takes an import of the app's file the way ROADMAP
//! §5.2 records rekordbox 7 as doing.
//!
//! The import follows those notes and nothing else:
//!
//! - A track is matched to rekordbox's own by `Location`, never TrackID,
//!   and rekordbox goes on spelling that `Location` its own way.
//! - A track rekordbox has is asked about and answered Yes: a full
//!   overwrite from the XML. An attribute the XML leaves out is reset
//!   (blank), since omission can't be relied on. The two analysis
//!   attributes are the exception T3 confirmed: left out, rekordbox keeps
//!   its own; given, they're loaded over it. A grid or cues the XML
//!   doesn't mention are left alone.
//! - A track rekordbox doesn't have is added, gets a TrackID of
//!   rekordbox's own choosing, and is analysed by rekordbox.
//! - A playlist replaces the one of the same name in the same folder;
//!   playlists and tracks the XML doesn't mention stay.
//! - A playlist entry resolves only against the same file's COLLECTION.
//!
//! It stands in for rekordbox so a test can say what the file would do
//! there. It isn't evidence about rekordbox itself.

use std::fmt::Write as _;
use std::path::Path;

use tracklist_pro_lib::rekordbox::location::decode;
use tracklist_pro_lib::rekordbox::{self, EntryTarget, RekordboxXml};

use super::harness::{Song, SONG_SECONDS};

/// The attributes that are rekordbox's analysis.
pub const ANALYSIS: [&str; 2] = ["AverageBpm", "Tonality"];

/// One track of rekordbox's collection.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: u64,
    /// Every `TRACK` attribute but `TrackID`, in rekordbox's order.
    pub attrs: Vec<(String, String)>,
    /// The beat grid's tempo (`TEMPO Bpm`).
    pub grid_bpm: String,
    /// Each cue's `Start`: hot cues A to C, then a memory cue.
    pub cues: Vec<String>,
}

impl Track {
    pub fn get(&self, name: &str) -> &str {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map_or("", |(_, v)| v)
    }

    fn set(&mut self, name: &str, value: &str) {
        match self.attrs.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = value.to_owned(),
            None => self.attrs.push((name.to_owned(), value.to_owned())),
        }
    }

    /// Drops an attribute: rekordbox never wrote it for this track.
    pub fn unset(&mut self, name: &str) {
        self.attrs.retain(|(n, _)| n != name);
    }

    /// The key two spellings of one `Location` share.
    pub fn location_key(&self) -> String {
        decode(self.get("Location")).unwrap().match_key()
    }
}

/// A node of rekordbox's playlist tree, below ROOT.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Folder(String, Vec<Node>),
    /// A playlist: its tracks' TrackIDs, in order.
    Playlist(String, Vec<u64>),
}

/// rekordbox's collection.
#[derive(Debug, Clone, PartialEq)]
pub struct Rekordbox {
    pub tracks: Vec<Track>,
    pub playlists: Vec<Node>,
    next_id: u64,
}

/// A path as rekordbox 7 writes a `Location`: `# ( ) , + !` raw, anything
/// else outside letters, digits and `- . _ ~ / :` as lowercase `%xx`
/// (§5.3).
pub fn location_of(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://localhost/");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:(),+#!".contains(&byte) {
            out.push(char::from(byte));
        } else {
            write!(out, "%{byte:02x}").unwrap();
        }
    }
    out
}

/// The key of a path on disk, comparable with [`Track::location_key`].
pub fn key_of(path: &Path) -> String {
    decode(&location_of(path)).unwrap().match_key()
}

fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Rekordbox {
    pub fn new() -> Rekordbox {
        Rekordbox {
            tracks: Vec::new(),
            playlists: Vec::new(),
            next_id: 9001,
        }
    }

    /// rekordbox imports `song`'s file at `path`, analyses it, and the
    /// user plays, rates and renames it. Returns its TrackID.
    pub fn has(&mut self, path: &Path, song: &Song, plays: u32, bpm: &str, key: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let attrs = [
            // The user retitled it in rekordbox: the file's tag says
            // otherwise, and rekordbox's value is the one to keep.
            ("Name", format!("{} (as rekordbox has it)", song.title)),
            ("Artist", song.artist.to_owned()),
            ("Composer", String::new()),
            ("Album", String::new()),
            ("Grouping", String::new()),
            ("Genre", song.genre.to_owned()),
            ("Kind", "WAV File".to_owned()),
            ("Size", "48000".to_owned()),
            ("TotalTime", SONG_SECONDS.to_string()),
            ("DiscNumber", "0".to_owned()),
            ("TrackNumber", "0".to_owned()),
            ("Year", "0".to_owned()),
            ("AverageBpm", bpm.to_owned()),
            ("DateAdded", "2026-01-15".to_owned()),
            ("BitRate", "128".to_owned()),
            ("SampleRate", "8000".to_owned()),
            ("Comments", "a note made in rekordbox & kept".to_owned()),
            ("PlayCount", plays.to_string()),
            ("Rating", "204".to_owned()),
            ("Location", location_of(path)),
            ("Remixer", String::new()),
            ("Tonality", key.to_owned()),
            ("Label", String::new()),
            ("Mix", String::new()),
        ];
        self.tracks.push(Track {
            id,
            attrs: attrs.into_iter().map(|(n, v)| (n.to_owned(), v)).collect(),
            grid_bpm: bpm.to_owned(),
            cues: vec![
                "0.250".into(),
                "1.000".into(),
                "2.000".into(),
                "0.500".into(),
            ],
        });
        id
    }

    /// The track at `path`, if rekordbox has one.
    pub fn at(&self, path: &Path) -> Option<&Track> {
        let key = key_of(path);
        self.tracks.iter().find(|t| t.location_key() == key)
    }

    pub fn highest_id(&self) -> u64 {
        self.tracks.iter().map(|t| t.id).max().unwrap_or(0)
    }

    /// File → Export Collection.
    pub fn export(&self) -> String {
        self.export_declaring(self.tracks.len())
    }

    /// An export whose COLLECTION says it holds `entries` tracks. More
    /// than it holds makes it an incomplete export (§5.3).
    pub fn export_declaring(&self, entries: usize) -> String {
        let mut xml = String::new();
        xml.push_str(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n<DJ_PLAYLISTS Version=\"1.0.0\">\n",
        );
        xml.push_str("  <PRODUCT Name=\"rekordbox\" Version=\"7.2.19\" Company=\"AlphaTheta\"/>\n");
        writeln!(xml, "  <COLLECTION Entries=\"{entries}\">").unwrap();
        for track in &self.tracks {
            write!(xml, "    <TRACK TrackID=\"{}\"", track.id).unwrap();
            for (name, value) in &track.attrs {
                write!(xml, " {name}=\"{}\"", escaped(value)).unwrap();
            }
            xml.push_str(">\n");
            if !track.grid_bpm.is_empty() {
                writeln!(
                    xml,
                    "      <TEMPO Inizio=\"0.025\" Bpm=\"{}\" Metro=\"4/4\" Battito=\"1\"/>",
                    track.grid_bpm
                )
                .unwrap();
            }
            for (i, start) in track.cues.iter().enumerate() {
                // The last one is a memory cue, the others hot cues.
                let num = if i + 1 == track.cues.len() {
                    -1
                } else {
                    i as i32
                };
                writeln!(
                    xml,
                    "      <POSITION_MARK Name=\"\" Type=\"0\" Start=\"{start}\" Num=\"{num}\"/>"
                )
                .unwrap();
            }
            xml.push_str("    </TRACK>\n");
        }
        xml.push_str("  </COLLECTION>\n  <PLAYLISTS>\n");
        writeln!(
            xml,
            "    <NODE Type=\"0\" Name=\"ROOT\" Count=\"{}\">",
            self.playlists.len()
        )
        .unwrap();
        fn nodes(xml: &mut String, list: &[Node], depth: usize) {
            let pad = "  ".repeat(depth);
            for node in list {
                match node {
                    Node::Folder(name, children) => {
                        writeln!(
                            xml,
                            "{pad}<NODE Name=\"{}\" Type=\"0\" Count=\"{}\">",
                            escaped(name),
                            children.len()
                        )
                        .unwrap();
                        nodes(xml, children, depth + 1);
                        writeln!(xml, "{pad}</NODE>").unwrap();
                    }
                    Node::Playlist(name, keys) => {
                        writeln!(
                            xml,
                            "{pad}<NODE Name=\"{}\" Type=\"1\" KeyType=\"0\" Entries=\"{}\">",
                            escaped(name),
                            keys.len()
                        )
                        .unwrap();
                        for key in keys {
                            writeln!(xml, "{pad}  <TRACK Key=\"{key}\"/>").unwrap();
                        }
                        writeln!(xml, "{pad}</NODE>").unwrap();
                    }
                }
            }
        }
        nodes(&mut xml, &self.playlists, 3);
        xml.push_str("    </NODE>\n  </PLAYLISTS>\n</DJ_PLAYLISTS>\n");
        xml
    }

    /// The user imports the app's file: Import to Collection, Import
    /// Playlist, and Yes to every dialog. Returns how many dialogs there
    /// were (one per track rekordbox already had).
    pub fn import(&mut self, sent: &RekordboxXml) -> usize {
        let mut dialogs = 0;
        // The file's TrackIDs only tie its entries to its tracks.
        let mut mine: Vec<u64> = Vec::new();
        for track in &sent.tracks {
            let key = track.location.as_ref().unwrap().match_key();
            let given = |name: &str| track.attrs.get(name);
            if let Some(known) = self.tracks.iter_mut().find(|t| t.location_key() == key) {
                dialogs += 1;
                let names: Vec<String> = known.attrs.iter().map(|(n, _)| n.clone()).collect();
                for name in names {
                    match given(&name) {
                        // Matched by Location: rekordbox keeps its own
                        // spelling of it.
                        _ if name == "Location" => {}
                        Some(value) => known.set(&name, value),
                        None if ANALYSIS.contains(&name.as_str()) => {}
                        None => known.set(&name, ""),
                    }
                }
                if !track.tempos.is_empty() || !track.cues.is_empty() {
                    known.grid_bpm = String::new();
                    known.cues = Vec::new();
                }
                mine.push(known.id);
            } else {
                let id = self.next_id;
                self.next_id += 1;
                let mut added = Track {
                    id,
                    attrs: track
                        .attrs
                        .iter()
                        .filter(|(name, _)| *name != "TrackID")
                        .map(|(n, v)| (n.to_owned(), v.to_owned()))
                        .collect(),
                    grid_bpm: "120.00".to_owned(),
                    cues: Vec::new(),
                };
                // rekordbox spells the Location its own way, and analyses a
                // new track itself.
                let path = track.location.as_ref().unwrap().as_file().unwrap();
                added.set(
                    "Location",
                    &location_of(Path::new(&path.to_windows().unwrap())),
                );
                added.set("AverageBpm", "120.00");
                added.set("Tonality", "1A");
                if added.get("PlayCount").is_empty() {
                    added.set("PlayCount", "0");
                }
                self.tracks.push(added);
                mine.push(id);
            }
        }

        fn merge(into: &mut Vec<Node>, from: &[rekordbox::Node], mine: &[u64]) {
            for node in from {
                match node {
                    rekordbox::Node::Folder(folder) => {
                        let at = into
                            .iter()
                            .position(
                                |n| matches!(n, Node::Folder(name, _) if *name == folder.name),
                            )
                            .unwrap_or_else(|| {
                                into.push(Node::Folder(folder.name.clone(), Vec::new()));
                                into.len() - 1
                            });
                        let Node::Folder(_, children) = &mut into[at] else {
                            unreachable!()
                        };
                        merge(children, &folder.children, mine);
                    }
                    rekordbox::Node::Playlist(playlist) => {
                        // An entry whose track isn't in the file's own
                        // COLLECTION is dropped without a word (T1).
                        let keys: Vec<u64> = playlist
                            .entries
                            .iter()
                            .filter_map(|e| match e.target {
                                EntryTarget::Track(i) => Some(mine[i]),
                                _ => None,
                            })
                            .collect();
                        let replaced = Node::Playlist(playlist.name.clone(), keys);
                        match into.iter().position(
                            |n| matches!(n, Node::Playlist(name, _) if *name == playlist.name),
                        ) {
                            Some(at) => into[at] = replaced,
                            None => into.push(replaced),
                        }
                    }
                }
            }
        }
        merge(&mut self.playlists, &sent.playlists.children, &mine);
        dialogs
    }

    /// The user deletes the playlist or folder at `path` in rekordbox.
    pub fn delete_playlist(&mut self, path: &[&str]) {
        fn remove(list: &mut Vec<Node>, path: &[&str]) {
            let name_of = |n: &Node| match n {
                Node::Folder(name, _) | Node::Playlist(name, _) => name.clone(),
            };
            if let [last] = path {
                list.retain(|n| name_of(n) != *last);
            } else if let Some(Node::Folder(_, children)) =
                list.iter_mut().find(|n| name_of(n) == path[0])
            {
                remove(children, &path[1..]);
            }
        }
        remove(&mut self.playlists, path);
    }

    /// The playlist at `path` (folder names from below ROOT down).
    pub fn playlist(&self, path: &[&str]) -> Option<&Vec<u64>> {
        let mut list = &self.playlists;
        for (i, name) in path.iter().enumerate() {
            let node = list.iter().find(|n| match n {
                Node::Folder(n, _) | Node::Playlist(n, _) => n == name,
            })?;
            match node {
                Node::Folder(_, children) => list = children,
                Node::Playlist(_, keys) if i + 1 == path.len() => return Some(keys),
                Node::Playlist(..) => return None,
            }
        }
        None
    }
}
