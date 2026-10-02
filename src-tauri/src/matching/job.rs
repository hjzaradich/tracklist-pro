//! The matching pass: brings the index up to date with `file.fingerprint`,
//! compares the candidate pairs that have no stored result yet, and stores
//! what it finds. It reads and writes the database only; no audio file is
//! opened.
//!
//! [`Matcher`] keeps the index in memory between passes, so a pass after
//! one new file works on that file alone: its fingerprint is read, its keys
//! go into the index, and only its candidates are compared. A new `Matcher`
//! (after a restart) reads every fingerprint once to rebuild the index, and
//! skips every pair that already has a stored result.
//!
//! Files with exactly the same fingerprint are one index entry. The first
//! of them (lowest file id) stands for all: it gets a full-coverage,
//! score-0 result with each of the others, and results with other
//! fingerprints are stored for it alone.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;

use crate::db::{DbError, Writer};
use crate::fingerprint::Fingerprint;
use crate::jobs::{JobContext, JobError, JobHandler};

use super::compare::{compare, Comparison};
use super::index::{BlockIndex, EntryId};
use super::store::{self, Side};

/// How many fingerprints are read per query.
const PAGE: usize = 500;

/// How many results are written per transaction.
const BATCH: usize = 64;

/// What one pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Present files with a fingerprint this build can read.
    pub files: u64,
    /// Distinct fingerprints among them: the index's entries.
    pub fingerprints: u64,
    /// Files whose fingerprint was new or changed since the last pass.
    pub changed: u64,
    /// Candidate pairs the index gave for the changed fingerprints.
    pub candidates: u64,
    /// Of those, pairs that already had a stored result.
    pub already_compared: u64,
    /// Pairs compared in this pass.
    pub compared: u64,
    /// Results stored, including the ones between identical fingerprints.
    pub stored: u64,
}

/// A fingerprint's identity: the BLAKE3 hash of its stored bytes.
type Digest = [u8; 32];

/// The files sharing one fingerprint.
#[derive(Debug)]
struct Class {
    entry: EntryId,
    /// The fingerprint's length in items.
    items: usize,
    files: BTreeSet<i64>,
    /// The file that stood for the class when its results were last
    /// stored.
    stored_for: Option<i64>,
}

impl Class {
    fn first(&self) -> Option<i64> {
        self.files.first().copied()
    }
}

#[derive(Debug, Default)]
struct State {
    index: BlockIndex,
    digest_of_file: HashMap<i64, Digest>,
    classes: HashMap<Digest, Class>,
    digest_of_entry: HashMap<EntryId, Digest>,
    next_entry: EntryId,
    /// Classes whose candidates haven't all been compared and stored yet.
    /// Kept across passes, so a cancelled pass is picked up by the next.
    due: BTreeSet<EntryId>,
}

impl State {
    fn forget(&mut self, file: i64) {
        let Some(digest) = self.digest_of_file.remove(&file) else {
            return;
        };
        let Some(class) = self.classes.get_mut(&digest) else {
            return;
        };
        class.files.remove(&file);
        if class.files.is_empty() {
            let entry = class.entry;
            self.index.remove(entry);
            self.digest_of_entry.remove(&entry);
            self.due.remove(&entry);
            self.classes.remove(&digest);
        } else if class.stored_for != class.first() {
            // Another file stands for the class now.
            self.due.insert(class.entry);
        }
    }

    /// Takes in `file`'s fingerprint `blob`. Returns whether the file was
    /// new or changed, or `None` if this build can't read the blob.
    fn learn(&mut self, file: i64, blob: &[u8]) -> Option<bool> {
        let digest: Digest = *blake3::hash(blob).as_bytes();
        if self.digest_of_file.get(&file) == Some(&digest) {
            return Some(false);
        }
        self.forget(file);
        if !self.classes.contains_key(&digest) {
            let fingerprint = Fingerprint::from_blob(blob).ok()?;
            let entry = self.next_entry;
            self.next_entry += 1;
            self.index.insert(entry, fingerprint.items());
            self.digest_of_entry.insert(entry, digest);
            self.classes.insert(
                digest,
                Class {
                    entry,
                    items: fingerprint.items().len(),
                    files: BTreeSet::new(),
                    stored_for: None,
                },
            );
        }
        let class = self.classes.get_mut(&digest)?;
        class.files.insert(file);
        self.due.insert(class.entry);
        self.digest_of_file.insert(file, digest);
        Some(true)
    }

    fn class_of_entry(&self, entry: EntryId) -> Option<&Class> {
        self.classes.get(self.digest_of_entry.get(&entry)?)
    }
}

/// Runs matching passes, keeping the index between them. Also the job
/// handler a later stage registers.
#[derive(Debug, Default)]
pub struct Matcher {
    state: Mutex<State>,
}

/// One result waiting to be written: the two files, the blobs compared,
/// and what was found.
struct Found {
    a: i64,
    blob_a: Vec<u8>,
    b: i64,
    blob_b: Vec<u8>,
    comparison: Comparison,
}

fn write(writer: &Writer, batch: Vec<Found>) -> Result<u64, DbError> {
    if batch.is_empty() {
        return Ok(0);
    }
    writer.call(move |conn| {
        let tx = conn.transaction()?;
        let mut stored = 0;
        for found in &batch {
            let a = Side {
                file: found.a,
                blob: &found.blob_a,
            };
            let b = Side {
                file: found.b,
                blob: &found.blob_b,
            };
            stored += u64::from(store::put(&tx, a, b, &found.comparison)?);
        }
        tx.commit()?;
        Ok(stored)
    })
}

