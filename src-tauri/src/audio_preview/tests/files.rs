//! Files on disk through the protocol: the bytes and the headers.

use std::fs;

use super::aiff::Made;
use super::support::Lib;
use crate::audio_preview::MAX_CHUNK;
use crate::fingerprint::tests::audio::{flac, m4a_alac, pcm, wav};

/// A WAV of a few MiB, so it takes several chunks.
fn long_wav() -> Vec<u8> {
    wav(&pcm(5, 40.0, 44_100, 1))
}

#[test]
fn a_range_of_a_native_file_returns_exactly_those_bytes_with_the_right_headers() {
    let lib = Lib::new();
    let file = long_wav();
    assert!(file.len() as u64 > 3 * MAX_CHUNK);
    let id = lib.put("a.wav", &file);

    let got = lib.bytes(id, 1_000, 1_999);
    assert_eq!(got.status, 206);
    assert_eq!(got.body, file[1_000..2_000]);
    assert_eq!(
        got.header("content-range"),
        Some(format!("bytes 1000-1999/{}", file.len()).as_str())
    );
    assert_eq!(got.header("content-length"), Some("1000"));
    assert_eq!(got.header("accept-ranges"), Some("bytes"));
    assert_eq!(got.header("content-type"), Some("audio/wav"));
}

#[test]
fn the_last_partial_range_of_a_native_file_is_the_rest_of_it() {
    let lib = Lib::new();
    let file = long_wav();
    let id = lib.put("a.wav", &file);
    let total = file.len() as u64;

    let start = total - 12_345;
    let got = lib.get(id, Some(&format!("bytes={start}-")));
    assert_eq!(got.status, 206);
    assert_eq!(got.body, file[start as usize..]);
    assert_eq!(
        got.header("content-range"),
        Some(format!("bytes {start}-{}/{total}", total - 1).as_str())
    );

    // Asking for more than there is, and for the last bytes by length.
    let got = lib.bytes(id, start, total + 10_000);
    assert_eq!(got.body, file[start as usize..]);
    let got = lib.get(id, Some("bytes=-100"));
    assert_eq!(got.body, file[file.len() - 100..]);
    assert_eq!(got.status, 206);
}

#[test]
fn a_range_past_the_end_is_not_satisfiable() {
    let lib = Lib::new();
    let file = long_wav();
    let id = lib.put("a.wav", &file);
    let got = lib.bytes(id, file.len() as u64, file.len() as u64 + 5);
    assert_eq!(got.status, 416);
    assert_eq!(
        got.header("content-range"),
        Some(format!("bytes */{}", file.len()).as_str())
    );
    assert!(got.body.is_empty());
}

#[test]
fn an_open_ended_or_missing_range_gets_one_chunk_never_the_whole_file() {
    let lib = Lib::new();
    let file = long_wav();
    let id = lib.put("a.wav", &file);

    for range in [None, Some("bytes=0-"), Some("bytes=0-999999999")] {
        let got = lib.get(id, range);
        assert_eq!(got.status, 206, "{range:?}");
        assert_eq!(got.body.len() as u64, MAX_CHUNK, "{range:?}");
        assert_eq!(got.body, file[..MAX_CHUNK as usize]);
        assert_eq!(
            got.header("content-range"),
            Some(format!("bytes 0-{}/{}", MAX_CHUNK - 1, file.len()).as_str())
        );
    }
}

#[test]
fn a_small_file_asked_for_without_a_range_is_a_plain_200() {
    let lib = Lib::new();
    let file = flac(&pcm(6, 1.0, 44_100, 2));
    let id = lib.put("a.flac", &file);
    let got = lib.get(id, None);
    assert_eq!(got.status, 200);
    assert_eq!(got.body, file);
    assert_eq!(got.header("content-type"), Some("audio/flac"));
    assert_eq!(got.header("content-range"), None);
}

#[test]
fn a_file_is_played_as_it_is_on_disk_now() {
    // Nothing derived from a preview is stored, so the size and date the
    // scan saw aren't checked: a file edited since (rekordbox rewriting its
    // tags, say) plays, and its length is the new one.
    let lib = Lib::new();
    let id = lib.put("a.wav", &wav(&pcm(7, 1.0, 44_100, 1)));
    let longer = wav(&pcm(7, 2.0, 44_100, 1));
    fs::write(lib.path_of("a.wav"), &longer).unwrap();
    let got = lib.get(id, Some("bytes=0-99"));
    assert_eq!(got.status, 206);
    assert_eq!(
        got.header("content-range"),
        Some(format!("bytes 0-99/{}", longer.len()).as_str())
    );

    // A decoded file's header is built from the file as it is at request
    // time too.
    let short = Made::new(44_100, 2, 16, 1_000);
    let id = lib.put("b.aiff", &short.bytes());
    let long = Made::new(44_100, 2, 16, 3_000);
    let expected = long.as_wav();
    fs::write(lib.path_of("b.aiff"), long.bytes()).unwrap();
    let got = lib.bytes(id, 0, 99);
    assert_eq!(got.status, 206);
    assert_eq!(
        got.header("content-range"),
        Some(format!("bytes 0-99/{}", expected.len()).as_str())
    );
    assert_eq!(got.body, expected[..100]);
}

#[test]
fn an_aiff_is_answered_as_a_wav_and_a_seek_into_it_returns_the_right_samples() {
    let lib = Lib::new();
    let aiff = Made::new(44_100, 2, 16, 300_000);
    let expected = aiff.as_wav();
    let id = lib.put("a.aiff", &aiff.bytes());

    let got = lib.bytes(id, 500_001, 600_000);
    assert_eq!(got.status, 206);
    assert_eq!(got.header("content-type"), Some("audio/wav"));
    assert_eq!(got.body, expected[500_001..=600_000]);
    assert_eq!(
        got.header("content-range"),
        Some(format!("bytes 500001-600000/{}", expected.len()).as_str())
    );

    // Without a range: the first chunk, header included.
    let got = lib.get(id, None);
    assert_eq!(got.status, 206);
    assert_eq!(got.body, expected[..MAX_CHUNK as usize]);
}

#[test]
fn an_alac_in_an_m4a_is_answered_as_a_wav() {
    let lib = Lib::new();
    let song = pcm(8, 3.0, 44_100, 1);
    let expected = wav(&song);
    let id = lib.put("a.m4a", &m4a_alac(&song));
    let got = lib.bytes(id, 70_001, 90_000);
    assert_eq!(got.status, 206);
    assert_eq!(got.header("content-type"), Some("audio/wav"));
    assert_eq!(got.body, expected[70_001..=90_000]);
}

#[test]
fn a_format_is_known_from_its_bytes_not_its_name() {
    let lib = Lib::new();
    let song = pcm(9, 1.0, 44_100, 1);
    // A WAV called .mp3 is a WAV, and an AIFF called .wav is decoded.
    let id = lib.put("really-a-wav.mp3", &wav(&song));
    assert_eq!(lib.get(id, None).header("content-type"), Some("audio/wav"));
    let id = lib.put(
        "really-an-aiff.wav",
        &Made::new(44_100, 1, 16, 5_000).bytes(),
    );
    let got = lib.get(id, Some("bytes=0-43"));
    assert_eq!(got.status, 206);
    assert_eq!(got.header("content-type"), Some("audio/wav"));
}
