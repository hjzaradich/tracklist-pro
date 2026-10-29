//! Behavior tests for waveforms (1cB-9). The audio is generated in code
//! (a tone that is quiet and then loud), never committed and never real
//! music.

// The fingerprint tests' audio writers (WAV, AIFF, FLAC, M4A…), used as they
// are.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../../fingerprint/tests/audio.rs"]
pub mod audio;
#[cfg(windows)]
mod job;
mod peaks;
// The quality tests' temporary music folder, volume and walk-and-hash.
#[cfg(windows)]
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../../quality/tests/support.rs"]
pub mod support;

use std::f64::consts::TAU;

use audio::Pcm;

/// A 440 Hz tone, mono, 16-bit: quiet (amplitude 0.1) for the first half,
/// loud (0.8) for the second.
pub fn shaped(seconds: f64, rate: u32) -> Pcm {
    let frames = (seconds * f64::from(rate)) as usize;
    let samples = (0..frames)
        .map(|i| {
            let amplitude = if i < frames / 2 { 0.1 } else { 0.8 };
            let x = amplitude * (TAU * 440.0 * i as f64 / f64::from(rate)).sin();
            (x * 32_767.0).round() as i16
        })
        .collect();
    Pcm {
        rate,
        channels: 1,
        samples,
    }
}

/// A source over bytes in memory, for the decoder.
pub fn source(bytes: &[u8]) -> Box<dyn symphonia::core::io::MediaSource> {
    Box::new(std::io::Cursor::new(bytes.to_vec()))
}

/// The peak and the level of each bucket.
pub fn split(peaks: &[u8]) -> (Vec<u8>, Vec<u8>) {
    peaks
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[p, l]| (p, l))
        .unzip()
}

#[test]
fn the_player_can_ask_for_peaks_and_the_three_answers_are_in_the_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bindings.ts");
    crate::ipc::export_bindings(&path).unwrap();
    let ts = std::fs::read_to_string(&path).unwrap();
    for expected in [
        "waveformPeaks: (fileId: number)",
        r#""waveform_peaks""#,
        "export type Waveform =",
        r#""ready""#,
        r#""notReady""#,
        r#""unavailable""#,
        "durationMs: number",
        "levels: number[]",
    ] {
        assert!(ts.contains(expected), "missing `{expected}` in:\n{ts}");
    }
}
