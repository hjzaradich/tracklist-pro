//! The model's input: CED's own feature extraction (its
//! `feature_extraction_ced.py`, torchaudio). 16 kHz mono in; a 512-point
//! periodic Hann STFT, hop 160, centred with reflect padding; power; 64 HTK
//! mel bands from 0 to 8 kHz, no normalisation; decibels with a floor 120 dB
//! under the clip's loudest value. A 10 s window gives 64 x 1001 values.

use std::f64::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

pub const RATE: usize = 16_000;
pub const WINDOW_SAMPLES: usize = 10 * RATE;
pub const N_MELS: usize = 64;
pub const N_FRAMES: usize = 1 + WINDOW_SAMPLES / HOP;
const N_FFT: usize = 512;
const HOP: usize = 160;
const N_BINS: usize = N_FFT / 2 + 1;
/// The floor under the loudest value, in dB.
const RANGE_DB: f32 = 120.0;

/// The tables (window, filter bank, FFT plan) one front end needs; built once
/// and shared by every thread.
pub struct Frontend {
    window: Vec<f32>,
    /// Per band: the first bin and the weights from there on (zeros dropped).
    bands: Vec<(usize, Vec<f32>)>,
    fft: Arc<dyn Fft<f32>>,
}

impl Frontend {
    pub fn new() -> Frontend {
        // torch.hann_window(512), periodic.
        let window = (0..N_FFT)
            .map(|n| (0.5 - 0.5 * (2.0 * PI * n as f64 / N_FFT as f64).cos()) as f32)
            .collect();
        let bands = htk_filters()
            .chunks(N_BINS)
            .map(|row| {
                let lo = row.iter().position(|&w| w != 0.0).unwrap_or(0);
                let hi = row.iter().rposition(|&w| w != 0.0).map_or(lo, |i| i + 1);
                (lo, row[lo..hi].to_vec())
            })
            .collect();
        Frontend {
            window,
            bands,
            fft: FftPlanner::<f32>::new().plan_fft_forward(N_FFT),
        }
    }

    /// One 10 s window at 16 kHz to the model input, row-major 64 x 1001.
    pub fn mel(&self, wave: &[f32]) -> Vec<f32> {
        assert_eq!(
            wave.len(),
            WINDOW_SAMPLES,
            "a window is exactly 10 s at 16 kHz"
        );
        let pad = N_FFT / 2;
        let n = wave.len();
        // Reflect padding, as torch.stft(center=True).
        let mut padded = Vec::with_capacity(n + 2 * pad);
        padded.extend((0..pad).map(|i| wave[pad - i]));
        padded.extend_from_slice(wave);
        padded.extend((0..pad).map(|i| wave[n - 2 - i]));
        let mut power = vec![0f32; N_FRAMES * N_BINS];
        let mut buf = vec![Complex32::new(0.0, 0.0); N_FFT];
        let mut scratch = vec![Complex32::new(0.0, 0.0); self.fft.get_inplace_scratch_len()];
        for t in 0..N_FRAMES {
            let frame = &padded[t * HOP..t * HOP + N_FFT];
            for (k, slot) in buf.iter_mut().enumerate() {
                *slot = Complex32::new(frame[k] * self.window[k], 0.0);
            }
            self.fft.process_with_scratch(&mut buf, &mut scratch);
            for (b, p) in power[t * N_BINS..(t + 1) * N_BINS].iter_mut().enumerate() {
                *p = buf[b].norm_sqr();
            }
        }
        let mut out = vec![0f32; N_MELS * N_FRAMES];
        let mut loudest = f32::MIN;
        for (m, (lo, weights)) in self.bands.iter().enumerate() {
            for t in 0..N_FRAMES {
                let p = &power[t * N_BINS + lo..t * N_BINS + lo + weights.len()];
                let sum: f32 = weights.iter().zip(p).map(|(w, x)| w * x).sum();
                let db = 10.0 * sum.max(1e-10).log10();
                loudest = loudest.max(db);
                out[m * N_FRAMES + t] = db;
            }
        }
        let floor = loudest - RANGE_DB;
        for v in &mut out {
            *v = v.max(floor);
        }
        out
    }
}

/// torchaudio.functional.melscale_fbanks (HTK, norm=None): 64 x 257,
/// row-major.
fn htk_filters() -> Vec<f32> {
    let hz_to_mel = |f: f64| 2595.0 * (1.0 + f / 700.0).log10();
    let mel_to_hz = |m: f64| 700.0 * (10f64.powf(m / 2595.0) - 1.0);
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(8000.0));
    let edges: Vec<f64> = (0..N_MELS + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64))
        .collect();
    let mut out = vec![0f32; N_MELS * N_BINS];
    for m in 0..N_MELS {
        for b in 0..N_BINS {
            let f = (RATE as f64 / 2.0) * b as f64 / (N_BINS - 1) as f64;
            let up = (f - edges[m]) / (edges[m + 1] - edges[m]);
            let down = (edges[m + 2] - f) / (edges[m + 2] - edges[m + 1]);
            out[m * N_BINS + b] = up.min(down).max(0.0) as f32;
        }
    }
    out
}
