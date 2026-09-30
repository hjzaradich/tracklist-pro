//! The raw tag frames of a file, for `file.raw_tags` (ROADMAP §2).
//!
//! The canonical fields in [`super::TagFields`] go through lofty's generic
//! `Tag`, which drops frames it has no field for: Serato's `GEOB` cue data,
//! `PRIV` frames, `TXXX` frames with unknown descriptions. This module reads
//! the format's own tag structures instead, so every frame the file holds is
//! listed. Text is kept as text; binary payloads are recorded by length only,
//! so artwork and cue blobs don't bloat the database.

use lofty::ape::ApeTag;
use lofty::id3::v1::Id3v1Tag;
use lofty::id3::v2::{Frame, Id3v2Tag};
use lofty::iff::aiff::AiffTextChunks;
use lofty::iff::wav::RiffInfoList;
use lofty::mp4::{AtomData, AtomIdent, Ilst};
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::OggPictureStorage;
use lofty::picture::{Picture, PictureInformation};
use serde::Serialize;

/// One tag block in a file, e.g. the ID3v2 tag at the start of an MP3.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RawTag {
    pub tag_type: RawTagType,
    pub items: Vec<RawItem>,
}

impl RawTag {
    /// The first item with this key.
    pub fn get(&self, key: &str) -> Option<&RawValue> {
        self.items.iter().find(|i| i.key == key).map(|i| &i.value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RawTagType {
    Id3v2,
    Id3v1,
    Ape,
    VorbisComments,
    Mp4Ilst,
    RiffInfo,
    AiffText,
}

/// One frame, field or atom, under the key the format itself uses.
///
/// Frames that carry a description or owner put it in the key, as mutagen
/// does: `TXXX:SERATO_PLAYCOUNT`, `COMM::eng`, `PRIV:www.example.com`,
/// `----:com.apple.iTunes:LABEL`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RawItem {
    pub key: String,
    pub value: RawValue,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RawValue {
    /// Text, cut to [`MAX_TEXT_BYTES`]. When it was cut, `full_len` is the
    /// original length in bytes.
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        full_len: Option<usize>,
    },
    /// A binary payload (artwork, cue data), recorded by size only.
    Binary { len: usize },
    /// A frame lofty parses into a structure we don't record (chapters,
    /// volume adjustment). Listed so its presence isn't lost.
    Unrecorded,
}

impl RawValue {
    pub fn text(&self) -> Option<&str> {
        match self {
            RawValue::Text { text, .. } => Some(text),
            _ => None,
        }
    }
}

/// Longest text value kept in `raw_tags`. Lyrics and pasted notes can run to
/// many kilobytes, and 100k files' worth would bloat the database.
pub const MAX_TEXT_BYTES: usize = 4096;

fn text(s: impl Into<String>) -> RawValue {
    let mut text = s.into();
    let full_len = (text.len() > MAX_TEXT_BYTES).then_some(text.len());
    if full_len.is_some() {
        let mut cut = MAX_TEXT_BYTES;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    RawValue::Text { text, full_len }
}

fn binary(data: &[u8]) -> RawValue {
    RawValue::Binary { len: data.len() }
}

fn item(key: impl Into<String>, value: RawValue) -> RawItem {
    RawItem {
        key: key.into(),
        value,
    }
}

pub(super) fn id3v2(tag: &Id3v2Tag) -> RawTag {
    let mut items = Vec::new();
    for frame in tag {
        let id = frame.id_str();
        match frame {
            Frame::Text(f) => items.push(item(id, text(f.value.as_ref()))),
            Frame::UserText(f) => items.push(item(
                format!("{id}:{}", f.description),
                text(f.content.as_ref()),
            )),
            Frame::Url(f) => items.push(item(id, text(f.url()))),
            Frame::UserUrl(f) => items.push(item(
                format!("{id}:{}", f.description),
                text(f.content.as_ref()),
            )),
            Frame::Comment(f) => items.push(item(
                format!("{id}:{}:{}", f.description, lang(&f.language)),
                text(f.content.as_ref()),
            )),
            Frame::UnsynchronizedText(f) => items.push(item(
                format!("{id}:{}:{}", f.description, lang(&f.language)),
                text(f.content.as_ref()),
            )),
            Frame::Picture(f) => items.push(picture(id, &f.picture)),
            Frame::Popularimeter(f) => items.push(item(
                format!("{id}:{}", f.email),
                text(format!("rating={} counter={}", f.rating, f.counter)),
            )),
            // TIPL / TMCL: one item per (role, person) pair.
            Frame::KeyValue(f) => {
                for (role, person) in f.key_value_pairs.iter() {
                    items.push(item(format!("{id}:{role}"), text(person.as_ref())));
                }
            }
            Frame::UniqueFileIdentifier(f) => {
                let value = match std::str::from_utf8(&f.identifier) {
                    Ok(s) if !s.chars().any(char::is_control) => text(s),
                    _ => binary(&f.identifier),
                };
                items.push(item(format!("{id}:{}", f.owner), value));
            }
            Frame::Private(f) => {
                items.push(item(format!("{id}:{}", f.owner), binary(&f.private_data)))
            }
            Frame::Timestamp(f) => items.push(item(id, text(f.timestamp.to_string()))),
            // lofty keeps frames it doesn't model, including Serato's GEOB
            // frames, as raw bytes.
            Frame::Binary(f) => items.push(item(id, binary(&f.data))),
            _ => items.push(item(id, RawValue::Unrecorded)),
        }
    }
    RawTag {
        tag_type: RawTagType::Id3v2,
        items,
    }
}

fn lang(code: &[u8; 3]) -> String {
    String::from_utf8_lossy(code).into_owned()
}

fn picture(key: &str, picture: &Picture) -> RawItem {
    let key = match picture.description() {
        Some(desc) => format!("{key}:{desc}"),
        None => key.to_string(),
    };
    item(key, binary(picture.data()))
}

pub(super) fn id3v1(tag: &Id3v1Tag) -> RawTag {
    let mut items = Vec::new();
    let mut push = |key: &str, value: Option<String>| {
        if let Some(v) = value {
            items.push(item(key, text(v)));
        }
    };
    push("title", tag.title.clone());
    push("artist", tag.artist.clone());
    push("album", tag.album.clone());
    push("year", tag.year.map(|y| y.to_string()));
    push("comment", tag.comment.clone());
    push("track", tag.track_number.map(|n| n.to_string()));
    push("genre", tag.genre.map(|g| g.to_string()));
    RawTag {
        tag_type: RawTagType::Id3v1,
        items,
    }
}

pub(super) fn ape(tag: &ApeTag) -> RawTag {
    let items = tag
        .into_iter()
        .map(|i| {
            let value = match i.value() {
                lofty::tag::ItemValue::Text(s) | lofty::tag::ItemValue::Locator(s) => text(s),
                lofty::tag::ItemValue::Binary(b) => binary(b),
            };
            item(i.key(), value)
        })
        .collect();
    RawTag {
        tag_type: RawTagType::Ape,
        items,
    }
}

/// Vorbis comments (FLAC, Ogg Vorbis, Opus, Speex). FLAC keeps its pictures
/// outside the comment block, so they're passed in separately.
pub(super) fn vorbis(
    tag: Option<&VorbisComments>,
    flac_pictures: &[(Picture, PictureInformation)],
) -> Option<RawTag> {
    let mut items: Vec<RawItem> = Vec::new();
    if let Some(tag) = tag {
        items.extend(tag.items().map(|(k, v)| item(k, text(v))));
        items.extend(
            tag.pictures()
                .iter()
                .map(|(p, _)| picture("METADATA_BLOCK_PICTURE", p)),
        );
    }
    items.extend(
        flac_pictures
            .iter()
            .map(|(p, _)| picture("METADATA_BLOCK_PICTURE", p)),
    );
    (tag.is_some() || !items.is_empty()).then_some(RawTag {
        tag_type: RawTagType::VorbisComments,
        items,
    })
}

pub(super) fn ilst(tag: &Ilst) -> RawTag {
    let mut items = Vec::new();
    for atom in tag {
        let key = match atom.ident() {
            // Fourccs are Latin-1: `©nam` is 0xA9 'n' 'a' 'm'.
            AtomIdent::Fourcc(code) => code.iter().map(|&b| b as char).collect(),
            AtomIdent::Freeform { mean, name } => format!("----:{mean}:{name}"),
        };
        for data in atom.data() {
            let value = match data {
                AtomData::UTF8(s) | AtomData::UTF16(s) => text(s.as_str()),
                AtomData::Picture(p) => binary(p.data()),
                AtomData::SignedInteger(n) => text(n.to_string()),
                AtomData::UnsignedInteger(n) => text(n.to_string()),
                AtomData::Bool(b) => text(b.to_string()),
                AtomData::Unknown { data, .. } => binary(data),
            };
            items.push(item(key.clone(), value));
        }
    }
    RawTag {
        tag_type: RawTagType::Mp4Ilst,
        items,
    }
}

pub(super) fn riff_info(tag: &RiffInfoList) -> RawTag {
    RawTag {
        tag_type: RawTagType::RiffInfo,
        items: tag.into_iter().map(|(k, v)| item(k, text(v))).collect(),
    }
}

pub(super) fn aiff_text(tag: &AiffTextChunks) -> RawTag {
    let mut items = Vec::new();
    let mut push = |key: &str, value: Option<&String>| {
        if let Some(v) = value {
            items.push(item(key, text(v.as_str())));
        }
    };
    push("NAME", tag.name.as_ref());
    push("AUTH", tag.author.as_ref());
    push("(c) ", tag.copyright.as_ref());
    for annotation in tag.annotations.iter().flatten() {
        items.push(item("ANNO", text(annotation.as_str())));
    }
    for comment in tag.comments.iter().flatten() {
        items.push(item("COMT", text(comment.text.as_str())));
    }
    RawTag {
        tag_type: RawTagType::AiffText,
        items,
    }
}
