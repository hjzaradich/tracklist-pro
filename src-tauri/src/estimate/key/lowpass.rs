// Translated from libKeyFinder lowpassfilter.cpp, with the parts of
// keyfinder.cpp (`preprocess`) and audiodata.cpp (`reduceToMono`,
// `downsample`) that feed it (https://github.com/mixxxdj/libkeyfinder).
// Copyright 2011-2015 Ibrahim Sha'ath, and the Mixxx contributors.
// libKeyFinder's filter is based on the work of Tony Fisher (mkfilter).
// libKeyFinder is free software under the GNU General Public License,
// version 3 or (at your option) any later version; so is this translation.
// Translated to Rust and modified by the tracklist-pro contributors, 2026-11.

//! The low-pass filter and the downsampling before the spectrum: the audio,
//! mixed to mono, loses everything above the top of the chroma range and is
//! kept at one sample in [`factor`] (4,410 Hz for a 44.1 kHz file).
//!
//! As libKeyFinder does it: a 160th-order FIR filter, designed as an ideal
//! low-pass response put through an inverse FFT of 2,048 points and shaped
//! by a Hamming window; each sample is divided by the filter's gain before
//! it goes in, and only the samples kept are computed. The audio is cut to a
//! whole number of `factor` samples first (the C++ drops the rest), and the
//! filter runs off the end of it on zeros.
//!
//! The one difference: it runs as the samples arrive ([`Lowpass::push`]),
//! rather than over a buffer of the whole audio, so the decode needn't keep
//! the audio at its own rate. The answer is the same: an output is computed
//! only once every sample it needs is known to be kept.

use std::collections::VecDeque;
use std::f64::consts::PI;

use super::chroma::last_frequency;

/// The filter's order: it has `ORDER + 1` coefficients.
const ORDER: usize = 160;
/// How far the output lags the input: half the order.
const DELAY: usize = ORDER / 2;
/// The size of the inverse FFT the filter is designed with.
const DESIGN_SIZE: usize = 2048;

/// How many samples of a file at `rate` make one sample of the spectrum's
/// input, or `None` when the rate is too low for the chroma range.
pub fn factor(rate: u32) -> Option<usize> {
    let ds_cutoff = last_frequency() * 1.10;
    // As the C++: the rate halved as an integer, then divided.
    let factor = (f64::from(rate / 2) / ds_cutoff).floor();
    (factor >= 1.0).then_some(factor as usize)
}

/// The filter's coefficients for audio at `rate`, and their sum (its gain).
fn design(rate: u32) -> (Vec<f64>, f64) {
    let corner = last_frequency() * 1.012;
    let cutoff = corner / f64::from(rate);
    let tau = 0.5 / cutoff;
    // The ideal response, real and even. Of what the C++ hands FFTW's
    // complex-to-real transform, only the first half and the middle bin
    // count; the middle bin is above any cutoff here, so zero.
    let response: Vec<f64> = (0..=DESIGN_SIZE / 2)
        .map(|i| {
            if i < DESIGN_SIZE / 2 && i as f64 / DESIGN_SIZE as f64 <= cutoff {
                tau
            } else {
                0.0
            }
        })
        .collect();
    let n = DESIGN_SIZE as f64;
    // The inverse transform at point `index`, normalised by its size: the
    // sum FFTW's complex-to-real transform makes of a Hermitian spectrum.
    let inverse = |index: usize| -> f64 {
        let mut sum = response[0]
            + response[DESIGN_SIZE / 2] * if index.is_multiple_of(2) { 1.0 } else { -1.0 };
        for (k, &x) in response.iter().enumerate().take(DESIGN_SIZE / 2).skip(1) {
            if x != 0.0 {
                sum += 2.0 * x * (2.0 * PI * (k * index) as f64 / n).cos();
            }
        }
        sum / n
    };
    let length = ORDER + 1;
    let centre = ORDER / 2;
    let mut gain = 0.0;
    let coefficients = (0..length)
        .map(|i| {
            let index = (DESIGN_SIZE - centre + i) % DESIGN_SIZE;
            let c = inverse(index) * hamming(i, length);
            gain += c;
            c
        })
        .collect();
    (coefficients, gain)
}

/// libKeyFinder's Hamming window, point `n` of `size`.
fn hamming(n: usize, size: usize) -> f64 {
    0.54 - 0.46 * ((2.0 * PI * n as f64) / (size as f64 - 1.0)).cos()
}

