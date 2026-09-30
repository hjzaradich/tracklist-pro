//! The streaming read: one pass over the file's elements, building tracks
//! and the playlist tree as each element closes.

use std::collections::HashSet;
use std::io::{self, BufRead};

use quick_xml::events::{BytesDecl, BytesStart, Event};
use quick_xml::Reader;

use super::attrs::{self, digits, Attrs};
use super::collection::{self, Track};
use super::location::Location;
use super::playlists::{self, Entry, EntryTarget, Folder, KeyType, Node, Playlist};
use super::skip::{self, SkipCounts};
use super::{Element, Problem, Product, RekordboxXml, SkippedTrack, Warning, XmlError};

pub(super) fn parse(mut source: impl BufRead) -> Result<RekordboxXml, XmlError> {
    // A UTF-8 BOM is dropped by quick-xml; a UTF-16 one means an encoding
    // rekordbox doesn't write, which would otherwise read as garbage.
    let head = source.fill_buf().map_err(XmlError::Io)?;
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return Err(XmlError::UnsupportedEncoding("UTF-16".into()));
    }

    let mut reader = Reader::from_reader(source);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::with_capacity(4096);
    let mut state = State::default();
    loop {
        let event = match reader.read_event_into(&mut buf) {
            Ok(event) => event,
            Err(quick_xml::Error::Io(e)) => {
                return Err(XmlError::Io(io::Error::new(e.kind(), e.to_string())))
            }
            Err(e) => {
                return Err(XmlError::Malformed {
                    offset: reader.error_position(),
                    detail: e.to_string(),
                })
            }
        };
        let offset = reader.buffer_position();
        match event {
            Event::Decl(decl) => check_encoding(&decl)?,
            Event::Start(e) => state.open(&e, offset, false)?,
            Event::Empty(e) => state.open(&e, offset, true)?,
            Event::End(_) => state.close(offset),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    state.finish()
}

fn check_encoding(decl: &BytesDecl) -> Result<(), XmlError> {
    match decl.encoding() {
        None => Ok(()),
        Some(Ok(name)) => {
            let name = name.into_owned();
            if name.eq_ignore_ascii_case("utf-8") || name.eq_ignore_ascii_case("utf8") {
                Ok(())
            } else {
                Err(XmlError::UnsupportedEncoding(name))
            }
        }
        Some(Err(e)) => Err(XmlError::Malformed {
            offset: 0,
            detail: e.to_string(),
        }),
    }
}

/// How deep playlist folders may nest, ROOT included. rekordbox's own
/// trees are a handful deep; a file nesting thousands deep is damaged or
/// hostile, and the tree's recursive code (walking, dropping, comparing)
/// would overflow the stack on it, which aborts the process uncatchably.
pub(super) const MAX_NODE_DEPTH: usize = 256;

/// Which element the reader is inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    Root,
    Product,
    Collection,
    Track,
    TrackChild,
    Playlists,
    Node,
    Entry,
    /// Unexpected, skipped with everything inside it.
    Other,
}

/// A `NODE` being read.
#[derive(Default)]
struct NodeBuilder {
    offset: u64,
    name: String,
    node_type: Option<String>,
    key_type: Option<String>,
    /// `Count` (folders) or `Entries` (playlists).
    count: Option<String>,
    children: Vec<Node>,
    keys: Vec<String>,
}

#[derive(Default)]
struct State {
    stack: Vec<Frame>,
    root_seen: bool,
    version: Option<String>,
    product: Option<Product>,
    collection_seen: bool,
    declared_entries: Option<u64>,
    /// `TRACK` elements in COLLECTION, skipped ones included.
    collection_count: u64,
    track: Option<Track>,
    tracks: Vec<Track>,
    skipped: Vec<SkippedTrack>,
    ids: HashSet<u64>,
    locations: HashSet<String>,
    /// The `NODE`s open right now, with PLAYLISTS itself at the bottom.
    nodes: Vec<NodeBuilder>,
    playlists: Option<Folder>,
    skips: SkipCounts,
    warnings: Vec<Warning>,
}

