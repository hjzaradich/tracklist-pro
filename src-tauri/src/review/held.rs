//! Held duplicate sets and Library tracks that are the same recording: the
//! tracks joined by the grouping pairs held in the states `set_disagrees`
//! and `both_in_library`, as sets (ROADMAP 1.4).
//!
//! The one place the queue works in Rust rather than SQL, because a set is
//! a connected component and SQL has no cheap way to ask for one. It reads
//! only the held pairs (two track ids and three flags each), never the
//! library. A set is named by its lowest track id, so the id does not depend
//! on the order the pairs are read in.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;

use super::PENDING_PAIRS;

/// One held pair, as its two tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldPair {
    pub a: i64,
    pub b: i64,
    /// The pair's state is `both_in_library` (else `set_disagrees`).
    pub both_in_library: bool,
    pub a_in_library: bool,
    pub b_in_library: bool,
}

/// A set of tracks joined by held pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// The lowest track id: the set's name.
    pub anchor: i64,
    /// Every track in the set, lowest first.
    pub tracks: Vec<i64>,
    /// The set holds two Library tracks that are the same recording
    /// (a `both_in_library` pair); else it is a held duplicate set.
    pub same_recording: bool,
}

/// Union-find over track ids, keeping each set's lowest id as its root.
#[derive(Default)]
struct Sets {
    parent: HashMap<i64, i64>,
}

impl Sets {
    fn find(&mut self, id: i64) -> i64 {
        let mut root = *self.parent.entry(id).or_insert(id);
        while self.parent[&root] != root {
            root = self.parent[&root];
        }
        // Compress the path.
        let mut at = id;
        while at != root {
            let next = self.parent[&at];
            self.parent.insert(at, root);
            at = next;
        }
        root
    }

    fn union(&mut self, a: i64, b: i64) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // The lower id is the root, so a set's root is its anchor.
            self.parent.insert(ra.max(rb), ra.min(rb));
        }
    }
}

/// The sets the pairs make that hold at least one Library track, by anchor.
/// Pure: the same pairs in any order give the same sets.
pub fn components_of(pairs: &[HeldPair]) -> Vec<Component> {
    let mut sets = Sets::default();
    for pair in pairs {
        sets.union(pair.a, pair.b);
    }
    // anchor -> (tracks, in the Library, same recording)
    let mut found: BTreeMap<i64, (Vec<i64>, bool, bool)> = BTreeMap::new();
    for pair in pairs {
        for (track, in_library) in [(pair.a, pair.a_in_library), (pair.b, pair.b_in_library)] {
            let anchor = sets.find(track);
            let entry = found.entry(anchor).or_default();
            entry.0.push(track);
            entry.1 |= in_library;
            entry.2 |= pair.both_in_library;
        }
    }
    found
        .into_iter()
        .filter(|(_, (_, in_library, _))| *in_library)
        .map(|(anchor, (mut tracks, _, same_recording))| {
            tracks.sort_unstable();
            tracks.dedup();
            Component {
                anchor,
                tracks,
                same_recording,
            }
        })
        .collect()
}

/// The query behind [`held_pairs`]; a test reads its plan.
pub(super) fn held_pairs_sql() -> String {
    format!(
        "SELECT ra.recording_id, rb.recording_id, state = 'both_in_library',
                ra.recording_id IN (SELECT recording_id FROM library_track),
                rb.recording_id IN (SELECT recording_id FROM library_track)
         FROM grouping_pair
         JOIN recording_file ra ON ra.file_id = grouping_pair.file_a
         JOIN recording_file rb ON rb.file_id = grouping_pair.file_b
         WHERE {PENDING_PAIRS} AND state <> 'open' AND ra.recording_id <> rb.recording_id"
    )
}

/// The held pairs: every `set_disagrees` and `both_in_library` pair between
/// two tracks, with whether each track is in the Library.
pub fn held_pairs(conn: &Connection) -> rusqlite::Result<Vec<HeldPair>> {
    let mut stmt = conn.prepare_cached(&held_pairs_sql())?;
    let rows = stmt.query_map([], |r| {
        Ok(HeldPair {
            a: r.get(0)?,
            b: r.get(1)?,
            both_in_library: r.get(2)?,
            a_in_library: r.get(3)?,
            b_in_library: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// Every queued held set and same-recording set, by anchor.
pub fn components(conn: &Connection) -> rusqlite::Result<Vec<Component>> {
    Ok(components_of(&held_pairs(conn)?))
}
