//! AIFF, 16-bit big-endian PCM, with tags in an `ID3 ` chunk as rekordbox
//! and iTunes write them.

use super::id3::{self, Id3Version};
use super::{be16, be32, Tags};
use crate::synth::Pcm;

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    be32(out, body.len() as u32);
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

/// An integer sample rate as an 80-bit IEEE 754 extended float.
pub fn extended(rate: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    if rate == 0 {
        return out;
    }
    let shift = 31 - rate.leading_zeros(); // floor(log2(rate))
    let exponent = 16_383 + shift as u16;
    let mantissa = u64::from(rate) << (63 - shift);
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

pub fn encode(pcm: &Pcm, tags: &Tags) -> Vec<u8> {
    let mut comm = Vec::new();
    be16(&mut comm, 1); // channels
    be32(&mut comm, pcm.samples.len() as u32);
    be16(&mut comm, 16);
    comm.extend_from_slice(&extended(pcm.rate));

    let mut ssnd = Vec::with_capacity(8 + pcm.samples.len() * 2);
    be32(&mut ssnd, 0); // offset
    be32(&mut ssnd, 0); // block size
    for s in &pcm.samples {
        ssnd.extend_from_slice(&s.to_be_bytes());
    }

    let mut body = b"AIFF".to_vec();
    chunk(&mut body, b"COMM", &comm);
    chunk(&mut body, b"SSND", &ssnd);
    let tag = id3::tag(tags, Id3Version::V23);
    if !tag.is_empty() {
        chunk(&mut body, b"ID3 ", &tag);
    }

    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"FORM");
    be32(&mut out, body.len() as u32);
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_rates_encode_as_80_bit_floats() {
        assert_eq!(extended(44_100), [0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended(22_050), [0x40, 0x0d, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);
    }
}
