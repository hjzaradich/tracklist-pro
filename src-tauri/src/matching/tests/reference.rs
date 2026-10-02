//! The block index as it was before 1bA-13 made it smaller: every entry
//! keeps its own key list, and taking an entry out removes its postings
//! there and then. Kept as it was, as the answer the smaller index is
//! checked against: the levers that claim to lose nothing must give the
//! same candidates.

use std::collections::HashMap;

use crate::matching::index::{keys, EntryId, MAX_ENTRIES_PER_KEY, MIN_SHARED_KEYS};

const BUCKET_BITS: u32 = 18;

fn mix(item: u32) -> u32 {
    let mut x = item;
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    x
}

fn bucket_of(key: u32) -> usize {
    (mix(key) >> (32 - BUCKET_BITS)) as usize
}

#[derive(Debug, Default)]
pub struct PlainIndex {
    /// The (key, entry) postings, by [`bucket_of`] the key, in no order.
    /// Empty until the first entry goes in.
    buckets: Vec<Vec<(u32, EntryId)>>,
    /// Each entry's keys, for taking it out again.
    keys_by_entry: HashMap<EntryId, Vec<u32>>,
}

impl PlainIndex {
    pub fn new() -> PlainIndex {
        PlainIndex::default()
    }

    /// How many entries it holds.
    pub fn len(&self) -> usize {
        self.keys_by_entry.len()
    }

    /// How many (key, entry) postings it holds: its size.
    pub fn postings(&self) -> usize {
        self.keys_by_entry.values().map(Vec::len).sum()
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
