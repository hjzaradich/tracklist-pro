//! RIFF WAVE, 16-bit PCM, with an optional `LIST`/`INFO` tag chunk.

use super::{le16, le32, Field, Tags};
use crate::synth::Pcm;

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    le32(out, body.len() as u32);
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

fn info(tags: &Tags) -> Vec<u8> {
    let mut body = b"INFO".to_vec();
    for (field, value) in tags.fields() {
        let id: &[u8; 4] = match field {
            Field::Title => b"INAM",
            Field::Artist => b"IART",
            Field::Album => b"IPRD",
            Field::Genre => b"IGNR",
            Field::Year => b"ICRD",
            Field::Track => b"ITRK",
            // No standard INFO field for these.
            Field::Bpm | Field::Key => continue,
        };
        let mut text = value.as_bytes().to_vec();
        text.push(0);
        chunk(&mut body, id, &text);
    }
    body
}

pub fn encode(pcm: &Pcm, tags: &Tags) -> Vec<u8> {
    let mut fmt = Vec::new();
    le16(&mut fmt, 1); // PCM
    le16(&mut fmt, 1); // mono
    le32(&mut fmt, pcm.rate);
    le32(&mut fmt, pcm.rate * 2); // bytes per second
    le16(&mut fmt, 2); // block align
    le16(&mut fmt, 16); // bits per sample

    let data: Vec<u8> = pcm.samples.iter().flat_map(|s| s.to_le_bytes()).collect();

    let mut body = b"WAVE".to_vec();
    chunk(&mut body, b"fmt ", &fmt);
    chunk(&mut body, b"data", &data);
    if !tags.is_empty() {
        chunk(&mut body, b"LIST", &info(tags));
    }

    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    le32(&mut out, body.len() as u32);
    out.extend_from_slice(&body);
    out
}
