//! Comparing one pair on full-track fingerprints (1bA-2; ROADMAP 1.4).

use crate::fingerprint::{CompareError, Fingerprint};
use crate::matching::compare::{MAX_ALIGNMENTS, MIN_SECONDARY_ITEMS, NO_MATCH_SCORE};
use crate::matching::{compare, Comparison};

use super::corpus::corpus;
use super::{audio, cut, items, pcm, print, RATE};

fn compared(a: &Fingerprint, b: &Fingerprint) -> Comparison {
    compare(a, b).expect("same version")
}

fn named(a: &str, b: &str) -> Comparison {
    compared(corpus().fingerprint(a), corpus().fingerprint(b))
}

/// ROADMAP 1.4's duplicate rule, as written, on the comparison's numbers.
fn is_duplicate(c: &Comparison) -> bool {
    c.coverage_a >= 0.9 && c.coverage_b >= 0.9 && c.score <= 4.0
}

/// How far into B the first matched stretch puts A's start, in items.
fn start_of_a_in_b(c: &Comparison) -> f64 {
    let first = c.segments.first().expect("something matched");
    first.offset_b as f64 - first.offset_a as f64
}

#[test]
fn identical_audio_in_two_formats_is_fully_covered_both_ways_with_a_low_score() {
    for (a, b) in corpus().duplicate_pairs() {
        let c = named(a, b);
        println!(
            "{a} vs {b}: {:.3} / {:.3}, score {:.2}",
            c.coverage_a, c.coverage_b, c.score
        );
        assert!(is_duplicate(&c), "{a} vs {b}: {c:?}");
        assert!(
            c.coverage_a >= 0.95 && c.coverage_b >= 0.95,
            "{a} vs {b}: {c:?}"
        );
        assert!(
            c.segments.iter().all(|s| s.alignment == 0),
            "{a} vs {b} needs one alignment only: {c:?}"
        );
    }
}

#[test]
fn a_rip_with_lead_in_silence_lines_up_shifted_by_the_silence() {
    let c = named("glasswing.wav", "glasswing (lead-in).flac");
    let shift = start_of_a_in_b(&c);
    assert!(
        (shift - items(1.5)).abs() <= 2.0,
        "the audio starts 1.5 s into the rip, found {shift} items in: {c:?}"
    );
}

#[test]
fn a_shortened_cut_is_found_whole_inside_the_longer_file_at_the_right_offset() {
    for contains in &corpus().contains {
        let (inner, outer) = (contains.inner, contains.outer);
        let c = named(inner, outer);
        println!(
            "{inner} in {outer}: {:.3} / {:.3}, score {:.2}, {:?}",
            c.coverage_a, c.coverage_b, c.score, c.segments
        );
        assert!(c.coverage_a >= 0.9, "the cut is all in there: {c:?}");
        let share = c.items_a as f32 / c.items_b as f32;
        assert!(
            c.coverage_b < 0.9 && (c.coverage_b - share).abs() < 0.08,
            "only the cut's share of the longer file ({share}) is covered: {c:?}"
        );
        assert!(!is_duplicate(&c), "a cut is never a duplicate: {c:?}");
        let at = start_of_a_in_b(&c);
        assert!(
            (at - items(contains.at_seconds)).abs() <= 3.0,
            "{inner} starts {} s into {outer}, found at item {at}: {c:?}",
            contains.at_seconds
        );
    }
}

#[test]
fn swapping_the_two_files_swaps_the_two_coverages_and_the_offsets() {
    for contains in &corpus().contains {
        let one_way = named(contains.inner, contains.outer);
        let other_way = named(contains.outer, contains.inner);
        let back = other_way.swapped();
        assert_eq!(
            (back.items_a, back.items_b),
            (one_way.items_a, one_way.items_b)
        );
        assert!(
            (back.coverage_a - one_way.coverage_a).abs() < 0.02,
            "{back:?} {one_way:?}"
        );
        assert!(
            (back.coverage_b - one_way.coverage_b).abs() < 0.02,
            "{back:?} {one_way:?}"
        );
        assert!(
            (start_of_a_in_b(&back) - start_of_a_in_b(&one_way)).abs() <= 2.0,
            "{back:?} {one_way:?}"
        );
    }
}

