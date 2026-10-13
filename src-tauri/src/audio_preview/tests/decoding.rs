//! AIFF and ALAC as a WAV that is computed, seeked and bounded (1cA-8).

use std::io::Cursor;

use symphonia::core::io::MediaSource;

use super::aiff::Made;
use crate::audio_preview::wav::{Decoded, Layout, HEADER_LEN};
use crate::audio_preview::{PreviewRefusal, MAX_CHUNK};
use crate::fingerprint::decode::{walk, Flow};
use crate::fingerprint::tests::audio::{m4a_alac, pcm, wav};

fn decoded(source: impl MediaSource + 'static) -> Decoded {
    Decoded::from_source(Box::new(source)).expect("a file the preview can decode")
}

fn refused(source: impl MediaSource + 'static) -> PreviewRefusal {
    match Decoded::from_source(Box::new(source)) {
        Ok(_) => panic!("expected the file to be refused"),
        Err(refusal) => refusal,
    }
}

/// The whole WAV, asked for `window` bytes at a time, as a media element
/// asks for ranges.
fn read_all(decoded: &mut Decoded, window: u64) -> Vec<u8> {
    let total = decoded.layout.total();
    let mut out = Vec::new();
    let mut at = 0;
    while at < total {
        let end = (at + window - 1).min(total - 1);
        out.extend(decoded.read(at, end).unwrap());
        at = end + 1;
    }
    out
}

/// What the existing decoder (the one the fingerprints use) gives for
/// `bytes`: every sample, 16-bit, interleaved.
fn existing_decoder_samples(bytes: &[u8]) -> Vec<i16> {
    let mut samples = Vec::new();
    let mut block = Vec::new();
    walk(
        Box::new(Cursor::new(bytes.to_vec())),
        &mut |_| true,
        &mut |b| {
            b.audio.copy_to_vec_interleaved(&mut block);
            samples.extend_from_slice(&block);
            Ok(Flow::Continue)
        },
    )
    .unwrap();
    samples
}

