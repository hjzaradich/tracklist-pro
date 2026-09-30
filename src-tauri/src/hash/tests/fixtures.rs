//! Audio files of every supported format, built byte by byte, plus every
//! kind of tag to wrap them in. The audio is the same whatever the tags,
//! which is what the tests check the audio_hash for.

use std::io::Cursor;

use crate::hash::{hash_reader, FileHashes};
use crate::tags::test_audio::{self, ape_tag, id3_text, id3v1_tag, id3v2};

/// Hashes `bytes` with a small buffer, so reads cross every structure.
pub fn hash(bytes: &[u8]) -> FileHashes {
    hash_with(bytes, 4096)
}

pub fn hash_with(bytes: &[u8], buffer: usize) -> FileHashes {
    hash_reader(&mut Cursor::new(bytes), &mut vec![0; buffer], &mut |_| {
        false
    })
    .unwrap()
    .expect("never stopped")
}

/// The stored audio_hash of `bytes`; panics with the reason if there's none.
pub fn audio(bytes: &[u8]) -> [u8; 34] {
    match hash(bytes).audio {
        Ok(a) => a.to_bytes(),
        Err(skip) => panic!("no audio_hash: {skip:?}"),
    }
}

/// Bytes that aren't all alike, so a changed byte always changes the audio.
fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ (i >> 8) as u8)
        .collect()
}

// ---- MPEG audio and ADTS --------------------------------------------------

/// MPEG-1 Layer III frames, 128 kbps, 44.1 kHz, each with its own bytes.
pub fn mp3_frames() -> Vec<u8> {
    const FRAME_LEN: usize = 417;
    let mut out = Vec::new();
    for n in 0..24u8 {
        let mut frame = pattern(FRAME_LEN, n);
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
        out.extend(frame);
    }
    out
}

/// AAC-LC ADTS frames, 44.1 kHz, mono, 64 bytes each.
pub fn adts_frames() -> Vec<u8> {
    const FRAME_LEN: usize = 64;
    let mut out = Vec::new();
    for n in 0..24u8 {
        let mut frame = pattern(FRAME_LEN, n);
        let len = FRAME_LEN as u32;
        frame[..7].copy_from_slice(&[
            0xFF,
            0xF1, // MPEG-4, layer 0, no CRC
            0x50, // AAC LC, 44.1 kHz
            0x40 | ((len >> 11) & 3) as u8,
            ((len >> 3) & 0xFF) as u8,
            (((len & 7) << 5) | 0x1F) as u8,
            0xFC,
        ]);
        out.extend(frame);
    }
    out
}

