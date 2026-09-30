//! COLLECTION's tracks: every `TRACK` attribute, its cues
//! (`POSITION_MARK`) and its beat grid (`TEMPO`).
//!
//! Each element keeps all its attributes as read ([`Attrs`]), because the
//! writer sends every attribute back from a fresh read (ROADMAP 1.9 rule
//! 1). On top of that, the known numeric, date, color and key attributes
//! are parsed into typed fields; text attributes are read through methods
//! ([`Track::name`], …) so they're stored once. A value that doesn't parse
//! clears that typed field and records a [`Warning`]; it never fails the
//! track or the file.

use crate::tags::key::{self, MusicalKey};

use super::attrs::{decimal, digits, Attrs, Date, Rgb};
use super::location::{self, Location, LocationError};
use super::{Element, Problem, Warning};

/// One `TRACK` of COLLECTION.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Where the element ends in the file, in bytes.
    pub offset: u64,
    /// Every attribute as read, in file order.
    pub attrs: Attrs,
    /// `TrackID`. Valid within this export only: rekordbox reassigns it on
    /// import (ROADMAP §5.2), so match tracks by [`Track::location`].
    pub track_id: Option<u64>,
    /// `Location`, decoded by hand (see [`location`]).
    pub location: Result<Location, LocationError>,
    /// `Size` in bytes. Can be stale (§5.3): read the real size from the file.
    pub size: Option<u64>,
    /// `TotalTime` in whole seconds.
    pub total_time: Option<u32>,
    /// `DiscNumber`; rekordbox's `0` ("none") reads as `None`.
    pub disc_number: Option<u32>,
    /// `TrackNumber`; `0` reads as `None`.
    pub track_number: Option<u32>,
    /// `Year`; `0` reads as `None`.
    pub year: Option<u16>,
    /// `AverageBpm`; `0.00` (not analyzed) reads as `None`.
    pub average_bpm: Option<f64>,
    pub date_modified: Option<Date>,
    pub date_added: Option<Date>,
    /// `BitRate` in kbit/s. Can be stale (§5.3).
    pub bit_rate: Option<u32>,
    /// `SampleRate` in Hz.
    pub sample_rate: Option<u32>,
    /// `PlayCount`; `0` is a real count and stays `Some(0)`.
    pub play_count: Option<u32>,
    pub last_played: Option<Date>,
    /// `Rating`, 0–255 as written (rekordbox uses 0, 51, … 255 for 0–5
    /// stars; see [`Track::stars`]).
    pub rating: Option<u8>,
    /// `Colour`, the track's color tag.
    pub colour: Option<Rgb>,
    /// `Tonality` normalized, in whatever notation it was written (the
    /// raw value is [`Track::tonality`]). `None` if it's empty or in a
    /// notation `tags::key` doesn't read.
    pub key: Option<MusicalKey>,
    /// The beat grid, in file order.
    pub tempos: Vec<Tempo>,
    /// Memory cues, hot cues and loops, in file order.
    pub cues: Vec<Cue>,
}

