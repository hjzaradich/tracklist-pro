//! M4A holding Apple Lossless (ALAC), with iTunes-style `ilst` tags.
//!
//! AAC would be the more common M4A codec, but no GPL-compatible AAC encoder
//! is small enough to pull in (FDK AAC's license is GPL-incompatible), so
//! M4A fixtures carry ALAC. The ALAC frames use the format's "uncompressed"
//! escape, which every decoder supports. The container is the part scanners
//! care about (`ftyp`, `moov`, sample tables, `mdat`).

use super::bits::BitWriter;
use super::{be16, be32, Field, Tags};
use crate::synth::Pcm;

const FRAME: usize = 4096;

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    be32(&mut out, body.len() as u32 + 8);
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A "full box": version 0 and the given 24-bit flags before the body.
fn full(kind: &[u8; 4], flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(body.len() + 4);
    be32(&mut b, flags & 0x00ff_ffff);
    b.extend_from_slice(body);
    bx(kind, &b)
}

fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

pub fn ftyp() -> Vec<u8> {
    bx(b"ftyp", b"M4A \0\0\0\0M4A mp42isom")
}

/// One ALAC packet: a mono element, uncompressed, then the end tag.
fn alac_packet(samples: &[i16]) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.put(3, 0); // SCE
    w.put(4, 0); // element instance
    w.put(12, 0); // unused
    let partial = samples.len() != FRAME;
    w.put(1, u64::from(partial));
    w.put(2, 0); // no shifted bytes
    w.put(1, 1); // uncompressed
    if partial {
        w.put(32, samples.len() as u64);
    }
    for &s in samples {
        w.put_signed(16, i64::from(s));
    }
    w.put(3, 7); // END
    w.into_bytes()
}

fn magic_cookie(rate: u32) -> Vec<u8> {
    let mut c = Vec::with_capacity(24);
    be32(&mut c, FRAME as u32);
    c.extend_from_slice(&[0, 16, 40, 10, 14, 1]); // version, depth, pb, mb, kb, channels
    be16(&mut c, 255); // max run
    be32(&mut c, 0); // max frame bytes: unknown
    be32(&mut c, 0); // average bit rate: unknown
    be32(&mut c, rate);
    c
}

fn ilst(tags: &Tags) -> Vec<u8> {
    let mut items = Vec::new();
    for (field, value) in tags.fields() {
        let (kind, data): ([u8; 4], Vec<u8>) = match field {
            Field::Title => (*b"\xa9nam", text(value)),
            Field::Artist => (*b"\xa9ART", text(value)),
            Field::Album => (*b"\xa9alb", text(value)),
            Field::Genre => (*b"\xa9gen", text(value)),
            Field::Year => (*b"\xa9day", text(value)),
            Field::Bpm => match value.parse::<u16>() {
                Ok(bpm) => {
                    let mut d = Vec::new();
                    be32(&mut d, 21); // big-endian signed integer
                    be32(&mut d, 0);
                    be16(&mut d, bpm);
                    (*b"tmpo", d)
                }
                Err(_) => continue,
            },
            Field::Track | Field::Key => continue,
        };
        items.push(bx(&kind, &bx(b"data", &data)));
    }
    bx(b"ilst", &items.concat())
}

fn text(value: &str) -> Vec<u8> {
    let mut d = Vec::new();
    be32(&mut d, 1); // UTF-8
    be32(&mut d, 0); // locale
    d.extend_from_slice(value.as_bytes());
    d
}

