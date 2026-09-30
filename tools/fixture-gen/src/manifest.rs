//! The ground truth written next to the fixture tree
//! (`fixture-manifest.json`), for later tests to assert against.

use serde::Serialize;

use crate::encode::Tags;

pub const SCHEMA: u32 = 1;
pub const FILE_NAME: &str = "fixture-manifest.json";
/// The folder under the target that holds the music. Scanners point here, so
/// they never see the manifest.
pub const MUSIC_ROOT: &str = "music";

#[derive(Debug, Serialize)]
pub struct Manifest {
    pub schema: u32,
    pub generator: String,
    pub seed: u64,
    /// Relative to the target folder; every `path` below starts with it.
    pub music_root: &'static str,
    pub counts: Counts,
    /// One per distinct recording. Files sharing a recording are duplicates.
    pub recordings: Vec<Recording>,
    /// Recordings with two or more files: what grouping must merge.
    pub duplicate_groups: Vec<DuplicateGroup>,
    /// Versions: linked, never merged (ROADMAP §1.2, 1.5).
    pub version_links: Vec<VersionLink>,
    /// Pairs whose names look related but whose audio isn't: nothing may
    /// link or merge them.
    pub not_related: Vec<NotRelated>,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Default, Serialize)]
pub struct Counts {
    pub files: usize,
    pub audio: usize,
    pub traps: usize,
    pub skips: usize,
    pub bulk: usize,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Recording {
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DuplicateGroup {
    pub recording: String,
    pub files: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Same production, different length or lyrics (Extended, Radio Edit,
    /// Clean/Dirty).
    Cut,
    /// A different production (remix, VIP, flip, bootleg, cover, live,
    /// mashup).
    Rework,
}

#[derive(Debug, Serialize)]
pub struct VersionLink {
    pub a: String,
    pub b: String,
    pub kind: LinkKind,
    /// E.g. "extended", "radio edit", "clean/dirty", "remix", "cover".
    pub label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
    /// Set when one recording is an exact slice of the other.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<Contains>,
}

/// `inner`'s audio is `outer`'s, starting at `at_frame`.
#[derive(Debug, Serialize)]
pub struct Contains {
    pub outer: String,
    pub inner: String,
    pub at_frame: u64,
}

#[derive(Debug, Serialize)]
pub struct NotRelated {
    pub a: String,
    pub b: String,
    pub note: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// A valid audio file a scanner should index.
    Audio,
    /// Deliberately damaged or mislabeled; see `trap`.
    Trap,
    /// Not music; a scanner must skip it (§5.5).
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrapKind {
    /// A `.wav` name holding MP3 bytes: detect the format from the bytes.
    Mp3InWav,
    /// An M4A with no `moov` box (an interrupted download): broken.
    M4aNoMoov,
    /// An ID3v2.4 `TDRC` that isn't a valid date. The audio is fine.
    BadTdrc,
    /// An APEv2 footer whose size and item count are wrong. The audio is fine.
    BrokenApe,
    /// A zero-byte file (a failed download): broken.
    EmptyFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipKind {
    /// A macOS `._` AppleDouble resource file.
    AppleDouble,
    /// A macOS `.DS_Store` folder settings file.
    DsStore,
}

#[derive(Clone, Debug, Serialize)]
pub struct Trap {
    pub kind: TrapKind,
    pub detail: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct QualityCase {
    /// The verdict ROADMAP 1.6 should give.
    pub expect: &'static str,
    /// The highest frequency in the source audio.
    pub content_max_hz: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileEntry {
    /// Relative to the target, `/`-separated, the exact on-disk name
    /// (normalization, trailing dots and spaces kept).
    pub path: String,
    pub role: Role,
    /// Labels for what this file exercises, e.g. "duplicate", "cut",
    /// "awkward-name:trailing-dot", "gig-stick-copy", "bulk".
    pub cases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording: Option<String>,
    /// The format the bytes really are (not the extension).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codec: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate_kbps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
    /// Samples per channel the audio decodes to (MP3s carry a LAME tag, so
    /// a gapless decoder gets exactly this).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<u64>,
    /// Whether a decoder can play it.
    pub decodes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Tags>,
    /// Set when this file is a byte-for-byte copy of another.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_copy_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trap: Option<Trap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<SkipKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<QualityCase>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub bytes: u64,
}
