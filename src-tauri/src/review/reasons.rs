//! Reasons as codes plus parameters (ROADMAP 1, "No black boxes"): the best
//! file ranking's reasons, the grouping rules' reasons and a relink's
//! method, as [`Reason`]s whose key is an i18n key in the `review` namespace
//! and whose parameters are numbers and format names. The words are in
//! `src/locales/en/review.json`, never here.

use crate::best_file::{Format, Reason as Rank};
use crate::read::Codec;
use crate::suggest::Reason;

/// How many grouping reasons an item shows (copy-style: at most three
/// fragments).
pub const MAX_DOUBTS: usize = 3;

/// A key that is known to be well formed.
fn fact(key: &str) -> Reason {
    Reason::fact(key).unwrap_or_else(|e| panic!("bad reason key {key}: {e}"))
}

fn number(reason: Reason, name: &str, value: f64) -> Reason {
    reason
        .with_number(name, value)
        .unwrap_or_else(|e| panic!("bad reason parameter {name}: {e}"))
}

fn text(reason: Reason, name: &str, value: &str) -> Reason {
    reason
        .with_text(name, value)
        .unwrap_or_else(|e| panic!("bad reason parameter {name}: {e}"))
}

/// The name of a codec as it is written ("MP3", "FLAC"): a format's name,
/// not a phrase.
pub fn codec_label(codec: Option<Codec>) -> &'static str {
    match codec {
        Some(Codec::Mp3) => "MP3",
        Some(Codec::Pcm) => "PCM",
        Some(Codec::PcmFloat) => "float PCM",
        Some(Codec::AdpcmMs | Codec::AdpcmIma) => "ADPCM",
        Some(Codec::Alaw) => "A-law",
        Some(Codec::Mulaw) => "mu-law",
        Some(Codec::Flac) => "FLAC",
        Some(Codec::Aac) => "AAC",
        Some(Codec::Alac) => "ALAC",
        Some(Codec::Vorbis) => "Vorbis",
        Some(Codec::Opus) => "Opus",
        Some(Codec::Speex) => "Speex",
        Some(Codec::Ac3) => "AC-3",
        Some(Codec::Eac3) => "E-AC-3",
        Some(Codec::Wma) => "WMA",
        Some(Codec::WavPack) => "WavPack",
        Some(Codec::Musepack) => "Musepack",
        Some(Codec::Ape) => "APE",
        Some(Codec::Other) | None => "?",
    }
}

fn khz(hz: u32) -> f64 {
    (f64::from(hz) / 100.0).round() / 10.0
}

/// Why the ranking puts a file ahead of another (`review:reason.<code>`).
pub fn from_rank(reason: &Rank) -> Reason {
    match reason {
        Rank::OnlyFile => fact("review:reason.onlyFile"),
        Rank::NotOnDisk => fact("review:reason.notOnDisk"),
        Rank::BetterVerdict { .. } => fact("review:reason.betterVerdict"),
        Rank::Playable => fact("review:reason.playable"),
        Rank::HigherCutoff {
            best_hz: Some(best),
            other_hz: Some(other),
        } => number(
            number(fact("review:reason.higherCutoff"), "bestKhz", khz(*best)),
            "otherKhz",
            khz(*other),
        ),
        Rank::HigherCutoff { best_hz, .. } => number(
            fact("review:reason.higherCutoffMeasured"),
            "bestKhz",
            best_hz.map_or(0.0, khz),
        ),
        Rank::FewerDecodeErrors { best, other } => number(
            number(
                fact("review:reason.fewerDecodeErrors"),
                "best",
                f64::from(*best),
            ),
            "other",
            f64::from(*other),
        ),
        Rank::BetterFormat { best, other } => better_format(best, other),
        Rank::MoreTags { .. } => fact("review:reason.moreTags"),
        Rank::HasArt => fact("review:reason.hasArt"),
        Rank::KnownToRekordbox { .. } => fact("review:reason.knownToRekordbox"),
        Rank::BetterPlace { .. } => fact("review:reason.betterPlace"),
        Rank::Tied => fact("review:reason.tied"),
    }
}

fn better_format(best: &Format, other: &Format) -> Reason {
    match (best.codec == other.codec, best.kbps, other.kbps) {
        // The same codec: only the bitrate separates them.
        (true, Some(best), Some(other)) => number(
            number(
                fact("review:reason.betterBitrate"),
                "bestKbps",
                f64::from(best),
            ),
            "otherKbps",
            f64::from(other),
        ),
        _ => text(
            text(
                fact("review:reason.betterFormat"),
                "best",
                codec_label(best.codec),
            ),
            "other",
            codec_label(other.codec),
        ),
    }
}

/// The method a relink was made by, as a key (`review:relink.<method>`).
pub fn from_relink(method: &str) -> Reason {
    let code = match method {
        "filename_only" => "filenameOnly",
        "unique_duration" => "uniqueDuration",
        "fingerprint" => "fingerprint",
        "filename_duration" => "filenameDuration",
        "user" => "user",
        _ => "other",
    };
    fact(&format!("review:relink.{code}"))
}

/// The grouping rules' reasons for a pair, from the stored JSON array
/// (deciding reason first), as `review:doubt.<code>` with no parameters: a
/// code that is not a plain word is dropped. At most [`MAX_DOUBTS`].
pub fn from_grouping(stored: &str) -> Vec<Reason> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(stored)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.get("code")?.as_str())
        .filter(|code| code.chars().all(|c| c.is_ascii_alphanumeric()))
        .filter_map(|code| Reason::fact(&format!("review:doubt.{code}")).ok())
        .take(MAX_DOUBTS)
        .collect()
}
