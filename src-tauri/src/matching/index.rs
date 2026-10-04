//! The block index: which fingerprints are worth comparing (1bA-1; ROADMAP
//! 1.4). The scheme and what it can miss are in the [module docs](super).
//!
//! It holds no audio, no whole fingerprints and no list of each entry's
//! keys: only the (key, entry) postings, 8 bytes each (1bA-13).
//!
//! - **A few big sorted runs, not many small lists.** The postings are
//!   kept in order in [`SEGMENTS`] runs. New ones wait in one unsorted
//!   list and are merged in, a run at a time, when there are enough of
//!   them or when [`BlockIndex::compact`] is called. So the index is never
//!   much bigger than its postings while it's built (the waiting list is
//!   at most an eighth of the rest), nothing is left over afterwards, and
//!   a key is found among a few hundred neighbours. Many small growing lists cost the process
//!   about twice their size on Windows: the room each one outgrew stays
//!   with the heap.
//! - **No key list per entry.** The caller hands an entry's fingerprint
//!   over again to ask for its candidates (the matching pass has it in
//!   hand anyway), and an entry that's taken out stops counting at once
//!   and leaves its postings behind, unseen, until the next merge.

use std::collections::{HashMap, HashSet};

/// The sorted postings are kept in this many runs (2^8), by the top bits
/// of the key's mixed value, so merging new ones in never needs a second
/// copy of more than one run.
const SEGMENT_BITS: u32 = 8;

/// How many runs there are.
const SEGMENTS: usize = 1 << SEGMENT_BITS;

/// Each run notes where each of this many (2^10) equal slices of its key
/// range starts, so a key is looked for among a few hundred postings that
/// sit together, not by halving the whole run. 1 MB for the whole index.
const PART_BITS: u32 = 10;

/// How many slices a run's key range is cut into.
const PARTS: usize = 1 << PART_BITS;

/// New postings are merged in once there are this many (half a megabyte)…
const MIN_WAITING: usize = 1 << 16;

