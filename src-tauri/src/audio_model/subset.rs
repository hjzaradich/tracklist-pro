//! Which of the model's 527 label scores the app keeps (ROADMAP 3.8): the
//! AudioSet music GENRE labels (Pop music to Independent music) and music
//! MOOD labels (Happy music to Scary music), 57 in all, in the model's own
//! order.
//!
//! The scores are kept as the model gives them (logits, before any
//! sigmoid), as little-endian f32, in the order of [`SUBSET`]. The list has
//! its own version, folded into [`model_id`], so changing it makes every
//! track listened to again.

use super::MODEL_SHA256;

/// The version of [`SUBSET`]. Bump it when the list changes.
pub const SUBSET_VERSION: u8 = 1;

/// How many scores a track keeps.
pub const SUBSET_LEN: usize = 57;

/// The kept labels: index in the model's 527 outputs (and in
/// `resources/models/ced-tiny/labels.json`), and that label's name. A test
/// checks every name against the bundled `labels.json`.
pub const SUBSET: [(usize, &str); SUBSET_LEN] = [
    (216, "Pop music"),
    (217, "Hip hop music"),
    (218, "Beatboxing"),
    (219, "Rock music"),
    (220, "Heavy metal"),
    (221, "Punk rock"),
    (222, "Grunge"),
    (223, "Progressive rock"),
    (224, "Rock and roll"),
    (225, "Psychedelic rock"),
    (226, "Rhythm and blues"),
    (227, "Soul music"),
    (228, "Reggae"),
    (229, "Country"),
    (230, "Swing music"),
    (231, "Bluegrass"),
    (232, "Funk"),
    (233, "Folk music"),
    (234, "Middle Eastern music"),
    (235, "Jazz"),
    (236, "Disco"),
    (237, "Classical music"),
    (238, "Opera"),
    (239, "Electronic music"),
    (240, "House music"),
    (241, "Techno"),
    (242, "Dubstep"),
    (243, "Drum and bass"),
    (244, "Electronica"),
    (245, "Electronic dance music"),
    (246, "Ambient music"),
    (247, "Trance music"),
    (248, "Music of Latin America"),
    (249, "Salsa music"),
    (250, "Flamenco"),
    (251, "Blues"),
    (252, "Music for children"),
    (253, "New-age music"),
    (254, "Vocal music"),
    (255, "A capella"),
    (256, "Music of Africa"),
    (257, "Afrobeat"),
    (258, "Christian music"),
    (259, "Gospel music"),
    (260, "Music of Asia"),
    (261, "Carnatic music"),
    (262, "Music of Bollywood"),
    (263, "Ska"),
    (264, "Traditional music"),
    (265, "Independent music"),
    (276, "Happy music"),
    (277, "Funny music"),
    (278, "Sad music"),
    (279, "Tender music"),
    (280, "Exciting music"),
    (281, "Angry music"),
    (282, "Scary music"),
];

/// Names the model and the list that made a stored result, e.g.
/// `ced-tiny@a455633f/labels1`: a result is current only for the same id.
pub fn model_id() -> String {
    format!("ced-tiny@{}/labels{SUBSET_VERSION}", &MODEL_SHA256[..8])
}

/// The kept scores out of all 527, in [`SUBSET`]'s order.
pub fn pick(scores: &[f32]) -> Vec<f32> {
    SUBSET
        .iter()
        .map(|&(i, _)| scores.get(i).copied().unwrap_or(f32::NAN))
        .collect()
}

/// `values` as little-endian f32 bytes.
pub fn to_blob(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// The f32s in little-endian `bytes`; `None` unless it is a whole number of them.
pub fn from_blob(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}
