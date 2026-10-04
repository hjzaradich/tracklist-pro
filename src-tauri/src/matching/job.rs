//! The matching pass: brings the index up to date with `file.fingerprint`,
//! compares the candidate pairs that have no stored result yet, and stores
//! what it finds. It reads and writes the database only; no audio file is
//! opened.
//!
//! [`Matcher`] keeps the index in memory between passes, so a pass after
//! one new file compares that file alone: every pass reads each file's
//! fingerprint bytes to see what changed, but only a new or changed one is
//! decoded, only its keys go into the index, and only its candidates are
//! compared. A new `Matcher` (after a restart) rebuilds the index from
//! every fingerprint, and skips every pair that already has a stored
//! result.
//!
//! What the index holds, and what a pass costs at 10,000 and 100,000
//! files, is measured by `tests/matching_memory.rs` (1bA-13). The index
//! keeps 8 bytes per (key, fingerprint) and little else: a fingerprint
//! that goes is swept out once per pass, and a fingerprint's candidates
//! are asked for with the fingerprint itself, read for comparing anyway.
//!
//! The database is the truth about what has been done, not the
//! `Matcher`'s memory. `fingerprint_matched` (migration 0019) has a row for
//! each file whose current fingerprint has been through a pass: its
//! candidates found and every result stored, the row written in the same
//! transaction as the last of them. A pass works on the fingerprints with
//! a file that has no such row, and on nothing else: after a restart only
//! what's new, after a stop only what was left. The database deletes a
//! file's row (and its results) when its fingerprint changes, even if it
//! changes back before the next pass, and when a missing file comes back.
//!
//! A big index isn't kept: above [`KEEP_INDEX_UP_TO`] it's dropped when a
//! pass finishes, and the next pass that has something to do builds it
//! again.
//!
//! A pair is always compared with the lower file id as A, so what's stored
//! doesn't depend on which of the two files arrived last (the matcher
//! isn't symmetric on weak matches).
//!
//! Files with exactly the same fingerprint are one index entry. The first
//! of them (lowest file id) stands for all: it gets a full-coverage,
//! score-0 result with each of the others, and results with other
//! fingerprints are stored for it alone.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;

use crate::db::{DbError, Writer};
use crate::fingerprint::Fingerprint;
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, NewJob, Priority};

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
    /// Files whose fingerprint this matcher hadn't read, or that changed
    /// since it did: every file, for a matcher that kept no index.
    pub changed: u64,
    /// Candidate pairs the index gave for the fingerprints that were due.
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
            self.classes.remove(&digest);
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
                },
            );
        }
        let class = self.classes.get_mut(&digest)?;
        class.files.insert(file);
        self.digest_of_file.insert(file, digest);
        Some(true)
    }

    /// The classes a pass has work on, lowest entry first: those with a
    /// file that `matched` (each covered file and the file standing for
    /// it) doesn't have, or has under another file than the one standing
    /// for the class now.
    fn due(&self, matched: &HashMap<i64, i64>) -> Vec<EntryId> {
        let mut due: Vec<EntryId> = self
            .classes
            .values()
            .filter(|class| {
                let first = class.first();
                class
                    .files
                    .iter()
                    .any(|file| matched.get(file).copied() != first)
            })
            .map(|class| class.entry)
            .collect();
        due.sort_unstable();
        due
    }

    fn class_of_entry(&self, entry: EntryId) -> Option<&Class> {
        self.classes.get(self.digest_of_entry.get(&entry)?)
    }
}

/// The matching job's priority: below the scan chain's background work,
/// so a relink or attach queued meanwhile is always taken first (1bA-14).
pub const PRIORITY: Priority = Priority(Priority::BACKGROUND.0 - 10);

/// How many progress reports go by between looks for a waiting relink or
/// attach job.
const LOOK_EVERY: u64 = 64;

/// The index is kept between passes while it holds no more than this
/// (250 MiB: about 45,000 six-minute tracks at their fullest). A bigger
/// one is dropped when a pass finishes, and built again by the next pass
/// that has something to do (about two minutes at 100,000 files, measured
/// in 1bA-13).
pub const KEEP_INDEX_UP_TO: usize = 250 << 20;

/// A matching job: one pass over every fingerprint.
pub fn matching_job() -> NewJob {
    NewJob::new(JobKind::Match).priority(PRIORITY)
}

