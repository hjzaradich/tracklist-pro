//! The codec inside a container, from its headers (1aB-3).
//!
//! The sniffer says what the container is; for most formats that names the
//! codec too (a FLAC file holds FLAC). Three don't: a WAV names its codec in
//! the `fmt ` chunk (PCM, float, an MP3 stream…), an AIFC in its `COMM`
//! chunk, and an MP4 in the sample entry under `moov/trak/…/stsd` (AAC or
//! ALAC). Those headers are read here, bounded like the sniffer: chunk and
//! box bodies are skipped by seeking, never read, so a 2 GB file costs the
//! same as a 2 MB one. The audio itself is never read.
//!
//! A codec this module can't place is [`Codec::Other`]; one it can't look
//! for (an MP4 with no `moov`) is `None`.

use std::io::{self, Read, Seek, SeekFrom};

use crate::sniff::SniffedFormat;

/// A file's codec. Stored in `file.codec` as [`Codec::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    /// MPEG audio (layer III, or the rare layer I/II).
    Mp3,
    /// Integer PCM, in a WAV or an AIFF.
    Pcm,
    /// Floating-point PCM.
    PcmFloat,
    AdpcmMs,
    AdpcmIma,
    Alaw,
    Mulaw,
    Flac,
    Aac,
    Alac,
    Vorbis,
    Opus,
    Speex,
    /// Dolby Digital (AC-3) and Dolby Digital Plus (E-AC-3).
    Ac3,
    Eac3,
    /// Windows Media Audio, any flavor.
    Wma,
    WavPack,
    Musepack,
    /// Monkey's Audio.
    Ape,
    /// A codec the container names but this module doesn't know.
    Other,
}

impl Codec {
    /// The stable name stored in `file.codec`.
    pub fn as_str(self) -> &'static str {
        match self {
            Codec::Mp3 => "mp3",
            Codec::Pcm => "pcm",
            Codec::PcmFloat => "pcm_float",
            Codec::AdpcmMs => "adpcm_ms",
            Codec::AdpcmIma => "adpcm_ima",
            Codec::Alaw => "alaw",
            Codec::Mulaw => "mulaw",
            Codec::Flac => "flac",
            Codec::Aac => "aac",
            Codec::Alac => "alac",
            Codec::Vorbis => "vorbis",
            Codec::Opus => "opus",
            Codec::Speex => "speex",
            Codec::Ac3 => "ac3",
            Codec::Eac3 => "eac3",
            Codec::Wma => "wma",
            Codec::WavPack => "wavpack",
            Codec::Musepack => "musepack",
            Codec::Ape => "ape",
            Codec::Other => "other",
        }
    }
}

/// Chunks or boxes looked at before giving up.
const MAX_CHUNKS: usize = 256;
/// Stacked ID3v2 tags skipped before the container.
const MAX_TAGS: usize = 8;
/// How deep the MP4 box walk goes: moov/trak/mdia/minf/stbl/stsd.
const MAX_DEPTH: usize = 6;

/// The codec in `reader`, whose bytes sniffed as `format`. `None` when the
/// headers that would name it are missing or cut short.
pub fn codec<R: Read + Seek>(reader: &mut R, format: SniffedFormat) -> io::Result<Option<Codec>> {
    use SniffedFormat as F;
    Ok(match format {
        F::Mp3 => Some(Codec::Mp3),
        F::Flac => Some(Codec::Flac),
        F::Adts => Some(Codec::Aac),
        F::OggVorbis => Some(Codec::Vorbis),
        F::OggOpus => Some(Codec::Opus),
        F::Asf => Some(Codec::Wma),
        F::WavPack => Some(Codec::WavPack),
        F::Musepack => Some(Codec::Musepack),
        F::Ape => Some(Codec::Ape),
        F::Aiff => Some(Codec::Pcm),
        F::Wav | F::Rf64 => {
            let mut h = Headers::new(reader)?;
            h.wav()?
        }
        F::Aifc => {
            let mut h = Headers::new(reader)?;
            h.aifc()?
        }
        F::Mp4 => {
            let mut h = Headers::new(reader)?;
            h.mp4()?
        }
        F::Ogg => {
            let mut h = Headers::new(reader)?;
            h.ogg()?
        }
        F::Unknown => None,
    })
}

