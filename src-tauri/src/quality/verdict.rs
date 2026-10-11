//! What a file's quality measurements mean: its verdict, and the findings
//! behind it (ROADMAP 1.6; 1bB-6).
//!
//! A pure function. [`assess`] takes what's stored about a file (its codec
//! and bitrate from the read stage, the read stage's own verdict, the
//! `file_quality` row) and gives back a [`Verdict`] and the [`Finding`]s
//! that applied. It reads no database and shows no text: a finding is a
//! code with numbers (the reason a screen words from the locale files), and
//! English never appears here.
//!
//! **Precedence.** `broken`, then `truncated`, then `suspect transcode`,
//! then `low bitrate`; the verdict is the first that applies, and
//! [`Assessment::findings`] lists every one that applied in that order, so
//! later code (the best-file picker, the warning before a `low bitrate`
//! file joins the Library) doesn't derive them again. When nothing is
//! flagged the verdict is `ok` and the findings are the *notes*: shown with
//! a file, never flagged. Notes come only with `ok`.
//!
//! **A file that wasn't measured is never `ok`.** Without a usable
//! measurement (no row, a row made from other audio or by another method
//! version, no `audio_hash` to vouch for it, or a file the decoder couldn't
//! make sense of for reasons that aren't its content) the verdict comes
//! from what the headers and the read stage say alone: `low bitrate` and
//! the read stage's `truncated` / `broken`. With none of those there's no
//! verdict, which is "not checked", not "fine".
//!
//! **Bitrate is the stored average** (`file.bitrate`; for a VBR MP3 the
//! average its Xing/VBRI header gives, else the first frame's). Every line
//! is a constant with the reason it sits where it does, and the per-codec
//! lines are in one place, [`class`].
//!
//! **Truncated by length.** A file is `truncated` when the audio decoded is
//! shorter than the header's length by more than 2 s *and* by more than 1%
//! of it. The limit: an MP3 without a Xing/VBRI header has an *estimated*
//! header length (its size over its first frame's bitrate), so a headerless
//! VBR file can be flagged wrongly, and a cut-off CBR file can't be caught
//! (the estimate shrinks with the file). "Keep anyway" in Review covers the
//! first. A damaged frame in the middle costs far less than 2 s and isn't
//! flagged; decode errors on an otherwise complete file are a note.
//!
//! **Not here: reading the rows into [`Facts`].** The job that uses this
//! (1bC, grouping) maps the stored strings itself. What it has to map:
//!
//! - `file.codec`: `mp3`, `pcm`, `pcm_float`, `adpcm_ms`, `adpcm_ima`,
//!   `alaw`, `mulaw`, `flac`, `aac`, `alac`, `vorbis`, `opus`, `speex`,
//!   `ac3`, `eac3`, `wma`, `wavpack`, `musepack`, `ape`, `other` (the names
//!   [`Codec::as_str`] writes) to a [`Codec`]; NULL, or a name it doesn't
//!   know, is `None`.
//! - `file.quality_verdict`: `truncated` and `broken` to the read stage's
//!   [`ReadVerdict`]; NULL or anything else is `None`. That column is also
//!   where these verdicts are stored (`ok`, `low_bitrate`,
//!   `suspect_transcode`, `truncated`, `broken`), and the read stage clears
//!   its own two on every read, so a `truncated` this function produced
//!   must never be fed back in as the read stage's: keep the two apart.
//! - `file.bitrate` (kbps), `file.sample_rate` (Hz): integers or NULL.
//! - `file.audio_hash`: the 34-byte blob or NULL.
//! - `file_quality` through [`super::ledger::stored`]; its `ended`
//!   (`complete`, `cut_short`, `gave_up`) and `failure` (`unsupported_format`,
//!   `unsupported_codec`, `no_audio`, `damaged`, `decoder_crashed`) strings
//!   are compared here, against [`Ended::as_str`] and [`Failure::as_str`].

use serde::Serialize;
use specta::Type;

use super::ledger::Stored;
use super::measure::{Ended, Failure};
use super::VERSION;
use crate::read::{Codec, Verdict as ReadVerdict};

