//! The PLAYLISTS tree: folders and playlists in their order, and each
//! playlist's entries resolved to COLLECTION tracks.
//!
//! An entry names its track by `TrackID` (`KeyType="0"`, what rekordbox
//! exports) or by `Location` (`KeyType="1"`). Entries are resolved after
//! the whole file is read, so COLLECTION may come before or after
//! PLAYLISTS. An entry that points at no track is kept, marked
//! [`EntryTarget::NoTrack`], and counted.

use std::collections::HashMap;

use super::collection::Track;
use super::location;
use super::skip::{SkipCounts, SkipReason, CUE_ANALYSIS_PLAYLIST};
use super::SkippedTrack;

/// A folder (`Type="0"`). The tree's top is rekordbox's `ROOT` folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Folder {
    pub name: String,
    /// Folders and playlists, in rekordbox's order.
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Folder(Folder),
    Playlist(Playlist),
}

impl Node {
    pub fn name(&self) -> &str {
        match self {
            Node::Folder(f) => &f.name,
            Node::Playlist(p) => &p.name,
        }
    }
}

/// A playlist (`Type="1"`).
#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    pub name: String,
    pub key_type: KeyType,
    /// In playlist order.
    pub entries: Vec<Entry>,
}

/// How a playlist's entries name their tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyType {
    /// `KeyType="0"`: by `TrackID`.
    TrackId,
    /// `KeyType="1"`: by `Location`.
    Location,
}

/// One playlist entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The `Key` as written: a TrackID or a Location.
    pub key: String,
    pub target: EntryTarget,
}

/// What an entry points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryTarget {
    /// This index into the export's tracks (a streaming track included).
    Track(usize),
    /// A COLLECTION track that was skipped, for this reason.
    Skipped(SkipReason),
    /// No track in COLLECTION.
    NoTrack,
}

impl Folder {
    /// Every playlist below this folder, depth first, in order, with the
    /// folder names leading to it.
    pub fn playlists(&self) -> Vec<(Vec<&str>, &Playlist)> {
        let mut out = Vec::new();
        self.collect(&mut Vec::new(), &mut out);
        out
    }

    fn collect<'a>(&'a self, path: &mut Vec<&'a str>, out: &mut Vec<(Vec<&'a str>, &'a Playlist)>) {
        for child in &self.children {
            match child {
                Node::Folder(folder) => {
                    path.push(&folder.name);
                    folder.collect(path, out);
                    path.pop();
                }
                Node::Playlist(playlist) => out.push((path.clone(), playlist)),
            }
        }
    }

    fn entries_mut(&mut self, visit: &mut impl FnMut(KeyType, &mut Entry)) {
        for child in &mut self.children {
            match child {
                Node::Folder(folder) => folder.entries_mut(visit),
                Node::Playlist(playlist) => {
                    for entry in &mut playlist.entries {
                        visit(playlist.key_type, entry);
                    }
                }
            }
        }
    }
}

/// Takes the CUE Analysis Playlist out of ROOT, counting it and its entries.
pub(crate) fn drop_cue_analysis(root: &mut Folder, skips: &mut SkipCounts) {
    root.children.retain(|child| match child {
        Node::Playlist(p) if p.name == CUE_ANALYSIS_PLAYLIST => {
            skips.cue_analysis_playlists += 1;
            skips.cue_analysis_entries += p.entries.len();
            false
        }
        _ => true,
    });
}

/// Resolves every entry below `root` against the tracks, counting entries
/// that point at skipped or streaming tracks. Returns how many point at no
/// track.
pub(crate) fn resolve(
    root: &mut Folder,
    tracks: &[Track],
    skipped: &[SkippedTrack],
    skips: &mut SkipCounts,
) -> usize {
    let mut by_id: HashMap<u64, EntryTarget> = HashMap::new();
    let mut by_location: HashMap<String, EntryTarget> = HashMap::new();
    // The first track with an id or location wins, as a warning already
    // said when the duplicate was read.
    for (i, track) in tracks.iter().enumerate() {
        let target = EntryTarget::Track(i);
        if let Some(id) = track.track_id {
            by_id.entry(id).or_insert(target);
        }
        if let Ok(location) = &track.location {
            by_location.entry(location.match_key()).or_insert(target);
        }
    }
    for track in skipped {
        let target = EntryTarget::Skipped(track.reason);
        if let Some(id) = track.track_id {
            by_id.entry(id).or_insert(target);
        }
        if let Ok(location) = location::decode(&track.location) {
            by_location.entry(location.match_key()).or_insert(target);
        }
    }

    let mut no_track = 0;
    root.entries_mut(&mut |key_type, entry| {
        let found = match key_type {
            KeyType::TrackId => entry
                .key
                .parse::<u64>()
                .ok()
                .filter(|_| entry.key.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|id| by_id.get(&id)),
            KeyType::Location => location::decode(&entry.key)
                .ok()
                .and_then(|location| by_location.get(&location.match_key())),
        };
        entry.target = found.copied().unwrap_or(EntryTarget::NoTrack);
        match entry.target {
            EntryTarget::Track(i) if tracks[i].is_streaming() => {
                skips.entries.add(SkipReason::Streaming)
            }
            EntryTarget::Skipped(reason) => skips.entries.add(reason),
            EntryTarget::NoTrack => no_track += 1,
            EntryTarget::Track(_) => {}
        }
    });
    no_track
}
