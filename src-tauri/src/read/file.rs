//! Reading one file: format, codec, tags, audio properties and whether it's
//! broken, in one pass (1aB-2, 1aB-3, 1aB-4).
//!
//! The file is opened read-only, through the `\\?\` path the caller gives
//! (ROADMAP §5.6). One handle serves the sniffer and the codec headers,
//! both bounded reads; the lenient tag reader then reads tags and audio
//! properties together. Nothing here writes, renames or touches the file's
//! times.

use std::fs::File;
use std::io;
use std::path::Path;

use serde_json::{Map, Value};

use super::codec::{self, Codec};
use crate::sniff::{self, Problem, Sniff, SniffedFormat};
use crate::tags::{self, TagProblemKind, TagRead, TagReadError};

/// A `file.quality_verdict` this stage sets. The other four are 1b's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The file ends early: an interrupted download or copy.
    Truncated,
    /// Not playable at all: zero bytes, an MP4 with no index (`moov`),
    /// audio no reader can make sense of, or a duration of zero.
    Broken,
}

impl Verdict {
    /// The name stored in `file.quality_verdict`.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Truncated => "truncated",
            Verdict::Broken => "broken",
        }
    }
}

/// What one read found, for the file's stage-2 columns. Every column is
/// written, so values from an older version of the file don't linger.
#[derive(Debug, Clone, PartialEq)]
pub struct FileRead {
    pub sniffed_format: SniffedFormat,
    pub codec: Option<Codec>,
    pub bitrate_kbps: Option<i64>,
    pub sample_rate: Option<i64>,
    pub duration_ms: Option<i64>,
    /// Every tag block as a JSON object (`file.raw_tags`); `None` when the
    /// tags couldn't be read at all.
    pub raw_tags: Option<String>,
    pub verdict: Option<Verdict>,
}

/// Why a file wasn't read. Nothing is known about it, and it isn't broken
/// as far as anyone knows: damaged content is never this, it's a
/// [`Verdict`].
#[derive(Debug)]
pub enum NotRead {
    /// It couldn't be reached, opened or read: gone, locked, no permission,
    /// a disk error.
    Unreachable(io::Error),
    /// It became an online-only OneDrive file, which opening would
    /// download (1aB-8).
    OnlineOnly,
}

impl From<io::Error> for NotRead {
    fn from(e: io::Error) -> Self {
        NotRead::Unreachable(e)
    }
}

/// Reads the file at `path`: absolute, in the `\\?\` form on Windows. Both
/// opens use it, so a trailing dot or space names this file and never a
/// sibling (§5.6).
///
/// `may_open` is asked right before each open (the sniff's, then the tag
/// reader's): a file that has become online-only (`Ok(false)`), or whose
/// attributes can't be read (`Err`), isn't opened.
pub fn read(
    path: &Path,
    may_open: impl Fn(&Path) -> io::Result<bool>,
) -> Result<FileRead, NotRead> {
    let local = |path: &Path| match may_open(path) {
        Ok(true) => Ok(()),
        Ok(false) => Err(NotRead::OnlineOnly),
        Err(e) => Err(NotRead::Unreachable(e)),
    };
    let extension = path.extension().and_then(|e| e.to_str());
    local(path)?;
    // `File::open` asks for read access only.
    let mut file = File::open(path)?;
    let sniffed = sniff::sniff(&mut file, extension)?;
    let (codec, magic) = match sniffed.problem {
        Some(Problem::Empty) => (None, None),
        _ => (
            codec::codec(&mut file, sniffed.format)?,
            codec::container_magic(&mut file)?,
        ),
    };
    drop(file);
    let tags = match sniffed.problem {
        // Nothing to read.
        Some(Problem::Empty) => None,
        _ => {
            local(path)?;
            Some(tags::read(path))
        }
    };
    let tags = match tags {
        Some(Err(TagReadError::Open(e))) => return Err(e.into()),
        other => other,
    };
    Ok(assemble(sniffed, magic, codec, tags))
}

/// The columns from what the sniffer, the codec headers and the tag reader
/// found.
fn assemble(
    sniffed: Sniff,
    magic: Option<[u8; 4]>,
    codec: Option<Codec>,
    tags: Option<Result<TagRead, TagReadError>>,
) -> FileRead {
    let read = tags.and_then(Result::ok);
    let properties = read.as_ref().map(|r| &r.properties);
    // The file table refuses zero rates; zero means unknown.
    let positive = |v: Option<i64>| v.filter(|v| *v > 0);
    let format = sniffed.format;
    // A duration of 0 means "no audio" only where the container counts the
    // audio itself; elsewhere (a FLAC whose encoder didn't know the length
    // yet) it means unknown.
    let duration_ms = properties
        .and_then(|p| p.duration_ms)
        .filter(|ms| *ms > 0 || zero_means_no_audio(format));
    // Tags that couldn't be read at all aren't "no tags".
    let tags_read = read.as_ref().filter(|r| {
        !r.problems
            .iter()
            .any(|p| p.kind == TagProblemKind::TagsUnreadable)
    });
    FileRead {
        sniffed_format: format,
        codec,
        bitrate_kbps: positive(properties.and_then(|p| p.bitrate_kbps)),
        sample_rate: positive(properties.and_then(|p| p.sample_rate)),
        duration_ms,
        raw_tags: tags_read.map(raw_tags_json),
        verdict: verdict(
            sniffed.problem,
            read.is_some(),
            unreadable_means_broken(format, magic),
            duration_ms,
        ),
    }
}

