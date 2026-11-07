//! The tempo detector and the window, on audio decoded from memory.

use super::{audio, clicks, scaled, silence, source};
use crate::estimate::bpm::{fold, Tempo};
use crate::estimate::features::Features;
use crate::estimate::window::{measure, Measured, Short};
use crate::settings::{BpmRange, DEFAULT_BPM_RANGE};

const RATE: u32 = 22_050;

/// What the window of `pcm`, written as a WAV, measures.
fn measured(pcm: &audio::Pcm) -> Measured {
    measure(source(&audio::wav(pcm)), &mut |_| true).unwrap()
}

fn tempo(pcm: &audio::Pcm) -> Tempo {
    match measured(pcm) {
        Measured::Window {
            tempo: Some(tempo), ..
        } => tempo,
        other => panic!("no tempo: {other:?}"),
    }
}

#[test]
fn click_tracks_at_known_tempos_are_found_within_half_a_bpm_once_folded_into_the_range() {
    for (bpm, shown) in [
        (100.0, 100.0),
        (128.0, 128.0),
        (174.0, 174.0),
        (87.0, 174.0),
    ] {
        let found = tempo(&clicks(bpm, 30.0, RATE));
        let folded = fold(found.bpm, DEFAULT_BPM_RANGE);
        assert!(
            (folded - shown).abs() <= 0.5,
            "{bpm} BPM clicks: detected {} and shown {folded}, not {shown}",
            found.bpm
        );
        assert!(
            found.confidence > 0.3 && found.confidence <= 1.0,
            "{bpm} BPM clicks: confidence {}",
            found.confidence
        );
    }
}

#[test]
fn a_tempo_is_folded_into_the_users_range_not_rekordboxs_default() {
    let range = BpmRange::new(70.0, 140.0).unwrap();
    let found = tempo(&clicks(174.0, 30.0, RATE));
    let folded = fold(found.bpm, range);
    assert!((folded - 87.0).abs() <= 0.5, "174 BPM shown as {folded}");
}

#[test]
fn folding_doubles_under_the_range_halves_at_its_top_and_rounds_to_hundredths() {
    let r = DEFAULT_BPM_RANGE;
    assert_eq!(fold(64.0, r), 128.0);
    assert_eq!(fold(60.0, r), 120.0);
    assert_eq!(fold(200.0, r), 100.0);
    // The range includes both its ends (as rekordbox's does) and the band
    // just over the top, which halving would drop under the bottom, stays.
    assert_eq!(fold(195.0, r), 195.0, "the top of the range is in it");
    assert_eq!(fold(195.5, r), 195.5);
    assert_eq!(fold(195.99, r), 195.99);
    assert_eq!(fold(196.0, r), 98.0);
    assert_eq!(fold(97.99, r), 195.98, "under the bottom: doubled");
    assert_eq!(fold(98.0, r), 98.0);
    assert_eq!(fold(127.996, r), 128.0);
    assert_eq!(fold(140.123, r), 140.12);
}

#[test]
fn silence_and_a_very_short_file_are_skipped_with_their_reason() {
    assert_eq!(
        measured(&silence(30.0, RATE)),
        Measured::Short(Short::Silent)
    );
    assert_eq!(
        measured(&clicks(128.0, 5.0, RATE)),
        Measured::Short(Short::TooShort)
    );
}

#[test]
fn the_window_skips_the_intro_of_a_long_track_and_stops_at_two_and_a_half_minutes() {
    // 100 s decoded: the first 30 s are dropped.
    let Measured::Window { analysed_ms, .. } = measured(&clicks(128.0, 100.0, 11_025)) else {
        panic!("not measured");
    };
    assert_eq!(analysed_ms, 70_000);
    // 200 s: only the first 150 s are decoded, less the intro.
    let Measured::Window { analysed_ms, .. } = measured(&clicks(128.0, 200.0, 11_025)) else {
        panic!("not measured");
    };
    assert_eq!(analysed_ms, 120_000);
    // 60 s: too short to drop an intro from.
    let Measured::Window { analysed_ms, .. } = measured(&clicks(128.0, 60.0, 11_025)) else {
        panic!("not measured");
    };
    assert_eq!(analysed_ms, 60_000);
}