#[test]
fn a_pair_that_only_diverges_after_two_minutes_is_not_reported_as_full_coverage() {
    // E3 pair #42: the same for about two minutes, then different. A
    // short-window comparison would call these the same file.
    let opening = audio::song(41, 125.0, RATE);
    let a = [opening.clone(), audio::song(42, 60.0, RATE)].concat();
    let b = [opening, audio::song(43, 60.0, RATE)].concat();
    let (a, b) = (print(&audio::wav(&pcm(&a))), print(&audio::flac(&pcm(&b))));
    let c = compared(&a, &b);
    println!("same for 125 s of 185 s: {c:?}");
    let shared = 125.0 / 185.0;
    assert!((c.coverage_a - shared).abs() < 0.05, "{c:?}");
    assert!((c.coverage_b - shared).abs() < 0.05, "{c:?}");
    assert!(!is_duplicate(&c), "{c:?}");
    assert!(
        (c.found_a() - shared).abs() < 0.05,
        "no other alignment adds to it: {c:?}"
    );
    // The matched part is the opening, and stops where they part.
    let end = c
        .segments
        .iter()
        .map(|s| s.offset_a + s.items)
        .max()
        .unwrap();
    assert!((end as f64 - items(125.0)).abs() <= 24.0, "{c:?}");
    assert!(c.segments.iter().all(|s| s.offset_a == s.offset_b), "{c:?}");
}

#[test]
fn unrelated_tracks_have_no_coverage() {
    for (a, b) in [
        ("core (a).wav", "core (b).wav"),
        ("glasswing.wav", "moth (dirty).flac"),
        ("glasswing.flac", "harbor (extended).flac"),
        ("core (a).wav", "harbor (original).flac"),
        ("moth (clean).flac", "harbor (radio edit)"),
    ] {
        let c = named(a, b);
        assert_eq!(
            (c.coverage_a, c.coverage_b),
            (0.0, 0.0),
            "{a} vs {b}: {c:?}"
        );
        assert_eq!((c.found_a(), c.found_b()), (0.0, 0.0), "{a} vs {b}: {c:?}");
        assert!(c.segments.is_empty(), "{a} vs {b}: {c:?}");
        assert_eq!(c.score, NO_MATCH_SCORE);
    }
}

/// `glasswing.wav`'s audio played `percent` faster (and higher).
fn faster(percent: f64) -> Fingerprint {
    let mut copy = audio::pcm(101, 60.0, RATE, 1);
    copy.rate = (f64::from(RATE) * (1.0 + percent / 100.0)).round() as u32;
    print(&audio::wav(&copy))
}

#[test]
fn a_copy_at_another_speed_or_pitch_is_never_a_duplicate_because_what_lines_up_differs_too_much() {
    // Pinned as measured. A resampled copy (a vinyl rip at the wrong speed,
    // a pitched-up upload) drifts out of step with the original. A fraction
    // of a percent still lines up end to end, but with a score well over 4;
    // at 2% only a stretch lines up, barely under the matcher's limit of
    // 10. Neither passes the duplicate rule: names have to vouch for these.
    let original = corpus().fingerprint("glasswing.wav");
    let slightly = compared(original, &faster(0.3));
    println!("0.3% faster: {slightly:?}");
    assert!(
        slightly.coverage_a >= 0.9 && slightly.coverage_b >= 0.9,
        "{slightly:?}"
    );
    assert!(slightly.score > 5.0, "{slightly:?}");
    assert!(!is_duplicate(&slightly), "{slightly:?}");
    for percent in [2.0, 6.0] {
        let c = compared(original, &faster(percent));
        println!("{percent}% faster: {c:?}");
        assert!(!is_duplicate(&c), "{c:?}");
        assert!(
            c.coverage_a < 0.5 && c.coverage_b < 0.5,
            "{percent}% faster: {c:?}"
        );
        assert!(c.score > 7.0, "{percent}% faster: {c:?}");
    }
}

#[test]
fn clean_and_dirty_compare_as_the_same_audio_so_only_the_version_check_can_tell_them_apart() {
    // ROADMAP 1.4: "Clean vs Dirty fingerprint as 100% identical". Pinned,
    // so the grouping rules know the veto is theirs to make.
    let c = named("moth (dirty).flac", "moth (clean).flac");
    println!("dirty vs clean: {c:?}");
    assert!(is_duplicate(&c), "{c:?}");
}

#[test]
fn a_vip_that_keeps_the_first_half_is_covered_for_that_half_only() {
    let c = named("harbor (original).flac", "harbor (vip).wav");
    println!("original vs vip: {c:?}");
    assert!((c.coverage_a - 0.5).abs() < 0.06, "{c:?}");
    assert!((c.coverage_b - 0.5).abs() < 0.06, "{c:?}");
    assert!(!is_duplicate(&c), "{c:?}");
    let end = c
        .segments
        .iter()
        .map(|s| s.offset_a + s.items)
        .max()
        .unwrap();
    assert!(
        end as f64 <= items(40.0) + 8.0,
        "only the first 40 s match: {c:?}"
    );
    assert!(c.segments.iter().all(|s| s.offset_a == s.offset_b), "{c:?}");
}

