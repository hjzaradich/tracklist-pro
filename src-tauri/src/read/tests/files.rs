//! One file read: format, codec, audio properties, tags and verdict, on
//! files built byte by byte in a temp dir.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::read::file::read;
use crate::read::{Codec, FileRead, NotRead, Verdict};
use crate::sniff::SniffedFormat;
use crate::tags::test_audio::{self as audio, id3_frame, id3_text, id3v2, Format};

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Reads a file that stays local throughout.
fn read_local(path: &Path) -> Result<FileRead, NotRead> {
    read(path, |_| Ok(true))
}

fn read_bytes(name: &str, bytes: &[u8]) -> FileRead {
    let dir = tempfile::tempdir().unwrap();
    read_local(&write(dir.path(), name, bytes)).unwrap()
}

/// Whether raw tags hold no items: no blocks, or only empty ones (an Ogg
/// file always has a comment header, even an empty one).
pub(super) fn untagged(raw_tags: Option<&str>) -> bool {
    let Some(Value::Object(blocks)) = raw_tags.and_then(|t| serde_json::from_str(t).ok()) else {
        return false;
    };
    blocks
        .values()
        .all(|items| items.as_array().is_some_and(Vec::is_empty))
}

fn raw_tags(read: &FileRead) -> Value {
    serde_json::from_str(read.raw_tags.as_deref().expect("no raw tags")).unwrap()
}

