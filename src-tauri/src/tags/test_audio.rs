//! Tiny audio files built byte by byte at test time, so no audio is committed.
//!
//! WAV, AIFF and FLAC hold a real 440 Hz tone. MP3 holds silent frames, and
//! the Ogg files hold headers plus empty packets: an encoder would be needed
//! for more, and the tag reader never decodes audio. Every file is valid
//! enough for lofty to read its properties and to write tags into it, which
//! is all these tests need.
//!
//! Test setup only: tagging these files with lofty happens in temp dirs.

use std::f64::consts::PI;
use std::path::{Path, PathBuf};

pub const SAMPLE_RATE: u32 = 8000;
const SAMPLES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mp3,
    Flac,
    Wav,
    Aiff,
    M4a,
    Ogg,
    Opus,
}

impl Format {
    pub const ALL: [Format; 7] = [
        Format::Mp3,
        Format::Flac,
        Format::Wav,
        Format::Aiff,
        Format::M4a,
        Format::Ogg,
        Format::Opus,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Format::Mp3 => "mp3",
            Format::Flac => "flac",
            Format::Wav => "wav",
            Format::Aiff => "aiff",
            Format::M4a => "m4a",
            Format::Ogg => "ogg",
            Format::Opus => "opus",
        }
    }

    /// What `TagRead::parsed_as` should say.
    pub fn parsed_as(self) -> &'static str {
        match self {
            Format::Mp3 => "MP3",
            Format::Flac => "FLAC",
            Format::Wav => "WAV",
            Format::Aiff => "AIFF",
            Format::M4a => "MP4",
            Format::Ogg => "Vorbis",
            Format::Opus => "Opus",
        }
    }

    /// The sample rate lofty should report.
    pub fn sample_rate(self) -> u32 {
        match self {
            Format::Mp3 => 44_100,
            Format::Opus => 48_000,
            _ => SAMPLE_RATE,
        }
    }

    pub fn bytes(self) -> Vec<u8> {
        match self {
            Format::Mp3 => mp3(),
            Format::Flac => flac(),
            Format::Wav => wav(),
            Format::Aiff => aiff(),
            Format::M4a => m4a(),
            Format::Ogg => ogg_vorbis(),
            Format::Opus => ogg_opus(),
        }
    }

    /// Writes an untagged file into `dir`.
    pub fn write_to(self, dir: &Path, stem: &str) -> PathBuf {
        let path = dir.join(format!("{stem}.{}", self.extension()));
        std::fs::write(&path, self.bytes()).unwrap();
        path
    }
}

fn tone() -> Vec<i16> {
    (0..SAMPLES)
        .map(|i| {
            let t = i as f64 / SAMPLE_RATE as f64;
            ((2.0 * PI * 440.0 * t).sin() * 8000.0) as i16
        })
        .collect()
}

/// A box / chunk: 4-byte big-endian size (including the header) + id + body.
fn be_box(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&(8 + body.len() as u32).to_be_bytes());
    out.extend_from_slice(id);
    out.extend_from_slice(body);
    out
}

