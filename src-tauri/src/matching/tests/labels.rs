//! What a pair's label requires of its measurements (ROADMAP 1.4, 1.5).
//!
//! E3's 43 pairs were labeled by hand as the same recording, a version or
//! different songs, and the versions split into cuts and reworks
//! (`spikes/results/e3-summary.md`). This is what each label demands of
//! [`compare`](crate::matching::compare) and the matching pass. The local
//! corpus holds the real pairs to it ([`super::local_corpus`]), and the
//! made-up library is held to the very same function
//! ([`super::e3_patterns`]), so CI checks the rules the private run uses.
//!
//! Which real pair has which label is private. What a label requires is
//! here, in the open.

use std::collections::BTreeSet;

use crate::fingerprint::Fingerprint;
use crate::matching::index::keys;
use crate::matching::Comparison;

/// A pair's label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    /// The same recording in two files: E3's "same".
    Duplicate,
    /// A version from the same production: extended, radio edit, Clean or
    /// Dirty.
    Cut,
    /// A version from another production: remix, VIP, flip, live, cover,
    /// mashup.
    Rework,
    /// Two different songs.
    Different,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Category::Duplicate,
        Category::Cut,
        Category::Rework,
        Category::Different,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Duplicate => "duplicate",
            Category::Cut => "cut",
            Category::Rework => "rework",
            Category::Different => "different",
        }
    }
}

/// ROADMAP 1.4's duplicate rule, as written, on the main alignment.
pub fn is_duplicate(c: &Comparison) -> bool {
    c.coverage_a >= 0.9 && c.coverage_b >= 0.9 && c.score <= 4.0
}

/// Two files are "the same length" when they differ by under this many
/// seconds. E3's Clean and Dirty pair differed by 0 s.
pub const SAME_LENGTH_SECONDS: f32 = 1.0;

/// Whether the two files are as long as each other, to
/// [`SAME_LENGTH_SECONDS`].
pub fn same_length(c: &Comparison) -> bool {
    let seconds = (c.items_a as f32 - c.items_b as f32).abs() * crate::matching::item_seconds();
    seconds < SAME_LENGTH_SECONDS
}

/// Different songs may line up by chance for this many seconds at most,
/// over every alignment. E3 measured 0 to 1% of a track, about two
/// seconds. Seconds and not a share, because a chance match is a short
/// stretch however long the files are.
pub const CHANCE_SECONDS: f32 = 5.0;

/// How many seconds of the two files line up, over every alignment.
pub fn lined_up_seconds(c: &Comparison) -> f32 {
    c.segments.iter().map(|s| s.items).sum::<usize>() as f32 * crate::matching::item_seconds()
}

/// A cut shorter than its other file must be found in it for at least this
/// share. E3's real cuts were not clean slices: the shorter files were
/// covered 72% and 85%.
pub const CUT_FOUND: f32 = 0.5;

/// The share of the shorter file found in the longer, over every
/// alignment.
pub fn shorter_found(c: &Comparison) -> f32 {
    if c.items_a <= c.items_b {
        c.found_a()
    } else {
        c.found_b()
    }
}

/// What `category` requires that `c` doesn't meet; empty when all is well.
/// `proposed` says whether the matching pass compared the pair at all (the
/// block index made it a candidate).
///
/// - **Duplicate:** the duplicate rule holds, in one alignment (a file that
///   needs two has had something taken out), and the pass proposed it.
/// - **Cut of another length:** never a duplicate; the pass proposed it;
///   at least [`CUT_FOUND`] of the shorter file is found in the longer.
/// - **Cut of the same length** (Clean and Dirty): the audio passes the
///   duplicate rule, and the pass proposed it. Only the names can keep
///   these apart (ROADMAP 1.4), which the local corpus checks beside this.
/// - **Rework:** never a duplicate. How much it shares is anything from
///   nothing to most of the track, so nothing more is required.
/// - **Different:** never a duplicate, and no more than
///   [`CHANCE_SECONDS`] line up.
pub fn unmet(category: Category, c: &Comparison, proposed: bool) -> Vec<&'static str> {
    let mut unmet = Vec::new();
    let mut require = |met: bool, what: &'static str| {
        if !met {
            unmet.push(what);
        }
    };
    match category {
        Category::Duplicate => {
            require(is_duplicate(c), "the duplicate rule must hold");
            require(
                c.segments.iter().all(|s| s.alignment == 0),
                "one alignment must cover it",
            );
            require(proposed, "the pass must propose it");
        }
        Category::Cut if same_length(c) => {
            require(
                is_duplicate(c),
                "a cut of the same length must pass the duplicate rule on audio",
            );
            require(proposed, "the pass must propose it");
        }
        Category::Cut => {
            require(!is_duplicate(c), "a cut is never a duplicate");
            require(proposed, "the pass must propose it");
            require(
                shorter_found(c) >= CUT_FOUND,
                "at least half of the shorter file must be found in the longer",
            );
        }
        Category::Rework => require(!is_duplicate(c), "a rework is never a duplicate"),
        Category::Different => {
            require(!is_duplicate(c), "different songs are never duplicates");
            require(
                lined_up_seconds(c) <= CHANCE_SECONDS,
                "different songs must share next to nothing",
            );
        }
    }
    unmet
}