/// The texts of the items under `block`, in order.
fn texts(tags: &Value, block: &str) -> Vec<(String, String)> {
    tags[block]
        .as_array()
        .unwrap_or_else(|| panic!("no {block} in {tags}"))
        .iter()
        .filter_map(|item| {
            Some((
                item["key"].as_str()?.to_owned(),
                item["value"]["text"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

/// An MP4 with its top-level box of type `kind` cut out.
fn without_box(mp4: &[u8], kind: &[u8; 4]) -> Vec<u8> {
    let mut at = 0;
    while at + 8 <= mp4.len() {
        let size = u32::from_be_bytes(mp4[at..at + 4].try_into().unwrap()) as usize;
        if &mp4[at + 4..at + 8] == kind {
            return [&mp4[..at], &mp4[at + size..]].concat();
        }
        at += size;
    }
    panic!("no {kind:?} box");
}

/// A PCM WAV whose `data` chunk is empty: a header and no audio.
fn wav_without_audio() -> Vec<u8> {
    let whole = audio::wav();
    // RIFF header (12) + fmt chunk (8 + 16), then the data chunk header.
    let mut bytes = whole[..44].to_vec();
    bytes[40..44].copy_from_slice(&0u32.to_le_bytes());
    let riff_len = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&riff_len.to_le_bytes());
    bytes
}

#[test]
fn every_test_format_gets_its_sniffed_format_codec_and_audio_properties() {
    for (format, sniffed, codec) in [
        (Format::Mp3, SniffedFormat::Mp3, Codec::Mp3),
        (Format::Flac, SniffedFormat::Flac, Codec::Flac),
        (Format::Wav, SniffedFormat::Wav, Codec::Pcm),
        (Format::Aiff, SniffedFormat::Aiff, Codec::Pcm),
        (Format::M4a, SniffedFormat::Mp4, Codec::Aac),
        (Format::Ogg, SniffedFormat::OggVorbis, Codec::Vorbis),
        (Format::Opus, SniffedFormat::OggOpus, Codec::Opus),
    ] {
        let got = read_bytes(&format!("a.{}", format.extension()), &format.bytes());
        assert_eq!(got.sniffed_format, sniffed, "{format:?}");
        assert_eq!(got.codec, Some(codec), "{format:?}");
        assert_eq!(
            got.sample_rate,
            Some(i64::from(format.sample_rate())),
            "{format:?}"
        );
        assert!(untagged(got.raw_tags.as_deref()), "{format:?}: {got:?}");
        assert_eq!(got.verdict, None, "{format:?}: {got:?}");
    }
}

#[test]
fn the_audio_properties_come_from_the_audio_itself() {
    // 20 frames of MPEG-1 layer III at 128 kbps, 44.1 kHz: 1152 samples each.
    let got = read_bytes("a.mp3", &audio::mp3());
    assert_eq!(got.bitrate_kbps, Some(128));
    assert_eq!(got.sample_rate, Some(44_100));
    let expected_ms = 20 * 1152 * 1000 / 44_100;
    let duration = got.duration_ms.unwrap();
    assert!((duration - expected_ms).abs() <= 30, "{duration} ms");
    // 2048 samples at 8 kHz.
    let wav = read_bytes("a.wav", &audio::wav());
    assert_eq!(wav.duration_ms, Some(256));
}

#[test]
fn a_file_with_a_broken_id3_block_still_gets_its_audio_properties_and_its_good_frames() {
    // TPE1 claims text encoding 9, which doesn't exist (§5.5).
    let tag = id3v2(
        &[
            id3_text(b"TIT2", "Survivor"),
            id3_frame(b"TPE1", &[9, 0xFF, 0xFE, 0x00]),
            id3_text(b"TALB", "After"),
        ],
        0,
    );
    let got = read_bytes("a.mp3", &[tag, audio::mp3()].concat());
    assert_eq!(got.sniffed_format, SniffedFormat::Mp3);
    assert_eq!(got.codec, Some(Codec::Mp3));
    assert_eq!(got.sample_rate, Some(44_100));
    assert_eq!(got.bitrate_kbps, Some(128));
    assert!(got.duration_ms.unwrap() > 0);
    assert_eq!(got.verdict, None);
    assert_eq!(
        texts(&raw_tags(&got), "id3v2"),
        [
            ("TIT2".to_owned(), "Survivor".to_owned()),
            ("TALB".to_owned(), "After".to_owned())
        ]
    );
}

#[test]
fn a_file_with_a_broken_ape_tag_still_gets_its_audio_properties() {
    // An APEv2 footer whose sizes point outside the file.
    let mut ape = b"APETAGEX".to_vec();
    ape.extend_from_slice(&2000u32.to_le_bytes());
    ape.extend_from_slice(&0x7FFF_FFF0u32.to_le_bytes());
    ape.extend_from_slice(&0xFFFFu32.to_le_bytes());
    ape.extend_from_slice(&0u32.to_le_bytes());
    ape.extend_from_slice(&[0; 8]);
    let tag = id3v2(&[id3_text(b"TIT2", "Still Here")], 0);
    let got = read_bytes("a.mp3", &[tag, audio::mp3(), ape].concat());
    assert_eq!(got.sample_rate, Some(44_100));
    assert_eq!(got.verdict, None);
    assert_eq!(
        texts(&raw_tags(&got), "id3v2"),
        [("TIT2".to_owned(), "Still Here".to_owned())]
    );
}

#[test]
fn trap_mp3_in_wav_a_wav_name_holding_an_mp3_gets_format_and_codec_mp3() {
    let tag = id3v2(&[id3_text(b"TIT2", "Not A Wave")], 16);
    let got = read_bytes("Track.wav", &[tag, audio::mp3()].concat());
    assert_eq!(got.sniffed_format, SniffedFormat::Mp3);
    assert_eq!(got.codec, Some(Codec::Mp3));
    assert_eq!(got.sample_rate, Some(44_100));
    assert_eq!(got.verdict, None);
    assert_eq!(
        texts(&raw_tags(&got), "id3v2"),
        [("TIT2".to_owned(), "Not A Wave".to_owned())]
    );
}

#[test]
fn trap_empty_file_a_zero_byte_file_is_broken_with_nothing_else_known() {
    let got = read_bytes("a.mp3", &[]);
    assert_eq!(got.verdict, Some(Verdict::Broken));
    assert_eq!(got.sniffed_format, SniffedFormat::Unknown);
    assert_eq!(
        (
            got.codec,
            got.bitrate_kbps,
            got.sample_rate,
            got.duration_ms
        ),
        (None, None, None, None)
    );
    assert_eq!(got.raw_tags, None);
}

#[test]
fn a_file_cut_off_mid_download_is_truncated() {
    for (name, whole) in [("a.wav", audio::wav()), ("a.aiff", audio::aiff())] {
        let cut = &whole[..whole.len() - 1000];
        let got = read_bytes(name, cut);
        assert_eq!(got.verdict, Some(Verdict::Truncated), "{name}: {got:?}");
    }
    // An ID3 tag that claims more bytes than the file has.
    let tag = id3v2(&[id3_text(b"TIT2", "Cut")], 4000);
    let got = read_bytes("a.mp3", &tag[..200]);
    assert_eq!(got.verdict, Some(Verdict::Truncated), "{got:?}");
}

#[test]
fn trap_m4a_no_moov_an_interrupted_m4a_is_broken_with_no_codec() {
    let got = read_bytes("a.m4a", &without_box(&audio::m4a(), b"moov"));
    assert_eq!(got.sniffed_format, SniffedFormat::Mp4);
    assert_eq!(got.verdict, Some(Verdict::Broken));
    assert_eq!(got.codec, None);
}

#[test]
fn bytes_no_reader_can_make_sense_of_are_broken() {
    // Text, not audio, under an audio name.
    let junk = b"This is not audio. ".repeat(200);
    for name in ["a.mp3", "a.flac", "a.m4a"] {
        let got = read_bytes(name, &junk);
        assert_eq!(got.verdict, Some(Verdict::Broken), "{name}: {got:?}");
        assert_eq!(got.sniffed_format, SniffedFormat::Unknown, "{name}");
        assert_eq!(got.raw_tags, None, "{name}");
    }
}

#[test]
fn a_wav_with_a_header_and_no_audio_is_broken() {
    // Zero samples: the tag reader refuses it, or reads a duration of 0
    // (the verdict rule's unit test covers both).
    let got = read_bytes("a.wav", &wav_without_audio());
    assert_eq!(got.sniffed_format, SniffedFormat::Wav);
    assert_eq!(got.codec, Some(Codec::Pcm));
    assert!(matches!(got.duration_ms, None | Some(0)), "{got:?}");
    assert_eq!(got.verdict, Some(Verdict::Broken));
}

#[test]
fn raw_tags_are_a_json_object_with_one_array_per_tag_block_type() {
    let head = id3v2(&[id3_text(b"TIT2", "Title")], 0);
    let tail = [
        audio::ape_tag(&[("Artist", "From APE")]),
        audio::id3v1_tag("Old Title"),
    ]
    .concat();
    let got = read_bytes("a.mp3", &[head, audio::mp3(), tail].concat());
    let tags = raw_tags(&got);
    let blocks: Vec<_> = tags.as_object().unwrap().keys().cloned().collect();
    assert_eq!(blocks, ["ape", "id3v1", "id3v2"]);
    assert_eq!(
        texts(&tags, "ape"),
        [("Artist".to_owned(), "From APE".to_owned())]
    );
    assert!(texts(&tags, "id3v1").contains(&("title".to_owned(), "Old Title".to_owned())));
}

#[test]
fn a_file_that_is_gone_or_cannot_be_opened_is_an_error_not_a_broken_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let gone = read_local(&dir.path().join("gone.mp3")).unwrap_err();
    assert!(
        matches!(&gone, NotRead::Unreachable(e) if e.kind() == std::io::ErrorKind::NotFound),
        "{gone:?}"
    );
    // A folder where a file should be can't be read as one.
    let folder = dir.path().join("folder.mp3");
    std::fs::create_dir(&folder).unwrap();
    assert!(matches!(read_local(&folder), Err(NotRead::Unreachable(_))));
}

#[test]
fn reading_a_file_changes_nothing_about_it() {
    let dir = tempfile::tempdir().unwrap();
    let tag = id3v2(&[id3_text(b"TIT2", "Untouched")], 64);
    let path = write(dir.path(), "a.mp3", &[tag, audio::mp3()].concat());
    let before = (
        std::fs::read(&path).unwrap(),
        std::fs::metadata(&path).unwrap().modified().unwrap(),
    );
    read_local(&path).unwrap();
    let after = (
        std::fs::read(&path).unwrap(),
        std::fs::metadata(&path).unwrap().modified().unwrap(),
    );
    assert_eq!(before, after);
}

#[test]
fn a_file_that_is_online_only_is_never_opened() {
    // A path that doesn't exist: opening it would be an error, so an
    // online-only answer proves no open was tried.
    let dir = tempfile::tempdir().unwrap();
    let asked = std::cell::Cell::new(0);
    let got = read(&dir.path().join("placeholder.mp3"), |_| {
        asked.set(asked.get() + 1);
        Ok(false)
    });
    assert!(matches!(got, Err(NotRead::OnlineOnly)), "{got:?}");
    assert_eq!(asked.get(), 1);
}

#[test]
fn a_file_that_turns_online_only_after_the_sniff_is_not_opened_again_for_its_tags() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.mp3", &audio::mp3());
    let asked = std::cell::Cell::new(0);
    let got = read(&path, |p| {
        assert_eq!(p, path, "asked about another path");
        asked.set(asked.get() + 1);
        Ok(asked.get() == 1)
    });
    assert!(matches!(got, Err(NotRead::OnlineOnly)), "{got:?}");
    assert_eq!(asked.get(), 2, "asked before each open");
}

#[test]
fn a_file_whose_attributes_cannot_be_read_is_unreachable_and_never_opened() {
    // It exists and is fine, but the online-only check can't tell: it
    // mustn't be opened, since opening an unclassified file could download it.
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.mp3", &audio::mp3());
    let got = read(&path, |_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "no attributes",
        ))
    });
    assert!(
        matches!(&got, Err(NotRead::Unreachable(e)) if e.kind() == std::io::ErrorKind::PermissionDenied),
        "{got:?}"
    );
}

