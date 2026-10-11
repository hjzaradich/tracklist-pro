//! The made-up library against what E3 measured on real files (1bA-4;
//! `spikes/results/e3-summary.md`, "Fingerprint grading").
//!
//! CI can't run the real pairs ([`super::local_corpus`]), so the made-up
//! library ([`super::corpus`]) has to stand in for them. These tests check
//! that it does: every pair in it is held to the same per-label
//! requirements as the real pairs ([`super::labels::unmet`]), and each
//! kind of pair lands, through the app's real fingerprint code, where E3
//! measured that kind.
//!
//! | E3 measured | Made up as |
//! |---|---|
//! | same: ≥ 98% both ways, score ≤ 2.5, exact copies 0.00 | one song as FLAC, AIFF, WAV, ALAC, MP3 320 and 128, and a rip with lead-in silence |
//! | one mix on two releases, a few seconds apart: still the same | the rip with 1.5 s of lead-in on a one-minute file |
//! | cuts: 72% of an original in its extended mix (50% of the mix); 85% of a radio edit in its original (47%) | an extended mix with a new intro and an added breakdown; a radio edit that jumps to the ending |
//! | Clean vs Dirty: 100% / 100%, score 0.23 | three quarter-second mutes |
//! | a VIP sharing sections: 48% / 36% | a VIP keeping the first half |
//! | most reworks: 0 to 25% | a remix keeping a ten-second hook; a rework keeping five scattered pieces |
//! | a second recording under the same name: 62% / 62%, weakly, then nothing | the first 75 s of a song in another mix, then something else |
//! | different: 0 to 1% | unrelated songs: nothing lines up |
//!
//! # What can't be made up
//!
//! Said plainly, so nobody reads more into a green run than is there:
//!
//! - **A chance match on purpose.** E3 had one pair of different songs
//!   lining up for 1% at a weak score. One happens in the made-up library
//!   too (two unrelated files line up for about three seconds at a score
//!   just under the matcher's limit, on Windows, where the MP3s exist), but
//!   by accident: nothing here can make one on purpose, or say how often a
//!   real library has them. Different songs are allowed
//!   [`super::labels::CHANCE_SECONDS`].
//! - **Why real matches are weak.** A remix that reuses a stem, a second
//!   performance, another master: each lines up at a score of 5 to 10 for
//!   its own reason. Here a weak match is made by laying other audio over
//!   the original, which gives the number and not the cause. How often
//!   real reworks match weakly, and for how long, only the local corpus
//!   shows.
//! - **A short weak match.** Most of E3's reworks that shared anything
//!   shared a few percent at a score close to the matcher's limit. Under
//!   enough other audio to score that, a ten-second hook doesn't line up
//!   at all here, or does by luck; a test on it would pin an accident.
//! - **A weak match the pass still proposes.** The block index needs a few
//!   items that are the same bit for bit. Laying audio over a track wipes
//!   those out much sooner than real masters and encodes do: at a score of
//!   5 the two made-up files share no key at all, while real files at that
//!   score can still be proposed. The one case that needs it (a cut found
//!   whole at a score over 4) is therefore made from fingerprint items, by
//!   flipping bits, and not from audio.
//! - **A real mashup.** E3's mashups shared 0 to 6% with their sources.
//!   The made-up one is two whole songs added together, which lines up
//!   with one of its sources for over half at a score over 9 and with the
//!   other not at all. It's kept as the generator's ground truth has it
//!   (`tools/fixture-gen`), and is held only to "never a duplicate".
//! - **MP3 against MP3 from different sources**, off Windows: the tests'
//!   MP3 encoder is Windows-only, so elsewhere the lossy files are left
//!   out or written as WAV.

use crate::fingerprint::Fingerprint;
use crate::matching::{compare, BlockIndex, Comparison};
use crate::versions::{self, Outcome};

use super::corpus::{
    corpus, CLEAN, DIRTY, EMBER_DEMO, EMBER_STUDIO, EXTENDED, FEN_ORIGINAL, FEN_REWORK, ORIGINAL,
    QUAY_ORIGINAL, QUAY_RADIO, REMIX, RIDGE_EXTENDED, RIDGE_ORIGINAL, VIP,
};
use super::labels::{identical_share, is_duplicate, shorter_found, unmet, Category, Shape};
use super::passes::corpus_pass;
use super::synthetic::{fingerprint, track, worn};

