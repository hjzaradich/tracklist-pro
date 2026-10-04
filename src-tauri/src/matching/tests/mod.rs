//! Behavior tests for fingerprint matching (1bA-1, 1bA-2).
//!
//! The audio is generated in code, never real music: the same generator
//! the fingerprint tests use (`fingerprint/tests/audio.rs`). The cases
//! mirror the duplicate and version ground truth of `tools/fixture-gen`
//! ([`corpus`]), at lengths chromaprint has something to work with (the
//! generator's own clips are a few seconds long, too short to match).

// The fingerprint tests' generator, taken as it is: it's private to them,
// and this lane doesn't edit their module.
#[path = "../../fingerprint/tests/audio.rs"]
#[allow(dead_code, clippy::duplicate_mod)]
mod audio;

mod blocking;
mod comparing;
mod corpus;
mod job;
mod passes;
mod reference;
mod scale;
mod smaller;
mod synthetic;

use std::io::Cursor;

use crate::fingerprint::{decode, Fingerprint};

use audio::Pcm;

/// The rate most test audio is made at: enough for chromaprint, which
/// resamples to 11 kHz anyway, and quick to generate.
const RATE: u32 = 22_050;

/// Fingerprints `bytes` as a file holding them would be.
fn print(bytes: &[u8]) -> Fingerprint {
    decode::fingerprint(Box::new(Cursor::new(bytes.to_vec())), &mut |_| true)
        .expect("the test audio decodes")
}

/// Mono samples from -1 to 1 as 16-bit PCM at [`RATE`].
fn pcm(samples: &[f64]) -> Pcm {
    Pcm {
        rate: RATE,
        channels: 1,
        samples: samples.iter().map(|s| (s * 30_000.0) as i16).collect(),
    }
}

/// Samples `from..to` (in seconds) of mono audio at [`RATE`].
fn cut(samples: &[f64], from: f64, to: f64) -> Vec<f64> {
    let at = |s: f64| ((s * f64::from(RATE)) as usize).min(samples.len());
    samples[at(from)..at(to)].to_vec()
}

/// `seconds` in fingerprint items.
fn items(seconds: f64) -> f64 {
    seconds / f64::from(super::item_seconds())
}
