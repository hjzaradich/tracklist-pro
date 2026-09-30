//! Detects a file's audio format from its bytes, not its name
//! (ROADMAP §5.5): a "`.wav`" can be an MP3. The result is
//! `file.sniffed_format` (ROADMAP §2).
//!
//! [`sniff`] is a pure function over any `Read + Seek`; [`sniff_path`] opens
//! a file read-only and calls it. Reads are bounded: a tag at the start is
//! skipped by its size field, then at most a few KB are read, and container
//! structure (RIFF/AIFF chunks, MP4 boxes) is walked by seeking past bodies
//! and reading only headers. The audio itself is never read, so a 2 GB file
//! costs the same as a 2 MB one.
//!
//! Only the container and stream type are decided here. Codec details (AAC
//! vs ALAC inside an MP4, PCM vs MP3 inside a WAV) belong to the audio
//! properties step (1aB-3).
//!
//! Damaged input never panics: a zero-byte file is [`Problem::Empty`], a file
//! that ends inside a header this module recognized is
//! [`Problem::Truncated`], and bytes it can't place are
//! [`SniffedFormat::Unknown`].

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// What a file's bytes say it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SniffedFormat {
    /// MPEG audio frames (MP3, and the rare MP1/MP2), with or without an ID3
    /// tag in front.
    Mp3,
    /// A RIFF `WAVE`, whatever codec its format tag names.
    Wav,
    /// A 64-bit WAVE (`RF64` or `BW64`), for files over 4 GB.
    Rf64,
    Aiff,
    /// Compressed (or little-endian) AIFF.
    Aifc,
    Flac,
    /// An MP4 / QuickTime container (M4A: AAC or ALAC).
    Mp4,
    /// Raw AAC in ADTS frames (a bare `.aac`).
    Adts,
    OggVorbis,
    OggOpus,
    /// An Ogg stream holding another codec (FLAC, Speex, …) or one whose
    /// first packet couldn't be read.
    Ogg,
    /// Windows Media (WMA) in an ASF container.
    Asf,
    WavPack,
    Musepack,
    /// Monkey's Audio.
    Ape,
    /// Nothing this module recognizes.
    Unknown,
}

impl SniffedFormat {
    /// The stable name stored in `file.sniffed_format`.
    pub fn as_str(self) -> &'static str {
        match self {
            SniffedFormat::Mp3 => "mp3",
            SniffedFormat::Wav => "wav",
            SniffedFormat::Rf64 => "rf64",
            SniffedFormat::Aiff => "aiff",
            SniffedFormat::Aifc => "aifc",
            SniffedFormat::Flac => "flac",
            SniffedFormat::Mp4 => "mp4",
            SniffedFormat::Adts => "adts",
            SniffedFormat::OggVorbis => "ogg_vorbis",
            SniffedFormat::OggOpus => "ogg_opus",
            SniffedFormat::Ogg => "ogg",
            SniffedFormat::Asf => "asf",
            SniffedFormat::WavPack => "wavpack",
            SniffedFormat::Musepack => "musepack",
            SniffedFormat::Ape => "ape",
            SniffedFormat::Unknown => "unknown",
        }
    }

    /// File extensions (lowercase, no dot) that honestly name this format.
    /// Empty for [`SniffedFormat::Unknown`].
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            SniffedFormat::Mp3 => &["mp3", "mp2", "mp1", "mpga"],
            SniffedFormat::Wav | SniffedFormat::Rf64 => &["wav", "wave", "rf64", "bwf"],
            // Apple and others use either extension for either kind.
            SniffedFormat::Aiff | SniffedFormat::Aifc => &["aif", "aiff", "aifc"],
            SniffedFormat::Flac => &["flac"],
            SniffedFormat::Mp4 => &["m4a", "mp4", "m4b", "m4p", "alac", "aac"],
            SniffedFormat::Adts => &["aac"],
            SniffedFormat::OggVorbis => &["ogg", "oga"],
            SniffedFormat::OggOpus => &["opus", "ogg", "oga"],
            SniffedFormat::Ogg => &["ogg", "oga", "spx"],
            SniffedFormat::Asf => &["wma", "asf"],
            SniffedFormat::WavPack => &["wv"],
            SniffedFormat::Musepack => &["mpc", "mp+", "mpp"],
            SniffedFormat::Ape => &["ape"],
            SniffedFormat::Unknown => &[],
        }
    }
}

