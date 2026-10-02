//! The spectral cutoff: the highest frequency still within 50 dB of the
//! level between 2 and 8 kHz (ROADMAP 1.6), ported from the Phase 0 spike
//! (`spikes/cutoff_scan.py`, which ran `audio-lab`'s `cutoff`).
//!
//! A lossy encoder low-passes what it can't afford to keep, so a file whose
//! spectrum stops well below the Nyquist limit was probably made from a
//! worse source (a "320" made from a 128 kbps file). This only measures
//! where the spectrum stops; what that means is a later step (1bB-6).
//!
//! **The sample.** The spike analyzed 60 s starting 30 s in, because a
//! track's first seconds are often an intro, a fade-in or a quiet DJ-tool
//! lead-in, whose spectrum says little about the encoder. A file shorter
//! than 50 s has no such stretch (under 20 s of it would be left), so its
//! first 60 s, which is the whole file, are used instead. The meter reads
//! the stream as it's decoded and keeps only one spectrum sum per window,
//! so memory stays flat however long the file is, and it stops doing any
//! work once 90 s have gone by.
//!
//! Per frame: a 4096-point Hann-windowed FFT of the mono mix. The power of
//! every frame in the sample is averaged per bin and turned to dB, and the
//! reference is the median of the 2-8 kHz bins. Each bin is then smoothed
//! over ±250 Hz, and the cutoff is the highest bin whose smoothed level is
//! above the reference minus 50 dB.

use std::f32::consts::TAU;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// Samples per FFT frame.
const FRAME: usize = 4096;
/// Seconds into the file where the sample starts, and how long it is.
const SAMPLE_START: f64 = 30.0;
const SAMPLE_LENGTH: f64 = 60.0;
/// A file with at least this many seconds has the sample above; a shorter
/// one is analyzed from its start.
const LONG_FILE: f64 = 50.0;
/// Less audio than this in the sample can't give a spectrum worth reading.
const MIN_SAMPLE: f64 = 5.0;
/// How far below the reference level the spectrum still counts (dB).
const WITHIN_DB: f64 = 50.0;
/// Below this level (RMS, dB relative to full scale) a sample is silence.
const SILENT_DB: f64 = -90.0;
/// Half the width the spectrum is smoothed over (Hz).
const SMOOTH_HZ: f64 = 250.0;
/// The band whose median level is the reference (Hz).
const REFERENCE_BAND: (f64, f64) = (2000.0, 8000.0);

/// Why a file has no cutoff. Stored as `file_quality.cutoff_gap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// Under 5 s of audio in the sample.
    TooShort,
    /// The sample is silent (below -90 dB).
    Silent,
    /// The sample rate is too low to have a 2-8 kHz band to compare with.
    LowRate,
}

impl Gap {
    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Gap::TooShort => "too_short",
            Gap::Silent => "silent",
            Gap::LowRate => "low_rate",
        }
    }
}

/// The spectrum summed over frames, for one part of the file.
struct Sum {
    power: Vec<f64>,
    frames: usize,
    /// Sum of the squared samples and how many, for the RMS.
    squares: f64,
    samples: usize,
}

impl Sum {
    fn new() -> Sum {
        Sum {
            power: vec![0.0; FRAME / 2],
            frames: 0,
            squares: 0.0,
            samples: 0,
        }
    }
}

/// Reads a mono stream and measures its cutoff. Feed it the decoded mono
/// samples in order, then [`finish`](Meter::finish).
pub struct Meter {
    rate: u32,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Frames starting in the first 60 s: the sample for a short file.
    early: Sum,
    /// Frames from 30 s to 90 s: the sample for a long one.
    late: Sum,
    /// The samples not yet a whole frame.
    pending: Vec<f32>,
    /// How many samples came before `pending`.
    position: usize,
    scratch: Vec<Complex<f32>>,
    /// Every sample fed, so a file's length is known at the end.
    total: usize,
}

