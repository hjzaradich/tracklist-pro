//! AIFF and ALAC for preview (1cA-8): a WAV that exists only as a computed
//! header and a decoder, never as bytes in memory or on disk.
//!
//! The webview can't play AIFF or ALAC, so they are answered as a WAV file
//! whose length is known from the container: a 44-byte header, then the
//! samples as 16-bit (source up to 16 bits) or 24-bit little-endian PCM,
//! one or two channels. A media element asks for byte ranges of it. A range
//! of the data is turned into a range of frames, the container is seeked to
//! the start of it (both containers can), and only the frames asked for are
//! decoded and written out. So what is held at once is one response (at most
//! [`super::MAX_CHUNK`] bytes) plus one decoded packet, however long the
//! track is, and nothing is written anywhere.

use std::io::ErrorKind;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{TimeBase, Timestamp};

use super::PreviewRefusal;

/// The size of the computed WAV header.
pub(super) const HEADER_LEN: u64 = 44;

/// Packets in a row that may fail to decode before the rest is left as
/// silence (the same allowance the fingerprint decoder gives).
const MAX_ERRORS_IN_A_ROW: u32 = 64;

/// What the computed WAV looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Layout {
    pub rate: u32,
    /// 1 or 2.
    pub channels: u16,
    /// Bytes per sample: 2 (16-bit) or 3 (24-bit).
    pub bytes: u16,
    pub frames: u64,
}

impl Layout {
    pub fn frame_bytes(&self) -> u64 {
        u64::from(self.channels) * u64::from(self.bytes)
    }

    fn data_bytes(&self) -> u64 {
        self.frames * self.frame_bytes()
    }

    /// Whether the data fits a WAV's 32-bit sizes (under 4 GiB).
    pub fn fits(&self) -> bool {
        self.data_bytes() <= u64::from(u32::MAX) - 36
    }

    /// The whole WAV file's length: what `Content-Range` calls the total.
    pub fn total(&self) -> u64 {
        HEADER_LEN + self.data_bytes()
    }

    /// The header: RIFF, `fmt ` (plain PCM) and the start of `data`.
    pub fn header(&self) -> [u8; HEADER_LEN as usize] {
        let data = self.data_bytes() as u32;
        let frame = self.frame_bytes() as u32;
        let mut h = [0u8; HEADER_LEN as usize];
        h[0..4].copy_from_slice(b"RIFF");
        h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
        h[8..16].copy_from_slice(b"WAVEfmt ");
        h[16..20].copy_from_slice(&16u32.to_le_bytes());
        h[20..22].copy_from_slice(&1u16.to_le_bytes());
        h[22..24].copy_from_slice(&self.channels.to_le_bytes());
        h[24..28].copy_from_slice(&self.rate.to_le_bytes());
        h[28..32].copy_from_slice(&(self.rate * frame).to_le_bytes());
        h[32..34].copy_from_slice(&(frame as u16).to_le_bytes());
        h[34..36].copy_from_slice(&(self.bytes * 8).to_le_bytes());
        h[36..40].copy_from_slice(b"data");
        h[40..44].copy_from_slice(&data.to_le_bytes());
        h
    }
}

/// A container opened for a ranged read of its audio as WAV.
pub(super) struct Decoded {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    pub layout: Layout,
    /// Nothing has been read yet, so the container is at its start.
    fresh: bool,
    /// Frames decoded so far by [`Decoded::read`], seeking aside; tests
    /// assert on it to show a long file isn't decoded from the start.
    pub decoded_frames: u64,
}

/// The bit depth in an ALAC sample entry's magic cookie, which symphonia
/// keeps in `extra_data` and doesn't report as a parameter: optionally an
/// `frma` and an `alac` atom, then frame length (4 bytes), compatible
/// version (1) and the bit depth (1).
fn alac_bit_depth(extra: &[u8]) -> Option<u32> {
    let mut cookie = extra;
    for atom in [b"frma", b"alac"] {
        if cookie.len() >= 12 && cookie[4..8] == atom[..] {
            cookie = &cookie[12..];
        }
    }
    cookie.get(5).map(|&bits| u32::from(bits))
}

