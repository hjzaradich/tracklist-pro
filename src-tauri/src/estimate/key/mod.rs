// Translated from libKeyFinder keyfinder.cpp (`keyOfAudio`, `preprocess`,
// `finalChromagram`) (https://github.com/mixxxdj/libkeyfinder).
// Copyright 2011-2015 Ibrahim Sha'ath, and the Mixxx contributors.
// libKeyFinder is free software under the GNU General Public License,
// version 3 or (at your option) any later version; so is this translation.
// Translated to Rust and modified by the tracklist-pro contributors, 2026-11.

//! The app's own key estimate (ROADMAP 3.7, "Key"): libKeyFinder's method
//! (Mixxx's key detector), translated into Rust. Owner decision 2026-11-06.
//!
//! 1. The decoded window, mixed to mono, goes through a low-pass filter and
//!    is kept at about 4.4 kHz ([`lowpass`]).
//! 2. Its chroma: the 72-band log-frequency spectrum, six octaves from C at
//!    32.7 Hz, averaged over FFT frames of 16,384 samples every 4,096
//!    ([`chroma`]).
//! 3. The key: the best cosine similarity with KeyFinder's major and minor
//!    profiles, one of 24 keys, or silence ([`classify`]).
//!
//! The answer is a pitch class and a mode ([`MusicalKey`]); its Camelot
//! name comes only from [`crate::tags::key`].
//!
//! Where it isn't libKeyFinder: it's given the analyze job's window (the
//! first 150 s less a 30 s intro, [`super::window`]) rather than the whole
//! file, and the filter runs over the whole decoded stretch before the intro
//! is cut, so the window's first few samples were filtered with the audio
//! before them rather than with zeros. The FFT is `rustfft`, not FFTW. It
//! adds a confidence (how far the best key stands above the next).

pub mod chroma;
pub mod classify;
pub mod lowpass;

pub use classify::Choice;
pub use lowpass::Lowpass;

/// The key of `audio`: mono, filtered and downsampled by [`Lowpass`] to
/// `rate`. `None` if the rate can't resolve the bands.
pub fn of_filtered(audio: &[f64], rate: u32) -> Option<Choice> {
    chroma::chroma(audio, rate).map(|c| classify::classify(&c))
}

/// The key of `audio`, mono at `rate` as decoded (libKeyFinder's
/// `keyOfAudio`). `None` if the rate is too low for the bands.
pub fn of_audio(audio: &[f64], rate: u32) -> Option<Choice> {
    let mut lowpass = Lowpass::new(rate)?;
    for &x in audio {
        lowpass.push(x);
    }
    of_filtered(&lowpass.finish(), Lowpass::rate(rate)?)
}

/// The chroma of `audio`, mono at `rate` as decoded: what [`of_audio`]
/// chooses from, for the golden vectors.
#[cfg(test)]
pub fn chroma_of_audio(audio: &[f64], rate: u32) -> Option<[f64; chroma::BANDS]> {
    let mut lowpass = Lowpass::new(rate)?;
    for &x in audio {
        lowpass.push(x);
    }
    chroma::chroma(&lowpass.finish(), Lowpass::rate(rate)?)
}
