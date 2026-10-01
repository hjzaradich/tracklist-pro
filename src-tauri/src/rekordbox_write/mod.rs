//! Writing the rekordbox XML a send hands to rekordbox (ROADMAP 1.9, §5.2).
//!
//! [`build`] turns the values to send and the crate and playlist trees
//! into the file's bytes, plus a record of exactly what went in and what
//! was left out; [`write_file`] puts the bytes on disk through the write
//! guard. [`gather`] collects a send's input: each track's values come
//! from [`crate::send_values`], which decides them; nothing here does.
//! The send flow (1aF) calls these.
//!
//! The file is imported into the user's real collection, where a "Yes"
//! overwrites the whole track (§5.2), so the rules are strict:
//!
//! - **Rule 1.** Every attribute given for a track is written, in the
//!   order given, value for value. Nothing is defaulted or filled in.
//!   Reserved characters are escaped, and tab, line feed and carriage
//!   return are written as character references so a parser gives them
//!   back unchanged.
//! - **Rule 3.** `AverageBpm` and `Tonality` are never written, for any
//!   track, and no `TEMPO` or `POSITION_MARK` either: a track rekordbox
//!   has keeps its own analysis (case A), and a new one is analysed by
//!   rekordbox. Case B (managed copies) arrives in Phase 2.
//! - **Rule 4.** COLLECTION holds every track a playlist or crate entry
//!   names, plus every track rekordbox doesn't know yet. A track rekordbox
//!   already has that no entry names is not sent: it would only raise a
//!   Yes/No dialog.
//! - **Rule 5.** `Location` is written fully percent-encoded
//!   ([`location`]), for known tracks too: it names the same path.
//! - **Rule 6.** PLAYLISTS holds exactly two folders below ROOT, `Crates`
//!   and `Playlists`, always both.
//!
//! **A track that can't be sent** is left out of COLLECTION, and so is
//! every entry naming it (rekordbox would drop such an entry silently,
//! §5.2). Both are reported in [`Outgoing::left_out`] with a reason code.
//! One track's problem never stops the others.
//!
//! **TrackIDs** only tie this file's playlist entries to its tracks;
//! rekordbox assigns its own on import (§5.2). A known track keeps
//! rekordbox's; a new one gets a number above every TrackID in the last
//! rekordbox read, so it can't be mistaken for another track's.
//!
//! Before the bytes are returned they're read back with the app's own
//! reader and compared with what was meant; a difference refuses the send.

pub mod location;
mod xml;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::library::LibraryTrackId;
use crate::rekordbox::attrs::digits;
use rusqlite::Connection;
use unicode_normalization::UnicodeNormalization;

use crate::paths::Volumes;
use crate::rekordbox::{self, RekordboxXml};
use crate::send_values::{self, CannotSend, Outcome, SendValues, SiblingEntry};
use crate::write_guard::{GuardError, WriteGuard};

pub use crate::send_values::ANALYSIS_ATTRIBUTES;
pub use location::LocationProblem;

/// The top-level folder the crate tree is written under.
pub const CRATES_FOLDER: &str = "Crates";
/// The top-level folder the playlist tree is written under.
pub const PLAYLISTS_FOLDER: &str = "Playlists";

/// How deep folders may nest below `Crates` or `Playlists`. The reader
/// refuses a file nested far deeper; no real tree comes near either.
const MAX_DEPTH: usize = 64;

/// Everything one send is made from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SendInput {
    /// The Library tracks the send may carry, each at most once.
    /// COLLECTION keeps this order.
    pub tracks: Vec<TrackInput>,
    /// The crate tree, written under [`CRATES_FOLDER`].
    pub crates: Vec<Node>,
    /// The playlist tree, written under [`PLAYLISTS_FOLDER`].
    pub playlists: Vec<Node>,
    /// The highest `TrackID` in the last rekordbox read (every track of
    /// it, sent or not); 0 if rekordbox was never read. New tracks are
    /// numbered above it.
    pub highest_rekordbox_track_id: u64,
}

