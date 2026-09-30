//! Format sniffing tests. Every file is built as bytes here (most from
//! `tags::test_audio`); nothing is committed and nothing from
//! `spikes/results/` is read. The trap tests mirror the fixture generator's
//! trap kinds (`tools/fixture-gen`, `TrapKind`).

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use super::{sniff, Problem, Sniff, SniffedFormat as F};
use crate::tags::test_audio::{self, ape_tag, id3_text, id3v1_tag, id3v2, Format, Rng};

fn sniffed(bytes: &[u8], extension: &str) -> Sniff {
    sniff(&mut Cursor::new(bytes), Some(extension)).unwrap()
}

/// What a clean file of `format` sniffs as.
fn expected(format: Format) -> F {
    match format {
        Format::Mp3 => F::Mp3,
        Format::Flac => F::Flac,
        Format::Wav => F::Wav,
        Format::Aiff => F::Aiff,
        Format::M4a => F::Mp4,
        Format::Ogg => F::OggVorbis,
        Format::Opus => F::OggOpus,
    }
}

fn clean(format: F) -> Sniff {
    Sniff {
        format,
        problem: None,
        extension_disagrees: false,
    }
}

// ---- builders for formats test_audio doesn't make --------------------------

fn le32(n: u32) -> [u8; 4] {
    n.to_le_bytes()
}

fn riff_chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend_from_slice(&le32(body.len() as u32));
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn riff(magic: &[u8; 4], size: u32, chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut out = magic.to_vec();
    out.extend_from_slice(&le32(size));
    out.extend_from_slice(b"WAVE");
    for chunk in chunks {
        out.extend_from_slice(chunk);
    }
    out
}

/// A WAVE whose format tag is 0x0055 (MPEG Layer III) and whose data chunk
/// holds MP3 frames: an MP3 wrapped in a WAV, which is still a WAV.
fn mp3_in_wav() -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0x0055u16.to_le_bytes()); // WAVE_FORMAT_MPEGLAYER3
    fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
    fmt.extend_from_slice(&le32(44_100));
    fmt.extend_from_slice(&le32(16_000)); // byte rate
    fmt.extend_from_slice(&1u16.to_le_bytes()); // block align
    fmt.extend_from_slice(&0u16.to_le_bytes()); // bits per sample
    fmt.extend_from_slice(&12u16.to_le_bytes()); // extra size
    fmt.extend_from_slice(&[1, 0, 2, 0, 0, 0, 0x01, 0xA1, 1, 0, 0x71, 0x05]); // MPEGLAYER3WAVEFORMAT
    let chunks = [
        riff_chunk(b"fmt ", &fmt),
        riff_chunk(b"data", &test_audio::mp3()),
    ];
    let size = 4 + chunks.iter().map(Vec::len).sum::<usize>() as u32;
    riff(b"RIFF", size, &chunks)
}

/// An RF64 file: sizes of 0xFFFFFFFF, the real ones in `ds64`.
fn rf64(magic: &[u8; 4]) -> Vec<u8> {
    let wav = test_audio::wav();
    // test_audio's WAV is RIFF, size, WAVE, `fmt ` (16), then `data`.
    let fmt = wav[12..12 + 8 + 16].to_vec();
    let pcm = &wav[12 + 8 + 16 + 8..];
    let mut ds64 = Vec::new();
    ds64.extend_from_slice(&(wav.len() as u64 + 28).to_le_bytes()); // RIFF size
    ds64.extend_from_slice(&(pcm.len() as u64).to_le_bytes()); // data size
    ds64.extend_from_slice(&0u64.to_le_bytes()); // sample count
    ds64.extend_from_slice(&0u32.to_le_bytes()); // table length
    let mut data = b"data".to_vec();
    data.extend_from_slice(&le32(u32::MAX));
    data.extend_from_slice(pcm);
    riff(magic, u32::MAX, &[riff_chunk(b"ds64", &ds64), fmt, data])
}

/// RIFX: a WAVE with big-endian sizes.
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