/// An RF64 WAVE (the 64-bit WAV for files over 4 GB): sizes of 0xFFFFFFFF,
/// the real ones in `ds64`. Built like the sniffer's own fixture.
fn rf64() -> Vec<u8> {
    let wav = audio::wav();
    // test_audio's WAV is RIFF, size, WAVE, `fmt ` (16), then `data`.
    let fmt = wav[12..12 + 8 + 16].to_vec();
    let pcm = &wav[12 + 8 + 16 + 8..];
    let mut ds64 = b"ds64".to_vec();
    ds64.extend_from_slice(&28u32.to_le_bytes());
    ds64.extend_from_slice(&(wav.len() as u64 + 36).to_le_bytes()); // RIFF size
    ds64.extend_from_slice(&(pcm.len() as u64).to_le_bytes()); // data size
    ds64.extend_from_slice(&0u64.to_le_bytes()); // sample count
    ds64.extend_from_slice(&0u32.to_le_bytes()); // table length
    let mut data = b"data".to_vec();
    data.extend_from_slice(&u32::MAX.to_le_bytes());
    data.extend_from_slice(pcm);
    let mut out = b"RF64".to_vec();
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend(ds64);
    out.extend(fmt);
    out.extend(data);
    out
}

/// RIFX: a WAVE with big-endian sizes and fields.
fn rifx() -> Vec<u8> {
    let mut fmt = b"fmt ".to_vec();
    fmt.extend_from_slice(&16u32.to_be_bytes());
    fmt.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0x1F, 0x40, 0, 0, 0x3E, 0x80, 0, 2, 0, 16]);
    let mut data = b"data".to_vec();
    data.extend_from_slice(&64u32.to_be_bytes());
    data.extend_from_slice(&[0; 64]);
    let mut out = b"RIFX".to_vec();
    out.extend_from_slice(&((4 + fmt.len() + data.len()) as u32).to_be_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend(fmt);
    out.extend(data);
    out
}