/// How many block-index keys two fingerprints share. The pass proposes a
/// pair from [`MIN_SHARED_KEYS`](crate::matching::index::MIN_SHARED_KEYS)
/// up.
pub fn shared_keys(a: &Fingerprint, b: &Fingerprint) -> usize {
    let a: BTreeSet<u32> = keys(a.items()).into_iter().collect();
    let b: BTreeSet<u32> = keys(b.items()).into_iter().collect();
    a.intersection(&b).count()
}

/// The share of the lined-up items that are the same bit for bit, 0 to 1:
/// what the block index has to go on. `c` compares `a` with `b`.
pub fn identical_share(c: &Comparison, a: &Fingerprint, b: &Fingerprint) -> f32 {
    let (a, b) = (a.items(), b.items());
    let (mut same, mut all) = (0usize, 0usize);
    for s in &c.segments {
        let lined_up = a[s.offset_a..s.offset_a + s.items]
            .iter()
            .zip(&b[s.offset_b..s.offset_b + s.items]);
        same += lined_up.filter(|(x, y)| x == y).count();
        all += s.items;
    }
    if all == 0 {
        return 0.0;
    }
    same as f32 / all as f32
}

/// How much audio two files share, in the grouping rules' terms (1bB-1).
/// Printed beside each pair for that lane, never asserted here: where the
/// lines sit is the grouping rules' call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The duplicate rule holds.
    Full,
    /// Not full, and at least 90% of one file is found in the other, over
    /// every alignment, with a score of 4 or under.
    OneSided,
    /// Some audio lines up, but neither of the above.
    Partial,
    /// Nothing lines up.
    Nothing,
}

impl Shape {
    pub fn of(c: &Comparison) -> Shape {
        if is_duplicate(c) {
            Shape::Full
        } else if c.found_a().max(c.found_b()) >= 0.9 && c.score <= 4.0 {
            Shape::OneSided
        } else if !c.segments.is_empty() {
            Shape::Partial
        } else {
            Shape::Nothing
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Shape::Full => "full",
            Shape::OneSided => "one-sided",
            Shape::Partial => "partial",
            Shape::Nothing => "none",
        }
    }
}

/// A comparison written by hand: files `items_a` and `items_b` long, whose
/// main alignment lines up `matched` items at `score`, and a second
/// alignment `more` items on top.
fn lined_up(items_a: usize, items_b: usize, matched: usize, score: f64, more: usize) -> Comparison {
    use crate::matching::compare::NO_MATCH_SCORE;
    use crate::matching::Segment;

    let mut segments = Vec::new();
    if matched > 0 {
        segments.push(Segment {
            offset_a: 0,
            offset_b: 0,
            items: matched,
            score,
            alignment: 0,
        });
    }
    if more > 0 {
        segments.push(Segment {
            offset_a: items_a - more,
            offset_b: items_b - more,
            items: more,
            score,
            alignment: 1,
        });
    }
    Comparison {
        items_a,
        items_b,
        coverage_a: matched as f32 / items_a as f32,
        coverage_b: matched as f32 / items_b as f32,
        score: if matched > 0 { score } else { NO_MATCH_SCORE },
        segments,
    }
}