impl Meter {
    pub fn new(rate: u32) -> Meter {
        let fft = FftPlanner::<f32>::new().plan_fft_forward(FRAME);
        let window = (0..FRAME)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / FRAME as f32).cos())
            .collect();
        Meter {
            rate,
            fft,
            window,
            early: Sum::new(),
            late: Sum::new(),
            pending: Vec::with_capacity(FRAME),
            position: 0,
            scratch: vec![Complex::new(0.0, 0.0); FRAME],
            total: 0,
        }
    }

    fn at(&self, seconds: f64) -> usize {
        (seconds * f64::from(self.rate)) as usize
    }

    /// Takes the next mono samples (-1 to 1).
    pub fn feed(&mut self, mono: &[f32]) {
        self.total += mono.len();
        let end = self.at(SAMPLE_START + SAMPLE_LENGTH);
        let mut mono = mono;
        while !mono.is_empty() {
            // Nothing past the sample is looked at.
            if self.position >= end {
                return;
            }
            let take = (FRAME - self.pending.len()).min(mono.len());
            self.pending.extend_from_slice(&mono[..take]);
            mono = &mono[take..];
            if self.pending.len() == FRAME {
                self.frame();
                self.position += FRAME;
                self.pending.clear();
            }
        }
    }

    /// Analyzes the whole frame in `pending`, which starts at `position`.
    fn frame(&mut self) {
        let start = self.position;
        let in_early = start < self.at(SAMPLE_LENGTH);
        let in_late = start >= self.at(SAMPLE_START)
            && start + FRAME <= self.at(SAMPLE_START + SAMPLE_LENGTH);
        if !in_early && !in_late {
            return;
        }
        for ((slot, &x), &w) in self.scratch.iter_mut().zip(&self.pending).zip(&self.window) {
            *slot = Complex::new(x * w, 0.0);
        }
        self.fft.process(&mut self.scratch);
        let squares: f64 = self.pending.iter().map(|&x| f64::from(x).powi(2)).sum();
        for (on, sum) in [(in_early, &mut self.early), (in_late, &mut self.late)] {
            if !on {
                continue;
            }
            for (p, c) in sum.power.iter_mut().zip(&self.scratch) {
                *p += f64::from(c.norm_sqr());
            }
            sum.frames += 1;
            sum.squares += squares;
            sum.samples += FRAME;
        }
    }

    /// The cutoff in Hz, or why there's none.
    pub fn finish(self) -> Result<u32, Gap> {
        let rate = f64::from(self.rate);
        let long = self.total as f64 / rate >= LONG_FILE;
        let sum = if long { &self.late } else { &self.early };
        if (sum.samples as f64) < MIN_SAMPLE * rate {
            return Err(Gap::TooShort);
        }
        let rms = (sum.squares / sum.samples as f64).sqrt();
        if 20.0 * rms.max(1e-12).log10() < SILENT_DB {
            return Err(Gap::Silent);
        }
        let bin_hz = rate / FRAME as f64;
        let db: Vec<f64> = sum
            .power
            .iter()
            .map(|p| 10.0 * (p / sum.frames as f64 + 1e-20).log10())
            .collect();
        let mut band: Vec<f64> = (0..FRAME / 2)
            .filter(|&k| (REFERENCE_BAND.0..REFERENCE_BAND.1).contains(&(k as f64 * bin_hz)))
            .map(|k| db[k])
            .collect();
        if band.is_empty() {
            return Err(Gap::LowRate);
        }
        band.sort_by(f64::total_cmp);
        let reference = band[band.len() / 2];
        // Prefix sums, so each bin's smoothed level is two lookups.
        let mut prefix = vec![0.0; db.len() + 1];
        for (k, d) in db.iter().enumerate() {
            prefix[k + 1] = prefix[k] + d;
        }
        let smooth = (SMOOTH_HZ / bin_hz).ceil() as usize;
        if smooth == 0 || db.len() <= 2 * smooth {
            return Err(Gap::LowRate);
        }
        (smooth..db.len() - smooth)
            .rev()
            .find(|&k| {
                let avg = (prefix[k + smooth] - prefix[k - smooth]) / (2 * smooth) as f64;
                avg > reference - WITHIN_DB
            })
            .map(|k| (k as f64 * bin_hz).round() as u32)
            .filter(|&hz| hz > 0)
            .ok_or(Gap::Silent)
    }
}
