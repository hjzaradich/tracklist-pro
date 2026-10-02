//! How a fingerprint is kept in `file.fingerprint`, and how two compare.
//!
//! The blob is `TLFP`, a [`VERSION`] byte, then chromaprint's own
//! compressed form: the algorithm id, the item count (24 bits), and each
//! item's change from the one before as 3-bit and 5-bit packed bit gaps.
//! That's the form AcoustID's lookup takes, and smaller than the raw items
//! (by how much depends on how fast the music changes).
//!
//! [`VERSION`] names everything that shapes the items: the rusty-chromaprint
//! release and preset, the mono downmix, and whole tracks rather than a
//! window. Fingerprints of different versions are never compared
//! ([`CompareError::Incomparable`]); bump it when any of those change, and
//! every file is fingerprinted again.

use std::fmt;

use rusty_chromaprint::{match_fingerprints, Configuration, FingerprintCompressor};

/// The version this build writes. 1: rusty-chromaprint 0.3, `preset_test2`
/// (algorithm 1, as E3 graded), mono downmix, the whole track.
pub const VERSION: u8 = 1;

/// The first bytes of every stored fingerprint.
const MAGIC: &[u8; 4] = b"TLFP";

/// The start of a blob this build writes.
#[cfg(test)]
pub(crate) const CURRENT_PREFIX: [u8; 5] = [MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], VERSION];

/// The chromaprint settings of [`VERSION`].
pub fn config() -> Configuration {
    Configuration::preset_test2()
}

/// A whole track's fingerprint: one 32-bit item per ~0.124 s of audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    version: u8,
    algorithm: u8,
    items: Vec<u32>,
}

/// Why a blob couldn't be read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobError {
    /// It doesn't start with `TLFP`.
    NotAFingerprint,
    /// Written by a later build.
    UnknownVersion(u8),
    /// Cut short or damaged.
    Corrupt,
}

impl fmt::Display for BlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlobError::NotAFingerprint => f.write_str("not a stored fingerprint"),
            BlobError::UnknownVersion(v) => write!(f, "fingerprint version {v} is unknown"),
            BlobError::Corrupt => f.write_str("the stored fingerprint is damaged"),
        }
    }
}

impl std::error::Error for BlobError {}

impl Fingerprint {
    /// Items computed by this build, with [`config`].
    pub(crate) fn new(items: Vec<u32>) -> Fingerprint {
        Fingerprint {
            version: VERSION,
            algorithm: config().id(),
            items,
        }
    }

    /// Pretends another version made it.
    #[cfg(test)]
    pub(crate) fn set_version_for_tests(&mut self, version: u8) {
        self.version = version;
    }

    /// Pretends another chromaprint algorithm made it.
    #[cfg(test)]
    pub(crate) fn set_algorithm_for_tests(&mut self, algorithm: u8) {
        self.algorithm = algorithm;
    }

    pub fn items(&self) -> &[u32] {
        &self.items
    }

    /// Which [`VERSION`] made it.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// Whether `other`'s items mean the same as this one's: made by the
    /// same [`VERSION`] and chromaprint algorithm.
    pub fn comparable_with(&self, other: &Fingerprint) -> bool {
        self.version == other.version && self.algorithm == other.algorithm
    }

    /// How much audio it covers.
    pub fn seconds(&self) -> f32 {
        self.items.len() as f32 * config().item_duration_in_seconds()
    }

    /// The bytes stored in `file.fingerprint`.
    pub fn to_blob(&self) -> Vec<u8> {
        let config = config();
        let compressed = FingerprintCompressor::from(&config).compress(&self.items);
        let mut blob = Vec::with_capacity(MAGIC.len() + 1 + compressed.len());
        blob.extend_from_slice(MAGIC);
        blob.push(self.version);
        blob.extend_from_slice(&compressed);
        blob
    }

    /// Reads a stored blob back. Never panics, whatever the bytes.
    pub fn from_blob(blob: &[u8]) -> Result<Fingerprint, BlobError> {
        let rest = blob
            .strip_prefix(MAGIC.as_slice())
            .ok_or(BlobError::NotAFingerprint)?;
        let (&version, compressed) = rest.split_first().ok_or(BlobError::Corrupt)?;
        if version != VERSION {
            return Err(BlobError::UnknownVersion(version));
        }
        let (algorithm, items) = decompress(compressed).ok_or(BlobError::Corrupt)?;
        Ok(Fingerprint {
            version,
            algorithm,
            items,
        })
    }
}

/// Reads `bits` bits (at most 8) starting at bit `at` of `bytes`, least
/// significant bit first, as chromaprint packs them.
fn bits_at(bytes: &[u8], at: usize, bits: usize) -> Option<u8> {
    let mut value = 0u8;
    for i in 0..bits {
        let bit = at + i;
        let byte = *bytes.get(bit / 8)?;
        value |= ((byte >> (bit % 8)) & 1) << i;
    }
    Some(value)
}