fn aifc() -> Vec<u8> {
    let mut aiff = test_audio::aiff();
    aiff[8..12].copy_from_slice(b"AIFC");
    aiff
}

/// One ADTS frame of `len` bytes: AAC-LC, 44.1 kHz, stereo.
fn adts_frame(len: usize) -> Vec<u8> {
    let mut frame = vec![
        0xFF,
        0xF1, // MPEG-4, layer 0, no CRC
        (1 << 6) | (4 << 2),
        (2 << 6) | ((len >> 11) & 3) as u8,
        ((len >> 3) & 0xFF) as u8,
        (((len & 7) << 5) as u8) | 0x1F,
        0xFC,
    ];
    frame.resize(len, 0);
    frame
}

fn adts() -> Vec<u8> {
    adts_frame(120).repeat(10)
}

/// The ASF Header Object's GUID and size, then filler.
fn asf() -> Vec<u8> {
    let mut out = vec![
        0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE,
        0x6C,
    ];
    out.extend_from_slice(&64u64.to_le_bytes());
    out.extend_from_slice(&le32(0)); // object count
    out.extend_from_slice(&[1, 2]); // reserved
    out.resize(64, 0);
    out
}

fn magic_then_zeros(magic: &[u8], len: usize) -> Vec<u8> {
    let mut out = magic.to_vec();
    out.resize(len, 0);
    out
}

/// An Ogg stream whose first packet is FLAC's (`\x7FFLAC`).
fn ogg_flac() -> Vec<u8> {
    let mut ogg = test_audio::ogg_vorbis();
    let packet = 27 + ogg[26] as usize;
    ogg[packet..packet + 7].copy_from_slice(b"\x7FFLAC\x01\x00");
    ogg
}

/// MPEG-2 Layer III, 64 kbps, 22.05 kHz: frames of 72 * 64000 / 22050 bytes.
fn mpeg2_layer3() -> Vec<u8> {
    let mut frame = vec![0u8; 72 * 64_000 / 22_050];
    frame[..4].copy_from_slice(&[0xFF, 0xF3, 0x80, 0xC0]);
    frame.repeat(10)
}

/// The top-level boxes of an MP4, in order, as (type, whole box).
fn boxes(mp4: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < mp4.len() {
        let size = u32::from_be_bytes(mp4[at..at + 4].try_into().unwrap()) as usize;
        let kind = mp4[at + 4..at + 8].try_into().unwrap();
        out.push((kind, mp4[at..at + size].to_vec()));
        at += size;
    }
    out
}

fn m4a_without(kind: &[u8; 4]) -> Vec<u8> {
    boxes(&test_audio::m4a())
        .into_iter()
        .filter(|(k, _)| k != kind)
        .flat_map(|(_, b)| b)
        .collect()
}

/// A tagged MP3, like the fixture generator's: an ID3v2 tag, then frames.
fn tagged_mp3() -> Vec<u8> {
    let tag = id3v2(
        &[
            id3_text(b"TIT2", "Sable Run"),
            id3_text(b"TPE1", "Orrin Vale"),
        ],
        256,
    );
    [tag, test_audio::mp3()].concat()
}

// ---- every format --------------------------------------------------------

#[test]
fn each_format_is_detected_from_its_bytes() {
    for format in Format::ALL {
        assert_eq!(
            sniffed(&format.bytes(), format.extension()),
            clean(expected(format)),
            "{format:?}"
        );
    }
    for (bytes, ext, want) in [
        (rf64(b"RF64"), "wav", F::Rf64),
        (rf64(b"BW64"), "wav", F::Rf64),
        (rifx(), "wav", F::Wav),
        (aifc(), "aif", F::Aifc),
        (adts(), "aac", F::Adts),
        (asf(), "wma", F::Asf),
        (ogg_flac(), "oga", F::Ogg),
        (mpeg2_layer3(), "mp3", F::Mp3),
        (magic_then_zeros(b"wvpk", 64), "wv", F::WavPack),
        (magic_then_zeros(b"MPCK", 64), "mpc", F::Musepack),
        (magic_then_zeros(b"MP+\x07", 64), "mpc", F::Musepack),
        (magic_then_zeros(b"MAC ", 64), "ape", F::Ape),
    ] {
        assert_eq!(sniffed(&bytes, ext), clean(want), "{want:?}");
    }
}