/// Whether a duration of 0 from this container means it holds no audio:
/// WAV and AIFF count the bytes of audio, and an MP4 its samples. Anywhere
/// else a 0 can mean "unknown".
fn zero_means_no_audio(format: SniffedFormat) -> bool {
    use SniffedFormat as F;
    matches!(format, F::Wav | F::Rf64 | F::Aiff | F::Aifc | F::Mp4)
}

/// Whether the tag reader failing on a file of this format means the file
/// is broken: true for the containers it reads, and for bytes nobody
/// recognizes under an audio name. False for valid containers it doesn't
/// read at all (RF64, a big-endian RIFX WAVE, WMA, Ogg FLAC or Speex),
/// where its failure says nothing about the file. `magic` is the
/// container's first four bytes, after any ID3 tag.
fn unreadable_means_broken(format: SniffedFormat, magic: Option<[u8; 4]>) -> bool {
    use SniffedFormat as F;
    match format {
        F::Wav => magic == Some(*b"RIFF"),
        F::Rf64 | F::Asf | F::Ogg => false,
        F::Mp3
        | F::Aiff
        | F::Aifc
        | F::Flac
        | F::Mp4
        | F::Adts
        | F::OggVorbis
        | F::OggOpus
        | F::WavPack
        | F::Musepack
        | F::Ape
        | F::Unknown => true,
    }
}

/// Whether the file is truncated or broken, from the sniffer's problem,
/// whether the tag reader could read it as audio at all (and whether that
/// failing means anything for this format), and its duration (0 only where
/// it means no audio).
pub(super) fn verdict(
    problem: Option<Problem>,
    readable: bool,
    unreadable_means_broken: bool,
    duration_ms: Option<i64>,
) -> Option<Verdict> {
    match problem {
        Some(Problem::Empty | Problem::NoMoov) => Some(Verdict::Broken),
        Some(Problem::Truncated) => Some(Verdict::Truncated),
        None if !readable && unreadable_means_broken => Some(Verdict::Broken),
        None if duration_ms == Some(0) => Some(Verdict::Broken),
        None => None,
    }
}

/// The tag blocks as one JSON object, keyed by block type (`id3v2`, `ape`,
/// `vorbis_comments`, …), each an array of `{key, value}` items in file
/// order. A type that appears twice (stacked ID3v2 tags) has its items in
/// one array, first block first. An untagged file gives `{}`.
fn raw_tags_json(read: &TagRead) -> String {
    let mut blocks = Map::new();
    for tag in &read.raw_tags {
        let Ok(Value::String(kind)) = serde_json::to_value(tag.tag_type) else {
            continue;
        };
        let items = serde_json::to_value(&tag.items).unwrap_or(Value::Array(Vec::new()));
        let Value::Array(items) = items else { continue };
        if let Value::Array(all) = blocks
            .entry(kind)
            .or_insert_with(|| Value::Array(Vec::new()))
        {
            all.extend(items);
        }
    }
    Value::Object(blocks).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_come_from_the_sniffers_problem_then_readability_then_duration() {
        use Problem::{Empty, NoMoov};
        use Verdict::Broken;
        let cut = Some(Problem::Truncated);
        let truncated = Some(Verdict::Truncated);
        // (problem, readable, unreadable means broken, duration, verdict)
        for (problem, readable, counts, duration, expected) in [
            (Some(Empty), false, true, None, Some(Broken)),
            (Some(NoMoov), true, true, Some(1000), Some(Broken)),
            (cut, true, true, Some(1000), truncated),
            (cut, false, true, None, truncated),
            (cut, false, false, None, truncated),
            (None, false, true, None, Some(Broken)),
            // A container the tag reader doesn't read (RF64, WMA...).
            (None, false, false, None, None),
            (None, true, true, Some(0), Some(Broken)),
            (None, true, true, Some(1), None),
            // No duration: unknown (too long to believe, or a 0 that means
            // unknown for this format), not broken.
            (None, true, true, None, None),
        ] {
            assert_eq!(
                verdict(problem, readable, counts, duration),
                expected,
                "{problem:?} {readable} {counts} {duration:?}"
            );
        }
    }

    #[test]
    fn only_audio_counting_containers_read_a_zero_duration_as_no_audio() {
        use SniffedFormat as F;
        for format in [F::Wav, F::Rf64, F::Aiff, F::Aifc, F::Mp4] {
            assert!(zero_means_no_audio(format), "{format:?}");
        }
        for format in [F::Flac, F::Mp3, F::OggVorbis, F::OggOpus, F::WavPack] {
            assert!(!zero_means_no_audio(format), "{format:?}");
        }
    }

    #[test]
    fn the_tag_reader_failing_means_broken_only_for_containers_it_reads() {
        use SniffedFormat as F;
        assert!(unreadable_means_broken(F::Wav, Some(*b"RIFF")));
        assert!(!unreadable_means_broken(F::Wav, Some(*b"RIFX")));
        for format in [F::Rf64, F::Asf, F::Ogg] {
            assert!(!unreadable_means_broken(format, None), "{format:?}");
        }
        for format in [F::Mp3, F::Flac, F::Mp4, F::Aiff, F::OggVorbis, F::Unknown] {
            assert!(unreadable_means_broken(format, None), "{format:?}");
        }
    }
}
