//! The block index: which fingerprints are worth comparing (1bA-1; ROADMAP
//! 1.4). The scheme and what it can miss are in the [module docs](super).
//!
//! It holds no audio and no whole fingerprints: only each entry's keys.
//! Adding or removing an entry touches that entry's keys alone.

use std::collections::{BTreeSet, HashMap};

/// Keys are spread over this many buckets (2^18) by their mixed value.
const BUCKET_BITS: u32 = 18;

/// One in this many distinct items of a fingerprint becomes a key. Which
/// ones is decided by the item's value alone ([`is_key`]), so two
/// fingerprints holding the same item agree on whether it's a key.
pub const SAMPLE: u32 = 4;

/// Two fingerprints are candidates when they share at least this many
/// keys.
pub const MIN_SHARED_KEYS: u32 = 2;

/// A key held by more than this many entries says nothing (silence, a
/// sound every track has) and is skipped, so it can't pair everything with
/// everything.
pub const MAX_ENTRIES_PER_KEY: usize = 200;

/// Whether item `item` is one of the sampled keys. A fixed mix of the
/// value, so it's the same in every run and for every fingerprint.
pub fn is_key(item: u32) -> bool {
    mix(item).is_multiple_of(SAMPLE)
}

/// The 32-bit finalizer of MurmurHash3: every input bit reaches every
/// output bit, so items that differ in one bit land far apart.
fn mix(item: u32) -> u32 {
    let mut x = item;
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    x
}

/// The bucket a key is kept in: the top bits of its mixed value (the
/// bottom ones decided whether it's a key at all).
fn bucket_of(key: u32) -> usize {
    (mix(key) >> (32 - BUCKET_BITS)) as usize
}

/// The keys of a fingerprint: its distinct sampled items, in order.
pub fn keys(items: &[u32]) -> Vec<u32> {
    let keys: BTreeSet<u32> = items.iter().copied().filter(|&i| is_key(i)).collect();
    keys.into_iter().collect()
}

/// An entry's name in the index: whatever the caller counts as one
/// fingerprint (the matching job uses one per distinct fingerprint).
pub type EntryId = u32;

/// Fingerprints by their keys. About 12 bytes per (key, entry) posting,
/// plus 6 MB once anything is in it.
#[derive(Debug, Default)]
pub struct BlockIndex {
    /// The (key, entry) postings, by [`bucket_of`] the key, in no order.
    /// Empty until the first entry goes in.
    buckets: Vec<Vec<(u32, EntryId)>>,
    /// Each entry's keys, for taking it out again.
    keys_by_entry: HashMap<EntryId, Vec<u32>>,
}

impl BlockIndex {
    pub fn new() -> BlockIndex {
        BlockIndex::default()
    }

    /// How many entries it holds.
    pub fn len(&self) -> usize {
        self.keys_by_entry.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys_by_entry.is_empty()
    }

    /// How many (key, entry) postings it holds: its size.
    pub fn postings(&self) -> usize {
        self.keys_by_entry.values().map(Vec::len).sum()
    }

    pub fn contains(&self, entry: EntryId) -> bool {
        self.keys_by_entry.contains_key(&entry)
    }

    /// Adds `entry` with the fingerprint `items`, replacing what it held.
    /// Touches only this entry's keys.
    pub fn insert(&mut self, entry: EntryId, items: &[u32]) {
        self.remove(entry);
        if self.buckets.is_empty() {
            self.buckets = vec![Vec::new(); 1 << BUCKET_BITS];
        }
        let keys = keys(items);
        for &key in &keys {
            self.buckets[bucket_of(key)].push((key, entry));
        }
        self.keys_by_entry.insert(entry, keys);
    }

    /// Takes `entry` out. Nothing happens if it isn't in. Touches only
    /// this entry's keys.
    pub fn remove(&mut self, entry: EntryId) {
        let Some(keys) = self.keys_by_entry.remove(&entry) else {
            return;
        };
        for key in keys {
            let bucket = &mut self.buckets[bucket_of(key)];
            if let Some(at) = bucket.iter().position(|&p| p == (key, entry)) {
                bucket.swap_remove(at);
            }
        }
    }

    /// The entries worth comparing with `entry`: those sharing at least
    /// [`MIN_SHARED_KEYS`] keys with it, not counting keys held by more
    /// than [`MAX_ENTRIES_PER_KEY`] entries. Lowest id first.
    pub fn candidates_of(&self, entry: EntryId) -> Vec<EntryId> {
        let Some(keys) = self.keys_by_entry.get(&entry) else {
            return Vec::new();
        };
        let mut shared: HashMap<EntryId, u32> = HashMap::new();
        let mut holders: Vec<EntryId> = Vec::new();
        for &key in keys {
            holders.clear();
            for posting in &self.buckets[bucket_of(key)] {
                if posting.0 == key {
                    holders.push(posting.1);
                    // One too many is enough to know it's skipped.
                    if holders.len() > MAX_ENTRIES_PER_KEY {
                        break;
                    }
                }
            }
            if holders.len() > MAX_ENTRIES_PER_KEY {
                continue;
            }
            for &other in &holders {
                if other != entry {
                    *shared.entry(other).or_default() += 1;
                }
            }
        }
        let mut found: Vec<EntryId> = shared
            .into_iter()
            .filter(|&(_, count)| count >= MIN_SHARED_KEYS)
            .map(|(other, _)| other)
            .collect();
        found.sort_unstable();
        found
    }

    /// Every candidate pair, the lower id first, in order.
    pub fn pairs(&self) -> Vec<(EntryId, EntryId)> {
        let mut entries: Vec<EntryId> = self.keys_by_entry.keys().copied().collect();
        entries.sort_unstable();
        let mut pairs = Vec::new();
        for entry in entries {
            pairs.extend(
                self.candidates_of(entry)
                    .into_iter()
                    .filter(|&other| other > entry)
                    .map(|other| (entry, other)),
            );
        }
        pairs
    }
}
