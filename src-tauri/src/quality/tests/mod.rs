//! Behavior tests for the quality measurements (1bA-10, 1bA-11). The audio
//! is generated in code (tones, noise, low-passed noise), never committed
//! and never real music.

// The fingerprint tests' audio writers (WAV, FLAC, MP3…), used as they are.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../../fingerprint/tests/audio.rs"]
mod audio;
#[cfg(windows)]
mod job;
mod measuring;
#[cfg(windows)]
mod support;

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::f64::consts::TAU;

use audio::Pcm;

/// splitmix64, so the noise is the same on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// -1 to 1.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// White noise, `seconds` long, at the given RMS level (0 to 1).
pub fn noise(seed: u64, seconds: f64, rate: u32, rms: f64) -> Vec<f64> {
    let mut rng = Rng(seed);
    let n = (seconds * f64::from(rate)) as usize;
    // Uniform noise has an RMS of 1/sqrt(3).
    (0..n).map(|_| rng.unit() * rms * 3f64.sqrt()).collect()
}

/// `x` with everything above `hz` removed (a brick-wall filter in the
/// frequency domain), then scaled back to the RMS it had.
pub fn low_passed(x: &[f64], rate: u32, hz: f64) -> Vec<f64> {
    let n = x.len();
    let mut planner = FftPlanner::<f64>::new();
    let forward = planner.plan_fft_forward(n);
    let inverse = planner.plan_fft_inverse(n);
    let mut spectrum: Vec<Complex<f64>> = x.iter().map(|&v| Complex::new(v, 0.0)).collect();
    forward.process(&mut spectrum);
    let bin_hz = f64::from(rate) / n as f64;
    for (k, bin) in spectrum.iter_mut().enumerate() {
        // Bin k and its mirror n - k are the same frequency.
        let hz_of_bin = k.min(n - k) as f64 * bin_hz;
        if hz_of_bin > hz {
            *bin = Complex::new(0.0, 0.0);
        }
    }
    inverse.process(&mut spectrum);
    let out: Vec<f64> = spectrum.iter().map(|c| c.re / n as f64).collect();
    let rms = |v: &[f64]| (v.iter().map(|s| s * s).sum::<f64>() / v.len() as f64).sqrt();
    let gain = rms(x) / rms(&out).max(1e-12);
    out.iter().map(|v| v * gain).collect()
}

/// A pure tone.
pub fn tone(hz: f64, seconds: f64, rate: u32, amplitude: f64) -> Vec<f64> {
    let n = (seconds * f64::from(rate)) as usize;
    (0..n)
        .map(|i| amplitude * (TAU * hz * i as f64 / f64::from(rate)).sin())
        .collect()
}

/// `x` as mono 16-bit PCM.
pub fn pcm(x: &[f64], rate: u32) -> Pcm {
    Pcm {
        rate,
        channels: 1,
        samples: x
            .iter()
            .map(|&v| (v.clamp(-1.0, 1.0) * 32_767.0).round() as i16)
            .collect(),
    }
}

/// `x` as a WAV file.
pub fn wav(x: &[f64], rate: u32) -> Vec<u8> {
    audio::wav(&pcm(x, rate))
}

/// A source over bytes in memory, for the decoder.
pub fn source(bytes: &[u8]) -> Box<dyn symphonia::core::io::MediaSource> {
    Box::new(std::io::Cursor::new(bytes.to_vec()))
}