impl Matcher {
    pub fn new() -> Matcher {
        Matcher::default()
    }

    /// One pass. `tick` is called often with how far along it is (0 to 1);
    /// an error from it stops the pass at once, and what was stored so far
    /// stays. The next pass goes on from there.
    pub fn pass<E: From<DbError>>(
        &self,
        writer: &Writer,
        tick: &mut dyn FnMut(f64) -> Result<(), E>,
    ) -> Result<Summary, E> {
        // A pass that panicked may have left the index half updated: start
        // over from the file table, which is always enough.
        let mut guard = self.state.lock().unwrap_or_else(|poisoned| {
            let mut guard = poisoned.into_inner();
            *guard = State::default();
            guard
        });
        let state = &mut *guard;
        let mut summary = Summary::default();
        writer.call(|c| store::drop_other_versions(c))?;

        // 1. The index catches up with the file table.
        let mut seen: HashSet<i64> = HashSet::new();
        let mut after = 0;
        loop {
            tick(0.0)?;
            let page = writer.call(move |c| store::fingerprints_after(c, after, PAGE))?;
            let Some(last) = page.last() else {
                break;
            };
            after = last.0;
            for (file, blob) in &page {
                match state.learn(*file, blob) {
                    Some(changed) => {
                        seen.insert(*file);
                        summary.changed += u64::from(changed);
                    }
                    // Unreadable now: whatever it held before is gone.
                    None => state.forget(*file),
                }
            }
        }
        let gone: Vec<i64> = state
            .digest_of_file
            .keys()
            .filter(|file| !seen.contains(file))
            .copied()
            .collect();
        for file in gone {
            state.forget(file);
        }
        summary.files = state.digest_of_file.len() as u64;
        summary.fingerprints = state.classes.len() as u64;

        // 2. Each due class: its identical files, then its candidates.
        let mut compared = writer.call(|c| store::compared_pairs(c))?;
        let due: Vec<EntryId> = state.due.iter().copied().collect();
        let total = due.len().max(1) as f64;
        let mut batch: Vec<Found> = Vec::new();
        // Candidate pairs met in this pass, so one found from both of its
        // sides is counted once.
        let mut met: HashSet<(EntryId, EntryId)> = HashSet::new();
        for (done, entry) in due.into_iter().enumerate() {
            tick(done as f64 / total)?;
            let Some(class) = state.class_of_entry(entry) else {
                state.due.remove(&entry);
                continue;
            };
            let Some(first) = class.first() else {
                continue;
            };
            let Some(blob) = writer.call(move |c| store::fingerprint_of(c, first))? else {
                // Changed under us: the next pass sees it.
                continue;
            };
            let pair = |a: i64, b: i64| (a.min(b), a.max(b));

            for &other in class.files.iter().filter(|&&f| f != first) {
                if compared.insert(pair(first, other)) {
                    batch.push(Found {
                        a: first,
                        blob_a: blob.clone(),
                        b: other,
                        blob_b: blob.clone(),
                        comparison: Comparison::identical(class.items),
                    });
                }
            }

            let Ok(fingerprint) = Fingerprint::from_blob(&blob) else {
                continue;
            };
            for candidate in state.index.candidates_of(entry) {
                if !met.insert((entry.min(candidate), entry.max(candidate))) {
                    continue;
                }
                summary.candidates += 1;
                let Some(other) = state.class_of_entry(candidate).and_then(Class::first) else {
                    continue;
                };
                if compared.contains(&pair(first, other)) {
                    summary.already_compared += 1;
                    continue;
                }
                tick(done as f64 / total)?;
                let Some(other_blob) = writer.call(move |c| store::fingerprint_of(c, other))?
                else {
                    continue;
                };
                let Ok(other_fingerprint) = Fingerprint::from_blob(&other_blob) else {
                    continue;
                };
                // Too long or of another version: nothing to store.
                let Ok(comparison) = compare(&fingerprint, &other_fingerprint) else {
                    continue;
                };
                summary.compared += 1;
                compared.insert(pair(first, other));
                batch.push(Found {
                    a: first,
                    blob_a: blob.clone(),
                    b: other,
                    blob_b: other_blob,
                    comparison,
                });
                if batch.len() >= BATCH {
                    summary.stored += write(writer, std::mem::take(&mut batch))?;
                }
            }
            summary.stored += write(writer, std::mem::take(&mut batch))?;
            if let Some(digest) = state.digest_of_entry.get(&entry).copied() {
                if let Some(class) = state.classes.get_mut(&digest) {
                    class.stored_for = Some(first);
                }
            }
            state.due.remove(&entry);
        }
        tick(1.0)?;
        Ok(summary)
    }
}

impl JobHandler for Matcher {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let summary = self.pass(job.writer(), &mut |fraction| job.progress(fraction))?;
        eprintln!("matching job {} done: {summary:?}", job.id());
        Ok(())
    }
}

/// One pass with a fresh index: everything is read from the file table,
/// and only pairs with no stored result are compared.
pub fn refresh(writer: &Writer) -> Result<Summary, DbError> {
    Matcher::new().pass(writer, &mut |_| Ok(()))
}
