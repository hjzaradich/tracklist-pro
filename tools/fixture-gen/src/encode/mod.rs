//! Writes PCM and tags into each file format.
//!
//! WAV, AIFF, FLAC and M4A (Apple Lossless) are written by hand: the formats
//! are simple, and hand-written output is deterministic and has no
//! dependencies. MP3 goes through LAME. Tag blocks (ID3v2, Vorbis comments,
//! RIFF INFO, MP4 `ilst`, APEv2) are hand-written too.

pub mod aiff;
pub mod ape;
mod bits;
pub mod flac;
pub mod id3;
pub mod m4a;
pub mod mp3;
pub mod wav;

use serde::Serialize;

/// Synthetic tag values. Every field is optional; an empty set writes no tag
/// block at all.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Tags {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl Tags {
    pub fn new(title: &str, artist: &str) -> Tags {
        Tags {
            title: Some(title.into()),
            artist: Some(artist.into()),
            ..Tags::default()
        }
    }

    pub fn album(mut self, album: &str) -> Tags {
        self.album = Some(album.into());
        self
    }

    pub fn genre(mut self, genre: &str) -> Tags {
        self.genre = Some(genre.into());
        self
    }

    pub fn year(mut self, year: &str) -> Tags {
        self.year = Some(year.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        *self == Tags::default()
    }

    /// The fields in a fixed order, as (name, value).
    pub fn fields(&self) -> Vec<(Field, &str)> {
        [
            (Field::Title, &self.title),
            (Field::Artist, &self.artist),
            (Field::Album, &self.album),
            (Field::Genre, &self.genre),
            (Field::Year, &self.year),
            (Field::Track, &self.track),
            (Field::Bpm, &self.bpm),
            (Field::Key, &self.key),
        ]
        .into_iter()
        .filter_map(|(f, v)| v.as_deref().map(|v| (f, v)))
        .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Artist,
    Album,
    Genre,
    Year,
    Track,
    Bpm,
    Key,
}

/// Appends a big-endian u32.
pub(crate) fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Appends a big-endian u16.
pub(crate) fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Appends a little-endian u32.
pub(crate) fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Appends a little-endian u16.
pub(crate) fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