impl State {
    fn open(&mut self, e: &BytesStart, offset: u64, empty: bool) -> Result<(), XmlError> {
        let name = e.name();
        let name = name.as_ref();
        let parent = self.stack.last().copied();
        let frame = match parent {
            None => {
                if self.root_seen {
                    return Err(XmlError::Malformed {
                        offset,
                        detail: "a second top element".into(),
                    });
                }
                self.root_seen = true;
                if name != "DJ_PLAYLISTS" {
                    return Err(XmlError::NotRekordboxXml {
                        root: name.to_owned(),
                    });
                }
                let attrs = self.attrs(e, Element::Product, offset, None)?;
                self.version = attrs.get("Version").map(str::to_owned);
                Frame::Root
            }
            Some(Frame::Root) => match name {
                "PRODUCT" => {
                    let attrs = self.attrs(e, Element::Product, offset, None)?;
                    let get = |n: &str| attrs.get(n).unwrap_or("").to_owned();
                    self.product = Some(Product {
                        name: get("Name"),
                        version: get("Version"),
                        company: get("Company"),
                    });
                    Frame::Product
                }
                "COLLECTION" => {
                    let attrs = self.attrs(e, Element::Collection, offset, None)?;
                    self.collection_seen = true;
                    self.declared_entries =
                        self.count_attr(&attrs, "Entries", Element::Collection, offset);
                    Frame::Collection
                }
                "PLAYLISTS" => {
                    self.nodes.push(NodeBuilder::default());
                    Frame::Playlists
                }
                _ => self.unexpected(name, "DJ_PLAYLISTS", offset),
            },
            Some(Frame::Collection) => match name {
                "TRACK" => {
                    self.collection_count += 1;
                    let first = self.warnings.len();
                    let attrs = self.attrs(e, Element::Track, offset, None)?;
                    let track = collection::track(attrs, offset, &mut self.warnings);
                    for w in &mut self.warnings[first..] {
                        w.track_id = w.track_id.or(track.track_id);
                    }
                    self.track = Some(track);
                    Frame::Track
                }
                _ => self.unexpected(name, "COLLECTION", offset),
            },
            Some(Frame::Track) => match name {
                "TEMPO" | "POSITION_MARK" => {
                    let id = self.track.as_ref().and_then(|t| t.track_id);
                    let element = if name == "TEMPO" {
                        Element::Tempo
                    } else {
                        Element::PositionMark
                    };
                    let attrs = self.attrs(e, element, offset, id)?;
                    if let Some(track) = self.track.as_mut() {
                        if element == Element::Tempo {
                            let tempo = collection::tempo(attrs, offset, id, &mut self.warnings);
                            track.tempos.push(tempo);
                        } else {
                            let cue = collection::cue(attrs, offset, id, &mut self.warnings);
                            track.cues.push(cue);
                        }
                    }
                    Frame::TrackChild
                }
                _ => self.unexpected(name, "TRACK", offset),
            },
            Some(Frame::Playlists) | Some(Frame::Node) => match name {
                "NODE" => {
                    // `nodes` holds PLAYLISTS itself plus every open NODE.
                    if self.nodes.len() > MAX_NODE_DEPTH {
                        return Err(XmlError::Malformed {
                            offset,
                            detail: format!("playlist folders nested over {MAX_NODE_DEPTH} deep"),
                        });
                    }
                    let attrs = self.attrs(e, Element::Node, offset, None)?;
                    let get = |n: &str| attrs.get(n).map(str::to_owned);
                    let count = match get("Type").as_deref() {
                        Some("0") => get("Count"),
                        _ => get("Entries"),
                    };
                    self.nodes.push(NodeBuilder {
                        offset,
                        name: get("Name").unwrap_or_default(),
                        node_type: get("Type"),
                        key_type: get("KeyType"),
                        count,
                        ..NodeBuilder::default()
                    });
                    Frame::Node
                }
                "TRACK" if parent == Some(Frame::Node) => {
                    let attrs = self.attrs(e, Element::Entry, offset, None)?;
                    let key = attrs.get("Key").unwrap_or("").to_owned();
                    if let Some(node) = self.nodes.last_mut() {
                        node.keys.push(key);
                    }
                    Frame::Entry
                }
                _ => {
                    let parent = if parent == Some(Frame::Node) {
                        "NODE"
                    } else {
                        "PLAYLISTS"
                    };
                    self.unexpected(name, parent, offset)
                }
            },
            Some(Frame::TrackChild) => self.unexpected(name, "TRACK child", offset),
            Some(Frame::Product) => self.unexpected(name, "PRODUCT", offset),
            Some(Frame::Entry) => self.unexpected(name, "playlist TRACK", offset),
            Some(Frame::Other) => Frame::Other,
        };
        if empty {
            self.finish_frame(frame, offset);
        } else {
            self.stack.push(frame);
        }
        Ok(())
    }