pub fn wav() -> Vec<u8> {
    let data: Vec<u8> = tone().iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
    fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
    fmt.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    fmt.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    fmt.extend_from_slice(&2u16.to_le_bytes()); // block align
    fmt.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    let mut body = b"WAVE".to_vec();
    for (id, chunk) in [(b"fmt ", fmt), (b"data", data)] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        body.extend_from_slice(&chunk);
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// An integer as an 80-bit IEEE extended float, as AIFF stores its rate.
fn extended(value: u32) -> [u8; 10] {
    let v = value as u64;
    let shift = v.leading_zeros();
    let exponent = 16383 + 63 - shift as u16;
    let mantissa = v << shift;
    let mut out = [0u8; 10];
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

pub fn aiff() -> Vec<u8> {
    let mut comm = Vec::new();
    comm.extend_from_slice(&1u16.to_be_bytes()); // mono
    comm.extend_from_slice(&(SAMPLES as u32).to_be_bytes());
    comm.extend_from_slice(&16u16.to_be_bytes());
    comm.extend_from_slice(&extended(SAMPLE_RATE));

    let mut ssnd = vec![0u8; 8]; // offset, block size
    ssnd.extend(tone().iter().flat_map(|s| s.to_be_bytes()));

    let mut body = b"AIFF".to_vec();
    for (id, chunk) in [(b"COMM", comm), (b"SSND", ssnd)] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
        body.extend_from_slice(&chunk);
    }
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// MPEG-1 Layer III, 128 kbps, 44.1 kHz, mono. All-zero side info and main
/// data decode as silence.
pub fn mp3() -> Vec<u8> {
    const FRAME_LEN: usize = 417; // 144 * 128000 / 44100
    let mut frame = vec![0u8; FRAME_LEN];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
    frame.repeat(20)
}

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// FLAC with fixed 256-sample blocks, each a verbatim mono 16-bit subframe.
pub fn flac() -> Vec<u8> {
    const BLOCK: usize = 256;
    let mut out = b"fLaC".to_vec();

    // STREAMINFO, the only (so last) metadata block.
    let mut info = Vec::new();
    info.extend_from_slice(&(BLOCK as u16).to_be_bytes());
    info.extend_from_slice(&(BLOCK as u16).to_be_bytes());
    info.extend_from_slice(&[0; 6]); // min/max frame size: unknown

    // Rate (20 bits), channels - 1 (3 bits: 0, mono), bits per sample - 1
    // (5 bits), total samples (36 bits).
    let packed: u64 = ((SAMPLE_RATE as u64) << 44) | (15 << 36) | SAMPLES as u64;
    info.extend_from_slice(&packed.to_be_bytes());
    info.extend_from_slice(&[0; 16]); // MD5: unknown
    out.push(0x80); // last block, type 0
    out.extend_from_slice(&(info.len() as u32).to_be_bytes()[1..]);
    out.extend_from_slice(&info);

    for (n, block) in tone().chunks(BLOCK).enumerate() {
        let mut frame = vec![
            0xFF,
            0xF8, // sync, fixed block size
            0x60, // block size: 8-bit value at the end; rate: from STREAMINFO
            0x08, // mono, 16-bit
            n as u8,
            (block.len() - 1) as u8,
        ];
        frame.push(crc8(&frame));
        frame.push(0x02); // verbatim subframe
        frame.extend(block.iter().flat_map(|s| s.to_be_bytes()));
        let crc = crc16(&frame);
        frame.extend_from_slice(&crc.to_be_bytes());
        out.extend(frame);
    }
    out
}

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

/// One Ogg page holding whole packets.
fn ogg_page(flags: u8, granule: u64, sequence: u32, packets: &[Vec<u8>]) -> Vec<u8> {
    let mut lacing = Vec::new();
    for packet in packets {
        let mut len = packet.len();
        while len >= 255 {
            lacing.push(255);
            len -= 255;
        }
        lacing.push(len as u8);
    }
    let mut page = b"OggS".to_vec();
    page.push(0); // version
    page.push(flags);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&0x7470_6c74u32.to_le_bytes()); // serial
    page.extend_from_slice(&sequence.to_le_bytes());
    page.extend_from_slice(&[0; 4]); // CRC, filled below
    page.push(lacing.len() as u8);
    page.extend_from_slice(&lacing);
    for packet in packets {
        page.extend_from_slice(packet);
    }
    let crc = ogg_crc(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

const BOS: u8 = 0x02;
const EOS: u8 = 0x04;

fn vorbis_comment_body(vendor: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    body.extend_from_slice(vendor.as_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // no comments
    body
}

pub fn ogg_vorbis() -> Vec<u8> {
    let mut ident = b"\x01vorbis".to_vec();
    ident.extend_from_slice(&0u32.to_le_bytes()); // version
    ident.push(1); // channels
    ident.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    ident.extend_from_slice(&0i32.to_le_bytes()); // bitrate max
    ident.extend_from_slice(&64_000i32.to_le_bytes()); // nominal
    ident.extend_from_slice(&0i32.to_le_bytes()); // min
    ident.push(0xB8); // block sizes 256 / 2048
    ident.push(1); // framing

    let mut comment = b"\x03vorbis".to_vec();
    comment.extend(vorbis_comment_body("tracklist-pro test"));
    comment.push(1); // framing

    // A stand-in setup header: lofty carries it through untouched.
    let setup = b"\x05vorbis\x00\x00\x00\x00\x01".to_vec();

    let mut out = ogg_page(BOS, 0, 0, &[ident]);
    out.extend(ogg_page(0, 0, 1, &[comment, setup]));
    out.extend(ogg_page(EOS, SAMPLES as u64, 2, &[vec![0u8; 16]]));
    out
}

pub fn ogg_opus() -> Vec<u8> {
    ogg_opus_with(1, None)
}

/// An Opus file with the given channel count in its header and, if given, a
/// final granule position instead of the true one. For corrupt-file tests.
pub fn ogg_opus_with(channels: u8, final_granule: Option<u64>) -> Vec<u8> {
    const PRE_SKIP: u16 = 312;
    const FRAMES: u64 = 25; // 20 ms each

    let mut head = b"OpusHead".to_vec();
    head.push(1); // version
    head.push(channels);
    head.extend_from_slice(&PRE_SKIP.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes()); // gain
    head.push(0); // mapping family

    let mut tags = b"OpusTags".to_vec();
    tags.extend(vorbis_comment_body("tracklist-pro test"));

    // TOC-only packets: CELT fullband 20 ms, mono, one empty frame.
    let packets: Vec<Vec<u8>> = (0..FRAMES).map(|_| vec![0xF8]).collect();

    let mut out = ogg_page(BOS, 0, 0, &[head]);
    out.extend(ogg_page(0, 0, 1, &[tags]));
    let granule = final_granule.unwrap_or(PRE_SKIP as u64 + FRAMES * 960);
    out.extend(ogg_page(EOS, granule, 2, &packets));
    out
}

fn full_box(id: &[u8; 4], version_flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = version_flags.to_be_bytes().to_vec();
    b.extend_from_slice(body);
    be_box(id, &b)
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

/// An AAC-LC M4A: the full `moov` tree lofty needs, and an `mdat` of
/// placeholder frames.
pub fn m4a() -> Vec<u8> {
    const FRAMES: u32 = 8;
    const FRAME_BYTES: usize = 6;
    let duration = FRAMES * 1024;

    let ftyp = be_box(b"ftyp", b"M4A \x00\x00\x00\x00M4A mp42isom");

    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&[0; 8]); // creation, modification
    mvhd.extend_from_slice(&SAMPLE_RATE.to_be_bytes()); // timescale
    mvhd.extend_from_slice(&duration.to_be_bytes());
    mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate
    mvhd.extend_from_slice(&0x0100u16.to_be_bytes()); // volume
    mvhd.extend_from_slice(&[0; 10]);
    MATRIX
        .iter()
        .for_each(|m| mvhd.extend_from_slice(&m.to_be_bytes()));
    mvhd.extend_from_slice(&[0; 24]);
    mvhd.extend_from_slice(&2u32.to_be_bytes()); // next track id
    let mvhd = full_box(b"mvhd", 0, &mvhd);

    let mut tkhd = Vec::new();
    tkhd.extend_from_slice(&[0; 8]);
    tkhd.extend_from_slice(&1u32.to_be_bytes()); // track id
    tkhd.extend_from_slice(&[0; 4]);
    tkhd.extend_from_slice(&duration.to_be_bytes());
    tkhd.extend_from_slice(&[0; 8]);
    tkhd.extend_from_slice(&[0; 4]); // layer, alternate group
    tkhd.extend_from_slice(&0x0100u16.to_be_bytes()); // volume
    tkhd.extend_from_slice(&[0; 2]);
    MATRIX
        .iter()
        .for_each(|m| tkhd.extend_from_slice(&m.to_be_bytes()));
    tkhd.extend_from_slice(&[0; 8]); // width, height
    let tkhd = full_box(b"tkhd", 7, &tkhd);

    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&[0; 8]);
    mdhd.extend_from_slice(&SAMPLE_RATE.to_be_bytes());
    mdhd.extend_from_slice(&duration.to_be_bytes());
    mdhd.extend_from_slice(&0x55C4u16.to_be_bytes()); // "und"
    mdhd.extend_from_slice(&[0; 2]);
    let mdhd = full_box(b"mdhd", 0, &mdhd);

    let mut hdlr = vec![0u8; 4];
    hdlr.extend_from_slice(b"soun");
    hdlr.extend_from_slice(&[0; 12]);
    hdlr.push(0); // empty name
    let hdlr = full_box(b"hdlr", 0, &hdlr);

    let smhd = full_box(b"smhd", 0, &[0; 4]);
    let url = full_box(b"url ", 1, &[]);
    let mut dref = 1u32.to_be_bytes().to_vec();
    dref.extend(url);
    let dinf = be_box(b"dinf", &full_box(b"dref", 0, &dref));

    // esds: ES descriptor > decoder config (AAC, audio) > AudioSpecificConfig
    // (AAC-LC, 8 kHz, mono) + SL config.
    let asc = [0x15, 0x88];
    let mut dcd = vec![0x40, 0x15, 0, 0, 0];
    dcd.extend_from_slice(&64_000u32.to_be_bytes());
    dcd.extend_from_slice(&64_000u32.to_be_bytes());
    dcd.extend_from_slice(&[0x05, asc.len() as u8]);
    dcd.extend_from_slice(&asc);
    let mut es = vec![0, 1, 0]; // ES id, flags
    es.extend_from_slice(&[0x04, dcd.len() as u8]);
    es.extend(dcd);
    es.extend_from_slice(&[0x06, 1, 0x02]);
    let mut esds = vec![0x03, es.len() as u8];
    esds.extend(es);
    let esds = full_box(b"esds", 0, &esds);

    let mut mp4a = vec![0u8; 6];
    mp4a.extend_from_slice(&1u16.to_be_bytes()); // data reference index
    mp4a.extend_from_slice(&[0; 8]);
    mp4a.extend_from_slice(&1u16.to_be_bytes()); // channels
    mp4a.extend_from_slice(&16u16.to_be_bytes()); // sample size
    mp4a.extend_from_slice(&[0; 4]);
    mp4a.extend_from_slice(&(SAMPLE_RATE << 16).to_be_bytes());
    mp4a.extend(esds);
    let mut stsd = 1u32.to_be_bytes().to_vec();
    stsd.extend(be_box(b"mp4a", &mp4a));
    let stsd = full_box(b"stsd", 0, &stsd);

    let mut stts = 1u32.to_be_bytes().to_vec();
    stts.extend_from_slice(&FRAMES.to_be_bytes());
    stts.extend_from_slice(&1024u32.to_be_bytes());
    let stts = full_box(b"stts", 0, &stts);

    let mut stsc = 1u32.to_be_bytes().to_vec();
    stsc.extend_from_slice(&1u32.to_be_bytes());
    stsc.extend_from_slice(&FRAMES.to_be_bytes());
    stsc.extend_from_slice(&1u32.to_be_bytes());
    let stsc = full_box(b"stsc", 0, &stsc);

    let mut stsz = (FRAME_BYTES as u32).to_be_bytes().to_vec();
    stsz.extend_from_slice(&FRAMES.to_be_bytes());
    let stsz = full_box(b"stsz", 0, &stsz);

    // The chunk offset depends on the moov size, which includes stco
    // itself; its size doesn't depend on the value, so build twice.
    let build = |mdat_offset: u32| {
        let mut stco = 1u32.to_be_bytes().to_vec();
        stco.extend_from_slice(&mdat_offset.to_be_bytes());
        let stco = full_box(b"stco", 0, &stco);
        let stbl = be_box(b"stbl", &[&stsd[..], &stts, &stsc, &stsz, &stco].concat());
        let minf = be_box(b"minf", &[&smhd[..], &dinf, &stbl].concat());
        let mdia = be_box(b"mdia", &[&mdhd[..], &hdlr, &minf].concat());
        let trak = be_box(b"trak", &[&tkhd[..], &mdia].concat());
        be_box(b"moov", &[&mvhd[..], &trak].concat())
    };
    let moov_len = build(0).len();
    let mdat_offset = (ftyp.len() + moov_len + 8) as u32;
    let moov = build(mdat_offset);

    // Silent AAC-LC frames would need an encoder; the reader never decodes.
    let mdat = be_box(b"mdat", &vec![0u8; FRAME_BYTES * FRAMES as usize]);
    [ftyp, moov, mdat].concat()
}

// Hand-made ID3v2.4 tags, for the broken-tag tests.

fn synchsafe(n: u32) -> [u8; 4] {
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

/// An ID3v2.4 frame with the given raw body.
pub fn id3_frame(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut f = id.to_vec();
    f.extend_from_slice(&synchsafe(body.len() as u32));
    f.extend_from_slice(&[0, 0]); // flags
    f.extend_from_slice(body);
    f
}

/// A UTF-8 text frame.
pub fn id3_text(id: &[u8; 4], text: &str) -> Vec<u8> {
    let mut body = vec![3u8];
    body.extend_from_slice(text.as_bytes());
    id3_frame(id, &body)
}

/// An ID3v2.4 tag around the given frames, plus `padding` zero bytes.
pub fn id3v2(frames: &[Vec<u8>], padding: usize) -> Vec<u8> {
    let body: Vec<u8> = frames.concat();
    let mut tag = b"ID3\x04\x00\x00".to_vec();
    tag.extend_from_slice(&synchsafe((body.len() + padding) as u32));
    tag.extend(body);
    tag.extend(std::iter::repeat_n(0u8, padding));
    tag
}

/// An APEv2 tag (items + footer, no header) with the given text items.
pub fn ape_tag(items: &[(&str, &str)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (key, value) in items {
        body.extend_from_slice(&(value.len() as u32).to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // flags: UTF-8 text
        body.extend_from_slice(key.as_bytes());
        body.push(0);
        body.extend_from_slice(value.as_bytes());
    }
    let mut footer = b"APETAGEX".to_vec();
    footer.extend_from_slice(&2000u32.to_le_bytes());
    footer.extend_from_slice(&(body.len() as u32 + 32).to_le_bytes());
    footer.extend_from_slice(&(items.len() as u32).to_le_bytes());
    footer.extend_from_slice(&0u32.to_le_bytes()); // footer, no header
    footer.extend_from_slice(&[0; 8]);
    [body, footer].concat()
}

/// A 128-byte ID3v1 tag with a title, and no genre.
pub fn id3v1_tag(title: &str) -> Vec<u8> {
    let field = |s: &str, n: usize| {
        let mut v = s.as_bytes().to_vec();
        v.resize(n, 0);
        v
    };
    let mut tag = b"TAG".to_vec();
    tag.extend(field(title, 30));
    tag.extend(field("", 30)); // artist
    tag.extend(field("", 30)); // album
    tag.extend(field("", 4)); // year
    tag.extend(field("", 30)); // comment
    tag.push(255); // no genre
    tag
}

/// A tiny deterministic random number generator for fuzzing (xorshift).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
