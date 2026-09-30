//! The partial hash (1aC-1, ROADMAP 1.1, §5.1): a cheap fingerprint of a
//! file's content, kept in `file.partial_hash`, so a file whose modified
//! time moved isn't hashed and fingerprinted again if nothing in it did.
//!
//! rekordbox bumps the mtime of every file it adds or analyzes, and cloud
//! sync and backups do it too. The walk (`scan::unchanged`) reads a touched
//! file's partial hash again and compares it with the one the hash stage
//! stored when it last read the whole file. Equal: the stages' results
//! still stand.
//!
//! # The stored value
//!
//! 33 bytes: `[PARTIAL_DEFINITION, 32-byte digest]`. The digest is BLAKE3 in
//! key-derivation mode, with the context string
//! `"tracklist-pro partial_hash v<definition> <format name>"`, over, in
//! this order:
//!
//! 1. the file's length (u64, little-endian);
//! 2. the number of metadata parts (u64), then for each: its offset and
//!    its length (u64 each) and its bytes. **Metadata is every byte of the
//!    file outside the audio** as [`super::audio`] defines it for the
//!    format (its `plan`): tags, artwork, chunk headers, container
//!    boxes, padding. All of it is read, because rekordbox writes tags and
//!    artwork can be far bigger than any fixed window, and AIFF and WAV
//!    `id3 ` chunks and MP4 `moov` boxes can sit anywhere;
//! 3. the audio's length (u64), then **the first and last [`EDGE`] bytes
//!    of the audio** (all of it if it's no longer than two edges). Where
//!    the audio is several ranges (MP4 with more than one `mdat`), it
//!    counts as those ranges joined.
//!
//! Two values are equal only if the definition, the format family and all
//! of that are, so bump [`PARTIAL_DEFINITION`] whenever this comment
//! changes: old values then never match a new one, so every touched file
//! counts as changed until its stages run again, which stores a new one.
//!
//! Nothing backfills it: a file hashed before this existed, or before a
//! bump, keeps a NULL or outdated value until the hash stage next reads it,
//! so its first touch after that counts as changed, once.
//!
//! # When there's no partial hash
//!
//! `None`, and the file counts as changed if it's touched: no shortcut. That's
//! a file with no audio range to plan (empty, truncated, unknown or
//! unsupported format, Ogg, malformed), or whose metadata is more than
//! [`MAX_METADATA`] (4 MiB), where reading it all would cost more than the
//! shortcut saves.
//!
//! # What it can't see
//!
//! Tags the audio_hash counts as audio (an ID3v2.3 tag appended at the
//! end, `TAG+`, Lyrics3 v1) are covered only by the last edge, so an edit of
//! the same length in the early part of a large one goes unseen.
//!
//! A change of the same length in the middle of the audio, away from both
//! edges (a frame rewritten in place). Tags never do that; a re-encode or an
//! edit in an audio editor changes the length or the audio's edges. Reading
//! all the audio to be sure would mean hashing the whole library whenever
//! rekordbox touches it, which is what this hash exists to avoid.

use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;

use super::audio::{self, Plan};

/// The partial hash definition this build computes. Bump it whenever the
/// definition in the module docs changes.
pub const PARTIAL_DEFINITION: u8 = 1;

/// How many bytes a stored partial hash has: definition and digest.
pub const PARTIAL_HASH_LEN: usize = 33;

/// How much of the audio is read from each end: 64 KiB.
pub const EDGE: u64 = 64 * 1024;

/// The most metadata a file may have and still get a partial hash: 4 MiB.
pub const MAX_METADATA: u64 = 4 * 1024 * 1024;

/// A file's partial hash, as stored.
pub type PartialHash = [u8; PARTIAL_HASH_LEN];

