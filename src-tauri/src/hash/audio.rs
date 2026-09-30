//! What `file.audio_hash` covers: the audio frames only, never the tags
//! (ROADMAP §2, §5.1). rekordbox and taggers rewrite tags in place, so a
//! file whose tags changed keeps its `audio_hash`, and 1.4 can match it as
//! an exact duplicate on the fast path.
//!
//! # The stored value
//!
//! 34 bytes: `[DEFINITION, format code, 32-byte digest]`. The digest is
//! BLAKE3 in key-derivation mode, with the context string
//! `"tracklist-pro audio_hash v1 <format name>"`, over the bytes listed
//! below, in that order. Two values are equal only if the definition, the
//! format family and the audio all are, so a later definition can never
//! falsely match an old one: bump [`DEFINITION`] whenever anything in this
//! comment changes, and every file is hashed again.
//!
//! | code | name         | sniffed as                |
//! |------|--------------|---------------------------|
//! | 1    | `mp3`        | `mp3`                     |
//! | 2    | `adts`       | `adts`                    |
//! | 3    | `wave`       | `wav` (RIFF, RIFX), `rf64` (RF64, BW64) |
//! | 4    | `aiff`       | `aiff`, `aifc`            |
//! | 5    | `flac`       | `flac`                    |
//! | 6    | `mp4`        | `mp4`                     |
//! | 7    | `ogg_vorbis` | `ogg_vorbis`              |
//! | 8    | `ogg_opus`   | `ogg_opus`                |
//!
//! # Definition 1, per format
//!
//! First, for every format, up to 8 ID3v2 tags at the very start of the
//! file are skipped by their size fields (some taggers put one in front of
//! a FLAC or WAV too). "Trailing tags" below means, repeated from the end
//! of the file in any order, up to 8 of: an APEv2 tag (by its `APETAGEX`
//! footer, header included if the footer says there is one; checked first,
//! so an APE tag is never taken for ID3v1), an ID3v1 tag (`TAG`, 128
//! bytes), a Lyrics3 v2 block (`LYRICSBEGIN` … size + `LYRICS200`) and an
//! ID3v2.4 tag with a footer (`3DI`). Known limits, counted as audio: an
//! ID3v2.3 tag appended at the end (it has no footer to find it by), the
//! extended ID3v1 `TAG+` block, and Lyrics3 v1 (no size field).
//!
//! "A length" below is a u64, little-endian, hashed before the bytes it
//! counts, so two parts can never run into each other.
//!
//! - **mp3, adts**: the bytes from the first frame header (the first offset
//!   within 4 KB after the leading tags where a frame header is followed
//!   by two more of the same stream, the same test as the sniffer's) to the
//!   start of the trailing tags. Junk or padding between the tag and the
//!   first frame is left out. The Xing/LAME info frame, if any, is a frame
//!   and is included.
//! - **wave**: the length of the `fmt ` chunk's body and the body, then
//!   the body of the first `data` chunk (its declared size, pad byte
//!   excluded), whatever order they're in and whatever other chunks
//!   (`LIST`, `id3 `, `ID3 `, `bext`, `iXML`, `cue `, …) surround them. In
//!   RF64/BW64 a data size of 0xFFFFFFFF is read from `ds64`. Otherwise a
//!   data size of 0 or 0xFFFFFFFF (left by streaming writers) is
//!   `malformed`: where the audio ends isn't guessed.
//! - **aiff**: 22 bytes describing the audio: the first 18 bytes of `COMM`
//!   (channels, frames, bits, rate) then the AIFC compression type, or
//!   `NONE` for plain AIFF, so an AIFF and an uncompressed AIFC of the same
//!   samples match. Then the sample bytes of the first `SSND` chunk (after
//!   its offset and block-size fields and `offset` bytes of alignment).
//!   Every other chunk (`ID3 `, `NAME`, `AUTH`, `ANNO`, `(c) `, `COMT`,
//!   `MARK`, `INST`, `APPL`, …) is left out, in any order.
//! - **flac**: bytes 10–17 of STREAMINFO's body (sample rate, channels,
//!   bits per sample, total samples; STREAMINFO must be the first block).
//!   Then, after `fLaC` and every metadata block (padding, Vorbis comments,
//!   pictures, seek table, cue sheet, application), the frames: from the
//!   first frame's sync code to the start of the trailing tags.
//! - **mp4**: for each sound track (a `moov/trak` whose `mdia` holds an
//!   `hdlr` naming the `soun` handler; no `hdlr`, or one too short to name
//!   one, and it isn't), in file order, the length and body of its
//!   `mdia/minf/stbl/stsd` box (an `stsd` anywhere else is ignored): the sample entries, which name the codec,
//!   rate, channels and decoder config. Then the bodies (headers excluded)
//!   of every top-level `mdat` box, in file order. Everything else in
//!   `moov` (the `udta`/`meta`/`ilst` metadata atoms, chunk offsets that
//!   move when it grows), `free`, `skip`, `wide` and any other box is left
//!   out. No sound track, or no `mdat`, is `malformed`.
//! - **ogg_vorbis, ogg_opus**: the packets of the (single) logical stream,
//!   reassembled from its pages, except the second packet (the Vorbis
//!   comment header or `OpusTags`). Each packet's bytes are followed by its
//!   length (u64, little-endian). Then the granule position of the last
//!   page that has one (u64, little-endian), which sets the stream's exact
//!   length. Page boundaries, sequence numbers and checksums are left out,
//!   because rewriting the comment packet renumbers every page after it.
//!   Bytes after the end-of-stream page that aren't a page (an appended
//!   tag) are ignored.
//!
//! # When there's no audio_hash
//!
//! The audio_hash is `None` with a [`Skip`] reason, never a hash of the
//! whole file: `empty` (zero bytes), `truncated` (a header, chunk, box,
//! block or page runs past the end of the file, the stream has no
//! end-of-stream page, or an MP4 has no `moov`), `unknown_format` (bytes
//! the sniffer can't place), `unsupported_format` (WMA, WavPack,
//! Musepack, Monkey's Audio, other Ogg codecs, multiplexed or chained Ogg
//! streams), `malformed` (the structure can't be followed: no format or
//! audio chunk, no frames where they should start, no audio at all).

