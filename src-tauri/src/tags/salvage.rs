//! The last rung of the reading ladder: recovering what lofty won't.
//!
//! lofty rejects a whole file over one bad part, even in its most relaxed
//! mode: one frame with an invalid text encoding loses every ID3v2 frame,
//! and a broken APE footer loses the audio properties too. ROADMAP §5.5
//! says neither may stop the rest being read, so this module:
//! - reads a trailing APE tag and the ID3v1 tag after it each on its own,
//!   and hides them from the rest of the read, and
//! - rebuilds the ID3v2 tag frame by frame, dropping only the frames lofty
//!   can't parse on their own.
//!
//! Everything here reads; the rebuilt tag lives only in memory.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use lofty::ape::ApeTag;
use lofty::config::{ParseOptions, ParsingMode};
use lofty::file::{AudioFile, FileType};
use lofty::id3::v1::Id3v1Tag;
use lofty::id3::v2::Id3v2Tag;
use lofty::mpeg::MpegFile;

use super::guard;

/// A read-only view of the first `len` bytes of a reader, so a parser never
/// sees what comes after.
pub(super) struct Prefix<'a, R> {
    inner: &'a mut R,
    len: u64,
    pos: u64,
}

impl<'a, R: Read + Seek> Prefix<'a, R> {
    pub(super) fn new(inner: &'a mut R, len: u64) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(0))?;
        Ok(Prefix { inner, len, pos: 0 })
    }
}

impl<R: Read + Seek> Read for Prefix<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.pos);
        let want = (buf.len() as u64).min(left) as usize;
        if want == 0 {
            return Ok(0);
        }
        let n = self.inner.read(&mut buf[..want])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for Prefix<'_, R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let target = target
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
        self.pos = self.inner.seek(SeekFrom::Start(target))?;
        Ok(self.pos)
    }
}

const APE_FOOTER_LEN: u64 = 32;
const ID3V1_LEN: u64 = 128;
/// An APE tag bigger than this is taken as a corrupt size, not read.
const MAX_APE_LEN: u64 = 64 << 20;

/// The tags at the end of a file that ends in an APE tag: the APE tag
/// itself and an ID3v1 tag after it, each read on its own.
pub(super) struct Trailing {
    /// Where the trailing tags start: a view this long hides them all.
    pub start: u64,
    /// The APE tag, when it parses on its own.
    pub ape: Option<ApeTag>,
    /// The APE tag is there but couldn't be read.
    pub ape_broken: bool,
    pub id3v1: Option<Id3v1Tag>,
}

/// Finds a trailing APE tag (possibly followed by an ID3v1 tag) and reads
/// each on its own, so one broken block can't hide the other.
pub(super) fn trailing<R: Read + Seek>(reader: &mut R) -> io::Result<Option<Trailing>> {
    let len = reader.seek(SeekFrom::End(0))?;
    for after in [0, ID3V1_LEN] {
        let Some(footer_at) = len.checked_sub(after + APE_FOOTER_LEN) else {
            continue;
        };
        let mut footer = [0u8; APE_FOOTER_LEN as usize];
        reader.seek(SeekFrom::Start(footer_at))?;
        reader.read_exact(&mut footer)?;
        if &footer[..8] != b"APETAGEX" {
            continue;
        }

        let id3v1 = if after == ID3V1_LEN {
            let tail = read_range(reader, len - ID3V1_LEN, len)?;
            parse_tail(&tail).and_then(|(_, v1)| v1)
        } else {
            None
        };

        // The footer's size counts the items and the footer; a flag says
        // whether a 32-byte header comes before the items.
        let size = u32::from_le_bytes(footer[12..16].try_into().unwrap()) as u64;
        let flags = u32::from_le_bytes(footer[20..24].try_into().unwrap());
        let header = if flags & 0x8000_0000 != 0 { 32 } else { 0 };
        let footer_end = footer_at + APE_FOOTER_LEN;
        let ape_start = (size >= APE_FOOTER_LEN && size + header <= MAX_APE_LEN)
            .then(|| footer_end.checked_sub(size + header))
            .flatten();
        let ape = match ape_start {
            Some(from) => parse_tail(&read_range(reader, from, footer_end)?).and_then(|t| t.0),
            None => None,
        };

        return Ok(Some(Trailing {
            // A broken APE tag's size can't be trusted, so cut at its footer
            // and leave its items as a few junk bytes after the audio, which
            // parsers skip.
            start: match (&ape, ape_start) {
                (Some(_), Some(from)) => from,
                _ => footer_at,
            },
            ape_broken: ape.is_none(),
            ape,
            id3v1,
        }));
    }
    Ok(None)
}