/// Queues a matching job if any file is due one, unless one is already
/// queued (a running one is asked to run once more). Called by the scan
/// chain when the fingerprints are in, or when none were due.
pub(crate) fn request<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    if writer.call(|c| store::any_due(c))? {
        crate::scan::chain::queue_once(writer, matching_job(), enqueue)?;
    }
    Ok(())
}

/// Whether a relink or attach job is waiting. The matching job makes way
/// for them: they're what the user sees next, matching only measures.
fn others_waiting(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM job WHERE status = 'queued' AND kind IN (?1, ?2))",
    )?
    .query_row([JobKind::Relink.as_str(), JobKind::Attach.as_str()], |r| {
        r.get(0)
    })
}

/// Why a matching job's pass stopped early.
enum Stop {
    /// Cancelled, or it failed.
    Job(JobError),
    /// A relink or attach is waiting for the worker.
    MakeWay,
}

impl From<DbError> for Stop {
    fn from(e: DbError) -> Stop {
        Stop::Job(e.into())
    }
}

impl From<JobError> for Stop {
    fn from(e: JobError) -> Stop {
        Stop::Job(e)
    }
}

/// Test hook: called with each progress report's number, from 1.
type ReportHook = Box<dyn Fn(u64) + Send + Sync>;

/// Runs matching passes, keeping the index between them. Also the matching
/// job's handler: one long-lived instance, so the index is kept from one
/// job to the next.
pub struct Matcher {
    state: Mutex<State>,
    /// See [`KEEP_INDEX_UP_TO`].
    keep_index_up_to: usize,
    on_report: Option<ReportHook>,
}

impl Default for Matcher {
    fn default() -> Matcher {
        Matcher {
            state: Mutex::default(),
            keep_index_up_to: KEEP_INDEX_UP_TO,
            on_report: None,
        }
    }
}

impl std::fmt::Debug for Matcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Matcher").finish_non_exhaustive()
    }
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

/// Files that are through: every result of theirs is stored, or goes in
/// with this.
struct Through {
    files: Vec<i64>,
    /// The fingerprint they all hold.
    blob: Vec<u8>,
    /// The file whose results stand for them.
    stands: i64,
}

/// Stores a batch of results and, in the same transaction, records the
/// files that are `through` with it. Returns how many results were stored.
fn write(writer: &Writer, batch: Vec<Found>, through: Option<Through>) -> Result<u64, DbError> {
    if batch.is_empty() && through.is_none() {
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
        if let Some(through) = &through {
            for &file in &through.files {
                let side = Side {
                    file,
                    blob: &through.blob,
                };
                store::mark_matched(&tx, side, through.stands)?;
            }
        }
        tx.commit()?;
        Ok(stored)
    })
}

impl Matcher {
    pub fn new() -> Matcher {
        Matcher::default()
    }

    /// Keeps the index between passes only up to `bytes`.
    #[cfg(test)]
    pub(crate) fn keep_index_up_to(mut self, bytes: usize) -> Self {
        self.keep_index_up_to = bytes;
        self
    }

    /// Calls `hook` with each progress report's number, as a job reports.
    #[cfg(test)]
    pub(crate) fn on_report(mut self, hook: impl Fn(u64) + Send + Sync + 'static) -> Self {
        self.on_report = Some(Box::new(hook));
        self
    }

