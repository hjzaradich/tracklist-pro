// Translated from libKeyFinder chromatransform.cpp, spectrumanalyser.cpp,
// windowfunctions.cpp, temporalwindowfactory.cpp, chromagram.cpp, the band
// frequencies of constants.cpp, and the zero padding of keyfinder.cpp
// (`finalChromagram`) (https://github.com/mixxxdj/libkeyfinder).
// Copyright 2011-2015 Ibrahim Sha'ath, and the Mixxx contributors.
// libKeyFinder is free software under the GNU General Public License,
// version 3 or (at your option) any later version; so is this translation.

//! The chroma: how strong each of the 72 bands (six octaves of semitones,
//! from C at 32.7 Hz) is, averaged over the window.
//!
//! As libKeyFinder does it: frames of [`FRAME`] samples every [`HOP`],
//! Blackman-windowed, through an FFT; each band is a weighted sum of the
//! magnitudes of the FFT bins around its frequency (the "direct spectral
//! kernel", a raised-cosine window [`STRETCH`] of a semitone wide, its
//! weights normalised and scaled by the band's frequency, which models a
//! constant-Q transform). A frame starts at every hop inside the audio, the
//! last ones padded with zeros, and the bands are averaged over the frames.
//! The FFT is `rustfft` rather than FFTW.

use std::f64::consts::PI;

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Samples per FFT frame, and how far apart frames start.
pub const FRAME: usize = 16_384;
pub const HOP: usize = FRAME / 4;
/// Semitones in an octave, and the octaves the bands cover.
pub const SEMITONES: usize = 12;
pub const OCTAVES: usize = 6;
pub const BANDS: usize = SEMITONES * OCTAVES;
/// How wide each band's kernel is, as a share of a semitone.
const STRETCH: f64 = 0.8;

/// The bands' centre frequencies in Hz, C1 to B6 (libKeyFinder's table).
const FREQUENCIES: [f64; BANDS] = [
    32.7031956625748,
    34.647828872109,
    36.708095989676,
    38.8908729652601,
    41.2034446141088,
    43.6535289291255,
    46.2493028389543,
    48.9994294977187,
    51.9130871974932,
    55.0,
    58.2704701897613,
    61.7354126570155,
    65.4063913251497,
    69.2956577442181,
    73.4161919793519,
    77.7817459305203,
    82.4068892282175,
    87.307057858251,
    92.4986056779087,
    97.9988589954374,
    103.826174394986,
    110.0,
    116.540940379523,
    123.470825314031,
    130.812782650299,
    138.591315488436,
    146.832383958704,
    155.563491861041,
    164.813778456435,
    174.614115716502,
    184.997211355817,
    195.997717990875,
    207.652348789973,
    220.0,
    233.081880759045,
    246.941650628062,
    261.625565300599,
    277.182630976872,
    293.664767917408,
    311.126983722081,
    329.62755691287,
    349.228231433004,
    369.994422711635,
    391.99543598175,
    415.304697579946,
    440.000000000001,
    466.163761518091,
    493.883301256125,
    523.251130601198,
    554.365261953745,
    587.329535834816,
    622.253967444163,
    659.255113825741,
    698.456462866009,
    739.98884542327,
    783.9908719635,
    830.609395159892,
    880.000000000002,
    932.327523036182,
    987.76660251225,
    1046.5022612024,
    1108.73052390749,
    1174.65907166963,
    1244.50793488833,
    1318.51022765148,
    1396.91292573202,
    1479.97769084654,
    1567.981743927,
    1661.21879031978,
    1760.0,
    1864.65504607236,
    1975.5332050245,
];

/// The top band's frequency, which the low-pass filter is set from.
pub fn last_frequency() -> f64 {
    FREQUENCIES[BANDS - 1]
}

/// One band's kernel: the first FFT bin it reads and the weight of each.
struct Band {
    first: usize,
    weights: Vec<f64>,
}

