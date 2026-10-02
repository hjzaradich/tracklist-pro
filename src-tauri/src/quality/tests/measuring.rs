//! What one decode pass measures, on audio generated in the test.

use super::{audio, low_passed, noise, pcm, source, tone, wav};
use crate::fingerprint::decode::Stopped;
use crate::fingerprint::Unfingerprintable;
use crate::quality::{measure, Ended, Errors, Failure, Gap, Measured};

const RATE: u32 = 44_100;

fn measured(bytes: &[u8]) -> Measured {
    measure(source(bytes), &mut |_| true).expect("the file decodes")
}

fn cutoff_of(x: &[f64]) -> Result<u32, Gap> {
    measured(&wav(x, RATE)).cutoff
}

#[test]
fn white_noise_low_passed_at_16_khz_measures_a_cutoff_near_16_khz() {
    let x = low_passed(&noise(1, 20.0, RATE, 0.1), RATE, 16_000.0);
    let hz = cutoff_of(&x).expect("it can be measured");
    assert!((15_300..=16_700).contains(&hz), "{hz} Hz");
}

#[test]
fn low_passing_at_other_frequencies_moves_the_cutoff_with_it() {
    for at in [11_000.0, 16_000.0, 19_000.0] {
        let x = low_passed(&noise(2, 12.0, RATE, 0.1), RATE, at);
        let hz = f64::from(cutoff_of(&x).unwrap());
        assert!((hz - at).abs() < 700.0, "cut at {at} Hz, measured {hz} Hz");
    }
}

#[test]
fn full_band_noise_measures_a_cutoff_near_the_top_of_the_spectrum() {
    let hz = cutoff_of(&noise(3, 20.0, RATE, 0.1)).unwrap();
    // The Nyquist limit is 22,050 Hz; the smoothing keeps 250 Hz off it.
    assert!(hz >= 21_500, "{hz} Hz");
}

#[test]
fn a_quiet_but_full_band_signal_is_not_reported_as_cut_off() {
    // -60 dB: the level is nothing like the 2-8 kHz reference of a loud
    // file, but the spectrum still runs to the top.
    for level in [0.001, 0.0003] {
        let hz = cutoff_of(&noise(4, 20.0, RATE, level)).unwrap();
        assert!(hz >= 21_000, "rms {level}: {hz} Hz");
    }
}

#[test]
fn the_sample_skips_the_first_30_seconds_of_a_long_file() {
    // An intro low-passed at 9 kHz, then full-band: only the rest says
    // anything about how the file was encoded.
    let mut x = low_passed(&noise(5, 30.0, RATE, 0.1), RATE, 9_000.0);
    x.extend(noise(6, 50.0, RATE, 0.1));
    let hz = cutoff_of(&x).unwrap();
    assert!(hz >= 21_500, "{hz} Hz");
}

#[test]
fn only_60_seconds_from_the_30_second_mark_are_looked_at() {
    // Full-band until 90 s, then low-passed: that part is never reached.
    let mut x = noise(7, 90.0, RATE, 0.1);
    x.extend(low_passed(&noise(8, 30.0, RATE, 0.1), RATE, 9_000.0));
    let hz = cutoff_of(&x).unwrap();
    assert!(hz >= 21_500, "{hz} Hz");
}

#[test]
fn a_short_file_is_measured_from_its_start() {
    // 12 s is under 50 s: nothing is skipped.
    let mut x = low_passed(&noise(9, 6.0, RATE, 0.1), RATE, 12_000.0);
    x.extend(low_passed(&noise(10, 6.0, RATE, 0.1), RATE, 12_000.0));
    let hz = f64::from(cutoff_of(&x).unwrap());
    assert!((hz - 12_000.0).abs() < 700.0, "{hz} Hz");
}

