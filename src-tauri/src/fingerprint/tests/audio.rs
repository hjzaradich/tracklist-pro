//! Test audio made in code (no commercial tracks, nothing
//! committed): short synthetic songs, and the same song written as WAV,
//! AIFF, FLAC, M4A (ALAC) and MP3 (LAME) at several rates and bitrates.
//!
//! A song is a seeded chord progression, a bass line and off-beat noise
//! hits, so its pitch content changes every beat, which is what chromaprint
//! keys on. Different seeds are different songs. Rendering is a function of
//! time, so the same song at 22.05 kHz and at 48 kHz is the same audio.

use std::f64::consts::TAU;

/// Interleaved 16-bit PCM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    /// Frames `from..to` (in seconds), e.g. a radio edit cut from a mix.
    pub fn slice(&self, from: f64, to: f64) -> Pcm {
        let ch = usize::from(self.channels);
        let at = |s: f64| ((s * f64::from(self.rate)) as usize).min(self.frames()) * ch;
        Pcm {
            rate: self.rate,
            channels: self.channels,
            samples: self.samples[at(from)..at(to)].to_vec(),
        }
    }
}

/// splitmix64: a seeded, platform-independent random source.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// -1 to 1.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// One beat of a song: three chord notes and a bass note, in Hz.
struct Beat {
    chord: [f64; 3],
    bass: f64,
}

/// Beats per second (120 BPM).
const BEATS_PER_SECOND: f64 = 2.0;

fn beats(seed: u64, count: usize) -> Vec<Beat> {
    let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0xA5A5);
    // A new chord and bass note every beat, so no two seconds of a song
    // sound alike.
    (0..count)
        .map(|_| {
            let root = 48 + rng.below(24) as u8;
            let third = if rng.below(2) == 0 { 3 } else { 4 };
            Beat {
                chord: [hz(root), hz(root + third), hz(root + 7)],
                bass: hz(28 + rng.below(12) as u8),
            }
        })
        .collect()
}

fn hz(midi: u8) -> f64 {
    440.0 * 2f64.powf((f64::from(midi) - 69.0) / 12.0)
}

/// Song `seed`, `seconds` long, at `rate`, mono, from -1 to 1.
pub fn song(seed: u64, seconds: f64, rate: u32) -> Vec<f64> {
    let frames = (seconds * f64::from(rate)) as usize;
    let beats = beats(seed, (seconds * BEATS_PER_SECOND) as usize + 1);
    let mut noise = Rng(seed ^ 0x5EED);
    (0..frames)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            let n = (t * BEATS_PER_SECOND) as usize;
            let in_beat = t * BEATS_PER_SECOND - n as f64; // 0..1
            let beat = &beats[n];
            let envelope = (in_beat * 40.0).min(1.0) * (1.0 - 0.6 * in_beat);
            let mut x = 0.0;
            for &f in &beat.chord {
                x += 0.18 * (TAU * f * t).sin() + 0.06 * (TAU * 2.0 * f * t).sin();
            }
            x += 0.25 * (TAU * beat.bass * t).sin();
            x *= envelope;
            // A short noise hit on the off-beat.
            if (0.5..0.55).contains(&in_beat) {
                x += 0.15 * noise.unit();
            }
            x.clamp(-1.0, 1.0)
        })
        .collect()
}

/// Song `seed` as 16-bit PCM with `channels` channels (the right channel a
/// little quieter, as a mix might be).
pub fn pcm(seed: u64, seconds: f64, rate: u32, channels: u16) -> Pcm {
    let mono = song(seed, seconds, rate);
    let mut samples = Vec::with_capacity(mono.len() * usize::from(channels));
    for x in mono {
        for c in 0..channels {
            let gain = if c == 0 { 0.9 } else { 0.8 };
            samples.push((x * gain * 32_767.0).round() as i16);
        }
    }
    Pcm {
        rate,
        channels,
        samples,
    }
}

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// A WAV header for `data_bytes` of PCM.
pub fn wav_header(rate: u32, channels: u16, bits: u16, data_bytes: u32) -> Vec<u8> {
    let block = channels * (bits / 8);
    let mut out = b"RIFF".to_vec();
    le32(&mut out, 36 + data_bytes);
    out.extend_from_slice(b"WAVEfmt ");
    le32(&mut out, 16);
    le16(&mut out, 1); // PCM
    le16(&mut out, channels);
    le32(&mut out, rate);
    le32(&mut out, rate * u32::from(block));
    le16(&mut out, block);
    le16(&mut out, bits);
    out.extend_from_slice(b"data");
    le32(&mut out, data_bytes);
    out
}