#[test]
fn every_format_has_a_distinct_stored_name() {
    let all = [
        F::Mp3,
        F::Wav,
        F::Rf64,
        F::Aiff,
        F::Aifc,
        F::Flac,
        F::Mp4,
        F::Adts,
        F::OggVorbis,
        F::OggOpus,
        F::Ogg,
        F::Asf,
        F::WavPack,
        F::Musepack,
        F::Ape,
        F::Unknown,
    ];
    let mut names: Vec<_> = all.iter().map(|f| f.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all.len());
}

// ---- the fixture generator's traps ------------------------------------------

#[test]
fn trap_mp3_in_wav_a_wav_name_holding_a_tagged_mp3_sniffs_as_mp3_and_disagrees() {
    let found = sniffed(&tagged_mp3(), "wav");
    assert_eq!(found.format, F::Mp3);
    assert!(found.extension_disagrees);
    assert_eq!(found.problem, None);
}

#[test]
fn trap_m4a_no_moov_an_interrupted_m4a_is_mp4_flagged_no_moov() {
    let found = sniffed(&m4a_without(b"moov"), "m4a");
    assert_eq!(found.format, F::Mp4);
    assert_eq!(found.problem, Some(Problem::NoMoov));
    assert!(!found.extension_disagrees);
}

#[test]
fn trap_empty_file_zero_bytes_is_unknown_and_empty() {
    let found = sniffed(&[], "mp3");
    assert_eq!(
        found,
        Sniff {
            format: F::Unknown,
            problem: Some(Problem::Empty),
            extension_disagrees: false,
        }
    );
}

#[test]
fn trap_bad_tdrc_an_invalid_date_in_the_tag_does_not_stop_mp3_detection() {
    let tag = id3v2(
        &[
            id3_text(b"TIT2", "Bad Date"),
            id3_text(b"TDRC", "2019-13-45"),
        ],
        64,
    );
    let found = sniffed(&[tag, test_audio::mp3()].concat(), "mp3");
    assert_eq!(found, clean(F::Mp3));
}

#[test]
fn trap_broken_ape_a_lying_ape_footer_does_not_stop_mp3_detection() {
    let mut ape = ape_tag(&[("Title", "Broken Ape")]);
    let footer = ape.len() - 32;
    // Claim a 16 MB tag with 7 items, as the fixture generator does.
    ape[footer + 12..footer + 16].copy_from_slice(&le32(16 << 20));
    ape[footer + 16..footer + 20].copy_from_slice(&le32(7));
    let file = [tagged_mp3(), ape, id3v1_tag("Broken Ape")].concat();
    assert_eq!(sniffed(&file, "mp3"), clean(F::Mp3));
}

// ---- tags in front ---------------------------------------------------------

#[test]
fn an_mp3_without_any_tag_is_an_mp3() {
    assert_eq!(sniffed(&test_audio::mp3(), "mp3"), clean(F::Mp3));
}

#[test]
fn an_id3_tag_is_skipped_by_its_size_field_however_large() {
    // 3 MB of padding, as a big embedded artwork would take.
    let tag = id3v2(&[id3_text(b"TIT2", "Big Art")], 3 << 20);
    let file = [tag, test_audio::mp3()].concat();
    let mut reader = Counting::new(Cursor::new(file));
    let found = sniff(&mut reader, Some("mp3")).unwrap();
    assert_eq!(found, clean(F::Mp3));
    assert!(reader.read < 16 * 1024, "read {} bytes", reader.read);
}

#[test]
fn an_id3v24_tag_with_a_footer_is_skipped_including_the_footer() {
    let mut tag = id3v2(&[id3_text(b"TIT2", "Footer")], 0);
    tag[5] |= 0x10; // footer present
    let mut footer = tag[..10].to_vec();
    footer[..3].copy_from_slice(b"3DI");
    let file = [tag, footer, test_audio::mp3()].concat();
    assert_eq!(sniffed(&file, "mp3"), clean(F::Mp3));
}

