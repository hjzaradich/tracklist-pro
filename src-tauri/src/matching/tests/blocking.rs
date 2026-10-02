//! The block index (1bA-1): it proposes every pair that really shares
//! audio, proposes nothing between unrelated tracks, and adding or removing
//! one fingerprint is the same as building it again without the rest of
//! the work.

use std::collections::BTreeSet;

use crate::matching::index::{
    is_key, keys, BlockIndex, EntryId, MAX_ENTRIES_PER_KEY, MIN_SHARED_KEYS, SAMPLE,
};

use super::corpus::corpus;
use super::synthetic::{reencoded, track};

/// The corpus in an index, one entry per file, in file order.
fn corpus_index() -> BlockIndex {
    let mut index = BlockIndex::new();
    for (i, file) in corpus().files.iter().enumerate() {
        index.insert(i as EntryId, file.fingerprint.items());
    }
    index
}

fn is_candidate(pairs: &[(EntryId, EntryId)], a: &str, b: &str) -> bool {
    let (a, b) = (
        corpus().index_of(a) as EntryId,
        corpus().index_of(b) as EntryId,
    );
    pairs.contains(&(a.min(b), a.max(b)))
}

#[test]
fn every_true_pair_in_the_ground_truth_is_a_candidate() {
    // Duplicates in every format, a rip with lead-in silence, cuts sliced
    // out of a longer mix, Clean/Dirty, and a VIP sharing half its audio.
    let pairs = corpus_index().pairs();
    for (a, b) in corpus().true_pairs() {
        assert!(
            is_candidate(&pairs, a, b),
            "{a} and {b} were never proposed"
        );
    }
    // For the record: the related pairs that share little or nothing.
    for &(a, b) in &corpus().other_reworks {
        println!("{a} / {b}: candidate = {}", is_candidate(&pairs, a, b));
    }
}

#[test]
fn unrelated_tracks_are_never_candidates() {
    let files = &corpus().files;
    for (a, b) in corpus_index().pairs() {
        let (a, b) = (files[a as usize].name, files[b as usize].name);
        assert!(corpus().related(a, b), "{a} and {b} share nothing");
    }
}

#[test]
fn a_copy_played_at_another_speed_shares_no_keys_so_it_is_a_known_miss() {
    // Pinned: the index can't propose a resampled copy (the comparison
    // wouldn't call it a duplicate either).
    let pairs = corpus_index().pairs();
    assert!(!is_candidate(
        &pairs,
        "harbor (original).flac",
        "harbor (live).wav"
    ));
}

#[test]
fn a_key_is_chosen_by_the_items_value_alone_and_about_one_item_in_four_is_one() {
    let items = track(7, 4000);
    let found = keys(&items);
    assert!(found.iter().all(|&k| is_key(k) && items.contains(&k)));
    assert!(found.windows(2).all(|w| w[0] < w[1]), "distinct, in order");
    let expected = items.len() / SAMPLE as usize;
    assert!(
        found.len() > expected * 8 / 10 && found.len() < expected * 12 / 10,
        "{} keys of {} items",
        found.len(),
        items.len()
    );
    // The same item is a key wherever it sits and whatever is around it.
    let mut shuffled = items.clone();
    shuffled.reverse();
    shuffled.extend(track(8, 100));
    assert!(found.iter().all(|k| keys(&shuffled).contains(k)));
}

/// Items that are keys, all different.
fn some_keys(count: usize) -> Vec<u32> {
    (1u32..).filter(|&i| is_key(i)).take(count).collect()
}

/// Items that aren't keys.
fn filler(seed: u64, len: usize) -> Vec<u32> {
    track(seed, len * 2)
        .into_iter()
        .filter(|&i| !is_key(i))
        .take(len)
        .collect()
}

#[test]
fn sharing_a_single_item_by_chance_is_not_enough_to_be_a_candidate() {
    assert_eq!(MIN_SHARED_KEYS, 2);
    let shared = some_keys(2);
    let with = |seed: u64, shared: &[u32]| [filler(seed, 500), shared.to_vec()].concat();
    let mut index = BlockIndex::new();
    index.insert(1, &with(1, &shared[..1]));
    index.insert(2, &with(2, &shared[..1]));
    assert_eq!(index.pairs(), []);
    index.insert(3, &with(3, &shared));
    index.insert(4, &with(4, &shared));
    assert_eq!(index.pairs(), [(3, 4)]);
}