#[test]
fn a_cancel_stops_the_decode_and_measures_nothing() {
    let wav = audio::wav(&clicks(128.0, 30.0, RATE));
    let mut packets = 0;
    let stopped = measure(source(&wav), &mut |_| {
        packets += 1;
        packets < 3
    });
    assert!(matches!(
        stopped,
        Err(crate::fingerprint::decode::Stopped::Cancelled)
    ));
}

// ---- the energy features -----------------------------------------------------

fn features(pcm: &audio::Pcm) -> Features {
    match measured(pcm) {
        Measured::Window { features, .. } => features,
        other => panic!("not measured: {other:?}"),
    }
}

fn lufs(pcm: &audio::Pcm) -> f64 {
    features(pcm).loudness_lufs.expect("a loudness")
}

#[test]
fn a_louder_signal_measures_more_loudness_by_about_the_gain() {
    let loud = lufs(&clicks(128.0, 30.0, RATE));
    let quiet = lufs(&scaled(clicks(128.0, 30.0, RATE), 0.1));
    assert!(loud <= 0.0 && quiet < loud, "{loud} {quiet}");
    assert!(
        ((loud - quiet) - 20.0).abs() < 1.0,
        "20 dB down measured {} LU down",
        loud - quiet
    );
}

#[test]
fn two_channels_are_measured_together_not_as_mono() {
    let mono = clicks(128.0, 30.0, RATE);
    let stereo = audio::Pcm {
        rate: RATE,
        channels: 2,
        samples: mono.samples.iter().flat_map(|s| [*s, *s]).collect(),
    };
    let more = lufs(&stereo) - lufs(&mono);
    assert!(
        (more - 3.01).abs() < 0.3,
        "the same in both channels is {more} LU up"
    );
}

#[test]
fn a_denser_signal_has_more_onsets_a_second_and_more_spectral_flux() {
    let slow = features(&clicks(90.0, 40.0, RATE));
    let fast = features(&clicks(180.0, 40.0, RATE));
    assert!(
        (slow.onset_density - 1.5).abs() < 0.3 && (fast.onset_density - 3.0).abs() < 0.5,
        "90 BPM clicks have {} onsets a second, 180 BPM {}",
        slow.onset_density,
        fast.onset_density
    );
    assert!(
        fast.spectral_flux > slow.spectral_flux * 1.5,
        "{slow:?} {fast:?}"
    );
}

#[test]
fn the_loudness_is_of_the_window_with_the_intro_dropped() {
    // 100 s: a loud 30 s intro, then quiet clicks. The window drops the intro
    // so it is as loud as the quiet part alone, not the whole.
    let rate = 11_025;
    let loud = clicks(128.0, 30.0, rate);
    let quiet = scaled(clicks(128.0, 70.0, rate), 0.1);
    let both = audio::Pcm {
        rate,
        channels: 1,
        samples: [loud.samples.clone(), quiet.samples.clone()].concat(),
    };
    let (dropped, only_quiet, whole) = (lufs(&both), lufs(&quiet), lufs(&loud));
    assert!(
        (dropped - only_quiet).abs() < 1.0 && whole - dropped > 10.0,
        "{dropped} {only_quiet} {whole}"
    );
}

#[test]
fn a_steady_tone_has_next_to_no_onsets_a_second() {
    let tone = audio::Pcm {
        rate: RATE,
        channels: 1,
        samples: (0..RATE * 20)
            .map(|i| {
                let t = f64::from(i) / f64::from(RATE);
                (0.3 * (std::f64::consts::TAU * 440.0 * t).sin() * 32_767.0) as i16
            })
            .collect(),
    };
    // Only its start, against 1.5 or more for the slowest click track.
    let density = features(&tone).onset_density;
    assert!(density < 0.5, "{density} onsets a second");
}
