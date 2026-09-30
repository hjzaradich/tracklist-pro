//! Cases from the #33 review: format settings count as audio, parts can't
//! run into each other, guesses are refused, the first-frame check and
//! every `mdat` matter, and an APE tag is never taken for ID3v1.

use super::fixtures::*;
use crate::hash::Skip;
use crate::tags::test_audio::{self, ape_tag, id3v1_tag};

fn flip(bytes: &[u8], at: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[at] ^= 0x01;
    out
}

#[test]
fn the_wave_format_chunk_length_is_hashed_so_fmt_and_data_cannot_trade_bytes() {
    // A 16-byte fmt with data "00 00 S…", and an 18-byte fmt ending "00 00"
    // with data "S…": the same bytes in a row, but not the same audio.
    let s = samples();
    let short = riff(
        b"RIFF",
        &[
            chunk(b"fmt ", &wav_fmt(), false),
            chunk(b"data", &[vec![0, 0], s.clone()].concat(), false),
        ],
    );
    let long = riff(
        b"RIFF",
        &[
            chunk(b"fmt ", &[wav_fmt(), vec![0, 0]].concat(), false),
            chunk(b"data", &s, false),
        ],
    );
    assert_ne!(audio(&short), audio(&long));
}

#[test]
fn a_streaming_wave_whose_data_size_is_0_or_unknown_gets_no_audio_hash() {
    for size in [0u32, u32::MAX] {
        let mut data = b"data".to_vec();
        data.extend_from_slice(&size.to_le_bytes());
        data.extend_from_slice(&samples());
        let file = riff(b"RIFF", &[chunk(b"fmt ", &wav_fmt(), false), data]);
        assert_eq!(
            hash(&file).audio,
            Err(Skip::Malformed),
            "data size {size:#x}"
        );
    }
}

#[test]
fn flac_streaminfo_audio_settings_are_part_of_the_audio_and_retagging_still_is_not() {
    let base = flac_with(&[]);
    // STREAMINFO's body starts at byte 8; bytes 10–17 of it (file bytes
    // 18–25) hold the rate, channels, bits and total samples.
    for at in [18, 21, 25] {
        assert_ne!(
            audio(&flip(&base, at)),
            audio(&base),
            "STREAMINFO byte {}",
            at - 8
        );
    }
    // The metadata variants (comments, pictures, padding) still match.
    let variants = flac_variants();
    for (name, bytes) in &variants {
        assert_eq!(audio(bytes), audio(&base), "{name}");
    }
}

#[test]
fn mp4_sample_entries_are_part_of_the_audio_and_retagging_still_is_not() {
    let base = test_audio::m4a();
    let entry = base.windows(4).position(|w| w == b"mp4a").unwrap();
    // After the type: 6 reserved, 2 reference index, 8 reserved,
    // 2 channels, 2 sample size, 4 reserved, then the 4-byte rate.
    let channels = entry + 4 + 16;
    let rate = entry + 4 + 24;
    // Inside the decoder config (esds), near the end of the entry.
    let esds = base.windows(4).position(|w| w == b"esds").unwrap();
    for at in [channels + 1, rate + 1, esds + 20] {
        assert_ne!(
            audio(&flip(&base, at)),
            audio(&base),
            "sample entry byte {at}"
        );
    }
    for (name, bytes) in &mp4_variants() {
        assert_eq!(audio(bytes), audio(&base), "{name}");
    }
}

#[test]
fn an_mp4_without_a_sound_track_gets_no_audio_hash() {
    let base = test_audio::m4a();
    let at = base.windows(4).position(|w| w == b"soun").unwrap();
    let mut video = base.clone();
    video[at..at + 4].copy_from_slice(b"vide");
    assert_eq!(hash(&video).audio, Err(Skip::Malformed));
}

#[test]
fn every_mdat_box_is_part_of_the_audio_not_just_the_first() {
    let two = [test_audio::m4a(), mp4_box(b"mdat", &samples())].concat();
    let original = audio(&two);
    // The second mdat's last byte.
    assert_ne!(audio(&flip(&two, two.len() - 1)), original);
    // And a copy without it is different audio.
    assert_ne!(audio(&test_audio::m4a()), original);
}

#[test]
fn a_lone_frame_header_in_the_junk_before_the_frames_is_not_where_the_audio_starts() {
    let frames = mp3_frames();
    // Junk holding one MPEG header that no frame follows.
    let mut junk = vec![0u8; 50];
    junk.extend_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
    junk.extend(vec![0u8; 100]);
    let with_junk = [junk, frames.clone()].concat();
    assert_eq!(audio(&with_junk), audio(&frames));
}

/// An APE tag whose bytes 128 from its end read `TAG`: 93 bytes of the
/// value, then the 32-byte footer, follow them.
fn ape_tag_looking_like_id3v1() -> Vec<u8> {
    let value = format!("TAG{}", "y".repeat(93));
    let tag = ape_tag(&[("Title", &value)]);
    assert_eq!(&tag[tag.len() - 128..tag.len() - 125], b"TAG");
    tag
}

#[test]
fn an_ape_tag_is_never_taken_for_id3v1() {
    let frames = mp3_frames();
    let tag = ape_tag_looking_like_id3v1();
    assert_eq!(
        audio(&[frames.clone(), tag.clone()].concat()),
        audio(&frames)
    );
    // And with a real ID3v1 after it, both come off.
    assert_eq!(
        audio(&[frames.clone(), tag, id3v1_tag("x")].concat()),
        audio(&frames)
    );
}