/// …or this share of the merged ones, whichever is more.
const WAITING_SHARE: usize = 8;

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
/// output bit, so items that differ in one bit land far apart. No two
/// items mix to the same value.
fn mix(item: u32) -> u32 {
    let mut x = item;
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    x
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

/// One key held by one entry: the key's mixed value in the top half, the
/// entry in the bottom half. In order, they're grouped by key, and the top
/// bits of the mixed value (the bottom ones decided whether it's a key at
/// all) spread them evenly over the runs.
type Posting = u64;

fn posting(mixed: u32, entry: EntryId) -> Posting {
    (u64::from(mixed) << 32) | u64::from(entry)
}

fn mixed_of(posting: Posting) -> u32 {
    (posting >> 32) as u32
}

fn entry_of(posting: Posting) -> EntryId {
    posting as u32
}

/// The run a key with this mixed value is kept in.
fn segment_of(mixed: u32) -> usize {
    (mixed >> (32 - SEGMENT_BITS)) as usize
}

/// The slice of its run's key range a key with this mixed value is in.
fn part_of(mixed: u32) -> usize {
    (mixed >> (32 - SEGMENT_BITS - PART_BITS)) as usize & (PARTS - 1)
}

/// Where the index keeps `key`: which run, and which slice of that run's
/// key range. For tests that place keys at the edges.
#[cfg(test)]
pub(super) fn place_of(key: u32) -> (usize, usize) {
    (segment_of(mix(key)), part_of(mix(key)))
}

/// How many runs there are, and how many slices each run's key range is
/// cut into.
#[cfg(test)]
pub(super) const PLACES: (usize, usize) = (SEGMENTS, PARTS);

/// The postings of the key with mixed value `mixed` among `sorted` ones.
fn of_key(sorted: &[Posting], mixed: u32) -> &[Posting] {
    let from = sorted.partition_point(|&p| mixed_of(p) < mixed);
    let len = sorted[from..].partition_point(|&p| mixed_of(p) == mixed);
    &sorted[from..from + len]
}

/// One sorted run of postings.
#[derive(Debug, Default, Clone)]
struct Segment {
    /// In order.
    postings: Vec<Posting>,
    /// Where each [`part_of`] the key range starts in `postings`, and
    /// where the last one ends. Empty while there are no postings.
    starts: Vec<u32>,
}

impl Segment {
    fn new(postings: Vec<Posting>) -> Segment {
        if postings.is_empty() {
            return Segment::default();
        }
        // Count each part, then add up: a part starts where the ones
        // before it end.
        let mut starts = vec![0u32; PARTS + 1];
        for &posting in &postings {
            starts[part_of(mixed_of(posting)) + 1] += 1;
        }
        for part in 0..PARTS {
            starts[part + 1] += starts[part];
        }
        Segment { postings, starts }
    }

    /// The postings of the key with mixed value `mixed`.
    fn of_key(&self, mixed: u32) -> &[Posting] {
        let part = part_of(mixed);
        match (self.starts.get(part), self.starts.get(part + 1)) {
            (Some(&from), Some(&to)) => of_key(&self.postings[from as usize..to as usize], mixed),
            _ => &[],
        }
    }
}

/// Fingerprints by their keys: 8 bytes per (key, entry) posting once
/// [compacted](BlockIndex::compact), plus 1 MB, and at most an eighth more
/// (or half a megabyte) while entries are being added.
#[derive(Debug, Default)]
pub struct BlockIndex {
    /// The merged postings, in order, by [`segment_of`] the key. Empty
    /// until the first merge.
    segments: Vec<Segment>,
    /// How many postings the segments hold.
    merged: usize,
    /// Postings added since the last merge, in the order they came.
    waiting: Vec<Posting>,
    /// The entries in the index, and how many keys each holds.
    entries: HashMap<EntryId, u32>,
    /// Entries taken out whose postings are still held. Nothing counts
    /// those postings; the next merge drops them.
    removed: HashSet<EntryId>,
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

    /// How many postings may wait before they're merged in.
    fn waiting_limit(&self) -> usize {
        MIN_WAITING.max(self.merged / WAITING_SHARE)
    }

    /// Adds `entry` with the fingerprint `items`, replacing what it held.
    ///
    /// A new name costs only its own keys, plus a merge now and then. A
    /// name that was used before (replacing, or adding again after
    /// [`BlockIndex::remove`]) merges at once, so the old fingerprint's
    /// postings can't pass for the new one's: callers with many changes
    /// give each fingerprint a name of its own, as the matching job does.
    pub fn insert(&mut self, entry: EntryId, items: &[u32]) {
        if self.entries.contains_key(&entry) || self.removed.contains(&entry) {
            self.remove(entry);
            self.compact();
        }
        let keys = keys(items);
        if !self.waiting.is_empty() && self.waiting.len() + keys.len() > self.waiting_limit() {
            self.compact();
        }
        // The waiting list grows by doubling, but never past its limit:
        // the last doubling would be the biggest thing in the index.
        let needed = self.waiting.len() + keys.len();
        if self.waiting.capacity() < needed {
            let room = (self.waiting.capacity() * 2)
                .clamp(MIN_WAITING, self.waiting_limit())
                .max(needed);
            self.waiting.reserve_exact(room - self.waiting.len());
        }
        self.waiting
            .extend(keys.iter().map(|&key| posting(mix(key), entry)));
        self.entries.insert(entry, keys.len() as u32);
    }

    /// Takes `entry` out. Nothing happens if it isn't in. Its postings
    /// stay where they are, unseen, until the next merge.
    pub fn remove(&mut self, entry: EntryId) {
        if self.entries.remove(&entry).is_some() {
            self.removed.insert(entry);
        }
    }

    /// Merges the waiting postings in, drops the postings of removed
    /// entries and gives back the room that leaves. Changes nothing a
    /// caller can see but the memory held and how fast
    /// [`BlockIndex::candidates_of`] is. Worth calling once after a run of
    /// inserts and removes; with nothing to do it does nothing.
    pub fn compact(&mut self) {
        if self.waiting.is_empty() && self.removed.is_empty() {
            return;
        }
        if self.segments.is_empty() {
            self.segments = vec![Segment::default(); SEGMENTS];
        }
        let removed = std::mem::take(&mut self.removed);
        let live = |posting: &Posting| removed.is_empty() || !removed.contains(&entry_of(*posting));
        let mut waiting = std::mem::take(&mut self.waiting);
        waiting.retain(live);
        waiting.sort_unstable();

        // One run at a time: a new run of exactly the size needed, filled
        // from the old one and the new postings that belong in it.
        let mut rest = waiting.as_slice();
        for (i, segment) in self.segments.iter_mut().enumerate() {
            let (new, after) =
                rest.split_at(rest.partition_point(|&p| segment_of(mixed_of(p)) <= i));
            rest = after;
            let kept = if removed.is_empty() {
                segment.postings.len()
            } else {
                segment.postings.iter().filter(|p| live(p)).count()
            };
            if new.is_empty() && kept == segment.postings.len() {
                continue;
            }
            let mut merged = Vec::with_capacity(kept + new.len());
            let mut old = segment.postings.iter().copied().filter(live).peekable();
            for &posting in new {
                while let Some(before) = old.next_if(|&o| o < posting) {
                    merged.push(before);
                }
                merged.push(posting);
            }
            merged.extend(old);
            *segment = Segment::new(merged);
        }
        self.merged = self.segments.iter().map(|s| s.postings.len()).sum();
    }

    /// The entries worth comparing with `entry`, whose fingerprint is
    /// `items` (the one it was added with): those sharing at least
    /// [`MIN_SHARED_KEYS`] keys with it, not counting keys held by more
    /// than [`MAX_ENTRIES_PER_KEY`] entries. Lowest id first. Nothing if
    /// `entry` isn't in the index.
    ///
    /// The answer is the same whether or not the index was
    /// [compacted](BlockIndex::compact) first, but every call reads the
    /// whole waiting list, so compact before asking for many.
    pub fn candidates_of(&self, entry: EntryId, items: &[u32]) -> Vec<EntryId> {
        if !self.contains(entry) {
            return Vec::new();
        }
        let mut mixed: Vec<u32> = keys(items).into_iter().map(mix).collect();
        mixed.sort_unstable();
        // The waiting postings that hold one of these keys, in order.
        let mut waiting: Vec<Posting> = self
            .waiting
            .iter()
            .copied()
            .filter(|&p| mixed.binary_search(&mixed_of(p)).is_ok())
            .collect();
        waiting.sort_unstable();
        let live = |posting: &&Posting| {
            self.removed.is_empty() || !self.removed.contains(&entry_of(**posting))
        };

        let mut shared: HashMap<EntryId, u32> = HashMap::new();
        let mut holders: Vec<EntryId> = Vec::new();
        for &key in &mixed {
            let merged = match self.segments.get(segment_of(key)) {
                Some(segment) => segment.of_key(key),
                None => &[],
            };
            holders.clear();
            // One too many is enough to know the key is skipped.
            holders.extend(
                merged
                    .iter()
                    .chain(of_key(&waiting, key))
                    .filter(live)
                    .take(MAX_ENTRIES_PER_KEY + 1)
                    .map(|&posting| entry_of(posting)),
            );
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
    /// a sorted copy of all the postings, key by key: for checking and
    /// counting, not for the matching pass (which asks for one entry's
    /// candidates at a time).
    pub fn pairs(&self) -> Vec<(EntryId, EntryId)> {
        let mut all: Vec<Posting> = self
            .segments
            .iter()
            .flat_map(|segment| &segment.postings)
            .chain(&self.waiting)
            .copied()
            .filter(|p| self.removed.is_empty() || !self.removed.contains(&entry_of(*p)))
            .collect();
        all.sort_unstable();
        let mut shared: HashMap<(EntryId, EntryId), u32> = HashMap::new();
        // Each run of one key is the entries holding it, lowest first.
        for holders in all.chunk_by(|&a, &b| mixed_of(a) == mixed_of(b)) {
            if holders.len() > MAX_ENTRIES_PER_KEY {
                continue;
            }
            for (i, &a) in holders.iter().enumerate() {
                for &b in &holders[i + 1..] {
                    *shared.entry((entry_of(a), entry_of(b))).or_default() += 1;
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
