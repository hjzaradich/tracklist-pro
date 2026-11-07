//! The tempo detector and the window, on audio decoded from memory.

use super::{audio, clicks, silence, source};
use crate::estimate::bpm::{fold, Tempo};
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
    for (bpm, shown) in [(100.0, 100.0), (128.0, 128.0), (174.0, 174.0), (87.0, 174.0)] {
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
    assert_eq!(fold(195.0, r), 97.5, "the top of the range is out of it");
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