#[test]
fn stacked_id3_tags_are_all_skipped() {
    let one = id3v2(&[id3_text(b"TIT2", "Old")], 16);
    let two = id3v2(&[id3_text(b"TIT2", "New")], 16);
    assert_eq!(
        sniffed(&[two, one, test_audio::mp3()].concat(), "mp3"),
        clean(F::Mp3)
    );
}

#[test]
fn flac_after_an_id3_tag_is_still_flac() {
    let tag = id3v2(&[id3_text(b"TIT2", "Tagged FLAC")], 32);
    assert_eq!(
        sniffed(&[tag, test_audio::flac()].concat(), "flac"),
        clean(F::Flac)
    );
}

#[test]
fn mp3_frames_after_junk_following_the_tag_are_found() {
    let tag = id3v2(&[id3_text(b"TIT2", "Junk")], 0);
    let junk = vec![0x55u8; 700];
    assert_eq!(
        sniffed(&[tag, junk, test_audio::mp3()].concat(), "mp3"),
        clean(F::Mp3)
    );
}

// ---- containers --------------------------------------------------------------

#[test]
fn a_wave_whose_format_tag_is_not_pcm_is_still_a_wav() {
    // MP3 inside a WAV container is a WAV (the codec is 1aB-3's business).
    let found = sniffed(&mp3_in_wav(), "wav");
    assert_eq!(found, clean(F::Wav));
}

#[test]
fn aifc_is_told_apart_from_aiff_and_either_extension_is_fine() {
    for ext in ["aif", "aiff", "aifc"] {
        assert_eq!(sniffed(&aifc(), ext), clean(F::Aifc));
        assert_eq!(sniffed(&test_audio::aiff(), ext), clean(F::Aiff));
    }
}

#[test]
fn aiff_chunks_are_found_in_any_order() {
    let aiff = test_audio::aiff();
    // FORM header, then COMM (8 + 18 bytes), then SSND: swap the two.
    let comm = &aiff[12..12 + 26];
    let ssnd = &aiff[12 + 26..];
    let swapped = [&aiff[..12], ssnd, comm].concat();
    assert_eq!(sniffed(&swapped, "aiff"), clean(F::Aiff));
}

#[test]
fn ogg_vorbis_opus_and_other_ogg_codecs_are_told_apart() {
    assert_eq!(
        sniffed(&test_audio::ogg_vorbis(), "ogg").format,
        F::OggVorbis
    );
    assert_eq!(sniffed(&test_audio::ogg_opus(), "ogg").format, F::OggOpus);
    assert_eq!(sniffed(&ogg_flac(), "ogg").format, F::Ogg);
}

#[test]
fn an_m4a_with_moov_after_mdat_is_complete() {
    let moov_last: Vec<u8> = {
        let b = boxes(&test_audio::m4a());
        let order: [&[u8; 4]; 3] = [b"ftyp", b"mdat", b"moov"];
        order
            .iter()
            .flat_map(|k| b.iter().find(|(kind, _)| kind == *k).unwrap().1.clone())
            .collect()
    };
    assert_eq!(sniffed(&moov_last, "m4a"), clean(F::Mp4));
}

#[test]
fn a_4_gb_mdat_is_skipped_by_its_64_bit_size_not_read() {
    let ftyp = boxes(&test_audio::m4a())[0].1.clone();
    let moov = boxes(&test_audio::m4a())[1].1.clone();
    let body: u64 = 4 << 30;
    let mut mdat = 1u32.to_be_bytes().to_vec();
    mdat.extend_from_slice(b"mdat");
    mdat.extend_from_slice(&(16 + body).to_be_bytes());
    let head = [ftyp, mdat].concat();
    let moov_at = head.len() as u64 + body;
    let mut reader = Counting::new(Sparse::new(head, moov_at, moov));
    assert_eq!(sniff(&mut reader, Some("m4a")).unwrap(), clean(F::Mp4));
    assert!(reader.read < 16 * 1024, "read {} bytes", reader.read);
}

