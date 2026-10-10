//! The smaller index (1bA-13) finds exactly what the index before it found.
//! Two changes claim to lose nothing: an entry's key list is no longer
//! kept (so a removed entry's postings stay, unseen, until the next
//! merge), and the postings live in a few sorted runs that new ones are
//! merged into, with no room left over. Each is checked against the index
//! as it was ([`PlainIndex`]) through adds, removals and replacements.

use std::collections::BTreeMap;

use crate::matching::index::{is_key, BlockIndex, EntryId, MAX_ENTRIES_PER_KEY};

use super::reference::PlainIndex;
use super::synthetic::{reencoded, track};

/// Items per made-up track.
const LEN: usize = 600;

/// Both indexes, fed the same, and the fingerprints now in them.
#[derive(Default)]
struct Both {
    smaller: BlockIndex,
    plain: PlainIndex,
    items: BTreeMap<EntryId, Vec<u32>>,
    /// Whether the smaller index is compacted after every change from now
    /// on.
    compacting: bool,
}

impl Both {
    fn insert(&mut self, entry: EntryId, items: Vec<u32>) {
        self.smaller.insert(entry, &items);
        self.plain.insert(entry, &items);
        self.items.insert(entry, items);
        self.changed();
    }

    fn remove(&mut self, entry: EntryId) {
        self.smaller.remove(entry);
        self.plain.remove(entry);
        self.items.remove(&entry);
        self.changed();
    }

    fn changed(&mut self) {
        if self.compacting {
            self.smaller.compact();
        }
    }

    /// The two agree on every pair, on every entry's own candidates, and
    /// on what they hold. Returns how many candidate pairs there are.
    fn assert_same(&self, when: &str) -> usize {
        self.assert_same_asking(when, 1)
    }

    /// The same, asking only every `nth` entry for its own candidates
    /// (asking is slow while postings wait to be merged; the pairs are
    /// always all checked).
    fn assert_same_asking(&self, when: &str, nth: usize) -> usize {
        let pairs = self.smaller.pairs();
        assert_eq!(pairs, self.plain.pairs(), "{when}: the candidate pairs");
        for (&entry, items) in self.items.iter().step_by(nth) {
            assert_eq!(
                self.smaller.candidates_of(entry, items),
                self.plain.candidates_of(entry),
                "{when}: entry {entry}'s candidates"
            );
        }
        assert_eq!(self.smaller.len(), self.plain.len(), "{when}");
        assert_eq!(self.smaller.postings(), self.plain.postings(), "{when}");
        pairs.len()
    }
}

/// Items that are keys, all different.
fn some_keys(count: usize) -> Vec<u32> {
    (1u32..).filter(|&i| is_key(i)).take(count).collect()
}

/// How many entries hold the jingle: just over the limit, so its keys are
/// skipped until a few of the entries go.
const JINGLE_HOLDERS: u32 = MAX_ENTRIES_PER_KEY as u32 + 5;

/// The first entry holding the jingle.
const JINGLE_FIRST: EntryId = 10_000;

/// A made-up library: 1,500 tracks that all start with the same silence;
/// of every 50, two have a re-encoded copy, one a cut and one a copy at
/// the duplicate rule's limit. On top, [`JINGLE_HOLDERS`] tracks sharing a
/// jingle. Copies are named 5,000 above their original.
fn library(both: &mut Both) {
    let silence = track(u64::MAX, 20);
    let with_silence = |items: &[u32]| [silence.as_slice(), items].concat();
    for seed in 0..1_500u64 {
        let items = track(seed, LEN);
        let copy = match seed % 50 {
            1 | 2 => Some(reencoded(&items, seed, 7)),
            3 => Some(reencoded(&items[LEN / 4..LEN * 3 / 4], seed, 7)),
            4 => Some(reencoded(&items, seed, 40)),
            _ => None,
        };
        if let Some(copy) = copy {
            both.insert(5_000 + seed as EntryId, with_silence(&copy));
        }
        both.insert(seed as EntryId, with_silence(&items));
    }
    let jingle = some_keys(6);
    for i in 0..JINGLE_HOLDERS {
        let own = track(20_000 + u64::from(i), LEN);
        both.insert(JINGLE_FIRST + i, [jingle.as_slice(), &own].concat());
    }
}

