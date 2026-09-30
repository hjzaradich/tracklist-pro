//! 1aB-6: what the audio_hash covers. The same audio under any tags hashes
//! the same; any change to the audio itself doesn't.

use std::collections::BTreeSet;

use super::fixtures::*;
use crate::hash::{AudioFormat, Skip, AUDIO_HASH_LEN, DEFINITION};

/// Asserts every variant has the same audio_hash, and that their blake3s
/// are all different (the files really differ).
fn same_audio_different_files(what: &str, variants: &[(&str, Vec<u8>)]) {
    let (first_name, first) = &variants[0];
    let expected = audio(first);
    let mut blake3s = BTreeSet::new();
    for (name, bytes) in variants {
        assert_eq!(audio(bytes), expected, "{what}: {name} vs {first_name}");
        assert!(
            blake3s.insert(hash(bytes).blake3),
            "{what}: {name} has the same bytes as another variant"
        );
    }
}

fn flip(bytes: &[u8], at: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[at] ^= 0x01;
    out
}

#[test]
fn every_supported_format_gets_a_34_byte_audio_hash_prefixed_with_definition_and_format() {
    let expected = [
        ("mp3", AudioFormat::Mp3),
        ("adts", AudioFormat::Adts),
        ("wav", AudioFormat::Wave),
        ("rf64", AudioFormat::Wave),
        ("rifx", AudioFormat::Wave),
        ("aiff", AudioFormat::Aiff),
        ("aifc", AudioFormat::Aiff),
        ("flac", AudioFormat::Flac),
        ("mp4", AudioFormat::Mp4),
        ("ogg_vorbis", AudioFormat::OggVorbis),
        ("ogg_opus", AudioFormat::OggOpus),
    ];
    let formats = every_format();
    assert_eq!(formats.len(), expected.len());
    for ((name, bytes), (expected_name, format)) in formats.iter().zip(expected) {
        assert_eq!(*name, expected_name);
        let hashed = hash(bytes);
        let audio = hashed.audio.unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(audio.format, format, "{name}");
        let stored = audio.to_bytes();
        assert_eq!(stored.len(), AUDIO_HASH_LEN);
        assert_eq!(stored[0], DEFINITION, "{name}");
        assert_eq!(stored[1], format.code(), "{name}");
        assert_eq!(&stored[2..], &audio.digest);
    }
}

#[test]
fn blake3_is_the_plain_blake3_of_every_byte_of_the_file() {
    let mut files: Vec<Vec<u8>> = every_format().into_iter().map(|f| f.1).collect();
    files.push(Vec::new());
    files.push(b"not audio at all".to_vec());
    files.push(vec![0xAB; 3 * 4096 + 17]);
    for bytes in files {
        let hashed = hash(&bytes);
        assert_eq!(hashed.blake3, *blake3::hash(&bytes).as_bytes());
        assert_eq!(hashed.len, bytes.len() as u64);
    }
}

#[test]
fn mp3_audio_hash_ignores_id3v2_id3v1_apev2_lyrics3_and_junk_at_either_end() {
    let frames = mp3_frames();
    let mut variants = Vec::new();
    for (lead, before) in leading_tags() {
        for (trail, after) in trailing_tags() {
            let name: &'static str = Box::leak(format!("{lead} + {trail}").into_boxed_str());
            variants.push((name, [before.clone(), frames.clone(), after].concat()));
        }
    }
    same_audio_different_files("mp3", &variants);
}

#[test]
fn adts_audio_hash_ignores_tags_at_either_end() {
    let frames = adts_frames();
    let mut variants = Vec::new();
    for (lead, before) in leading_tags() {
        for (trail, after) in trailing_tags() {
            let name: &'static str = Box::leak(format!("{lead} + {trail}").into_boxed_str());
            variants.push((name, [before.clone(), frames.clone(), after].concat()));
        }
    }
    same_audio_different_files("adts", &variants);
}

#[test]
fn wave_audio_hash_ignores_list_id3_bext_ixml_chunks_and_chunk_order() {
    same_audio_different_files("wave", &wav_variants());
}

#[test]
fn a_wave_and_its_rf64_twin_share_an_audio_hash() {
    assert_eq!(audio(&wav_plain()), audio(&rf64()));
}

#[test]
fn aiff_audio_hash_ignores_id3_and_text_chunks_chunk_order_and_alignment() {
    same_audio_different_files("aiff", &aiff_variants());
}

#[test]
fn flac_audio_hash_ignores_every_metadata_block_and_tags_at_either_end() {
    same_audio_different_files("flac", &flac_variants());
}

