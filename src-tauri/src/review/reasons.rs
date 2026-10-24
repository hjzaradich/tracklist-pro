//! Reasons as codes plus parameters (ROADMAP 1, "No black boxes"): the best
//! file ranking's reasons, the grouping rules' reasons and a relink's
//! method, as [`Reason`]s whose key is an i18n key in the `review` namespace
//! and whose parameters are numbers and format names. The words are in
//! `src/locales/en/review.json`, never here.

use super::held::Holding;
use crate::best_file::{FolderRole, Format, Place, Reason as Rank, VolumeKind};
use crate::read::Codec;
use crate::suggest::Reason;

/// How many grouping reasons an item shows (copy-style: at most three
/// fragments).
pub const MAX_DOUBTS: usize = 3;

/// How many of a pair's reasons the sentence that says what holds a set
/// carries: the sentence is a fragment of its own, so with these it is
/// three (copy-style).
pub const MAX_HELD_DOUBTS: usize = 2;

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
        Rank::BetterPlace { best, other } => {
            let (best, other) = place_codes(best, other);
            text(
                text(fact("review:reason.betterPlace"), "best", best),
                "other",
                other,
            )
        }
        Rank::Tied => fact("review:reason.tied"),
    }
}

/// Every code [`place_codes`] can give (`review:place.<code>`).
pub const PLACE_CODES: [&str; 9] = [
    "connected",
    "notConnected",
    "scanFolder",
    "inboxFolder",
    "internalDrive",
    "externalDrive",
    "networkDrive",
    "addedFirst",
    "addedLater",
];

/// The two places a "better location" reason sets side by side, each as a
/// code for the first thing that separates it from the other, in the order
/// the ranking looks at them: a connected drive, the scan folder over the
/// Inbox, the kind of drive, the folder added first. The words are in the
/// locale file (`review:place.<code>`), never here.
fn place_codes(best: &Place, other: &Place) -> (&'static str, &'static str) {
    let kind = |kind: VolumeKind| match kind {
        VolumeKind::Internal => "internalDrive",
        VolumeKind::External => "externalDrive",
        VolumeKind::Network => "networkDrive",
    };
    if best.volume_online != other.volume_online {
        let name = |online| if online { "connected" } else { "notConnected" };
        (name(best.volume_online), name(other.volume_online))
    } else if best.folder_role != other.folder_role {
        let name = |role| match role {
            FolderRole::Scan => "scanFolder",
            FolderRole::Inbox => "inboxFolder",
        };
        (name(best.folder_role), name(other.folder_role))
    } else if best.volume_kind != other.volume_kind {
        (kind(best.volume_kind), kind(other.volume_kind))
    } else {
        // Only the order the folders were added in is left.
        ("addedFirst", "addedLater")
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

/// The codes of the grouping rules' reasons that the locale files have words
/// for (`review:doubt.<code>`). A held set's reason words a pair's reasons
/// inside one sentence, so a code with no words can't be left out of it by
/// the screen the way a line of its own is: it is left out here. A test
/// holds this list to the locale file.
pub const WORDED_DOUBTS: [&str; 22] = [
    "identicalAudio",
    "sameAudio",
    "versionVeto",
    "sameAudioNamesDisagree",
    "audioInsideOther",
    "audioInsideOtherNamesDisagree",
    "sameTagsPartialAudio",
    "partialAudioNamesUnclear",
    "namesSayCut",
    "namesSayRework",
    "mashupContains",
    "labelWordsDiffer",
    "fingerprintVeto",
    "noFingerprint",
    "notCompared",
    "sameTitleDifferentArtists",
    "mashupPart",
    "namesUnclear",
    "titlesDiffer",
    "namesDiffer",
    "artistsDiffer",
    "creditedToRemixer",
];

/// What holds a held duplicate set (`review:held.<code>`), worded with the
/// names of the files it names, as the comparison's rows show them
/// (`name_of`). A pair's own reasons are the ones a possible version
/// mix-up shows for it (`review:doubt.<code>`, at most [`MAX_HELD_DOUBTS`]),
/// put into the sentence by their codes.
pub fn from_holding(holding: &Holding, name_of: impl Fn(i64) -> String) -> Reason {
    let two = |key: &str, files: &[i64; 2]| {
        text(
            text(fact(key), "file", &name_of(files[0])),
            "other",
            &name_of(files[1]),
        )
    };
    match holding {
        Holding::TwoInLibrary { files } => two("review:held.library", files),
        Holding::KeptSeparate { files } => two("review:held.keptSeparate", files),
        Holding::LinkedVersions { files } => two("review:held.linked", files),
        Holding::TooManyFiles { files, most } => number(
            number(fact("review:held.tooMany"), "files", f64::from(*files)),
            "most",
            f64::from(*most),
        ),
        Holding::Pair { files, reasons } => {
            let codes = pair_codes(reasons);
            let key = match codes.len() {
                0 => "review:held.pair",
                1 => "review:held.pairOne",
                _ => "review:held.pairTwo",
            };
            let mut reason = two(key, files);
            for (name, code) in ["first", "second"].into_iter().zip(&codes) {
                reason = text(reason, name, code);
            }
            reason
        }
    }
}

/// The codes of a stored pair's reasons that have words, the deciding one
/// first, at most [`MAX_HELD_DOUBTS`].
fn pair_codes(stored: &str) -> Vec<String> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(stored)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.get("code")?.as_str())
        .filter(|code| WORDED_DOUBTS.contains(code))
        .map(str::to_owned)
        .take(MAX_HELD_DOUBTS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The English locale file of the review screen.
    fn locale() -> serde_json::Value {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/locales/en/review.json");
        let text = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn the_worded_doubts_are_the_ones_the_locale_file_has_words_for() {
        let locale = locale();
        let mut in_locale: Vec<&str> = locale["doubt"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        in_locale.sort_unstable();
        let mut listed = WORDED_DOUBTS.to_vec();
        listed.sort_unstable();
        assert_eq!(listed, in_locale);
    }

    #[test]
    fn every_sentence_that_says_what_holds_a_set_has_words() {
        let locale = locale();
        let held = locale["held"].as_object().unwrap();
        let name = |file: i64| format!("file {file}");
        let pair = |reasons: &str| Holding::Pair {
            files: [1, 2],
            reasons: reasons.to_owned(),
        };
        for holding in [
            Holding::TwoInLibrary { files: [1, 2] },
            Holding::KeptSeparate { files: [1, 2] },
            Holding::LinkedVersions { files: [1, 2] },
            Holding::TooManyFiles {
                files: 60,
                most: 50,
            },
            pair("[]"),
            pair(r#"[{"code":"sameAudio"}]"#),
            pair(r#"[{"code":"sameAudio"},{"code":"namesDiffer"}]"#),
        ] {
            let reason = from_holding(&holding, name);
            let key = reason.key().strip_prefix("review:held.").unwrap();
            let text = held[key].as_str().unwrap();
            // The sentence uses exactly the parameters the reason carries.
            let mut used: Vec<&str> = text
                .split("{{")
                .skip(1)
                .map(|rest| rest.split(['}', ',']).next().unwrap().trim())
                .collect();
            used.sort_unstable();
            used.dedup();
            let given: Vec<&str> = reason.params().keys().map(String::as_str).collect();
            let mut given_sorted = given.clone();
            given_sorted.sort_unstable();
            assert_eq!(used, given_sorted, "{key}");
        }
    }
}
