//! The index on 10,000 made-up fingerprints: candidates stay close to the
//! pairs that are really there, far from n². Counts only, never time.

use std::collections::HashSet;

use crate::matching::index::{BlockIndex, EntryId};

use super::synthetic::{reencoded, track};

/// Items per made-up track: about two and a half minutes.
const LEN: usize = 1200;

/// What was planted in a made-up library, and what the index proposed.
struct Outcome {
    /// Pairs that share audio.
    planted: usize,
    /// Planted pairs of a kind the index promises to find (ordinary
    /// re-encodes and cuts), and how many of them it proposed.
    strong: usize,
    strong_found: usize,
    /// Planted pairs at the duplicate rule's limit, and how many of them
    /// it proposed.
    weak: usize,
    weak_found: usize,
    candidates: usize,
    postings: usize,
}

/// A library of `n` fingerprints. Every track starts with the same
/// "silence". Of every 100 tracks: three have a re-encoded copy (about 1
/// item in 7 identical, as a WAV against its MP3 measured), one has a cut
/// (the middle half, re-encoded), and every second hundred has one copy at
/// the duplicate rule's limit (about 1 item in 40 identical).
fn library(n: usize) -> Outcome {
    let silence: Vec<u32> = track(u64::MAX, 20);
    let with_silence = |items: &[u32]| [silence.as_slice(), items].concat();
    let mut index = BlockIndex::new();
    let mut strong: Vec<(EntryId, EntryId)> = Vec::new();
    let mut weak: Vec<(EntryId, EntryId)> = Vec::new();
    let mut next = 0 as EntryId;
    let mut add = |index: &mut BlockIndex, items: &[u32]| {
        index.insert(next, items);
        next += 1;
        next - 1
    };
    let mut made = 0;
    let mut seed = 0u64;
    while made < n {
        seed += 1;
        let items = track(seed, LEN);
        let original = add(&mut index, &with_silence(&items));
        made += 1;
        let copy = match seed % 100 {
            1..=3 => Some((reencoded(&items, seed, 7), true)),
            4 => Some((reencoded(&items[LEN / 4..LEN * 3 / 4], seed, 7), true)),
            5 if (seed / 100).is_multiple_of(2) => Some((reencoded(&items, seed, 40), false)),
            _ => None,
        };
        if let Some((copy, is_strong)) = copy {
            let copy = add(&mut index, &with_silence(&copy));
            made += 1;
            if is_strong {
                strong.push((original, copy));
            } else {
                weak.push((original, copy));
            }
        }
    }
    let candidates: HashSet<(EntryId, EntryId)> = index.pairs().into_iter().collect();
    Outcome {
        planted: strong.len() + weak.len(),
        strong: strong.len(),
        strong_found: strong.iter().filter(|p| candidates.contains(p)).count(),
        weak: weak.len(),
        weak_found: weak.iter().filter(|p| candidates.contains(p)).count(),
        candidates: candidates.len(),
        postings: index.postings(),
    }
}

#[test]
fn on_ten_thousand_fingerprints_the_candidates_are_the_real_pairs_not_n_squared() {
    let quarter = library(2_500);
    let whole = library(10_000);
    for (n, o) in [(2_500, &quarter), (10_000, &whole)] {
        println!(
            "{n} fingerprints: {} planted pairs ({} of {} strong found, {} of {} at the limit), \
             {} candidates of {} possible pairs, {} postings",
            o.planted,
            o.strong_found,
            o.strong,
            o.weak_found,
            o.weak,
            o.candidates,
            n * (n - 1) / 2,
            o.postings
        );
    }
    // Every ordinary re-encode and cut is proposed.
    assert_eq!(whole.strong_found, whole.strong);
    assert_eq!(quarter.strong_found, quarter.strong);
    // At the rule's limit a pair shares few identical items; nearly all
    // are still proposed (the stated false-negative risk).
    assert!(
        whole.weak_found * 10 >= whole.weak * 9,
        "{} of {}",
        whole.weak_found,
        whole.weak
    );
    // Hardly anything else is: the silence every track shares pairs
    // nothing, and chance pairs almost nothing.
    assert!(
        whole.candidates <= whole.planted + whole.planted / 20 + 5,
        "{} candidates for {} planted pairs",
        whole.candidates,
        whole.planted
    );
    // Four times the fingerprints: about four times the candidates (n²
    // would be sixteen), and nowhere near the 50 million possible pairs.
    assert!(
        whole.candidates <= quarter.candidates * 5,
        "{} then {}",
        quarter.candidates,
        whole.candidates
    );
    assert!(whole.candidates < 10_000 / 10);
    // The index's size grows with the fingerprints, about LEN / 4 each.
    assert!(whole.postings <= 10_000 * (LEN + 20) / 3);
}
