//! Reading rekordbox's XML export (ROADMAP 1.2, §5.3).
//!
//! The parser reads a file and returns what's in it, and writes nothing
//! anywhere. [`store`] keeps what it read as the `rekordbox_track` snapshot
//! (1aB-10), and [`source`] is where the export comes from and the job that
//! reads it (1aB-11).
//!
//! - **Streaming.** The file is read element by element (quick-xml), never
//!   held whole in memory. A generated 50,000-track export (56 MiB) parses
//!   in about 3 s, close to a bare quick-xml read of the same file, and the
//!   result holds about 130 MiB (`tests/rekordbox_xml_scale.rs`, release).
//! - **Lenient per value, strict per file.** A value that doesn't parse
//!   clears that one typed field and records a [`Warning`]; the track is
//!   still read, and no value fails the file. But a file that isn't
//!   well-formed XML is refused ([`XmlError`]), a malformed attribute
//!   included: a cut-off export (rekordbox still writing it, a full disk)
//!   would otherwise read as a collection missing its last tracks, and
//!   those would look deleted. See also [`RekordboxXml::is_complete`].
//! - **Everything the writer needs.** Every attribute of every track,
//!   cue and grid entry is kept as read ([`attrs::Attrs`]), since the XML
//!   writer sends each one back from a fresh read (ROADMAP 1.9 rule 1).
//! - **Skips are counted, not silent** ([`skip`]).
//!
//! `Location` is decoded by hand ([`location`]). Tracks are matched by it,
//! never by `TrackID`, which rekordbox reassigns on import.

pub mod attrs;
pub mod collection;
pub mod location;
pub mod my_tags;
pub mod playlists;
mod reader;
pub mod skip;
pub mod source;
pub mod store;

use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

pub use attrs::{Attrs, Date, Rgb};
pub use collection::{Cue, CueKind, CueSlot, Meter, Tempo, Track};
pub use location::{FilePath, Location, LocationError, PathStyle, StreamingId};
pub use playlists::{Entry, EntryTarget, Folder, KeyType, Node, Playlist};
pub use skip::{ReasonCounts, SkipCounts, SkipReason};

/// Everything read from one rekordbox XML export.
#[derive(Debug, Clone, PartialEq)]
pub struct RekordboxXml {
    /// `DJ_PLAYLISTS Version`.
    pub version: Option<String>,
    /// Who wrote the file: rekordbox and its version, usually.
    pub product: Option<Product>,
    /// COLLECTION's `Entries`, as declared.
    pub declared_entries: Option<u64>,
    /// COLLECTION's tracks, in file order, without the skipped ones.
    /// Streaming tracks are included ([`Track::is_streaming`]).
    pub tracks: Vec<Track>,
    /// The tracks left out, so playlist entries pointing at them can say
    /// why.
    pub skipped: Vec<SkippedTrack>,
    /// rekordbox's `ROOT` folder, without the CUE Analysis Playlist.
    pub playlists: Folder,
    /// What was skipped, by reason.
    pub skips: SkipCounts,
    /// Playlist entries that point at no COLLECTION track.
    pub entries_without_track: usize,
    /// Every value that didn't parse, and every other oddity.
    pub warnings: Vec<Warning>,
}

impl RekordboxXml {
    /// Reads an export from disk.
    pub fn read_file(path: &Path) -> Result<RekordboxXml, XmlError> {
        let file = File::open(path).map_err(XmlError::Io)?;
        RekordboxXml::parse(BufReader::with_capacity(1 << 16, file))
    }

    /// Reads an export from any buffered source.
    pub fn parse(source: impl BufRead) -> Result<RekordboxXml, XmlError> {
        reader::parse(source)
    }

    /// Whether COLLECTION held as many `TRACK`s as its `Entries` says (or
    /// it says nothing). When this is `false` (a [`Problem::CountMismatch`]
    /// on [`Element::Collection`]), the export may be missing tracks, so a
    /// track absent from it must not be treated as removed from rekordbox.
    pub fn is_complete(&self) -> bool {
        !self.warnings.iter().any(|w| {
            matches!(
                w.problem,
                Problem::CountMismatch {
                    element: Element::Collection,
                    ..
                }
            )
        })
    }
}

