//! Decoding: every format the app reads gives a fingerprint of the right
//! length, and anything that can't be fingerprinted ends with a reason,
//! never a panic.

use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;

use super::audio;
use super::decode_bytes;
use crate::fingerprint::decode::{fingerprint, Stopped};
use crate::fingerprint::Unfingerprintable;
use crate::tags::test_audio;
use symphonia::core::io::MediaSource;

fn failed(bytes: &[u8]) -> Unfingerprintable {
    match decode_bytes(bytes) {
        Err(Stopped::Failed(why)) => why,
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn wav_flac_aiff_alac_and_mp3_each_give_a_fingerprint_as_long_as_the_audio() {
    let seconds = 20.0;
    let mono = audio::pcm(5, seconds, 44_100, 1);
    let stereo = audio::pcm(5, seconds, 44_100, 2);
    #[allow(unused_mut)]
    let mut files = vec![
        ("WAV", audio::wav(&stereo)),
        ("FLAC", audio::flac(&stereo)),
        ("AIFF", audio::aiff(&stereo)),
        ("M4A (ALAC)", audio::m4a_alac(&mono)),
    ];
    #[cfg(windows)]
    files.push(("MP3", audio::mp3(&stereo, 192)));
    for (name, bytes) in files {
        let fp = decode_bytes(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // chromaprint's filters use up the first ~2.5 s.
        let covered = f64::from(fp.seconds());
        assert!(
            (seconds - 3.5..=seconds).contains(&covered),
            "{name} covers {covered} s of {seconds}"
        );
    }
}

#[test]
fn an_opus_file_has_no_fingerprint_because_its_codec_is_unsupported() {
    assert_eq!(
        failed(&test_audio::ogg_opus()),
        Unfingerprintable::UnsupportedCodec
    );
}

#[test]
fn bytes_that_are_not_audio_are_an_unsupported_format() {
    assert_eq!(
        failed(b"This is a text file, not a song.\n".repeat(200).as_slice()),
        Unfingerprintable::UnsupportedFormat
    );
    assert!(matches!(decode_bytes(b""), Err(Stopped::Failed(_))));
}

#[test]
fn an_m4a_whose_download_was_cut_off_before_its_index_has_no_fingerprint() {
    // An interrupted download: `ftyp` and audio, no `moov` (ROADMAP 1.1).
    let full = audio::m4a_alac(&audio::pcm(6, 10.0, 22_050, 1));
    let no_moov = [&full[..24], b"\0\0\0\x10mdat\0\0\0\0\0\0\0\0"].concat();
    assert!(matches!(decode_bytes(&no_moov), Err(Stopped::Failed(_))));
}

#[test]
fn a_clip_under_three_seconds_is_too_short_to_fingerprint() {
    let clip = audio::wav(&audio::pcm(7, 2.0, 22_050, 1));
    assert_eq!(failed(&clip), Unfingerprintable::TooShort);
}

#[test]
fn a_file_cut_short_is_fingerprinted_as_far_as_it_goes() {
    let whole = audio::flac(&audio::pcm(8, 30.0, 22_050, 1));
    let half = &whole[..whole.len() / 2];
    let fp = decode_bytes(half).expect("the first half still decodes");
    assert!((10.0..17.0).contains(&fp.seconds()), "{}", fp.seconds());
}

/// Damages `bytes` in one of several ways, chosen by `n`.
fn damage(bytes: &[u8], n: u64) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut state = n.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let len = out.len() as u64;
    match n % 5 {
        // Truncated anywhere, header included.
        0 => out.truncate((next() % len) as usize),
        // Random bytes flipped all over.
        1 => {
            for _ in 0..(1 + next() % 64) {
                let at = (next() % len) as usize;
                out[at] ^= (next() % 255 + 1) as u8;
            }
        }
        // A run of zeros.
        2 => {
            let at = (next() % len) as usize;
            let end = (at + 1 + (next() % 4096) as usize).min(out.len());
            out[at..end].fill(0);
        }
        // Garbage in the header.
        3 => {
            for b in out.iter_mut().take(64) {
                *b = next() as u8;
            }
        }
        // Everything after the header replaced with noise.
        _ => {
            for b in out.iter_mut().skip(64) {
                *b = next() as u8;
            }
        }
    }
    out
}

#[test]
fn damaged_audio_in_every_format_never_panics_and_ends_with_a_fingerprint_or_a_reason() {
    let pcm = audio::pcm(9, 6.0, 22_050, 1);
    #[allow(unused_mut)]
    let mut formats = vec![
        ("WAV", audio::wav(&pcm)),
        ("FLAC", audio::flac(&pcm)),
        ("AIFF", audio::aiff(&pcm)),
        ("M4A", audio::m4a_alac(&pcm)),
        ("Ogg Vorbis", test_audio::ogg_vorbis()),
        ("M4A (AAC)", test_audio::m4a()),
    ];
    #[cfg(windows)]
    formats.push(("MP3", audio::mp3(&pcm, 128)));
    let mut outcomes = std::collections::BTreeMap::<String, usize>::new();
    for (name, bytes) in &formats {
        for n in 0..60 {
            let damaged = damage(bytes, n);
            let result = panic::catch_unwind(AssertUnwindSafe(|| decode_bytes(&damaged)));
            let outcome = match result {
                Ok(Err(Stopped::Failed(Unfingerprintable::Unreadable))) => {
                    panic!("{name} #{n}: damaged bytes in memory aren't unreadable")
                }
                Ok(Ok(_)) => "fingerprint".to_owned(),
                Ok(Err(Stopped::Failed(why))) => why.reason().to_owned(),
                Ok(Err(Stopped::Cancelled)) => panic!("{name} #{n}: nothing cancelled it"),
                Err(_) => panic!("{name} #{n}: decoding damaged audio panicked"),
            };
            *outcomes.entry(outcome).or_default() += 1;
        }
    }
    println!("{outcomes:?}");
    assert!(outcomes.len() > 1, "{outcomes:?}");
}

#[test]
fn one_damaged_frame_in_a_long_file_costs_that_frame_not_the_fingerprint() {
    let mut flac = audio::flac(&audio::pcm(13, 120.0, 22_050, 1));
    // Scribble over a stretch in the middle: a frame or two fail their
    // checks, and Symphonia reports some of that as I/O errors.
    let middle = flac.len() / 2;
    for (i, b) in flac[middle..middle + 3000].iter_mut().enumerate() {
        *b = (i * 37 % 251) as u8;
    }
    let fp = decode_bytes(&flac).expect("the rest still decodes");
    assert!(fp.seconds() > 110.0, "{}", fp.seconds());
}

/// Bytes that read fine up to `fail_at`, then the OS reports an error, as
/// when a drive is unplugged mid-read.
struct FailingDisk {
    bytes: Cursor<Vec<u8>>,
    fail_at: u64,
}

impl Read for FailingDisk {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.bytes.position() >= self.fail_at {
            return Err(io::Error::other("the device is not ready"));
        }
        let room = (self.fail_at - self.bytes.position()) as usize;
        let n = buf.len().min(room);
        self.bytes.read(&mut buf[..n])
    }
}

impl Seek for FailingDisk {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.bytes.seek(to)
    }
}