#[test]
fn mp4_audio_hash_ignores_metadata_atoms_padding_and_where_moov_sits() {
    same_audio_different_files("mp4", &mp4_variants());
}

#[test]
fn ogg_vorbis_audio_hash_ignores_the_comment_packet_and_how_pages_are_cut() {
    same_audio_different_files("ogg_vorbis", &ogg_vorbis_variants());
}

#[test]
fn ogg_opus_audio_hash_ignores_the_opus_tags_packet() {
    same_audio_different_files("ogg_opus", &ogg_opus_variants());
}

/// A byte inside the audio of each fixture from [`every_format`].
fn audio_byte(name: &str, len: usize) -> Vec<usize> {
    match name {
        // Inside frame 5's main data, and the last byte of the last frame.
        "mp3" => vec![417 * 5 + 100, len - 1],
        "adts" => vec![64 * 5 + 20, len - 1],
        // The samples are odd-length, so the file ends with a pad byte.
        "wav" | "rf64" | "rifx" | "aiff" | "aifc" => vec![len - 2, len - 2000],
        // Inside the last frame, and inside the first one.
        "flac" => vec![len - 10, 42 + 20],
        "mp4" => vec![len - 1, len - 20],
        // The last audio packet's last byte.
        "ogg_vorbis" | "ogg_opus" => vec![len - 1, len - 300],
        other => panic!("no audio byte known for {other}"),
    }
}

#[test]
fn one_changed_audio_byte_changes_the_audio_hash_in_every_format() {
    for (name, bytes) in every_format() {
        let original = audio(&bytes);
        for at in audio_byte(name, bytes.len()) {
            let changed = flip(&bytes, at);
            let hashed = hash(&changed);
            assert_ne!(
                hashed.audio.map(|a| a.to_bytes()),
                Ok(original),
                "{name}: byte {at} changed but the audio_hash didn't"
            );
            assert_ne!(hashed.blake3, hash(&bytes).blake3);
        }
    }
}

#[test]
fn bytes_around_the_audio_never_change_the_audio_hash() {
    let cases: Vec<(&str, Vec<u8>, Vec<usize>)> = vec![
        // The pad byte after odd-length samples, and the RIFF size field.
        (
            "wav pad byte and RIFF size",
            wav_plain(),
            vec![wav_plain().len() - 1, 4],
        ),
        (
            "aiff pad byte and FORM size",
            aiff_plain(),
            vec![aiff_plain().len() - 1, 4],
        ),
        // STREAMINFO's MD5 and frame-size fields.
        ("flac STREAMINFO", flac_with(&[]), vec![8 + 4, 8 + 30]),
        // mvhd's creation time, in moov, and the ftyp brand.
        (
            "mp4 moov and ftyp",
            crate::tags::test_audio::m4a(),
            vec![8, 20 + 8 + 12],
        ),
    ];
    for (name, bytes, offsets) in cases {
        let original = audio(&bytes);
        for at in offsets {
            assert_eq!(audio(&flip(&bytes, at)), original, "{name}: byte {at}");
        }
    }
    // Ogg page checksums and sequence numbers of the audio pages.
    let ogg = ogg_vorbis_with(vorbis_comment_packet(&[]));
    let last_page = ogg.windows(4).rposition(|w| w == b"OggS").unwrap();
    for field in [18, 22] {
        assert_eq!(
            audio(&flip(&ogg, last_page + field)),
            audio(&ogg),
            "ogg page byte {field}"
        );
    }
}

#[test]
fn the_wave_format_chunk_is_part_of_the_audio() {
    let mut fmt = wav_fmt();
    fmt[4..8].copy_from_slice(&16_000u32.to_le_bytes()); // another sample rate
    let other = riff(
        b"RIFF",
        &[
            chunk(b"fmt ", &fmt, false),
            chunk(b"data", &samples(), false),
        ],
    );
    assert_ne!(audio(&other), audio(&wav_plain()));
}

#[test]
fn the_aiff_sample_format_is_part_of_the_audio_and_aifc_none_matches_aiff() {
    let mut comm = aiff_comm();
    comm[2..6].copy_from_slice(&1999u32.to_be_bytes()); // frame count
    let other = form(
        b"AIFF",
        &[chunk(b"COMM", &comm, true), chunk(b"SSND", &ssnd(0), true)],
    );
    assert_ne!(audio(&other), audio(&aiff_plain()));
    assert_eq!(audio(&aifc_none()), audio(&aiff_plain()));
}