fn moov(pcm: &Pcm, tags: &Tags, packet_sizes: &[u32], mdat_offset: u32) -> Vec<u8> {
    let rate = pcm.rate;
    let duration = pcm.samples.len() as u32;

    let mut mvhd = Vec::new();
    for v in [0, 0, rate, duration, 0x0001_0000] {
        be32(&mut mvhd, v);
    }
    be16(&mut mvhd, 0x0100);
    mvhd.extend_from_slice(&[0; 10]);
    MATRIX.iter().for_each(|&m| be32(&mut mvhd, m));
    mvhd.extend_from_slice(&[0; 24]);
    be32(&mut mvhd, 2); // next track id

    let mut tkhd = Vec::new();
    for v in [0, 0, 1, 0, duration, 0, 0] {
        be32(&mut tkhd, v);
    }
    be16(&mut tkhd, 0); // layer
    be16(&mut tkhd, 0); // alternate group
    be16(&mut tkhd, 0x0100); // volume
    be16(&mut tkhd, 0);
    MATRIX.iter().for_each(|&m| be32(&mut tkhd, m));
    be32(&mut tkhd, 0); // width
    be32(&mut tkhd, 0); // height

    let mut mdhd = Vec::new();
    for v in [0, 0, rate, duration] {
        be32(&mut mdhd, v);
    }
    be16(&mut mdhd, 0x55c4); // "und"
    be16(&mut mdhd, 0);

    let hdlr = full(
        b"hdlr",
        0,
        b"\0\0\0\0soun\0\0\0\0\0\0\0\0\0\0\0\0SoundHandler\0",
    );

    let mut entry = vec![0; 6];
    be16(&mut entry, 1); // data reference index
    entry.extend_from_slice(&[0; 8]);
    be16(&mut entry, 1); // channels
    be16(&mut entry, 16); // sample size
    be16(&mut entry, 0);
    be16(&mut entry, 0);
    be32(&mut entry, rate << 16);
    entry.extend_from_slice(&full(b"alac", 0, &magic_cookie(rate)));
    let mut stsd = Vec::new();
    be32(&mut stsd, 1);
    stsd.extend_from_slice(&bx(b"alac", &entry));

    let n = packet_sizes.len() as u32;
    let last = pcm.samples.len() - (n as usize - 1) * FRAME;
    let mut stts = Vec::new();
    if last == FRAME {
        be32(&mut stts, 1);
        be32(&mut stts, n);
        be32(&mut stts, FRAME as u32);
    } else {
        be32(&mut stts, if n > 1 { 2 } else { 1 });
        if n > 1 {
            be32(&mut stts, n - 1);
            be32(&mut stts, FRAME as u32);
        }
        be32(&mut stts, 1);
        be32(&mut stts, last as u32);
    }

    let mut stsc = Vec::new();
    for v in [1, 1, n, 1] {
        be32(&mut stsc, v);
    }

    let mut stsz = Vec::new();
    be32(&mut stsz, 0);
    be32(&mut stsz, n);
    packet_sizes.iter().for_each(|&s| be32(&mut stsz, s));

    let mut stco = Vec::new();
    be32(&mut stco, 1);
    be32(&mut stco, mdat_offset);

    let stbl = bx(
        b"stbl",
        &cat(&[
            full(b"stsd", 0, &stsd),
            full(b"stts", 0, &stts),
            full(b"stsc", 0, &stsc),
            full(b"stsz", 0, &stsz),
            full(b"stco", 0, &stco),
        ]),
    );
    let dinf = bx(
        b"dinf",
        &full(
            b"dref",
            0,
            &[&1u32.to_be_bytes()[..], &full(b"url ", 1, &[])].concat(),
        ),
    );
    let minf = bx(b"minf", &cat(&[full(b"smhd", 0, &[0; 4]), dinf, stbl]));
    let mdia = bx(b"mdia", &cat(&[full(b"mdhd", 0, &mdhd), hdlr, minf]));
    let trak = bx(b"trak", &cat(&[full(b"tkhd", 7, &tkhd), mdia]));

    let mut parts = vec![full(b"mvhd", 0, &mvhd), trak];
    if !tags.is_empty() {
        let meta_hdlr = full(b"hdlr", 0, b"\0\0\0\0mdirappl\0\0\0\0\0\0\0\0\0");
        let meta = full(b"meta", 0, &cat(&[meta_hdlr, ilst(tags)]));
        parts.push(bx(b"udta", &meta));
    }
    bx(b"moov", &cat(&parts))
}

/// The audio packets and the `mdat` box holding them.
fn mdat(pcm: &Pcm) -> (Vec<u32>, Vec<u8>) {
    let packets: Vec<Vec<u8>> = pcm.samples.chunks(FRAME).map(alac_packet).collect();
    let sizes = packets.iter().map(|p| p.len() as u32).collect();
    (sizes, bx(b"mdat", &packets.concat()))
}

pub fn encode(pcm: &Pcm, tags: &Tags) -> Vec<u8> {
    assert!(!pcm.samples.is_empty(), "an M4A needs at least one sample");
    let ftyp = ftyp();
    let (sizes, mdat) = mdat(pcm);
    // moov's size doesn't depend on the offset value, so measure it first.
    let probe = moov(pcm, tags, &sizes, 0);
    let offset = (ftyp.len() + probe.len() + 8) as u32;
    cat(&[ftyp, moov(pcm, tags, &sizes, offset), mdat])
}

/// An interrupted download: the file type and the audio, but no `moov`, so
/// there's no index to find or decode the audio with.
pub fn encode_without_moov(pcm: &Pcm) -> Vec<u8> {
    let (_, mdat) = mdat(pcm);
    cat(&[ftyp(), mdat])
}

/// The four-character codes of a file's top-level boxes, for tests.
pub fn top_level_boxes(bytes: &[u8]) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        out.push(bytes[at + 4..at + 8].try_into().unwrap());
        if size < 8 {
            break;
        }
        at += size;
    }
    out
}
