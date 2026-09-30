//! Synthetic audio: short mono songs built from bars, tones for bulk files,
//! and band-limited noise for quality cases.
//!
//! Everything here uses plain arithmetic (no `sin`, `exp` or `powf` from the
//! platform's math library), so a seed gives the same samples on every
//! machine.

use std::ops::Range;

use crate::rng::Rng;

/// Mono 16-bit PCM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<i16>,
}

impl Pcm {
    pub fn from_float(rate: u32, samples: &[f64]) -> Pcm {
        let samples = samples
            .iter()
            .map(|&x| (x.clamp(-1.0, 1.0) * 32_767.0).round() as i16)
            .collect();
        Pcm { rate, samples }
    }

    pub fn frames(&self) -> u64 {
        self.samples.len() as u64
    }
}

const PI: f64 = std::f64::consts::PI;

/// `sin(2π · cycles)`, from a polynomial so it's identical on every platform.
pub fn sine(cycles: f64) -> f64 {
    let mut x = (cycles - cycles.floor()) * 2.0 * PI; // [0, 2π)
    if x > PI {
        x -= 2.0 * PI; // (-π, π]
    }
    if x > PI / 2.0 {
        x = PI - x;
    } else if x < -PI / 2.0 {
        x = -PI - x;
    }
    // Taylor series to x^13; the error at π/2 is below 1e-9.
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    for n in 1..=6 {
        let k = (2 * n) as f64;
        term *= -x2 / (k * (k + 1.0));
        sum += term;
    }
    sum
}

/// 12-tone equal temperament ratios for 0..=24 semitones, written out so no
/// `powf` is needed.
const SEMITONE: [f64; 25] = [
    1.0,
    1.059_463_094_359_295_3,
    1.122_462_048_309_373,
    1.189_207_115_002_721,
    1.259_921_049_894_873_2,
    1.334_839_854_170_034_4,
    std::f64::consts::SQRT_2,
    1.498_307_076_876_681_5,
    1.587_401_051_968_199_4,
    1.681_792_830_507_429,
    1.781_797_436_280_678_5,
    1.887_748_625_363_386_9,
    2.0,
    2.118_926_188_718_590_6,
    2.244_924_096_618_746,
    2.378_414_230_005_442,
    2.519_842_099_789_746_3,
    2.669_679_708_340_068_8,
    2.828_427_124_746_190_2,
    2.996_614_153_753_363,
    3.174_802_103_936_398_8,
    3.363_585_661_014_858,
    3.563_594_872_561_357,
    3.775_497_250_726_773_8,
    4.0,
];

/// Minor pentatonic degrees over two octaves.
const SCALE: [usize; 8] = [0, 3, 5, 7, 10, 12, 15, 17];

/// How a song sounds, as opposed to what it plays.
#[derive(Clone, Copy, Debug)]
pub struct Timbre {
    /// Levels of the 2nd and 3rd harmonics.
    pub harmonics: [f64; 2],
    /// Seeds the drum pattern and hi-hat noise.
    pub drum_seed: u64,
    /// Hi-hat noise level.
    pub noise: f64,
    /// Extra steady noise (a crowd, tape hiss) under everything.
    pub hiss: f64,
}

impl Timbre {
    pub fn from_seed(seed: u64) -> Timbre {
        let mut rng = Rng::derive(seed, "timbre");
        Timbre {
            harmonics: [0.1 + 0.5 * rng.unit(), 0.05 + 0.3 * rng.unit()],
            drum_seed: rng.next_u64(),
            noise: 0.05 + 0.1 * rng.unit(),
            hiss: 0.0,
        }
    }
}

/// A song: a melody (what's played) rendered with a timbre (how it sounds),
/// bar by bar. Bars are independent, so bars `2..6` of a song are exactly
/// samples of bars `0..8`: that's how a Radio Edit sits inside an Original.
#[derive(Clone, Debug)]
pub struct Song {
    pub melody_seed: u64,
    /// Root note of the key, in Hz.
    pub root_hz: f64,
    pub bar_ms: u32,
    pub timbre: Timbre,
    pub rate: u32,
}

impl Song {
    pub fn from_seed(seed: u64, rate: u32) -> Song {
        let mut rng = Rng::derive(seed, "song");
        Song {
            melody_seed: rng.next_u64(),
            root_hz: 110.0 * SEMITONE[rng.below(12) as usize],
            bar_ms: 900 + 50 * rng.below(5) as u32,
            timbre: Timbre::from_seed(rng.next_u64()),
            rate,
        }
    }

    pub fn bar_len(&self) -> usize {
        (self.rate as usize * self.bar_ms as usize) / 1000
    }

