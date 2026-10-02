//! Decoding a file and fingerprinting it as it goes (ROADMAP 1.4, §5.6).
//!
//! Symphonia reads the container and codec from the bytes, never the
//! extension (§5.5). Each packet is decoded, downmixed to mono and handed
//! straight to rusty-chromaprint, so memory stays flat however long the
//! file is: a packet's samples, chromaprint's fixed buffers, and the
//! fingerprint itself (about 32 bytes per second of audio).
//!
//! Between packets it asks whether to keep going, so a cancel stops a long
//! file mid-decode.

use std::fmt;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rusty_chromaprint::Fingerprinter;
use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

use super::stored::{config, Fingerprint};

/// This many decode errors in a row and the file counts as damaged.
const MAX_ERRORS_IN_A_ROW: u32 = 64;

/// Why a file has no fingerprint. Stored as [`Unfingerprintable::reason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unfingerprintable {
    /// The bytes aren't a container Symphonia reads.
    UnsupportedFormat,
    /// The container holds a codec Symphonia can't decode, e.g. Opus.
    UnsupportedCodec,
    /// No audio track, or no audio in it.
    NoAudio,
    /// The audio is damaged beyond decoding.
    Damaged,
    /// Too little audio for even one fingerprint item (under ~3 s).
    TooShort,
    /// Reading the file failed (the OS reported an error, e.g. the drive
    /// went away). Never recorded against the file: it's tried again next
    /// time. Damaged audio is never this, even where Symphonia reports it
    /// as an I/O error.
    Unreadable,
    /// The decoder crashed on it. Caught; only this file is affected.
    DecoderCrashed,
}

impl Unfingerprintable {
    /// Every reason, for tests.
    pub const ALL: [Unfingerprintable; 7] = [
        Unfingerprintable::UnsupportedFormat,
        Unfingerprintable::UnsupportedCodec,
        Unfingerprintable::NoAudio,
        Unfingerprintable::Damaged,
        Unfingerprintable::TooShort,
        Unfingerprintable::Unreadable,
        Unfingerprintable::DecoderCrashed,
    ];

    /// The one whose [`reason`](Self::reason) is `reason`.
    pub fn parse(reason: &str) -> Option<Unfingerprintable> {
        Unfingerprintable::ALL
            .into_iter()
            .find(|why| why.reason() == reason)
    }

    /// The reason as stored.
    pub fn reason(self) -> &'static str {
        match self {
            Unfingerprintable::UnsupportedFormat => "unsupported_format",
            Unfingerprintable::UnsupportedCodec => "unsupported_codec",
            Unfingerprintable::NoAudio => "no_audio",
            Unfingerprintable::Damaged => "damaged",
            Unfingerprintable::TooShort => "too_short",
            Unfingerprintable::Unreadable => "unreadable",
            Unfingerprintable::DecoderCrashed => "decoder_crashed",
        }
    }
}

impl fmt::Display for Unfingerprintable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

/// Why fingerprinting a file stopped without a fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// `keep_going` said no.
    Cancelled,
    /// The file can't be fingerprinted.
    Failed(Unfingerprintable),
}

impl From<Unfingerprintable> for Stopped {
    fn from(why: Unfingerprintable) -> Self {
        Stopped::Failed(why)
    }
}

/// Decodes `source` and fingerprints the whole track.
///
/// `keep_going` is called before every packet with how far through the
/// track the decode is (0 to 1, when the length is known); returning
/// false stops at once with [`Stopped::Cancelled`].
pub fn fingerprint(
    source: Box<dyn MediaSource>,
    keep_going: &mut dyn FnMut(Option<f64>) -> bool,
) -> Result<Fingerprint, Stopped> {
    let config = config();
    let mut printer: Option<(Fingerprinter, u32)> = None;
    let (mut samples, mut mono) = (Vec::<i16>::new(), Vec::<i16>::new());
    let mut decoded_any = false;
    let walked = walk(source, keep_going, &mut |block| {
        block.audio.copy_to_vec_interleaved(&mut samples);
        if printer.is_none() {
            let mut fp = Fingerprinter::new(&config);
            // Only a rate of 1 kHz or less is refused.
            fp.start(block.rate, 1)
                .map_err(|_| Unfingerprintable::UnsupportedCodec)?;
            printer = Some((fp, block.rate));
        }
        let Some((fp, started_at)) = printer.as_mut() else {
            return Ok(Flow::Continue);
        };
        // A rate change mid-stream would skew every item after it; the
        // track up to there is still a fingerprint.
        if block.rate != *started_at {
            return Ok(Flow::Stop);
        }
        downmix(&samples, block.channels, &mut mono);
        fp.consume(&mono);
        decoded_any = true;
        Ok(Flow::Continue)
    })?;
    if walked.end == End::GaveUp {
        return Err(Unfingerprintable::Damaged.into());
    }
    let Some((mut fp, _)) = printer.filter(|_| decoded_any) else {
        return Err(Unfingerprintable::NoAudio.into());
    };
    fp.finish();
    let items = fp.fingerprint().to_vec();
    if items.is_empty() {
        return Err(Unfingerprintable::TooShort.into());
    }
    Ok(Fingerprint::new(items))
}