fn synchsafe(n: u32) -> [u8; 4] {
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

/// An ID3v2.4 tag with a footer, the kind that can sit at the end of a file.
pub fn id3v2_with_footer(title: &str) -> Vec<u8> {
    let body = id3_text(b"TIT2", title);
    let size = synchsafe(body.len() as u32);
    let mut tag = b"ID3\x04\x00\x10".to_vec();
    tag.extend_from_slice(&size);
    tag.extend(&body);
    tag.extend_from_slice(b"3DI\x04\x00\x10");
    tag.extend_from_slice(&size);
    tag
}

/// An APEv2 tag with a header as well as a footer.
pub fn ape_tag_with_header(items: &[(&str, &str)]) -> Vec<u8> {
    let footer_only = ape_tag(items);
    let split = footer_only.len() - 32;
    let (body, footer) = footer_only.split_at(split);
    let mut header = footer.to_vec();
    let mut footer = footer.to_vec();
    // Flags: has header (bit 31); the header also says it is one (bit 29).
    footer[20..24].copy_from_slice(&(1u32 << 31).to_le_bytes());
    header[20..24].copy_from_slice(&((1u32 << 31) | (1 << 29)).to_le_bytes());
    [header, body.to_vec(), footer].concat()
}

/// A Lyrics3 v2 block (one lyrics field).
pub fn lyrics3v2(lyrics: &str) -> Vec<u8> {
    let mut block = b"LYRICSBEGIN".to_vec();
    block.extend_from_slice(b"LYR");
    block.extend_from_slice(format!("{:05}", lyrics.len()).as_bytes());
    block.extend_from_slice(lyrics.as_bytes());
    let size = block.len();
    block.extend_from_slice(format!("{size:06}").as_bytes());
    block.extend_from_slice(b"LYRICS200");
    block
}

/// Every way tags can sit in front of frames: none, small, resized
/// padding, stacked, and junk after the tag.
pub fn leading_tags() -> Vec<(&'static str, Vec<u8>)> {
    let small = id3v2(&[id3_text(b"TIT2", "Night Drive")], 0);
    let padded = id3v2(
        &[
            id3_text(b"TIT2", "Night Drive (Extended Mix)"),
            id3_text(b"TKEY", "8A"),
        ],
        4096,
    );
    let big = id3v2(&[id3_text(b"COMM", &"x".repeat(70_000))], 100);
    vec![
        ("no tag", Vec::new()),
        ("ID3v2", small.clone()),
        ("ID3v2 with padding", padded.clone()),
        ("a 70 KB ID3v2", big),
        ("two stacked ID3v2", [small.clone(), padded].concat()),
        ("ID3v2 then junk", [small, vec![0u8; 300]].concat()),
    ]
}

/// Every kind of tag that can follow the frames, alone and stacked.
pub fn trailing_tags() -> Vec<(&'static str, Vec<u8>)> {
    let v1 = id3v1_tag("Night Drive");
    let ape = ape_tag(&[("Title", "Night Drive"), ("Key", "8A")]);
    vec![
        ("nothing", Vec::new()),
        ("ID3v1", v1.clone()),
        ("APEv2", ape.clone()),
        ("APEv2 with header", ape_tag_with_header(&[("Title", "x")])),
        ("APEv2 then ID3v1", [ape.clone(), v1.clone()].concat()),
        (
            "Lyrics3 v2 then ID3v1",
            [lyrics3v2("la la"), v1.clone()].concat(),
        ),
        ("ID3v2.4 with footer", id3v2_with_footer("Appended")),
        (
            "ID3v2.4 footer, APEv2, ID3v1",
            [id3v2_with_footer("A"), ape, v1].concat(),
        ),
    ]
}

// ---- RIFF WAVE ------------------------------------------------------------

pub fn wav_fmt() -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
    fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
    fmt.extend_from_slice(&8000u32.to_le_bytes());
    fmt.extend_from_slice(&16000u32.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    fmt
}

/// Samples with an odd length, so the data chunk needs a pad byte.
pub fn samples() -> Vec<u8> {
    pattern(4001, 7)
}

/// A chunk: id, 32-bit size in the given byte order, body, pad byte.
pub fn chunk(id: &[u8; 4], body: &[u8], big_endian: bool) -> Vec<u8> {
    let mut out = id.to_vec();
    let size = body.len() as u32;
    out.extend_from_slice(&if big_endian {
        size.to_be_bytes()
    } else {
        size.to_le_bytes()
    });
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A RIFF (or `RIFX`, `RF64`) WAVE holding `chunks` as given.
pub fn riff(magic: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = [b"WAVE".to_vec(), chunks.concat()].concat();
    let mut out = magic.to_vec();
    let size = body.len() as u32;
    out.extend_from_slice(&if magic == b"RIFX" {
        size.to_be_bytes()
    } else {
        size.to_le_bytes()
    });
    out.extend(body);
    out
}

pub fn wav_plain() -> Vec<u8> {
    riff(
        b"RIFF",
        &[
            chunk(b"fmt ", &wav_fmt(), false),
            chunk(b"data", &samples(), false),
        ],
    )
}

/// A `LIST`/`INFO` chunk, as Windows Explorer and taggers write it.
pub fn list_info(title: &str) -> Vec<u8> {
    let mut body = b"INFO".to_vec();
    let mut name = title.as_bytes().to_vec();
    name.push(0);
    body.extend(chunk(b"INAM", &name, false));
    chunk(b"LIST", &body, false)
}

/// Every way a tagger might lay out the same WAVE audio.
pub fn wav_variants() -> Vec<(&'static str, Vec<u8>)> {
    let (fmt, data) = (
        chunk(b"fmt ", &wav_fmt(), false),
        chunk(b"data", &samples(), false),
    );
    let id3 = id3v2(&[id3_text(b"TIT2", "Night Drive")], 512);
    vec![
        ("plain", wav_plain()),
        (
            "LIST INFO before the data",
            riff(
                b"RIFF",
                &[fmt.clone(), list_info("Night Drive"), data.clone()],
            ),
        ),
        (
            "LIST INFO after the data",
            riff(b"RIFF", &[fmt.clone(), data.clone(), list_info("Title")]),
        ),
        (
            "`id3 ` chunk",
            riff(
                b"RIFF",
                &[fmt.clone(), data.clone(), chunk(b"id3 ", &id3, false)],
            ),
        ),
        (
            "`ID3 ` chunk, resized",
            riff(
                b"RIFF",
                &[
                    fmt.clone(),
                    data.clone(),
                    chunk(b"ID3 ", &[id3.clone(), vec![0; 999]].concat(), false),
                ],
            ),
        ),
        (
            "bext and iXML chunks",
            riff(
                b"RIFF",
                &[
                    chunk(b"bext", &pattern(602, 1), false),
                    fmt.clone(),
                    chunk(b"iXML", b"<BWFXML/>", false),
                    data.clone(),
                ],
            ),
        ),
        (
            "fmt after data",
            riff(b"RIFF", &[data.clone(), list_info("x"), fmt.clone()]),
        ),
        (
            "ID3v2 in front of the RIFF",
            [id3.clone(), wav_plain()].concat(),
        ),
    ]
}

/// An RF64 WAVE whose data size lives in `ds64`.
pub fn rf64() -> Vec<u8> {
    let samples = samples();
    let mut ds64 = Vec::new();
    ds64.extend_from_slice(&0u64.to_le_bytes()); // RIFF size (unused here)
    ds64.extend_from_slice(&(samples.len() as u64).to_le_bytes());
    ds64.extend_from_slice(&0u64.to_le_bytes()); // sample count
    ds64.extend_from_slice(&0u32.to_le_bytes()); // table length
    let mut data = b"data".to_vec();
    data.extend_from_slice(&u32::MAX.to_le_bytes());
    data.extend_from_slice(&samples);
    data.push(0); // pad
    riff(
        b"RF64",
        &[
            chunk(b"ds64", &ds64, false),
            chunk(b"fmt ", &wav_fmt(), false),
            data,
        ],
    )
}

// ---- AIFF -----------------------------------------------------------------

/// COMM for AIFF: mono, frames, 16-bit, 8 kHz (80-bit float).
pub fn aiff_comm() -> Vec<u8> {
    let mut comm = Vec::new();
    comm.extend_from_slice(&1u16.to_be_bytes());
    comm.extend_from_slice(&2000u32.to_be_bytes());
    comm.extend_from_slice(&16u16.to_be_bytes());
    comm.extend_from_slice(&[0x40, 0x0B, 0xFA, 0, 0, 0, 0, 0, 0, 0]);
    comm
}

pub fn ssnd(offset: u32) -> Vec<u8> {
    let mut body = offset.to_be_bytes().to_vec();
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend(vec![0xEE; offset as usize]);
    body.extend(samples());
    body
}

pub fn form(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = [kind.to_vec(), chunks.concat()].concat();
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend(body);
    out
}

pub fn aiff_plain() -> Vec<u8> {
    form(
        b"AIFF",
        &[
            chunk(b"COMM", &aiff_comm(), true),
            chunk(b"SSND", &ssnd(0), true),
        ],
    )
}

/// An uncompressed AIFC of the same samples.
pub fn aifc_none() -> Vec<u8> {
    let mut comm = aiff_comm();
    comm.extend_from_slice(b"NONE");
    comm.extend_from_slice(b"\x0enot compressed\x00");
    form(
        b"AIFC",
        &[
            chunk(b"FVER", &0xA280_5140u32.to_be_bytes(), true),
            chunk(b"COMM", &comm, true),
            chunk(b"SSND", &ssnd(0), true),
        ],
    )
}

/// Every way a tagger (or rekordbox's import) might lay out the same AIFF
/// audio.
pub fn aiff_variants() -> Vec<(&'static str, Vec<u8>)> {
    let (comm, samples) = (
        chunk(b"COMM", &aiff_comm(), true),
        chunk(b"SSND", &ssnd(0), true),
    );
    let id3 = id3v2(
        &[id3_text(b"TIT2", "Night Drive"), id3_text(b"TKEY", "8A")],
        1024,
    );
    vec![
        ("plain", aiff_plain()),
        (
            "ID3 chunk after the samples",
            form(
                b"AIFF",
                &[comm.clone(), samples.clone(), chunk(b"ID3 ", &id3, true)],
            ),
        ),
        (
            "ID3 chunk before the samples, resized",
            form(
                b"AIFF",
                &[
                    comm.clone(),
                    chunk(b"ID3 ", &[id3.clone(), vec![0; 77]].concat(), true),
                    samples.clone(),
                ],
            ),
        ),
        (
            "text chunks",
            form(
                b"AIFF",
                &[
                    chunk(b"NAME", b"Night Drive", true),
                    chunk(b"AUTH", b"Test Artist", true),
                    chunk(b"ANNO", b"8A", true),
                    chunk(b"(c) ", b"2019", true),
                    comm.clone(),
                    samples.clone(),
                ],
            ),
        ),
        (
            "COMM after SSND, with MARK",
            form(
                b"AIFF",
                &[samples.clone(), chunk(b"MARK", &[0, 0], true), comm.clone()],
            ),
        ),
        (
            "SSND with an alignment offset",
            form(b"AIFF", &[comm.clone(), chunk(b"SSND", &ssnd(16), true)]),
        ),
        ("uncompressed AIFC", aifc_none()),
    ]
}

// ---- FLAC -----------------------------------------------------------------

/// The generated FLAC split into STREAMINFO's body and the frames.
pub fn flac_parts() -> (Vec<u8>, Vec<u8>) {
    let file = test_audio::flac();
    assert_eq!(&file[..4], b"fLaC");
    assert_eq!(file[4], 0x80, "STREAMINFO is the only block");
    (file[8..42].to_vec(), file[42..].to_vec())
}

/// `fLaC`, then `blocks` (type, body) with the last one flagged, then the
/// frames.
pub fn flac_with(blocks: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let (info, frames) = flac_parts();
    let mut all = vec![(0u8, info)];
    all.extend(blocks.iter().cloned());
    let mut out = b"fLaC".to_vec();
    for (i, (kind, body)) in all.iter().enumerate() {
        let last = if i + 1 == all.len() { 0x80 } else { 0 };
        out.push(last | kind);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(body);
    }
    out.extend(frames);
    out
}

fn vorbis_comment(fields: &[&str]) -> Vec<u8> {
    let vendor = b"tracklist-pro test";
    let mut body = (vendor.len() as u32).to_le_bytes().to_vec();
    body.extend_from_slice(vendor);
    body.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for field in fields {
        body.extend_from_slice(&(field.len() as u32).to_le_bytes());
        body.extend_from_slice(field.as_bytes());
    }
    body
}

/// Every way a tagger might lay out the same FLAC frames.
pub fn flac_variants() -> Vec<(&'static str, Vec<u8>)> {
    let comments = vorbis_comment(&["TITLE=Night Drive", "INITIALKEY=8A"]);
    let my_tag = vorbis_comment(&["TITLE=Night Drive", "COMMENT=/* My Tag */ Peak"]);
    vec![
        ("STREAMINFO only", flac_with(&[])),
        ("Vorbis comments", flac_with(&[(4, comments.clone())])),
        (
            "Vorbis comments grown, padding shrunk",
            flac_with(&[(4, my_tag), (1, vec![0; 100])]),
        ),
        (
            "padding and a picture",
            flac_with(&[
                (1, vec![0; 8192]),
                (6, pattern(20_000, 3)),
                (4, comments.clone()),
            ]),
        ),
        (
            "seek table and application block",
            flac_with(&[(3, vec![0; 18]), (2, b"test1234".to_vec())]),
        ),
        (
            "ID3v2 in front",
            [
                id3v2(&[id3_text(b"TIT2", "x")], 64),
                flac_with(&[(4, comments.clone())]),
            ]
            .concat(),
        ),
        (
            "APEv2 and ID3v1 at the end",
            [flac_with(&[]), ape_tag(&[("Title", "x")]), id3v1_tag("x")].concat(),
        ),
    ]
}

// ---- MP4 ------------------------------------------------------------------

/// A box: 32-bit size, id, body.
pub fn mp4_box(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = (8 + body.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(id);
    out.extend_from_slice(body);
    out
}

/// The generated M4A's top-level boxes, in order.
pub fn m4a_boxes() -> Vec<([u8; 4], Vec<u8>)> {
    let file = test_audio::m4a();
    let mut boxes = Vec::new();
    let mut at = 0;
    while at < file.len() {
        let size = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
        let id: [u8; 4] = file[at + 4..at + 8].try_into().unwrap();
        boxes.push((id, file[at + 8..at + size].to_vec()));
        at += size;
    }
    boxes
}

/// The generated M4A with the boxes in `order` (ids), plus extra boxes.
pub fn m4a_as(order: &[&[u8; 4]], extra: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let boxes = m4a_boxes();
    let mut out = Vec::new();
    for id in order {
        if let Some((_, body)) = boxes.iter().find(|(b, _)| b == *id) {
            out.extend(mp4_box(id, body));
        } else if let Some((_, body)) = extra.iter().find(|(b, _)| b == *id) {
            out.extend(mp4_box(id, body));
        }
    }
    out
}

/// `moov` with a `udta`/`meta`/`ilst` title added.
pub fn moov_with_ilst(title: &str) -> Vec<u8> {
    let moov = m4a_boxes()
        .into_iter()
        .find(|(id, _)| id == b"moov")
        .unwrap()
        .1;
    let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0]; // type UTF-8, locale
    data.extend_from_slice(title.as_bytes());
    let nam = mp4_box(b"\xA9nam", &mp4_box(b"data", &data));
    let ilst = mp4_box(b"ilst", &nam);
    let mut meta = vec![0u8; 4]; // version and flags
    meta.extend(mp4_box(
        b"hdlr",
        &[vec![0; 8], b"mdirappl".to_vec(), vec![0; 9]].concat(),
    ));
    meta.extend(ilst);
    let udta = mp4_box(b"udta", &mp4_box(b"meta", &meta));
    [moov, udta].concat()
}

/// Every way a tagger might lay out the same MP4 samples.
pub fn mp4_variants() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("as generated", test_audio::m4a()),
        (
            "free padding before mdat",
            m4a_as(
                &[b"ftyp", b"moov", b"free", b"mdat"],
                &[(*b"free", vec![0; 2048])],
            ),
        ),
        (
            "moov moved after mdat",
            m4a_as(&[b"ftyp", b"mdat", b"moov"], &[]),
        ),
    ]
    .into_iter()
    .chain([("metadata atoms in moov", {
        let boxes = m4a_boxes();
        let mut out = Vec::new();
        for (id, body) in boxes {
            if &id == b"moov" {
                out.extend(mp4_box(b"moov", &moov_with_ilst("Night Drive")));
            } else {
                out.extend(mp4_box(&id, &body));
            }
        }
        out
    })])
    .collect()
}

