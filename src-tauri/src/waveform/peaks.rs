//! Reducing a decoded track to a short array of peaks, in one pass and in
//! memory that doesn't grow with the track.
//!
//! The track's length isn't known before it is decoded (a header can leave
//! it out or get it wrong), so the buckets can't be sized up front.
//! [`Accumulator`] keeps a few small slots of a fixed number of frames; when
//! the slots are full it merges them in pairs and doubles the number of
//! frames a slot covers. At the end the slots are merged down to at most
//! [`BUCKETS`]. Two merges of a max and of sums of squares are exact, so the
//! result doesn't depend on how the decoder cut the track into packets.

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::io::MediaSource;

use crate::fingerprint::decode::{self, End, Flow, Stopped, Unfingerprintable};

/// The most buckets a track's waveform has. A short track has fewer (never
/// fewer than one per [`FIRST_SPAN`] frames). Each bucket is two bytes, so a
/// waveform is at most 2 KiB.
pub const BUCKETS: usize = 1024;

/// How many slots are held before they are merged in pairs.
const HELD: usize = 2 * BUCKETS;

/// How many frames a slot covers to begin with (6 ms at 44.1 kHz).
const FIRST_SPAN: u64 = 256;

/// What one slot, and then one bucket, has seen.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Slot {
    /// The largest absolute sample, over every channel.
    peak: f32,
    /// The sum of every sample squared, and how many samples that is.
    squares: f64,
    samples: u64,
}

impl Slot {
    fn merge(self, other: Slot) -> Slot {
        Slot {
            peak: self.peak.max(other.peak),
            squares: self.squares + other.squares,
            samples: self.samples + other.samples,
        }
    }

    /// The bucket as two bytes: the peak and the RMS level, each 0 to 255
    /// for 0 to full scale.
    fn bytes(self) -> [u8; 2] {
        let rms = if self.samples == 0 {
            0.0
        } else {
            (self.squares / self.samples as f64).sqrt() as f32
        };
        [level(self.peak), level(rms)]
    }
}

/// `v` (0 to 1 is silence to full scale) as a byte. A sample past full
/// scale (a float file) counts as full scale.
fn level(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Takes a track's samples in order and gives its peaks.
pub struct Accumulator {
    /// Frames per slot.
    span: u64,
    done: Vec<Slot>,
    current: Slot,
    /// Frames in `current`.
    in_current: u64,
}

impl Default for Accumulator {
    fn default() -> Self {
        Accumulator {
            span: FIRST_SPAN,
            done: Vec::with_capacity(HELD),
            current: Slot::default(),
            in_current: 0,
        }
    }
}

impl Accumulator {
    /// Takes interleaved `samples` of `channels` channels (a trailing
    /// partial frame is dropped).
    pub fn feed(&mut self, samples: &[f32], channels: usize) {
        for frame in samples.chunks_exact(channels.max(1)) {
            for &s in frame {
                let s = if s.is_nan() { 0.0 } else { s.abs() };
                self.current.peak = self.current.peak.max(s);
                self.current.squares += f64::from(s) * f64::from(s);
            }
            self.current.samples += frame.len() as u64;
            self.in_current += 1;
            if self.in_current == self.span {
                self.close();
            }
        }
    }

    fn close(&mut self) {
        self.done.push(std::mem::take(&mut self.current));
        self.in_current = 0;
        if self.done.len() == HELD {
            self.done = self
                .done
                .chunks(2)
                .map(|pair| pair.iter().copied().fold(Slot::default(), Slot::merge))
                .collect();
            self.span *= 2;
        }
    }

    /// The peaks: for each bucket, its peak then its RMS level. `None` for
    /// a track with no samples.
    pub fn finish(mut self) -> Option<Vec<u8>> {
        if self.in_current > 0 {
            self.done.push(self.current);
        }
        let n = self.done.len();
        if n == 0 {
            return None;
        }
        let buckets = n.min(BUCKETS);
        let mut out = Vec::with_capacity(2 * buckets);
        for i in 0..buckets {
            // Bucket i takes slots [i*n/buckets, (i+1)*n/buckets): every
            // slot goes into exactly one bucket, and none is empty.
            let (from, to) = (i * n / buckets, (i + 1) * n / buckets);
            let bucket = self.done[from..to]
                .iter()
                .copied()
                .fold(Slot::default(), Slot::merge);
            out.extend_from_slice(&bucket.bytes());
        }
        Some(out)
    }
}

/// A track's peaks and length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Computed {
    /// Peak then RMS level per bucket, bucket after bucket.
    pub peaks: Vec<u8>,
    /// The audio actually decoded, in milliseconds.
    pub duration_ms: i64,
}

/// Decodes `source` once and reduces it to peaks, walking it as the
/// fingerprints and the quality checks do ([`decode::walk`]), so what counts
/// as a damaged packet is decided in one place. A damaged file gets the
/// peaks of what decodes.
///
/// `keep_going` is called before every packet; returning false stops at once
/// with [`Stopped::Cancelled`].
pub fn compute(
    source: Box<dyn MediaSource>,
    keep_going: &mut dyn FnMut(Option<f64>) -> bool,
) -> Result<Computed, Stopped> {
    let mut accumulator = Accumulator::default();
    let mut started_at = 0u32;
    let mut samples = Vec::<f32>::new();
    let walked = decode::walk(source, keep_going, &mut |block| {
        if started_at == 0 {
            started_at = block.rate;
        } else if block.rate != started_at {
            // The rest of the file is another stream: the waveform is of
            // the first.
            return Ok(Flow::Stop);
        }
        take(block.audio, block.channels, &mut samples, &mut accumulator);
        Ok(Flow::Continue)
    })?;
    let Some(peaks) = accumulator.finish() else {
        // Nothing decoded: the same two words as for a fingerprint.
        let why = if walked.end == End::GaveUp {
            Unfingerprintable::Damaged
        } else {
            Unfingerprintable::NoAudio
        };
        return Err(why.into());
    };
    let rate = f64::from(walked.rate.max(1));
    Ok(Computed {
        peaks,
        duration_ms: (walked.frames as f64 / rate * 1000.0).round() as i64,
    })
}

fn take(
    audio: &GenericAudioBufferRef<'_>,
    channels: usize,
    samples: &mut Vec<f32>,
    accumulator: &mut Accumulator,
) {
    audio.copy_to_vec_interleaved(samples);
    accumulator.feed(samples, channels);
}
