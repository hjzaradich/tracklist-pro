//! The grouping rules' tests. Every name here is invented.
//!
//! - `table`: one test per row of the decision table in the module docs.
//! - `one_file`: a file's labels are those of its tag and its file name
//!   together, and the two files' labels are compared.
//! - `own_names`: a file's tag and file name set side by side.
//! - `readings`: which reading of the titles (tags, file names) decides.
//! - `measured`: numbers that can't be measurements are refused.
//! - `boundaries`: each threshold (90%, score 4, the shared-audio floor),
//!   on both sides.
//! - `e3_rules`: one test per rule under "Rules from the E3 labels" in
//!   ROADMAP §1.5, named after it.
//! - `shape`: what holds for every decision, and how one is serialized.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

mod boundaries;
mod e3_rules;
mod measured;
mod one_file;
mod own_names;
mod readings;
mod shape;
mod table;

/// A file name that says nothing: no artist, no label, and a title no
/// other file has. Beside any other it reads as "another title", which
/// never decides anything while the tags say more.
fn plain_file_name() -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let mut n = NEXT.fetch_add(1, Ordering::Relaxed);
    // Letters only: digits in a file name can be read as a track number.
    let mut word = String::new();
    for _ in 0..4 {
        word.push(char::from(b'a' + (n % 26) as u8));
        n /= 26;
    }
    format!("untitled {word}.wav")
}

/// A file with a title tag, and a file name that says nothing.
pub(super) fn titled(title: &str) -> FileFacts {
    FileFacts::new(Some(title), None, &plain_file_name())
}

/// A file with a title tag and an artist tag, and a file name that says
/// nothing.
pub(super) fn by(artist: &str, title: &str) -> FileFacts {
    FileFacts::new(Some(title), Some(artist), &plain_file_name())
}

/// A file with no tags, known by its file name alone.
pub(super) fn named(file_name: &str) -> FileFacts {
    FileFacts::new(None, None, file_name)
}

/// A file with a title tag and a file name that says something too.
pub(super) fn tagged_and_named(title: &str, file_name: &str) -> FileFacts {
    FileFacts::new(Some(title), None, file_name)
}

/// Measurements with one alignment: what was found is what the main
/// alignment covered.
pub(super) fn measured(coverage_a: f32, coverage_b: f32, score: f64) -> Measured {
    Measured {
        coverage_a,
        coverage_b,
        score,
        found_a: coverage_a,
        found_b: coverage_b,
        found_score: score,
        matched_items: 2000,
    }
}

/// Coverage well over 90% both ways, a score well under 4.
pub(super) fn full() -> Audio {
    Audio::Compared(measured(0.98, 0.97, 1.5))
}

/// All of A found inside B, which is much longer.
pub(super) fn a_inside_b() -> Audio {
    Audio::Compared(measured(0.99, 0.41, 1.5))
}

/// All of B found inside A, which is much longer.
pub(super) fn b_inside_a() -> Audio {
    Audio::Compared(measured(0.41, 0.99, 1.5))
}

/// About half of each file matched.
pub(super) fn partial() -> Audio {
    Audio::Compared(measured(0.55, 0.48, 2.0))
}

/// The fingerprints were compared, and nothing matched.
pub(super) fn nothing_shared() -> Audio {
    Audio::Compared(Measured {
        coverage_a: 0.0,
        coverage_b: 0.0,
        score: matching::compare::NO_MATCH_SCORE,
        found_a: 0.0,
        found_b: 0.0,
        found_score: matching::compare::NO_MATCH_SCORE,
        matched_items: 0,
    })
}

/// Every kind of audio the table tells apart, with a name for messages.
pub(super) fn every_audio() -> Vec<(&'static str, Audio)> {
    vec![
        ("identical", Audio::Identical),
        ("full", full()),
        ("a inside b", a_inside_b()),
        ("b inside a", b_inside_a()),
        ("partial", partial()),
        ("nothing shared", nothing_shared()),
        ("not compared", Audio::NotCompared),
        ("unknown", Audio::Unknown),
    ]
}

/// The decision for `a` and `b`, after checking that asking about `b` and
/// `a` gives the same one with the sides swapped.
pub(super) fn decided(audio: Audio, a: &FileFacts, b: &FileFacts) -> Decision {
    let pair = Pair::new(audio, a, b);
    let decision = decide(&pair);
    assert_eq!(
        decide(&pair.swapped()),
        decision.clone().swapped(),
        "the answer changed with the order of the files"
    );
    decision
}

/// The codes of a decision's reasons, in order.
pub(super) fn codes(decision: &Decision) -> Vec<&'static str> {
    decision.reasons.iter().map(Reason::code).collect()
}

pub(super) fn cut_link(labels: &[MarkerKind]) -> Outcome {
    Outcome::CutLink {
        labels: labels.to_vec(),
    }
}

pub(super) fn rework_link(labels: &[MarkerKind]) -> Outcome {
    Outcome::ReworkLink {
        labels: labels.to_vec(),
    }
}
