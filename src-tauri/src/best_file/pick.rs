//! Picking each tag and the cover art from the files of one track (1bC-3).
//! The rule is in the module docs: the best-ranked file with a value wins,
//! and every other file's differing value is returned with the pick.

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use super::rank::sorted;
use super::{Art, FileFacts};
use crate::tags::TagFields;

/// The tag fields that are picked: the ones [`TagFields`] has. BPM, key and
/// energy aren't here: `analysis` is their source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TagField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Comment,
    Composer,
    Remixer,
    Label,
    Isrc,
    Year,
    TrackNo,
    DiscNo,
}

impl TagField {
    /// Every picked field, in the order [`Picks::fields`] lists them.
    pub const ALL: [TagField; 13] = [
        TagField::Title,
        TagField::Artist,
        TagField::Album,
        TagField::AlbumArtist,
        TagField::Genre,
        TagField::Comment,
        TagField::Composer,
        TagField::Remixer,
        TagField::Label,
        TagField::Isrc,
        TagField::Year,
        TagField::TrackNo,
        TagField::DiscNo,
    ];

    /// The file's value of this field, if it has one: a blank text, or a
    /// number that isn't positive, is no value.
    fn value(self, tags: &TagFields) -> Option<FieldValue> {
        let text = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(|text| FieldValue::Text {
                    text: text.to_owned(),
                })
        };
        let number = |value: Option<i64>| {
            value
                .filter(|n| *n > 0)
                .map(|number| FieldValue::Number { number })
        };
        match self {
            TagField::Title => text(&tags.title),
            TagField::Artist => text(&tags.artist),
            TagField::Album => text(&tags.album),
            TagField::AlbumArtist => text(&tags.album_artist),
            TagField::Genre => text(&tags.genre),
            TagField::Comment => text(&tags.comment),
            TagField::Composer => text(&tags.composer),
            TagField::Remixer => text(&tags.remixer),
            TagField::Label => text(&tags.label),
            TagField::Isrc => text(&tags.isrc),
            TagField::Year => number(tags.year),
            TagField::TrackNo => number(tags.track_no),
            TagField::DiscNo => number(tags.disc_no),
        }
    }
}

/// The fields tag completeness counts (ranking step 7): the descriptive
/// tags a DJ's library is searched and sorted by. Comment is left out (it
/// holds URLs and rip notes), and so are composer, remixer, ISRC and the
/// numbers of track and disc. A proposal.
pub const COMPLETENESS_FIELDS: [TagField; 6] = [
    TagField::Title,
    TagField::Artist,
    TagField::Album,
    TagField::Genre,
    TagField::Label,
    TagField::Year,
];

/// How many of [`COMPLETENESS_FIELDS`] the tags have a value for.
pub(super) fn tags_filled(tags: &TagFields) -> u32 {
    COMPLETENESS_FIELDS
        .iter()
        .filter(|field| field.value(tags).is_some())
        .count() as u32
}

/// A tag's value: text, trimmed, or a positive number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FieldValue {
    Text { text: String },
    Number { number: i64 },
}

impl FieldValue {
    /// What two values are compared by: text without regard to case,
    /// spacing or Unicode form.
    fn key(&self) -> String {
        match self {
            FieldValue::Text { text } => text
                .nfc()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
            FieldValue::Number { number } => number.to_string(),
        }
    }
}

/// Why a value was taken from the file it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PickReason {
    /// Only one file has a value.
    OnlyValue,
    /// Several files have one, and it's the same value.
    FilesAgree,
    /// The files differ, and the best file has a value: it wins.
    FromBestFile,
    /// The files differ, and the best file has none: the best of the files
    /// that do.
    BestFileBlank,
}

/// A value another file has instead of the picked one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disagreement {
    pub file_id: i64,
    pub value: FieldValue,
}

/// The value picked for one tag field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldPick {
    pub field: TagField,
    pub value: FieldValue,
    /// The file it was taken from.
    pub from_file: i64,
    pub reason: PickReason,
    /// Every file with a different value, best-ranked first. Empty when the
    /// files agree.
    pub disagreements: Vec<Disagreement>,
}

/// Why a picture was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtReason {
    /// Only one file has art.
    OnlyArt,
    /// Several files have art, and it's the same picture.
    FilesAgree,
    /// The pictures differ; this one has the most pixels, or failing that
    /// the most bytes.
    Largest,
}