/// The kernel of every band, for audio at `rate`; `None` if the rate can't
/// resolve the bands (the C++ refuses it).
fn kernel(rate: u32) -> Option<Vec<Band>> {
    let rate_f = f64::from(rate);
    if rate == 0
        || last_frequency() > rate_f / 2.0
        || rate_f / FRAME as f64 > FREQUENCIES[1] - FREQUENCIES[0]
    {
        return None;
    }
    let q = STRETCH * (2f64.powf(1.0 / SEMITONES as f64) - 1.0);
    Some(
        FREQUENCIES
            .iter()
            .map(|&frequency| {
                let centre = frequency * FRAME as f64 / rate_f;
                let width = centre * q;
                let beginning = centre - width / 2.0;
                let end = beginning + width;
                let first = beginning.ceil() as usize;
                let mut weights = Vec::new();
                let mut sum = 0.0;
                let mut bin = first;
                while bin as f64 <= end.floor() {
                    let n = bin as f64 - beginning;
                    let w = 1.0 - ((2.0 * PI * n) / width).cos();
                    sum += w;
                    weights.push(w);
                    bin += 1;
                }
                for w in &mut weights {
                    *w = *w / sum * frequency;
                }
                Band { first, weights }
            })
            .collect(),
    )
}

/// libKeyFinder's Blackman window over a frame.
fn blackman() -> Vec<f64> {
    let n = (FRAME - 1) as f64;
    (0..FRAME)
        .map(|i| {
            let i = i as f64;
            0.42 - 0.5 * ((2.0 * PI * i) / n).cos() + 0.08 * ((4.0 * PI * i) / n).cos()
        })
        .collect()
}

/// The chroma of `audio` (mono, already filtered and downsampled) at
/// `rate`: the 72 bands averaged over every frame. All zeros for no audio;
/// `None` if the rate can't resolve the bands.
pub fn chroma(audio: &[f64], rate: u32) -> Option<[f64; BANDS]> {
    let kernel = kernel(rate)?;
    let mut sums = [0.0; BANDS];
    // A frame at every hop that starts inside the audio; the rest of the
    // last ones is zeros (`finalChromagram`'s padding).
    let frames = audio.len().div_ceil(HOP);
    if frames == 0 {
        return Some(sums);
    }
    let window = blackman();
    let fft = FftPlanner::<f64>::new().plan_fft_forward(FRAME);
    let mut buffer = vec![Complex::new(0.0, 0.0); FRAME];
    let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    let mut magnitudes = vec![0.0; FRAME / 2 + 1];
    let count = frames as f64;
    for hop in 0..frames {
        let start = hop * HOP;
        for (i, slot) in buffer.iter_mut().enumerate() {
            let x = audio.get(start + i).copied().unwrap_or(0.0);
            *slot = Complex::new(x * window[i], 0.0);
        }
        fft.process_with_scratch(&mut buffer, &mut scratch);
        for (m, c) in magnitudes.iter_mut().zip(&buffer) {
            *m = (c.re * c.re + c.im * c.im).sqrt();
        }
        // `collapseToOneHop`: each frame's share added in turn.
        for (total, band) in sums.iter_mut().zip(&kernel) {
            let mut sum = 0.0;
            for (j, w) in band.weights.iter().enumerate() {
                // FFTW's real transform leaves the bins past the middle at
                // zero; no band reaches them at a rate the filter gives.
                let m = magnitudes.get(band.first + j).copied().unwrap_or(0.0);
                sum += m * w;
            }
            *total += sum / count;
        }
    }
    Some(sums)
}

/// The chroma folded to the 12 pitch classes, C first: each band added to
/// its semitone over the six octaves. Not what keys are chosen by (that is
/// all 72 bands, [`super::classify`]); a summary for the tests and the
/// golden vectors.
pub fn pitch_classes(chroma: &[f64; BANDS]) -> [f64; SEMITONES] {
    let mut classes = [0.0; SEMITONES];
    for (band, v) in chroma.iter().enumerate() {
        classes[band % SEMITONES] += v;
    }
    classes
}
