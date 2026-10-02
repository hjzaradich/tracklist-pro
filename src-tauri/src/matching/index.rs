//! The block index: which fingerprints are worth comparing (1bA-1; ROADMAP
//! 1.4). The scheme and what it can miss are in the [module docs](super).
//!
//! It holds no audio, no whole fingerprints and no list of each entry's
//! keys: only the (key, entry) postings, 8 bytes each (1bA-13). So the
//! caller hands an entry's fingerprint over again to ask for its
//! candidates, and an entry that's taken out leaves its postings behind,
//! unseen, until [`BlockIndex::compact`] sweeps them. A compact also
//! gives back the room the buckets grew into and puts each in order, so a
//! key is found without reading its whole bucket.

use std::collections::{HashMap, HashSet};

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
    let mut keys: Vec<u32> = items.iter().copied().filter(|&i| is_key(i)).collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// An entry's name in the index: whatever the caller counts as one
/// fingerprint (the matching job uses one per distinct fingerprint).
pub type EntryId = u32;

/// One key held by one entry.
type Posting = (u32, EntryId);

/// Fingerprints by their keys. 8 bytes per (key, entry) posting once
/// [compacted](BlockIndex::compact), plus 6 MB once anything is in it.
#[derive(Debug, Default)]
pub struct BlockIndex {
    /// The postings, by [`bucket_of`] the key, in no order. Empty until
    /// the first entry goes in.
    buckets: Vec<Vec<Posting>>,
    /// The entries in the index, and how many keys each holds.
    entries: HashMap<EntryId, u32>,
    /// Entries taken out whose postings are still in the buckets. Nothing
    /// reads those postings; [`BlockIndex::compact`] drops them.
    removed: HashSet<EntryId>,
    /// Whether every bucket is in order (by key, then entry), so a key's
    /// postings can be found by halving rather than by reading the whole
    /// bucket. True from a [`BlockIndex::compact`] until the next insert.
    sorted: bool,
}

/// Adds `posting` to `bucket`, growing it by an eighth when it's full
/// rather than doubling it: while a big index is built, the spare room
/// stays a small part of it.
fn push(bucket: &mut Vec<Posting>, posting: Posting) {
    if bucket.len() == bucket.capacity() {
        bucket.reserve_exact(bucket.len() / 8 + 4);
    }
    bucket.push(posting);
}

impl BlockIndex {
    pub fn new() -> BlockIndex {
        BlockIndex::default()
    }

    /// How many entries it holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many (key, entry) postings its entries hold: its size.
    pub fn postings(&self) -> usize {
        self.entries.values().map(|&keys| keys as usize).sum()
    }

    pub fn contains(&self, entry: EntryId) -> bool {
        self.entries.contains_key(&entry)
    }

    /// Adds `entry` with the fingerprint `items`, replacing what it held.
    ///
    /// A new name costs only its own keys. A name that was used before
    /// (replacing, or adding again after [`BlockIndex::remove`]) sweeps
    /// the whole index first, so the old fingerprint's postings can't pass
    /// for the new one's: callers with many changes give each fingerprint
    /// a name of its own, as the matching job does.
    pub fn insert(&mut self, entry: EntryId, items: &[u32]) {
        if self.entries.contains_key(&entry) || self.removed.contains(&entry) {
            self.remove(entry);
            self.compact();
        }
        if self.buckets.is_empty() {
            self.buckets = vec![Vec::new(); 1 << BUCKET_BITS];
        }
        let keys = keys(items);
        for &key in &keys {
            push(&mut self.buckets[bucket_of(key)], (key, entry));
        }
        self.sorted &= keys.is_empty();
        self.entries.insert(entry, keys.len() as u32);
    }

    /// Takes `entry` out. Nothing happens if it isn't in. Its postings
    /// stay where they are, unseen, until [`BlockIndex::compact`].
    pub fn remove(&mut self, entry: EntryId) {
        if self.entries.remove(&entry).is_some() {
            self.removed.insert(entry);
        }
    }

    /// Drops the postings of removed entries, gives back the buckets'
    /// spare room and puts each bucket in order. Changes nothing a caller
    /// can see but the memory held and how fast
    /// [`BlockIndex::candidates_of`] is. Worth calling once after a run of
    /// inserts and removes; it looks at every bucket.
    pub fn compact(&mut self) {
        let removed = std::mem::take(&mut self.removed);
        for bucket in &mut self.buckets {
            if !removed.is_empty() {
                bucket.retain(|posting| !removed.contains(&posting.1));
            }
            bucket.shrink_to_fit();
            if !self.sorted {
                bucket.sort_unstable();
            }
        }
        self.sorted = true;
    }

    /// The postings in `key`'s bucket that may be `key`'s: exactly its
    /// own when the buckets are in order, the whole bucket otherwise.
    fn bucket_of_key(&self, key: u32) -> &[Posting] {
        let bucket = &self.buckets[bucket_of(key)];
        if !self.sorted {
            return bucket;
        }
        let from = bucket.partition_point(|posting| posting.0 < key);
        let len = bucket[from..].partition_point(|posting| posting.0 == key);
        &bucket[from..from + len]
    }

    /// Whether a posting is of an entry still in the index.
    fn live(&self, posting: &Posting) -> bool {
        self.removed.is_empty() || !self.removed.contains(&posting.1)
    }

    /// The entries worth comparing with `entry`, whose fingerprint is
    /// `items` (the one it was added with): those sharing at least
    /// [`MIN_SHARED_KEYS`] keys with it, not counting keys held by more
    /// than [`MAX_ENTRIES_PER_KEY`] entries. Lowest id first. Nothing if
    /// `entry` isn't in the index.
    pub fn candidates_of(&self, entry: EntryId, items: &[u32]) -> Vec<EntryId> {
        if !self.contains(entry) {
            return Vec::new();
        }
        let mut shared: HashMap<EntryId, u32> = HashMap::new();
        let mut holders: Vec<EntryId> = Vec::new();
        for key in keys(items) {
            holders.clear();
            for posting in self.bucket_of_key(key) {
                if posting.0 == key && self.live(posting) {
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

    /// Every candidate pair, the lower id first, in order. Worked out from
    /// the postings alone, key by key: for checking and counting, not for
    /// the matching pass (which asks for one entry's candidates at a time).
    pub fn pairs(&self) -> Vec<(EntryId, EntryId)> {
        let mut shared: HashMap<(EntryId, EntryId), u32> = HashMap::new();
        let mut sorted: Vec<Posting> = Vec::new();
        for bucket in &self.buckets {
            sorted.clear();
            sorted.extend(bucket.iter().filter(|posting| self.live(posting)));
            sorted.sort_unstable();
            // Each run of one key is the entries holding it, lowest first.
            for holders in sorted.chunk_by(|a, b| a.0 == b.0) {
                if holders.len() > MAX_ENTRIES_PER_KEY {
                    continue;
                }
                for (i, a) in holders.iter().enumerate() {
                    for b in &holders[i + 1..] {
                        *shared.entry((a.1, b.1)).or_default() += 1;
                    }
                }
            }
        }
        let mut pairs: Vec<(EntryId, EntryId)> = shared
            .into_iter()
            .filter(|&(_, count)| count >= MIN_SHARED_KEYS)
            .map(|(pair, _)| pair)
            .collect();
        pairs.sort_unstable();
        pairs
    }
}
