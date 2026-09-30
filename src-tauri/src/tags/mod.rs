//! Reading tags from audio files, leniently (ROADMAP §5.5).
//!
//! Every format names its fields differently — ID3 `TPE1`, Vorbis `ARTIST`,
//! MP4 `©ART` — and the DJ-relevant fields are the least standardized of all.
//! `lofty`'s `ItemKey` handles most of that mapping; this module owns the
//! fallback chains and value parsing on top of it.
//!
//! Ported from musicmanager's `tags/mod.rs`, read side only (ROADMAP §0.2).
//! Tag writing arrives in Phase 2; nothing here opens a file for writing.
//! Changed from musicmanager:
//! - A read returns what was readable **plus a list of problems**, instead of
//!   a single "degraded" note, and a strict first pass names what was wrong.
//! - BPM, key and the Mixed In Key energy come back as [`AnalysisReading`]s
//!   with their source, not as plain fields: `analysis` is the source of truth
//!   for them (ROADMAP §2) and these are only inputs to it.
//! - The raw tag frames come back too, for `file.raw_tags` ([`raw`]).
//! - When lofty rejects a file, the `salvage` module reads around the
//!   broken part, and a lofty panic becomes an error instead of killing the
//!   caller.
//! - BPM accepts a "bpm" suffix, must be 20–300, and "1,234" is rejected as
//!   ambiguous instead of read as 1.234.
//! - Remixer and composer are read (rekordbox has fields for both).
//! - Artwork isn't hashed; artwork handling isn't part of this task.

pub mod key;
pub mod raw;
mod salvage;

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use lofty::aac::AacFile;
use lofty::ape::ApeFile;
use lofty::config::{ParseOptions, ParsingMode};
use lofty::file::{AudioFile, FileType, TaggedFile};
use lofty::flac::FlacFile;
use lofty::iff::aiff::AiffFile;
use lofty::iff::wav::WavFile;
use lofty::mp4::Mp4File;
use lofty::mpeg::MpegFile;
use lofty::musepack::MpcFile;
use lofty::ogg::{OggPictureStorage, OpusFile, SpeexFile, VorbisFile};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag};
use lofty::wavpack::WavPackFile;

pub use raw::{RawItem, RawTag, RawTagType, RawValue};

/// File extensions we attempt to index. Anything else is skipped silently
/// rather than reported as an error — music folders are full of cue sheets,
/// artwork, and `.DS_Store`. The format itself is detected from the bytes.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "wave", "aif", "aiff", "aifc", "m4a", "mp4", "aac", "alac", "ogg", "oga",
    "opus", "wv", "mpc", "ape",
];

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            AUDIO_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

/// Everything a tag read found in one file.
#[derive(Debug, Clone, Default)]
pub struct TagRead {
    /// The container lofty parsed the file as. This is lofty's own guess,
    /// used to pick a parser; `file.sniffed_format` comes from the format
    /// sniffer, not from here.
    pub parsed_as: &'static str,
    pub properties: AudioProperties,
    pub fields: TagFields,
    /// BPM and key from the tags, and energy from a Mixed In Key comment.
    /// Inputs to `analysis`, never truth on their own.
    pub analysis: Vec<AnalysisReading>,
    /// Every tag block in the file, frame by frame, for `file.raw_tags`.
    pub raw_tags: Vec<RawTag>,
    /// What was wrong with the tags. Empty for a clean file.
    pub problems: Vec<TagProblem>,
}

impl TagRead {
    /// The reading from one source, if the tags gave one.
    pub fn analysis_from(&self, source: AnalysisSource) -> Option<&AnalysisReading> {
        self.analysis.iter().find(|a| a.source == source)
    }

