//! The provisional grouping rule, as a pure function: which files go to
//! which track. It reads nothing and writes nothing; [`super::store`]
//! loads its input and applies its answer.
//!
//! This is the piece 1bC-1 replaces with real grouping (fingerprint,
//! versions). Keep it that small.
//!
//! # The rule
//!
//! - Files with the same non-NULL `audio_hash` share one track.
//! - Every other file has a track of its own. A file with no `audio_hash`
//!   (not hashed yet, unknown format, damaged) is never merged with
//!   anything.
//!
//! # Settling a track that already holds files
//!
//! Every track is either one file with no `audio_hash`, or files that all
//! share one. A track that breaks this (a file's audio changed since it was
//! grouped) keeps the files sharing the most common `audio_hash`, and the
//! rest leave for the track their own audio belongs to. Ties go to the
//! hash held by the lowest file id, so a re-run never flips a decision.
//! Where several tracks hold the same audio, they all merge into one:
//! the one with a pinned file, else the lowest id, so the id everything
//! else refers to survives.
//!
//! # What never moves
//!
//! - A **pinned** file, one a Library track points at, never leaves its
//!   track: that would leave the Library track playing a file its track
//!   doesn't hold.
//! - A file never moves between two tracks that are linked versions:
//!   versions never merge (the schema refuses it too). It's left where it
//!   is and counted.

use std::collections::{BTreeMap, BTreeSet, HashSet};

/// A file that has, or should get, a track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// `file.id`.
    pub file: i64,
    /// Its track now, if it has one.
    pub recording: Option<i64>,
    /// Its `audio_hash`, `None` for a file with none.
    pub key: Option<Vec<u8>>,
    /// A Library track points at the file, so it can't move.
    pub pinned: bool,
}

/// Where a file goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Dest {
    /// An existing track.
    Recording(i64),
    /// A new track; files with the same number share it.
    New(usize),
}

/// One file's change of track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub file: i64,
    /// Its track now, `None` if it has none yet.
    pub from: Option<i64>,
    pub to: Dest,
}

/// What to do, in the order to do it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub moves: Vec<Move>,
    /// How many new tracks the moves need.
    pub new_recordings: usize,
    /// Files that should have moved but were left: between linked
    /// versions.
    pub left_alone: usize,
}

/// The two tracks are linked versions of each other. Pairs are stored with
/// the smaller id first.
fn linked(pairs: &HashSet<(i64, i64)>, a: i64, b: i64) -> bool {
    pairs.contains(&(a.min(b), a.max(b)))
}

/// What one `audio_hash` weighs within a track holding files with several.
#[derive(Default)]
struct Weight {
    pinned: bool,
    count: usize,
    lowest_file: i64,
}

/// The `audio_hash` a track keeps, if it has files with one.
fn winning_key<'a>(members: &[&'a Member]) -> Option<&'a [u8]> {
    let mut weights: BTreeMap<&[u8], Weight> = BTreeMap::new();
    for m in members {
        if let Some(key) = m.key.as_deref() {
            let w = weights.entry(key).or_insert(Weight {
                pinned: false,
                count: 0,
                lowest_file: i64::MAX,
            });
            w.pinned |= m.pinned;
            w.count += 1;
            w.lowest_file = w.lowest_file.min(m.file);
        }
    }
    weights
        .into_iter()
        .max_by_key(|(_, w)| (w.pinned, w.count, std::cmp::Reverse(w.lowest_file)))
        .map(|(key, _)| key)
}

/// Works out where every file should go. `members` holds every file with a
/// track and every present file without one; `versions` the pairs of
/// tracks that are linked versions.
pub fn plan(members: &[Member], versions: &HashSet<(i64, i64)>) -> Plan {
    let mut by_recording: BTreeMap<i64, Vec<&Member>> = BTreeMap::new();
    for m in members {
        if let Some(r) = m.recording {
            by_recording.entry(r).or_default().push(m);
        }
    }

    // Settle each track: which audio it keeps, and which files stay.
    let mut stays: HashSet<i64> = HashSet::new();
    // The audio each track keeps, by hash; tracks that hold none aren't
    // listed (a track with one hash-less file).
    let mut homes: BTreeMap<&[u8], Vec<i64>> = BTreeMap::new();
    for (&recording, held) in &by_recording {
        let keeps = winning_key(held);
        let lone = held.iter().map(|m| m.file).min();
        for m in held {
            let fits = match keeps {
                Some(key) => m.key.as_deref() == Some(key),
                None => Some(m.file) == lone,
            };
            if fits || m.pinned {
                stays.insert(m.file);
            }
        }
        if let Some(key) = keeps {
            homes.entry(key).or_default().push(recording);
        }
    }
    let pinned_in = |recording: i64| {
        by_recording[&recording]
            .iter()
            .any(|m| m.pinned && stays.contains(&m.file))
    };

    let mut moves = Vec::new();
    let mut new_recordings = 0;
    let mut left_alone = 0;
    let mut alone = |moves: &mut Vec<Move>, file: i64, from: Option<i64>, to: Dest| {
        if let (Some(from), Dest::Recording(to)) = (from, to) {
            if linked(versions, from, to) {
                left_alone += 1;
                return;
            }
        }
        moves.push(Move { file, from, to });
    };

    // Files with a hash, one hash at a time.
    let mut hashes: BTreeSet<&[u8]> = homes.keys().copied().collect();
    for m in members {
        if let (Some(key), false) = (m.key.as_deref(), stays.contains(&m.file)) {
            hashes.insert(key);
        }
    }
    for key in hashes {
        // The track this audio ends up in: an existing one that keeps it,
        // preferring one with a pinned file, then the lowest id.
        let candidates = homes.get(key).map(Vec::as_slice).unwrap_or(&[]);
        let target = candidates
            .iter()
            .copied()
            .min_by_key(|&r| (!pinned_in(r), r));
        let dest = match target {
            Some(r) => Dest::Recording(r),
            None => {
                new_recordings += 1;
                Dest::New(new_recordings - 1)
            }
        };
        for m in members {
            if m.key.as_deref() != Some(key) {
                continue;
            }
            // Files already in the target stay; other tracks holding this
            // audio merge into it, unless a file is pinned.
            if stays.contains(&m.file) && (m.recording == target || m.pinned) {
                continue;
            }
            alone(&mut moves, m.file, m.recording, dest);
        }
    }

    // Everything else has no audio_hash: a track of its own each.
    for m in members {
        if m.key.is_none() && !stays.contains(&m.file) {
            new_recordings += 1;
            moves.push(Move {
                file: m.file,
                from: m.recording,
                to: Dest::New(new_recordings - 1),
            });
        }
    }

    Plan {
        moves,
        new_recordings,
        left_alone,
    }
}