    fn close(&mut self, offset: u64) {
        if let Some(frame) = self.stack.pop() {
            self.finish_frame(frame, offset);
        }
    }

    fn finish_frame(&mut self, frame: Frame, offset: u64) {
        match frame {
            Frame::Track => {
                if let Some(track) = self.track.take() {
                    self.finish_track(track);
                }
            }
            Frame::Collection => {
                if let Some(declared) = self.declared_entries {
                    if declared != self.collection_count {
                        self.warn(
                            offset,
                            None,
                            Problem::CountMismatch {
                                element: Element::Collection,
                                declared,
                                found: self.collection_count,
                            },
                        );
                    }
                }
            }
            Frame::Node => {
                if let Some(builder) = self.nodes.pop() {
                    let node = self.finish_node(builder);
                    if let Some(parent) = self.nodes.last_mut() {
                        parent.children.push(node);
                    }
                }
            }
            Frame::Playlists => {
                if let Some(top) = self.nodes.pop() {
                    self.playlists = Some(self.finish_root(top.children, offset));
                }
            }
            _ => {}
        }
    }

    fn finish_track(&mut self, track: Track) {
        let id = track.track_id;
        if let Some(id) = id {
            if !self.ids.insert(id) {
                self.warn(
                    track.offset,
                    Some(id),
                    Problem::DuplicateTrackId { track_id: id },
                );
            }
        }
        if let Some(reason) = skip::reason(&track) {
            self.skips.tracks.add(reason);
            if reason != skip::SkipReason::Streaming {
                self.skipped.push(SkippedTrack {
                    track_id: id,
                    location: track.location_raw().to_owned(),
                    reason,
                });
                return;
            }
        }
        if let Ok(location) = &track.location {
            if matches!(location, Location::File(_)) && !self.locations.insert(location.match_key())
            {
                let problem = Problem::DuplicateLocation {
                    location: track.location_raw().to_owned(),
                };
                self.warn(track.offset, id, problem);
            }
        }
        self.tracks.push(track);
    }

    fn finish_node(&mut self, b: NodeBuilder) -> Node {
        let is_folder = match b.node_type.as_deref() {
            Some("0") => true,
            Some("1") => false,
            other => {
                let problem = Problem::UnknownNodeType {
                    name: b.name.clone(),
                    value: other.unwrap_or("").to_owned(),
                };
                self.warn(b.offset, None, problem);
                b.keys.is_empty()
            }
        };
        let found = if is_folder {
            b.children.len()
        } else {
            b.keys.len()
        } as u64;
        if let Some(declared) = b.count.as_deref() {
            let count_name = if is_folder { "Count" } else { "Entries" };
            match digits::<u64>(declared) {
                Some(declared) if declared != found => {
                    self.warn(
                        b.offset,
                        None,
                        Problem::CountMismatch {
                            element: Element::Node,
                            declared,
                            found,
                        },
                    );
                }
                Some(_) => {}
                None => self.warn(
                    b.offset,
                    None,
                    Problem::BadValue {
                        element: Element::Node,
                        attribute: count_name.into(),
                        value: declared.to_owned(),
                    },
                ),
            }
        }
        if is_folder {
            return Node::Folder(Folder {
                name: b.name,
                children: b.children,
            });
        }
        let key_type = match b.key_type.as_deref() {
            None | Some("0") => KeyType::TrackId,
            Some("1") => KeyType::Location,
            Some(other) => {
                self.warn(
                    b.offset,
                    None,
                    Problem::BadValue {
                        element: Element::Node,
                        attribute: "KeyType".into(),
                        value: other.to_owned(),
                    },
                );
                KeyType::TrackId
            }
        };
        let entries = b
            .keys
            .into_iter()
            .map(|key| Entry {
                key,
                target: EntryTarget::NoTrack,
            })
            .collect();
        Node::Playlist(Playlist {
            name: b.name,
            key_type,
            entries,
        })
    }

