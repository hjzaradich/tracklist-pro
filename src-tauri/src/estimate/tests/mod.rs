//! Behaviour tests for the app's own estimates (ROADMAP 3.7; 3C-E1a). The
//! audio is generated in code (click tracks at known tempos, silence),
//! never committed and never real music.

// The fingerprint tests' audio writers, used as they are.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../../fingerprint/tests/audio.rs"]
pub mod audio;
mod detector;
#[cfg(windows)]
mod job;
mod key;
#[cfg(windows)]
mod key_job;
// The quality tests' temporary music folder, volume and walk-and-hash.
#[cfg(windows)]
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../../quality/tests/support.rs"]
pub mod support;

use audio::Pcm;

/// A click track: a short, decaying 2 kHz burst on every beat at `bpm`,
/// mono, 16-bit, `seconds` long.
pub fn clicks(bpm: f64, seconds: f64, rate: u32) -> Pcm {
    let frames = (seconds * f64::from(rate)) as usize;
    let mut samples = vec![0i16; frames];
    let period = 60.0 / bpm * f64::from(rate);
    let click = (0.01 * f64::from(rate)) as usize;
    let mut beat = 0.0f64;
    while (beat as usize) < frames {
        let at = beat.round() as usize;
        for i in 0..click.min(frames.saturating_sub(at)) {
            let t = i as f64 / f64::from(rate);
            let x = 0.8 * (-t * 400.0).exp() * (std::f64::consts::TAU * 2000.0 * t).sin();
            samples[at + i] = (x * 32_767.0).round() as i16;
        }
        beat += period;
    }
    Pcm {
        rate,
        channels: 1,
        samples,
    }
}

/// `seconds` of digital silence.
pub fn silence(seconds: f64, rate: u32) -> Pcm {
    Pcm {
        rate,
        channels: 1,
        samples: vec![0; (seconds * f64::from(rate)) as usize],
    }
}

/// A source over bytes in memory, for the decoder.
pub fn source(bytes: &[u8]) -> Box<dyn symphonia::core::io::MediaSource> {
    Box::new(std::io::Cursor::new(bytes.to_vec()))
}