/// Why a file can't be what it looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Problem {
    /// Zero bytes: a failed download.
    Empty,
    /// The file ends inside a tag or header, a chunk or box claims more
    /// bytes than the file has, or the chunk structure breaks off: an
    /// interrupted download or copy.
    Truncated,
    /// An MP4 whose top-level boxes are all there but include no `moov`, the
    /// index a player needs (an interrupted download that wrote `mdat` first).
    NoMoov,
}

/// The result of sniffing one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sniff {
    pub format: SniffedFormat,
    pub problem: Option<Problem>,
    /// The format is known and the extension isn't one of its
    /// [`SniffedFormat::extensions`] (letter case ignored), e.g. MP3 bytes
    /// in a `.wav`. Always false for [`SniffedFormat::Unknown`]: there's no
    /// format to disagree with.
    pub extension_disagrees: bool,
}

/// How many bytes are looked at for magic numbers and MPEG frame sync, from
/// where the audio starts (after any tag).
const WINDOW: u64 = 4096;
/// Stacked ID3v2 tags skipped before giving up (some taggers prepend a new
/// one without removing the old).
const MAX_TAGS: usize = 8;
/// Chunks or boxes walked before giving up.
const MAX_CHUNKS: usize = 64;
/// MPEG frames checked after the first one, to tell real audio from bytes
/// that happen to look like a frame header.
const FRAMES_TO_CONFIRM: usize = 2;

/// Sniffs the bytes in `reader`. `extension` is the file's extension (no
/// dot, any letter case), or `None` if it has none.
///
/// Reads a bounded number of bytes wherever the reader is positioned; the
/// position afterwards is unspecified. I/O errors are returned; a file that
/// merely ends early is a [`Problem`], not an error.
pub fn sniff<R: Read + Seek>(reader: &mut R, extension: Option<&str>) -> io::Result<Sniff> {
    let len = reader.seek(SeekFrom::End(0))?;
    let (format, problem) = if len == 0 {
        (SniffedFormat::Unknown, Some(Problem::Empty))
    } else {
        Sniffer { r: reader, len }.run()?
    };
    let extension_disagrees = format != SniffedFormat::Unknown
        && !extension.is_some_and(|ext| {
            format
                .extensions()
                .iter()
                .any(|known| known.eq_ignore_ascii_case(ext))
        });
    Ok(Sniff {
        format,
        problem,
        extension_disagrees,
    })
}

/// Opens `path` read-only and sniffs it. `path` must be absolute; on Windows
/// it's opened in the path model's `\\?\` form (ROADMAP §5.6), so a name with
/// a trailing dot or space opens that exact file and never a sibling.
pub fn sniff_path(path: &Path) -> io::Result<Sniff> {
    // `File::open` asks for read access only.
    let mut file = File::open(open_form(path)?)?;
    let extension = path.extension().and_then(|e| e.to_str());
    sniff(&mut file, extension)
}

#[cfg(windows)]
fn open_form(path: &Path) -> io::Result<std::path::PathBuf> {
    crate::paths::verbatim_absolute(path)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

#[cfg(not(windows))]
fn open_form(path: &Path) -> io::Result<std::path::PathBuf> {
    Ok(path.to_path_buf())
}

type Outcome = (SniffedFormat, Option<Problem>);

struct Sniffer<'r, R> {
    r: &'r mut R,
    len: u64,
}

