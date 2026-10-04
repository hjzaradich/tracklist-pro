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
//! The table is the truth about what has been compared, not the
//! `Matcher`'s memory. The database deletes a file's results when its
//! fingerprint changes, even if it changes back before the next pass, so
//! each pass looks at which files have fewer results than it left them
//! with and works those out again.
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
    /// How many stored results each file was part of when the last
    /// finished pass ended. A file with fewer now has lost some.
    results_of_file: HashMap<i64, usize>,
}

/// How many of the `pairs` each file is part of.
fn results_per_file(pairs: &HashSet<(i64, i64)>) -> HashMap<i64, usize> {
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for &(a, b) in pairs {
        *counts.entry(a).or_default() += 1;
        *counts.entry(b).or_default() += 1;
    }
    counts
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

    /// Makes due again every class with a file that has lost stored
    /// results since the last finished pass: `now` is each file's count as
    /// the table has it.
    fn notice_lost_results(&mut self, now: &HashMap<i64, usize>) {
        for (file, &before) in &self.results_of_file {
            if now.get(file).copied().unwrap_or(0) >= before {
                continue;
            }
            let class = self
                .digest_of_file
                .get(file)
                .and_then(|digest| self.classes.get(digest));
            if let Some(class) = class {
                self.due.insert(class.entry);
            }
        }
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
#[derive(Default)]
pub struct Matcher {
    state: Mutex<State>,
    on_report: Option<ReportHook>,
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

    /// Calls `hook` with each progress report's number, as a job reports.
    #[cfg(test)]
    pub(crate) fn on_report(mut self, hook: impl Fn(u64) + Send + Sync + 'static) -> Self {
        self.on_report = Some(Box::new(hook));
        self
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
        // The new fingerprints' keys are merged in and the ones that went
        // leave for good, so finding candidates is quick and nothing spare
        // is held.
        state.index.compact();
        summary.files = state.digest_of_file.len() as u64;
        summary.fingerprints = state.classes.len() as u64;

        // 2. Results the database dropped since the last pass are due
        // again, whatever this matcher remembers.
        let mut compared = writer.call(|c| store::compared_pairs(c))?;
        state.notice_lost_results(&results_per_file(&compared));

        // 3. Each due class: its identical files, then its candidates.
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
        state.results_of_file = results_per_file(&compared);
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
            Err(Stop::Job(e)) => Err(e),
        }
    }
}

/// One pass with a fresh index: everything is read from the file table,
/// and only pairs with no stored result are compared.
pub fn refresh(writer: &Writer) -> Result<Summary, DbError> {
    Matcher::new().pass(writer, &mut |_| Ok(()))
}
