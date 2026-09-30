//! Speed: seconds of work per minute of audio, decode included (ROADMAP
//! 1.4 budgets about 1 s per track per core).

use std::time::Instant;

use super::audio;
use super::decode_bytes;

/// Fingerprints `bytes` (`minutes` of audio) and returns seconds per minute.
fn seconds_per_minute(bytes: &[u8], minutes: f64) -> f64 {
    let started = Instant::now();
    decode_bytes(bytes).expect("the test audio decodes");
    started.elapsed().as_secs_f64() / minutes
}

#[test]
fn fingerprinting_takes_under_a_second_per_minute_of_cd_audio() {
    let minutes = 1.0;
    let cd = audio::pcm(90, minutes * 60.0, 44_100, 2);
    #[allow(unused_mut)]
    let mut files = vec![("FLAC", audio::flac(&cd)), ("WAV", audio::wav(&cd))];
    #[cfg(windows)]
    files.push(("MP3 320", audio::mp3(&cd, 320)));
    let optimized = !cfg!(debug_assertions);
    for (name, bytes) in files {
        let rate = seconds_per_minute(&bytes, minutes);
        println!(
            "{name}, 44.1 kHz stereo: {rate:.3} s per minute of audio ({} build)",
            if optimized { "release" } else { "debug" }
        );
        // Measured in release on a Windows PC (2026-09-29): 0.27 (WAV),
        // 0.43 (FLAC) and 0.64 (MP3 320) s per minute. The limit leaves
        // room for a busy machine. Debug builds only check it finishes.
        let limit = if optimized { 1.0 } else { 30.0 };
        assert!(rate < limit, "{name}: {rate:.3} s per minute");
    }
}