impl<R: Read + Seek> Sniffer<'_, R> {
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

    fn run(&mut self) -> io::Result<Outcome> {
        let mut at = 0u64;
        let mut tagged = false;
        for _ in 0..MAX_TAGS {
            let head = self.read_at(at, 10)?;
            match id3v2_size(&head) {
                Some(size) => {
                    tagged = true;
                    at += size;
                    if at >= self.len {
                        // The tag runs to (or past) the end: no audio.
                        return Ok((SniffedFormat::Unknown, Some(Problem::Truncated)));
                    }
                }
                None if head.len() < 10 && b"ID3".starts_with(&head[..head.len().min(3)]) => {
                    // "ID3" (or a prefix of it) with the header cut short.
                    return Ok((SniffedFormat::Unknown, Some(Problem::Truncated)));
                }
                None => break,
            }
        }
        let window = self.read_at(at, WINDOW)?;
        if let Some(outcome) = self.by_magic(at, &window)? {
            return Ok(outcome);
        }
        if let Some(format) = self.by_frames(at, &window)? {
            return Ok((format, None));
        }
        let problem = tagged
            .then_some(Problem::Truncated)
            .filter(|_| window.len() < 4);
        Ok((SniffedFormat::Unknown, problem))
    }

    /// Formats with a signature at `at`, where `window` starts.
    fn by_magic(&mut self, at: u64, w: &[u8]) -> io::Result<Option<Outcome>> {
        use SniffedFormat as F;
        let cut = |format: F, need: usize| {
            let problem = (w.len() < need).then_some(Problem::Truncated);
            Some((format, problem))
        };
        let form_type = w.get(8..12);
        // A RIFF, RIFX, RF64/BW64 or FORM magic in a file too short to
        // hold its 12-byte form header gives Unknown + Truncated.
        let outcome = match w.get(..4) {
            Some(b"fLaC") => cut(F::Flac, 42), // magic + block header + STREAMINFO
            Some(b"RIFF") => match form_type {
                Some(b"WAVE") => Some((F::Wav, self.walk_chunks(at, Endian::Little)?)),
                Some(_) => None,
                None => Some((F::Unknown, Some(Problem::Truncated))),
            },
            // RIFX: a big-endian RIFF.
            Some(b"RIFX") => match form_type {
                Some(b"WAVE") => Some((F::Wav, self.walk_chunks(at, Endian::BigRiff)?)),
                Some(_) => None,
                None => Some((F::Unknown, Some(Problem::Truncated))),
            },
            Some(b"RF64" | b"BW64") => match form_type {
                Some(b"WAVE") => Some((F::Rf64, self.walk_chunks(at, Endian::Little)?)),
                Some(_) => None,
                None => Some((F::Unknown, Some(Problem::Truncated))),
            },
            Some(b"FORM") => match form_type {
                Some(b"AIFF") => Some((F::Aiff, self.walk_chunks(at, Endian::Big)?)),
                Some(b"AIFC") => Some((F::Aifc, self.walk_chunks(at, Endian::Big)?)),
                Some(_) => None,
                None => Some((F::Unknown, Some(Problem::Truncated))),
            },
            Some(b"OggS") => Some(ogg(w)),
            Some(b"wvpk") => cut(F::WavPack, 32),
            Some(b"MPCK") => cut(F::Musepack, 8),
            Some(b"MAC ") => cut(F::Ape, 8),
            _ if w.starts_with(b"MP+") => cut(F::Musepack, 8),
            _ if w.starts_with(&ASF_HEADER) => cut(F::Asf, 30),
            // Every M4A starts with `ftyp`; old QuickTime files may start
            // with `moov` or `mdat`. The box walk says whether it's whole.
            _ if w.get(4..8).is_some_and(is_mp4_start) => Some((F::Mp4, self.walk_boxes(at)?)),
            // A leading padding box is QuickTime too, but four letters like
            // `free` turn up in text, so it counts only if `moov` is there.
            _ if w.get(4..8).is_some_and(is_padding_box) => match self.walk_boxes(at)? {
                None => Some((F::Mp4, None)),
                Some(_) => None,
            },
            _ => None,
        };
        Ok(outcome)
    }

    /// MPEG audio or ADTS: a run of consistent frame headers, starting
    /// anywhere in the window (some files have junk or padding first).
    fn by_frames(&mut self, at: u64, w: &[u8]) -> io::Result<Option<SniffedFormat>> {
        for i in 0..w.len().saturating_sub(1) {
            if w[i] != 0xFF || w[i + 1] & 0xE0 != 0xE0 {
                continue;
            }
            let start = at + i as u64;
            // A lone frame cut off by the end of the file only counts where
            // the audio should start, not at some offset inside junk.
            let lone_ok = i == 0;
            if let Some(first) = w.get(i..i + 4).and_then(MpegHeader::parse) {
                let next = |b: &[u8]| {
                    MpegHeader::parse(b)
                        .filter(|h| h.same_stream(&first))
                        .map(|h| h.frame_len)
                };
                if self.frames_follow(start + first.frame_len, 4, next, lone_ok)? {
                    return Ok(Some(SniffedFormat::Mp3));
                }
            }
            if let Some(first) = w.get(i..i + 7).and_then(AdtsHeader::parse) {
                let next = |b: &[u8]| {
                    AdtsHeader::parse(b)
                        .filter(|h| h.same_stream(&first))
                        .map(|h| h.frame_len)
                };
                if self.frames_follow(start + first.frame_len, 7, next, lone_ok)? {
                    return Ok(Some(SniffedFormat::Adts));
                }
            }
        }
        Ok(None)
    }

    /// Whether the frames from `at` on parse as the same stream as the one
    /// before them. `next` parses a header and gives its frame length. A
    /// file that ends before [`FRAMES_TO_CONFIRM`] frames still counts if at
    /// least one frame agreed, or `lone_ok` and none were left to check.
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
            let header = self.read_at(at, header_len)?;
            match next(&header) {
                Some(len) => at += len,
                None => return Ok(false),
            }
        }
        Ok(true)
    }

    /// Walks RIFF/AIFF chunks after the 12-byte form header until the
    /// format chunk and the audio chunk are both seen. Returns the problem,
    /// if any. Chunk order isn't assumed (AIFF allows any).
    fn walk_chunks(&mut self, form_at: u64, endian: Endian) -> io::Result<Option<Problem>> {
        let (fmt_id, data_id): (&[u8], &[u8]) = match endian {
            Endian::Little | Endian::BigRiff => (b"fmt ", b"data"),
            Endian::Big => (b"COMM", b"SSND"),
        };
        let (mut seen_fmt, mut seen_data) = (false, false);
        let mut at = form_at + 12;
        for _ in 0..MAX_CHUNKS {
            let header = self.read_at(at, 8)?;
            if header.len() < 8 {
                // The file ended between chunks. Fine once the audio is
                // there (a missing format chunk is for the full reader);
                // otherwise the audio was cut off.
                return Ok((!seen_data).then_some(Problem::Truncated));
            }
            let id = &header[..4];
            let size = endian.u32(&header[4..8]);
            let body_end = at + 8 + u64::from(size);
            seen_fmt |= id == fmt_id;
            if id == data_id {
                seen_data = true;
                // RF64 keeps the real size in `ds64` and writes 0xFFFFFFFF
                // here; some recorders leave 0 in unfinished files. Either
                // way the size can't be checked or skipped.
                if size == u32::MAX || size == 0 {
                    return Ok(None);
                }
            }
            if body_end > self.len {
                return Ok(Some(Problem::Truncated));
            }
            if seen_fmt && seen_data {
                return Ok(None);
            }
            // Bodies are padded to an even length (not every writer does).
            at = body_end + u64::from(size & 1);
        }
        // Too many chunks to follow: leave the verdict to the full reader.
        Ok(None)
    }

    /// Walks the top-level MP4 boxes (headers only) to the end of the file:
    /// every box must fit, and one must be `moov`.
    fn walk_boxes(&mut self, start: u64) -> io::Result<Option<Problem>> {
        let no_moov = |seen: bool| (!seen).then_some(Problem::NoMoov);
        let mut seen_moov = false;
        let mut at = start;
        for _ in 0..MAX_CHUNKS {
            if at == self.len {
                return Ok(no_moov(seen_moov));
            }
            let header = self.read_at(at, 16)?;
            if header.len() < 8 {
                return Ok(Some(Problem::Truncated));
            }
            seen_moov |= &header[4..8] == b"moov";
            let (size, header_len) = match be_u32(&header[..4]) {
                // The box runs to the end of the file, so nothing follows it.
                0 => return Ok(no_moov(seen_moov)),
                1 => match header.get(8..16) {
                    Some(large) => (u64::from_be_bytes(large.try_into().expect("8 bytes")), 16),
                    None => return Ok(Some(Problem::Truncated)),
                },
                n => (u64::from(n), 8),
            };
            if size < header_len {
                // Not a box: the structure breaks off here.
                return Ok(Some(Problem::Truncated));
            }
            match at.checked_add(size) {
                Some(end) if end <= self.len => at = end,
                _ => return Ok(Some(Problem::Truncated)),
            }
        }
        // Too many boxes to follow: leave the verdict to the full reader.
        Ok(None)
    }
}