    /// Drops the index if it's over what's kept between passes. A pass
    /// that finishes does this itself; this is for one that failed.
    fn drop_a_big_index(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.index.held_bytes() > self.keep_index_up_to {
            *state = State::default();
        }
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
        // Files whose fingerprint this build can't read: nothing to compare.
        let mut unreadable: Vec<(i64, Vec<u8>)> = Vec::new();
        let mut after = 0;
        loop {
            tick(0.0)?;
            let page = writer.call(move |c| store::fingerprints_after(c, after, PAGE))?;
            let Some(last) = page.last() else {
                break;
            };
            after = last.0;
            for (file, blob) in page {
                match state.learn(file, &blob) {
                    Some(changed) => {
                        seen.insert(file);
                        summary.changed += u64::from(changed);
                    }
                    // Unreadable now: whatever it held before is gone.
                    None => {
                        state.forget(file);
                        unreadable.push((file, blob));
                    }
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
        // The new fingerprints' keys are merged in and the ones that went
        // leave for good, so finding candidates is quick and nothing spare
        // is held.
        state.index.compact();
        summary.files = state.digest_of_file.len() as u64;
        summary.fingerprints = state.classes.len() as u64;

        // 2. What's due: the fingerprints with a file that hasn't been
        // through a pass, whatever this matcher remembers. A fingerprint
        // that can't be read is through as it is.
        let matched = writer.call(|c| store::matched(c))?;
        unreadable.retain(|(file, _)| matched.get(file) != Some(file));
        if !unreadable.is_empty() {
            writer.call(move |conn| {
                let tx = conn.transaction()?;
                for (file, blob) in &unreadable {
                    store::mark_matched(&tx, Side { file: *file, blob }, *file)?;
                }
                tx.commit()
            })?;
        }
        let due = state.due(&matched);
        drop(matched);
        let mut compared = if due.is_empty() {
            HashSet::new()
        } else {
            writer.call(|c| store::compared_pairs(c))?
        };

        // 3. Each due class: its identical files, then its candidates.
        let total = due.len().max(1) as f64;
        let mut batch: Vec<Found> = Vec::new();
        // Candidate pairs met in this pass, so one found from both of its
        // sides is counted once.
        let mut met: HashSet<(EntryId, EntryId)> = HashSet::new();
        for (done, entry) in due.into_iter().enumerate() {
            tick(done as f64 / total)?;
            let Some(class) = state.class_of_entry(entry) else {
                continue;
            };
            let Some(first) = class.first() else {
                continue;
            };
            let Some(blob) = writer.call(move |c| store::fingerprint_of(c, first))? else {
                // Changed under us: the next pass sees it.
                continue;
            };
            // The index holds this class's keys only as postings: its
            // candidates are asked for with the fingerprint itself, which
            // must be the one the index read.
            if state.digest_of_entry.get(&entry) != Some(blake3::hash(&blob).as_bytes()) {
                continue;
            }
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
            for candidate in state.index.candidates_of(entry, fingerprint.items()) {
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
                // The lower file id is always A.
                let mut a = (first, &fingerprint, blob.clone());
                let mut b = (other, &other_fingerprint, other_blob);
                if a.0 > b.0 {
                    std::mem::swap(&mut a, &mut b);
                }
                // Too long or of another version: nothing to store.
                let Ok(comparison) = compare(a.1, b.1) else {
                    continue;
                };
                summary.compared += 1;
                compared.insert(pair(first, other));
                batch.push(Found {
                    a: a.0,
                    blob_a: a.2,
                    b: b.0,
                    blob_b: b.2,
                    comparison,
                });
                if batch.len() >= BATCH {
                    summary.stored += write(writer, std::mem::take(&mut batch), None)?;
                }
            }
            // The last of its results and "these files are through" go in
            // together: a pass cut short before this leaves them due.
            let through = Through {
                files: class.files.iter().copied().collect(),
                blob,
                stands: first,
            };
            summary.stored += write(writer, std::mem::take(&mut batch), Some(through))?;
        }
        if state.index.held_bytes() > self.keep_index_up_to {
            *state = State::default();
        }
        tick(1.0)?;
        Ok(summary)
    }
}

impl JobHandler for Matcher {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let mut reports = 0u64;
        let passed = self.pass(job.writer(), &mut |fraction| -> Result<(), Stop> {
            job.progress(fraction)?;
            reports += 1;
            if let Some(hook) = &self.on_report {
                hook(reports);
            }
            if reports.is_multiple_of(LOOK_EVERY) && job.writer().call(|c| others_waiting(c))? {
                return Err(Stop::MakeWay);
            }
            Ok(())
        });
        match passed {
            Ok(summary) => {
                eprintln!("matching job {} done: {summary:?}", job.id());
                Ok(())
            }
            // What was stored stays. The rest is done by one more run, at
            // this job's priority, so it comes after the waiting job: this
            // job is still running, so it's asked to run once more, and its
            // chain wrapper queues that as it ends.
            Err(Stop::MakeWay) => {
                crate::scan::chain::queue_once(job.writer(), matching_job(), |j| job.enqueue(j))?;
                Ok(())
            }
            Err(Stop::Job(e)) => {
                self.drop_a_big_index();
                Err(e)
            }
        }
    }
}

/// One pass with a fresh index: everything is read from the file table,
/// and only the fingerprints that are due are worked on.
pub fn refresh(writer: &Writer) -> Result<Summary, DbError> {
    Matcher::new().pass(writer, &mut |_| Ok(()))
}