    /// The first tag block of this type.
    pub fn raw_tag(&self, tag_type: RawTagType) -> Option<&RawTag> {
        self.raw_tags.iter().find(|t| t.tag_type == tag_type)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioProperties {
    pub duration_ms: Option<i64>,
    pub bitrate_kbps: Option<i64>,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
}

/// The descriptive fields, from the file's main tag.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagFields {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub comment: Option<String>,
    pub composer: Option<String>,
    pub remixer: Option<String>,
    pub label: Option<String>,
    pub isrc: Option<String>,
    pub year: Option<i64>,
    pub track_no: Option<i64>,
    pub disc_no: Option<i64>,
    // Rating is deliberately absent: it comes from ID3 `POPM`, whose 0-255
    // scale each program maps to stars differently. The raw frame is kept in
    // `raw_tags`.
}

/// Where an [`AnalysisReading`] came from. These match `analysis.source`
/// (ROADMAP §2) for the two sources a tag read can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisSource {
    /// The file's BPM and key tags. Whoever wrote them (rekordbox, Mixed In
    /// Key, a store) isn't recorded in the file, so this is just "tag".
    Tag,
    /// Mixed In Key's `Energy N` comment.
    Mik,
}

/// BPM, key or energy as one source states it. An input to `analysis`.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisReading {
    pub source: AnalysisSource,
    pub bpm: Option<f64>,
    /// Camelot, normalized by [`key::to_camelot`].
    pub key: Option<String>,
    /// The key exactly as the tag wrote it.
    pub key_raw: Option<String>,
    /// 1–10.
    pub energy: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TagProblem {
    pub kind: TagProblemKind,
    /// lofty's error, or the value that couldn't be read. For logs and the
    /// diagnostics bundle, not for the UI as is.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagProblemKind {
    /// A strict read rejected the tags. Everything else was then read
    /// leniently, so values may be missing.
    Malformed,
    /// Only a relaxed read worked: some tag fields were malformed and were
    /// skipped.
    FieldsSkipped,
    /// A trailing APE tag was too broken to read, so the file was read
    /// without it. Its frames are missing from `raw_tags`.
    TagBlockSkipped,
    /// Some ID3v2 frames were too broken to read and were dropped; the rest
    /// of the tag was read. `detail` names them.
    FramesDropped,
    /// The tags couldn't be read at all; only the audio properties were.
    TagsUnreadable,
    /// The duration made no sense (over [`MAX_DURATION_MS`]) and was dropped.
    ImplausibleDuration,
    /// A raw tag value was longer than [`raw::MAX_TEXT_BYTES`] and was cut.
    /// `detail` names the key and the full length.
    ValueTruncated,
    /// A field was present but its value made no sense (a BPM of "fast", a
    /// year of "n/a", a key of "unknown").
    BadValue(Field),
}

/// Fields whose values are parsed, and so can be unreadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Bpm,
    Key,
    Year,
}

#[derive(Debug)]
pub enum TagReadError {
    /// The file couldn't be opened for reading.
    Open(std::io::Error),
    /// Neither the bytes nor the extension name a format we can read.
    UnknownFormat,
    /// Every read, down to properties only, failed: this isn't readable
    /// audio. Broken-file detection (1aB-4) takes it from here.
    Unreadable(String),
}

impl fmt::Display for TagReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TagReadError::Open(e) => write!(f, "could not open the file: {e}"),
            TagReadError::UnknownFormat => write!(f, "not a format we can read"),
            TagReadError::Unreadable(e) => write!(f, "could not read the file: {e}"),
        }
    }
}

impl std::error::Error for TagReadError {}

/// One rung of the parsing ladder.
struct Attempt {
    mode: ParsingMode,
    /// The problem to record when this rung is the one that worked. `None`
    /// for the strict rung, which is the expected path.
    problem: Option<TagProblemKind>,
}

/// Escalating fallbacks, strictest first; [`salvage`] is the last rung.
///
/// Real libraries are full of files whose audio is perfectly good but whose
/// tags are not: a junk APE tag appended to an MP3, a malformed ID3 header, a
/// text frame in a dead encoding. Giving up at the first rung throws away a
/// usable track over metadata we did not need. Each fallback loses something,
/// so what was lost is recorded rather than hidden.
///
/// The strict rung comes first (musicmanager started at best-attempt) because
/// best-attempt quietly blanks what it can't decode; only a strict failure
/// says what was wrong. A clean file is read once.
const ATTEMPTS: &[Attempt] = &[
    Attempt {
        mode: ParsingMode::Strict,
        problem: None,
    },
    Attempt {
        mode: ParsingMode::BestAttempt,
        problem: Some(TagProblemKind::Malformed),
    },
    Attempt {
        mode: ParsingMode::Relaxed,
        problem: Some(TagProblemKind::FieldsSkipped),
    },
];