/// The container's first four bytes, after any ID3v2 tags (e.g. `RIFF`
/// or `RIFX` for a WAVE). `None` for a file too short to hold them.
pub fn container_magic<R: Read + Seek>(reader: &mut R) -> io::Result<Option<[u8; 4]>> {
    let mut h = Headers::new(reader)?;
    let head = h.read_at(h.start, 4)?;
    Ok(head.try_into().ok())
}

/// A WAVE format tag (or an extensible format's sub-format) as a codec.
fn wave_format(tag: u16) -> Codec {
    match tag {
        0x0001 => Codec::Pcm,
        0x0002 => Codec::AdpcmMs,
        0x0003 => Codec::PcmFloat,
        0x0006 => Codec::Alaw,
        0x0007 => Codec::Mulaw,
        0x0011 => Codec::AdpcmIma,
        0x0050 | 0x0055 => Codec::Mp3,
        0x0092 | 0x2000 => Codec::Ac3,
        0x00FF | 0x1600 | 0x1610 => Codec::Aac,
        0x0161..=0x0163 => Codec::Wma,
        0xF1AC => Codec::Flac,
        _ => Codec::Other,
    }
}

/// An AIFC compression type as a codec.
fn aifc_compression(kind: &[u8]) -> Codec {
    match kind {
        b"NONE" | b"none" | b"twos" | b"sowt" | b"raw " | b"in24" | b"in32" | b"42ni" => Codec::Pcm,
        b"fl32" | b"FL32" | b"fl64" | b"FL64" => Codec::PcmFloat,
        b"alaw" | b"ALAW" => Codec::Alaw,
        b"ulaw" | b"ULAW" => Codec::Mulaw,
        b"ima4" => Codec::AdpcmIma,
        _ => Codec::Other,
    }
}

/// An MP4 audio sample entry's type as a codec.
fn sample_entry(kind: &[u8]) -> Codec {
    match kind {
        b"mp4a" => Codec::Aac,
        b"alac" => Codec::Alac,
        b"fLaC" => Codec::Flac,
        b"Opus" => Codec::Opus,
        b"ac-3" => Codec::Ac3,
        b"ec-3" => Codec::Eac3,
        b".mp3" => Codec::Mp3,
        b"lpcm" | b"sowt" | b"twos" | b"raw " | b"in24" | b"in32" => Codec::Pcm,
        b"fl32" | b"fl64" => Codec::PcmFloat,
        _ => Codec::Other,
    }
}

struct Headers<'r, R> {
    r: &'r mut R,
    len: u64,
    /// Where the container starts, after any ID3v2 tags.
    start: u64,
    /// Chunks or boxes looked at so far.
    seen: usize,
}

impl<'r, R: Read + Seek> Headers<'r, R> {
    fn new(r: &'r mut R) -> io::Result<Headers<'r, R>> {
        let len = r.seek(SeekFrom::End(0))?;
        let mut h = Headers {
            r,
            len,
            start: 0,
            seen: 0,
        };
        h.start = h.skip_id3()?;
        Ok(h)
    }

    /// Up to `n` bytes from `at`, fewer if the file ends first.
    fn read_at(&mut self, at: u64, n: u64) -> io::Result<Vec<u8>> {
        if at >= self.len {
            return Ok(Vec::new());
        }
        let n = n.min(self.len - at);
        self.r.seek(SeekFrom::Start(at))?;
        let mut buf = Vec::with_capacity(n as usize);
        self.r.by_ref().take(n).read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Where the bytes after any leading ID3v2 tags start.
    fn skip_id3(&mut self) -> io::Result<u64> {
        let mut at = 0;
        for _ in 0..MAX_TAGS {
            let head = self.read_at(at, 10)?;
            if head.len() < 10 || &head[..3] != b"ID3" || head[6..10].iter().any(|b| b & 0x80 != 0)
            {
                break;
            }
            let size = head[6..10]
                .iter()
                .fold(0u64, |n, b| (n << 7) | u64::from(*b));
            let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
            at += 10 + size + footer;
        }
        Ok(at)
    }

    /// The codec a RIFF / RIFX / RF64 WAVE names in its `fmt ` chunk.
    fn wav(&mut self) -> io::Result<Option<Codec>> {
        let head = self.read_at(self.start, 12)?;
        if head.len() < 12 {
            return Ok(None);
        }
        let big = &head[..4] == b"RIFX";
        let u32_of = |b: &[u8]| {
            let b = [b[0], b[1], b[2], b[3]];
            if big {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            }
        };
        let u16_of = |b: &[u8]| {
            let b = [b[0], b[1]];
            if big {
                u16::from_be_bytes(b)
            } else {
                u16::from_le_bytes(b)
            }
        };
        let mut at = self.start + 12;
        while self.seen < MAX_CHUNKS {
            self.seen += 1;
            let chunk = self.read_at(at, 8)?;
            if chunk.len() < 8 {
                return Ok(None);
            }
            let size = u64::from(u32_of(&chunk[4..8]));
            if &chunk[..4] == b"fmt " {
                let fmt = self.read_at(at + 8, size.min(40))?;
                if fmt.len() < 2 {
                    return Ok(None);
                }
                let tag = u16_of(&fmt[..2]);
                // WAVE_FORMAT_EXTENSIBLE: the real format is the first two
                // bytes of the sub-format GUID.
                if tag == 0xFFFE {
                    return Ok(fmt.get(24..26).map(|b| wave_format(u16_of(b))));
                }
                return Ok(Some(wave_format(tag)));
            }
            // Chunks are padded to an even length.
            at += 8 + size + (size & 1);
        }
        Ok(None)
    }

    /// The codec an AIFC names in its `COMM` chunk.
    fn aifc(&mut self) -> io::Result<Option<Codec>> {
        let mut at = self.start + 12;
        while self.seen < MAX_CHUNKS {
            self.seen += 1;
            let chunk = self.read_at(at, 8)?;
            if chunk.len() < 8 {
                return Ok(None);
            }
            let size = u64::from(u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]));
            if &chunk[..4] == b"COMM" {
                // channels (2), frames (4), bits (2), rate (10), then the
                // compression type.
                let comm = self.read_at(at + 8, 22)?;
                return Ok(comm.get(18..22).map(aifc_compression));
            }
            at += 8 + size + (size & 1);
        }
        Ok(None)
    }