/// One decoded packet's audio, handed to [`walk`]'s `on_block`.
pub struct Block<'a> {
    /// Its sample rate and channel count.
    pub rate: u32,
    pub channels: usize,
    /// The samples, for the caller to copy out in whatever sample type it
    /// needs (`copy_to_vec_interleaved`).
    pub audio: &'a GenericAudioBufferRef<'a>,
}

/// What `on_block` tells [`walk`] to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// Stop reading here; what was read so far stands.
    Stop,
}

/// How a [`walk`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The stream ran to its end (or a new chained stream began).
    Finished,
    /// The file ended in the middle of the stream.
    CutShort,
    /// [`MAX_ERRORS_IN_A_ROW`] packets in a row were damaged, so the rest
    /// wasn't read.
    GaveUp,
    /// `on_block` said [`Flow::Stop`].
    Stopped,
}

/// The packets that failed, by kind: ones the decoder refused, and ones the
/// container couldn't produce.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ErrorCounts {
    pub decode: u32,
    pub container: u32,
}

/// What a [`walk`] learned about the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walked {
    /// The sample rate of the first block; 0 if nothing was decoded.
    pub rate: u32,
    /// Frames decoded.
    pub frames: u64,
    /// The length the container's header claims, in frames, and the rate it
    /// gives, if it gives them.
    pub header_frames: Option<u64>,
    pub header_rate: Option<u32>,
    pub errors: ErrorCounts,
    pub end: End,
}

impl Walked {
    /// The duration the header claims, in seconds, if it gives one.
    pub fn header_seconds(&self) -> Option<f64> {
        let (frames, rate) = (self.header_frames?, self.header_rate?);
        (rate > 0).then(|| frames as f64 / f64::from(rate))
    }
}

/// Reads `source` packet by packet and hands each decoded block to
/// `on_block`: the one decoding loop fingerprints and quality measurements
/// share (§5.6).
///
/// `keep_going` is called before every packet with how far through the
/// track the decode is (0 to 1, when the length is known); returning false
/// stops at once with [`Stopped::Cancelled`]. A packet that fails to
/// decode is counted and skipped; [`MAX_ERRORS_IN_A_ROW`] of them in a row
/// end the walk as [`End::GaveUp`], which is for the caller to judge.
/// `on_block` can fail the file (e.g. a codec it can't use).
pub fn walk(
    source: Box<dyn MediaSource>,
    keep_going: &mut dyn FnMut(Option<f64>) -> bool,
    on_block: &mut dyn FnMut(&Block<'_>) -> Result<Flow, Unfingerprintable>,
) -> Result<Walked, Stopped> {
    // Symphonia reports damaged audio as I/O errors too (e.g. "unexpected
    // end of bitstream"), so only the source itself can say the OS failed.
    let os_failed = Arc::new(AtomicBool::new(false));
    let source = Watched {
        inner: source,
        failed: os_failed.clone(),
    };
    let unreadable = || os_failed.load(Ordering::SeqCst);
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    // No hint: the format comes from the bytes alone (§5.5).
    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| match e {
            _ if unreadable() => Unfingerprintable::Unreadable,
            SymphoniaError::Unsupported(_) => Unfingerprintable::UnsupportedFormat,
            _ => Unfingerprintable::Damaged,
        })?;
    let track = format
        .first_track(TrackType::Audio)
        .ok_or(Unfingerprintable::NoAudio)?;
    let (track_id, header_frames) = (track.id, track.num_frames);
    let total_frames = header_frames.filter(|&n| n > 0);
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
        return Err(Unfingerprintable::NoAudio.into());
    };
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|_| Unfingerprintable::UnsupportedCodec)?;

    let mut walked = Walked {
        rate: 0,
        frames: 0,
        header_frames,
        header_rate: params.sample_rate,
        errors: ErrorCounts::default(),
        end: End::Finished,
    };
    let mut errors_in_a_row = 0u32;
    loop {
        let fraction = total_frames.map(|n| (walked.frames as f64 / n as f64).min(1.0));
        if !keep_going(fraction) {
            return Err(Stopped::Cancelled);
        }
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) if unreadable() => return Err(Unfingerprintable::Unreadable.into()),
            // A file cut short: what's there is what there is.
            Err(SymphoniaError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => {
                walked.end = End::CutShort;
                break;
            }
            // A new chained stream: the track so far is the track.
            Err(SymphoniaError::ResetRequired) => break,
            Err(_) => {
                walked.errors.container += 1;
                errors_in_a_row += 1;
                if errors_in_a_row >= MAX_ERRORS_IN_A_ROW {
                    walked.end = End::GaveUp;
                    break;
                }
                continue;
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        // The packet is already in memory: every error from here on is about
        // its content, so it counts toward giving up, never as unreadable.
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(_) => {
                walked.errors.decode += 1;
                errors_in_a_row += 1;
                if errors_in_a_row >= MAX_ERRORS_IN_A_ROW {
                    walked.end = End::GaveUp;
                    break;
                }
                continue;
            }
        };
        errors_in_a_row = 0;
        let rate = decoded.spec().rate();
        let channels = decoded.spec().channels().count().max(1);
        walked.frames += decoded.frames() as u64;
        if decoded.frames() == 0 {
            continue;
        }
        if walked.rate == 0 {
            walked.rate = rate;
        }
        let block = Block {
            rate,
            channels,
            audio: &decoded,
        };
        if on_block(&block)? == Flow::Stop {
            walked.end = End::Stopped;
            break;
        }
    }
    Ok(walked)
}