use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;

use crate::sniff::{self, AdtsHeader, MpegHeader, Problem, SniffedFormat};

/// The audio_hash definition this build computes. Bump it whenever the
/// definition in the module docs changes.
pub const DEFINITION: u8 = 1;

/// How many bytes a stored audio_hash has: definition, format code, digest.
pub const AUDIO_HASH_LEN: usize = 34;

/// Stacked tags skipped at either end before giving up.
const MAX_TAGS: usize = 8;
/// Chunks, boxes or metadata blocks walked before calling the file
/// malformed.
const MAX_CHUNKS: usize = 4096;
/// The biggest `fmt ` chunk read into memory; real ones are 16–40 bytes.
const MAX_FMT: u32 = 64 * 1024;
/// The biggest `stsd` box read into memory; real ones are under 1 KB.
const MAX_STSD: u64 = 64 * 1024;
/// How deep `moov` is searched for sound tracks (`moov/trak/mdia/minf/stbl`).
const MP4_DEPTH: usize = 5;
/// Where the first MPEG or ADTS frame is looked for, after the leading
/// tags. The sniffer's window, so a file it calls MP3 has a frame here.
const FRAME_WINDOW: u64 = 4096;
/// Frames that must follow the first one, as in the sniffer.
const FRAMES_TO_CONFIRM: usize = 2;

/// A format family, as the audio_hash defines it (module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioFormat {
    Mp3,
    Adts,
    Wave,
    Aiff,
    Flac,
    Mp4,
    OggVorbis,
    OggOpus,
}

impl AudioFormat {
    /// Every family, in code order.
    pub const ALL: [AudioFormat; 8] = [
        AudioFormat::Mp3,
        AudioFormat::Adts,
        AudioFormat::Wave,
        AudioFormat::Aiff,
        AudioFormat::Flac,
        AudioFormat::Mp4,
        AudioFormat::OggVorbis,
        AudioFormat::OggOpus,
    ];

    /// The second byte of a stored audio_hash. Never reuse a code.
    pub fn code(self) -> u8 {
        match self {
            AudioFormat::Mp3 => 1,
            AudioFormat::Adts => 2,
            AudioFormat::Wave => 3,
            AudioFormat::Aiff => 4,
            AudioFormat::Flac => 5,
            AudioFormat::Mp4 => 6,
            AudioFormat::OggVorbis => 7,
            AudioFormat::OggOpus => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Adts => "adts",
            AudioFormat::Wave => "wave",
            AudioFormat::Aiff => "aiff",
            AudioFormat::Flac => "flac",
            AudioFormat::Mp4 => "mp4",
            AudioFormat::OggVorbis => "ogg_vorbis",
            AudioFormat::OggOpus => "ogg_opus",
        }
    }

    /// The BLAKE3 key-derivation context: globally unique, and fixed for
    /// this definition and family.
    fn context(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "tracklist-pro audio_hash v1 mp3",
            AudioFormat::Adts => "tracklist-pro audio_hash v1 adts",
            AudioFormat::Wave => "tracklist-pro audio_hash v1 wave",
            AudioFormat::Aiff => "tracklist-pro audio_hash v1 aiff",
            AudioFormat::Flac => "tracklist-pro audio_hash v1 flac",
            AudioFormat::Mp4 => "tracklist-pro audio_hash v1 mp4",
            AudioFormat::OggVorbis => "tracklist-pro audio_hash v1 ogg_vorbis",
            AudioFormat::OggOpus => "tracklist-pro audio_hash v1 ogg_opus",
        }
    }

    fn hasher(self) -> blake3::Hasher {
        blake3::Hasher::new_derive_key(self.context())
    }
}