    /// The codec of an Ogg stream the sniffer didn't place, from its first
    /// packet.
    fn ogg(&mut self) -> io::Result<Option<Codec>> {
        let page = self.read_at(self.start, 27 + 255 + 16)?;
        let Some(&segments) = page.get(26) else {
            return Ok(None);
        };
        let packet = page.get(27 + usize::from(segments)..).unwrap_or_default();
        Ok(if packet.starts_with(b"\x7fFLAC") {
            Some(Codec::Flac)
        } else if packet.starts_with(b"Speex   ") {
            Some(Codec::Speex)
        } else if packet.starts_with(b"\x01vorbis") {
            Some(Codec::Vorbis)
        } else if packet.starts_with(b"OpusHead") {
            Some(Codec::Opus)
        } else if packet.is_empty() {
            None
        } else {
            Some(Codec::Other)
        })
    }

    /// One box header at `at`, inside a parent ending at `end`: its type,
    /// where its body starts and where it ends. `None` past the end or for a
    /// header that's cut short.
    fn mp4_box(&mut self, at: u64, end: u64) -> io::Result<Option<([u8; 4], u64, u64)>> {
        if at + 8 > end {
            return Ok(None);
        }
        let head = self.read_at(at, 16)?;
        if head.len() < 8 {
            return Ok(None);
        }
        let kind = [head[4], head[5], head[6], head[7]];
        let size = u64::from(u32::from_be_bytes([head[0], head[1], head[2], head[3]]));
        let (body, box_end) = match size {
            // To the end of the parent.
            0 => (at + 8, end),
            // A 64-bit size follows the type.
            1 => {
                let Some(large) = head.get(8..16) else {
                    return Ok(None);
                };
                let mut b = [0u8; 8];
                b.copy_from_slice(large);
                (at + 16, at.saturating_add(u64::from_be_bytes(b)))
            }
            n => (at + 8, at.saturating_add(n)),
        };
        if box_end < body || box_end > end {
            return Ok(None);
        }
        Ok(Some((kind, body, box_end)))
    }

    /// The boxes directly inside `[from, end)`, at most [`MAX_CHUNKS`]
    /// (each level has its own limit), stopping after the first of type
    /// `until` if given.
    fn children(
        &mut self,
        from: u64,
        end: u64,
        until: Option<&[u8; 4]>,
    ) -> io::Result<Vec<([u8; 4], u64, u64)>> {
        let mut out = Vec::new();
        let mut at = from;
        for _ in 0..MAX_CHUNKS {
            let Some(b) = self.mp4_box(at, end)? else {
                break;
            };
            at = b.2;
            let last = until.is_some_and(|kind| b.0 == *kind);
            out.push(b);
            if last {
                break;
            }
        }
        Ok(out)
    }

