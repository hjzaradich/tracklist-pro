//! The tempo detector (ROADMAP 3.7, "BPM"), ported from the Phase 0 audio
//! spike (`spikes/audio-lab`, `estimate_bpm`), which measured it on the
//! owner's analysed tracks:
//!
//! 1. **Onsets by spectral flux** ([`onset_envelope`]): mono audio at about
//!    11 kHz, cut into Hann-windowed frames of [`FRAME`] samples every
//!    [`HOP`]; for each frame, how much the log magnitude spectrum rose
//!    since the frame before, summed over the bins. A local mean over about
//!    a second either side is taken away and what is left under zero is
//!    dropped, so only the onsets stand out.
//! 2. **Comb autocorrelation** over [`SEARCH_MIN`] to [`SEARCH_MAX`] BPM in
//!    half-BPM steps: a tempo's score is the envelope's autocorrelation at
//!    one to [`COMB`] beat periods, summed, so a tempo whose beats line up
//!    for bars on end wins over one that lines up once.
//! 3. **Fine search** half a BPM either side of the best, in 0.02 steps.
//! 4. **Folding** ([`fold`]) into the user's BPM range (the setting
//!    `bpm_range`, rekordbox's 98 to 195 by default): doubled while under
//!    it, then halved while over it, so a detector that locked onto half or
//!    double the beat still gives the tempo a DJ reads. The ledger keeps the
//!    detector's own answer; only the shown value is folded, so a change of
//!    range needs no decode.
//!
//! What the spike didn't have: a **confidence**, 0 to 1, from how far the
//! best score stands above the average one, and "no beat" (`None`) for audio
//! whose envelope has no onsets at all.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use crate::settings::BpmRange;

/// The sample rate the detector wants: the audio is averaged down to about
/// this (an integer factor of its own rate).
pub const TARGET_RATE: u32 = 11_025;

/// Samples per frame of the onset envelope, and how far apart frames start.
pub const FRAME: usize = 1024;
pub const HOP: usize = 128;

/// The tempos the comb searches, in BPM, before folding.
pub const SEARCH_MIN: f64 = 60.0;
pub const SEARCH_MAX: f64 = 200.0;

/// How many beat periods a tempo's score sums.
const COMB: usize = 8;

/// The local mean taken away from the envelope, in frames either side
/// (about half a second at 11 kHz).
const TREND: usize = 40;

/// What the detector found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tempo {
    /// The best tempo in the search range, unfolded.
    pub bpm: f64,
    /// How far the best score stands above the average one: 0 (no tempo
    /// stands out) to 1.
    pub confidence: f64,
}

/// The factor `rate` is averaged down by to get near [`TARGET_RATE`].
pub fn decimation(rate: u32) -> usize {
    (rate / TARGET_RATE).max(1) as usize
}

/// The tempo of `mono`, audio at `rate` samples a second that is already
/// averaged down near [`TARGET_RATE`]. `None` when the envelope has no
/// onsets (silence, a steady tone) or the audio is too short for a comb.
pub fn detect(mono: &[f32], rate: f64) -> Option<Tempo> {
    detect_envelope(&onset_envelope(mono, FRAME, HOP), rate)
}