fn io_refusal(e: &std::io::Error) -> PreviewRefusal {
    match e.kind() {
        // Damage that runs the data out early is a file that can't be
        // decoded; anything else is the OS failing.
        ErrorKind::UnexpectedEof | ErrorKind::InvalidData => PreviewRefusal::NotDecodable,
        ErrorKind::NotFound => PreviewRefusal::Missing,
        _ => PreviewRefusal::Unreadable,
    }
}

fn refusal(e: &SymphoniaError) -> PreviewRefusal {
    match e {
        SymphoniaError::IoError(io) => io_refusal(io),
        _ => PreviewRefusal::NotDecodable,
    }
}

impl Decoded {
    /// Probes `source` (the format comes from the bytes, never the
    /// extension) and prepares its first audio track. Refused as
    /// [`PreviewRefusal::NotDecodable`] when symphonia can't read or decode it,
    /// when its length isn't in its header, when it has more than two
    /// channels, or when the WAV would pass 4 GiB.
    pub fn from_source(source: Box<dyn MediaSource>) -> Result<Decoded, PreviewRefusal> {
        let stream = MediaSourceStream::new(source, Default::default());
        let format = symphonia::default::get_probe()
            .probe(
                &Hint::new(),
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| refusal(&e))?;
        let track = format
            .first_track(TrackType::Audio)
            .ok_or(PreviewRefusal::NotDecodable)?;
        let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
            return Err(PreviewRefusal::NotDecodable);
        };
        let frames = track
            .num_frames
            .filter(|&n| n > 0)
            .ok_or(PreviewRefusal::NotDecodable)?;
        let rate = params
            .sample_rate
            .filter(|&r| r > 0)
            .ok_or(PreviewRefusal::NotDecodable)?;
        let channels = params
            .channels
            .as_ref()
            .map(|c| c.count())
            .filter(|&c| (1..=2).contains(&c))
            .ok_or(PreviewRefusal::NotDecodable)? as u16;
        // 24 bits when the file doesn't say: nothing is lost that way.
        let bits = params
            .bits_per_sample
            .or(params.bits_per_coded_sample)
            .or_else(|| params.extra_data.as_deref().and_then(alac_bit_depth))
            .unwrap_or(24);
        let layout = Layout {
            rate,
            channels,
            bytes: if bits <= 16 { 2 } else { 3 },
            frames,
        };
        // The WAV header's sizes are 32-bit.
        if !layout.fits() {
            return Err(PreviewRefusal::NotDecodable);
        }
        let (track_id, time_base) = (track.id, track.time_base);
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|_| PreviewRefusal::NotDecodable)?;
        Ok(Decoded {
            format,
            decoder,
            track_id,
            time_base,
            layout,
            fresh: true,
            decoded_frames: 0,
        })
    }

    /// Bytes `start..=end` of the WAV. `end` is inside the file.
    pub fn read(&mut self, start: u64, end: u64) -> Result<Vec<u8>, PreviewRefusal> {
        debug_assert!(start <= end && end < self.layout.total());
        let mut bytes = Vec::with_capacity((end - start + 1) as usize);
        if start < HEADER_LEN {
            let stop = (end + 1).min(HEADER_LEN);
            bytes.extend_from_slice(&self.layout.header()[start as usize..stop as usize]);
        }
        if end >= HEADER_LEN {
            let from = start.max(HEADER_LEN) - HEADER_LEN;
            let to = end - HEADER_LEN;
            let frame = self.layout.frame_bytes();
            let (first, last) = (from / frame, to / frame);
            let pcm = self.frames(first, last - first + 1)?;
            let skip = (from - first * frame) as usize;
            let take = (to - from + 1) as usize;
            bytes.extend_from_slice(&pcm[skip..skip + take]);
        }
        Ok(bytes)
    }

    /// `ticks` of the track's time base as frames.
    fn frames_of(&self, ticks: i64) -> u64 {
        let ticks = ticks.max(0) as u128;
        match self.time_base {
            None => ticks as u64,
            Some(tb) => {
                (ticks * u128::from(tb.numer.get()) * u128::from(self.layout.rate)
                    / u128::from(tb.denom.get())) as u64
            }
        }
    }

    /// The time-base ticks at frame `frame`.
    fn ticks_of(&self, frame: u64) -> i64 {
        match self.time_base {
            None => frame as i64,
            Some(tb) => {
                (u128::from(frame) * u128::from(tb.denom.get())
                    / (u128::from(tb.numer.get()) * u128::from(self.layout.rate)))
                    as i64
            }
        }
    }

    /// `count` frames from frame `first`, as PCM in the layout's sample
    /// size. Frames the file doesn't hold (it ends early, or a stretch is
    /// damaged) are silence, so the length is always what the header said.
    fn frames(&mut self, first: u64, count: u64) -> Result<Vec<u8>, PreviewRefusal> {
        let layout = self.layout;
        let end = first + count;
        let mut out = Vec::with_capacity((count * layout.frame_bytes()) as usize);
        // The frame the next decoded block starts at.
        let mut pos = 0u64;
        // Wherever the reader is, a request starts where it asks: only a
        // reader nothing has been read from is already at the start.
        let needs_seek = first > 0 || !self.fresh;
        self.fresh = false;
        if needs_seek {
            let to = SeekTo::Timestamp {
                ts: Timestamp::new(self.ticks_of(first)),
                track_id: self.track_id,
            };
            match self.format.seek(SeekMode::Accurate, to) {
                Ok(seeked) => {
                    self.decoder.reset();
                    pos = self.frames_of(seeked.actual_ts.get());
                }
                // Past the end of the data: all silence.
                Err(SymphoniaError::SeekError(_)) => pos = end,
                Err(e) => return Err(refusal(&e)),
            }
        }
        let mut cursor = first;
        let mut scratch16: Vec<i16> = Vec::new();
        let mut scratch32: Vec<i32> = Vec::new();
        let mut errors_in_a_row = 0u32;
        while cursor < end && pos < end {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(SymphoniaError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(SymphoniaError::ResetRequired) => break,
                Err(SymphoniaError::IoError(e)) => return Err(io_refusal(&e)),
                Err(_) => {
                    errors_in_a_row += 1;
                    if errors_in_a_row >= MAX_ERRORS_IN_A_ROW {
                        break;
                    }
                    continue;
                }
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(_) => {
                    errors_in_a_row += 1;
                    if errors_in_a_row >= MAX_ERRORS_IN_A_ROW {
                        break;
                    }
                    continue;
                }
            };
            errors_in_a_row = 0;
            let n = decoded.frames() as u64;
            self.decoded_frames += n;
            let block_end = pos + n;
            if block_end <= cursor || n == 0 {
                pos = block_end;
                continue;
            }
            // A hole before this block (damage skipped): silence.
            if pos > cursor {
                let hole = pos.min(end) - cursor;
                out.resize(out.len() + (hole * layout.frame_bytes()) as usize, 0);
                cursor += hole;
                if cursor >= end {
                    break;
                }
            }
            let skip = (cursor - pos) as usize;
            let take = ((block_end.min(end) - cursor) as usize).min(n as usize - skip);
            let ch = usize::from(layout.channels);
            if layout.bytes == 2 {
                decoded.copy_to_vec_interleaved(&mut scratch16);
                for sample in &scratch16[skip * ch..(skip + take) * ch] {
                    out.extend_from_slice(&sample.to_le_bytes());
                }
            } else {
                decoded.copy_to_vec_interleaved(&mut scratch32);
                for sample in &scratch32[skip * ch..(skip + take) * ch] {
                    // The top 24 bits of the decoder's 32-bit scale.
                    out.extend_from_slice(&(sample >> 8).to_le_bytes()[..3]);
                }
            }
            cursor += take as u64;
            pos = block_end;
        }
        out.resize((count * layout.frame_bytes()) as usize, 0);
        Ok(out)
    }
}