fn options(mode: ParsingMode, read_tags: bool) -> ParseOptions {
    ParseOptions::new()
        .read_properties(true)
        .read_tags(read_tags)
        .read_cover_art(true)
        .parsing_mode(mode)
}

/// Reads tags, raw frames and audio properties from a file.
///
/// Opens the file read-only; it never writes, renames or touches its times.
/// Returns `Err` only when the file can't be opened or isn't readable audio
/// at all. Broken tags give an `Ok` with whatever was readable and the
/// problems listed. A file with no tags gives empty fields, which is correct:
/// plenty of DJ rips are untagged.
pub fn read(path: &Path) -> Result<TagRead, TagReadError> {
    let mut read = read_file(path)?;
    for item in read.raw_tags.iter().flat_map(|t| &t.items) {
        if let RawValue::Text {
            full_len: Some(len),
            ..
        } = item.value
        {
            read.problems.push(TagProblem {
                kind: TagProblemKind::ValueTruncated,
                detail: format!("{}: {len} bytes", item.key),
            });
        }
    }
    Ok(read)
}

fn read_file(path: &Path) -> Result<TagRead, TagReadError> {
    // `File::open` asks for read access only.
    let file = File::open(path).map_err(TagReadError::Open)?;
    let mut reader = BufReader::new(file);

    let guessed = guard(|| {
        Probe::new(&mut reader)
            .guess_file_type()
            .map(|p| p.file_type())
            .map_err(|e| e.to_string())
    })
    .map_err(TagReadError::Unreadable)?;
    let file_type = guessed
        .or_else(|| FileType::from_path(path))
        .ok_or(TagReadError::UnknownFormat)?;

    let mut first_error: Option<String> = None;
    for attempt in ATTEMPTS {
        reader.rewind().map_err(TagReadError::Open)?;
        match read_as(file_type, &mut reader, options(attempt.mode, true)) {
            Ok(parsed) => {
                let mut read = parsed.into_read();
                if let Some(kind) = attempt.problem {
                    // The strict rung's error says what was wrong; later
                    // rungs' errors only repeat it less precisely.
                    let detail = first_error.clone().unwrap_or_default();
                    read.problems.insert(0, TagProblem { kind, detail });
                }
                return Ok(read);
            }
            Err(err) => {
                first_error.get_or_insert(err);
            }
        }
    }

    let error = first_error.unwrap_or_default();
    salvage(file_type, &mut reader, &error)
        .map_err(TagReadError::Open)?
        .ok_or(TagReadError::Unreadable(error))
}