/// One Library track's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInput {
    pub library_track: LibraryTrackId,
    pub values: Values,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Values {
    Ready {
        /// rekordbox already has this track at this file. Its attributes
        /// are then rekordbox's own, `TrackID` and `Location` (as
        /// rekordbox wrote it) included.
        in_rekordbox: bool,
        /// Each `TRACK` attribute to write, name and value, in order. For
        /// a track rekordbox doesn't know, `Location` is the file's full
        /// Windows path and there's no `TrackID`.
        attributes: Vec<(String, String)>,
        /// rekordbox holds another file of this track
        /// ([`send_values::TrackValues::rekordbox_holds_other_file`]).
        /// Carried through to [`SentTrack`] for the send flow to warn
        /// about; it changes nothing in the file.
        rekordbox_holds_other_file: Option<SiblingEntry>,
    },
    /// There are no values to send for this track.
    CannotSend(CannotSend),
}

impl From<SendValues> for TrackInput {
    fn from(values: SendValues) -> TrackInput {
        TrackInput {
            library_track: values.library_track,
            values: match values.outcome {
                Outcome::CannotSend(why) => Values::CannotSend(why),
                Outcome::Ready(track) => Values::Ready {
                    in_rekordbox: track.in_rekordbox,
                    attributes: track
                        .values
                        .into_iter()
                        .map(|v| (v.attribute, v.value))
                        .collect(),
                    rekordbox_holds_other_file: track.rekordbox_holds_other_file,
                },
            },
        }
    }
}

/// Collects a send's input from the database: the values of every track
/// in `tracks` ([`send_values::send_values`]) and the highest `TrackID` of
/// the last rekordbox read. Call it inside one read transaction, so the
/// values and that `TrackID` come from the same read.
pub fn gather(
    conn: &Connection,
    volumes: &impl Volumes,
    tracks: &[LibraryTrackId],
    crates: Vec<Node>,
    playlists: Vec<Node>,
) -> rusqlite::Result<SendInput> {
    let highest: Option<i64> =
        conn.query_row("SELECT max(track_id) FROM rekordbox_track", [], |r| {
            r.get(0)
        })?;
    Ok(SendInput {
        tracks: send_values::send_values(conn, volumes, tracks)?
            .into_iter()
            .map(TrackInput::from)
            .collect(),
        crates,
        playlists,
        highest_rekordbox_track_id: highest.and_then(|id| u64::try_from(id).ok()).unwrap_or(0),
    })
}

/// A folder, crate or playlist to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Folder {
        name: String,
        children: Vec<Node>,
    },
    /// A crate or a playlist: its tracks in order. A track may repeat.
    Playlist {
        name: String,
        entries: Vec<LibraryTrackId>,
    },
}

/// Why a track was left out. A code for the send flow to word, not text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// It had no values to send.
    NoValues(CannotSend),
    /// An entry names it, but [`SendInput::tracks`] doesn't hold it.
    NotGiven,
    /// Its values have no `Location`.
    NoLocation,
    /// Its `Location` can't be written.
    Location(LocationProblem),
    /// rekordbox has it, but its values lack a whole-number `TrackID`.
    BadTrackId,
    /// rekordbox doesn't have it, yet its values carry a `TrackID`.
    TrackIdOnNewTrack,
    /// This attribute's name can't be written as an XML name.
    BadAttributeName { attribute: String },
    /// This attribute is given twice.
    RepeatedAttribute { attribute: String },
    /// This attribute's value holds a character XML can't carry. The
    /// value is never stripped or rewritten.
    UncarriableCharacter { attribute: String },
    /// An earlier track of this send is at the same `Location`.
    DuplicateLocation,
    /// An earlier track of this send has the same `TrackID`.
    DuplicateTrackId,
}

/// A playlist entry that wasn't written because its track wasn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedEntry {
    /// The folder names down to the crate or playlist, from `Crates` or
    /// `Playlists`.
    pub path: Vec<String>,
    /// Where it was among the entries given, from 0.
    pub position: usize,
}

/// A track that wasn't sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    pub library_track: LibraryTrackId,
    pub reason: Reason,
    /// Every entry that named it, in tree order.
    pub entries: Vec<DroppedEntry>,
}