#[test]
fn a_valid_rf64_or_rifx_wav_the_tag_reader_cannot_read_is_not_called_broken() {
    for (name, bytes, format) in [
        ("big.wav", rf64(), SniffedFormat::Rf64),
        ("mac.wav", rifx(), SniffedFormat::Wav),
    ] {
        let got = read_bytes(name, &bytes);
        assert_eq!(got.sniffed_format, format, "{name}");
        assert_eq!(got.codec, Some(Codec::Pcm), "{name}");
        assert_eq!(got.verdict, None, "{name}: {got:?}");
    }
}

#[test]
fn a_flac_whose_encoder_did_not_know_its_length_is_not_broken() {
    // STREAMINFO's total samples (36 bits from byte 21) = 0 means unknown:
    // streaming encoders write it before they know.
    let mut flac = audio::flac();
    flac[21] &= 0xF0;
    flac[22..26].copy_from_slice(&[0; 4]);
    let got = read_bytes("stream.flac", &flac);
    assert_eq!(got.sniffed_format, SniffedFormat::Flac);
    assert_eq!(got.codec, Some(Codec::Flac));
    assert_ne!(
        got.duration_ms,
        Some(0),
        "a 0 here is unknown, stored as none"
    );
    assert_eq!(got.verdict, None, "{got:?}");
}

#[test]
fn tags_that_could_not_be_read_at_all_are_stored_as_unknown_not_as_untagged() {
    // The only ID3 frame claims text encoding 9, which doesn't exist: no
    // frame survives, but the audio is fine.
    let tag = id3v2(&[id3_frame(b"TPE1", &[9, 0xFF, 0xFE, 0x00])], 0);
    let got = read_bytes("a.mp3", &[tag, audio::mp3()].concat());
    assert_eq!(got.raw_tags, None, "{got:?}");
    assert_eq!(got.sample_rate, Some(44_100));
    assert_eq!(got.verdict, None);
    // An untagged file is `{}`, which is different.
    assert_eq!(
        read_bytes("b.mp3", &audio::mp3()).raw_tags.as_deref(),
        Some("{}")
    );
}