    /// The codec of an MP4's sound track: the first sample entry under
    /// `moov/trak/mdia/minf/stbl/stsd` of a track whose handler is `soun`,
    /// or of the first track if none says so.
    fn mp4(&mut self) -> io::Result<Option<Codec>> {
        // A fragmented MP4 can have thousands of top-level boxes after
        // `moov`; the walk stops there.
        let top = self.children(self.start, self.len, Some(b"moov"))?;
        let Some(&(_, body, end)) = top.iter().find(|b| &b.0 == b"moov") else {
            return Ok(None);
        };
        let mut first = None;
        for (kind, body, end) in self.children(body, end, None)? {
            if &kind != b"trak" {
                continue;
            }
            let (sound, entry) = self.track(body, end, 1)?;
            if sound && entry.is_some() {
                return Ok(entry.map(|e| sample_entry(&e)));
            }
            first = first.or(entry);
        }
        Ok(first.map(|e| sample_entry(&e)))
    }

    /// Inside a `trak` (or a box below it, at `depth`): whether its handler
    /// is `soun`, and the type of its first sample entry.
    fn track(&mut self, from: u64, end: u64, depth: usize) -> io::Result<(bool, Option<[u8; 4]>)> {
        let mut sound = false;
        let mut entry = None;
        if depth > MAX_DEPTH {
            return Ok((sound, entry));
        }
        for (kind, body, box_end) in self.children(from, end, None)? {
            match &kind {
                b"mdia" | b"minf" | b"stbl" => {
                    let (s, e) = self.track(body, box_end, depth + 1)?;
                    sound |= s;
                    entry = entry.or(e);
                }
                // Full box: version and flags (4), predefined (4), then the
                // handler type.
                b"hdlr" => sound |= self.read_at(body + 8, 4)? == b"soun",
                // Full box (4), entry count (4), then the first entry's
                // size (4) and type (4).
                b"stsd" => {
                    let head = self.read_at(body + 8, 8)?;
                    if let Some(t) = head.get(4..8) {
                        entry = entry.or(Some([t[0], t[1], t[2], t[3]]));
                    }
                }
                _ => {}
            }
        }
        Ok((sound, entry))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn codec_of(bytes: &[u8], format: SniffedFormat) -> Option<Codec> {
        codec(&mut Cursor::new(bytes), format).unwrap()
    }

    fn le_chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn wave(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body = [b"WAVE".to_vec(), chunks.concat()].concat();
        [
            b"RIFF".to_vec(),
            (body.len() as u32).to_le_bytes().to_vec(),
            body,
        ]
        .concat()
    }

    fn fmt(tag: u16) -> Vec<u8> {
        let mut f = tag.to_le_bytes().to_vec();
        f.extend_from_slice(&[0; 14]);
        le_chunk(b"fmt ", &f)
    }

    fn be_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (8 + body.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    /// A `trak` whose handler is `handler` and whose first sample entry is
    /// of type `entry`.
    fn trak(handler: &[u8; 4], entry: &[u8; 4]) -> Vec<u8> {
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(handler);
        hdlr.extend_from_slice(&[0; 13]);
        let mut stsd = vec![0u8; 4];
        stsd.extend_from_slice(&1u32.to_be_bytes());
        stsd.extend(be_box(entry, &[0; 8]));
        let stbl = be_box(b"stbl", &be_box(b"stsd", &stsd));
        let minf = be_box(b"minf", &stbl);
        let mdia = be_box(b"mdia", &[be_box(b"hdlr", &hdlr), minf].concat());
        be_box(b"trak", &mdia)
    }

    fn mp4(traks: &[Vec<u8>], after_moov: &[Vec<u8>]) -> Vec<u8> {
        let ftyp = be_box(b"ftyp", b"M4A \x00\x00\x00\x00M4A mp42isom");
        let moov = be_box(b"moov", &traks.concat());
        [vec![ftyp, moov], after_moov.to_vec()].concat().concat()
    }

    /// An Ogg page holding one packet.
    fn ogg_page(packet: &[u8]) -> Vec<u8> {
        let mut page = b"OggS".to_vec();
        page.extend_from_slice(&[0, 2]); // version, first page
        page.extend_from_slice(&[0; 8]); // granule position
        page.extend_from_slice(&[1, 0, 0, 0]); // serial
        page.extend_from_slice(&[0; 8]); // sequence, CRC
        page.push(1);
        page.push(packet.len() as u8);
        page.extend_from_slice(packet);
        page
    }

    #[test]
    fn every_known_codec_name_in_each_container_maps_to_its_codec() {
        for (tag, expected) in [
            (0x0001, Codec::Pcm),
            (0x0002, Codec::AdpcmMs),
            (0x0003, Codec::PcmFloat),
            (0x0006, Codec::Alaw),
            (0x0007, Codec::Mulaw),
            (0x0011, Codec::AdpcmIma),
            (0x0050, Codec::Mp3),
            (0x0055, Codec::Mp3),
            (0x0092, Codec::Ac3),
            (0x2000, Codec::Ac3),
            (0x00FF, Codec::Aac),
            (0x1610, Codec::Aac),
            (0x0161, Codec::Wma),
            (0x0163, Codec::Wma),
            (0xF1AC, Codec::Flac),
            (0x0000, Codec::Other),
        ] {
            assert_eq!(wave_format(tag), expected, "WAVE {tag:#06x}");
        }
        for (kind, expected) in [
            (b"NONE", Codec::Pcm),
            (b"twos", Codec::Pcm),
            (b"sowt", Codec::Pcm),
            (b"in24", Codec::Pcm),
            (b"fl32", Codec::PcmFloat),
            (b"FL64", Codec::PcmFloat),
            (b"alaw", Codec::Alaw),
            (b"ulaw", Codec::Mulaw),
            (b"ima4", Codec::AdpcmIma),
            (b"MAC3", Codec::Other),
        ] {
            assert_eq!(aifc_compression(kind), expected, "AIFC {kind:?}");
        }
        for (kind, expected) in [
            (b"mp4a", Codec::Aac),
            (b"alac", Codec::Alac),
            (b"fLaC", Codec::Flac),
            (b"Opus", Codec::Opus),
            (b"ac-3", Codec::Ac3),
            (b"ec-3", Codec::Eac3),
            (b".mp3", Codec::Mp3),
            (b"lpcm", Codec::Pcm),
            (b"fl32", Codec::PcmFloat),
            (b"samr", Codec::Other),
        ] {
            assert_eq!(sample_entry(kind), expected, "MP4 {kind:?}");
            let bytes = mp4(&[trak(b"soun", kind)], &[]);
            assert_eq!(
                codec_of(&bytes, SniffedFormat::Mp4),
                Some(expected),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn an_ogg_stream_the_sniffer_did_not_place_is_named_by_its_first_packet() {
        for (packet, expected) in [
            (&b"\x7fFLAC\x01\x00"[..], Some(Codec::Flac)),
            (b"Speex   1.2", Some(Codec::Speex)),
            (b"\x01vorbis", Some(Codec::Vorbis)),
            (b"OpusHead", Some(Codec::Opus)),
            (b"\x80theora", Some(Codec::Other)),
        ] {
            let got = codec_of(&ogg_page(packet), SniffedFormat::Ogg);
            assert_eq!(got, expected, "{packet:?}");
        }
    }

    #[test]
    fn an_mp4_with_a_video_track_first_is_named_by_its_sound_track() {
        let bytes = mp4(&[trak(b"vide", b"avc1"), trak(b"soun", b"alac")], &[]);
        assert_eq!(codec_of(&bytes, SniffedFormat::Mp4), Some(Codec::Alac));
        // With no sound handler anywhere, the first track's entry is used.
        let bytes = mp4(&[trak(b"xxxx", b"mp4a")], &[]);
        assert_eq!(codec_of(&bytes, SniffedFormat::Mp4), Some(Codec::Aac));
    }

    #[test]
    fn a_fragmented_mp4_with_thousands_of_boxes_after_moov_keeps_its_codec() {
        let fragments: Vec<_> = (0..3000).map(|_| be_box(b"moof", &[])).collect();
        let bytes = mp4(&[trak(b"soun", b"mp4a")], &fragments);
        assert_eq!(codec_of(&bytes, SniffedFormat::Mp4), Some(Codec::Aac));
    }

    #[test]
    fn a_wav_names_its_codec_in_its_fmt_chunk_even_after_other_chunks() {
        let odd = le_chunk(b"LIST", b"odd");
        for (tag, expected) in [
            (1, Codec::Pcm),
            (3, Codec::PcmFloat),
            (0x55, Codec::Mp3),
            (0x11, Codec::AdpcmIma),
            (0x1234, Codec::Other),
        ] {
            let bytes = wave(&[odd.clone(), fmt(tag), le_chunk(b"data", &[0; 4])]);
            assert_eq!(
                codec_of(&bytes, SniffedFormat::Wav),
                Some(expected),
                "{tag:#x}"
            );
        }
    }

    #[test]
    fn an_extensible_wav_is_named_by_its_sub_format() {
        let mut f = 0xFFFEu16.to_le_bytes().to_vec();
        f.extend_from_slice(&[0; 22]);
        f.extend_from_slice(&3u16.to_le_bytes()); // float sub-format
        f.extend_from_slice(&[0; 14]);
        let bytes = wave(&[le_chunk(b"fmt ", &f)]);
        assert_eq!(codec_of(&bytes, SniffedFormat::Wav), Some(Codec::PcmFloat));
    }

    #[test]
    fn the_test_wav_aiff_and_m4a_fixtures_are_pcm_pcm_and_aac() {
        use crate::tags::test_audio as audio;
        assert_eq!(
            codec_of(&audio::wav(), SniffedFormat::Wav),
            Some(Codec::Pcm)
        );
        assert_eq!(
            codec_of(&audio::aiff(), SniffedFormat::Aiff),
            Some(Codec::Pcm)
        );
        assert_eq!(
            codec_of(&audio::m4a(), SniffedFormat::Mp4),
            Some(Codec::Aac)
        );
        assert_eq!(
            codec_of(&audio::mp3(), SniffedFormat::Mp3),
            Some(Codec::Mp3)
        );
    }

    #[test]
    fn an_m4a_holding_alac_is_alac() {
        let mut bytes = crate::tags::test_audio::m4a();
        let at = bytes.windows(4).position(|w| w == b"mp4a").unwrap();
        bytes[at..at + 4].copy_from_slice(b"alac");
        assert_eq!(codec_of(&bytes, SniffedFormat::Mp4), Some(Codec::Alac));
    }

    #[test]
    fn an_mp4_without_moov_has_no_codec_to_find() {
        let mut bytes = crate::tags::test_audio::m4a();
        let at = bytes.windows(4).position(|w| w == b"moov").unwrap();
        bytes[at..at + 4].copy_from_slice(b"free");
        assert_eq!(codec_of(&bytes, SniffedFormat::Mp4), None);
    }

    #[test]
    fn an_aifc_names_its_codec_in_its_comm_chunk() {
        for (kind, expected) in [
            (b"sowt", Codec::Pcm),
            (b"fl32", Codec::PcmFloat),
            (b"QDM2", Codec::Other),
        ] {
            let mut comm = vec![0u8; 18];
            comm.extend_from_slice(kind);
            comm.extend_from_slice(&[0, 0]);
            let mut body = b"AIFC".to_vec();
            body.extend_from_slice(b"COMM");
            body.extend_from_slice(&(comm.len() as u32).to_be_bytes());
            body.extend_from_slice(&comm);
            let bytes = [
                b"FORM".to_vec(),
                (body.len() as u32).to_be_bytes().to_vec(),
                body,
            ]
            .concat();
            assert_eq!(codec_of(&bytes, SniffedFormat::Aifc), Some(expected));
        }
    }

    #[test]
    fn a_wav_behind_an_id3_tag_is_still_read() {
        let tag = crate::tags::test_audio::id3v2(&[], 20);
        let bytes = [tag, crate::tags::test_audio::wav()].concat();
        assert_eq!(codec_of(&bytes, SniffedFormat::Wav), Some(Codec::Pcm));
    }

    #[test]
    fn cut_short_or_lying_headers_give_no_codec_and_never_panic() {
        let whole = crate::tags::test_audio::m4a();
        for len in 0..whole.len().min(600) {
            let _ = codec_of(&whole[..len], SniffedFormat::Mp4);
        }
        let wav = crate::tags::test_audio::wav();
        // The format tag is bytes 20..22.
        for len in 0..22 {
            assert_eq!(codec_of(&wav[..len], SniffedFormat::Wav), None, "{len}");
        }
        // A chunk claiming 4 GB is skipped by seeking, not read.
        let bytes = wave(&[le_chunk(b"JUNK", &[]), fmt(1)]);
        let mut lying = bytes.clone();
        lying[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(codec_of(&lying, SniffedFormat::Wav), None);
    }
}