/// A lossy file below this average is `low bitrate`. The owner's line is
/// 192 kbps; two kbps of allowance so a nominal 192 whose average rounds to
/// 191 isn't flagged.
pub const LOW_BITRATE_BELOW_KBPS: u32 = 190;
/// A lossy file (MP3, Vorbis, Opus, WMA…) is at full quality from this
/// average. The line is 320 kbps; two kbps of allowance for rounding.
pub const FULL_QUALITY_KBPS: u32 = 318;
/// AAC is at full quality from this average. Stores sell it at a nominal
/// 256 kbps but it's variable, so real averages sit around that.
pub const FULL_QUALITY_AAC_KBPS: u32 = 240;
/// An MP3 at this average or more has a cutoff worth judging: below it the
/// encoder low-passes on purpose (a 192 kbps MP3 stops near 18 kHz). The
/// line is 256 kbps; two kbps of allowance, so a nominal 256 whose average
/// comes out at 255 is still checked.
pub const TRANSCODE_FROM_KBPS: u32 = 254;
/// An MP3 at [`TRANSCODE_FROM_KBPS`] or more with a cutoff below this was
/// made from a much worse source (a "320" from a ~128 kbps file).
pub const MP3_TRANSCODE_BELOW_HZ: u32 = 17_000;
/// A lossless file with a cutoff below this was made from a lossy one.
pub const LOSSLESS_TRANSCODE_BELOW_HZ: u32 = 19_500;
/// From [`MP3_TRANSCODE_BELOW_HZ`] up to this, a high-bitrate MP3's cutoff
/// is "possibly lower quality": not enough to flag.
pub const POSSIBLY_LOWER_BELOW_HZ: u32 = 19_000;
/// Audio decoded shorter than the header's length by *more* than this many
/// milliseconds, and by more than [`SHORT_MORE_THAN_PERCENT`], is
/// `truncated`. A longer allowance than a frame or two, because a header's
/// length is an estimate for some files.
pub const SHORT_MORE_THAN_MS: u32 = 2_000;
/// The relative half of the same rule, in percent of the header's length.
pub const SHORT_MORE_THAN_PERCENT: u32 = 1;

/// The lines for a lossy codec's bitrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Lines {
    low_below: u32,
    full_from: u32,
}

/// What bitrate says about a codec's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// Judged by bitrate, and its cutoff means something for MP3 only.
    Lossy(Lines),
    /// No bitrate level; the cutoff can show a lossy source.
    Lossless,
    /// Nothing to judge it by: telephony codecs, speech, and codecs no
    /// lines were set for (AC-3, E-AC-3, Musepack, unknown ones).
    Unrated,
}

/// The one table of per-codec lines. Exhaustive on purpose: a new codec
/// has to be put somewhere.
fn class(codec: Codec) -> Class {
    let general = Lines {
        low_below: LOW_BITRATE_BELOW_KBPS,
        full_from: FULL_QUALITY_KBPS,
    };
    match codec {
        Codec::Mp3 | Codec::Vorbis | Codec::Opus | Codec::Wma => Class::Lossy(general),
        Codec::Aac => Class::Lossy(Lines {
            full_from: FULL_QUALITY_AAC_KBPS,
            ..general
        }),
        // WavPack has a hybrid lossy mode and WMA a lossless one; the codec
        // headers don't tell them apart, and what's said of them here is
        // harmless either way (a lossless WMA is far above every line).
        Codec::Pcm | Codec::PcmFloat | Codec::Flac | Codec::Alac | Codec::WavPack | Codec::Ape => {
            Class::Lossless
        }
        // No lines were set for these (owner, via the foreman, 1bB-6): the
        // app makes no bitrate claim about a codec nobody decided about.
        Codec::Ac3
        | Codec::Eac3
        | Codec::Musepack
        | Codec::AdpcmMs
        | Codec::AdpcmIma
        | Codec::Alaw
        | Codec::Mulaw
        | Codec::Speex
        | Codec::Other => Class::Unrated,
    }
}

/// The verdicts. Stored in `file.quality_verdict` under [`Verdict::as_str`]
/// (`clipped` is held back).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Ok,
    LowBitrate,
    SuspectTranscode,
    Truncated,
    Broken,
}

impl Verdict {
    /// The name stored in `file.quality_verdict`.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::LowBitrate => "low_bitrate",
            Verdict::SuspectTranscode => "suspect_transcode",
            Verdict::Truncated => "truncated",
            Verdict::Broken => "broken",
        }
    }
}