/// The filter, fed mono samples as they're decoded; it keeps the
/// downsampled output.
pub struct Lowpass {
    coefficients: Vec<f64>,
    gain: f64,
    factor: usize,
    /// The input still needed, divided by the gain, from absolute sample
    /// `base` on.
    recent: VecDeque<f64>,
    base: usize,
    /// Samples pushed.
    count: usize,
    /// The next output to compute, as an input sample index (a multiple of
    /// `factor`).
    next: usize,
    out: Vec<f64>,
}

impl Lowpass {
    /// A filter for audio at `rate`, or `None` if the rate is too low.
    pub fn new(rate: u32) -> Option<Lowpass> {
        let factor = factor(rate)?;
        let (coefficients, gain) = design(rate);
        Some(Lowpass {
            coefficients,
            gain,
            factor,
            recent: VecDeque::with_capacity(ORDER + 1 + factor),
            base: 0,
            count: 0,
            next: 0,
            out: Vec::new(),
        })
    }

    /// The rate of the output, as libKeyFinder rounds it (down).
    pub fn rate(rate: u32) -> Option<u32> {
        factor(rate).map(|f| rate / f as u32)
    }

    /// The downsampling factor.
    pub fn factor(&self) -> usize {
        self.factor
    }

    /// Adds one mono sample. Not a number counts as silence (the C++ would
    /// refuse the audio).
    pub fn push(&mut self, sample: f64) {
        let sample = if sample.is_finite() { sample } else { 0.0 };
        self.recent.push_back(sample / self.gain);
        self.count += 1;
        // However the audio ends, fewer than `factor` samples are dropped
        // from its end, so these are kept for sure.
        while self.next + DELAY + self.factor <= self.count {
            let y = self.output(self.count);
            self.out.push(y);
            self.next += self.factor;
            while self.base + DELAY < self.next {
                self.recent.pop_front();
                self.base += 1;
            }
        }
    }

    /// Output `self.next`, from the input before `end` (zero from there on
    /// and before the start).
    fn output(&self, end: usize) -> f64 {
        let first = self.next as isize - DELAY as isize;
        let mut sum = 0.0;
        for (j, c) in self.coefficients.iter().enumerate() {
            let at = first + j as isize;
            let x = if at < 0 || at as usize >= end {
                0.0
            } else {
                self.recent[at as usize - self.base]
            };
            sum += c * x;
        }
        sum
    }

    /// The downsampled audio, once every sample is in: cut to a whole
    /// number of `factor` samples, the filter run off its end on zeros.
    pub fn finish(mut self) -> Vec<f64> {
        let kept = self.count - self.count % self.factor;
        while self.next < kept {
            let y = self.output(kept);
            self.out.push(y);
            self.next += self.factor;
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_factor_and_rate_are_libkeyfinders() {
        assert_eq!(factor(44_100), Some(10));
        assert_eq!(Lowpass::rate(44_100), Some(4_410));
        assert_eq!(factor(48_000), Some(11));
        assert_eq!(Lowpass::rate(48_000), Some(4_363));
        assert_eq!(factor(4_000), None);
    }

    #[test]
    fn the_filter_is_symmetric_and_passes_dc_at_unity() {
        let (c, gain) = design(44_100);
        assert_eq!(c.len(), ORDER + 1);
        for i in 0..c.len() {
            assert!((c[i] - c[ORDER - i]).abs() < 1e-12);
        }
        let dc: f64 = c.iter().map(|c| c / gain).sum();
        assert!((dc - 1.0).abs() < 1e-12);
    }

    #[test]
    fn streaming_gives_what_filtering_the_whole_buffer_gives() {
        let input: Vec<f64> = (0..10_007)
            .map(|i| ((i * 7919) % 101) as f64 / 50.0 - 1.0)
            .collect();
        let mut lp = Lowpass::new(44_100).unwrap();
        for &x in &input {
            lp.push(x);
        }
        let streamed = lp.finish();
        // The C++'s way: the whole buffer, cut, filtered, every tenth kept.
        let (c, gain) = design(44_100);
        let kept = input.len() - input.len() % 10;
        let x = |i: isize| {
            if i < 0 || i as usize >= kept {
                0.0
            } else {
                input[i as usize] / gain
            }
        };
        let whole: Vec<f64> = (0..kept)
            .step_by(10)
            .map(|out| {
                c.iter()
                    .enumerate()
                    .fold(0.0, |s, (j, c)| s + c * x(out as isize - 80 + j as isize))
            })
            .collect();
        assert_eq!(streamed.len(), whole.len());
        for (a, b) in streamed.iter().zip(&whole) {
            assert_eq!(a, b);
        }
    }
}