fn named(a: &str, b: &str) -> Comparison {
    compare(corpus().fingerprint(a), corpus().fingerprint(b)).expect("same version")
}

/// Whether the block index makes the two a candidate pair, so that a pass
/// compares them.
fn proposed(a: &Fingerprint, b: &Fingerprint) -> bool {
    let mut index = BlockIndex::new();
    index.insert(0, a.items());
    index.insert(1, b.items());
    index.pairs() == [(0, 1)]
}

fn alignments(c: &Comparison) -> usize {
    c.segments
        .iter()
        .map(|s| usize::from(s.alignment) + 1)
        .max()
        .unwrap_or(0)
}

fn numbers(c: &Comparison) -> String {
    format!(
        "coverage {:.3}/{:.3}, found {:.3}/{:.3}, score {:.2}, {} segments in {} alignments, {}",
        c.coverage_a,
        c.coverage_b,
        c.found_a(),
        c.found_b(),
        c.score,
        c.segments.len(),
        alignments(c),
        Shape::of(c).as_str()
    )
}

#[test]
fn every_pair_of_the_made_up_library_measures_as_its_label_requires_of_the_real_pairs() {
    let passed = corpus_pass();
    let library = corpus();
    let mut labeled: Vec<(Category, &str, &str)> = Vec::new();
    let mut label = |category: Category, pairs: Vec<(&'static str, &'static str)>| {
        labeled.extend(pairs.into_iter().map(|(a, b)| (category, a, b)));
    };
    label(Category::Duplicate, library.duplicate_pairs());
    label(
        Category::Cut,
        library
            .contains
            .iter()
            .map(|c| (c.inner, c.outer))
            .collect(),
    );
    label(Category::Cut, library.rough_cuts.clone());
    label(Category::Cut, library.clean_dirty.clone());
    label(Category::Rework, library.reworks_sharing_audio.clone());
    label(Category::Rework, library.weak_matches.clone());
    label(Category::Rework, library.other_reworks.clone());
    // Every pair the library doesn't call related is two different songs.
    let mut different = Vec::new();
    for (i, a) in library.files.iter().enumerate() {
        for b in &library.files[i + 1..] {
            if !library.related(a.name, b.name) {
                different.push((a.name, b.name));
            }
        }
    }
    label(Category::Different, different);

    let mut failures = Vec::new();
    let mut seen = [0usize; 4];
    for (category, a, b) in labeled {
        // What the pass stored is what grouping will read; a pair it never
        // proposed is compared here, as the local corpus does.
        let stored = passed.measured(a, b);
        let was_proposed = stored.is_some();
        let c = stored.unwrap_or_else(|| named(a, b));
        let wrong = unmet(category, &c, was_proposed);
        seen[Category::ALL.iter().position(|k| *k == category).unwrap()] += 1;
        if category != Category::Different {
            println!(
                "{:<9} {a} | {b}: {}, proposed {was_proposed}",
                category.as_str(),
                numbers(&c)
            );
        }
        if !wrong.is_empty() {
            failures.push(format!(
                "{} {a} | {b}: {}; unmet: {}",
                category.as_str(),
                numbers(&c),
                wrong.join("; ")
            ));
        }
    }
    println!("pairs per label (duplicate, cut, rework, different): {seen:?}");
    assert!(
        seen.iter().all(|&n| n > 0),
        "every label has pairs: {seen:?}"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn made_up_duplicates_are_covered_and_score_as_e3s_same_pairs_did() {
    // E3: at least 98% both ways and a score of 2.5 or under. The made-up
    // ones get a little more room on both: the rip with lead-in silence is
    // covered 97.5%, because 1.5 s is a bigger share of one minute than a
    // few seconds are of a real track, and against the 128 kbps MP3 it
    // scores 2.8.
    for (a, b) in corpus().duplicate_pairs() {
        let c = named(a, b);
        println!("{a} | {b}: {}", numbers(&c));
        assert!(
            c.coverage_a >= 0.97 && c.coverage_b >= 0.97,
            "{a} | {b}: {}",
            numbers(&c)
        );
        assert!(c.score <= 3.0, "{a} | {b}: {}", numbers(&c));
    }
    // Exact copies score 0.00, as E3's did.
    let exact = named("glasswing.aiff", "glasswing.wav");
    assert_eq!(exact.score, 0.0, "{}", numbers(&exact));
}

#[test]
fn made_up_cuts_that_are_not_clean_slices_line_up_as_e3s_two_real_cuts_did() {
    // E3: an original 72% covered by its extended mix (itself 50%), and a
    // radio edit 85% covered by its original (47%). The main alignment
    // holds one piece; the rest of the shorter file is found by another.
    for (shorter, longer, of_shorter, of_longer) in [
        (RIDGE_ORIGINAL, RIDGE_EXTENDED, 0.72, 0.50),
        (QUAY_RADIO, QUAY_ORIGINAL, 0.85, 0.47),
    ] {
        let c = named(shorter, longer);
        println!("{shorter} in {longer}: {}", numbers(&c));
        let label = format!("{shorter} in {longer}: {}", numbers(&c));
        assert!((c.coverage_a - of_shorter).abs() < 0.08, "{label}");
        assert!((c.coverage_b - of_longer).abs() < 0.08, "{label}");
        assert!(!is_duplicate(&c), "{label}");
        assert!(
            alignments(&c) >= 2,
            "one alignment can't hold both pieces: {label}"
        );
        assert!(
            shorter_found(&c) >= 0.9,
            "all of the shorter file is in there: {label}"
        );
        assert!(c.score <= 4.0, "{label}");
        assert_eq!(Shape::of(&c), Shape::OneSided, "{label}");
    }
}

#[test]
fn a_cut_from_another_master_is_found_whole_at_a_score_over_4_which_makes_it_a_partial_match() {
    // A real cut can be all there and still differ too much, item by item,
    // for the grouping rules' "one-sided" (score 4 or under): the two
    // files come from different masters or encodes. Made from fingerprint
    // items (see "What can't be made up"): the original's own, with 1 to 12
    // bits flipped in nine of every ten, laid out as the made-up extended
    // mix is: an intro, 55 of the original's 76 seconds, a breakdown, the
    // rest.
    let original = corpus().fingerprint(RIDGE_ORIGINAL);
    let items = worn(original.items(), 16, 10, 12);
    let split = items.len() * 55 / 76;
    let other_master = fingerprint(
        [
            track(601, 145),
            items[..split].to_vec(),
            track(602, 130),
            items[split..].to_vec(),
        ]
        .concat(),
    );
    let c = compare(original, &other_master).unwrap();
    println!(
        "original in an extended mix from another master: {}, identical {:.3}",
        numbers(&c),
        identical_share(&c, original, &other_master)
    );
    assert!((c.coverage_a - 0.72).abs() < 0.08, "{}", numbers(&c));
    assert!((c.coverage_b - 0.50).abs() < 0.08, "{}", numbers(&c));
    assert!(shorter_found(&c) >= 0.9, "{}", numbers(&c));
    assert!(c.score > 4.0 && c.score < 7.0, "{}", numbers(&c));
    assert_eq!(Shape::of(&c), Shape::Partial, "{}", numbers(&c));
    // The pass still proposes it, and it meets what a cut's label requires:
    // linking it is left to the names.
    assert!(proposed(original, &other_master));
    assert_eq!(unmet(Category::Cut, &c, true), Vec::<&str>::new());
}

#[test]
fn made_up_clean_and_dirty_are_as_alike_as_e3s_pair() {
    // E3: 100% / 100%, score 0.23.
    let c = named(DIRTY, CLEAN);
    println!("dirty | clean: {}", numbers(&c));
    assert!(
        c.coverage_a >= 0.98 && c.coverage_b >= 0.98,
        "{}",
        numbers(&c)
    );
    assert!(c.score < 1.0, "{}", numbers(&c));
    assert_eq!(Shape::of(&c), Shape::Full);
}

#[test]
fn a_made_up_vip_shares_sections_with_its_original_as_e3s_did() {
    // E3: 48% / 36%, at a score near 0: the shared sections are the very
    // same audio. Between a quarter and two thirds counts as that shape.
    for (a, b) in [(ORIGINAL, VIP), (EXTENDED, VIP)] {
        let c = named(a, b);
        println!("{a} | {b}: {}", numbers(&c));
        for coverage in [c.coverage_a, c.coverage_b] {
            assert!(
                (0.25..=0.66).contains(&coverage),
                "{a} | {b}: {}",
                numbers(&c)
            );
        }
        assert!(c.score <= 2.5, "{a} | {b}: {}", numbers(&c));
        assert_eq!(Shape::of(&c), Shape::Partial, "{a} | {b}");
    }
}

#[test]
fn made_up_reworks_that_keep_a_hook_or_scattered_pieces_share_a_quarter_or_less_as_most_of_e3s_did()
{
    // E3: most reworks 0 to 25%.
    let hook = named(ORIGINAL, REMIX);
    let scattered = named(FEN_ORIGINAL, FEN_REWORK);
    for (what, c) in [("hook", &hook), ("scattered pieces", &scattered)] {
        println!("{what}: {}", numbers(c));
        assert!(
            c.coverage_a > 0.05 && c.coverage_a <= 0.25,
            "{what}: {}",
            numbers(c)
        );
        assert!(
            c.coverage_b > 0.05 && c.coverage_b <= 0.25,
            "{what}: {}",
            numbers(c)
        );
        assert_eq!(Shape::of(c), Shape::Partial, "{what}");
    }
    // The pieces line up where they were: several stretches, one shift.
    assert!(scattered.segments.len() >= 4, "{}", numbers(&scattered));
    assert_eq!(alignments(&scattered), 1, "{}", numbers(&scattered));
}

#[test]
fn another_recording_under_the_same_name_lines_up_weakly_for_its_first_part_and_is_never_proposed()
{
    // E3 had a second recording under identical tags: it lined up with
    // the first for well over half at a weak score, then not at all.
    // ROADMAP 1.5 wants a partial match between files with identical tags
    // sent to review. Pinned, because real pairs of this kind behave the
    // same way: the two share so few items bit for bit that the pass never
    // compares them, so no partial match is stored for the grouping rules
    // to see.
    //
    // This test must flip when the grouping job compares pairs linked by
    // name directly: a pair like this then gets a stored partial match,
    // and the "nothing stored" assertion below has to say so.
    let studio = corpus().fingerprint(EMBER_STUDIO);
    let demo = corpus().fingerprint(EMBER_DEMO);
    let c = compare(studio, demo).unwrap();
    println!(
        "studio | demo: {}, identical {:.3}",
        numbers(&c),
        identical_share(&c, studio, demo)
    );
    for coverage in [c.coverage_a, c.coverage_b] {
        assert!((0.4..=0.75).contains(&coverage), "{}", numbers(&c));
    }
    assert!(c.score > 5.0 && c.score < 10.0, "{}", numbers(&c));
    assert!(!is_duplicate(&c));
    assert_eq!(Shape::of(&c), Shape::Partial);
    // The main alignment lines them up from the start, at no shift to
    // speak of.
    assert!(
        c.segments
            .iter()
            .all(|s| s.alignment > 0 || s.shift().abs() <= 2),
        "{c:?}"
    );
    assert!(!proposed(studio, demo), "{}", numbers(&c));
    assert_eq!(corpus_pass().measured(EMBER_STUDIO, EMBER_DEMO), None);
    // And the names give nothing away: the same title tag on both, and
    // file names that differ in their extension only. So what the grouping
    // rules get is "the same by name, never compared".
    let title = versions::parse_title("Ember");
    let by_tags = versions::compare(&title, &title).outcome;
    let by_files = versions::compare(
        &versions::parse_file_name(EMBER_STUDIO),
        &versions::parse_file_name(EMBER_DEMO),
    )
    .outcome;
    assert_eq!(
        (by_tags, by_files),
        (Outcome::SameVersion, Outcome::SameVersion)
    );
}

#[test]
fn a_full_audio_match_can_be_a_cut_by_its_tags_while_its_file_names_look_unrelated_and_no_reading_says_same(
) {
    // What E3's Clean and Dirty pair showed, with made-up names: the audio
    // passes the duplicate rule, the title tags say "cut", and the file
    // names are no help, because one of them was never named after the
    // track. One reading says cut and none says same, which is all that
    // stands between the pair and a false merge; a rule that asked the file
    // names alone would not see a cut here.
    let audio = named(DIRTY, CLEAN);
    assert_eq!(Shape::of(&audio), Shape::Full, "{}", numbers(&audio));
    let by_tags = versions::compare(
        &versions::parse_title("Neon Moth (Dirty)"),
        &versions::parse_title("Neon Moth (Clean)"),
    )
    .outcome;
    let by_files = versions::compare(
        &versions::parse_file_name("Solvane - Neon Moth (Dirty).mp3"),
        &versions::parse_file_name("Solvane - Radio Promo Seven.mp3"),
    )
    .outcome;
    assert_eq!(by_tags, Outcome::DifferentCut);
    assert_eq!(by_files, Outcome::DifferentBase);
}
