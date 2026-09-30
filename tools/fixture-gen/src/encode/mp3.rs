//! MP3 through LAME 3.100, constant bitrate, with the LAME/Info frame so
//! decoders can trim the encoder's delay and padding exactly.

use std::mem::MaybeUninit;

use mp3lame_encoder::{Bitrate, Builder, FlushGap, Mode, MonoPcm, Quality};

use crate::synth::Pcm;

fn bitrate(kbps: u32) -> Bitrate {
    match kbps {
        32 => Bitrate::Kbps32,
        48 => Bitrate::Kbps48,
        64 => Bitrate::Kbps64,
        96 => Bitrate::Kbps96,
        112 => Bitrate::Kbps112,
        128 => Bitrate::Kbps128,
        160 => Bitrate::Kbps160,
        192 => Bitrate::Kbps192,
        256 => Bitrate::Kbps256,
        320 => Bitrate::Kbps320,
        other => panic!("no fixture uses {other} kbps"),
    }
}

/// The audio frames only; tags are added around them by the caller.
pub fn encode(pcm: &Pcm, kbps: u32) -> Vec<u8> {
    let mut b = Builder::new().expect("LAME initialises");
    b.set_num_channels(1).expect("mono");
    b.set_sample_rate(pcm.rate).expect("sample rate");
    b.set_mode(Mode::Mono).expect("mode");
    b.set_brate(bitrate(kbps)).expect("bitrate");
    // Speed matters at 100k files; quality doesn't.
    b.set_quality(Quality::Decent).expect("quality");
    b.set_to_write_vbr_tag(true).expect("info tag");
    let mut enc = b.build().expect("LAME accepts the settings");

    // LAME treats a zero-length buffer as "unbounded", so always reserve.
    let mut out: Vec<u8> =
        Vec::with_capacity(mp3lame_encoder::max_required_buffer_size(pcm.samples.len()));
    enc.encode_to_vec(MonoPcm(&pcm.samples), &mut out)
        .expect("encoding");
    out.reserve(7_200);
    // The plain flush: it pads the last frame with silence, which the Info
    // tag records. The "no gap" flush doesn't, and decodes come out short.
    enc.flush_to_vec::<FlushGap>(&mut out).expect("flushing");

    // LAME reserves the first frame for the Info tag and fills it only on
    // request. Write it in place so delay and padding are recorded.
    let mut tag = vec![MaybeUninit::<u8>::uninit(); 2_880];
    if let Some(n) = enc.lame_tag_encode(&mut tag) {
        let n = n.get();
        let tag: Vec<u8> = tag[..n]
            .iter()
            // SAFETY: LAME wrote the first `n` bytes.
            .map(|b| unsafe { b.assume_init() })
            .collect();
        if n <= out.len() {
            out[..n].copy_from_slice(&tag);
        }
    }
    out
}