#[test]
fn the_ogg_stream_length_is_part_of_the_audio() {
    let longer = ogg_stream(
        SERIAL,
        &[
            (vec![vorbis_ident()], 0),
            (vec![vorbis_comment_packet(&[]), vorbis_setup()], 0),
            (audio_packets(), FINAL_GRANULE + 1),
        ],
    );
    assert_ne!(
        audio(&longer),
        audio(&ogg_vorbis_with(vorbis_comment_packet(&[])))
    );
}

#[test]
fn ogg_vorbis_setup_and_identification_headers_are_part_of_the_audio() {
    let base = ogg_vorbis_with(vorbis_comment_packet(&[]));
    let mut setup = vorbis_setup();
    setup[100] ^= 1;
    let other = ogg_stream(
        SERIAL,
        &[
            (vec![vorbis_ident()], 0),
            (vec![vorbis_comment_packet(&[]), setup], 0),
            (audio_packets(), FINAL_GRANULE),
        ],
    );
    assert_ne!(audio(&other), audio(&base));
}

#[test]
fn the_same_audio_bytes_in_two_formats_never_share_an_audio_hash() {
    // MPEG and ADTS frames are different bytes anyway; what keeps two
    // families apart is the format code and the hashing context.
    let mut digests = BTreeSet::new();
    let mut codes = BTreeSet::new();
    for format in AudioFormat::ALL {
        let bytes = crate::hash::AudioHash {
            format,
            digest: [0; 32],
        }
        .to_bytes();
        assert!(codes.insert(bytes[1]), "{format:?} reuses a code");
        let mut hasher = blake3::Hasher::new_derive_key(&format!(
            "tracklist-pro audio_hash v1 {}",
            format.name()
        ));
        hasher.update(b"same bytes");
        assert!(digests.insert(*hasher.finalize().as_bytes()));
    }
}

#[test]
fn format_codes_and_names_are_fixed_for_definition_1() {
    let table: Vec<_> = AudioFormat::ALL
        .iter()
        .map(|f| (f.code(), f.name()))
        .collect();
    assert_eq!(
        table,
        [
            (1, "mp3"),
            (2, "adts"),
            (3, "wave"),
            (4, "aiff"),
            (5, "flac"),
            (6, "mp4"),
            (7, "ogg_vorbis"),
            (8, "ogg_opus"),
        ]
    );
    assert_eq!(DEFINITION, 1);
    let reasons: Vec<_> = Skip::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        reasons,
        [
            "empty",
            "truncated",
            "unknown_format",
            "unsupported_format",
            "malformed"
        ]
    );
}