/// Adds, removals and replacements over the library, checking after each
/// step that the two indexes agree.
fn through_changes(compacting: bool) {
    let mut both = Both::default();
    library(&mut both);
    // From here on, after every single change.
    both.compacting = compacting;
    both.changed();
    let built = both.assert_same("built");
    assert!(built >= 100, "the copies are proposed: {built} pairs");

    // One half of a duplicate pair, an original with a cut, and a track
    // with no copy go.
    for gone in [5_001, 3, 77] {
        both.remove(gone);
    }
    let fewer = both.assert_same_asking("after three removals", 8);
    assert_eq!(fewer, built - 2, "two pairs went with their files");

    // Enough of the tracks sharing the jingle go that its keys are held by
    // few enough entries to count: the rest become candidates of each
    // other. A removed entry must stop counting at once.
    for i in 0..10 {
        both.remove(JINGLE_FIRST + i);
    }
    let left = (JINGLE_HOLDERS - 10) as usize;
    let with_jingle = both.assert_same_asking("after the jingle dropped under the limit", 8);
    assert_eq!(with_jingle, fewer + left * (left - 1) / 2);

    // New files under new names: a copy of a track that had none, and a
    // new copy of the track whose first copy was removed.
    both.insert(30_000, reencoded(&track(500, LEN), 91, 7));
    both.insert(30_001, reencoded(&track(1, LEN), 92, 7));
    assert_eq!(
        both.assert_same_asking("after two new files", 8),
        with_jingle + 2
    );

    // A name that's in use gets other audio: a copy of track 600.
    both.insert(5_002, reencoded(&track(600, LEN), 93, 7));
    both.assert_same_asking("after a replacement", 8);
    assert!(
        both.items.contains_key(&2) && both.smaller.candidates_of(2, &both.items[&2]).is_empty()
    );

    // A removed name comes back with other audio than it left with.
    both.insert(77, reencoded(&track(700, LEN), 94, 7));
    both.insert(JINGLE_FIRST, track(40_000, LEN));
    both.assert_same_asking("after two names came back", 8);

    // Enough new tracks with the jingle that it's over the limit again.
    for i in 0..10 {
        let own = track(50_000 + u64::from(i), LEN);
        both.insert(31_000 + i, [some_keys(6).as_slice(), &own].concat());
    }
    let over = both.assert_same_asking("after the jingle went over the limit again", 8);
    assert!(over < with_jingle);

    // Sweeping at the end changes nothing either way.
    both.smaller.compact();
    assert_eq!(both.assert_same("after a last sweep"), over);
}

#[test]
fn without_each_entrys_key_list_the_candidates_are_the_same_through_adds_removals_and_replacements()
{
    // Never compacted by hand on the way: removed entries' postings are
    // still held and must be passed over, and new postings are partly
    // merged in (the library is big enough for that) and partly waiting.
    through_changes(false);
}

#[test]
fn merging_and_giving_back_room_after_every_change_leaves_the_candidates_the_same() {
    through_changes(true);
}

#[test]
fn compacting_changes_nothing_but_what_is_held() {
    let mut both = Both::default();
    library(&mut both);
    both.remove(5_001);
    let before = both.smaller.pairs();
    both.smaller.compact();
    assert_eq!(both.smaller.pairs(), before);
    both.smaller.compact();
    assert_eq!(both.smaller.pairs(), before);
    both.assert_same("after compacting twice");
}

#[test]
fn a_removed_entry_has_no_candidates_and_is_nobodys_candidate_before_any_sweep() {
    let mut index = BlockIndex::new();
    let a = track(1, LEN);
    let copy = reencoded(&a, 5, 7);
    index.insert(1, &a);
    index.insert(2, &copy);
    assert_eq!(index.candidates_of(1, &a), [2]);
    index.remove(2);
    assert!(index.candidates_of(1, &a).is_empty());
    assert!(index.candidates_of(2, &copy).is_empty());
    assert_eq!((index.len(), index.contains(2)), (1, false));
    assert_eq!(index.pairs(), []);
}