/// A track as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentTrack {
    pub library_track: LibraryTrackId,
    pub in_rekordbox: bool,
    /// Its `TrackID` in this file.
    pub track_id: u64,
    /// Every attribute written, in order, values unescaped.
    pub attributes: Vec<(String, String)>,
    /// As given ([`Values::Ready`]): rekordbox holds another file of this
    /// track, so this send adds a second entry beside it.
    pub rekordbox_holds_other_file: Option<SiblingEntry>,
}

impl SentTrack {
    /// The fields sent: every attribute written except `TrackID`, which
    /// only numbers the track within the file.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attributes
            .iter()
            .filter(|(name, _)| name != "TrackID")
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// The `Location` written.
    pub fn location(&self) -> &str {
        self.fields()
            .find(|(name, _)| *name == "Location")
            .map_or("", |(_, value)| value)
    }
}

/// A send, ready to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    xml: Vec<u8>,
    /// COLLECTION's tracks, in file order.
    pub sent: Vec<SentTrack>,
    /// The tracks left out, each with the entries dropped with it: first
    /// those in [`SendInput::tracks`], in that order, then tracks only an
    /// entry named.
    pub left_out: Vec<LeftOut>,
    /// Crates and playlists that were given entries and are written with
    /// none, because every one was dropped. Importing one empties
    /// rekordbox's playlist of that name (§5.2).
    pub emptied: Vec<Vec<String>>,
    /// Tracks rekordbox already has that no entry names: not sent, and
    /// nothing is wrong with them.
    pub not_needed: Vec<LibraryTrackId>,
}

impl Outgoing {
    /// The file's bytes (UTF-8).
    pub fn xml(&self) -> &[u8] {
        &self.xml
    }
}

/// Why no file can be made at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// [`SendInput::tracks`] holds this track twice.
    RepeatedTrack(LibraryTrackId),
    /// A folder, crate or playlist has no name. `path` leads to it.
    EmptyName { path: Vec<String> },
    /// This name holds a character XML can't carry.
    UncarriableName { path: Vec<String> },
    /// Two siblings share this name, or names that differ only in letter
    /// case: rekordbox would replace one with the other on import (§5.2).
    RepeatedName { path: Vec<String> },
    /// Folders nest deeper than any real tree.
    TooDeep { path: Vec<String> },
    /// No `TrackID` is left above the highest one in use.
    TrackIdsExhausted,
    /// The bytes didn't read back as what was meant. A bug; nothing is
    /// written.
    ReadBack(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::RepeatedTrack(id) => write!(f, "Library track {} is given twice", id.0),
            BuildError::EmptyName { path } => write!(f, "a nameless node under {path:?}"),
            BuildError::UncarriableName { path } => {
                write!(f, "{path:?} holds a character XML can't carry")
            }
            BuildError::RepeatedName { path } => write!(f, "{path:?} is there twice"),
            BuildError::TooDeep { path } => write!(f, "{path:?} is nested too deep"),
            BuildError::TrackIdsExhausted => f.write_str("no TrackID is left"),
            BuildError::ReadBack(what) => write!(f, "the file doesn't read back: {what}"),
        }
    }
}

impl std::error::Error for BuildError {}

/// A track that will be written, before `TrackID`s are assigned.
struct Prepared {
    /// rekordbox's `TrackID`, for a track it has.
    track_id: Option<u64>,
    /// The attributes to write, in order; a new track's has no `TrackID`
    /// yet.
    attributes: Vec<(String, String)>,
    /// The `Location`'s match key, for spotting two tracks at one path.
    location_key: String,
}