/// The last rung: read around the broken part (see [`salvage`](mod@salvage)).
/// `None` when even the audio properties can't be read.
fn salvage<R: Read + Seek>(
    file_type: FileType,
    reader: &mut R,
    error: &str,
) -> std::io::Result<Option<TagRead>> {
    let problem = |kind, detail: String| TagProblem { kind, detail };
    let relaxed = |tags| options(ParsingMode::Relaxed, tags);
    let len = reader.seek(std::io::SeekFrom::End(0))?;

    // A trailing APE tag (and any ID3v1 tag after it) is read on its own,
    // and the rest of the file is read without them.
    let trailing = salvage::trailing(reader)?;
    let trailing_tags = |read: &mut TagRead| {
        let Some(t) = &trailing else { return };
        if let Some(ape) = &t.ape {
            read.raw_tags.push(raw::ape(ape));
            fill_gaps(read, &Tag::from(ape.clone()));
        }
        if let Some(v1) = &t.id3v1 {
            read.raw_tags.push(raw::id3v1(v1));
            fill_gaps(read, &Tag::from(v1.clone()));
        }
    };
    let ape_broken = trailing.as_ref().is_some_and(|t| t.ape_broken);
    let view_len = trailing.as_ref().map_or(len, |t| t.start);

    if trailing.is_some() {
        let mut view = salvage::Prefix::new(reader, view_len)?;
        if let Ok(parsed) = read_as(file_type, &mut view, relaxed(true)) {
            let mut read = parsed.into_read();
            trailing_tags(&mut read);
            // The read only worked without the trailing tags, so they were
            // the trouble: the broken APE tag, or else lofty's handling of
            // the pair.
            let kind = if ape_broken {
                TagProblemKind::TagBlockSkipped
            } else {
                TagProblemKind::Malformed
            };
            read.problems.insert(0, problem(kind, error.to_string()));
            return Ok(Some(read));
        }
    }

    // The tags are beyond lofty: read the audio properties alone (without
    // the trailing tags, which can stop even that), then rebuild what we can
    // of the ID3v2 tag, and add the trailing tags read on their own.
    let mut view = salvage::Prefix::new(reader, view_len)?;
    let Ok(parsed) = read_as(file_type, &mut view, relaxed(false)) else {
        return Ok(None);
    };
    let mut read = parsed.into_read();
    let mut problems = Vec::new();

    match salvage::id3v2(file_type, reader)? {
        Some(salvaged) if !salvaged.tag.is_empty() || !salvaged.dropped.is_empty() => {
            if !salvaged.tag.is_empty() {
                read.raw_tags.push(raw::id3v2(&salvaged.tag));
                apply_tag(&mut read, &Tag::from(salvaged.tag));
            }
            if !salvaged.dropped.is_empty() {
                let detail = format!("dropped {}: {error}", salvaged.dropped.join(", "));
                problems.push(problem(TagProblemKind::FramesDropped, detail));
            }
        }
        _ => {}
    }
    trailing_tags(&mut read);
    if ape_broken {
        problems.push(problem(TagProblemKind::TagBlockSkipped, error.to_string()));
    }
    if read.raw_tags.is_empty() {
        problems.insert(
            0,
            problem(TagProblemKind::TagsUnreadable, error.to_string()),
        );
    } else if problems.is_empty() {
        problems.push(problem(TagProblemKind::Malformed, error.to_string()));
    }
    read.problems.splice(0..0, problems);
    Ok(Some(read))
}

/// Fills the fields and readings `read` lacks from a secondary tag. Used
/// only when salvaging, where the main tag may have lost frames.
fn fill_gaps(read: &mut TagRead, tag: &Tag) {
    let mut other = TagRead::default();
    apply_tag(&mut other, tag);
    macro_rules! fill {
        ($($field:ident),*) => {
            $(
                if read.fields.$field.is_none() {
                    read.fields.$field = other.fields.$field.take();
                }
            )*
        };
    }
    fill!(
        title,
        artist,
        album,
        album_artist,
        genre,
        comment,
        composer,
        remixer,
        label,
        isrc,
        year,
        track_no,
        disc_no
    );
    for reading in other.analysis {
        if read.analysis_from(reading.source).is_none() {
            read.analysis.push(reading);
        }
    }
}

/// Runs lofty code, turning a panic into an error. lofty panics on some
/// corrupt input (e.g. an Opus header with zero channels), and one bad file
/// must not kill a scan worker.
pub(super) fn guard<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(format!("the tag library crashed on this file: {message}"))
        }
    }
}

/// A file as lofty parsed it, plus its raw tag blocks.
struct Parsed {
    tagged: TaggedFile,
    raw_tags: Vec<RawTag>,
}

impl Parsed {
    fn into_read(self) -> TagRead {
        let mut read = from_tagged_file(&self.tagged);
        read.raw_tags = self.raw_tags;
        read
    }
}

/// Reads the file as its concrete type, so the raw tag blocks can be listed
/// before they're folded into lofty's generic `Tag`. A lofty panic comes
/// back as an error.
fn read_as<R: Read + Seek>(
    file_type: FileType,
    reader: &mut R,
    options: ParseOptions,
) -> Result<Parsed, String> {
    guard(|| read_as_unguarded(file_type, reader, options))
}