macro_rules! text_attrs {
    ($($(#[$doc:meta])* $method:ident => $name:literal,)*) => {
        impl Track {
            $(
                $(#[$doc])*
                pub fn $method(&self) -> &str {
                    self.attrs.get($name).unwrap_or("")
                }
            )*
        }
    };
}

text_attrs! {
    /// `Name`: the title.
    name => "Name",
    artist => "Artist",
    composer => "Composer",
    album => "Album",
    /// `Grouping` holds rekordbox's color name, not user data (§5.3).
    grouping => "Grouping",
    genre => "Genre",
    /// `Kind`: `MP3 File`, `FLAC File`… As rekordbox saw it, not sniffed.
    kind => "Kind",
    comments => "Comments",
    remixer => "Remixer",
    /// `Tonality` as written, in the user's key notation (§5.3).
    tonality => "Tonality",
    label => "Label",
    mix => "Mix",
    /// `Location` as written, before decoding.
    location_raw => "Location",
}

impl Track {
    /// The rating in stars, when it's one of rekordbox's six values.
    pub fn stars(&self) -> Option<u8> {
        match self.rating? {
            0 => Some(0),
            51 => Some(1),
            102 => Some(2),
            153 => Some(3),
            204 => Some(4),
            255 => Some(5),
            _ => None,
        }
    }

    /// Whether this is a streaming entry, with no file.
    pub fn is_streaming(&self) -> bool {
        matches!(self.location, Ok(Location::Streaming(_)))
    }
}

/// One `TEMPO` entry of the beat grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Tempo {
    pub attrs: Attrs,
    /// `Inizio`: where this grid section starts, in seconds.
    pub start: Option<f64>,
    /// `Bpm`.
    pub bpm: Option<f64>,
    /// `Metro`: the time signature, `4/4`.
    pub meter: Option<Meter>,
    /// `Battito`: which beat of the bar `start` falls on, from 1.
    pub beat: Option<u8>,
}

/// A time signature: `beats`/`unit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meter {
    pub beats: u8,
    pub unit: u8,
}

/// One `POSITION_MARK`: a memory cue, hot cue or loop.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub attrs: Attrs,
    /// `Type`.
    pub kind: Option<CueKind>,
    /// `Start` in seconds.
    pub start: Option<f64>,
    /// `End` in seconds, for loops.
    pub end: Option<f64>,
    /// `Num`: `-1` for a memory cue, `0` up for hot cues A, B, …
    pub num: Option<i32>,
    /// `Red`, `Green`, `Blue`, when all three are there.
    pub color: Option<Rgb>,
}

/// A `POSITION_MARK`'s `Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueKind {
    /// `0`
    Cue,
    /// `1`
    FadeIn,
    /// `2`
    FadeOut,
    /// `3`
    Load,
    /// `4`
    Loop,
}

/// Where a cue sits: in the memory list, or on a hot-cue pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueSlot {
    Memory,
    /// Pad 0 is A.
    Hot(u8),
}

impl Cue {
    /// `Name`.
    pub fn name(&self) -> &str {
        self.attrs.get("Name").unwrap_or("")
    }

    pub fn slot(&self) -> Option<CueSlot> {
        match self.num? {
            -1 => Some(CueSlot::Memory),
            n => u8::try_from(n).ok().map(CueSlot::Hot),
        }
    }
}

/// Reads typed values out of one element's attributes, reporting each
/// value that doesn't parse.
pub(crate) struct Fields<'a> {
    pub attrs: &'a Attrs,
    pub element: Element,
    pub offset: u64,
    pub track_id: Option<u64>,
    pub warnings: &'a mut Vec<Warning>,
}

impl Fields<'_> {
    /// `None` when the attribute is missing or empty; `None` plus a warning
    /// when it's there but doesn't parse.
    fn get<T>(&mut self, name: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<T> {
        let value = self.attrs.get(name).filter(|v| !v.is_empty())?;
        let parsed = parse(value);
        if parsed.is_none() {
            let problem = Problem::BadValue {
                element: self.element,
                attribute: name.to_owned(),
                value: value.to_owned(),
            };
            self.warn(problem);
        }
        parsed
    }

    pub fn warn(&mut self, problem: Problem) {
        self.warnings.push(Warning {
            offset: self.offset,
            track_id: self.track_id,
            problem,
        });
    }
}