#[test]
fn audio_too_short_to_measure_has_no_cutoff() {
    let m = measured(&wav(&noise(11, 3.0, RATE, 0.1), RATE));
    assert_eq!(m.cutoff, Err(Gap::TooShort));
    // It still decoded: the other measurements are there.
    assert!((2_990..=3_010).contains(&m.decoded_ms), "{}", m.decoded_ms);
}

#[test]
fn silent_audio_has_no_cutoff() {
    let m = measured(&wav(&vec![0.0; RATE as usize * 20], RATE));
    assert_eq!(m.cutoff, Err(Gap::Silent));
}

#[test]
fn a_very_quiet_hiss_below_minus_90_db_counts_as_silence() {
    assert_eq!(
        cutoff_of(&noise(12, 20.0, RATE, 0.000_01)),
        Err(Gap::Silent)
    );
}

#[test]
fn a_loud_tone_over_a_quiet_full_band_hiss_still_measures_where_the_hiss_ends() {
    // The reference is the median of 2-8 kHz, so one strong tone doesn't
    // set it.
    let hiss = noise(21, 20.0, RATE, 0.003);
    let x: Vec<f64> = tone(1_000.0, 20.0, RATE, 0.5)
        .iter()
        .zip(&hiss)
        .map(|(t, h)| t + h)
        .collect();
    let hz = cutoff_of(&x).unwrap();
    assert!(hz >= 21_000, "{hz} Hz");
}

#[test]
fn a_low_sample_rate_without_a_2_to_8_khz_band_says_so() {
    let m = measured(&wav(&noise(13, 20.0, 4_000, 0.1), 4_000));
    assert_eq!(m.cutoff, Err(Gap::LowRate));
}

#[test]
fn a_whole_file_decodes_as_long_as_its_header_claims() {
    let m = measured(&wav(&noise(14, 20.0, RATE, 0.1), RATE));
    assert_eq!(m.decoded_ms, 20_000);
    assert_eq!(m.header_ms, Some(20_000));
    assert_eq!(m.errors, Errors::default());
    assert_eq!(m.ended, Ended::Complete);
}

#[test]
fn a_truncated_file_reports_a_decoded_duration_shorter_than_its_headers() {
    let whole = wav(&noise(15, 20.0, RATE, 0.1), RATE);
    // The header still says 20 s; the file stops at about 8 s.
    let cut = &whole[..44 + RATE as usize * 2 * 8];
    let m = measured(cut);
    assert_eq!(m.header_ms, Some(20_000));
    assert!((7_900..=8_100).contains(&m.decoded_ms), "{}", m.decoded_ms);
    assert!(m.decoded_ms < m.header_ms.unwrap());
}

#[test]
fn a_file_that_stops_early_says_how_it_ended() {
    let whole = wav(&noise(16, 20.0, RATE, 0.1), RATE);
    let m = measured(&whole[..whole.len() / 2]);
    assert_eq!(m.ended, Ended::CutShort);
}

/// An MP3 with a byte flipped every 997 bytes: some of its frames fail to
/// decode. (LAME builds on Windows only.)
#[cfg(windows)]
fn mp3_with_damaged_frames(seconds: f64) -> (Vec<u8>, Vec<u8>) {
    let whole = audio::mp3(&pcm(&noise(17, seconds, RATE, 0.1), RATE), 192);
    let mut damaged = whole.clone();
    for at in (1000..damaged.len()).step_by(997) {
        damaged[at] ^= 0x81;
    }
    (whole, damaged)
}

#[cfg(windows)]
#[test]
fn a_corrupt_frame_is_counted_as_a_decode_error_and_the_rest_is_still_measured() {
    let (whole, damaged) = mp3_with_damaged_frames(30.0);
    let clean = measured(&whole);
    assert_eq!(clean.errors, Errors::default());

    let m = measured(&damaged);
    assert!(m.errors.decode >= 1, "{:?}", m.errors);
    assert_eq!(
        m.errors.total(),
        i64::from(m.errors.decode + m.errors.container)
    );
    // The frames that failed are missing from what decoded, and the rest
    // was still read to the end and measured.
    assert!(m.decoded_ms < clean.decoded_ms, "{} ms", m.decoded_ms);
    assert!(
        m.decoded_ms > clean.decoded_ms * 9 / 10,
        "{} ms",
        m.decoded_ms
    );
    assert!(m.cutoff.is_ok());
    assert_eq!(m.ended, Ended::Complete);
}