/// The partial hash of the `len` bytes in `r`, or `None` if it has no
/// shortcut (module docs). Reads at most [`MAX_METADATA`] plus two
/// [`EDGE`]s, and a few small reads to find the audio. I/O errors (and a
/// file that shrank while being read) are returned.
pub fn partial_hash<R: Read + Seek>(r: &mut R, len: u64) -> io::Result<Option<PartialHash>> {
    let Plan::Ranges { format, ranges, .. } = audio::plan(r, len)? else {
        return Ok(None);
    };
    if !well_formed(&ranges, len) {
        return Ok(None);
    }
    let parts = complement(&ranges, len);
    let metadata: u64 = parts.iter().map(|p| p.end - p.start).sum();
    if metadata > MAX_METADATA {
        return Ok(None);
    }
    let audio_len: u64 = ranges.iter().map(|p| p.end - p.start).sum();

    let context = format!(
        "tracklist-pro partial_hash v{PARTIAL_DEFINITION} {}",
        format.name()
    );
    let mut hasher = blake3::Hasher::new_derive_key(&context);
    hasher.update(&len.to_le_bytes());
    hasher.update(&(parts.len() as u64).to_le_bytes());
    for part in &parts {
        hasher.update(&part.start.to_le_bytes());
        hasher.update(&(part.end - part.start).to_le_bytes());
        feed(r, part.start, part.end - part.start, &mut hasher)?;
    }
    hasher.update(&audio_len.to_le_bytes());
    if audio_len <= 2 * EDGE {
        feed_audio(r, &ranges, 0..audio_len, &mut hasher)?;
    } else {
        feed_audio(r, &ranges, 0..EDGE, &mut hasher)?;
        feed_audio(r, &ranges, audio_len - EDGE..audio_len, &mut hasher)?;
    }
    let mut out = [0u8; PARTIAL_HASH_LEN];
    out[0] = PARTIAL_DEFINITION;
    out[1..].copy_from_slice(hasher.finalize().as_bytes());
    Ok(Some(out))
}

/// Whether `ranges` are sorted, disjoint and inside the file, as the plan
/// promises (a file that shrank since it was measured can break that).
fn well_formed(ranges: &[Range<u64>], len: u64) -> bool {
    let mut at = 0;
    for r in ranges {
        if r.start < at || r.end < r.start || r.end > len {
            return false;
        }
        at = r.end;
    }
    true
}

/// The parts of `0..len` outside `ranges` (sorted and disjoint), in order,
/// leaving out empty ones.
fn complement(ranges: &[Range<u64>], len: u64) -> Vec<Range<u64>> {
    let mut parts = Vec::new();
    let mut at = 0;
    for r in ranges {
        if r.start > at {
            parts.push(at..r.start);
        }
        at = r.end;
    }
    if at < len {
        parts.push(at..len);
    }
    parts
}

/// Feeds the `n` bytes of `r` from `at` to `hasher`. A file that ends
/// early is an error, not a shorter hash.
fn feed<R: Read + Seek>(r: &mut R, at: u64, n: u64, hasher: &mut blake3::Hasher) -> io::Result<()> {
    r.seek(SeekFrom::Start(at))?;
    let mut buf = [0u8; 16 * 1024];
    let mut left = n;
    while left > 0 {
        let want = usize::try_from(left).map_or(buf.len(), |l| l.min(buf.len()));
        let got = match r.read(&mut buf[..want]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(got) => got,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buf[..got]);
        left -= got as u64;
    }
    Ok(())
}

/// Feeds `window` of the audio (the `ranges` joined) to `hasher`.
fn feed_audio<R: Read + Seek>(
    r: &mut R,
    ranges: &[Range<u64>],
    window: Range<u64>,
    hasher: &mut blake3::Hasher,
) -> io::Result<()> {
    // Where the current range starts within the joined audio.
    let mut joined = 0;
    for range in ranges {
        let len = range.end - range.start;
        let from = window.start.max(joined);
        let to = window.end.min(joined + len);
        if from < to {
            feed(r, range.start + (from - joined), to - from, hasher)?;
        }
        joined += len;
    }
    Ok(())
}