    /// One bar: four beats of melody, a kick on beats 1 and 3, hats between.
    pub fn bar(&self, index: i32) -> Vec<f64> {
        let len = self.bar_len();
        let beat = len / 4;
        let rate = self.rate as f64;
        let mut out = vec![0.0; len];
        let mut notes = Rng::derive(self.melody_seed, &format!("bar {index}"));
        let mut drums = Rng::derive(self.timbre.drum_seed, &format!("bar {index}"));
        let mut hiss = Rng::derive(self.timbre.drum_seed, &format!("hiss {index}"));
        for b in 0..4 {
            let start = b * beat;
            let freq = self.root_hz * SEMITONE[*notes.pick(&SCALE)];
            let [h2, h3] = self.timbre.harmonics;
            for i in 0..beat {
                let t = i as f64 / rate;
                let env = {
                    let d = 1.0 - i as f64 / beat as f64;
                    let attack = (i as f64 / (0.005 * rate)).min(1.0);
                    attack * d * d
                };
                let c = freq * t;
                let tone = sine(c) + h2 * sine(2.0 * c) + h3 * sine(3.0 * c);
                out[start + i] += 0.3 * env * tone;
            }
            // Kick: a falling sine, 150 ms.
            if b % 2 == 0 && drums.chance(0.9) {
                let klen = (0.15 * rate) as usize;
                let mut phase = 0.0;
                for i in 0..klen.min(beat) {
                    let p = i as f64 / klen as f64;
                    phase += (50.0 + 70.0 * (1.0 - p)) / rate;
                    let d = 1.0 - p;
                    out[start + i] += 0.4 * d * d * d * sine(phase);
                }
            }
            // Hat: a 40 ms noise burst on the off-beat.
            if drums.chance(0.8) {
                let hstart = start + beat / 2;
                let hlen = (0.04 * rate) as usize;
                for i in 0..hlen.min(len - hstart) {
                    let d = 1.0 - i as f64 / hlen as f64;
                    let n = drums.unit() * 2.0 - 1.0;
                    out[hstart + i] += self.timbre.noise * d * d * n;
                }
            }
        }
        if self.timbre.hiss > 0.0 {
            for s in out.iter_mut() {
                *s += self.timbre.hiss * (hiss.unit() * 2.0 - 1.0);
            }
        }
        out
    }

    pub fn render(&self, bars: Range<i32>) -> Vec<f64> {
        bars.flat_map(|i| self.bar(i)).collect()
    }
}

/// A short tone for bulk files: two or three partials, a little noise, and
/// short fades. Different seeds give different audio, so no two bulk files
/// are duplicates.
pub fn tone(seed: u64, rate: u32, ms: u32) -> Vec<f64> {
    let mut rng = Rng::derive(seed, "tone");
    let len = (rate as usize * ms as usize) / 1000;
    let partials: Vec<(f64, f64)> = (0..rng.range(2, 3))
        .map(|_| (110.0 + 1800.0 * rng.unit(), 0.15 + 0.2 * rng.unit()))
        .collect();
    let noise = 0.02 * rng.unit();
    let fade = (rate as usize / 100).max(1).min(len / 2).max(1);
    (0..len)
        .map(|i| {
            let t = i as f64 / rate as f64;
            let mut x: f64 = partials.iter().map(|&(f, a)| a * sine(f * t)).sum();
            x += noise * (rng.unit() * 2.0 - 1.0);
            let edge = i.min(len - 1 - i);
            if edge < fade {
                x *= edge as f64 / fade as f64;
            }
            x
        })
        .collect()
}

/// Noise-like audio made of many sines, none above `max_hz`. Used for the
/// quality cases: content that stops at 16 kHz looks like an MP3 transcode.
pub fn band_limited(seed: u64, rate: u32, ms: u32, max_hz: f64) -> Vec<f64> {
    let mut rng = Rng::derive(seed, "band");
    let len = (rate as usize * ms as usize) / 1000;
    let count = 120;
    let partials: Vec<(f64, f64)> = (0..count)
        .map(|k| {
            // Spread evenly from 60 Hz to max_hz, with a little jitter.
            let f = 60.0 + (max_hz - 60.0) * ((k as f64 + rng.unit()) / count as f64);
            (f.min(max_hz), rng.unit())
        })
        .collect();
    let gain = 0.5 / (count as f64).sqrt();
    (0..len)
        .map(|i| {
            let t = i as f64 / rate as f64;
            gain * partials
                .iter()
                .map(|&(f, ph)| sine(f * t + ph))
                .sum::<f64>()
        })
        .collect()
}

/// Mixes two signals at half level each, as long as the longer one.
pub fn mix(a: &[f64], b: &[f64]) -> Vec<f64> {
    (0..a.len().max(b.len()))
        .map(|i| 0.6 * (a.get(i).unwrap_or(&0.0) + b.get(i).unwrap_or(&0.0)))
        .collect()
}

/// Silences the given sample ranges (a radio "bleep" of an explicit word).
pub fn mute(mut samples: Vec<f64>, spans: &[Range<usize>]) -> Vec<f64> {
    let len = samples.len();
    for span in spans {
        for s in &mut samples[span.start.min(len)..span.end.min(len)] {
            *s = 0.0;
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polynomial_sine_matches_the_real_one() {
        for i in 0..1000 {
            let c = i as f64 / 137.0 - 3.0;
            assert!((sine(c) - (2.0 * PI * c).sin()).abs() < 1e-8, "at {c}");
        }
    }

    #[test]
    fn a_bar_range_is_exactly_a_slice_of_a_longer_render() {
        let song = Song::from_seed(3, 22_050);
        let full = song.render(0..8);
        let edit = song.render(2..6);
        let start = 2 * song.bar_len();
        assert_eq!(&full[start..start + edit.len()], &edit[..]);
    }
}