/// How a form's chunk sizes are stored, and which chunk ids it uses.
#[derive(Clone, Copy)]
enum Endian {
    /// RIFF / RF64 WAVE.
    Little,
    /// RIFX: a WAVE with big-endian sizes.
    BigRiff,
    /// AIFF / AIFC.
    Big,
}

impl Endian {
    fn u32(self, b: &[u8]) -> u32 {
        let b: [u8; 4] = b.try_into().expect("4 bytes");
        match self {
            Endian::Little => u32::from_le_bytes(b),
            Endian::BigRiff | Endian::Big => u32::from_be_bytes(b),
        }
    }
}

fn be_u32(b: &[u8]) -> u32 {
    Endian::Big.u32(b)
}

/// The size of an ID3v2 tag starting `head`, header (and footer) included,
/// if `head` is a valid ID3v2 header.
pub(crate) fn id3v2_size(head: &[u8]) -> Option<u64> {
    let head = head.get(..10)?;
    let valid = head.starts_with(b"ID3")
        && (2..=4).contains(&head[3])
        && head[4] != 0xFF
        && head[6..10].iter().all(|b| b & 0x80 == 0);
    if !valid {
        return None;
    }
    let body = head[6..10]
        .iter()
        .fold(0u64, |n, b| (n << 7) | u64::from(*b));
    let footer = if head[3] == 4 && head[5] & 0x10 != 0 {
        10
    } else {
        0
    };
    Some(10 + body + footer)
}

