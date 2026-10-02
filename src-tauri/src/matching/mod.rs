//! Fingerprint matching: which files hold the same audio, and how much of
//! it (1bA-1, 1bA-2; ROADMAP 1.4).
//!
//! These are measuring tools. Nothing here merges tracks, links versions
//! or writes to `recording` or `version_link`: the grouping rules (1bB)
//! read the measurements and decide. Nothing here opens an audio file
//! either; it all works on the fingerprints already in `file.fingerprint`.
//!
//! Exact `audio_hash` matches are the fast path and never come through
//! here ([`crate::grouping`]).
//!
//! # Finding candidates: the block index ([`index`])
//!
//! Comparing every file with every other is n² comparisons. Instead, each
//! fingerprint gives a small set of **keys**, and only fingerprints sharing
//! keys are compared.
//!
//! - A fingerprint is one 32-bit item per ~0.124 s of audio. Two encodings
//!   of the same audio agree on some items bit for bit, wherever in the
//!   track they sit: about 9 in 10 between lossless files, and about 1 in 7
//!   between a WAV and an MP3 of it (measured on generated audio, at a
//!   difference score of about 2).
//! - A fingerprint's keys are its distinct items whose mixed value falls in
//!   one class out of [`index::SAMPLE`]. The choice depends on the item's
//!   value only, so two fingerprints holding the same item agree on whether
//!   it's a key. Keys come from the **whole track**, not a short window, so
//!   a Radio Edit that starts three minutes into an Extended mix still
//!   shares keys with it.
//! - Two fingerprints are a **candidate pair** when they share at least
//!   [`index::MIN_SHARED_KEYS`] keys. A key held by more than
//!   [`index::MAX_ENTRIES_PER_KEY`] fingerprints (silence, a sound every
//!   track has) is skipped.
//!
//! The index is built from `file.fingerprint` alone and holds only keys.
//! Adding or removing a fingerprint touches that fingerprint's keys and
//! nothing else. Work per fingerprint is its keys times the few
//! fingerprints sharing each, so the whole pass grows with the number of
//! files and of real matches, not with n².
//!
//! **What it can miss** (a false negative is a pair never compared, so
//! never reported):
//!
//! - **A weak match with little shared audio.** With a share `e` of items
//!   bit-identical over `n` shared items, the pair shares about
//!   `n · e / SAMPLE` keys. At the duplicate rule's limit (score 4, where
//!   `e` is roughly 2–3%) a pair sharing two minutes still has about 6
//!   keys, but one sharing 30 seconds has under 2 and may be missed. A
//!   rework that keeps only a short hook is the usual case; ROADMAP 1.4
//!   leaves those to names.
//! - **A copy at another speed or pitch.** Its items differ nearly
//!   everywhere, so it shares no keys. (The comparison wouldn't call it a
//!   duplicate either: see the tests.)
//! - **Audio whose only shared items are skipped keys:** two files that are
//!   mostly silence, or more than [`index::MAX_ENTRIES_PER_KEY`] different
//!   encodings of one track. Files with the very same fingerprint count
//!   once, so any number of exact copies is fine.
//!
//! # Comparing a pair ([`compare()`])
//!
//! Always on the two full-track fingerprints.
//!
//! **The main alignment** is rusty-chromaprint's matcher over both whole
//! fingerprints: the single best way to lay B over A, cut into stretches
//! where the audio agrees. ROADMAP 1.4's rule is judged on it, exactly as
//! E3 graded it:
//!
//! - **`coverage_a`** is the number of A's items inside the main
//!   alignment's matched stretches, divided by A's item count. 0 to 1.
//!   **`coverage_b`** is the same for B. They differ when one file is
//!   longer: a Radio Edit inside an Extended mix is covered ~100%, the mix
//!   much less.
//! - **`score`** is the difference score: over the main alignment's matched
//!   stretches, the average number of bits (out of 32) that differ between
//!   A's item and the B item laid over it. Unit: bits per item, 0 to 32. 0
//!   is bit-identical; unrelated audio averages 16; a stretch only counts
//!   as matched under 10 (the matcher's own limit). When nothing matched
//!   it's 32.
//!
//! So "duplicate: coverage ≥ 90% on both and score ≤ 4" is
//! `coverage_a >= 0.9 && coverage_b >= 0.9 && score <= 4.0`, unchanged.
//! These three numbers equal [`crate::fingerprint::compare`]'s.
//!
//! **The segments** are every matched stretch with its offsets, in items
//! ([`item_seconds`] each): `offset_a`, `offset_b`, `items`, its own
//! `score`, and which `alignment` found it. A cut is "B found inside A":
//! B's start sits `offset_a - offset_b` items into A.
//!
//! **Secondary alignments.** A cut with a part taken out (an edit that
//! drops a breakdown) lines up with the original in two pieces at two
//! different shifts, and the main alignment can only hold one. So after
//! the main alignment, the unmatched stretches of A are matched against
//! the unmatched stretches of B, the best pair is kept, and so on, up to
//! [`compare::MAX_ALIGNMENTS`] alignments. Only stretches of at least
//! [`compare::MIN_SECONDARY_ITEMS`] items (~5 s) are tried or kept, and no
//! item is ever in two segments. [`Comparison::found_a`] and
//! [`Comparison::found_b`] give the share of each file found over every
//! alignment: what a cut should be judged on. They never feed the
//! duplicate rule: a file that needs two alignments to cover it has had
//! something taken out, so it isn't the same audio.
//!
//! # Keeping results ([`store`], [`job`])
//!
//! Results live in `fingerprint_match` (migration 0016), one row per
//! compared pair of files, compared-and-nothing-found included. A file's
//! rows are deleted by the database when its `fingerprint` bytes change,
//! and stay through a tag rewrite or a new modified time (ROADMAP 5.1).
//! Files with the very same fingerprint are handled as one: see [`job`].
//!
//! [`Matcher`] is the pass that keeps the table up to date, and a job
//! handler. It isn't wired into the scan chain yet.

pub mod compare;
pub mod index;
pub mod job;
pub mod store;

pub use compare::{compare, item_seconds, Comparison, Segment};
pub use index::BlockIndex;
pub use job::{refresh, Matcher, Summary};
pub use store::StoredMatch;

/// The version of the comparison's definition, stored with every result.
/// Bump it when [`compare()`] would give other numbers for the same
/// fingerprints; results of another version are worked out again.
pub const VERSION: u32 = 1;

#[cfg(test)]
mod tests;