/// Checks one track's values and puts them in their written form.
fn prepare(in_rekordbox: bool, given: &[(String, String)]) -> Result<Prepared, Reason> {
    let mut names = HashSet::with_capacity(given.len());
    let mut attributes = Vec::with_capacity(given.len() + 1);
    let mut track_id = None;
    let mut location_key = None;
    for (name, value) in given {
        let attribute = || name.clone();
        if !xml::is_name(name) {
            return Err(Reason::BadAttributeName {
                attribute: attribute(),
            });
        }
        if !names.insert(name.as_str()) {
            return Err(Reason::RepeatedAttribute {
                attribute: attribute(),
            });
        }
        // Rule 3: never rekordbox's analysis back to it, never the app's.
        if ANALYSIS_ATTRIBUTES.contains(&name.as_str()) {
            continue;
        }
        let value = match name.as_str() {
            "Location" => {
                let written = if in_rekordbox {
                    location::from_rekordbox(value)
                } else {
                    location::from_windows_path(value)
                }
                .map_err(Reason::Location)?;
                let read = rekordbox::location::decode(&written)
                    .map_err(|_| Reason::Location(LocationProblem::DoesNotReadBack))?;
                location_key = Some(read.match_key());
                written
            }
            "TrackID" => {
                if !in_rekordbox {
                    return Err(Reason::TrackIdOnNewTrack);
                }
                let id = digits::<u64>(value)
                    .filter(|id| id.to_string() == *value)
                    .ok_or(Reason::BadTrackId)?;
                track_id = Some(id);
                value.clone()
            }
            _ => value.clone(),
        };
        if xml::uncarriable(&value).is_some() {
            return Err(Reason::UncarriableCharacter {
                attribute: attribute(),
            });
        }
        attributes.push((name.clone(), value));
    }
    let location_key = location_key.ok_or(Reason::NoLocation)?;
    if in_rekordbox && track_id.is_none() {
        return Err(Reason::BadTrackId);
    }
    Ok(Prepared {
        track_id,
        attributes,
        location_key,
    })
}

/// What two sibling names are compared by: NFC, letter case ignored.
/// Whether rekordbox itself treats names that differ only in case as one
/// playlist is unverified (§5.2), so they're refused as the same.
fn sibling_key(name: &str) -> String {
    name.nfc().flat_map(char::to_lowercase).collect()
}

/// Checks a tree's names and collects every track its entries name.
fn check_nodes(
    nodes: &[Node],
    path: &mut Vec<String>,
    named: &mut HashSet<LibraryTrackId>,
) -> Result<(), BuildError> {
    let mut siblings = HashSet::with_capacity(nodes.len());
    for node in nodes {
        let (Node::Folder { name, .. } | Node::Playlist { name, .. }) = node;
        if name.is_empty() {
            return Err(BuildError::EmptyName { path: path.clone() });
        }
        path.push(name.clone());
        if xml::uncarriable(name).is_some() {
            return Err(BuildError::UncarriableName { path: path.clone() });
        }
        if !siblings.insert(sibling_key(name)) {
            return Err(BuildError::RepeatedName { path: path.clone() });
        }
        match node {
            Node::Folder { children, .. } => {
                if path.len() > MAX_DEPTH {
                    return Err(BuildError::TooDeep { path: path.clone() });
                }
                check_nodes(children, path, named)?;
            }
            Node::Playlist { entries, .. } => named.extend(entries.iter().copied()),
        }
        path.pop();
    }
    Ok(())
}

/// What turning the trees into elements needs and collects.
struct Entries<'a> {
    /// The `TrackID` of every track written.
    written: &'a HashMap<LibraryTrackId, u64>,
    /// The entries dropped, by the track they name, in first-seen order.
    dropped: Vec<(LibraryTrackId, Vec<DroppedEntry>)>,
    emptied: Vec<Vec<String>>,
}

impl Entries<'_> {
    fn elements(&mut self, nodes: &[Node], path: &mut Vec<String>) -> Vec<xml::NodeElement> {
        let mut out = Vec::with_capacity(nodes.len());
        for node in nodes {
            let (Node::Folder { name, .. } | Node::Playlist { name, .. }) = node;
            path.push(name.clone());
            out.push(match node {
                Node::Folder { children, .. } => xml::NodeElement::Folder {
                    name: name.clone(),
                    children: self.elements(children, path),
                },
                Node::Playlist { entries, .. } => {
                    let mut keys = Vec::with_capacity(entries.len());
                    for (position, track) in entries.iter().enumerate() {
                        match self.written.get(track) {
                            Some(&id) => keys.push(id),
                            None => self.drop_entry(*track, path, position),
                        }
                    }
                    if keys.is_empty() && !entries.is_empty() {
                        self.emptied.push(path.clone());
                    }
                    xml::NodeElement::Playlist {
                        name: name.clone(),
                        keys,
                    }
                }
            });
            path.pop();
        }
        out
    }

    fn drop_entry(&mut self, track: LibraryTrackId, path: &[String], position: usize) {
        let entry = DroppedEntry {
            path: path.to_vec(),
            position,
        };
        match self.dropped.iter_mut().find(|(id, _)| *id == track) {
            Some((_, entries)) => entries.push(entry),
            None => self.dropped.push((track, vec![entry])),
        }
    }
}

