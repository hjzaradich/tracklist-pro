//! AIFF made in code at any bit depth, as bytes or as a source that makes
//! its bytes only as they are read (so a made-up hour of audio needs no
//! memory and no disk).

use std::io::{self, Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

/// The 80-bit float AIFF uses for its sample rate.
fn extended(rate: u32) -> [u8; 10] {
    let exp = 31 - rate.leading_zeros();
    let mantissa = u64::from(rate) << (63 - exp);
    let mut out = [0; 10];
    out[..2].copy_from_slice(&((16383 + exp) as u16).to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// Sample `channel` of frame `frame` at `bits` bits: a made-up value that
/// differs from frame to frame and channel to channel and uses the whole
/// range.
pub fn sample(frame: u64, channel: u16, bits: u16) -> i32 {
    let mixed = frame
        .wrapping_mul(2_654_435_761)
        .wrapping_add(u64::from(channel) * 40_503)
        .wrapping_mul(0x9E37_79B9)
        >> 7;
    let shift = 32 - u32::from(bits);
    (((mixed as u32) << shift) as i32) >> shift
}

/// An AIFF of `frames` frames whose samples are [`sample`], made as it is
/// read.
pub struct Made {
    header: Vec<u8>,
    rate: u32,
    channels: u16,
    bits: u16,
    frames: u64,
    pos: u64,
}

impl Made {
    pub fn new(rate: u32, channels: u16, bits: u16, frames: u64) -> Made {
        Made::claiming(rate, channels, bits, frames, frames)
    }

    /// A file holding `frames` frames whose header says it holds
    /// `claimed`.
    pub fn claiming(rate: u32, channels: u16, bits: u16, frames: u64, claimed: u64) -> Made {
        let data = frames * u64::from(channels) * u64::from(bits / 8);
        let mut comm = Vec::new();
        comm.extend_from_slice(&channels.to_be_bytes());
        comm.extend_from_slice(&(claimed as u32).to_be_bytes());
        comm.extend_from_slice(&bits.to_be_bytes());
        comm.extend_from_slice(&extended(rate));
        let mut header = b"FORM".to_vec();
        header
            .extend_from_slice(&((4 + 8 + comm.len() as u64 + 8 + 8 + data) as u32).to_be_bytes());
        header.extend_from_slice(b"AIFF");
        header.extend_from_slice(b"COMM");
        header.extend_from_slice(&(comm.len() as u32).to_be_bytes());
        header.extend_from_slice(&comm);
        header.extend_from_slice(b"SSND");
        header.extend_from_slice(&((8 + data) as u32).to_be_bytes());
        header.extend_from_slice(&[0; 8]);
        Made {
            header,
            rate,
            channels,
            bits,
            frames,
            pos: 0,
        }
    }

    pub fn len(&self) -> u64 {
        self.header.len() as u64 + self.data_len()
    }

    fn data_len(&self) -> u64 {
        self.frames * u64::from(self.channels) * u64::from(self.bits / 8)
    }

    /// The whole file. Only for short ones.
    pub fn bytes(mut self) -> Vec<u8> {
        let mut out = Vec::new();
        self.read_to_end(&mut out).unwrap();
        out
    }

    /// What decoding this should give as a WAV, byte for byte: the header a
    /// WAV writer would write and the samples little-endian. Only for
    /// short ones.
    pub fn as_wav(&self) -> Vec<u8> {
        let width = usize::from(self.bits / 8);
        let mut out = crate::fingerprint::tests::audio::wav_header(
            self.rate,
            self.channels,
            self.bits,
            self.data_len() as u32,
        );
        for frame in 0..self.frames {
            for channel in 0..self.channels {
                let s = sample(frame, channel, self.bits);
                out.extend_from_slice(&s.to_le_bytes()[..width]);
            }
        }
        out
    }

    /// The `n`th sample byte of the data, big-endian.
    fn data_byte(&self, n: u64) -> u8 {
        let width = u64::from(self.bits / 8);
        let per_frame = width * u64::from(self.channels);
        let (frame, within) = (n / per_frame, n % per_frame);
        let (channel, byte) = (within / width, within % width);
        let s = sample(frame, channel as u16, self.bits);
        s.to_be_bytes()[(4 - width + byte) as usize]
    }
}

impl Read for Made {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let head = self.header.len() as u64;
        let mut n = 0;
        while n < buf.len() && self.pos < self.len() {
            buf[n] = if self.pos < head {
                self.header[self.pos as usize]
            } else {
                self.data_byte(self.pos - head)
            };
            n += 1;
            self.pos += 1;
        }
        Ok(n)
    }
}

impl Seek for Made {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => self.len() as i64 + n,
            SeekFrom::Current(n) => self.pos as i64 + n,
        };
        if target < 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.pos = target as u64;
        Ok(self.pos)
    }
}

impl MediaSource for Made {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len())
    }
}
