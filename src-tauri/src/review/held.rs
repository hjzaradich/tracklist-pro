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
use crate::grouping::{CONTENT_KEY, MAX_SET_FILES};

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

// ---- what holds a set -------------------------------------------------------

/// What holds a held duplicate set (ROADMAP 1.12): the one thing the user
/// can see and settle. A set can be held for several reasons at once; this
/// is the first of them that can be named, in the order the grouping
/// planner would look, except that a pair of files the rules didn't call
/// duplicates comes before the user's decisions and the links (a cut or a
/// rework pair is why those links exist, and its reasons say more): two of
/// its tracks are in the Library; a pair of its files that is not a
/// duplicate (the usual case: one file's names say another version, and
/// that one pair holds every file beside it); two tracks the user kept
/// separate; two tracks linked as versions; too many files to compare.
///
/// It is read from what grouping stored, not from the plan that held the
/// set, so a set held for a reason that can't be seen here has no holding.
/// The files are named by id, the lower track first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holding {
    /// Two of the set's tracks are in the Library, and the job never merges
    /// those: a file of each (its linked file).
    TwoInLibrary { files: [i64; 2] },
    /// Two files of two tracks of the set whose pair is not a duplicate: the
    /// rules' own stored reasons for it (a JSON array, the deciding first).
    Pair { files: [i64; 2], reasons: String },
    /// The user said these two tracks aren't the same ("Keep separate" or a
    /// split): a file of each.
    KeptSeparate { files: [i64; 2] },
    /// Two of the set's tracks are linked as versions: a file of each.
    LinkedVersions { files: [i64; 2] },
    /// More files than the job compares (the planner's cap), holding more
    /// than one kind of audio.
    TooManyFiles { files: u32, most: u32 },
}

impl Holding {
    /// The files the reason names, to be shown by their names.
    pub fn named_files(&self) -> Vec<i64> {
        match self {
            Holding::TwoInLibrary { files }
            | Holding::Pair { files, .. }
            | Holding::KeptSeparate { files }
            | Holding::LinkedVersions { files } => files.to_vec(),
            Holding::TooManyFiles { .. } => Vec::new(),
        }
    }
}

/// A track's file for a sentence that names it: the Library's linked file,
/// else the track's best file, else its lowest file id.
fn file_of(conn: &Connection, track: i64) -> rusqlite::Result<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT rf.file_id FROM recording_file rf
         LEFT JOIN library_track lt ON lt.linked_file_id = rf.file_id
         WHERE rf.recording_id = ?1
         ORDER BY lt.recording_id IS NOT NULL DESC, rf.role = 'best' DESC, rf.file_id
         LIMIT 1",
    )?;
    let mut rows = stmt.query_map([track], |r| r.get(0))?;
    rows.next().transpose()
}

/// A file of each of two tracks.
fn files_of(conn: &Connection, a: i64, b: i64) -> rusqlite::Result<Option<[i64; 2]>> {
    Ok(file_of(conn, a)?
        .zip(file_of(conn, b)?)
        .map(|(a, b)| [a, b]))
}