/// Averages interleaved frames into one channel, as chromaprint would.
/// Downmixing here means chromaprint only ever sees whole mono frames, even
/// if a stream changes its channel count.
fn downmix(interleaved: &[i16], channels: usize, mono: &mut Vec<i16>) {
    mono.clear();
    if channels == 1 {
        mono.extend_from_slice(interleaved);
        return;
    }
    mono.extend(interleaved.chunks_exact(channels).map(|frame| {
        let sum: i32 = frame.iter().map(|&s| i32::from(s)).sum();
        (sum / channels as i32) as i16
    }));
}

/// A source that remembers whether the OS ever failed a read or seek.
struct Watched {
    inner: Box<dyn MediaSource>,
    failed: Arc<AtomicBool>,
}

/// Read errors that aren't the OS failing: a retry.
const NOT_A_READ_FAILURE: &[ErrorKind] = &[ErrorKind::Interrupted];
/// Seek errors that aren't the OS failing: a retry, or a seek to a bad
/// offset. (A read's `InvalidInput` can be Windows' ERROR_INVALID_PARAMETER
/// from a failing device, so it counts there.)
const NOT_A_SEEK_FAILURE: &[ErrorKind] = &[ErrorKind::Interrupted, ErrorKind::InvalidInput];

impl Watched {
    fn watch<T>(&self, result: io::Result<T>, ignored: &[ErrorKind]) -> io::Result<T> {
        if let Err(e) = &result {
            if !ignored.contains(&e.kind()) {
                self.failed.store(true, Ordering::SeqCst);
            }
        }
        result
    }
}

impl Read for Watched {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let result = self.inner.read(buf);
        self.watch(result, NOT_A_READ_FAILURE)
    }
}

impl Seek for Watched {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let result = self.inner.seek(to);
        self.watch(result, NOT_A_SEEK_FAILURE)
    }
}

impl MediaSource for Watched {
    fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner.byte_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source whose every read and seek fails with the given kinds.
    struct Failing {
        read: ErrorKind,
        seek: ErrorKind,
    }

    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(self.read))
        }
    }

    impl Seek for Failing {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            Err(io::Error::from(self.seek))
        }
    }

    impl MediaSource for Failing {
        fn is_seekable(&self) -> bool {
            true
        }

        fn byte_len(&self) -> Option<u64> {
            None
        }
    }

    /// Whether one read (or one seek) failing with `kind` marks the source
    /// as failed by the OS.
    fn flags(kind: ErrorKind, seek: bool) -> bool {
        let failed = Arc::new(AtomicBool::new(false));
        let mut source = Watched {
            inner: Box::new(Failing {
                read: kind,
                seek: kind,
            }),
            failed: failed.clone(),
        };
        if seek {
            let _ = source.seek(SeekFrom::Start(0));
        } else {
            let _ = source.read(&mut [0; 4]);
        }
        failed.load(Ordering::SeqCst)
    }

    #[test]
    fn a_read_failing_with_invalid_input_counts_as_the_os_failing_but_a_seek_does_not() {
        // Windows reports some device failures as ERROR_INVALID_PARAMETER.
        assert!(flags(ErrorKind::InvalidInput, false));
        assert!(!flags(ErrorKind::InvalidInput, true));
        // A retry is never a failure; anything else always is.
        assert!(!flags(ErrorKind::Interrupted, false));
        assert!(!flags(ErrorKind::Interrupted, true));
        assert!(flags(ErrorKind::Other, false));
        assert!(flags(ErrorKind::Other, true));
        assert!(flags(ErrorKind::PermissionDenied, false));
    }
}
