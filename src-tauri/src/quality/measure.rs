//! One decode pass that gives every quality measurement of a file:
//!
//! - the spectral cutoff ([`super::cutoff`]),
//! - the duration actually decoded, and the duration the header claims,
//! - the packets that failed to decode, counted by kind, and how the
//!   stream ended.
//!
//! It walks the file with [`crate::fingerprint::decode::walk`], the same
//! loop fingerprints are made with, so what counts as a damaged packet or
//! a file cut short is decided in one place. It never gives up on a file
//! for its errors: a damaged file is measured as far as it decodes, and the
//! errors are the finding.

use std::fmt;

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::io::MediaSource;

use super::cutoff::{Gap, Meter};
use crate::fingerprint::decode::{self, End, Flow, Stopped, Unfingerprintable};

/// How the decoded stream ended. Stored as `file_quality.ended`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// It ran to its end.
    Complete,
    /// The file stopped in the middle of the stream (it was cut off, e.g.
    /// by an interrupted copy).
    CutShort,
    /// Too many packets in a row were damaged; the rest wasn't read.
    GaveUp,
}

impl Ended {
    pub fn as_str(self) -> &'static str {
        match self {
            Ended::Complete => "complete",
            Ended::CutShort => "cut_short",
            Ended::GaveUp => "gave_up",
        }
    }
}

/// Why a file has no measurements: its content can't be decoded at all.
/// Stored as `file_quality.failure`; the same reasons as a file without a
/// fingerprint, bar the ones that aren't about content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    UnsupportedFormat,
    UnsupportedCodec,
    NoAudio,
    Damaged,
    DecoderCrashed,
}

impl Failure {
    pub fn as_str(self) -> &'static str {
        match self {
            Failure::UnsupportedFormat => "unsupported_format",
            Failure::UnsupportedCodec => "unsupported_codec",
            Failure::NoAudio => "no_audio",
            Failure::Damaged => "damaged",
            Failure::DecoderCrashed => "decoder_crashed",
        }
    }

    /// The failure for a file that couldn't be decoded, or `None` if it
    /// wasn't about the content: the OS failing to read it, which is tried
    /// again, never recorded.
    pub fn of(why: Unfingerprintable) -> Option<Failure> {
        match why {
            Unfingerprintable::UnsupportedFormat => Some(Failure::UnsupportedFormat),
            Unfingerprintable::UnsupportedCodec => Some(Failure::UnsupportedCodec),
            // Too little audio for a fingerprint is no problem here: the
            // file decoded, and the cutoff says it's too short.
            Unfingerprintable::NoAudio | Unfingerprintable::TooShort => Some(Failure::NoAudio),
            Unfingerprintable::Damaged => Some(Failure::Damaged),
            Unfingerprintable::DecoderCrashed => Some(Failure::DecoderCrashed),
            Unfingerprintable::Unreadable => None,
        }
    }
}

/// How many packets failed to decode, by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Errors {
    /// The decoder refused a packet (a damaged frame).
    pub decode: u32,
    /// The container couldn't produce a packet (a damaged stream between
    /// frames).
    pub container: u32,
}

impl Errors {
    pub fn total(self) -> i64 {
        i64::from(self.decode) + i64::from(self.container)
    }

    /// The counts as stored in `file_quality.error_kinds`: a JSON object
    /// with a key for each kind that happened, `None` if none did.
    pub fn kinds_json(self) -> Option<String> {
        let kinds: serde_json::Map<String, serde_json::Value> =
            [("decode", self.decode), ("container", self.container)]
                .into_iter()
                .filter(|&(_, n)| n > 0)
                .map(|(kind, n)| (kind.to_owned(), n.into()))
                .collect();
        (!kinds.is_empty()).then(|| serde_json::Value::Object(kinds).to_string())
    }
}

/// What measuring a file found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measured {
    /// The spectral cutoff in Hz, or why it couldn't be measured.
    pub cutoff: Result<u32, Gap>,
    /// The audio actually decoded, in milliseconds.
    pub decoded_ms: i64,
    /// The duration the header claims, if it gives one.
    pub header_ms: Option<i64>,
    pub errors: Errors,
    pub ended: Ended,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Decodes `source` once and measures it.
///
/// `keep_going` is called before every packet with how far through the
/// track the decode is (0 to 1, when the length is known); returning false
/// stops at once with [`Stopped::Cancelled`] and measures nothing.
pub fn measure(
    source: Box<dyn MediaSource>,
    keep_going: &mut dyn FnMut(Option<f64>) -> bool,
) -> Result<Measured, Stopped> {
    let mut meter: Option<Meter> = None;
    let mut started_at = 0u32;
    let (mut samples, mut mono) = (Vec::<f32>::new(), Vec::<f32>::new());
    let walked = decode::walk(source, keep_going, &mut |block| {
        if started_at == 0 {
            started_at = block.rate;
            meter = Some(Meter::new(block.rate));
        } else if block.rate != started_at {
            // The rest of the file is another stream: the measurements are
            // of the first.
            return Ok(Flow::Stop);
        }
        feed(
            block.audio,
            block.channels,
            &mut samples,
            &mut mono,
            &mut meter,
        );
        Ok(Flow::Continue)
    })?;
    let rate = f64::from(walked.rate.max(1));
    let ms = |frames: u64| (frames as f64 / rate * 1000.0).round() as i64;
    let Some(meter) = meter else {
        return Err(Unfingerprintable::NoAudio.into());
    };
    Ok(Measured {
        cutoff: meter.finish(),
        decoded_ms: ms(walked.frames),
        header_ms: walked.header_seconds().map(|s| (s * 1000.0).round() as i64),
        errors: Errors {
            decode: walked.errors.decode,
            container: walked.errors.container,
        },
        ended: match walked.end {
            End::CutShort => Ended::CutShort,
            End::GaveUp => Ended::GaveUp,
            End::Finished | End::Stopped => Ended::Complete,
        },
    })
}

/// Mixes the block down to mono (the average of its channels) and gives it
/// to the meter.
fn feed(
    audio: &GenericAudioBufferRef<'_>,
    channels: usize,
    samples: &mut Vec<f32>,
    mono: &mut Vec<f32>,
    meter: &mut Option<Meter>,
) {
    audio.copy_to_vec_interleaved(samples);
    mono.clear();
    if channels == 1 {
        mono.extend_from_slice(samples);
    } else {
        mono.extend(
            samples
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
    }
    if let Some(meter) = meter {
        meter.feed(mono);
    }
}