#[test]
fn a_duplicate_label_needs_the_rule_to_hold_in_one_alignment_and_the_pass_to_propose_the_pair() {
    let full = lined_up(1000, 1010, 990, 2.4, 0);
    assert!(unmet(Category::Duplicate, &full, true).is_empty());
    assert!(!unmet(Category::Duplicate, &full, false).is_empty());
    // Covered on one side only, covered weakly, and covered in two pieces.
    for not_one in [
        lined_up(1000, 2000, 990, 2.4, 0),
        lined_up(1000, 1000, 990, 4.5, 0),
        lined_up(1000, 1000, 620, 2.4, 0),
        lined_up(1000, 1000, 920, 2.4, 60),
    ] {
        assert!(
            !unmet(Category::Duplicate, &not_one, true).is_empty(),
            "{not_one:?}"
        );
    }
}

#[test]
fn a_cut_of_another_length_must_be_proposed_and_half_found_and_is_never_a_duplicate() {
    // E3's shapes: 72% of the shorter file in one piece and the rest in
    // another, whatever the score.
    let rough = lined_up(1500, 2200, 1080, 5.5, 330);
    assert!(unmet(Category::Cut, &rough, true).is_empty(), "{rough:?}");
    assert!(!unmet(Category::Cut, &rough, false).is_empty());
    let barely_there = lined_up(1500, 2200, 300, 2.0, 0);
    assert!(!unmet(Category::Cut, &barely_there, true).is_empty());
    // A few seconds longer and otherwise the same: by the audio it's a
    // duplicate, which a cut of another length must not be.
    let merged = lined_up(1500, 1560, 1490, 2.0, 0);
    assert!(!unmet(Category::Cut, &merged, true).is_empty());
}

#[test]
fn a_cut_of_the_same_length_must_pass_the_duplicate_rule_on_audio() {
    let clean_and_dirty = lined_up(1700, 1700, 1700, 0.2, 0);
    assert!(same_length(&clean_and_dirty));
    assert!(unmet(Category::Cut, &clean_and_dirty, true).is_empty());
    assert!(!unmet(Category::Cut, &clean_and_dirty, false).is_empty());
    let something_else = lined_up(1700, 1702, 1000, 0.2, 0);
    assert!(!unmet(Category::Cut, &something_else, true).is_empty());
}

#[test]
fn a_rework_may_share_anything_from_nothing_to_most_of_the_track_but_is_never_a_duplicate() {
    for fine in [
        lined_up(1500, 1400, 0, 0.0, 0),
        lined_up(1500, 1400, 60, 9.0, 0),
        lined_up(1000, 1400, 500, 0.5, 0),
        lined_up(1500, 1500, 900, 8.0, 400),
    ] {
        // Proposed or not: weak matches often aren't.
        assert!(unmet(Category::Rework, &fine, false).is_empty(), "{fine:?}");
    }
    let merged = lined_up(1500, 1500, 1480, 1.0, 0);
    assert!(!unmet(Category::Rework, &merged, true).is_empty());
}

#[test]
fn different_songs_may_line_up_by_chance_for_a_few_seconds_and_no_more() {
    assert!(unmet(Category::Different, &lined_up(1700, 1400, 0, 0.0, 0), false).is_empty());
    assert!(unmet(
        Category::Different,
        &lined_up(1700, 1400, 20, 9.0, 0),
        false
    )
    .is_empty());
    assert!(!unmet(
        Category::Different,
        &lined_up(1700, 1400, 300, 9.0, 0),
        false
    )
    .is_empty());
    assert!(!unmet(
        Category::Different,
        &lined_up(1700, 1400, 40, 9.0, 60),
        false
    )
    .is_empty());
    assert!(!unmet(
        Category::Different,
        &lined_up(1400, 1400, 1400, 0.0, 0),
        true
    )
    .is_empty());
}

#[test]
fn the_audio_reading_is_full_one_sided_partial_or_none_as_the_grouping_rules_define_them() {
    assert_eq!(Shape::of(&lined_up(1000, 1010, 990, 2.4, 0)), Shape::Full);
    // 95% of the shorter file found over two alignments, at a good score.
    let cut = lined_up(2000, 4000, 1700, 3.0, 200);
    assert_eq!(Shape::of(&cut), Shape::OneSided);
    // Found just as whole, but too weakly: the names have to decide.
    let weak_cut = lined_up(1500, 2200, 1080, 5.5, 330);
    assert!(shorter_found(&weak_cut) >= 0.9);
    assert_eq!(Shape::of(&weak_cut), Shape::Partial);
    assert_eq!(
        Shape::of(&lined_up(1500, 1500, 900, 2.0, 0)),
        Shape::Partial
    );
    assert_eq!(Shape::of(&lined_up(1500, 1400, 0, 0.0, 0)), Shape::Nothing);
}