/// Builds the file for one send. See the module's notes for the rules.
pub fn build(input: &SendInput) -> Result<Outgoing, BuildError> {
    let mut given = HashSet::with_capacity(input.tracks.len());
    for track in &input.tracks {
        if !given.insert(track.library_track) {
            return Err(BuildError::RepeatedTrack(track.library_track));
        }
    }
    let mut named = HashSet::new();
    for (top, nodes) in [
        (CRATES_FOLDER, &input.crates),
        (PLAYLISTS_FOLDER, &input.playlists),
    ] {
        check_nodes(nodes, &mut vec![top.to_owned()], &mut named)?;
    }

    // Rule 4: every track an entry names, plus every new track.
    let mut sent: Vec<SentTrack> = Vec::new();
    let mut new_tracks: Vec<usize> = Vec::new();
    let mut reasons: Vec<(LibraryTrackId, Reason)> = Vec::new();
    let mut not_needed = Vec::new();
    let mut locations = HashSet::new();
    let mut track_ids = HashSet::new();
    for track in &input.tracks {
        let id = track.library_track;
        let (in_rekordbox, attributes, other_file) = match &track.values {
            Values::CannotSend(why) => {
                reasons.push((id, Reason::NoValues(why.clone())));
                continue;
            }
            Values::Ready {
                in_rekordbox,
                attributes,
                rekordbox_holds_other_file,
            } => (*in_rekordbox, attributes, rekordbox_holds_other_file),
        };
        if in_rekordbox && !named.contains(&id) {
            not_needed.push(id);
            continue;
        }
        let prepared = prepare(in_rekordbox, attributes).and_then(|p| {
            if locations.contains(&p.location_key) {
                return Err(Reason::DuplicateLocation);
            }
            if p.track_id.is_some_and(|id| track_ids.contains(&id)) {
                return Err(Reason::DuplicateTrackId);
            }
            Ok(p)
        });
        match prepared {
            Err(reason) => reasons.push((id, reason)),
            Ok(p) => {
                locations.insert(p.location_key);
                track_ids.extend(p.track_id);
                if p.track_id.is_none() {
                    new_tracks.push(sent.len());
                }
                sent.push(SentTrack {
                    library_track: id,
                    in_rekordbox,
                    track_id: p.track_id.unwrap_or(0),
                    attributes: p.attributes,
                    rekordbox_holds_other_file: other_file.clone(),
                });
            }
        }
    }

    // New tracks are numbered above every TrackID rekordbox has.
    let mut highest = track_ids
        .iter()
        .copied()
        .fold(input.highest_rekordbox_track_id, u64::max);
    for i in new_tracks {
        highest = highest
            .checked_add(1)
            .ok_or(BuildError::TrackIdsExhausted)?;
        sent[i].track_id = highest;
        sent[i]
            .attributes
            .insert(0, ("TrackID".to_owned(), highest.to_string()));
    }

    let written: HashMap<LibraryTrackId, u64> =
        sent.iter().map(|t| (t.library_track, t.track_id)).collect();
    let mut entries = Entries {
        written: &written,
        dropped: Vec::new(),
        emptied: Vec::new(),
    };
    // Rule 6: exactly these two below ROOT, always both.
    let top = [
        (CRATES_FOLDER, &input.crates),
        (PLAYLISTS_FOLDER, &input.playlists),
    ]
    .map(|(name, nodes)| xml::NodeElement::Folder {
        name: name.to_owned(),
        children: entries.elements(nodes, &mut vec![name.to_owned()]),
    });
    let Entries {
        mut dropped,
        emptied,
        ..
    } = entries;

    let mut left_out: Vec<LeftOut> = reasons
        .into_iter()
        .map(|(library_track, reason)| LeftOut {
            library_track,
            reason,
            entries: dropped
                .iter()
                .position(|(id, _)| *id == library_track)
                .map(|at| dropped.remove(at).1)
                .unwrap_or_default(),
        })
        .collect();
    // What's left names tracks the input never held.
    left_out.extend(dropped.into_iter().map(|(library_track, entries)| LeftOut {
        library_track,
        reason: Reason::NotGiven,
        entries,
    }));

    let elements: Vec<xml::TrackElement<'_>> = sent
        .iter()
        .map(|t| xml::TrackElement {
            attributes: &t.attributes,
        })
        .collect();
    let text = xml::render(&elements, &top);
    read_back(&text, &sent, &top).map_err(BuildError::ReadBack)?;
    Ok(Outgoing {
        xml: text.into_bytes(),
        sent,
        left_out,
        emptied,
        not_needed,
    })
}