fn read_range<R: Read + Seek>(reader: &mut R, from: u64, to: u64) -> io::Result<Vec<u8>> {
    reader.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    reader.take(to - from).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Reads an APE and/or ID3v1 tag on its own, by putting it after silent
/// audio, where lofty looks for trailing tags.
fn parse_tail(tail: &[u8]) -> Option<(Option<ApeTag>, Option<Id3v1Tag>)> {
    let mut bytes = silent_mpeg_frame().repeat(2);
    bytes.extend_from_slice(tail);
    let file = guard(|| {
        MpegFile::read_from(&mut Cursor::new(bytes), tag_only()).map_err(|e| e.to_string())
    })
    .ok()?;
    Some((file.ape().cloned(), file.id3v1().cloned()))
}

fn tag_only() -> ParseOptions {
    ParseOptions::new()
        .read_properties(false)
        .parsing_mode(ParsingMode::BestAttempt)
}

/// An ID3v2 tag rebuilt from the frames that parsed.
pub(super) struct Salvaged {
    pub tag: Id3v2Tag,
    /// What was dropped, in file order: frame IDs, or a note naming bytes
    /// that couldn't be split into frames.
    pub dropped: Vec<String>,
}

/// Finds the ID3v2 tag of an MP3/AAC (at the start) or WAV/AIFF (in an
/// `ID3 ` chunk) and rebuilds it frame by frame.
pub(super) fn id3v2<R: Read + Seek>(
    file_type: FileType,
    reader: &mut R,
) -> io::Result<Option<Salvaged>> {
    let start = match file_type {
        FileType::Mpeg | FileType::Aac => Some(0),
        FileType::Wav => id3_chunk(reader, b"RIFF", u32::from_le_bytes)?,
        FileType::Aiff => id3_chunk(reader, b"FORM", u32::from_be_bytes)?,
        _ => None,
    };
    let Some(start) = start else {
        return Ok(None);
    };
    reader.seek(SeekFrom::Start(start))?;
    let mut header = [0u8; 10];
    if reader.read_exact(&mut header).is_err() || &header[..3] != b"ID3" {
        return Ok(None);
    }
    let size = synchsafe(&header[6..10]) as u64;
    let mut body = Vec::new();
    reader.take(size).read_to_end(&mut body)?;
    Ok(rebuild(&header, &body))
}

/// The start of the `ID3 ` (or `id3 `) chunk in a RIFF or IFF file.
fn id3_chunk<R: Read + Seek>(
    reader: &mut R,
    magic: &[u8; 4],
    size_of: fn([u8; 4]) -> u32,
) -> io::Result<Option<u64>> {
    let len = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;
    let mut head = [0u8; 12];
    if reader.read_exact(&mut head).is_err() || &head[..4] != magic {
        return Ok(None);
    }
    let mut at = 12u64;
    while at + 8 <= len {
        reader.seek(SeekFrom::Start(at))?;
        let mut chunk = [0u8; 8];
        reader.read_exact(&mut chunk)?;
        let id = &chunk[..4];
        if id.eq_ignore_ascii_case(b"ID3 ") {
            return Ok(Some(at + 8));
        }
        let size = size_of(chunk[4..8].try_into().unwrap()) as u64;
        // Chunks are padded to an even length.
        at += 8 + size + (size & 1);
    }
    Ok(None)
}

fn synchsafe(b: &[u8]) -> u32 {
    b.iter().fold(0, |n, &byte| (n << 7) | (byte & 0x7F) as u32)
}

fn synchsafe_bytes(n: u32) -> [u8; 4] {
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

fn is_frame_id(id: &[u8]) -> bool {
    id.len() == 4
        && id
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Whether a frame ending at `end` leaves the tag at a sensible place: its
/// end, zero padding, or the next frame's ID.
fn lands_on_boundary(body: &[u8], end: usize) -> bool {
    match body.get(end..) {
        None => false,
        Some([]) => true,
        Some(rest) => rest.iter().all(|&b| b == 0) || rest.get(..4).is_some_and(is_frame_id),
    }
}

/// The size of the frame at `pos`.
///
/// v2.4 sizes are synchsafe, but some taggers (old iTunes) wrote plain ones
/// anyway. Like mutagen and TagLib, take the reading whose frame ends on a
/// boundary; synchsafe first, since that's the standard.
fn frame_size(version: u8, body: &[u8], pos: usize) -> usize {
    let b = &body[pos + 4..pos + 8];
    let plain = u32::from_be_bytes(b.try_into().unwrap()) as usize;
    if version != 4 || b.iter().any(|byte| byte & 0x80 != 0) {
        return plain;
    }
    let safe = synchsafe(b) as usize;
    [safe, plain]
        .into_iter()
        .find(|size| lands_on_boundary(body, pos + 10 + size))
        .unwrap_or(safe)
}

/// Splits the tag body into frames, keeps those lofty parses alone, and
/// parses the survivors as one tag.
fn rebuild(header: &[u8; 10], body: &[u8]) -> Option<Salvaged> {
    let version = header[3];
    let flags = header[5];
    // v2.2 has 3-byte frame IDs; v2.3 tag-wide unsynchronisation rewrites the
    // bytes. Neither is worth hand-parsing here.
    if !(version == 3 || version == 4) || (version == 3 && flags & 0x80 != 0) {
        return None;
    }

    let mut pos = 0usize;
    if flags & 0x40 != 0 {
        // Extended header: v2.4 counts its own size field, v2.3 doesn't.
        let size_bytes = body.get(..4)?;
        pos = match version {
            4 => synchsafe(size_bytes) as usize,
            _ => 4 + u32::from_be_bytes(size_bytes.try_into().ok()?) as usize,
        };
    }

    let mut kept: Vec<Vec<u8>> = Vec::new();
    let mut dropped = Vec::new();
    while pos < body.len() {
        let rest = &body[pos..];
        if rest.len() < 10 || !is_frame_id(&rest[..4]) {
            if rest.iter().any(|&b| b != 0) {
                // Offsets count from the start of the tag header.
                dropped.push(format!(
                    "unreadable data at tag offset {} ({} bytes)",
                    10 + pos,
                    rest.len()
                ));
            }
            break; // padding, or garbage we can't find our way past
        }
        let id_text = String::from_utf8_lossy(&rest[..4]).into_owned();
        let size = frame_size(version, body, pos);
        let end = pos + 10 + size;
        if end > body.len() {
            dropped.push(id_text);
            break;
        }
        let mut frame = body[pos..end].to_vec();
        if version == 4 {
            // Re-encode the size the standard way, so lofty reads the frame
            // the way we split it.
            frame[4..8].copy_from_slice(&synchsafe_bytes(size as u32));
        }
        if parse_tag(version, &frame).is_some_and(|t| !t.is_empty()) {
            kept.push(frame);
        } else {
            dropped.push(id_text);
        }
        pos = end;
    }

    let tag = parse_tag(version, &kept.concat())?;
    Some(Salvaged { tag, dropped })
}

/// A silent MPEG-1 Layer III frame (128 kbps, 44.1 kHz, mono), so lofty has
/// audio to find next to a tag it's asked to parse.
fn silent_mpeg_frame() -> Vec<u8> {
    let mut frame = vec![0u8; 417];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
    frame
}

/// Parses ID3v2 frames by wrapping them in a tag in front of silent audio.
fn parse_tag(version: u8, frames: &[u8]) -> Option<Id3v2Tag> {
    let size = u32::try_from(frames.len()).ok()?;
    if size >= 1 << 28 {
        return None;
    }
    let mut bytes = vec![b'I', b'D', b'3', version, 0, 0];
    bytes.extend_from_slice(&synchsafe_bytes(size));
    bytes.extend_from_slice(frames);
    bytes.extend(silent_mpeg_frame().repeat(2));

    let file = guard(|| {
        MpegFile::read_from(&mut Cursor::new(bytes), tag_only()).map_err(|e| e.to_string())
    })
    .ok()?;
    Some(file.id3v2().cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_frame(id: &[u8; 4], text: &str) -> Vec<u8> {
        let mut f = id.to_vec();
        let len = text.len() as u32 + 1;
        f.extend_from_slice(&len.to_be_bytes());
        f.extend_from_slice(&[0, 0, 3]);
        f.extend_from_slice(text.as_bytes());
        f
    }

    fn header(version: u8, body_len: usize) -> [u8; 10] {
        let n = body_len as u32;
        [
            b'I',
            b'D',
            b'3',
            version,
            0,
            0,
            ((n >> 21) & 0x7F) as u8,
            ((n >> 14) & 0x7F) as u8,
            ((n >> 7) & 0x7F) as u8,
            (n & 0x7F) as u8,
        ]
    }

    #[test]
    fn a_prefix_view_hides_everything_after_its_length() {
        let mut data = Cursor::new(b"0123456789".to_vec());
        let mut view = Prefix::new(&mut data, 4).unwrap();
        let mut all = Vec::new();
        view.read_to_end(&mut all).unwrap();
        assert_eq!(all, b"0123");
        assert_eq!(view.seek(SeekFrom::End(-1)).unwrap(), 3);
        let mut one = [0u8; 4];
        assert_eq!(view.read(&mut one).unwrap(), 1);
        assert_eq!(one[0], b'3');
        assert!(view.seek(SeekFrom::End(-5)).is_err());
    }

    #[test]
    fn a_broken_ape_footer_is_found_at_the_end_or_before_an_id3v1_tag() {
        // A footer whose size (0) can't be right.
        let mut ape_last = vec![0u8; 100];
        ape_last.extend_from_slice(b"APETAGEX");
        ape_last.extend_from_slice(&[0; 24]);
        let found = |bytes: &[u8]| {
            trailing(&mut Cursor::new(bytes))
                .unwrap()
                .map(|t| (t.start, t.ape_broken))
        };
        assert_eq!(found(&ape_last), Some((100, true)));

        let mut then_id3v1 = ape_last.clone();
        then_id3v1.extend_from_slice(&[0; 128]);
        assert_eq!(found(&then_id3v1), Some((100, true)));

        assert_eq!(found(&[0u8; 10]), None);
    }

    #[test]
    fn a_plain_v24_frame_size_is_used_when_the_synchsafe_reading_lands_mid_frame() {
        // A 300-byte TIT2 with a plain size: 00 00 01 2C, which as synchsafe
        // is 172 and would end inside the text.
        let text = "x".repeat(299);
        let mut tit2 = b"TIT2".to_vec();
        tit2.extend_from_slice(&300u32.to_be_bytes());
        tit2.extend_from_slice(&[0, 0, 3]);
        tit2.extend_from_slice(text.as_bytes());
        let body = [tit2, text_frame(b"TALB", "Album")].concat();

        let salvaged = rebuild(&header(4, body.len()), &body).unwrap();
        assert_eq!(salvaged.dropped, Vec::<String>::new());
        let tag = lofty::tag::Tag::from(salvaged.tag);
        use lofty::tag::Accessor;
        assert_eq!(tag.title().as_deref(), Some(text.as_str()));
        assert_eq!(tag.album().as_deref(), Some("Album"));
    }

    #[test]
    fn garbage_the_rebuild_cant_get_past_is_named_with_its_offset_and_size() {
        let garbage = [1u8, 2, b'j', b'u', b'n', b'k', b'!'];
        let body = [text_frame(b"TIT2", "Title"), garbage.to_vec()].concat();
        let salvaged = rebuild(&header(3, body.len()), &body).unwrap();
        let offset = 10 + text_frame(b"TIT2", "Title").len();
        assert_eq!(
            salvaged.dropped,
            vec![format!("unreadable data at tag offset {offset} (7 bytes)")]
        );
        assert_eq!(salvaged.tag.len(), 1);
    }

    #[test]
    fn zero_padding_after_the_frames_is_not_reported() {
        let body = [text_frame(b"TIT2", "Title"), vec![0; 64]].concat();
        let salvaged = rebuild(&header(3, body.len()), &body).unwrap();
        assert_eq!(salvaged.dropped, Vec::<String>::new());
    }

    #[test]
    fn a_v24_extended_header_is_skipped() {
        // Size 6 (synchsafe, counts itself), one flag byte, no flags.
        let ext = vec![0, 0, 0, 6, 1, 0];
        let body = [ext, text_frame(b"TIT2", "Title")].concat();
        let mut head = header(4, body.len());
        head[5] = 0x40;
        let salvaged = rebuild(&head, &body).unwrap();
        assert_eq!(salvaged.dropped, Vec::<String>::new());
        assert_eq!(salvaged.tag.len(), 1);
    }

    #[test]
    fn rebuilding_keeps_good_frames_and_names_the_dropped_ones() {
        let mut bad = b"TPE1".to_vec();
        bad.extend_from_slice(&[0, 0, 0, 2, 0, 0, 9, b'x']); // encoding 9
                                                             // v2.3: plain frame sizes.
        let body = [
            text_frame(b"TIT2", "Title"),
            bad,
            text_frame(b"TALB", "Album"),
        ]
        .concat();
        let salvaged = rebuild(&header(3, body.len()), &body).unwrap();
        assert_eq!(salvaged.dropped, vec!["TPE1".to_string()]);
        let ids: Vec<_> = salvaged
            .tag
            .into_iter()
            .map(|f| f.id_str().to_string())
            .collect();
        assert_eq!(ids, vec!["TIT2", "TALB"]);
    }

    #[test]
    fn rebuilding_stops_at_a_frame_that_runs_past_the_tag() {
        let mut overrun = b"TPE1".to_vec();
        overrun.extend_from_slice(&[0, 0, 0x10, 0, 0, 0, 3, b'x']);
        let body = [text_frame(b"TIT2", "Title"), overrun].concat();
        let salvaged = rebuild(&header(3, body.len()), &body).unwrap();
        assert_eq!(salvaged.dropped, vec!["TPE1".to_string()]);
        assert_eq!(salvaged.tag.len(), 1);
    }

    #[test]
    fn id3v22_tags_are_left_alone() {
        assert!(rebuild(&header(2, 0), &[]).is_none());
    }
}