/// Builds a track from its `TRACK` attributes; cues and grid entries are
/// added as they're read.
pub(crate) fn track(attrs: Attrs, offset: u64, warnings: &mut Vec<Warning>) -> Track {
    let mut f = Fields {
        attrs: &attrs,
        element: Element::Track,
        offset,
        track_id: None,
        warnings,
    };
    let track_id = f.get("TrackID", digits::<u64>);
    f.track_id = track_id;
    if track_id.is_none() && attrs.get("TrackID").is_none_or(str::is_empty) {
        f.warn(Problem::MissingTrackId);
    }
    let location = location::decode(attrs.get("Location").unwrap_or(""));
    if let Err(error) = &location {
        f.warn(Problem::BadLocation {
            value: attrs.get("Location").unwrap_or("").to_owned(),
            error: error.clone(),
        });
    }
    let nonzero_u32 = |s: &str| digits::<u32>(s).map(|n| (n != 0).then_some(n));
    let size = f.get("Size", digits::<u64>);
    let total_time = f.get("TotalTime", digits::<u32>);
    let disc_number = f.get("DiscNumber", nonzero_u32).flatten();
    let track_number = f.get("TrackNumber", nonzero_u32).flatten();
    let year = f
        .get("Year", |s| digits::<u16>(s).map(|n| (n != 0).then_some(n)))
        .flatten();
    let average_bpm = f
        .get("AverageBpm", |s| {
            decimal(s)
                .filter(|&v| v >= 0.0)
                .map(|v| (v > 0.0).then_some(v))
        })
        .flatten();
    let date_modified = f.get("DateModified", Date::parse);
    let date_added = f.get("DateAdded", Date::parse);
    let bit_rate = f.get("BitRate", digits::<u32>);
    let sample_rate = f.get("SampleRate", digits::<u32>);
    let play_count = f.get("PlayCount", digits::<u32>);
    let last_played = f.get("LastPlayed", Date::parse);
    let rating = f.get("Rating", digits::<u8>);
    let colour = f.get("Colour", Rgb::parse_hex);
    let tonality = attrs.get("Tonality").unwrap_or("");
    let key = key::parse(tonality);
    if key.is_none() && !tonality.trim().is_empty() {
        f.warn(Problem::UnknownKey {
            value: tonality.to_owned(),
        });
    }
    Track {
        offset,
        track_id,
        location,
        size,
        total_time,
        disc_number,
        track_number,
        year,
        average_bpm,
        date_modified,
        date_added,
        bit_rate,
        sample_rate,
        play_count,
        last_played,
        rating,
        colour,
        key,
        tempos: Vec::new(),
        cues: Vec::new(),
        attrs,
    }
}

pub(crate) fn tempo(
    attrs: Attrs,
    offset: u64,
    track_id: Option<u64>,
    warnings: &mut Vec<Warning>,
) -> Tempo {
    let mut f = Fields {
        attrs: &attrs,
        element: Element::Tempo,
        offset,
        track_id,
        warnings,
    };
    let start = f.get("Inizio", |s| decimal(s).filter(|&v| v >= 0.0));
    let bpm = f.get("Bpm", |s| decimal(s).filter(|&v| v > 0.0));
    let meter = f.get("Metro", |s| {
        let (beats, unit) = s.split_once('/')?;
        let meter = Meter {
            beats: digits(beats)?,
            unit: digits(unit)?,
        };
        (meter.beats > 0 && meter.unit > 0).then_some(meter)
    });
    let beat = f.get("Battito", |s| digits::<u8>(s).filter(|&b| b > 0));
    Tempo {
        start,
        bpm,
        meter,
        beat,
        attrs,
    }
}

pub(crate) fn cue(
    attrs: Attrs,
    offset: u64,
    track_id: Option<u64>,
    warnings: &mut Vec<Warning>,
) -> Cue {
    let mut f = Fields {
        attrs: &attrs,
        element: Element::PositionMark,
        offset,
        track_id,
        warnings,
    };
    let kind = f.get("Type", |s| match s {
        "0" => Some(CueKind::Cue),
        "1" => Some(CueKind::FadeIn),
        "2" => Some(CueKind::FadeOut),
        "3" => Some(CueKind::Load),
        "4" => Some(CueKind::Loop),
        _ => None,
    });
    let start = f.get("Start", |s| decimal(s).filter(|&v| v >= 0.0));
    let end = f.get("End", |s| decimal(s).filter(|&v| v >= 0.0));
    let num = f.get("Num", |s| {
        let n: i32 = match s.strip_prefix('-') {
            Some(rest) => -digits::<i32>(rest)?,
            None => digits(s)?,
        };
        (n >= -1).then_some(n)
    });
    let red = f.get("Red", digits::<u8>);
    let green = f.get("Green", digits::<u8>);
    let blue = f.get("Blue", digits::<u8>);
    let present = ["Red", "Green", "Blue"]
        .iter()
        .filter(|n| attrs.get(n).is_some_and(|v| !v.is_empty()))
        .count();
    let color = match (red, green, blue) {
        (Some(r), Some(g), Some(b)) => Some(Rgb { r, g, b }),
        _ => None,
    };
    if present > 0 && present < 3 {
        f.warn(Problem::PartialColor);
    }
    Cue {
        kind,
        start,
        end,
        num,
        color,
        attrs,
    }
}