#[test]
fn a_cut_with_a_part_taken_out_is_found_in_two_pieces_but_is_not_a_duplicate() {
    // An edit that drops 20 s from the middle lines up with the mix at two
    // different shifts.
    let mix = audio::song(51, 100.0, RATE);
    let edit = [cut(&mix, 0.0, 35.0), cut(&mix, 55.0, 100.0)].concat();
    let (mix, edit) = (
        print(&audio::flac(&pcm(&mix))),
        print(&audio::wav(&pcm(&edit))),
    );
    let c = compared(&edit, &mix);
    println!("edit vs mix: {c:?}");
    // The main alignment holds one piece only.
    assert!(c.coverage_a < 0.7, "{c:?}");
    assert!(!is_duplicate(&c), "something was taken out: {c:?}");
    // Every alignment together finds nearly all of the edit in the mix.
    assert!(c.found_a() >= 0.9, "{c:?}");
    assert!((c.found_b() - 0.8).abs() < 0.08, "{c:?}");
    // How much later each piece sits in the mix than in the edit.
    let mut later: Vec<(u8, f64)> = c
        .segments
        .iter()
        .map(|s| (s.alignment, -(s.shift() as f64)))
        .collect();
    later.dedup_by_key(|s| s.0);
    later.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert_eq!(later.len(), 2, "two alignments: {c:?}");
    // The piece before the gap sits at the same place; the one after it,
    // 20 s later.
    assert!(later[0].1.abs() <= 3.0, "{c:?}");
    assert!((later[1].1 - items(20.0)).abs() <= 3.0, "{c:?}");
}

#[test]
fn no_part_of_either_file_is_counted_twice_and_secondary_matches_are_never_short() {
    let files = &corpus().files;
    for (i, a) in files.iter().enumerate() {
        for b in &files[i + 1..] {
            let c = compared(&a.fingerprint, &b.fingerprint);
            let label = format!("{} vs {}: {c:?}", a.name, b.name);
            assert_eq!(c.items_a, a.fingerprint.items().len());
            assert_eq!(c.items_b, b.fingerprint.items().len());
            for side in [0, 1] {
                let (len, mut spans): (usize, Vec<(usize, usize)>) = if side == 0 {
                    (
                        c.items_a,
                        c.segments.iter().map(|s| (s.offset_a, s.items)).collect(),
                    )
                } else {
                    (
                        c.items_b,
                        c.segments.iter().map(|s| (s.offset_b, s.items)).collect(),
                    )
                };
                spans.sort_unstable();
                let mut at = 0;
                for (start, items) in spans {
                    assert!(start >= at, "overlap: {label}");
                    at = start + items;
                }
                assert!(at <= len, "past the end: {label}");
            }
            for s in &c.segments {
                assert!(s.score < 10.0, "{label}");
                assert!((s.alignment as usize) < MAX_ALIGNMENTS, "{label}");
                assert!(
                    s.alignment == 0 || s.items >= MIN_SECONDARY_ITEMS,
                    "{label}"
                );
            }
            assert!(
                c.found_a() >= c.coverage_a && c.found_b() >= c.coverage_b,
                "{label}"
            );
            assert!((0.0..=1.0).contains(&c.found_a()) && (0.0..=1.0).contains(&c.found_b()));
        }
    }
}

#[test]
fn the_duplicate_rules_three_numbers_are_the_ones_e3_graded() {
    // ROADMAP 1.4's thresholds came from `fingerprint::compare` (E3). The
    // main alignment here must give the same numbers, so the rule applies
    // unchanged.
    let files = &corpus().files;
    for (i, a) in files.iter().enumerate() {
        for b in &files[i + 1..] {
            let ours = compared(&a.fingerprint, &b.fingerprint);
            let e3 = crate::fingerprint::compare(&a.fingerprint, &b.fingerprint).unwrap();
            let label = format!("{} vs {}: {ours:?} / {e3:?}", a.name, b.name);
            assert!((ours.coverage_a - e3.coverage_a).abs() < 1e-3, "{label}");
            assert!((ours.coverage_b - e3.coverage_b).abs() < 1e-3, "{label}");
            assert!((ours.score - e3.score).abs() < 1e-3, "{label}");
            assert_eq!(is_duplicate(&ours), e3.is_duplicate(), "{label}");
        }
    }
}

#[test]
fn fingerprints_made_by_different_versions_are_refused_not_compared() {
    let a = corpus().fingerprint("glasswing.wav").clone();
    let mut other_version = a.clone();
    other_version.set_version_for_tests(a.version() + 1);
    assert_eq!(compare(&a, &other_version), Err(CompareError::Incomparable));
    let mut other_algorithm = a.clone();
    other_algorithm.set_algorithm_for_tests(9);
    assert_eq!(
        compare(&a, &other_algorithm),
        Err(CompareError::Incomparable)
    );
}