// ---- Ogg ------------------------------------------------------------------

fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in data {
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn ogg_page(
    flags: u8,
    granule: u64,
    serial: u32,
    sequence: u32,
    lacing: &[u8],
    body: &[u8],
) -> Vec<u8> {
    let mut page = b"OggS".to_vec();
    page.push(0);
    page.push(flags);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&sequence.to_le_bytes());
    page.extend_from_slice(&[0; 4]);
    page.push(lacing.len() as u8);
    page.extend_from_slice(lacing);
    page.extend_from_slice(body);
    let crc = ogg_crc(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

/// One logical Ogg stream. Each group of packets starts on a fresh page
/// and spans as many pages as its segments need (255 per page); the page
/// where a group ends carries its granule position.
pub fn ogg_stream(serial: u32, groups: &[(Vec<Vec<u8>>, u64)]) -> Vec<u8> {
    let mut pages: Vec<(bool, u64, Vec<u8>, Vec<u8>)> = Vec::new(); // continued, granule, lacing, body
    for (packets, granule) in groups {
        let mut segments: Vec<(u8, &[u8])> = Vec::new();
        for packet in packets {
            let mut rest: &[u8] = packet;
            loop {
                let take = rest.len().min(255);
                segments.push((take as u8, &rest[..take]));
                rest = &rest[take..];
                if take < 255 {
                    break;
                }
            }
        }
        let mut continued = false;
        for (n, page) in segments.chunks(255).enumerate() {
            let last = (n + 1) * 255 >= segments.len();
            let lacing: Vec<u8> = page.iter().map(|s| s.0).collect();
            let body: Vec<u8> = page.iter().flat_map(|s| s.1.to_vec()).collect();
            let granule = if last { *granule } else { u64::MAX };
            pages.push((continued, granule, lacing.clone(), body));
            continued = lacing.last() == Some(&255);
        }
    }
    let count = pages.len();
    let mut out = Vec::new();
    for (i, (continued, granule, lacing, body)) in pages.into_iter().enumerate() {
        let mut flags = if continued { 0x01 } else { 0 };
        if i == 0 {
            flags |= 0x02;
        }
        if i + 1 == count {
            flags |= 0x04;
        }
        out.extend(ogg_page(flags, granule, serial, i as u32, &lacing, &body));
    }
    out
}

pub fn vorbis_ident() -> Vec<u8> {
    let mut ident = b"\x01vorbis".to_vec();
    ident.extend_from_slice(&0u32.to_le_bytes());
    ident.push(1);
    ident.extend_from_slice(&8000u32.to_le_bytes());
    ident.extend_from_slice(&0i32.to_le_bytes());
    ident.extend_from_slice(&64_000i32.to_le_bytes());
    ident.extend_from_slice(&0i32.to_le_bytes());
    ident.push(0xB8);
    ident.push(1);
    ident
}

pub fn vorbis_comment_packet(fields: &[&str]) -> Vec<u8> {
    let mut packet = b"\x03vorbis".to_vec();
    packet.extend(vorbis_comment(fields));
    packet.push(1);
    packet
}

pub fn vorbis_setup() -> Vec<u8> {
    [b"\x05vorbis".to_vec(), pattern(700, 9)].concat()
}

/// Audio packets of assorted sizes, one over 255 bytes (two segments) and
/// one exactly 255 (a zero-length segment ends it).
pub fn audio_packets() -> Vec<Vec<u8>> {
    [40, 300, 255, 17, 510, 90]
        .iter()
        .enumerate()
        .map(|(i, &n)| pattern(n, i as u8 + 50))
        .collect()
}

pub const SERIAL: u32 = 0x7470_6c74;
pub const FINAL_GRANULE: u64 = 12_345;

/// Ogg Vorbis with the given comment packet, laid out the usual way:
/// ident alone, then comment and setup, then the audio.
pub fn ogg_vorbis_with(comment: Vec<u8>) -> Vec<u8> {
    ogg_stream(
        SERIAL,
        &[
            (vec![vorbis_ident()], 0),
            (vec![comment, vorbis_setup()], 0),
            (audio_packets(), FINAL_GRANULE),
        ],
    )
}

pub fn opus_head() -> Vec<u8> {
    let mut head = b"OpusHead".to_vec();
    head.push(1);
    head.push(1);
    head.extend_from_slice(&312u16.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes());
    head.push(0);
    head
}

pub fn opus_tags(fields: &[&str]) -> Vec<u8> {
    [b"OpusTags".to_vec(), vorbis_comment(fields)].concat()
}

pub fn ogg_opus_with(tags: Vec<u8>) -> Vec<u8> {
    ogg_stream(
        SERIAL,
        &[
            (vec![opus_head()], 0),
            (vec![tags], 0),
            (audio_packets(), FINAL_GRANULE),
        ],
    )
}

/// Every way a tagger might repaginate the same Vorbis stream.
pub fn ogg_vorbis_variants() -> Vec<(&'static str, Vec<u8>)> {
    let big = format!("COMMENT={}", "x".repeat(70_000));
    let huge = format!("METADATA_BLOCK_PICTURE={}", "y".repeat(200_000));
    vec![
        ("no comments", ogg_vorbis_with(vorbis_comment_packet(&[]))),
        (
            "a few comments",
            ogg_vorbis_with(vorbis_comment_packet(&["TITLE=Night Drive", "KEY=8A"])),
        ),
        (
            "comments spanning two pages",
            ogg_vorbis_with(vorbis_comment_packet(&[&big])),
        ),
        (
            "a picture spanning four pages",
            ogg_vorbis_with(vorbis_comment_packet(&[&huge])),
        ),
        (
            "comment and setup on separate pages",
            ogg_stream(
                SERIAL,
                &[
                    (vec![vorbis_ident()], 0),
                    (vec![vorbis_comment_packet(&["TITLE=x"])], 0),
                    (vec![vorbis_setup()], 0),
                    (audio_packets(), FINAL_GRANULE),
                ],
            ),
        ),
        (
            "audio split over more pages",
            ogg_stream(
                SERIAL,
                &[
                    (vec![vorbis_ident()], 0),
                    (vec![vorbis_comment_packet(&[]), vorbis_setup()], 0),
                    (audio_packets()[..3].to_vec(), 100),
                    (audio_packets()[3..].to_vec(), FINAL_GRANULE),
                ],
            ),
        ),
        (
            "ID3v1 appended after the stream",
            [ogg_vorbis_with(vorbis_comment_packet(&[])), id3v1_tag("x")].concat(),
        ),
    ]
}

pub fn ogg_opus_variants() -> Vec<(&'static str, Vec<u8>)> {
    let big = format!("COMMENT={}", "z".repeat(90_000));
    vec![
        ("no tags", ogg_opus_with(opus_tags(&[]))),
        (
            "a few tags",
            ogg_opus_with(opus_tags(&["TITLE=Night Drive"])),
        ),
        ("tags spanning two pages", ogg_opus_with(opus_tags(&[&big]))),
    ]
}

/// Every supported format, untagged: name and bytes.
pub fn every_format() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("mp3", mp3_frames()),
        ("adts", adts_frames()),
        ("wav", wav_plain()),
        ("rf64", rf64()),
        ("rifx", rifx()),
        ("aiff", aiff_plain()),
        ("aifc", aifc_none()),
        ("flac", flac_with(&[])),
        ("mp4", test_audio::m4a()),
        ("ogg_vorbis", ogg_vorbis_with(vorbis_comment_packet(&[]))),
        ("ogg_opus", ogg_opus_with(opus_tags(&[]))),
    ]
}

/// A big-endian RIFX WAVE.
pub fn rifx() -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_be_bytes()); // PCM
    fmt.extend_from_slice(&1u16.to_be_bytes()); // mono
    fmt.extend_from_slice(&8000u32.to_be_bytes());
    fmt.extend_from_slice(&16000u32.to_be_bytes());
    fmt.extend_from_slice(&2u16.to_be_bytes());
    fmt.extend_from_slice(&16u16.to_be_bytes());
    riff(
        b"RIFX",
        &[chunk(b"fmt ", &fmt, true), chunk(b"data", &samples(), true)],
    )
}