impl MediaSource for FailingDisk {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.bytes.get_ref().len() as u64)
    }
}

#[test]
fn a_read_the_os_fails_is_unreadable_not_damaged() {
    let bytes = audio::flac(&audio::pcm(14, 30.0, 22_050, 1));
    for fail_at in [0, 20, bytes.len() as u64 / 2] {
        let disk = FailingDisk {
            fail_at,
            bytes: Cursor::new(bytes.clone()),
        };
        assert_eq!(
            fingerprint(Box::new(disk), &mut |_| true),
            Err(Stopped::Failed(Unfingerprintable::Unreadable)),
            "failing at byte {fail_at}"
        );
    }
}

#[test]
fn saying_stop_ends_the_decode_before_the_next_packet() {
    let bytes = audio::flac(&audio::pcm(10, 60.0, 22_050, 1));
    let mut asked = 0;
    let started = Instant::now();
    let result = fingerprint(Box::new(Cursor::new(bytes)), &mut |_| {
        asked += 1;
        asked <= 20
    });
    assert_eq!(result, Err(Stopped::Cancelled));
    assert_eq!(asked, 21, "asked once per packet, and not again after no");
    assert!(started.elapsed().as_secs() < 5);
}

#[test]
fn progress_through_a_file_rises_steadily_to_the_end() {
    let bytes = audio::wav(&audio::pcm(12, 20.0, 22_050, 1));
    let mut seen = Vec::new();
    fingerprint(Box::new(Cursor::new(bytes)), &mut |fraction| {
        seen.push(fraction.expect("a WAV knows its length"));
        true
    })
    .unwrap();
    assert_eq!(seen[0], 0.0);
    assert!(seen.windows(2).all(|w| w[1] >= w[0]), "{seen:?}");
    assert!(*seen.last().unwrap() > 0.99, "{seen:?}");
}