#[test]
fn a_4_gb_wav_is_sniffed_from_its_headers_only() {
    let wav = test_audio::wav();
    let head = wav[..12 + 8 + 16 + 8].to_vec();
    let len = u64::from(u32::MAX - 1);
    let mut file = head.clone();
    // The data chunk claims everything after the headers.
    let data_size = (len - head.len() as u64) as u32;
    file[40..44].copy_from_slice(&le32(data_size));
    let mut reader = Counting::new(Sparse::new(file, len, Vec::new()));
    assert_eq!(sniff(&mut reader, Some("wav")).unwrap(), clean(F::Wav));
    assert!(reader.read < 16 * 1024, "read {} bytes", reader.read);
}

// ---- extensions --------------------------------------------------------------

#[test]
fn the_extension_check_ignores_letter_case() {
    assert!(!sniffed(&test_audio::wav(), "WAV").extension_disagrees);
    assert!(!sniffed(&test_audio::mp3(), "Mp3").extension_disagrees);
    assert!(sniffed(&test_audio::mp3(), "FLAC").extension_disagrees);
}

#[test]
fn a_known_format_with_no_extension_disagrees() {
    let found = sniff(&mut Cursor::new(test_audio::flac()), None).unwrap();
    assert_eq!(found.format, F::Flac);
    assert!(found.extension_disagrees);
}

#[test]
fn every_mislabeled_pairing_of_real_formats_disagrees() {
    for bytes_of in Format::ALL {
        for named in Format::ALL {
            let found = sniffed(&bytes_of.bytes(), named.extension());
            assert_eq!(found.format, expected(bytes_of));
            let honest = expected(bytes_of).extensions().contains(&named.extension());
            assert_eq!(
                found.extension_disagrees,
                !honest,
                "{bytes_of:?} as .{}",
                named.extension()
            );
        }
    }
}

