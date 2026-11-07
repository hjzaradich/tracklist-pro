//! Decoding the window a file is measured on, and measuring it.
//!
//! The window is the spike's (`spikes/audio-lab`, `bpmkey`): the first
//! [`UP_TO`] seconds are decoded and no more, and when that came to more
//! than [`SKIP_IF_OVER`] seconds the first [`SKIP`] are dropped, so an intro
//! without drums doesn't count. The decode is [`decode::walk`], the loop
//! the fingerprints, quality checks and waveforms share, so what counts as
//! a damaged packet is decided in one place; it stops as soon as the window
//! is full.
//!
//! Memory is bounded: the window is kept as mono averaged down to about
//! 11 kHz ([`bpm::TARGET_RATE`]), at most 150 s of it, about 6.6 MB; and
//! for the key, filtered and kept at about 4.4 kHz ([`key::Lowpass`]),
//! about 5.3 MB more.
//!
//! The energy features ([`features`]) are measured on the same window: the
//! onset density and mean flux on the averaged-down audio, the loudness on
//! the file's own samples, which [`Window::feed`] also hands to two meters,
//! one from the start and one from the [`SKIP`], since whether the intro is
//! dropped is known only when the window is full.

use symphonia::core::io::MediaSource;

use super::bpm::{self, Tempo};
use super::features::{self, Features, Meter};
use super::key::{self, Lowpass};
use crate::fingerprint::decode::{self, End, Flow, Stopped, Unfingerprintable};

/// How much of a file is decoded, in seconds from its start.
pub const UP_TO: f64 = 150.0;
/// The intro dropped from a file that decoded to more than [`SKIP_IF_OVER`]
/// seconds.
pub const SKIP: f64 = 30.0;
pub const SKIP_IF_OVER: f64 = 90.0;
/// The least audio worth measuring, in seconds: a comb of eight beats at
/// 60 BPM needs eight.
pub const MIN_SECONDS: f64 = 10.0;
/// The peak under which a window counts as silence (about -80 dBFS).
const SILENCE: f32 = 1e-4;

/// Why a file has no measurements, beyond the content not decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Short {
    /// Less than [`MIN_SECONDS`] of audio.
    TooShort,
    /// Nothing but silence.
    Silent,
}

/// What a window gave.
#[derive(Debug, Clone, PartialEq)]
pub enum Measured {
    /// The window was measured: `tempo` is `None` when no beat stood out.
    Window {
        analysed_ms: i64,
        tempo: Option<Tempo>,
        /// `None` when the file's sample rate is too low for the key's
        /// bands.
        key: Option<key::Choice>,
        features: Features,
    },
    /// There was audio, but none worth measuring.
    Short(Short),
}

/// The window being filled: mono, averaged down by `factor` samples, and
/// filtered for the key.
#[derive(Default)]
struct Window {
    key: Option<Lowpass>,
    rate: u32,
    channels: usize,
    factor: usize,
    /// Frames to take from the file before the window is full.
    wanted: u64,
    taken: u64,
    mono: Vec<f32>,
    sum: f32,
    in_sum: usize,
    interleaved: Vec<f32>,
    /// Loudness of the whole window, and of what is left when the intro is
    /// dropped.
    whole: Option<Meter>,
    after_skip: Option<Meter>,
}

impl Window {
    fn start(rate: u32, channels: usize) -> Window {
        let channels = channels.max(1);
        Window {
            rate,
            channels,
            whole: Some(Meter::new(channels, rate)),
            after_skip: Some(Meter::new(channels, rate)),
            factor: bpm::decimation(rate),
            key: Lowpass::new(rate),
            wanted: (UP_TO * f64::from(rate)) as u64,
            ..Window::default()
        }
    }

    /// Adds a packet's frames, as many as the window still wants. Returns
    /// whether it's full.
    fn feed(&mut self) -> bool {
        let channels = self.channels;
        self.meter(channels);
        let mut frames = self.interleaved.chunks_exact(channels);
        for frame in frames.by_ref() {
            if self.taken >= self.wanted {
                break;
            }
            self.taken += 1;
            if let Some(key) = &mut self.key {
                key.push(frame.iter().map(|&s| f64::from(s)).sum::<f64>() / channels as f64);
            }
            self.sum += frame.iter().sum::<f32>() / channels as f32;
            self.in_sum += 1;
            if self.in_sum == self.factor {
                self.mono.push(self.sum / self.factor as f32);
                self.sum = 0.0;
                self.in_sum = 0;
            }
        }
        self.taken >= self.wanted
    }