/// What holds the set of `tracks` (its [`Component::tracks`]), or `None`
/// when nothing stored shows it.
pub fn holding(conn: &Connection, tracks: &[i64]) -> rusqlite::Result<Option<Holding>> {
    let json = serde_json::to_string(tracks).unwrap_or_else(|_| "[]".into());

    let in_library: Vec<i64> = conn
        .prepare_cached(
            "SELECT recording_id FROM library_track
             WHERE recording_id IN (SELECT value FROM json_each(?1))
             ORDER BY recording_id LIMIT 2",
        )?
        .query_map([&json], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if let [a, b] = in_library[..] {
        if let Some(files) = files_of(conn, a, b)? {
            return Ok(Some(Holding::TwoInLibrary { files }));
        }
    }

    // A pair that is not a duplicate, between two files on disk that hold
    // different audio, in two tracks of the set: the states a stored pair
    // that nobody has settled can be in (`open`, `linked` for a cut or a
    // rework, `apart`).
    let pair: Option<(i64, i64, String)> = conn
        .prepare_cached(
            "SELECT p.file_a, p.file_b, p.reasons
             FROM grouping_pair p
             JOIN recording_file ra ON ra.file_id = p.file_a
             JOIN recording_file rb ON rb.file_id = p.file_b
             JOIN file fa ON fa.id = p.file_a
             JOIN file fb ON fb.id = p.file_b
             WHERE p.outcome <> 'duplicate' AND p.state IN ('open', 'linked', 'apart')
               AND ra.recording_id IN (SELECT value FROM json_each(?1))
               AND rb.recording_id IN (SELECT value FROM json_each(?1))
               AND ra.recording_id <> rb.recording_id
               AND fa.present = 1 AND fb.present = 1
               AND NOT coalesce(length(fa.audio_hash) > 0 AND fa.audio_hash = fb.audio_hash, 0)
             ORDER BY p.file_a, p.file_b LIMIT 1",
        )?
        .query_map([&json], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .next()
        .transpose()?;
    if let Some((a, b, reasons)) = pair {
        return Ok(Some(Holding::Pair {
            files: [a, b],
            reasons,
        }));
    }

    // The user's "apart": decided on the audio two files hold, so it
    // covers every pair of files between the two tracks.
    let apart: Option<[i64; 2]> = conn
        .prepare_cached(&format!(
            "SELECT ra.file_id, rb.file_id
             FROM recording_file ra
             JOIN recording_file rb ON rb.recording_id > ra.recording_id
             JOIN file fa ON fa.id = ra.file_id
             JOIN file fb ON fb.id = rb.file_id
             JOIN grouping_decision d ON d.decision = 'apart'
                  AND d.key_a = min({key_a}, {key_b}) AND d.key_b = max({key_a}, {key_b})
             WHERE ra.recording_id IN (SELECT value FROM json_each(?1))
               AND rb.recording_id IN (SELECT value FROM json_each(?1))
             ORDER BY ra.file_id, rb.file_id LIMIT 1",
            key_a = CONTENT_KEY.replace("f.", "fa."),
            key_b = CONTENT_KEY.replace("f.", "fb."),
        ))?
        .query_map([&json], |r| Ok([r.get(0)?, r.get(1)?]))?
        .next()
        .transpose()?;
    if let Some(files) = apart {
        return Ok(Some(Holding::KeptSeparate { files }));
    }

    let link: Option<(i64, i64)> = conn
        .prepare_cached(
            "SELECT recording_a, recording_b FROM version_link
             WHERE recording_a IN (SELECT value FROM json_each(?1))
               AND recording_b IN (SELECT value FROM json_each(?1))
             ORDER BY id LIMIT 1",
        )?
        .query_map([&json], |r| Ok((r.get(0)?, r.get(1)?)))?
        .next()
        .transpose()?;
    if let Some((a, b)) = link {
        if let Some(files) = files_of(conn, a, b)? {
            return Ok(Some(Holding::LinkedVersions { files }));
        }
    }

    // Every file counts, on disk or not, as the planner counts them; one
    // kind of audio never holds a set.
    let (files, with_hash, kinds): (u32, u32, u32) = conn.query_row(
        "SELECT count(*), count(nullif(f.audio_hash, x'')),
                count(DISTINCT nullif(f.audio_hash, x''))
         FROM recording_file rf JOIN file f ON f.id = rf.file_id
         WHERE rf.recording_id IN (SELECT value FROM json_each(?1))",
        [&json],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let most = MAX_SET_FILES as u32;
    if files > most && !(with_hash == files && kinds == 1) {
        return Ok(Some(Holding::TooManyFiles { files, most }));
    }
    Ok(None)
}