fn read_as_unguarded<R: Read + Seek>(
    file_type: FileType,
    reader: &mut R,
    options: ParseOptions,
) -> Result<Parsed, String> {
    fn tags(parts: &[Option<RawTag>]) -> Vec<RawTag> {
        parts.iter().flatten().cloned().collect()
    }

    let (tagged, raw_tags) = match file_type {
        FileType::Mpeg => {
            let f = MpegFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[
                f.id3v2().map(raw::id3v2),
                f.ape().map(raw::ape),
                f.id3v1().map(raw::id3v1),
            ]);
            (f.into(), raw)
        }
        FileType::Flac => {
            let f = FlacFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[
                raw::vorbis(f.vorbis_comments(), f.pictures()),
                f.id3v2().map(raw::id3v2),
            ]);
            (f.into(), raw)
        }
        FileType::Wav => {
            let f = WavFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[f.id3v2().map(raw::id3v2), f.riff_info().map(raw::riff_info)]);
            (f.into(), raw)
        }
        FileType::Aiff => {
            let f = AiffFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[
                f.id3v2().map(raw::id3v2),
                f.text_chunks().map(raw::aiff_text),
            ]);
            (f.into(), raw)
        }
        FileType::Mp4 => {
            let f = Mp4File::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[f.ilst().map(raw::ilst)]);
            (f.into(), raw)
        }
        FileType::Vorbis => {
            let f = VorbisFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[raw::vorbis(Some(f.vorbis_comments()), &[])]);
            (f.into(), raw)
        }
        FileType::Opus => {
            let f = OpusFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[raw::vorbis(Some(f.vorbis_comments()), &[])]);
            (f.into(), raw)
        }
        FileType::Speex => {
            let f = SpeexFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[raw::vorbis(Some(f.vorbis_comments()), &[])]);
            (f.into(), raw)
        }
        FileType::Aac => {
            let f = AacFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[f.id3v2().map(raw::id3v2), f.id3v1().map(raw::id3v1)]);
            (f.into(), raw)
        }
        FileType::Ape => {
            let f = ApeFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[
                f.ape().map(raw::ape),
                f.id3v2().map(raw::id3v2),
                f.id3v1().map(raw::id3v1),
            ]);
            (f.into(), raw)
        }
        FileType::WavPack => {
            let f = WavPackFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[f.ape().map(raw::ape), f.id3v1().map(raw::id3v1)]);
            (f.into(), raw)
        }
        FileType::Mpc => {
            let f = MpcFile::read_from(reader, options).map_err(describe)?;
            let raw = tags(&[
                f.ape().map(raw::ape),
                f.id3v2().map(raw::id3v2),
                f.id3v1().map(raw::id3v1),
            ]);
            (f.into(), raw)
        }
        // lofty's own file-type list is open-ended; we read what we know.
        _ => return Err(format!("no reader for {file_type:?}")),
    };
    Ok(Parsed { tagged, raw_tags })
}

/// lofty's error with its causes, e.g. "ID3v2: invalid frame: ...".
fn describe(err: lofty::error::FileParseError) -> String {
    let mut text = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

/// Longest believable duration: two days. Longer is a corrupt header (lofty
/// computes Opus durations from the last granule position, which a bad file
/// can set to anything).
pub const MAX_DURATION_MS: u64 = 48 * 60 * 60 * 1000;

fn from_tagged_file(tagged: &TaggedFile) -> TagRead {
    let properties = tagged.properties();
    let mut read = TagRead {
        parsed_as: format_name(tagged.file_type()),
        properties: AudioProperties {
            duration_ms: None,
            bitrate_kbps: properties.audio_bitrate().map(|b| b as i64),
            sample_rate: properties.sample_rate().map(|s| s as i64),
            channels: properties.channels().map(|c| c as i64),
        },
        ..Default::default()
    };
    let duration_ms = properties.duration().as_millis();
    if duration_ms <= MAX_DURATION_MS as u128 {
        read.properties.duration_ms = Some(duration_ms as i64);
    } else {
        read.problems.push(TagProblem {
            kind: TagProblemKind::ImplausibleDuration,
            detail: format!("{duration_ms} ms"),
        });
    }

    // `primary_tag` picks the format's preferred tag (ID3v2 over ID3v1 on MP3,
    // for instance). Falling back to `first_tag` catches files carrying only a
    // secondary tag, which is common on older rips, and files whose main tag
    // was too broken to keep.
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        apply_tag(&mut read, tag);
    }

    read
}