/// chromaprint's decompressor: the inverse of [`FingerprintCompressor`].
/// `None` if the bytes don't hold exactly what their header says.
fn decompress(bytes: &[u8]) -> Option<(u8, Vec<u32>)> {
    const NORMAL_BITS: usize = 3;
    const EXCEPTION_BITS: usize = 5;
    const ESCAPE: u8 = 7;

    let (header, packed) = (bytes.get(..4)?, bytes.get(4..)?);
    let algorithm = header[0];
    let count = usize::from(header[1]) << 16 | usize::from(header[2]) << 8 | usize::from(header[3]);
    // Every item ends with a 0 gap, so the header can't claim more items
    // than there are 3-bit slots. Checked before anything is allocated.
    if count > packed.len() * 8 / NORMAL_BITS {
        return None;
    }

    let mut gaps = Vec::with_capacity(count);
    let mut ends = 0;
    while ends < count {
        let gap = bits_at(packed, gaps.len() * NORMAL_BITS, NORMAL_BITS)?;
        ends += usize::from(gap == 0);
        gaps.push(gap);
    }
    let exceptions = packed.get((gaps.len() * NORMAL_BITS).div_ceil(8)..)?;

    let mut items = Vec::with_capacity(count);
    let (mut previous, mut item, mut bit) = (0u32, 0u32, 0u32);
    let mut escapes = 0;
    for gap in gaps {
        if gap == 0 {
            // Each item is stored as its XOR with the one before.
            previous ^= item;
            items.push(previous);
            (item, bit) = (0, 0);
            continue;
        }
        let mut step = u32::from(gap);
        if gap == ESCAPE {
            step += u32::from(bits_at(
                exceptions,
                escapes * EXCEPTION_BITS,
                EXCEPTION_BITS,
            )?);
            escapes += 1;
        }
        bit += step;
        if bit > 32 {
            return None;
        }
        item |= 1 << (bit - 1);
    }
    // Nothing left over: a blob with trailing bytes isn't one we wrote.
    if exceptions.len() != (escapes * EXCEPTION_BITS).div_ceil(8) {
        return None;
    }
    Some((algorithm, items))
}

/// How two fingerprints line up (ROADMAP 1.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Comparison {
    /// The share of the first track's length that matched, 0 to 1.
    pub coverage_a: f32,
    /// The share of the second track's length that matched, 0 to 1.
    pub coverage_b: f32,
    /// The average number of differing bits (of 32) over the matched
    /// parts, weighted by length: 0 is identical. 32 when nothing matched.
    pub score: f64,
}

/// Matched coverage both ways at or above this…
pub const DUPLICATE_COVERAGE: f32 = 0.9;
/// …and a score at or below this make a duplicate (ROADMAP 1.4).
pub const DUPLICATE_SCORE: f64 = 4.0;

impl Comparison {
    /// ROADMAP 1.4's duplicate rule, on the audio alone: at least 90%
    /// matched both ways and a score of 4 or less. The version check (1.5)
    /// can still veto a merge.
    pub fn is_duplicate(&self) -> bool {
        self.coverage_a >= DUPLICATE_COVERAGE
            && self.coverage_b >= DUPLICATE_COVERAGE
            && self.score <= DUPLICATE_SCORE
    }
}

/// Why two fingerprints couldn't be compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareError {
    /// Made by different versions or algorithms: their items mean different
    /// things.
    Incomparable,
    /// Longer than rusty-chromaprint's matcher takes (about 18 hours).
    TooLong,
}

impl fmt::Display for CompareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompareError::Incomparable => {
                f.write_str("fingerprints of different versions can't be compared")
            }
            CompareError::TooLong => f.write_str("a fingerprint is too long to compare"),
        }
    }
}

impl std::error::Error for CompareError {}

/// Lines two full-track fingerprints up with rusty-chromaprint's matcher.
pub fn compare(a: &Fingerprint, b: &Fingerprint) -> Result<Comparison, CompareError> {
    if a.version != b.version || a.algorithm != b.algorithm {
        return Err(CompareError::Incomparable);
    }
    let config = config();
    let segments =
        match_fingerprints(&a.items, &b.items, &config).map_err(|_| CompareError::TooLong)?;
    let matched: f32 = segments.iter().map(|s| s.duration(&config)).sum();
    let score = if matched > 0.0 {
        segments
            .iter()
            .map(|s| s.score * f64::from(s.duration(&config)))
            .sum::<f64>()
            / f64::from(matched)
    } else {
        32.0
    };
    let coverage = |fp: &Fingerprint| (matched / fp.seconds().max(f32::EPSILON)).min(1.0);
    Ok(Comparison {
        coverage_a: coverage(a),
        coverage_b: coverage(b),
        score,
    })
}
