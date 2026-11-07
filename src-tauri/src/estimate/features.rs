//! The energy features (ROADMAP 3.7; 3C-E1b), measured on the same window
//! as the tempo: how loud it is, how many onsets a second it has, and how
//! much its spectrum moves. They are raw numbers in the ledger; what the app
//! suggests for a track's energy is worked out from them across the Library
//! ([`super::suggest`]) and is only ever a suggestion.

use ebur128::{EbuR128, Mode};

/// What the window gave besides the tempo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Features {
    /// Integrated loudness in LUFS (EBU R128, gated), `None` when the meter
    /// couldn't take the audio (an odd channel count) or heard nothing.
    pub loudness_lufs: Option<f64>,
    /// Onsets a second.
    pub onset_density: f64,
    /// The mean of the spectral flux.
    pub spectral_flux: f64,
}

/// The least frames between two onsets, about 58 ms at 11 kHz.
const ONSET_GAP: usize = 5;
/// An onset stands this many times above the onset envelope's mean, and
/// rises by at least this share of the mean flux: a steady tone's flux
/// wavers a little around its level and has no onsets.
const ONSET_ABOVE_MEAN: f32 = 2.0;
const ONSET_RISE: f32 = 0.5;

/// The onsets a second of `env`, the onset envelope of the flux `raw` (both
/// with `fps` values a second): its local peaks, [`ONSET_GAP`] frames apart
/// at the least, that stand [`ONSET_ABOVE_MEAN`] times above its mean and
/// rise by [`ONSET_RISE`] of the mean flux.
pub fn onset_density(raw: &[f32], env: &[f32], fps: f64) -> f64 {
    if env.is_empty() {
        return 0.0;
    }
    let mean = env.iter().sum::<f32>() / env.len() as f32;
    let level = raw.iter().sum::<f32>() / raw.len() as f32;
    let floor = (mean * ONSET_ABOVE_MEAN).max(level * ONSET_RISE);
    let mut onsets = 0usize;
    for t in 0..env.len() {
        let v = env[t];
        if v <= floor || v <= 0.0 {
            continue;
        }
        let left = &env[t.saturating_sub(ONSET_GAP)..t];
        let right = &env[t + 1..(t + 1 + ONSET_GAP).min(env.len())];
        if left.iter().all(|&o| o < v) && right.iter().all(|&o| o <= v) {
            onsets += 1;
        }
    }
    onsets as f64 / (env.len() as f64 / fps)
}

/// The mean of `flux`.
pub fn mean_flux(flux: &[f32]) -> f64 {
    if flux.is_empty() {
        return 0.0;
    }
    flux.iter().map(|&f| f64::from(f)).sum::<f64>() / flux.len() as f64
}

/// A loudness meter over interleaved frames at the file's own rate (K
/// weighting is defined at 48 kHz and drifts when the audio is averaged
/// down). It gives up for good at the first thing it can't take.
pub struct Meter(Option<EbuR128>);

impl Meter {
    pub fn new(channels: usize, rate: u32) -> Meter {
        Meter(EbuR128::new(channels as u32, rate, Mode::I).ok())
    }

    /// Takes `interleaved` frames.
    pub fn add(&mut self, interleaved: &[f32]) {
        if let Some(meter) = &mut self.0 {
            if meter.add_frames_f32(interleaved).is_err() {
                self.0 = None;
            }
        }
    }

    /// The integrated loudness of what was taken, if any.
    pub fn lufs(&self) -> Option<f64> {
        let lufs = self.0.as_ref()?.loudness_global().ok()?;
        (lufs.is_finite() && lufs <= 0.0).then_some(lufs)
    }
}