fn apply_tag(read: &mut TagRead, tag: &Tag) {
    let fields = &mut read.fields;
    let problems = &mut read.problems;

    fields.title = non_empty(tag.title());
    fields.artist = non_empty(tag.artist());
    fields.album = non_empty(tag.album());
    fields.genre = non_empty(tag.genre());
    fields.comment = non_empty(tag.comment());

    fields.album_artist = string_of(tag, ItemKey::AlbumArtist);
    fields.composer = string_of(tag, ItemKey::Composer);
    fields.remixer = string_of(tag, ItemKey::Remixer);
    // ID3v2's `TPUB` maps to `Publisher`, with `Label` as an alias — so writing
    // `Label` lands in TPUB but reading it back only answers to `Publisher`.
    // Vorbis and MP4 keep the two genuinely separate, hence the fallback rather
    // than picking one.
    fields.label = string_of(tag, ItemKey::Label).or_else(|| string_of(tag, ItemKey::Publisher));
    fields.isrc = string_of(tag, ItemKey::Isrc);

    fields.track_no = tag.track().map(|n| n as i64);
    fields.disc_no = tag.disk().map(|n| n as i64);

    // Year: an explicit year field wins, then the leading year of a full
    // recording date ("2019-04-12" -> 2019).
    let year_raw = string_of(tag, ItemKey::Year).or_else(|| string_of(tag, ItemKey::RecordingDate));
    fields.year = year_raw
        .as_deref()
        .and_then(parse_year)
        .or_else(|| tag.date().map(|d| d.year as i64));
    if let (Some(raw), None) = (&year_raw, fields.year) {
        problems.push(bad_value(Field::Year, raw));
    }

    // BPM: prefer the precise field, fall back to the rounded one. Some
    // taggers write "128.00", others "128", others "128,5" in locales that use
    // a comma decimal separator.
    let bpm_raw = string_of(tag, ItemKey::Bpm).or_else(|| string_of(tag, ItemKey::IntegerBpm));
    let bpm = string_of(tag, ItemKey::Bpm)
        .and_then(|v| parse_bpm(&v))
        .or_else(|| string_of(tag, ItemKey::IntegerBpm).and_then(|v| parse_bpm(&v)));
    if let (Some(raw), None) = (&bpm_raw, bpm) {
        problems.push(bad_value(Field::Bpm, raw));
    }

    let key_raw = string_of(tag, ItemKey::InitialKey);
    let key = key_raw.as_deref().and_then(key::to_camelot);
    if let (Some(raw), None) = (&key_raw, &key) {
        problems.push(bad_value(Field::Key, raw));
    }

    if bpm.is_some() || key_raw.is_some() {
        read.analysis.push(AnalysisReading {
            source: AnalysisSource::Tag,
            bpm,
            key,
            key_raw,
            energy: None,
        });
    }

    // rekordbox has no energy field, so Mixed In Key writes it into the
    // comment ("8A - Energy 7"). It's MIK's opinion, not a measurement.
    if let Some(energy) = fields.comment.as_deref().and_then(parse_energy) {
        read.analysis.push(AnalysisReading {
            source: AnalysisSource::Mik,
            bpm: None,
            key: None,
            key_raw: None,
            energy: Some(energy),
        });
    }
}

fn bad_value(field: Field, raw: &str) -> TagProblem {
    TagProblem {
        kind: TagProblemKind::BadValue(field),
        detail: raw.to_string(),
    }
}