/// The GUID that starts every ASF file (the Header Object).
const ASF_HEADER: [u8; 16] = [
    0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C,
];

/// Box types an MP4 or QuickTime file with audio can start with.
fn is_mp4_start(kind: &[u8]) -> bool {
    matches!(kind, b"ftyp" | b"moov" | b"mdat")
}

/// Padding boxes a QuickTime file can start with.
fn is_padding_box(kind: &[u8]) -> bool {
    matches!(kind, b"free" | b"skip" | b"wide")
}

/// The codec of an Ogg stream, from its first page's first packet.
fn ogg(w: &[u8]) -> Outcome {
    let Some(&segments) = w.get(26) else {
        return (SniffedFormat::Ogg, Some(Problem::Truncated));
    };
    let packet = &w[(27 + segments as usize).min(w.len())..];
    if packet.starts_with(b"\x01vorbis") {
        (SniffedFormat::OggVorbis, None)
    } else if packet.starts_with(b"OpusHead") {
        (SniffedFormat::OggOpus, None)
    } else if packet.len() < 8 {
        (SniffedFormat::Ogg, Some(Problem::Truncated))
    } else {
        (SniffedFormat::Ogg, None)
    }
}

/// An MPEG audio frame header (MPEG-1, -2 or 2.5, layers I–III).
pub(crate) struct MpegHeader {
    version: u8,
    layer: u8,
    rate_index: u8,
    pub(crate) frame_len: u64,
}