/// `PRODUCT`: the program that wrote the export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    pub name: String,
    pub version: String,
    pub company: String,
}

/// A COLLECTION track that isn't one of the user's files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedTrack {
    pub track_id: Option<u64>,
    /// `Location` as written.
    pub location: String,
    pub reason: SkipReason,
}

/// Something that didn't read cleanly. The read went on without it.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    /// Where in the file (in bytes) the element holding it ends.
    pub offset: u64,
    /// The track it belongs to, when there is one.
    pub track_id: Option<u64>,
    pub problem: Problem,
}

/// Which element a [`Warning`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    Product,
    Collection,
    Track,
    Tempo,
    PositionMark,
    Node,
    /// A playlist's `TRACK` entry.
    Entry,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// A value that doesn't parse as its type. The typed field is empty;
    /// the raw value is still in the element's attributes.
    BadValue {
        element: Element,
        attribute: String,
        value: String,
    },
    /// A value with a bad `&…;` reference (`&nbsp;`, a bare `&`), kept
    /// with the reference as written.
    BadEntity { element: Element, attribute: String },
    /// A `Location` that doesn't decode. The track is kept; it can't be
    /// matched to a file.
    BadLocation { value: String, error: LocationError },
    /// A `Tonality` in a notation `tags::key` doesn't read.
    UnknownKey { value: String },
    /// A track without a `TrackID`. Playlists can't refer to it by id.
    MissingTrackId,
    /// A second track with the same `TrackID`. Playlist entries resolve to
    /// the first.
    DuplicateTrackId { track_id: u64 },
    /// A second track at the same `Location`. Playlist entries resolve to
    /// the first.
    DuplicateLocation { location: String },
    /// A cue with some but not all of `Red`, `Green`, `Blue`.
    PartialColor,
    /// A count attribute (`Entries`, `Count`) that doesn't match what the
    /// element holds.
    CountMismatch {
        element: Element,
        declared: u64,
        found: u64,
    },
    /// An element this parser doesn't expect here. Skipped with everything
    /// inside it.
    UnexpectedElement { name: String, parent: String },
    /// A `NODE` whose `Type` isn't `0` (folder) or `1` (playlist); read as
    /// a playlist if it holds entries, otherwise as a folder.
    UnknownNodeType { name: String, value: String },
    /// PLAYLISTS didn't hold exactly one top `NODE`; its nodes were put
    /// under one `ROOT`.
    UnusualPlaylistRoot { nodes: usize },
}

/// Why an export couldn't be read at all.
#[derive(Debug)]
pub enum XmlError {
    Io(io::Error),
    /// Not well-formed XML, at this byte. Bytes that aren't UTF-8 count,
    /// as do an attribute that isn't well-formed (`Rating=255` unquoted, a
    /// repeated name) and playlist folders nested over 256 deep: rekordbox
    /// writes none of these, so the file is damaged.
    Malformed {
        offset: u64,
        detail: String,
    },
    /// The file ends inside an element: cut off, or still being written.
    Truncated,
    /// An encoding other than UTF-8.
    UnsupportedEncoding(String),
    /// The top element isn't `DJ_PLAYLISTS`.
    NotRekordboxXml {
        root: String,
    },
    /// No `COLLECTION` element: not a collection export.
    NoCollection,
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XmlError::Io(e) => write!(f, "can't read the file: {e}"),
            XmlError::Malformed { offset, detail } => {
                write!(f, "not well-formed XML at byte {offset}: {detail}")
            }
            XmlError::Truncated => f.write_str("the file ends early"),
            XmlError::UnsupportedEncoding(e) => write!(f, "unsupported encoding {e}"),
            XmlError::NotRekordboxXml { root } => {
                write!(f, "not a rekordbox export (top element {root})")
            }
            XmlError::NoCollection => f.write_str("no COLLECTION element"),
        }
    }
}

impl std::error::Error for XmlError {}

#[cfg(test)]
mod tests;