// ---- MP4 sound tracks (second review) ---------------------------------------

type Boxes = Vec<([u8; 4], Vec<u8>)>;

/// The boxes laid end to end in `bytes`.
fn boxes(bytes: &[u8]) -> Boxes {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let id: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
        out.push((id, bytes[at + 8..at + size].to_vec()));
        at += size;
    }
    out
}

fn build(list: &Boxes) -> Vec<u8> {
    list.iter()
        .flat_map(|(id, body)| mp4_box(id, body))
        .collect()
}

/// `list` with the box at `path` (ids, first match at each level) edited.
fn edit(list: &Boxes, path: &[&[u8; 4]], f: &dyn Fn(&mut Vec<u8>)) -> Boxes {
    let mut list = list.clone();
    let (first, rest) = path.split_first().unwrap();
    let (_, body) = list.iter_mut().find(|(id, _)| id == *first).unwrap();
    if rest.is_empty() {
        f(body);
    } else {
        *body = build(&edit(&boxes(body), rest, f));
    }
    list
}

/// The generated M4A's top level, with its `moov` children replaced.
fn m4a_with_moov(children: &Boxes) -> Vec<u8> {
    let top: Boxes = m4a_boxes()
        .into_iter()
        .map(|(id, body)| {
            if &id == b"moov" {
                (id, build(children))
            } else {
                (id, body)
            }
        })
        .collect();
    build(&top)
}

fn moov_children() -> Boxes {
    let moov = m4a_boxes()
        .into_iter()
        .find(|(id, _)| id == b"moov")
        .unwrap()
        .1;
    boxes(&moov)
}

/// The generated sound track turned into a video track, with a sample
/// entry of its own.
fn video_trak() -> ([u8; 4], Vec<u8>) {
    let trak = moov_children()
        .into_iter()
        .find(|(id, _)| id == b"trak")
        .unwrap();
    let edited = edit(&boxes(&trak.1), &[b"mdia", b"hdlr"], &|hdlr| {
        hdlr[8..12].copy_from_slice(b"vide")
    });
    let edited = edit(&edited, &[b"mdia", b"minf", b"stbl", b"stsd"], &|stsd| {
        let last = stsd.len() - 1;
        stsd[last] ^= 0x5A;
    });
    (*b"trak", build(&edited))
}

/// mvhd, a video track, then the sound track.
fn video_then_sound(video: ([u8; 4], Vec<u8>)) -> Boxes {
    let moov = moov_children();
    let mut children = vec![moov[0].clone(), video];
    children.extend(moov[1..].iter().cloned());
    children
}

#[test]
fn a_video_track_before_the_sound_track_is_not_part_of_the_audio() {
    let base = audio(&test_audio::m4a());
    let mixed = m4a_with_moov(&video_then_sound(video_trak()));
    assert_eq!(
        audio(&mixed),
        base,
        "the video track's sample entry was hashed"
    );
    // Another video sample entry: still the same audio.
    let (id, body) = video_trak();
    let other = edit(
        &boxes(&body),
        &[b"mdia", b"minf", b"stbl", b"stsd"],
        &|stsd| stsd[20] ^= 0x01,
    );
    let mixed_other = m4a_with_moov(&video_then_sound((id, build(&other))));
    assert_eq!(audio(&mixed_other), base);
}

#[test]
fn a_track_whose_mdia_has_no_hdlr_or_a_short_one_is_not_a_sound_track() {
    let trak = moov_children()
        .into_iter()
        .find(|(id, _)| id == b"trak")
        .unwrap();
    let mdia = boxes(
        &boxes(&trak.1)
            .into_iter()
            .find(|(id, _)| id == b"mdia")
            .unwrap()
            .1,
    );
    for (name, hdlr) in [("no hdlr", None), ("an 8-byte hdlr", Some(vec![0u8; 8]))] {
        let mut children: Boxes = mdia
            .iter()
            .filter(|(id, _)| id != b"hdlr")
            .cloned()
            .collect();
        if let Some(body) = hdlr {
            children.insert(1, (*b"hdlr", body));
        }
        let trak_children = edit(&boxes(&trak.1), &[b"mdia"], &|body| {
            *body = build(&children)
        });
        let odd = (*b"trak", build(&trak_children));
        // Alone, there's no sound track at all.
        let alone = m4a_with_moov(&vec![moov_children()[0].clone(), odd.clone()]);
        assert_eq!(hash(&alone).audio, Err(Skip::Malformed), "{name}");
        // Beside a real sound track, it's ignored.
        let beside = m4a_with_moov(&video_then_sound(odd));
        assert_eq!(audio(&beside), audio(&test_audio::m4a()), "{name}");
    }
}

#[test]
fn a_stray_stsd_outside_stbl_is_not_part_of_the_audio() {
    let base = audio(&test_audio::m4a());
    let stray = (*b"stsd", vec![0xAB; 40]);
    for path in [
        &[b"trak"][..],
        &[b"trak", b"mdia"][..],
        &[b"trak", b"mdia", b"minf"][..],
    ] {
        let moov = edit(&moov_children(), path, &|body| {
            body.extend(mp4_box(&stray.0, &stray.1))
        });
        assert_eq!(audio(&m4a_with_moov(&moov)), base, "stsd in {path:?}");
    }
}