/// One thing found about a file: a reason code and the numbers it's
/// worded with. Serialized with a camelCase `code` and camelCase fields;
/// the words come with the screen that shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum Finding {
    /// The read stage found the file unplayable (zero bytes, an MP4 with no
    /// index, audio no reader can make sense of, a duration of zero).
    BrokenUnplayable,
    /// The content is damaged past decoding.
    BrokenDamaged,
    /// The file decoded to no audio.
    BrokenNoAudio,
    /// Too many packets in a row were damaged; the rest couldn't be read.
    BrokenGaveUp,
    /// The read stage found the file ends inside a header or chunk.
    TruncatedFile,
    /// The stream stopped in the middle.
    TruncatedStream,
    /// Less audio than the header claims, by more than the allowance.
    #[serde(rename_all = "camelCase")]
    TruncatedShort { decoded_ms: u32, header_ms: u32 },
    /// An MP3 at a high bitrate whose spectrum stops low.
    #[serde(rename_all = "camelCase")]
    SuspectTranscodeLossy { cutoff_hz: u32, kbps: u32 },
    /// A lossless file whose spectrum stops where a lossy file's does.
    #[serde(rename_all = "camelCase")]
    SuspectTranscodeLossless { cutoff_hz: u32 },
    /// A lossy file below the low line.
    #[serde(rename_all = "camelCase")]
    LowBitrate { kbps: u32 },
    /// Note: a high-bitrate MP3 with a cutoff between the two lines.
    #[serde(rename_all = "camelCase")]
    PossiblyLowerQuality { cutoff_hz: u32, kbps: u32 },
    /// Note: a lossy file from the low line up to full quality.
    #[serde(rename_all = "camelCase")]
    CouldBeBetter { kbps: u32, full_kbps: u32 },
    /// Note: packets that failed to decode in a file that otherwise reads
    /// to its end.
    #[serde(rename_all = "camelCase")]
    DecodeErrors { count: u32 },
}

impl Finding {
    /// The verdict this finding makes, or `None` for a note.
    pub fn verdict(&self) -> Option<Verdict> {
        match self {
            Finding::BrokenUnplayable
            | Finding::BrokenDamaged
            | Finding::BrokenNoAudio
            | Finding::BrokenGaveUp => Some(Verdict::Broken),
            Finding::TruncatedFile | Finding::TruncatedStream | Finding::TruncatedShort { .. } => {
                Some(Verdict::Truncated)
            }
            Finding::SuspectTranscodeLossy { .. } | Finding::SuspectTranscodeLossless { .. } => {
                Some(Verdict::SuspectTranscode)
            }
            Finding::LowBitrate { .. } => Some(Verdict::LowBitrate),
            Finding::PossiblyLowerQuality { .. }
            | Finding::CouldBeBetter { .. }
            | Finding::DecodeErrors { .. } => None,
        }
    }

    pub fn is_note(&self) -> bool {
        self.verdict().is_none()
    }
}

/// What a verdict is made from.
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    /// `file.codec`.
    pub codec: Option<Codec>,
    /// `file.bitrate`, in kbps.
    pub bitrate_kbps: Option<i64>,
    /// `file.sample_rate`, in Hz.
    pub sample_rate_hz: Option<i64>,
    /// What the read stage said (`file.quality_verdict`'s `truncated` or
    /// `broken`).
    pub read: Option<ReadVerdict>,
    /// The file's `audio_hash` now; a measurement is used only if it was
    /// made from this audio.
    pub audio_hash: Option<&'a [u8]>,
    /// The `file_quality` row, if there is one.
    pub stored: Option<&'a Stored>,
}

/// A verdict and what it was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    /// `None` when there's nothing to say yet: not measured, and nothing in
    /// the headers or the read stage flags it.
    pub verdict: Option<Verdict>,
    /// Every finding that applied. Flagged ones first, in precedence order
    /// (the first is the verdict's reason); when nothing is flagged, the
    /// notes, damage first.
    pub findings: Vec<Finding>,
}

impl Assessment {
    /// The finding that is the verdict's reason: the first one that
    /// applied, for a flagged verdict.
    pub fn reason(&self) -> Option<&Finding> {
        self.findings.first().filter(|f| !f.is_note())
    }

    /// The note to show, if there is one (only with `ok`).
    pub fn note(&self) -> Option<&Finding> {
        self.findings.iter().find(|f| f.is_note())
    }
}