/// Why a file has no audio_hash. Stored as [`Skip::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Skip {
    Empty,
    Truncated,
    UnknownFormat,
    UnsupportedFormat,
    Malformed,
}

impl Skip {
    pub const ALL: [Skip; 5] = [
        Skip::Empty,
        Skip::Truncated,
        Skip::UnknownFormat,
        Skip::UnsupportedFormat,
        Skip::Malformed,
    ];

    /// The reason as stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Skip::Empty => "empty",
            Skip::Truncated => "truncated",
            Skip::UnknownFormat => "unknown_format",
            Skip::UnsupportedFormat => "unsupported_format",
            Skip::Malformed => "malformed",
        }
    }
}

/// A computed audio_hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioHash {
    pub format: AudioFormat,
    pub digest: [u8; 32],
}

impl AudioHash {
    /// The stored form: definition, format code, digest.
    pub fn to_bytes(self) -> [u8; AUDIO_HASH_LEN] {
        let mut out = [0u8; AUDIO_HASH_LEN];
        out[0] = DEFINITION;
        out[1] = self.format.code();
        out[2..].copy_from_slice(&self.digest);
        out
    }
}

/// How to hash one file's audio while it's streamed: worked out first from
/// its headers, with a few small reads.
pub(crate) enum Plan {
    /// Hash `head` (small descriptor bytes read while planning), then the
    /// bytes of `ranges` (sorted, disjoint) as the stream passes them.
    Ranges {
        format: AudioFormat,
        head: Vec<u8>,
        ranges: Vec<Range<u64>>,
    },
    /// Demux Ogg pages from `start`.
    Ogg { format: AudioFormat, start: u64 },
    /// No audio_hash for this file.
    Skip(Skip),
}

/// Works out the [`Plan`] for the file in `r`, `len` bytes long. Reads a
/// bounded number of bytes, except that it seeks past chunk and block
/// bodies without reading them.
pub(crate) fn plan<R: Read + Seek>(r: &mut R, len: u64) -> io::Result<Plan> {
    let sniffed = sniff::sniff(r, None)?;
    match sniffed.problem {
        Some(Problem::Empty) => return Ok(Plan::Skip(Skip::Empty)),
        Some(Problem::Truncated | Problem::NoMoov) => return Ok(Plan::Skip(Skip::Truncated)),
        None => {}
    }
    let mut p = Probe { r, len };
    let Some(start) = p.skip_leading_tags()? else {
        return Ok(Plan::Skip(Skip::Truncated));
    };
    use SniffedFormat as F;
    let planned = match sniffed.format {
        F::Mp3 => p.frames(start, AudioFormat::Mp3)?,
        F::Adts => p.frames(start, AudioFormat::Adts)?,
        F::Wav | F::Rf64 => p.wave(start)?,
        F::Aiff | F::Aifc => p.aiff(start)?,
        F::Flac => p.flac(start)?,
        F::Mp4 => p.mp4(start)?,
        F::OggVorbis => Ok(Plan::Ogg {
            format: AudioFormat::OggVorbis,
            start,
        }),
        F::OggOpus => Ok(Plan::Ogg {
            format: AudioFormat::OggOpus,
            start,
        }),
        F::Ogg | F::Asf | F::WavPack | F::Musepack | F::Ape => Err(Skip::UnsupportedFormat),
        F::Unknown => Err(Skip::UnknownFormat),
    };
    Ok(planned.unwrap_or_else(Plan::Skip))
}

/// A plan, or why there's none.
type Planned = Result<Plan, Skip>;

/// A box inside another: its type and body.
type Child = ([u8; 4], Range<u64>);

struct Probe<'r, R> {
    r: &'r mut R,
    len: u64,
}