/// Art another file has instead of the picked one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DifferentArt {
    pub file_id: i64,
    pub art: Art,
}

/// The cover art picked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtPick {
    pub from_file: i64,
    pub art: Art,
    pub reason: ArtReason,
    /// Every file whose picture isn't this one, best-ranked first.
    pub disagreements: Vec<DifferentArt>,
}

/// Everything picked for a track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Picks {
    /// A field no file has a value for isn't listed. In [`TagField::ALL`]
    /// order.
    pub fields: Vec<FieldPick>,
    /// `None` when no file has art.
    pub art: Option<ArtPick>,
}

impl Picks {
    /// The fields where the files differ.
    pub fn disagreeing_fields(&self) -> impl Iterator<Item = &FieldPick> {
        self.fields
            .iter()
            .filter(|pick| !pick.disagreements.is_empty())
    }
}

/// Picks every tag field and the art from `files`. The same files in any
/// order give the same picks.
pub fn pick(files: &[FileFacts]) -> Picks {
    let ranked = sorted(files);
    let fields = TagField::ALL
        .iter()
        .filter_map(|field| pick_field(*field, &ranked))
        .collect();
    Picks {
        fields,
        art: pick_art(&ranked),
    }
}

/// `ranked` is best first.
fn pick_field(field: TagField, ranked: &[&FileFacts]) -> Option<FieldPick> {
    let holders: Vec<(&FileFacts, FieldValue)> = ranked
        .iter()
        .filter_map(|file| field.value(&file.tags).map(|value| (*file, value)))
        .collect();
    let (from, value) = holders.first()?;
    let picked_key = value.key();
    let disagreements: Vec<Disagreement> = holders
        .iter()
        .filter(|(_, other)| other.key() != picked_key)
        .map(|(file, other)| Disagreement {
            file_id: file.file_id,
            value: other.clone(),
        })
        .collect();
    let reason = if holders.len() == 1 {
        PickReason::OnlyValue
    } else if disagreements.is_empty() {
        PickReason::FilesAgree
    } else if ranked
        .first()
        .is_some_and(|best| best.file_id == from.file_id)
    {
        PickReason::FromBestFile
    } else {
        PickReason::BestFileBlank
    };
    Some(FieldPick {
        field,
        value: value.clone(),
        from_file: from.file_id,
        reason,
        disagreements,
    })
}

fn pick_art(ranked: &[&FileFacts]) -> Option<ArtPick> {
    let holders: Vec<(&FileFacts, &Art)> = ranked
        .iter()
        .filter_map(|file| file.art.as_ref().map(|art| (*file, art)))
        .collect();
    // The first of the largest: `ranked` is best first, so a tie in size
    // goes to the better-ranked file.
    let mut winner = *holders.first()?;
    for candidate in &holders[1..] {
        if size_key(candidate.1) > size_key(winner.1) {
            winner = *candidate;
        }
    }
    let disagreements: Vec<DifferentArt> = holders
        .iter()
        .filter(|(_, art)| !same_picture(winner.1, art))
        .map(|(file, art)| DifferentArt {
            file_id: file.file_id,
            art: (*art).clone(),
        })
        .collect();
    let reason = if holders.len() == 1 {
        ArtReason::OnlyArt
    } else if disagreements.is_empty() {
        ArtReason::FilesAgree
    } else {
        ArtReason::Largest
    };
    Some(ArtPick {
        from_file: winner.0.file_id,
        art: winner.1.clone(),
        reason,
        disagreements,
    })
}

/// Greater is bigger: pixels where both sides are known (else 0), then
/// bytes.
fn size_key(art: &Art) -> (u64, u32) {
    let pixels = match (art.width, art.height) {
        (Some(w), Some(h)) => u64::from(w) * u64::from(h),
        _ => 0,
    };
    (pixels, art.bytes)
}

/// With a hash on both sides, the hashes decide; otherwise the same size in
/// bytes is the same picture (the tags layer records nothing more).
fn same_picture(a: &Art, b: &Art) -> bool {
    match (&a.hash, &b.hash) {
        (Some(x), Some(y)) => x == y,
        _ => a.bytes == b.bytes,
    }
}