fn samples_of_wav(wav: &[u8]) -> Vec<i16> {
    let (pairs, _) = wav[HEADER_LEN as usize..].as_chunks::<2>();
    pairs.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

#[test]
fn a_16_bit_stereo_aiff_comes_out_as_the_wav_of_the_same_samples() {
    let aiff = Made::new(44_100, 2, 16, 20_000);
    let expected = aiff.as_wav();
    let mut d = decoded(Cursor::new(aiff.bytes()));
    assert_eq!(d.layout.total(), expected.len() as u64);
    assert_eq!(read_all(&mut d, 4_097), expected);
}

#[test]
fn a_24_bit_aiff_keeps_its_24_bits() {
    let aiff = Made::new(48_000, 1, 24, 30_000);
    let expected = aiff.as_wav();
    let mut d = decoded(Cursor::new(aiff.bytes()));
    assert_eq!(d.layout.bytes, 3);
    assert_eq!(read_all(&mut d, 10_001), expected);
}

#[test]
fn aiff_gives_the_same_samples_the_existing_decoder_gives() {
    let bytes = Made::new(44_100, 2, 16, 15_000).bytes();
    let mut d = decoded(Cursor::new(bytes.clone()));
    let ours = samples_of_wav(&read_all(&mut d, 8_192));
    assert_eq!(ours, existing_decoder_samples(&bytes));
}

#[test]
fn alac_comes_out_as_the_wav_of_the_same_samples_and_the_existing_decoder_agrees() {
    let song = pcm(11, 3.0, 44_100, 1);
    let m4a = m4a_alac(&song);
    let mut d = decoded(Cursor::new(m4a.clone()));
    let ours = read_all(&mut d, 6_001);
    assert_eq!(ours, wav(&song));
    assert_eq!(samples_of_wav(&ours), existing_decoder_samples(&m4a));
}

#[test]
fn a_seek_into_the_middle_returns_the_right_samples() {
    let aiff = Made::new(44_100, 2, 16, 200_000);
    let expected = aiff.as_wav();
    let mut d = decoded(Cursor::new(aiff.bytes()));
    // Odd offsets, one that spans the header and the data, and ones going
    // backwards: each is its own request, with no state from the last.
    for (from, to) in [
        (400_001u64, 450_000u64),
        (30, 100),
        (0, 43),
        (44, 44),
        (123_457, 123_457),
        (700_000, 710_003),
        (10, 20_000),
    ] {
        let got = d.read(from, to).unwrap();
        assert_eq!(got, expected[from as usize..=to as usize], "{from}..={to}");
    }
}

#[test]
fn a_seek_into_alac_returns_the_right_samples() {
    let song = pcm(12, 4.0, 44_100, 1);
    let expected = wav(&song);
    let m4a = m4a_alac(&song);
    // A fresh reader per request, as the protocol makes one.
    for (from, to) in [
        (100_001u64, 130_000u64),
        (60, 70),
        (expected.len() as u64 - 5000, expected.len() as u64 - 1),
        (8_188, 8_194),
        (20_000, 20_001),
    ] {
        let mut d = decoded(Cursor::new(m4a.clone()));
        let got = d.read(from, to).unwrap();
        assert_eq!(got, expected[from as usize..=to as usize], "{from}..={to}");
    }
}

#[test]
fn the_last_partial_range_of_a_decoded_file_is_the_rest_of_it() {
    let aiff = Made::new(44_100, 2, 16, 10_000);
    let expected = aiff.as_wav();
    let mut d = decoded(Cursor::new(aiff.bytes()));
    let total = d.layout.total();
    let got = d.read(total - 7, total - 1).unwrap();
    assert_eq!(got, expected[expected.len() - 7..]);
}

#[test]
fn a_header_claiming_more_audio_than_the_file_holds_gets_the_length_the_data_has() {
    // The header says 50,000 frames and the data holds 30,000: the
    // container reader trusts the data, and so does the WAV's length.
    let aiff = Made::claiming(44_100, 2, 16, 30_000, 50_000);
    let expected = Made::new(44_100, 2, 16, 30_000).as_wav();
    let mut d = decoded(aiff);
    assert_eq!(d.layout.frames, 30_000);
    assert_eq!(read_all(&mut d, 20_000), expected);
}

#[test]
fn a_cut_off_aiff_whose_header_runs_past_the_end_is_not_decodable() {
    let full = Made::new(44_100, 2, 16, 1_000).bytes();
    assert_eq!(
        refused(Cursor::new(full[..full.len() / 2].to_vec())),
        PreviewRefusal::NotDecodable
    );
}

#[test]
fn more_than_two_channels_is_not_decodable() {
    assert_eq!(
        refused(Made::new(48_000, 3, 16, 1_000)),
        PreviewRefusal::NotDecodable
    );
}

#[test]
fn a_wav_that_would_pass_four_gib_is_not_decodable() {
    let fits = |frames| {
        Layout {
            rate: 96_000,
            channels: 2,
            bytes: 3,
            frames,
        }
        .fits()
    };
    assert!(fits(700_000_000));
    assert!(!fits(716_000_000));
}

#[test]
fn bytes_that_are_not_audio_are_not_decodable() {
    assert_eq!(
        refused(Cursor::new(Vec::<u8>::new())),
        PreviewRefusal::NotDecodable
    );
    assert_eq!(
        refused(Cursor::new(b"not audio, only words".repeat(50))),
        PreviewRefusal::NotDecodable
    );
}

/// A decoded packet is a few thousand frames at most.
const PACKET_SLACK_FRAMES: u64 = 16_384;

#[test]
fn memory_stays_bounded_on_an_hour_of_96_khz_24_bit_stereo() {
    // 2.07 GB as a WAV; holding a whole ten-minute track of it would be
    // 330 MB. Made as it is read: nothing here is that big.
    let frames = 3_600 * 96_000;
    let aiff = Made::new(96_000, 2, 24, frames);
    let mut d = decoded(aiff);
    let total = d.layout.total();
    assert_eq!(total, HEADER_LEN + frames * 6);
    assert!(total > 2_000_000_000);

    let chunk_frames = MAX_CHUNK / 6;
    for start in [
        HEADER_LEN,
        total / 2,
        total / 2 + 1,
        total - MAX_CHUNK,
        // The last, partial chunk.
        total - 1000,
    ] {
        let end = (start + MAX_CHUNK - 1).min(total - 1);
        let before = d.decoded_frames;
        let got = d.read(start, end).unwrap();
        // The buffer is the chunk and no more.
        assert_eq!(got.len() as u64, end - start + 1);
        assert!(got.len() as u64 <= MAX_CHUNK);
        assert!(
            got.capacity() as u64 <= MAX_CHUNK + 64,
            "{}",
            got.capacity()
        );
        // The decode started at the range, not at the start of the track.
        let decoded_for_it = d.decoded_frames - before;
        assert!(
            decoded_for_it <= chunk_frames + 2 * PACKET_SLACK_FRAMES,
            "decoded {decoded_for_it} frames for a {chunk_frames}-frame chunk"
        );
        // And it is the right audio.
        let first_frame = (start - HEADER_LEN) / 6;
        for probe in [0u64, 1, 2, 777] {
            let frame = first_frame + probe + 1;
            let at = (HEADER_LEN + frame * 6 - start) as usize;
            if at + 6 > got.len() {
                continue;
            }
            for channel in 0..2u16 {
                let s = super::aiff::sample(frame, channel, 24);
                let o = at + usize::from(channel) * 3;
                assert_eq!(&got[o..o + 3], &s.to_le_bytes()[..3], "frame {frame}");
            }
        }
    }
}
