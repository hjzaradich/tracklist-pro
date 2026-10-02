//! Golden fingerprints: the exact bytes `decode::fingerprint` produces for
//! generated files, in every format, truncated and damaged. Stored
//! fingerprints of real libraries stay valid only while these never change,
//! so a change to the decoding code that moves one of them is a bug, not a
//! detail. If a value must change, `stored::VERSION` has to change with it.

use sha2::{Digest, Sha256};

use super::audio;
use super::decode_bytes;
use crate::fingerprint::decode::Stopped;
use crate::tags::test_audio;

/// How decoding `bytes` ended: the SHA-256 of the stored blob (first 16 hex
/// digits) and the item count, or the reason there's no fingerprint.
fn outcome(bytes: &[u8]) -> String {
    match decode_bytes(bytes) {
        Ok(fp) => {
            let digest = Sha256::digest(fp.to_blob());
            let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
            format!("{hex} {} items", fp.items().len())
        }
        Err(Stopped::Failed(why)) => format!("failed: {why}"),
        Err(Stopped::Cancelled) => "cancelled".to_owned(),
    }
}

/// Scribbles over `len` bytes from the middle of `bytes`.
fn scribbled(bytes: &[u8], len: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let middle = out.len() / 2;
    for (i, b) in out[middle..middle + len].iter_mut().enumerate() {
        *b = (i * 37 % 251) as u8;
    }
    out
}

fn files() -> Vec<(&'static str, Vec<u8>)> {
    let mono = audio::pcm(5, 20.0, 44_100, 1);
    let stereo = audio::pcm(5, 20.0, 44_100, 2);
    let long_flac = audio::flac(&audio::pcm(13, 60.0, 22_050, 1));
    let cut_flac = audio::flac(&audio::pcm(8, 30.0, 22_050, 1));
    let cut_wav = audio::wav(&stereo);
    #[allow(unused_mut)]
    let mut files = vec![
        ("wav", audio::wav(&stereo)),
        ("wav mono", audio::wav(&mono)),
        ("flac", audio::flac(&stereo)),
        ("aiff", audio::aiff(&stereo)),
        ("m4a alac", audio::m4a_alac(&mono)),
        ("ogg vorbis", test_audio::ogg_vorbis()),
        ("m4a aac", test_audio::m4a()),
        ("flac cut short", cut_flac[..cut_flac.len() / 2].to_vec()),
        ("wav cut short", cut_wav[..cut_wav.len() / 3].to_vec()),
        ("flac with damaged frames", scribbled(&long_flac, 3000)),
        ("opus", test_audio::ogg_opus()),
        ("not audio", b"not a song\n".repeat(300)),
        ("too short", audio::wav(&audio::pcm(7, 2.0, 22_050, 1))),
    ];
    #[cfg(windows)]
    {
        let mp3 = audio::mp3(&stereo, 192);
        files.push(("mp3", mp3.clone()));
        files.push(("mp3 cut short", mp3[..mp3.len() / 2].to_vec()));
        files.push(("mp3 with damaged frames", scribbled(&mp3, 1500)));
    }
    files
}

#[test]
fn fingerprints_of_generated_files_never_change_one_bit() {
    #[allow(unused_mut)]
    let mut expected: Vec<&str> = vec![
        "wav: 1890eaad23e4d37d 140 items",
        "wav mono: 1890eaad23e4d37d 140 items",
        "flac: 1890eaad23e4d37d 140 items",
        "aiff: 1890eaad23e4d37d 140 items",
        "m4a alac: 1890eaad23e4d37d 140 items",
        "ogg vorbis: failed: unsupported_codec",
        "m4a aac: failed: no_audio",
        "flac cut short: 6440a9b1483ae015 99 items",
        "wav cut short: 4b1e288bb36dab29 32 items",
        "flac with damaged frames: 5a2583f8e7cf3721 462 items",
        "opus: failed: unsupported_codec",
        "not audio: failed: unsupported_format",
        "too short: failed: too_short",
    ];
    // LAME only builds on Windows, so the MP3s are only checked there.
    #[cfg(windows)]
    expected.extend([
        "mp3: c6731fed70f8a627 140 items",
        "mp3 cut short: 2d5b0969606903c5 59 items",
        "mp3 with damaged frames: 28d3ee2ee2b2ed70 139 items",
    ]);
    let got: Vec<String> = files()
        .into_iter()
        .map(|(name, bytes)| format!("{name}: {}", outcome(&bytes)))
        .collect();
    assert_eq!(got, expected, "{got:#?}");
}