impl<R: Read + Seek> Probe<'_, R> {
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

    /// Exactly `n` bytes from `at`, or `None` if the file ends first
    /// (including a file that shrank since its length was taken).
    fn read_exact_at(&mut self, at: u64, n: u64) -> io::Result<Option<Vec<u8>>> {
        let bytes = self.read_at(at, n)?;
        Ok((bytes.len() as u64 == n).then_some(bytes))
    }

    /// Where the file starts after its leading ID3v2 tags; `None` if they
    /// run to the end.
    fn skip_leading_tags(&mut self) -> io::Result<Option<u64>> {
        let mut at = 0u64;
        for _ in 0..MAX_TAGS {
            let Some(size) = sniff::id3v2_size(&self.read_at(at, 10)?) else {
                break;
            };
            at += size;
            if at >= self.len {
                return Ok(None);
            }
        }
        Ok(Some(at))
    }

    /// Where the audio ends: before any trailing tags between `start` and
    /// the end of the file (module docs).
    fn strip_trailing_tags(&mut self, start: u64) -> io::Result<u64> {
        let mut end = self.len;
        for _ in 0..MAX_TAGS {
            let room = end.saturating_sub(start);
            let cut = self.trailing_tag(end, room)?;
            match cut {
                Some(size) if size <= room => end -= size,
                _ => break,
            }
        }
        Ok(end)
    }

    /// The size of the tag ending at `end`, if there is one there. A read
    /// that comes back short (the file shrank) means no tag.
    fn trailing_tag(&mut self, end: u64, room: u64) -> io::Result<Option<u64>> {
        if room >= 32 {
            if let Some(footer) = self.read_exact_at(end - 32, 32)? {
                if footer.starts_with(b"APETAGEX") {
                    let size = u64::from(le_u32(&footer[12..16]));
                    let flags = le_u32(&footer[20..24]);
                    let header = if flags & (1 << 31) != 0 { 32 } else { 0 };
                    if size >= 32 {
                        return Ok(Some(size + header));
                    }
                }
            }
        }
        if room >= 128 && self.read_exact_at(end - 128, 3)?.as_deref() == Some(b"TAG") {
            return Ok(Some(128));
        }
        if room >= 15 {
            if let Some(tail) = self.read_exact_at(end - 15, 15)? {
                if &tail[6..] == b"LYRICS200" && tail[..6].iter().all(u8::is_ascii_digit) {
                    let size: u64 = std::str::from_utf8(&tail[..6])
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let total = size + 15;
                    if total <= room
                        && self.read_exact_at(end - total, 11)?.as_deref() == Some(b"LYRICSBEGIN")
                    {
                        return Ok(Some(total));
                    }
                }
            }
        }
        if room >= 20 {
            if let Some(footer) = self.read_exact_at(end - 10, 10)? {
                if footer.starts_with(b"3DI") {
                    // The footer mirrors the header.
                    let mut head = footer;
                    head[..3].copy_from_slice(b"ID3");
                    // `id3v2_size` counts the header and the footer.
                    if let Some(size) = sniff::id3v2_size(&head) {
                        if size <= room
                            && self.read_exact_at(end - size, 3)?.as_deref() == Some(b"ID3")
                        {
                            return Ok(Some(size));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// MP3 or ADTS: the frames from the first one to the trailing tags.
    fn frames(&mut self, start: u64, format: AudioFormat) -> io::Result<Planned> {
        let Some(first) = self.first_frame(start, format)? else {
            return Ok(Err(Skip::Malformed));
        };
        let end = self.strip_trailing_tags(first)?;
        Ok(ranges(
            format,
            Vec::new(),
            std::iter::once(first..end).collect(),
        ))
    }

    /// The first offset in the window after `start` where a frame header is
    /// followed by more of the same stream.
    fn first_frame(&mut self, start: u64, format: AudioFormat) -> io::Result<Option<u64>> {
        let w = self.read_at(start, FRAME_WINDOW)?;
        for i in 0..w.len().saturating_sub(1) {
            if w[i] != 0xFF || w[i + 1] & 0xE0 != 0xE0 {
                continue;
            }
            let at = start + i as u64;
            let lone_ok = i == 0;
            let found = match format {
                AudioFormat::Adts => match w.get(i..i + 7).and_then(AdtsHeader::parse) {
                    Some(first) => {
                        let next = |b: &[u8]| {
                            AdtsHeader::parse(b)
                                .filter(|h| h.same_stream(&first))
                                .map(|h| h.frame_len)
                        };
                        self.frames_follow(at + first.frame_len, 7, next, lone_ok)?
                    }
                    None => false,
                },
                _ => match w.get(i..i + 4).and_then(MpegHeader::parse) {
                    Some(first) => {
                        let next = |b: &[u8]| {
                            MpegHeader::parse(b)
                                .filter(|h| h.same_stream(&first))
                                .map(|h| h.frame_len)
                        };
                        self.frames_follow(at + first.frame_len, 4, next, lone_ok)?
                    }
                    None => false,
                },
            };
            if found {
                return Ok(Some(at));
            }
        }
        Ok(None)
    }

    /// The sniffer's frame check: [`FRAMES_TO_CONFIRM`] more frames of the
    /// same stream, or the end of the file after at least one (or at once,
    /// if `lone_ok`).
    fn frames_follow(
        &mut self,
        mut at: u64,
        header_len: u64,
        next: impl Fn(&[u8]) -> Option<u64>,
        lone_ok: bool,
    ) -> io::Result<bool> {
        for confirmed in 0..FRAMES_TO_CONFIRM {
            if at + header_len > self.len {
                return Ok(confirmed > 0 || lone_ok);
            }
            match next(&self.read_at(at, header_len)?) {
                Some(len) => at += len,
                None => return Ok(false),
            }
        }
        Ok(true)
    }

    /// RIFF / RIFX / RF64 / BW64 WAVE: `fmt ` body, then the `data` body.
    fn wave(&mut self, start: u64) -> io::Result<Planned> {
        let form = self.read_at(start, 12)?;
        if form.len() < 12 {
            return Ok(Err(Skip::Truncated));
        }
        let big = &form[..4] == b"RIFX";
        let rf64 = matches!(&form[..4], b"RF64" | b"BW64");
        let u32_at = |b: &[u8]| {
            let b: [u8; 4] = b.try_into().expect("4 bytes");
            if big {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            }
        };
        let (mut fmt, mut data, mut ds64_data) = (None::<Vec<u8>>, None::<Range<u64>>, None);
        let mut at = start + 12;
        for _ in 0..MAX_CHUNKS {
            if fmt.is_some() && data.is_some() {
                break;
            }
            let header = self.read_at(at, 8)?;
            if header.len() < 8 {
                if !header.is_empty() {
                    return Ok(Err(Skip::Truncated));
                }
                break;
            }
            let size = u32_at(&header[4..8]);
            let body = at + 8;
            let mut next = body + u64::from(size) + u64::from(size & 1);
            match &header[..4] {
                b"ds64" if rf64 => {
                    let ds64 = self.read_at(body, 16)?;
                    if ds64.len() < 16 {
                        return Ok(Err(Skip::Truncated));
                    }
                    ds64_data = Some(le_u64(&ds64[8..16]));
                }
                b"fmt " if fmt.is_none() => {
                    if size > MAX_FMT {
                        return Ok(Err(Skip::Malformed));
                    }
                    let bytes = self.read_at(body, u64::from(size))?;
                    if bytes.len() < size as usize {
                        return Ok(Err(Skip::Truncated));
                    }
                    fmt = Some(bytes);
                }
                b"data" if data.is_none() => {
                    let real = match (size, ds64_data) {
                        (u32::MAX, Some(real)) if rf64 => real,
                        // Left by streaming writers: where the audio ends
                        // isn't guessed.
                        (0 | u32::MAX, _) => return Ok(Err(Skip::Malformed)),
                        (size, _) => u64::from(size),
                    };
                    let Some(end) = body.checked_add(real) else {
                        return Ok(Err(Skip::Malformed));
                    };
                    if end > self.len {
                        return Ok(Err(Skip::Truncated));
                    }
                    data = Some(body..end);
                    next = end + (real & 1);
                }
                _ => {}
            }
            at = next;
        }
        match (fmt, data) {
            (Some(fmt), Some(data)) => {
                let head = [&(fmt.len() as u64).to_le_bytes()[..], &fmt].concat();
                Ok(ranges(AudioFormat::Wave, head, vec![data]))
            }
            _ => Ok(Err(Skip::Malformed)),
        }
    }

    /// AIFF / AIFC: 22 bytes of `COMM`, then the `SSND` samples.
    fn aiff(&mut self, start: u64) -> io::Result<Planned> {
        let form = self.read_at(start, 12)?;
        if form.len() < 12 {
            return Ok(Err(Skip::Truncated));
        }
        let aifc = &form[8..12] == b"AIFC";
        let (mut comm, mut samples) = (None::<Vec<u8>>, None::<Range<u64>>);
        let mut at = start + 12;
        for _ in 0..MAX_CHUNKS {
            if comm.is_some() && samples.is_some() {
                break;
            }
            let header = self.read_at(at, 8)?;
            if header.len() < 8 {
                if !header.is_empty() {
                    return Ok(Err(Skip::Truncated));
                }
                break;
            }
            let size = u64::from(be_u32(&header[4..8]));
            let body = at + 8;
            let end = body + size;
            match &header[..4] {
                b"COMM" if comm.is_none() => {
                    let need = if aifc { 22 } else { 18 };
                    if size < need {
                        return Ok(Err(Skip::Malformed));
                    }
                    let mut bytes = self.read_at(body, need)?;
                    if bytes.len() < need as usize {
                        return Ok(Err(Skip::Truncated));
                    }
                    if !aifc {
                        bytes.extend_from_slice(b"NONE");
                    }
                    comm = Some(bytes);
                }
                b"SSND" if samples.is_none() => {
                    if size < 8 {
                        return Ok(Err(Skip::Malformed));
                    }
                    if end > self.len {
                        return Ok(Err(Skip::Truncated));
                    }
                    let Some(fields) = self.read_exact_at(body, 8)? else {
                        return Ok(Err(Skip::Truncated));
                    };
                    let offset = u64::from(be_u32(&fields[..4]));
                    if offset > size - 8 {
                        return Ok(Err(Skip::Malformed));
                    }
                    samples = Some(body + 8 + offset..end);
                }
                _ => {}
            }
            at = end + (size & 1);
        }
        match (comm, samples) {
            (Some(comm), Some(samples)) => Ok(ranges(AudioFormat::Aiff, comm, vec![samples])),
            _ => Ok(Err(Skip::Malformed)),
        }
    }

    /// FLAC: every frame, after the metadata blocks and before the trailing
    /// tags.
    fn flac(&mut self, start: u64) -> io::Result<Planned> {
        // STREAMINFO first: its 4-byte block header, 10 bytes of block and
        // frame sizes, then the 8 bytes hashed.
        let Some(info) = self.read_exact_at(start + 4, 4 + 18)? else {
            return Ok(Err(Skip::Truncated));
        };
        let info_size = u32::from_be_bytes([0, info[1], info[2], info[3]]);
        if info[0] & 0x7F != 0 || info_size < 34 {
            return Ok(Err(Skip::Malformed));
        }
        let head = info[14..22].to_vec();
        let mut at = start + 4; // `fLaC`
        let mut last = false;
        for _ in 0..MAX_CHUNKS {
            let header = self.read_at(at, 4)?;
            if header.len() < 4 {
                return Ok(Err(Skip::Truncated));
            }
            last = header[0] & 0x80 != 0;
            let size = u64::from(be_u32(&[0, header[1], header[2], header[3]]));
            at += 4 + size;
            if at > self.len {
                return Ok(Err(Skip::Truncated));
            }
            if last {
                break;
            }
        }
        if !last {
            return Ok(Err(Skip::Malformed));
        }
        let sync = self.read_at(at, 2)?;
        if sync.len() < 2 || sync[0] != 0xFF || sync[1] & 0xFE != 0xF8 {
            return Ok(Err(Skip::Malformed));
        }
        let end = self.strip_trailing_tags(at)?;
        Ok(ranges(
            AudioFormat::Flac,
            head,
            std::iter::once(at..end).collect(),
        ))
    }

    /// MP4: the bodies of the top-level `mdat` boxes.
    fn mp4(&mut self, start: u64) -> io::Result<Planned> {
        let mut mdats = Vec::new();
        let mut moovs = Vec::new();
        let mut at = start;
        for _ in 0..MAX_CHUNKS {
            if at == self.len {
                break;
            }
            let header = self.read_at(at, 16)?;
            if header.len() < 8 {
                return Ok(Err(Skip::Truncated));
            }
            let (size, header_len) = match be_u32(&header[..4]) {
                0 => (self.len - at, 8),
                1 => match header.get(8..16) {
                    Some(large) => (be_u64(large), 16),
                    None => return Ok(Err(Skip::Truncated)),
                },
                n => (u64::from(n), 8),
            };
            if size < header_len {
                return Ok(Err(Skip::Malformed));
            }
            let end = match at.checked_add(size) {
                Some(end) if end <= self.len => end,
                _ => return Ok(Err(Skip::Truncated)),
            };
            match &header[4..8] {
                b"mdat" => mdats.push(at + header_len..end),
                b"moov" => moovs.push(at + header_len..end),
                _ => {}
            }
            at = end;
        }
        if at != self.len || mdats.is_empty() {
            return Ok(Err(Skip::Malformed));
        }
        let mut head = Vec::new();
        for moov in moovs {
            if let Err(skip) = self.sound_sample_entries(b"moov", moov, 0, &mut head)? {
                return Ok(Err(skip));
            }
        }
        if head.is_empty() {
            return Ok(Err(Skip::Malformed));
        }
        Ok(ranges(AudioFormat::Mp4, head, mdats))
    }

    /// The boxes directly inside `body`, as (type, body range).
    fn boxes_in(&mut self, body: Range<u64>) -> io::Result<Result<Vec<Child>, Skip>> {
        let mut found = Vec::new();
        let mut at = body.start;
        for _ in 0..MAX_CHUNKS {
            if at == body.end {
                return Ok(Ok(found));
            }
            if body.end - at < 8 {
                return Ok(Err(Skip::Malformed));
            }
            let Some(header) = self.read_exact_at(at, 8)? else {
                return Ok(Err(Skip::Truncated));
            };
            let (size, header_len) = match be_u32(&header[..4]) {
                0 => (body.end - at, 8),
                1 => match self.read_exact_at(at + 8, 8)? {
                    Some(large) => (be_u64(&large), 16),
                    None => return Ok(Err(Skip::Truncated)),
                },
                n => (u64::from(n), 8),
            };
            let end = match at.checked_add(size) {
                Some(end) if size >= header_len && end <= body.end => end,
                _ => return Ok(Err(Skip::Malformed)),
            };
            let kind: [u8; 4] = header[4..8].try_into().expect("4 bytes");
            found.push((kind, at + header_len..end));
            at = end;
        }
        Ok(Err(Skip::Malformed))
    }

    /// Appends, for every sound track under `body` (a `moov`, or a box on
    /// the way down to `stsd`), the length and body of its `stsd`.
    fn sound_sample_entries(
        &mut self,
        kind: &[u8; 4],
        body: Range<u64>,
        depth: usize,
        head: &mut Vec<u8>,
    ) -> io::Result<Result<(), Skip>> {
        if depth >= MP4_DEPTH {
            return Ok(Ok(()));
        }
        let children = match self.boxes_in(body)? {
            Ok(children) => children,
            Err(skip) => return Ok(Err(skip)),
        };
        // In `mdia`, only a track whose handler says `soun` counts: no
        // `hdlr`, or one too short to name a handler, and the track is
        // skipped. (QuickTime's `minf` can hold a data-handler `hdlr` too;
        // that one isn't it.)
        if kind == b"mdia" {
            let hdlr = children.iter().find(|(child, _)| child == b"hdlr");
            let Some((_, hdlr)) = hdlr else {
                return Ok(Ok(()));
            };
            if hdlr.end - hdlr.start < 12 {
                return Ok(Ok(()));
            }
            let Some(fields) = self.read_exact_at(hdlr.start, 12)? else {
                return Ok(Err(Skip::Truncated));
            };
            if &fields[8..12] != b"soun" {
                return Ok(Ok(()));
            }
        }
        for (child_kind, child) in children {
            // Only down the path moov/trak/mdia/minf/stbl/stsd.
            match (kind, &child_kind) {
                (b"stbl", b"stsd") => {
                    let len = child.end - child.start;
                    if len > MAX_STSD {
                        return Ok(Err(Skip::Malformed));
                    }
                    let Some(bytes) = self.read_exact_at(child.start, len)? else {
                        return Ok(Err(Skip::Truncated));
                    };
                    head.extend_from_slice(&len.to_le_bytes());
                    head.extend_from_slice(&bytes);
                }
                (b"moov", b"trak")
                | (b"trak", b"mdia")
                | (b"mdia", b"minf")
                | (b"minf", b"stbl") => {
                    if let Err(skip) =
                        self.sound_sample_entries(&child_kind, child, depth + 1, head)?
                    {
                        return Ok(Err(skip));
                    }
                }
                _ => {}
            }
        }
        Ok(Ok(()))
    }
}

/// A ranges plan, unless there's no audio in it.
fn ranges(format: AudioFormat, head: Vec<u8>, ranges: Vec<Range<u64>>) -> Planned {
    let ranges: Vec<_> = ranges.into_iter().filter(|r| r.start < r.end).collect();
    if ranges.is_empty() {
        return Err(Skip::Malformed);
    }
    Ok(Plan::Ranges {
        format,
        head,
        ranges,
    })
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().expect("4 bytes"))
}

fn le_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().expect("8 bytes"))
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes(b.try_into().expect("4 bytes"))
}

fn be_u64(b: &[u8]) -> u64 {
    u64::from_be_bytes(b.try_into().expect("8 bytes"))
}

/// Takes the file's bytes in order and hashes the audio in them.
pub(crate) enum Sink {
    Ranges(Box<RangeSink>),
    Ogg(Box<OggSink>),
    Skip(Skip),
}

impl Sink {
    pub(crate) fn new(plan: Plan) -> Sink {
        match plan {
            Plan::Ranges {
                format,
                head,
                ranges,
            } => {
                let mut hasher = format.hasher();
                hasher.update(&head);
                Sink::Ranges(Box::new(RangeSink {
                    format,
                    hasher,
                    ranges,
                    next: 0,
                }))
            }
            Plan::Ogg { format, start } => Sink::Ogg(Box::new(OggSink::new(format, start))),
            Plan::Skip(skip) => Sink::Skip(skip),
        }
    }

    /// The file's bytes from `offset`; each call continues the last.
    pub(crate) fn feed(&mut self, offset: u64, bytes: &[u8]) {
        match self {
            Sink::Ranges(sink) => sink.feed(offset, bytes),
            Sink::Ogg(sink) => sink.feed(offset, bytes),
            Sink::Skip(_) => {}
        }
    }

    /// The audio_hash, once the whole file (`len` bytes) went through.
    pub(crate) fn finish(self, len: u64) -> Result<AudioHash, Skip> {
        match self {
            Sink::Ranges(sink) => sink.finish(len),
            Sink::Ogg(sink) => sink.finish(),
            Sink::Skip(skip) => Err(skip),
        }
    }
}

pub(crate) struct RangeSink {
    format: AudioFormat,
    hasher: blake3::Hasher,
    ranges: Vec<Range<u64>>,
    /// The first range not yet fully hashed.
    next: usize,
}

impl RangeSink {
    fn feed(&mut self, offset: u64, bytes: &[u8]) {
        let end = offset + bytes.len() as u64;
        while let Some(range) = self.ranges.get(self.next) {
            let from = range.start.max(offset);
            let to = range.end.min(end);
            if from < to {
                self.hasher
                    .update(&bytes[(from - offset) as usize..(to - offset) as usize]);
            }
            if range.end <= end {
                self.next += 1;
            } else {
                break;
            }
        }
    }

    fn finish(self, len: u64) -> Result<AudioHash, Skip> {
        // The file shrank after it was planned: part of the audio is gone.
        if self.next < self.ranges.len() || self.ranges.last().is_some_and(|r| r.end > len) {
            return Err(Skip::Truncated);
        }
        Ok(AudioHash {
            format: self.format,
            digest: *self.hasher.finalize().as_bytes(),
        })
    }
}

/// Where the Ogg demuxer is within a page.
enum OggState {
    /// Collecting the 27-byte page header.
    Header,
    /// Collecting the segment table.
    Segments,
    /// Inside the page body, in segment `index`, with `left` bytes to go.
    Body { index: usize, left: usize },
    /// After the end-of-stream page, on bytes that aren't a page: ignored.
    Trailing,
}

/// Reassembles packets from Ogg pages and hashes all but the second.
pub(crate) struct OggSink {
    format: AudioFormat,
    hasher: blake3::Hasher,
    start: u64,
    state: OggState,
    /// The page header, then the segment table, as they arrive.
    page: Vec<u8>,
    serial: Option<u32>,
    /// Packets finished so far.
    packets: u64,
    /// Bytes of the packet in progress.
    packet_len: u64,
    /// Whether the last segment seen left a packet open (lacing 255).
    open: bool,
    last_granule: Option<u64>,
    eos: bool,
    failed: Option<Skip>,
}

impl OggSink {
    fn new(format: AudioFormat, start: u64) -> OggSink {
        OggSink {
            format,
            hasher: format.hasher(),
            start,
            state: OggState::Header,
            page: Vec::with_capacity(27 + 255),
            serial: None,
            packets: 0,
            packet_len: 0,
            open: false,
            last_granule: None,
            eos: false,
            failed: None,
        }
    }

    fn feed(&mut self, offset: u64, bytes: &[u8]) {
        let skip = self.start.saturating_sub(offset).min(bytes.len() as u64) as usize;
        let mut bytes = &bytes[skip..];
        while !bytes.is_empty() && self.failed.is_none() {
            match self.state {
                OggState::Trailing => return,
                OggState::Header => {
                    let take = (27 - self.page.len()).min(bytes.len());
                    self.page.extend_from_slice(&bytes[..take]);
                    bytes = &bytes[take..];
                    let have = self.page.len();
                    if !b"OggS".starts_with(&self.page[..have.min(4)]) {
                        if self.eos {
                            self.state = OggState::Trailing;
                        } else {
                            self.failed = Some(Skip::Malformed);
                        }
                        return;
                    }
                    if have == 27 {
                        self.page_header();
                    }
                }
                OggState::Segments => {
                    let want = 27 + self.page[26] as usize;
                    let take = (want - self.page.len()).min(bytes.len());
                    self.page.extend_from_slice(&bytes[..take]);
                    bytes = &bytes[take..];
                    if self.page.len() == want {
                        self.state = OggState::Body {
                            index: 0,
                            left: self.page[27] as usize,
                        };
                        self.settle();
                    }
                }
                OggState::Body { index, left } => {
                    let take = left.min(bytes.len());
                    if self.packets != 1 {
                        self.hasher.update(&bytes[..take]);
                    }
                    self.packet_len += take as u64;
                    bytes = &bytes[take..];
                    self.state = OggState::Body {
                        index,
                        left: left - take,
                    };
                    self.settle();
                }
            }
        }
    }

    /// Checks a whole page header and moves on to its segment table.
    fn page_header(&mut self) {
        let h = &self.page;
        if h[4] != 0 {
            self.failed = Some(Skip::Malformed);
            return;
        }
        let serial = le_u32(&h[14..18]);
        if self.serial.is_some_and(|s| s != serial) {
            // Multiplexed or chained streams.
            self.failed = Some(Skip::UnsupportedFormat);
            return;
        }
        self.serial = Some(serial);
        let continued = h[5] & 0x01 != 0;
        if continued != self.open {
            self.failed = Some(Skip::Malformed);
            return;
        }
        if h[26] == 0 {
            self.end_page();
        } else {
            self.state = OggState::Segments;
        }
    }

    /// Ends every finished segment at the current position, then the page
    /// once its last segment is done.
    fn settle(&mut self) {
        while let OggState::Body { index, left: 0 } = self.state {
            let lacing = self.page[27 + index];
            self.open = lacing == 255;
            if !self.open {
                if self.packets != 1 {
                    self.hasher.update(&self.packet_len.to_le_bytes());
                }
                self.packets += 1;
                self.packet_len = 0;
            }
            let segments = self.page[26] as usize;
            if index + 1 < segments {
                self.state = OggState::Body {
                    index: index + 1,
                    left: self.page[28 + index] as usize,
                };
            } else {
                self.end_page();
            }
        }
    }

    fn end_page(&mut self) {
        let granule = le_u64(&self.page[6..14]);
        if granule != u64::MAX {
            self.last_granule = Some(granule);
        }
        if self.page[5] & 0x04 != 0 {
            self.eos = true;
        }
        self.page.clear();
        self.state = OggState::Header;
    }

    fn finish(mut self) -> Result<AudioHash, Skip> {
        if let Some(skip) = self.failed {
            return Err(skip);
        }
        let between_pages = match self.state {
            OggState::Header => self.page.is_empty(),
            OggState::Trailing => true,
            OggState::Segments | OggState::Body { .. } => false,
        };
        if !between_pages || self.open || !self.eos {
            return Err(Skip::Truncated);
        }
        if self.packets < 2 {
            return Err(Skip::Malformed);
        }
        self.hasher
            .update(&self.last_granule.unwrap_or(0).to_le_bytes());
        Ok(AudioHash {
            format: self.format,
            digest: *self.hasher.finalize().as_bytes(),
        })
    }
}