impl MpegHeader {
    pub(crate) fn parse(b: &[u8]) -> Option<MpegHeader> {
        let h = u32::from_be_bytes(b.get(..4)?.try_into().ok()?);
        let version = ((h >> 19) & 3) as u8; // 0: 2.5, 1: reserved, 2: 2, 3: 1
        let layer = ((h >> 17) & 3) as u8; // 0: reserved, 1: III, 2: II, 3: I
        let bitrate_index = ((h >> 12) & 0xF) as usize;
        let rate_index = ((h >> 10) & 3) as u8;
        let padding = u64::from((h >> 9) & 1);
        let emphasis = h & 3;
        if h >> 21 != 0x7FF
            || version == 1
            || layer == 0
            || bitrate_index == 0 // free format: no length to check against
            || bitrate_index == 15
            || rate_index == 3
            || emphasis == 2
        {
            return None;
        }
        let mpeg1 = version == 3;
        const V1: [[u64; 14]; 3] = [
            [
                32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
            ],
            [
                32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
            ],
            [
                32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ],
        ];
        const V2_L1: [u64; 14] = [
            32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
        ];
        const V2_L23: [u64; 14] = [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
        let kbps = match (mpeg1, layer) {
            (true, 3) => V1[0][bitrate_index - 1],
            (true, 2) => V1[1][bitrate_index - 1],
            (true, _) => V1[2][bitrate_index - 1],
            (false, 3) => V2_L1[bitrate_index - 1],
            (false, _) => V2_L23[bitrate_index - 1],
        };
        let base_rate = [44_100u64, 48_000, 32_000][rate_index as usize];
        let rate = match version {
            3 => base_rate,
            2 => base_rate / 2,
            _ => base_rate / 4,
        };
        let bps = kbps * 1000;
        let frame_len = match layer {
            3 => (12 * bps / rate + padding) * 4,
            2 => 144 * bps / rate + padding,
            _ if mpeg1 => 144 * bps / rate + padding,
            _ => 72 * bps / rate + padding,
        };
        Some(MpegHeader {
            version,
            layer,
            rate_index,
            frame_len,
        })
    }

    pub(crate) fn same_stream(&self, other: &MpegHeader) -> bool {
        (self.version, self.layer, self.rate_index)
            == (other.version, other.layer, other.rate_index)
    }
}

/// An ADTS (raw AAC) frame header.
pub(crate) struct AdtsHeader {
    rate_index: u8,
    pub(crate) frame_len: u64,
}

impl AdtsHeader {
    pub(crate) fn parse(b: &[u8]) -> Option<AdtsHeader> {
        let b = b.get(..7)?;
        let layer = (b[1] >> 1) & 3;
        let rate_index = (b[2] >> 2) & 0xF;
        let frame_len = (u64::from(b[3] & 3) << 11) | (u64::from(b[4]) << 3) | u64::from(b[5] >> 5);
        if b[0] != 0xFF || b[1] & 0xF0 != 0xF0 || layer != 0 || rate_index > 12 || frame_len < 7 {
            return None;
        }
        Some(AdtsHeader {
            rate_index,
            frame_len,
        })
    }

    pub(crate) fn same_stream(&self, other: &AdtsHeader) -> bool {
        self.rate_index == other.rate_index
    }
}

#[cfg(test)]
mod tests;