/// [`detect`] on an onset envelope already worked out from audio at `rate`
/// ([`onset_envelope`], or [`flux`] then [`detrend`]), so a caller that
/// wants the envelope for something else too computes it once.
pub fn detect_envelope(env: &[f32], rate: f64) -> Option<Tempo> {
    let fps = rate / HOP as f64;
    // The longest comb reaches COMB periods of the slowest tempo.
    let longest = COMB as f64 * 60.0 * fps / SEARCH_MIN;
    if env.len() as f64 <= longest + 2.0 {
        return None;
    }
    let score = |bpm: f64| -> f64 {
        let period = 60.0 * fps / bpm;
        (1..=COMB).map(|k| acf_at(env, period * k as f64)).sum()
    };
    let mut best = (0.0, f64::MIN);
    let (mut sum, mut count) = (0.0, 0usize);
    let steps = ((SEARCH_MAX - SEARCH_MIN) / 0.5).round() as usize;
    for step in 0..=steps {
        let bpm = SEARCH_MIN + step as f64 * 0.5;
        let s = score(bpm);
        sum += s;
        count += 1;
        if s > best.1 {
            best = (bpm, s);
        }
    }
    // Silence and a steady tone score nothing; infinite samples score
    // infinity, and a confidence of infinity over infinity is no number.
    if best.1 <= f64::EPSILON || !best.1.is_finite() {
        return None;
    }
    let mut fine = best;
    for step in 0..=50 {
        let bpm = best.0 - 0.5 + step as f64 * 0.02;
        let s = score(bpm);
        if s > fine.1 {
            fine = (bpm, s);
        }
    }
    let mean = sum / count as f64;
    let confidence = ((best.1 - mean) / best.1).clamp(0.0, 1.0);
    Some(Tempo {
        bpm: fine.0,
        confidence,
    })
}

/// `bpm` doubled while it's under the range, then halved while it's over
/// the top of it (and the half is not under the bottom: a range narrower
/// than an octave has a band just over its top that no halving can bring
/// in, which stays as it is): the tempo a DJ reads, for a detector that may have
/// locked onto half or double the beat. Rounded to hundredths, as rekordbox
/// shows a BPM.
pub fn fold(bpm: f64, range: BpmRange) -> f64 {
    let mut v = bpm;
    if v > 0.0 && v.is_finite() {
        while v < range.min {
            v *= 2.0;
        }
        while v > range.max && v / 2.0 >= range.min {
            v /= 2.0;
        }
    }
    (v * 100.0).round() / 100.0
}

/// The spectral-flux onset envelope of `x`, one value per `hop` samples.
pub fn onset_envelope(x: &[f32], frame: usize, hop: usize) -> Vec<f32> {
    detrend(&flux(x, frame, hop))
}

/// The spectral flux of `x`: for each frame, how much the log magnitude
/// spectrum rose since the frame before, summed over the bins; one value per
/// `hop` samples. The energy features' mean flux is the mean of this.
pub fn flux(x: &[f32], frame: usize, hop: usize) -> Vec<f32> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(frame);
    let window: Vec<f32> = (0..frame)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / frame as f32).cos())
        .collect();
    let mut prev = vec![0f32; frame / 2];
    let mut env = Vec::with_capacity(x.len() / hop + 1);
    let mut buf = vec![Complex::new(0f32, 0f32); frame];
    let mut i = 0;
    while i + frame <= x.len() {
        for (k, b) in buf.iter_mut().enumerate() {
            *b = Complex::new(x[i + k] * window[k], 0.0);
        }
        fft.process(&mut buf);
        let mut rise = 0f32;
        for k in 1..frame / 2 {
            let m = (1.0 + 1000.0 * buf[k].norm()).ln();
            rise += (m - prev[k]).max(0.0);
            prev[k] = m;
        }
        env.push(rise);
        i += hop;
    }
    env
}

/// `env` with its slow trend (the local mean, [`TREND`] frames either side)
/// taken away and what is left under zero dropped: only the onsets stand
/// out.
pub fn detrend(env: &[f32]) -> Vec<f32> {
    let mut out = vec![0f32; env.len()];
    for (t, o) in out.iter_mut().enumerate() {
        let lo = t.saturating_sub(TREND);
        let hi = (t + TREND).min(env.len());
        let mean = env[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        *o = (env[t] - mean).max(0.0);
    }
    out
}

/// The envelope's autocorrelation at a fractional `lag`, by linear
/// interpolation between the two lags either side.
fn acf_at(env: &[f32], lag: f64) -> f64 {
    let l0 = lag.floor() as usize;
    let frac = (lag - l0 as f64) as f32;
    let n = env.len().saturating_sub(l0 + 1);
    let mut s = 0f32;
    for t in 0..n {
        let v = env[t + l0] * (1.0 - frac) + env[t + l0 + 1] * frac;
        s += env[t] * v;
    }
    f64::from(s) / n.max(1) as f64
}