#[test]
fn a_sound_nearly_every_track_has_does_not_make_every_pair_a_candidate() {
    // The same stretch (silence, say) at the start of every track: its
    // keys are held by too many entries to mean anything.
    let silence = some_keys(12);
    let count = MAX_ENTRIES_PER_KEY as u64 + 50;
    let mut index = BlockIndex::new();
    for seed in 0..count {
        index.insert(
            seed as EntryId,
            &[silence.clone(), track(seed, 600)].concat(),
        );
    }
    // One real duplicate among them.
    let copy = [silence.clone(), reencoded(&track(3, 600), 99, 7)].concat();
    index.insert(count as EntryId, &copy);
    assert_eq!(index.pairs(), [(3, count as EntryId)]);
}

#[test]
fn the_same_stretch_in_a_few_tracks_does_make_them_candidates() {
    // Under the limit, shared audio is shared audio: three tracks with the
    // same intro are worth comparing.
    let intro = some_keys(12);
    let mut index = BlockIndex::new();
    for seed in 0..3 {
        index.insert(seed, &[intro.clone(), track(u64::from(seed), 600)].concat());
    }
    index.insert(3, &track(3, 600));
    assert_eq!(index.pairs(), [(0, 1), (0, 2), (1, 2)]);
}

/// 120 made-up tracks; every tenth also has a re-encoded copy.
fn small_library() -> Vec<(EntryId, Vec<u32>)> {
    let mut all = Vec::new();
    for seed in 0..120u64 {
        let items = track(seed, 800);
        if seed % 10 == 0 {
            all.push((1000 + seed as EntryId, reencoded(&items, seed, 7)));
        }
        all.push((seed as EntryId, items));
    }
    all
}

fn built(entries: &[(EntryId, Vec<u32>)]) -> BlockIndex {
    let mut index = BlockIndex::new();
    for (entry, items) in entries {
        index.insert(*entry, items);
    }
    index
}

#[test]
fn removing_one_fingerprint_leaves_exactly_the_index_built_without_it() {
    let library = small_library();
    let mut index = built(&library);
    assert_eq!(index.pairs().len(), 12);
    // Take out one half of a duplicate pair and an unrelated track.
    for gone in [1000, 55] {
        index.remove(gone);
    }
    let without: Vec<_> = library
        .iter()
        .filter(|(entry, _)| ![1000, 55].contains(entry))
        .cloned()
        .collect();
    let rebuilt = built(&without);
    assert_eq!(index.pairs(), rebuilt.pairs());
    assert_eq!(index.len(), rebuilt.len());
    assert_eq!(index.postings(), rebuilt.postings());
    let items = |entry: EntryId| &library.iter().find(|(e, _)| *e == entry).unwrap().1;
    assert!(!index.contains(1000) && index.candidates_of(1000, items(1000)).is_empty());
    assert!(
        index.candidates_of(0, items(0)).is_empty(),
        "its copy is gone"
    );
}

#[test]
fn adding_one_fingerprint_gives_exactly_the_index_built_with_it_in_any_order() {
    let library = small_library();
    let whole = built(&library);
    // Everything but one copy, then that copy last.
    let (last, rest): (Vec<_>, Vec<_>) = library.iter().cloned().partition(|(e, _)| *e == 1040);
    let mut index = built(&rest);
    assert_eq!(index.pairs().len(), 11);
    index.insert(last[0].0, &last[0].1);
    assert_eq!(index.pairs(), whole.pairs());
    assert_eq!(index.candidates_of(1040, &last[0].1), [40]);
    // Backwards gives the same pairs too.
    let mut reversed = library.clone();
    reversed.reverse();
    assert_eq!(built(&reversed).pairs(), whole.pairs());
}

#[test]
fn a_fingerprint_that_changes_is_replaced_not_added_to() {
    let mut index = BlockIndex::new();
    let (a, b) = (track(1, 800), track(2, 800));
    index.insert(1, &a);
    index.insert(2, &reencoded(&a, 5, 7));
    assert_eq!(index.pairs(), [(1, 2)]);
    // Entry 2 now holds other audio.
    index.insert(2, &b);
    assert_eq!(index.pairs(), []);
    assert_eq!(index.len(), 2);
    let expected: BTreeSet<u32> = keys(&a).into_iter().chain(keys(&b)).collect();
    assert_eq!(index.postings(), expected.len());
}