#[test]
fn a_damaged_frame_the_decoder_skips_silently_still_shows_as_missing_time() {
    // The decoder resynchronizes past some damage without reporting it as
    // an error; the shortfall against the header's duration is what shows.
    let whole = audio::flac(&pcm(&noise(17, 30.0, RATE, 0.1), RATE));
    let mut damaged = whole.clone();
    let at = damaged.len() / 2;
    for b in &mut damaged[at..at + 40] {
        *b ^= 0x5A;
    }
    let clean = measured(&whole);
    assert_eq!(clean.decoded_ms, clean.header_ms.unwrap());
    let m = measured(&damaged);
    assert_eq!(m.header_ms, clean.header_ms);
    assert!(m.decoded_ms < m.header_ms.unwrap(), "{m:?}");
    // One FLAC block is 4096 frames, 93 ms; no more than a few were lost.
    assert!(m.header_ms.unwrap() - m.decoded_ms < 500, "{m:?}");
    assert!(m.cutoff.is_ok());
}

#[test]
fn every_decode_error_is_filed_under_its_kind() {
    assert_eq!(Errors::default().kinds_json(), None);
    assert_eq!(
        Errors {
            decode: 3,
            container: 0
        }
        .kinds_json()
        .as_deref(),
        Some(r#"{"decode":3}"#)
    );
    let both = Errors {
        decode: 2,
        container: 1,
    };
    assert_eq!(both.total(), 3);
    let json: serde_json::Value = serde_json::from_str(&both.kinds_json().unwrap()).unwrap();
    assert_eq!(json["decode"], 2);
    assert_eq!(json["container"], 1);
}

#[test]
fn a_file_with_nothing_decodable_is_a_failure_not_a_measurement() {
    let text = b"This is a text file, not a song.\n".repeat(200);
    match measure(source(&text), &mut |_| true) {
        Err(Stopped::Failed(why)) => {
            assert_eq!(why, Unfingerprintable::UnsupportedFormat);
            assert_eq!(Failure::of(why), Some(Failure::UnsupportedFormat));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_the_os_could_not_read_is_never_recorded_as_a_failure() {
    assert_eq!(Failure::of(Unfingerprintable::Unreadable), None);
    for why in Unfingerprintable::ALL {
        if why != Unfingerprintable::Unreadable {
            assert!(Failure::of(why).is_some(), "{why}");
        }
    }
}

#[test]
fn cancelling_stops_the_decode_and_measures_nothing() {
    let bytes = audio::flac(&pcm(&noise(18, 60.0, RATE, 0.1), RATE));
    let mut packets = 0;
    let result = measure(source(&bytes), &mut |_| {
        packets += 1;
        packets <= 5
    });
    assert!(matches!(result, Err(Stopped::Cancelled)), "{result:?}");
    assert_eq!(packets, 6, "it stopped at the next packet");
}

#[test]
fn stereo_is_measured_as_the_mix_of_its_channels() {
    // Left full-band, right low-passed: the mix still reaches the top.
    let left = noise(19, 12.0, RATE, 0.1);
    let right = low_passed(&noise(20, 12.0, RATE, 0.1), RATE, 10_000.0);
    let samples = left
        .iter()
        .zip(&right)
        .flat_map(|(&l, &r)| [l, r])
        .map(|v| (v * 32_767.0).round() as i16)
        .collect();
    let stereo = audio::Pcm {
        rate: RATE,
        channels: 2,
        samples,
    };
    let hz = measured(&audio::wav(&stereo)).cutoff.unwrap();
    assert!(hz >= 21_000, "{hz} Hz");
}