fn non_empty(value: Option<std::borrow::Cow<'_, str>>) -> Option<String> {
    value
        .map(|v| v.into_owned())
        .filter(|s| !s.trim().is_empty())
}

fn string_of(tag: &Tag, item_key: ItemKey) -> Option<String> {
    tag.get_string(item_key)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn format_name(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Aac => "AAC",
        FileType::Aiff => "AIFF",
        FileType::Ape => "APE",
        FileType::Flac => "FLAC",
        FileType::Mpeg => "MP3",
        FileType::Mp4 => "MP4",
        FileType::Mpc => "MPC",
        FileType::Opus => "Opus",
        FileType::Vorbis => "Vorbis",
        FileType::Speex => "Speex",
        FileType::Wav => "WAV",
        FileType::WavPack => "WavPack",
        _ => "Unknown",
    }
}

fn parse_year(raw: &str) -> Option<i64> {
    let digits: String = raw
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let year: i64 = digits.parse().ok()?;
    // Guards against a track number or a stray "0" landing in the year column.
    (1000..=9999).contains(&year).then_some(year)
}

/// BPMs outside this range are junk (0 means "unset" in practice). Wide
/// enough for half- and double-time tagging.
const BPM_RANGE: std::ops::RangeInclusive<f64> = 20.0..=300.0;

fn parse_bpm(raw: &str) -> Option<f64> {
    let mut s = raw.trim();
    // "128 BPM", "128bpm".
    if let Some(at) = s.len().checked_sub(3) {
        if s.get(at..).is_some_and(|u| u.eq_ignore_ascii_case("bpm")) {
            s = s[..at].trim_end();
        }
    }
    let cleaned = match (s.matches(',').count(), s.contains('.')) {
        (0, _) => s.to_string(),
        // A decimal comma, as some locales write it: "124,5". Three digits
        // after it read as a thousands separator ("1,234"), which is
        // ambiguous, so it's rejected rather than guessed.
        (1, false) if s.split_once(',').is_some_and(|(_, frac)| frac.len() < 3) => {
            s.replace(',', ".")
        }
        _ => return None,
    };
    // Digits and one point only: no signs, exponents, "inf" or "NaN".
    if cleaned.is_empty() || !cleaned.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let value: f64 = cleaned.parse().ok()?;
    BPM_RANGE.contains(&value).then_some(value)
}