/// Reads the file's text with the app's reader and checks it holds
/// exactly the tracks, attributes and playlists meant.
fn read_back(text: &str, sent: &[SentTrack], top: &[xml::NodeElement]) -> Result<(), String> {
    let read = RekordboxXml::parse(text.as_bytes()).map_err(|e| e.to_string())?;
    if read.declared_entries != Some(sent.len() as u64) || !read.is_complete() {
        return Err("COLLECTION's count is off".into());
    }
    // The reader sets aside rekordbox's own demo tracks; they're still in
    // the file.
    if read.tracks.len() + read.skipped.len() != sent.len() {
        return Err(format!(
            "{} tracks read, {} written",
            read.tracks.len() + read.skipped.len(),
            sent.len()
        ));
    }
    let by_id: HashMap<u64, &rekordbox::Track> = read
        .tracks
        .iter()
        .filter_map(|t| t.track_id.map(|id| (id, t)))
        .collect();
    for track in sent {
        match by_id.get(&track.track_id) {
            Some(as_read) => {
                let same = as_read.attrs.len() == track.attributes.len()
                    && as_read
                        .attrs
                        .iter()
                        .zip(&track.attributes)
                        .all(|((n, v), (name, value))| n == name && v == value);
                if !same || !as_read.tempos.is_empty() || !as_read.cues.is_empty() {
                    return Err(format!("TrackID {} reads back changed", track.track_id));
                }
            }
            None => {
                if !read
                    .skipped
                    .iter()
                    .any(|s| s.track_id == Some(track.track_id))
                {
                    return Err(format!("TrackID {} is missing", track.track_id));
                }
            }
        }
    }
    if !same_nodes(&read.playlists.children, top) {
        return Err("PLAYLISTS reads back changed".into());
    }
    Ok(())
}

fn same_nodes(read: &[rekordbox::Node], written: &[xml::NodeElement]) -> bool {
    read.len() == written.len()
        && read.iter().zip(written).all(|pair| match pair {
            (rekordbox::Node::Folder(folder), xml::NodeElement::Folder { name, children }) => {
                folder.name == *name && same_nodes(&folder.children, children)
            }
            (rekordbox::Node::Playlist(playlist), xml::NodeElement::Playlist { name, keys }) => {
                playlist.name == *name
                    && playlist.key_type == rekordbox::KeyType::TrackId
                    && playlist.entries.len() == keys.len()
                    && playlist
                        .entries
                        .iter()
                        .zip(keys)
                        .all(|(entry, key)| entry.key == key.to_string())
            }
            _ => false,
        })
}

/// Writes a built send to `dest` through the write guard: to a new file
/// beside it first, renamed into place once complete, so a failed write
/// never leaves a half file ([`WriteGuard::write_then_rename`]). The guard
/// refuses any destination outside the app's own folders, so this can't
/// write into a music folder or anything rekordbox owns.
pub fn write_file(guard: &WriteGuard, dest: &Path, send: &Outgoing) -> Result<(), GuardError> {
    guard.write_then_rename(dest, send.xml())
}

#[cfg(test)]
mod gather_tests;
#[cfg(test)]
mod tests;