    /// PLAYLISTS normally holds one folder, `ROOT`.
    fn finish_root(&mut self, mut top: Vec<Node>, offset: u64) -> Folder {
        if top.len() == 1 {
            if let Node::Folder(_) = &top[0] {
                let Some(Node::Folder(root)) = top.pop() else {
                    unreachable!()
                };
                return root;
            }
        }
        if !top.is_empty() {
            self.warn(
                offset,
                None,
                Problem::UnusualPlaylistRoot { nodes: top.len() },
            );
        }
        Folder {
            name: "ROOT".into(),
            children: top,
        }
    }

    fn finish(mut self) -> Result<RekordboxXml, XmlError> {
        if !self.root_seen || !self.stack.is_empty() {
            return Err(XmlError::Truncated);
        }
        if !self.collection_seen {
            return Err(XmlError::NoCollection);
        }
        let mut root = self.playlists.take().unwrap_or_else(|| Folder {
            name: "ROOT".into(),
            children: Vec::new(),
        });
        playlists::drop_cue_analysis(&mut root, &mut self.skips);
        let entries_without_track =
            playlists::resolve(&mut root, &self.tracks, &self.skipped, &mut self.skips);
        self.tracks.shrink_to_fit();
        Ok(RekordboxXml {
            version: self.version,
            product: self.product,
            declared_entries: self.declared_entries,
            tracks: self.tracks,
            skipped: self.skipped,
            playlists: root,
            skips: self.skips,
            entries_without_track,
            warnings: self.warnings,
        })
    }

    /// Reads an element's attributes. An attribute that isn't well-formed
    /// (`Rating=255` unquoted, a name without a value, a repeated name)
    /// refuses the whole file: rekordbox never writes one, so the file is
    /// damaged, and dropping the attribute would make a later send omit it,
    /// which can reset rekordbox's value (ROADMAP §5.2).
    fn attrs(
        &mut self,
        e: &BytesStart,
        element: Element,
        offset: u64,
        track_id: Option<u64>,
    ) -> Result<Attrs, XmlError> {
        let mut out = Attrs::default();
        for attr in e.attributes() {
            let attr = attr.map_err(|err| XmlError::Malformed {
                offset,
                detail: format!("bad attribute in <{}>: {err}", e.name().as_ref()),
            })?;
            let name = attr.key.as_ref();
            let (value, bad) = attrs::unescape(&attr.value);
            if bad {
                let attribute = name.to_owned();
                self.warn(offset, track_id, Problem::BadEntity { element, attribute });
            }
            out.push(name, &value);
        }
        out.shrink();
        Ok(out)
    }

    fn count_attr(
        &mut self,
        attrs: &Attrs,
        name: &str,
        element: Element,
        offset: u64,
    ) -> Option<u64> {
        let value = attrs.get(name)?;
        let parsed = digits::<u64>(value);
        if parsed.is_none() {
            self.warn(
                offset,
                None,
                Problem::BadValue {
                    element,
                    attribute: name.to_owned(),
                    value: value.to_owned(),
                },
            );
        }
        parsed
    }

    fn unexpected(&mut self, name: &str, parent: &str, offset: u64) -> Frame {
        let track_id = self.track.as_ref().and_then(|t| t.track_id);
        self.warn(
            offset,
            track_id,
            Problem::UnexpectedElement {
                name: name.to_owned(),
                parent: parent.to_owned(),
            },
        );
        Frame::Other
    }

    fn warn(&mut self, offset: u64, track_id: Option<u64>, problem: Problem) {
        self.warnings.push(Warning {
            offset,
            track_id,
            problem,
        });
    }
}
