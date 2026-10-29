//! Reducing a decoded track to peaks, without the database.

use super::{audio, shaped, source, split};
use crate::fingerprint::decode::{Stopped, Unfingerprintable};
use crate::waveform::peaks::{compute, Accumulator, BUCKETS};

const RATE: u32 = 44_100;

/// The value at `fraction` of the way along `values`.
fn at(values: &[u8], fraction: f64) -> u8 {
    values[(fraction * values.len() as f64) as usize]
}

#[test]
fn a_quiet_stretch_and_a_loud_stretch_show_as_low_and_high_peaks_with_the_level_below_the_peak() {
    let wav = audio::wav(&shaped(10.0, RATE));
    let computed = compute(source(&wav), &mut |_| true).unwrap();
    assert_eq!(computed.duration_ms, 10_000);
    let (peaks, levels) = split(&computed.peaks);
    assert_eq!(peaks.len(), BUCKETS);
    // 0.1 and 0.8 of full scale, in bytes of 255; a sine's RMS is 0.707 of
    // its peak.
    assert!(
        (24..=27).contains(&at(&peaks, 0.25)),
        "{}",
        at(&peaks, 0.25)
    );
    assert!(
        (202..=205).contains(&at(&peaks, 0.75)),
        "{}",
        at(&peaks, 0.75)
    );
    assert!(
        (16..=20).contains(&at(&levels, 0.25)),
        "{}",
        at(&levels, 0.25)
    );
    assert!(
        (139..=149).contains(&at(&levels, 0.75)),
        "{}",
        at(&levels, 0.75)
    );
}

#[test]
fn a_track_gets_at_most_1024_buckets_of_two_bytes_and_a_short_one_gets_fewer() {
    let long = compute(source(&audio::wav(&shaped(20.0, RATE))), &mut |_| true).unwrap();
    assert_eq!(long.peaks.len(), 2 * BUCKETS);
    // 3 s is 132,300 frames in slots of 256: 516 whole ones and a part.
    let short = compute(source(&audio::wav(&shaped(3.0, RATE))), &mut |_| true).unwrap();
    assert_eq!(short.peaks.len(), 2 * 517);
    assert_eq!(short.duration_ms, 3_000);
}

#[test]
fn the_peaks_do_not_depend_on_how_the_audio_is_cut_into_packets() {
    let song: Vec<f32> = audio::song(3, 9.0, RATE)
        .iter()
        .map(|&x| x as f32)
        .collect();
    // The same samples as stereo, fed whole and in odd-sized pieces.
    let stereo: Vec<f32> = song.iter().flat_map(|&x| [x, x * 0.5]).collect();
    let mut whole = Accumulator::default();
    whole.feed(&stereo, 2);
    let mut pieces = Accumulator::default();
    for piece in stereo.chunks(2 * 1_001) {
        pieces.feed(piece, 2);
    }
    assert_eq!(whole.finish(), pieces.finish());
}

#[test]
fn a_long_track_is_reduced_as_it_streams_to_the_same_number_of_buckets() {
    // Five minutes of a constant level, fed a tenth of a second at a time.
    let block = vec![0.5f32; RATE as usize / 10];
    let mut accumulator = Accumulator::default();
    for _ in 0..3_000 {
        accumulator.feed(&block, 1);
    }
    let (peaks, levels) = split(&accumulator.finish().unwrap());
    assert_eq!(peaks.len(), BUCKETS);
    // Half of full scale, at every point along it.
    assert!(peaks.iter().chain(&levels).all(|&b| b == 128), "{peaks:?}");
}

#[test]
fn a_sample_past_full_scale_counts_as_full_scale_and_silence_as_zero() {
    let mut accumulator = Accumulator::default();
    // One slot of 256 frames louder than full scale, then one of silence.
    accumulator.feed(&[1.7, -2.0].repeat(128), 1);
    accumulator.feed(&[0.0; 256], 1);
    let (peaks, levels) = split(&accumulator.finish().unwrap());
    assert_eq!((peaks[0], levels[0]), (255, 255));
    assert_eq!((peaks[1], levels[1]), (0, 0));
}

#[test]
fn a_track_with_no_samples_has_no_peaks() {
    assert_eq!(Accumulator::default().finish(), None);
}

#[test]
fn every_format_the_decoder_supports_gives_the_peaks_of_the_same_audio() {
    let pcm = shaped(8.0, RATE);
    let wav = compute(source(&audio::wav(&pcm)), &mut |_| true).unwrap();
    // Lossless formats decode to the very samples the WAV holds.
    for (format, bytes) in [
        ("AIFF", audio::aiff(&pcm)),
        ("FLAC", audio::flac(&pcm)),
        ("ALAC in M4A", audio::m4a_alac(&pcm)),
    ] {
        let computed = compute(source(&bytes), &mut |_| true)
            .unwrap_or_else(|e| panic!("{format} gave no peaks: {e:?}"));
        assert_eq!(computed, wav, "{format}");
    }
    // MP3 is lossy and starts a little late: the shape is the same.
    #[cfg(windows)]
    {
        let mp3 = compute(source(&audio::mp3(&pcm, 192)), &mut |_| true).unwrap();
        assert!(
            mp3.duration_ms.abs_diff(8_000) <= 100,
            "{} ms",
            mp3.duration_ms
        );
        let (peaks, _) = split(&mp3.peaks);
        let (quiet, loud) = (at(&peaks, 0.25), at(&peaks, 0.75));
        assert!((20..=34).contains(&quiet), "{quiet}");
        assert!((190..=240).contains(&loud), "{loud}");
    }
}

#[test]
fn a_stereo_file_shows_the_louder_channel() {
    let mono = shaped(4.0, RATE);
    let stereo = audio::Pcm {
        rate: RATE,
        channels: 2,
        // The left channel silent, the right the tone.
        samples: mono.samples.iter().flat_map(|&s| [0, s]).collect(),
    };
    let computed = compute(source(&audio::wav(&stereo)), &mut |_| true).unwrap();
    let alone = compute(source(&audio::wav(&mono)), &mut |_| true).unwrap();
    assert_eq!(split(&computed.peaks).0, split(&alone.peaks).0);
}

#[test]
fn bytes_that_are_not_audio_give_a_reason_not_peaks() {
    let garbage = vec![0x5A; 50_000];
    assert_eq!(
        compute(source(&garbage), &mut |_| true),
        Err(Stopped::Failed(Unfingerprintable::UnsupportedFormat))
    );
}

#[test]
fn a_cancel_stops_the_decode_and_gives_nothing() {
    let wav = audio::wav(&shaped(10.0, RATE));
    let mut packets = 0;
    let stopped = compute(source(&wav), &mut |_| {
        packets += 1;
        packets < 5
    });
    assert_eq!(stopped, Err(Stopped::Cancelled));
}