pub fn wav(pcm: &Pcm) -> Vec<u8> {
    let mut out = wav_header(pcm.rate, pcm.channels, 16, (pcm.samples.len() * 2) as u32);
    out.extend(pcm.samples.iter().flat_map(|s| s.to_le_bytes()));
    out
}

/// The 80-bit float AIFF uses for its sample rate.
fn extended(rate: u32) -> [u8; 10] {
    let exp = 31 - rate.leading_zeros();
    let mantissa = u64::from(rate) << (63 - exp);
    let mut out = [0; 10];
    out[..2].copy_from_slice(&((16383 + exp) as u16).to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

pub fn aiff(pcm: &Pcm) -> Vec<u8> {
    let mut comm = Vec::new();
    be16(&mut comm, pcm.channels);
    be32(&mut comm, pcm.frames() as u32);
    be16(&mut comm, 16);
    comm.extend_from_slice(&extended(pcm.rate));
    let mut ssnd = vec![0; 8]; // offset, block size
    ssnd.extend(pcm.samples.iter().flat_map(|s| s.to_be_bytes()));
    let mut body = b"AIFF".to_vec();
    for (id, chunk) in [(b"COMM", comm), (b"SSND", ssnd)] {
        body.extend_from_slice(id);
        be32(&mut body, chunk.len() as u32);
        body.extend_from_slice(&chunk);
    }
    let mut out = b"FORM".to_vec();
    be32(&mut out, body.len() as u32);
    out.extend_from_slice(&body);
    out
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
        crc ^= u16::from(byte) << 8;
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

/// UTF-8-style frame number, as FLAC codes it.
fn flac_number(n: u32, out: &mut Vec<u8>) {
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x800 {
        out.push(0xC0 | (n >> 6) as u8);
        out.push(0x80 | (n & 0x3F) as u8);
    } else {
        out.push(0xE0 | (n >> 12) as u8);
        out.push(0x80 | ((n >> 6) & 0x3F) as u8);
        out.push(0x80 | (n & 0x3F) as u8);
    }
}

/// FLAC with fixed 4096-frame blocks of verbatim 16-bit subframes.
pub fn flac(pcm: &Pcm) -> Vec<u8> {
    const BLOCK: usize = 4096;
    let ch = usize::from(pcm.channels);
    let mut out = b"fLaC".to_vec();
    let mut info = Vec::new();
    be16(&mut info, BLOCK as u16);
    be16(&mut info, BLOCK as u16);
    info.extend_from_slice(&[0; 6]); // frame sizes unknown
    let packed: u64 =
        (u64::from(pcm.rate) << 44) | ((ch as u64 - 1) << 41) | (15 << 36) | pcm.frames() as u64;
    info.extend_from_slice(&packed.to_be_bytes());
    info.extend_from_slice(&[0; 16]); // no MD5
    out.push(0x80); // last metadata block: STREAMINFO
    out.extend_from_slice(&(info.len() as u32).to_be_bytes()[1..]);
    out.extend_from_slice(&info);

    for (n, block) in pcm.samples.chunks(BLOCK * ch).enumerate() {
        let frames = block.len() / ch;
        // Block size: 16-bit value at the end; rate: from STREAMINFO;
        // channels independent; 16 bits per sample.
        let mut frame = vec![0xFF, 0xF8, 0x70, (((ch - 1) as u8) << 4) | 0x08];
        flac_number(n as u32, &mut frame);
        be16(&mut frame, (frames - 1) as u16);
        frame.push(crc8(&frame));
        for c in 0..ch {
            frame.push(0x02); // verbatim subframe
            for f in 0..frames {
                be16(&mut frame, block[f * ch + c] as u16);
            }
        }
        let crc = crc16(&frame);
        be16(&mut frame, crc);
        out.extend(frame);
    }
    out
}

/// An M4A box.
fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    be32(&mut out, body.len() as u32 + 8);
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A "full box": version 0 and `flags` before the body.
fn full(kind: &[u8; 4], flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = (flags & 0x00FF_FFFF).to_be_bytes().to_vec();
    b.extend_from_slice(body);
    bx(kind, &b)
}

const ALAC_FRAME: usize = 4096;

/// Big-endian bits, for ALAC packets.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, n: u32, value: u64) {
        self.acc = (self.acc << n) | (value & ((1u64 << n) - 1));
        self.n += n;
        while self.n >= 8 {
            self.n -= 8;
            self.bytes.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.put(8 - self.n, 0);
        }
        self.bytes
    }
}

/// One mono ALAC packet using the format's uncompressed escape.
fn alac_packet(samples: &[i16]) -> Vec<u8> {
    let mut w = Bits::default();
    w.put(3, 0); // single channel element
    w.put(4, 0);
    w.put(12, 0);
    let partial = samples.len() != ALAC_FRAME;
    w.put(1, u64::from(partial));
    w.put(2, 0);
    w.put(1, 1); // uncompressed
    if partial {
        w.put(32, samples.len() as u64);
    }
    for &s in samples {
        w.put(16, s as u16 as u64);
    }
    w.put(3, 7); // end
    w.finish()
}

/// Mono Apple Lossless in an M4A. (No GPL-compatible AAC encoder is small
/// enough to use here; the M4A container is what matters to the demuxer.)
pub fn m4a_alac(pcm: &Pcm) -> Vec<u8> {
    assert_eq!(pcm.channels, 1, "the ALAC writer is mono only");
    let rate = pcm.rate;
    let duration = pcm.samples.len() as u32;
    let packets: Vec<Vec<u8>> = pcm.samples.chunks(ALAC_FRAME).map(alac_packet).collect();
    let sizes: Vec<u32> = packets.iter().map(|p| p.len() as u32).collect();
    let mdat = bx(b"mdat", &packets.concat());
    let ftyp = bx(b"ftyp", b"M4A \0\0\0\0M4A mp42isom");

    let moov = |mdat_offset: u32| {
        let matrix = [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
        let mut mvhd = Vec::new();
        for v in [0, 0, rate, duration, 0x0001_0000] {
            be32(&mut mvhd, v);
        }
        be16(&mut mvhd, 0x0100);
        mvhd.extend_from_slice(&[0; 10]);
        matrix.iter().for_each(|&m| be32(&mut mvhd, m));
        mvhd.extend_from_slice(&[0; 24]);
        be32(&mut mvhd, 2);

        let mut tkhd = Vec::new();
        for v in [0, 0, 1, 0, duration, 0, 0] {
            be32(&mut tkhd, v);
        }
        for v in [0, 0, 0x0100, 0] {
            be16(&mut tkhd, v);
        }
        matrix.iter().for_each(|&m| be32(&mut tkhd, m));
        be32(&mut tkhd, 0);
        be32(&mut tkhd, 0);

        let mut mdhd = Vec::new();
        for v in [0, 0, rate, duration] {
            be32(&mut mdhd, v);
        }
        be16(&mut mdhd, 0x55C4);
        be16(&mut mdhd, 0);

        let mut cookie = Vec::new();
        be32(&mut cookie, ALAC_FRAME as u32);
        cookie.extend_from_slice(&[0, 16, 40, 10, 14, 1]);
        be16(&mut cookie, 255);
        be32(&mut cookie, 0);
        be32(&mut cookie, 0);
        be32(&mut cookie, rate);
        let mut entry = vec![0; 6];
        be16(&mut entry, 1);
        entry.extend_from_slice(&[0; 8]);
        be16(&mut entry, 1); // channels
        be16(&mut entry, 16);
        be16(&mut entry, 0);
        be16(&mut entry, 0);
        be32(&mut entry, rate << 16);
        entry.extend_from_slice(&full(b"alac", 0, &cookie));
        let mut stsd = Vec::new();
        be32(&mut stsd, 1);
        stsd.extend_from_slice(&bx(b"alac", &entry));

        let n = sizes.len() as u32;
        let last = pcm.samples.len() - (n as usize - 1) * ALAC_FRAME;
        let mut stts = Vec::new();
        if last == ALAC_FRAME || n == 1 {
            for v in [1, n, last as u32] {
                be32(&mut stts, v);
            }
        } else {
            for v in [2, n - 1, ALAC_FRAME as u32, 1, last as u32] {
                be32(&mut stts, v);
            }
        }
        let mut stsc = Vec::new();
        for v in [1, 1, n, 1] {
            be32(&mut stsc, v);
        }
        let mut stsz = Vec::new();
        be32(&mut stsz, 0);
        be32(&mut stsz, n);
        sizes.iter().for_each(|&s| be32(&mut stsz, s));
        let mut stco = Vec::new();
        be32(&mut stco, 1);
        be32(&mut stco, mdat_offset);

        let stbl = bx(
            b"stbl",
            &[
                full(b"stsd", 0, &stsd),
                full(b"stts", 0, &stts),
                full(b"stsc", 0, &stsc),
                full(b"stsz", 0, &stsz),
                full(b"stco", 0, &stco),
            ]
            .concat(),
        );
        let dref = [&1u32.to_be_bytes()[..], &full(b"url ", 1, &[])].concat();
        let dinf = bx(b"dinf", &full(b"dref", 0, &dref));
        let minf = bx(b"minf", &[full(b"smhd", 0, &[0; 4]), dinf, stbl].concat());
        let hdlr = full(b"hdlr", 0, b"\0\0\0\0soun\0\0\0\0\0\0\0\0\0\0\0\0Sound\0");
        let mdia = bx(b"mdia", &[full(b"mdhd", 0, &mdhd), hdlr, minf].concat());
        let trak = bx(b"trak", &[full(b"tkhd", 7, &tkhd), mdia].concat());
        bx(b"moov", &[full(b"mvhd", 0, &mvhd), trak].concat())
    };
    // moov's size doesn't depend on the offset, so measure it first.
    let offset = (ftyp.len() + moov(0).len() + 8) as u32;
    [ftyp, moov(offset), mdat].concat()
}

/// MP3 through LAME 3.100 at `kbps`, constant bitrate, mono or stereo.
/// Windows only, like the LAME dev-dependency.
#[cfg(windows)]
pub fn mp3(pcm: &Pcm, kbps: u32) -> Vec<u8> {
    use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, InterleavedPcm, Mode, MonoPcm, Quality};
    let bitrate = match kbps {
        96 => Bitrate::Kbps96,
        128 => Bitrate::Kbps128,
        192 => Bitrate::Kbps192,
        320 => Bitrate::Kbps320,
        other => panic!("no test uses {other} kbps"),
    };
    let mut b = Builder::new().expect("LAME starts");
    b.set_num_channels(pcm.channels as u8).expect("channels");
    b.set_sample_rate(pcm.rate).expect("rate");
    b.set_mode(if pcm.channels == 1 {
        Mode::Mono
    } else {
        Mode::JointStereo
    })
    .expect("mode");
    b.set_brate(bitrate).expect("bitrate");
    b.set_quality(Quality::Decent).expect("quality");
    let mut enc = b.build().expect("LAME takes the settings");
    let mut out = Vec::with_capacity(mp3lame_encoder::max_required_buffer_size(pcm.frames()));
    if pcm.channels == 1 {
        enc.encode_to_vec(MonoPcm(&pcm.samples), &mut out)
    } else {
        enc.encode_to_vec(InterleavedPcm(&pcm.samples), &mut out)
    }
    .expect("encoding");
    out.reserve(7200);
    enc.flush_to_vec::<FlushNoGap>(&mut out).expect("flushing");
    out
}