#[test]
fn bytes_that_are_not_audio_are_unknown_and_never_disagree() {
    for (bytes, ext) in [
        (&b"FILE \"Mix.wav\" WAVE\r\n  TRACK 01 AUDIO\r\n"[..], "mp3"),
        (b"<!DOCTYPE html><html><body>404</body></html>", "m4a"),
        (b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", "flac"),
        (b"RIFF\x24\0\0\0AVI LIST", "wav"),
        (b"FORM\0\0\0\x10ILBMBMHD", "aiff"),
    ] {
        let found = sniffed(bytes, ext);
        assert_eq!(
            found.format,
            F::Unknown,
            "{}",
            String::from_utf8_lossy(bytes)
        );
        assert!(!found.extension_disagrees);
    }
}

#[test]
fn adts_aac_is_not_mistaken_for_mp3_nor_mp3_for_adts() {
    assert_eq!(sniffed(&adts(), "mp3").format, F::Adts);
    assert_eq!(sniffed(&test_audio::mp3(), "aac").format, F::Mp3);
}

// ---- truncated and damaged ---------------------------------------------------

#[test]
fn a_tag_that_runs_past_the_end_of_the_file_is_unknown_and_truncated() {
    let tag = id3v2(&[id3_text(b"TIT2", "Cut")], 1 << 20);
    let found = sniffed(&tag[..200], "mp3");
    assert_eq!(
        (found.format, found.problem),
        (F::Unknown, Some(Problem::Truncated))
    );
    // A tag and nothing after it: the audio is missing.
    let found = sniffed(&tag, "mp3");
    assert_eq!(
        (found.format, found.problem),
        (F::Unknown, Some(Problem::Truncated))
    );
}

#[test]
fn a_wav_cut_off_mid_audio_is_a_truncated_wav() {
    let wav = test_audio::wav();
    let found = sniffed(&wav[..wav.len() / 2], "wav");
    assert_eq!(
        (found.format, found.problem),
        (F::Wav, Some(Problem::Truncated))
    );
}

#[test]
fn a_wav_cut_off_before_its_audio_chunk_is_a_truncated_wav() {
    let wav = test_audio::wav();
    let found = sniffed(&wav[..12 + 8 + 16], "wav");
    assert_eq!(
        (found.format, found.problem),
        (F::Wav, Some(Problem::Truncated))
    );
}

#[test]
fn an_aiff_cut_off_mid_audio_is_a_truncated_aiff() {
    let aiff = test_audio::aiff();
    let found = sniffed(&aiff[..aiff.len() - 100], "aiff");
    assert_eq!(
        (found.format, found.problem),
        (F::Aiff, Some(Problem::Truncated))
    );
}

#[test]
fn an_m4a_cut_off_inside_a_box_is_a_truncated_mp4() {
    let m4a = test_audio::m4a();
    let found = sniffed(&m4a[..m4a.len() - 10], "m4a");
    assert_eq!(
        (found.format, found.problem),
        (F::Mp4, Some(Problem::Truncated))
    );
}

#[test]
fn a_flac_or_ogg_cut_off_in_its_first_header_is_truncated() {
    let found = sniffed(&test_audio::flac()[..20], "flac");
    assert_eq!(
        (found.format, found.problem),
        (F::Flac, Some(Problem::Truncated))
    );
    let found = sniffed(&test_audio::ogg_vorbis()[..20], "ogg");
    assert_eq!(
        (found.format, found.problem),
        (F::Ogg, Some(Problem::Truncated))
    );
}

#[test]
fn a_quicktime_file_starting_with_moov_or_mdat_is_mp4_with_its_problem() {
    let moov_first = m4a_without(b"ftyp");
    assert_eq!(sniffed(&moov_first, "m4a"), clean(F::Mp4));
    let found = sniffed(&moov_first[..moov_first.len() - 10], "m4a");
    assert_eq!(
        (found.format, found.problem),
        (F::Mp4, Some(Problem::Truncated))
    );
    let mdat_only: Vec<u8> = boxes(&test_audio::m4a())
        .into_iter()
        .filter(|(k, _)| k == b"mdat")
        .flat_map(|(_, b)| b)
        .collect();
    let found = sniffed(&mdat_only, "m4a");
    assert_eq!(
        (found.format, found.problem),
        (F::Mp4, Some(Problem::NoMoov))
    );
}

#[test]
fn a_leading_padding_box_counts_as_mp4_only_with_a_moov() {
    let mut free = vec![0, 0, 0, 8];
    free.extend_from_slice(b"free");
    let padded = [free, m4a_without(b"ftyp")].concat();
    assert_eq!(sniffed(&padded, "m4a"), clean(F::Mp4));
    // Text that happens to have "free" at offset 4 isn't a QuickTime file.
    assert_eq!(
        sniffed(b"Feelfree to ignore this file", "m4a").format,
        F::Unknown
    );
}

// ---- hostile sizes: no panic, and reads stay bounded -------------------------

/// Sniffs `bytes` through a counting reader, checking the read bound.
fn sniffed_bounded(bytes: Vec<u8>, extension: &str) -> Sniff {
    let mut reader = Counting::new(Cursor::new(bytes));
    let found = sniff(&mut reader, Some(extension)).unwrap();
    assert!(reader.read < 16 * 1024, "read {} bytes", reader.read);
    found
}

fn ftyp_then(rest: &[u8]) -> Vec<u8> {
    [boxes(&test_audio::m4a())[0].1.clone(), rest.to_vec()].concat()
}

fn large_box(kind: &[u8; 4], size: &[u8]) -> Vec<u8> {
    [&1u32.to_be_bytes()[..], kind, size].concat()
}

#[test]
fn a_box_claiming_u64_max_bytes_is_truncated_not_an_overflow() {
    let file = ftyp_then(&large_box(b"mdat", &u64::MAX.to_be_bytes()));
    let found = sniffed_bounded(file, "m4a");
    assert_eq!(
        (found.format, found.problem),
        (F::Mp4, Some(Problem::Truncated))
    );
}

#[test]
fn a_64_bit_box_cut_off_inside_its_size_field_is_truncated() {
    let file = ftyp_then(&large_box(b"mdat", &[0, 0, 0]));
    let found = sniffed_bounded(file, "m4a");
    assert_eq!(
        (found.format, found.problem),
        (F::Mp4, Some(Problem::Truncated))
    );
}

#[test]
fn a_64_bit_box_smaller_than_its_own_header_is_truncated() {
    let mut rest = large_box(b"mdat", &8u64.to_be_bytes());
    rest.extend_from_slice(&[0; 32]);
    let found = sniffed_bounded(ftyp_then(&rest), "m4a");
    assert_eq!(
        (found.format, found.problem),
        (F::Mp4, Some(Problem::Truncated))
    );
}

#[test]
fn a_long_chain_of_empty_boxes_stops_at_the_box_limit() {
    let mut free = vec![0, 0, 0, 8];
    free.extend_from_slice(b"free");
    let file = ftyp_then(&free.repeat(super::MAX_CHUNKS * 4));
    // Too many to follow: the verdict is left to the full reader.
    assert_eq!(sniffed_bounded(file, "m4a"), clean(F::Mp4));
}

#[test]
fn a_long_chain_of_zero_size_chunks_stops_at_the_chunk_limit() {
    let chunks = |id: &[u8; 4]| [&id[..], &[0u8; 4]].concat().repeat(super::MAX_CHUNKS * 4);
    let mut wav = riff(b"RIFF", 0, &[]);
    wav.extend(chunks(b"junk"));
    assert_eq!(sniffed_bounded(wav, "wav"), clean(F::Wav));

    let mut aiff = b"FORM\0\0\0\0AIFF".to_vec();
    aiff.extend(chunks(b"COMT"));
    assert_eq!(sniffed_bounded(aiff, "aiff"), clean(F::Aiff));
}

#[test]
fn a_riff_or_form_magic_shorter_than_its_form_header_is_unknown_and_truncated() {
    for magic in [&b"RIFF"[..], b"RIFX", b"RF64", b"BW64", b"FORM"] {
        let file = [magic, &[0u8; 4]].concat();
        let found = sniffed(&file, "wav");
        assert_eq!(
            (found.format, found.problem),
            (F::Unknown, Some(Problem::Truncated))
        );
    }
}

/// Cutting a file anywhere gives its own format or Unknown, never some
/// other format, and never a panic. An Ogg file cut before its codec's
/// first packet can only be plain `Ogg`.
#[test]
fn every_prefix_of_every_format_sniffs_as_that_format_or_unknown() {
    let mut files: Vec<(Vec<u8>, F)> = Format::ALL
        .iter()
        .map(|f| (f.bytes(), expected(*f)))
        .collect();
    files.extend([
        (tagged_mp3(), F::Mp3),
        (mp3_in_wav(), F::Wav),
        (rf64(b"RF64"), F::Rf64),
        (aifc(), F::Aifc),
        (adts(), F::Adts),
        (asf(), F::Asf),
    ]);
    for (bytes, want) in files {
        for cut in 0..bytes.len() {
            let found = sniffed(&bytes[..cut], "x");
            let container_only =
                found.format == F::Ogg && matches!(want, F::OggVorbis | F::OggOpus);
            assert!(
                found.format == want || found.format == F::Unknown || container_only,
                "{want:?} cut at {cut}: {found:?}"
            );
            if container_only {
                assert_eq!(
                    found.problem,
                    Some(Problem::Truncated),
                    "{want:?} cut at {cut}"
                );
            }
            if cut == 0 {
                assert_eq!(found.problem, Some(Problem::Empty));
            }
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x5EED_CAFE_F00D_0001);
    let mut unknown = 0;
    for round in 0..3000 {
        let len = rng.below(if round % 10 == 0 { 20_000 } else { 600 });
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if sniffed(&bytes, "mp3").format == F::Unknown {
            unknown += 1;
        }
    }
    // Noise is almost never mistaken for audio.
    assert!(unknown > 2950, "only {unknown} of 3000 were unknown");
}

#[test]
fn random_damage_to_real_files_never_panics() {
    let mut rng = Rng(0xBAD_F11E);
    let mut files: Vec<Vec<u8>> = Format::ALL.iter().map(|f| f.bytes()).collect();
    files.extend([
        tagged_mp3(),
        mp3_in_wav(),
        rf64(b"RF64"),
        aifc(),
        adts(),
        asf(),
    ]);
    for _ in 0..4000 {
        let mut bytes = files[rng.below(files.len())].clone();
        // Damage the headers, where the sniffer looks.
        for _ in 0..1 + rng.below(8) {
            let at = rng.below(bytes.len().min(96));
            bytes[at] = rng.next() as u8;
        }
        let cut = rng.below(bytes.len() + 1);
        sniffed(&bytes[..cut], "wav");
    }
}

#[test]
fn a_read_error_is_returned_not_swallowed() {
    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("disk gone"))
        }
    }
    impl Seek for Failing {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            Ok(1000)
        }
    }
    assert!(sniff(&mut Failing, Some("mp3")).is_err());
}