/// Known answers: if one of these changes, the definition changed. Bump
/// `DEFINITION` (so every file is hashed again), then update them.
#[test]
fn known_answers_pin_definition_1_for_every_format() {
    let known = [
        (
            "mp3",
            "010180b7c9265cf40c28c64ac8cfca907f527f76a3e6713930251a712d662cf8f62d",
        ),
        (
            "adts",
            "0102220dcd20b2cb9e0d4814f75d647087e9b85e1e8d982618c6924591365b8a2757",
        ),
        (
            "wav",
            "01032704c8b83f24f5ec461fab6360ca45113803b9595e61fded5a8bf360243bf8f9",
        ),
        (
            "rf64",
            "01032704c8b83f24f5ec461fab6360ca45113803b9595e61fded5a8bf360243bf8f9",
        ),
        (
            "rifx",
            "01036d001def655ffc24c1565ef78db6d7ffb8b76eb8410192d1b235e5bd18f8fb9d",
        ),
        (
            "aiff",
            "01047a944f9b11efb861528b8aa94f3b1412baafb089d1f4e8c2a529657555556c74",
        ),
        (
            "aifc",
            "01047a944f9b11efb861528b8aa94f3b1412baafb089d1f4e8c2a529657555556c74",
        ),
        (
            "flac",
            "01054cf3e265155f63a4524941dfff9d967a1b3777054bf14acd4a43aab9a8ddc733",
        ),
        (
            "mp4",
            "01068ea79656dc91ae77595add44f44bcf89bb7cb1b940f4dd96d394ffb0c95dfdd7",
        ),
        (
            "ogg_vorbis",
            "01071267b6dc85c3ab91e81c47a96c794efd42ebcc5372e6fc3edebc84e52f93e3d3",
        ),
        (
            "ogg_opus",
            "01086f909e3728016f2e47e1f954e0efdd872bfaf5a3bc58dc0112dd41ba8da0702f",
        ),
    ];
    let mut wrong = Vec::new();
    for ((name, bytes), (known_name, expected)) in every_format().iter().zip(known) {
        assert_eq!(*name, known_name);
        let got = hex(&audio(bytes));
        if got != expected {
            wrong.push(format!("(\"{name}\", \"{got}\"),"));
        }
    }
    assert!(
        wrong.is_empty(),
        "audio_hash changed:\n{}",
        wrong.join("\n")
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_audio_hash_does_not_depend_on_how_the_file_is_read() {
    let mut files: Vec<Vec<u8>> = every_format().into_iter().map(|f| f.1).collect();
    files.extend(ogg_vorbis_variants().into_iter().map(|f| f.1));
    files.extend(flac_variants().into_iter().map(|f| f.1));
    for bytes in files {
        let expected = hash_with(&bytes, 1 << 20);
        for buffer in [1, 2, 3, 7, 26, 27, 28, 255, 256, 4095] {
            assert_eq!(hash_with(&bytes, buffer), expected, "buffer {buffer}");
        }
    }
}

#[test]
fn unsupported_unknown_and_broken_files_get_no_audio_hash_and_say_why() {
    let wavpack = [b"wvpk".to_vec(), vec![0; 60]].concat();
    let monkeys = [b"MAC ".to_vec(), vec![0; 60]].concat();
    let musepack = [b"MPCK".to_vec(), vec![0; 60]].concat();
    let mut asf = vec![
        0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE,
        0x6C,
    ];
    asf.extend(vec![0; 60]);
    let ogg_flac = ogg_stream(
        SERIAL,
        &[(vec![b"\x7FFLAC\x01\x00\x00\x01fLaC".to_vec()], 0)],
    );
    let chained = [
        ogg_vorbis_with(vorbis_comment_packet(&[])),
        ogg_stream(
            SERIAL + 1,
            &[
                (vec![vorbis_ident()], 0),
                (vec![vorbis_comment_packet(&[]), vorbis_setup()], 0),
                (audio_packets(), 1),
            ],
        ),
    ]
    .concat();
    let no_data = riff(
        b"RIFF",
        &[chunk(b"fmt ", &wav_fmt(), false), list_info("x")],
    );
    let no_fmt = riff(b"RIFF", &[chunk(b"data", &samples(), false)]);
    let no_ssnd = form(b"AIFF", &[chunk(b"COMM", &aiff_comm(), true)]);
    let flac_no_frames = {
        let mut f = flac_with(&[]);
        f.truncate(42);
        f
    };
    let flac_junk_frames = {
        let mut f = flac_with(&[]);
        f[42] = 0x00;
        f
    };
    let mp4_no_mdat = m4a_as(&[b"ftyp", b"moov"], &[]);
    let id3_only = crate::tags::test_audio::id3v2(&[], 10);
    let cases: Vec<(&str, Vec<u8>, Skip)> = vec![
        ("empty", Vec::new(), Skip::Empty),
        (
            "text",
            b"just some text, not audio".repeat(10),
            Skip::UnknownFormat,
        ),
        ("WavPack", wavpack, Skip::UnsupportedFormat),
        ("Monkey's Audio", monkeys, Skip::UnsupportedFormat),
        ("Musepack", musepack, Skip::UnsupportedFormat),
        ("WMA", asf, Skip::UnsupportedFormat),
        ("FLAC in Ogg", ogg_flac, Skip::UnsupportedFormat),
        ("chained Ogg", chained, Skip::UnsupportedFormat),
        (
            "WAVE whose chunks end before any data",
            no_data,
            Skip::Truncated,
        ),
        ("WAVE without fmt", no_fmt, Skip::Malformed),
        (
            "AIFF whose chunks end before any samples",
            no_ssnd,
            Skip::Truncated,
        ),
        ("FLAC without frames", flac_no_frames, Skip::Malformed),
        (
            "FLAC with junk for frames",
            flac_junk_frames,
            Skip::Malformed,
        ),
        ("MP4 without mdat", mp4_no_mdat, Skip::Malformed),
        ("an ID3v2 tag and nothing else", id3_only, Skip::Truncated),
    ];
    for (name, bytes, expected) in cases {
        let hashed = hash(&bytes);
        assert_eq!(hashed.audio, Err(expected), "{name}");
        // Never a whole-file fallback: blake3 is there, audio_hash isn't.
        assert_eq!(hashed.blake3, *blake3::hash(&bytes).as_bytes(), "{name}");
    }
}