    /// Hands the frames of the packet that the window still wants to the
    /// loudness meters.
    fn meter(&mut self, channels: usize) {
        let room = self.wanted.saturating_sub(self.taken) as usize;
        let frames = (self.interleaved.len() / channels).min(room);
        let samples = &self.interleaved[..frames * channels];
        if let Some(meter) = &mut self.whole {
            meter.add(samples);
        }
        let skipped = (SKIP * f64::from(self.rate)) as u64;
        let past = (self.taken + frames as u64)
            .saturating_sub(skipped)
            .min(frames as u64) as usize;
        if let Some(meter) = &mut self.after_skip {
            meter.add(&samples[(frames - past) * channels..]);
        }
    }

    /// The measurements, once the walk is over.
    fn measure(mut self) -> Measured {
        if self.in_sum > 0 {
            self.mono.push(self.sum / self.in_sum as f32);
        }
        let rate = f64::from(self.rate) / self.factor as f64;
        let seconds = self.mono.len() as f64 / rate;
        let skip = seconds > SKIP_IF_OVER;
        let from = if skip { (SKIP * rate) as usize } else { 0 };
        let window = &self.mono[from..];
        let analysed = window.len() as f64 / rate;
        if analysed < MIN_SECONDS {
            return Measured::Short(Short::TooShort);
        }
        if window.iter().all(|s| s.abs() < SILENCE) {
            return Measured::Short(Short::Silent);
        }
        let file_rate = self.rate;
        let key = self.key.take().and_then(|lowpass| {
            let factor = lowpass.factor();
            let filtered = lowpass.finish();
            // The same intro dropped: every output from before it.
            let from = if skip {
                (SKIP * f64::from(file_rate) / factor as f64).ceil() as usize
            } else {
                0
            };
            key::of_filtered(
                filtered.get(from..).unwrap_or_default(),
                file_rate / factor as u32,
            )
        });
        let raw = bpm::flux(window, bpm::FRAME, bpm::HOP);
        let env = bpm::detrend(&raw);
        let loudness = if skip { &self.after_skip } else { &self.whole };
        Measured::Window {
            analysed_ms: (analysed * 1000.0).round() as i64,
            tempo: bpm::detect_envelope(&env, rate),
            key,
            features: Features {
                loudness_lufs: loudness.as_ref().and_then(Meter::lufs),
                onset_density: features::onset_density(&raw, &env, rate / bpm::HOP as f64),
                spectral_flux: features::mean_flux(&raw),
            },
        }
    }
}

/// Decodes `source`'s window and measures it.
///
/// `keep_going` is called before every packet; returning false stops at once
/// with [`Stopped::Cancelled`]. A file that decodes to nothing fails as a
/// fingerprint would (no audio, or damaged).
pub fn measure(
    source: Box<dyn MediaSource>,
    keep_going: &mut dyn FnMut(Option<f64>) -> bool,
) -> Result<Measured, Stopped> {
    let mut window: Option<Window> = None;
    let walked = decode::walk(source, keep_going, &mut |block| {
        let w = window.get_or_insert_with(|| Window::start(block.rate, block.channels));
        if block.rate != w.rate || block.channels.max(1) != w.channels {
            // The rest of the file is another stream (another rate or channel
            // count): the window is of the first.
            return Ok(Flow::Stop);
        }
        block.audio.copy_to_vec_interleaved(&mut w.interleaved);
        Ok(if w.feed() { Flow::Stop } else { Flow::Continue })
    })?;
    match window {
        Some(w) if w.taken > 0 => Ok(w.measure()),
        _ => Err(if walked.end == End::GaveUp {
            Unfingerprintable::Damaged
        } else {
            Unfingerprintable::NoAudio
        }
        .into()),
    }
}