/// Extracts an energy rating from a comment field.
///
/// Recognizes the conventions DJs actually use: `Energy 7`, `energy7`,
/// `Energy Level 7`, and a bare `E7` token. Deliberately conservative — a
/// comment mentioning "E7" as a chord should be rare, but more importantly a
/// missed energy value is harmless while a wrong one is misleading.
fn parse_energy(comment: &str) -> Option<i64> {
    let lower = comment.to_ascii_lowercase();

    if let Some(index) = lower.find("energy") {
        let rest = &lower[index + "energy".len()..];
        let rest = rest.trim_start_matches(|c: char| !c.is_ascii_digit() && c != '\n');
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(value) = digits.parse::<i64>() {
            if (1..=10).contains(&value) {
                return Some(value);
            }
        }
    }

    // Bare "E7" style, but only as a standalone token.
    for token in lower.split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '/') {
        if let Some(digits) = token.strip_prefix('e') {
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(value) = digits.parse::<i64>() {
                    if (1..=10).contains(&value) {
                        return Some(value);
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
pub(crate) mod test_audio;
#[cfg(test)]
mod tests;

/// Ported from musicmanager's `tags/mod.rs` tests.
#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn recognizes_audio_extensions_case_insensitively() {
        assert!(is_audio_file(Path::new("track.mp3")));
        assert!(is_audio_file(Path::new("track.MP3")));
        assert!(is_audio_file(Path::new("track.Flac")));
        assert!(is_audio_file(Path::new("track.aiff")));
        assert!(!is_audio_file(Path::new("cover.jpg")));
        assert!(!is_audio_file(Path::new("playlist.m3u8")));
        assert!(!is_audio_file(Path::new("no_extension")));
    }

    #[test]
    fn parses_bpm_across_tagger_conventions() {
        assert_eq!(parse_bpm("128"), Some(128.0));
        assert_eq!(parse_bpm("128.00"), Some(128.0));
        assert_eq!(parse_bpm(" 174.5 "), Some(174.5));
        // Comma decimal separator, written by taggers in some locales.
        assert_eq!(parse_bpm("124,5"), Some(124.5));
    }

    #[test]
    fn rejects_junk_bpm() {
        assert_eq!(parse_bpm(""), None);
        assert_eq!(parse_bpm("0"), None);
        assert_eq!(parse_bpm("unknown"), None);
        assert_eq!(parse_bpm("-120"), None);
    }

    #[test]
    fn parses_year_from_plain_and_full_dates() {
        assert_eq!(parse_year("2019"), Some(2019));
        assert_eq!(parse_year("2019-04-12"), Some(2019));
        assert_eq!(parse_year("1997/01/01"), Some(1997));
    }

    #[test]
    fn rejects_implausible_years() {
        assert_eq!(parse_year(""), None);
        assert_eq!(parse_year("0"), None);
        // A track number that wandered into the year field.
        assert_eq!(parse_year("7"), None);
        assert_eq!(parse_year("n/a"), None);
    }

    #[test]
    fn extracts_energy_from_common_comment_conventions() {
        assert_eq!(parse_energy("Energy 7"), Some(7));
        assert_eq!(parse_energy("energy7"), Some(7));
        assert_eq!(parse_energy("Energy Level 8"), Some(8));
        assert_eq!(parse_energy("ENERGY 10"), Some(10));
        assert_eq!(parse_energy("peak time / Energy 9"), Some(9));
        assert_eq!(parse_energy("E5"), Some(5));
        assert_eq!(parse_energy("banger, E8, warmup"), Some(8));
    }

    #[test]
    fn ignores_comments_without_an_energy_rating() {
        assert_eq!(parse_energy(""), None);
        assert_eq!(parse_energy("great track"), None);
        // Out of range: not an energy rating.
        assert_eq!(parse_energy("Energy 42"), None);
        assert_eq!(parse_energy("E99"), None);
        // Must be a standalone token, not part of a word.
        assert_eq!(parse_energy("take5"), None);
    }

    // Not in musicmanager.

    #[test]
    fn bpm_values_with_a_bpm_suffix_are_read() {
        assert_eq!(parse_bpm("128 BPM"), Some(128.0));
        assert_eq!(parse_bpm("128bpm"), Some(128.0));
        assert_eq!(parse_bpm("174.5 Bpm"), Some(174.5));
        assert_eq!(parse_bpm("bpm"), None);
    }

    #[test]
    fn bpm_values_outside_20_to_300_are_rejected() {
        assert_eq!(parse_bpm("20"), Some(20.0));
        assert_eq!(parse_bpm("300"), Some(300.0));
        assert_eq!(parse_bpm("19.9"), None);
        assert_eq!(parse_bpm("301"), None);
        assert_eq!(parse_bpm("999"), None);
    }

    #[test]
    fn ambiguous_or_non_numeric_bpm_values_are_rejected() {
        // Thousands separator or decimal comma? Don't guess.
        assert_eq!(parse_bpm("1,234"), None);
        assert_eq!(parse_bpm("1,2,3"), None);
        assert_eq!(parse_bpm("1.234,5"), None);
        assert_eq!(parse_bpm("124,50"), Some(124.5));
        assert_eq!(parse_bpm("1e2"), None);
        assert_eq!(parse_bpm("inf"), None);
        assert_eq!(parse_bpm("NaN"), None);
        assert_eq!(parse_bpm("+128"), None);
    }

    #[test]
    fn reads_the_energy_from_mixed_in_keys_key_and_energy_comment() {
        // Mixed In Key's default comment layout.
        assert_eq!(parse_energy("8A - Energy 7"), Some(7));
        assert_eq!(parse_energy("11B - 124 - Energy 6"), Some(6));
    }
}
