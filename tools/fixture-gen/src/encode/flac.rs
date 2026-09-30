//! FLAC, 16-bit mono, with a Vorbis comment tag.
//!
//! Each block uses the smallest of three subframe kinds: CONSTANT (silence),
//! FIXED order 2 with Rice-coded residuals, or VERBATIM. That's real
//! compression, so FLAC fixtures are smaller than WAV ones, while staying
//! short and simple. The STREAMINFO MD5 is left as zero ("not computed"),
//! which the format allows.

use super::bits::BitWriter;
use super::{Field, Tags};
use crate::synth::Pcm;

const BLOCK: usize = 4096;
const VENDOR: &str = "tracklist-pro fixture-gen";

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
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
    for &b in data {
        crc ^= u16::from(b) << 8;
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

/// FLAC's UTF-8-style coding of the frame number.
fn utf8_number(out: &mut Vec<u8>, n: u32) {
    let c = char::from_u32(n).expect("frame numbers stay below the surrogate range");
    let mut buf = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

fn sample_rate_code(rate: u32) -> u64 {
    match rate {
        8_000 => 0b0100,
        16_000 => 0b0101,
        22_050 => 0b0110,
        24_000 => 0b0111,
        32_000 => 0b1000,
        44_100 => 0b1001,
        48_000 => 0b1010,
        96_000 => 0b1011,
        _ => 0b0000, // from STREAMINFO
    }
}

fn zigzag(r: i32) -> u32 {
    ((r << 1) ^ (r >> 31)) as u32
}

/// Bits needed to Rice-code `values` with parameter `k`.
fn rice_cost(values: &[u32], k: u32) -> u64 {
    values
        .iter()
        .map(|&u| u64::from(u >> k) + 1 + u64::from(k))
        .sum()
}

fn subframe(w: &mut BitWriter, block: &[i16]) {
    // CONSTANT: a block of one repeated value (silence).
    if block.iter().all(|&s| s == block[0]) {
        w.put(8, 0b0000_0000);
        w.put_signed(16, i64::from(block[0]));
        return;
    }

    // FIXED order 2: predict s[n] = 2·s[n-1] − s[n-2].
    let order = 2;
    let fixed = if block.len() > order {
        let residuals: Vec<u32> = (order..block.len())
            .map(|n| {
                let p = 2 * i32::from(block[n - 1]) - i32::from(block[n - 2]);
                zigzag(i32::from(block[n]) - p)
            })
            .collect();
        let (k, bits) = (0..=14)
            .map(|k| (k, rice_cost(&residuals, k)))
            .min_by_key(|&(_, bits)| bits)
            .expect("non-empty range");
        // header 8 + warm-up 32 + method 2 + order 4 + param 4
        Some((k, residuals, 50 + bits))
    } else {
        None
    };

    let verbatim_bits = 8 + 16 * block.len() as u64;
    match fixed {
        Some((k, residuals, bits)) if bits < verbatim_bits => {
            w.put(8, 0b0001_0000 | (order as u64) << 1);
            for &s in &block[..order] {
                w.put_signed(16, i64::from(s));
            }
            w.put(2, 0b00); // Rice, 4-bit parameters
            w.put(4, 0); // partition order 0
            w.put(4, u64::from(k));
            for u in residuals {
                w.unary(u >> k);
                w.put(k, u64::from(u));
            }
        }
        _ => {
            w.put(8, 0b0000_0010);
            for &s in block {
                w.put_signed(16, i64::from(s));
            }
        }
    }
}

fn frame(out: &mut Vec<u8>, number: u32, block: &[i16], rate: u32) {
    let mut w = BitWriter::new();
    w.put(14, 0b11_1111_1111_1110); // sync
    w.put(1, 0); // reserved
    w.put(1, 0); // fixed block size
    let full = block.len() == BLOCK;
    w.put(4, if full { 0b1100 } else { 0b0111 }); // 4096, or 16-bit size at the end
    let rate_code = sample_rate_code(rate);
    w.put(4, rate_code);
    w.put(4, 0b0000); // mono
    w.put(3, 0b100); // 16 bits per sample
    w.put(1, 0); // reserved
    let mut header = w.into_bytes();
    utf8_number(&mut header, number);
    if !full {
        header.extend_from_slice(&(block.len() as u16 - 1).to_be_bytes());
    }
    header.push(crc8(&header));

    let mut w = BitWriter::new();
    subframe(&mut w, block);
    let mut frame = header;
    frame.extend_from_slice(&w.into_bytes());
    frame.extend_from_slice(&crc16(&frame).to_be_bytes());
    out.extend_from_slice(&frame);
}

fn vorbis_comment(tags: &Tags) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    body.extend_from_slice(VENDOR.as_bytes());
    let fields = tags.fields();
    body.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for (field, value) in fields {
        let name = match field {
            Field::Title => "TITLE",
            Field::Artist => "ARTIST",
            Field::Album => "ALBUM",
            Field::Genre => "GENRE",
            Field::Year => "DATE",
            Field::Track => "TRACKNUMBER",
            Field::Bpm => "BPM",
            Field::Key => "INITIALKEY",
        };
        let entry = format!("{name}={value}");
        body.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        body.extend_from_slice(entry.as_bytes());
    }
    body
}

fn block_header(out: &mut Vec<u8>, last: bool, kind: u8, len: usize) {
    out.push(if last { 0x80 } else { 0 } | kind);
    out.extend_from_slice(&(len as u32).to_be_bytes()[1..]);
}

pub fn encode(pcm: &Pcm, tags: &Tags) -> Vec<u8> {
    let mut out = b"fLaC".to_vec();

    let mut info = Vec::with_capacity(34);
    info.extend_from_slice(&(BLOCK as u16).to_be_bytes()); // min block
    info.extend_from_slice(&(BLOCK as u16).to_be_bytes()); // max block
    info.extend_from_slice(&[0; 3]); // min frame size: unknown
    info.extend_from_slice(&[0; 3]); // max frame size: unknown
    let frames = pcm.samples.len() as u64;
    // rate (20 bits) | channels−1 (3) | bps−1 (5) | total samples (36)
    let packed: u64 = (u64::from(pcm.rate) << 44) | (15 << 36) | frames;
    info.extend_from_slice(&packed.to_be_bytes());
    info.extend_from_slice(&[0; 16]); // MD5: not computed

    let comment = (!tags.is_empty()).then(|| vorbis_comment(tags));
    block_header(&mut out, comment.is_none(), 0, info.len());
    out.extend_from_slice(&info);
    if let Some(comment) = comment {
        block_header(&mut out, true, 4, comment.len());
        out.extend_from_slice(&comment);
    }

    for (i, block) in pcm.samples.chunks(BLOCK).enumerate() {
        frame(&mut out, i as u32, block, pcm.rate);
    }
    out
}

/// The STREAMINFO total-samples field of a FLAC file, for tests.
pub fn total_samples(bytes: &[u8]) -> Option<u64> {
    let packed = u64::from_be_bytes(bytes.get(18..26)?.try_into().ok()?);
    Some(packed & ((1 << 36) - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crcs_match_the_flac_reference_values() {
        // "123456789" check values for CRC-8/SMBUS-style (poly 0x07) and
        // CRC-16/BUYPASS (poly 0x8005), the two FLAC uses.
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc16(b"123456789"), 0xFEE8);
    }

    #[test]
    fn a_tone_compresses_below_verbatim_size() {
        let samples = crate::synth::tone(1, 44_100, 500);
        let pcm = Pcm::from_float(44_100, &samples);
        let bytes = encode(&pcm, &Tags::default());
        assert!(bytes.len() < pcm.samples.len() * 2);
        assert_eq!(total_samples(&bytes), Some(pcm.frames()));
    }
}