// ---- opening files -------------------------------------------------------------

#[cfg(windows)]
mod files {
    use super::*;
    use crate::sniff::sniff_path;

    /// Writes `bytes` at `dir\rel` through the `\\?\` form, so names Win32
    /// would rewrite are created exactly (test setup only).
    fn write_exact(dir: &std::path::Path, rel: &str, bytes: &[u8]) -> std::path::PathBuf {
        let verbatim = crate::paths::verbatim_absolute(&dir.join(rel)).unwrap();
        std::fs::create_dir_all(verbatim.parent().unwrap()).unwrap();
        std::fs::write(&verbatim, bytes).unwrap();
        dir.join(rel)
    }

    #[test]
    fn a_folder_name_ending_in_a_dot_opens_that_folder_not_its_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let exact = write_exact(dir.path(), r"Q.X.Z.\a.wav", &tagged_mp3());
        write_exact(dir.path(), r"Q.X.Z\a.wav", &test_audio::wav());
        assert_eq!(sniff_path(&exact).unwrap().format, F::Mp3);
    }

    #[test]
    fn sniffing_a_file_leaves_its_bytes_and_modified_time_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_exact(dir.path(), "a.mp3", &tagged_mp3());
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();

        let found = sniff_path(&path).unwrap();

        assert_eq!(found, clean(F::Mp3));
        assert_eq!(std::fs::read(&path).unwrap(), tagged_mp3());
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );

        // Let the temp dir clean itself up.
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn a_relative_path_is_refused() {
        let err = sniff_path(std::path::Path::new(r"music\a.mp3")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = sniff_path(&dir.path().join("gone.mp3")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}

// ---- readers for the bounded-read tests --------------------------------------

/// Counts the bytes read through it.
struct Counting<R> {
    inner: R,
    read: u64,
}

impl<R> Counting<R> {
    fn new(inner: R) -> Self {
        Counting { inner, read: 0 }
    }
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        Ok(n)
    }
}

impl<R: Seek> Seek for Counting<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// A huge file without the memory: `head` at 0, `tail` at `tail_at`, and
/// zeros in between. It ends after `tail`, or at `tail_at` if `tail` is empty.
struct Sparse {
    head: Vec<u8>,
    tail_at: u64,
    tail: Vec<u8>,
    pos: u64,
}

impl Sparse {
    fn new(head: Vec<u8>, tail_at: u64, tail: Vec<u8>) -> Self {
        Sparse {
            head,
            tail_at,
            tail,
            pos: 0,
        }
    }

    fn len(&self) -> u64 {
        self.tail_at + self.tail.len() as u64
    }

    fn byte(&self, at: u64) -> u8 {
        if at < self.head.len() as u64 {
            self.head[at as usize]
        } else if at >= self.tail_at {
            self.tail[(at - self.tail_at) as usize]
        } else {
            0
        }
    }
}

impl Read for Sparse {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = (buf.len() as u64).min(self.len().saturating_sub(self.pos)) as usize;
        for (i, b) in buf[..n].iter_mut().enumerate() {
            *b = self.byte(self.pos + i as u64);
        }
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Sparse {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.pos = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::End(d) => self.len().checked_add_signed(d).unwrap(),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).unwrap(),
        };
        Ok(self.pos)
    }
}