/// The verdict and findings for a file.
pub fn assess(facts: &Facts<'_>) -> Assessment {
    let kbps = facts
        .bitrate_kbps
        .filter(|k| *k > 0)
        .map(|k| u32::try_from(k).unwrap_or(u32::MAX));
    let class = facts.codec.map(class);
    // The row is used only if it's of this audio, by this method.
    let row = facts.stored.filter(|s| {
        facts.audio_hash == Some(s.audio_hash.as_slice()) && s.method_version == i64::from(VERSION)
    });
    let failure = row.and_then(|r| r.failure.as_deref());
    let failed = |why: Failure| failure == Some(why.as_str());
    // A row that decoded: its numbers can be read.
    let measured = row.filter(|r| r.failure.is_none());
    let ended = measured.and_then(|r| r.ended.as_deref());
    let ended_as = |how: Ended| ended == Some(how.as_str());

    let mut flagged = Vec::new();

    // Broken.
    if facts.read == Some(ReadVerdict::Broken) {
        flagged.push(Finding::BrokenUnplayable);
    }
    if failed(Failure::Damaged) {
        flagged.push(Finding::BrokenDamaged);
    }
    if failed(Failure::NoAudio) {
        flagged.push(Finding::BrokenNoAudio);
    }
    if ended_as(Ended::GaveUp) {
        flagged.push(Finding::BrokenGaveUp);
    }

    // Truncated.
    if facts.read == Some(ReadVerdict::Truncated) {
        flagged.push(Finding::TruncatedFile);
    }
    if ended_as(Ended::CutShort) {
        flagged.push(Finding::TruncatedStream);
    }
    if let Some((decoded_ms, header_ms)) = measured.and_then(|r| r.decoded_ms.zip(r.header_ms)) {
        if is_short(decoded_ms, header_ms) {
            flagged.push(Finding::TruncatedShort {
                decoded_ms: to_u32(decoded_ms),
                header_ms: to_u32(header_ms),
            });
        }
    }

    // The cutoff: only where it was measured, and only where the file's
    // sample rate leaves room above the line for it to be found (a 32 kHz
    // file can't have a cutoff above 16 kHz, whatever it was made from).
    let cutoff_hz = measured
        .and_then(|r| r.cutoff_hz)
        .map(|hz| u32::try_from(hz).unwrap_or(u32::MAX));
    let room_above = |line_hz: u32| {
        facts
            .sample_rate_hz
            .is_some_and(|rate| rate / 2 > i64::from(line_hz))
    };
    let mp3_rate = match (facts.codec, kbps) {
        (Some(Codec::Mp3), Some(kbps)) if kbps >= TRANSCODE_FROM_KBPS => Some(kbps),
        _ => None,
    };

    // Suspect transcode.
    if let Some(hz) = cutoff_hz {
        match class {
            Some(Class::Lossy(_)) => {
                if let Some(kbps) = mp3_rate {
                    if hz < MP3_TRANSCODE_BELOW_HZ && room_above(MP3_TRANSCODE_BELOW_HZ) {
                        flagged.push(Finding::SuspectTranscodeLossy {
                            cutoff_hz: hz,
                            kbps,
                        });
                    }
                }
            }
            Some(Class::Lossless) => {
                if hz < LOSSLESS_TRANSCODE_BELOW_HZ && room_above(LOSSLESS_TRANSCODE_BELOW_HZ) {
                    flagged.push(Finding::SuspectTranscodeLossless { cutoff_hz: hz });
                }
            }
            Some(Class::Unrated) | None => {}
        }
    }

    // Low bitrate: from the headers alone.
    let lines = match class {
        Some(Class::Lossy(lines)) => Some(lines),
        _ => None,
    };
    if let (Some(lines), Some(kbps)) = (lines, kbps) {
        if kbps < lines.low_below {
            flagged.push(Finding::LowBitrate { kbps });
        }
    }

    if let Some(first) = flagged.first() {
        return Assessment {
            verdict: first.verdict(),
            findings: flagged,
        };
    }

    // Nothing flagged. Only a file that was actually measured is `ok`;
    // `measured` is `None` for a failed row, and a failed one with a reason
    // that's the file's own was flagged above.
    if measured.is_none() {
        return Assessment {
            verdict: None,
            findings: Vec::new(),
        };
    }

    // Notes.
    let mut notes = Vec::new();
    if let Some(count) = measured.map(|r| r.decode_errors).filter(|n| *n > 0) {
        notes.push(Finding::DecodeErrors {
            count: to_u32(count),
        });
    }
    if let (Some(hz), Some(kbps)) = (cutoff_hz, mp3_rate) {
        if (MP3_TRANSCODE_BELOW_HZ..POSSIBLY_LOWER_BELOW_HZ).contains(&hz) {
            notes.push(Finding::PossiblyLowerQuality {
                cutoff_hz: hz,
                kbps,
            });
        }
    }
    if let (Some(lines), Some(kbps)) = (lines, kbps) {
        if kbps < lines.full_from {
            notes.push(Finding::CouldBeBetter {
                kbps,
                full_kbps: lines.full_from,
            });
        }
    }
    Assessment {
        verdict: Some(Verdict::Ok),
        findings: notes,
    }
}

/// Whether `decoded_ms` is short of `header_ms` by more than both
/// allowances.
fn is_short(decoded_ms: i64, header_ms: i64) -> bool {
    // i128: a corrupt header can claim any length, and the products below
    // would overflow an i64.
    let (decoded, header) = (i128::from(decoded_ms), i128::from(header_ms));
    let short = header - decoded;
    short > i128::from(SHORT_MORE_THAN_MS)
        && short * 100 > header * i128::from(SHORT_MORE_THAN_PERCENT)
}

fn to_u32(n: i64) -> u32 {
    u32::try_from(n.max(0)).unwrap_or(u32::MAX)
}
