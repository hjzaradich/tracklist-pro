//! Grouping: which files are one track, which tracks are versions of each
//! other, and which pairs the user has to decide (1bC-1; ROADMAP 1.4, 1.5,
//! §2).
//!
//! Every present file belongs to exactly one track (`recording`). Three
//! parts do the work:
//!
//! - [`rules`] says what two files are to each other (duplicate, cut,
//!   rework, different, or needs review), from their audio and their names.
//!   A pure library (1bB).
//! - [`decide`] is the **deciding run**: it finds the pairs worth asking
//!   about, compares directly the ones the matching pass never proposed and
//!   whose names call for it, asks the rules, and stores the answers
//!   (`grouping_pair`).
//! - [`plan`] and [`regroup`] are the **grouping pass**: from the files'
//!   audio hashes, the stored answers and what the user decided, they put
//!   files into tracks, keep the job's version links, and say where each
//!   stored pair stands.
//!
//! # What happens to a pair
//!
//! - **Duplicate:** the two files end up in one track, if every other pair
//!   of their set agrees ([`plan`]). Files with the very same `audio_hash`
//!   need no stored answer for that.
//! - **Cut or rework:** the two tracks get a `version_link`. Never merged.
//! - **Review:** the pair stays apart and is stored with its reasons, for
//!   the Review screen (1bD, 1bE).
//! - **Different:** nothing.
//!
//! # What the job never does
//!
//! - **Two tracks that are both in the Library are never merged**, whatever
//!   the rules or the audio say (owner decision, ROADMAP 1.4). They stay
//!   two tracks and the pair is stored as held (`both_in_library`); merging
//!   them is the user's action (1bE-9). So no Library track, crate entry or
//!   sync base is ever re-pointed here.
//! - **A file a Library track points at never moves.** A file outside the
//!   Library may join a Library track as one more of its files.
//! - **Linked versions never merge**, and a file never moves between two of
//!   them ([`store::relocate`] checks it itself; the schema's triggers only
//!   see an update or a replace).
//! - **A decision sticks** (`grouping_decision`, written by [`remember`]):
//!   "apart" keeps two tracks apart and the pair is no longer open; "same"
//!   makes their pairs duplicates. A version link the user made or
//!   confirmed settles its two tracks the same way. Decisions name what the
//!   files hold, not file ids, so they survive a retag, a move, and a music
//!   folder removed and added again.
//! - **A merge is never undone by the job.** A new file or a retag never
//!   splits a track; a pair inside a track that the rules no longer call a
//!   duplicate is stored as such (`in_one_track`), and the track stays
//!   whole.
//!
//! # The same again, and a fresh database
//!
//! A run over a database nothing changed in changes nothing. A new file
//! joins without moving the others, and a file whose fingerprint changed
//! has only its own pairs decided again.
//!
//! Because merges are kept, a set whose pairs disagree (A and B duplicates,
//! B and C duplicates, A and C not) can come out differently depending on
//! the order its files arrived in: scanned in one go, none of the three
//! merge; with C arriving after A and B merged, A and B stay one track and
//! C stays apart. A fresh database built from the same files may so differ
//! from an old one in such sets, and only there.
//!
//! # When it runs
//!
//! - The **quick job** ([`group_job`]), after every hash stage and after a
//!   walk that found files nothing will read: one grouping pass, so every
//!   file has a track early and exact copies are together before the long
//!   fingerprint run.
//! - The **deciding job** ([`decide_job`]), once matching has nothing left
//!   to compare ([`request_decide`]): a grouping pass, the deciding run,
//!   and a grouping pass over what it stored. It runs below everything
//!   else, like matching, and makes way for any job of a higher priority
//!   between two pages; what it wrote stays, and the run queued after it
//!   goes on from there.
//!
//! Both only touch the database. No audio file is read or written.
//! Removing a music folder lets go of its files first
//! ([`release_folder_files`]), or their track rows would refuse it.

pub mod decide;
pub mod plan;
pub mod rules;
mod store;

pub use decide::{Decided, Names, MAX_TITLE_BUCKET, PAGE};
pub use plan::Verdict;
pub use store::{
    any_ungrouped, regroup, regroup_step, release_folder_files, remember, CantRemember, Refused,
    Summary, BATCH, MAX_SET_FILES,
};

use std::sync::{Mutex, PoisonError};

use tauri::State;

use crate::db::{DbError, Writer};
use crate::ipc::IpcError;
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, JobQueue, NewJob, Priority};

/// The version of what the deciding run stores. Bump it when the rules
/// ([`rules::decide`]), the pairs asked about or what is stored for one
/// change: every file is then due again, and its pairs are decided afresh.
pub const RULES_VERSION: u32 = 1;

/// What a file's record says it was decided under
/// (`grouping_file.decided_as`): the rules' version and the comparison's.
/// A record with any other value is due.
pub fn decided_as() -> i64 {
    (i64::from(RULES_VERSION) << 16) | i64::from(crate::matching::VERSION)
}

/// The deciding job's priority: matching's, below the scan chain's
/// background work and everything the user asks for.
pub const DECIDE_PRIORITY: Priority = crate::matching::PRIORITY;

/// The key in a grouping job's target that makes it a deciding job.
const DECIDE_KEY: &str = "decide";

/// The quick grouping job, at background priority: nobody is waiting on it.
pub fn group_job() -> NewJob {
    NewJob::new(JobKind::Group).priority(Priority::BACKGROUND)
}

/// The deciding job: a grouping pass, the deciding run, and a grouping
/// pass over what it stored.
pub fn decide_job() -> NewJob {
    NewJob::new(JobKind::Group)
        .target(serde_json::json!({ DECIDE_KEY: true }))
        .priority(DECIDE_PRIORITY)
}

/// Whether a grouping job's target makes it a deciding job.
pub fn is_decide(target: Option<&serde_json::Value>) -> bool {
    target
        .and_then(|t| t.get(DECIDE_KEY))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Queues a deciding job if a file is due one and matching has nothing left
/// to compare (a file matching hasn't been through can't be decided yet),
/// unless one is already queued (a running one is asked to run once more).
/// Called by the scan chain wherever it asks for matching, and when a
/// matching job ends.
pub(crate) fn request_decide<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    let due = writer.call(|c| Ok(!crate::matching::any_due(c)? && decide::any_due(c)?))?;
    if due {
        crate::scan::chain::queue_once(writer, decide_job(), enqueue)?;
    }
    Ok(())
}

/// Whether a job of a higher priority than the deciding job's is waiting.
fn higher_priority_waiting(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM job WHERE status = 'queued' AND priority > ?1)",
    )?
    .query_row([DECIDE_PRIORITY.0], |r| r.get(0))
}

/// Why a deciding run stopped early.
enum Stop {
    /// Cancelled, or it failed.
    Job(JobError),
    /// A job of a higher priority is waiting for the worker.
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

type SummaryHook = Box<dyn Fn(Summary) + Send + Sync>;
type DecidedHook = Box<dyn Fn(Decided, Summary) + Send + Sync>;
type TickHook = Box<dyn Fn() + Send + Sync>;

/// The grouping jobs' handler: one long-lived instance, so the index of
/// names the deciding run looks candidates up in is kept from one job to
/// the next.
pub struct Grouper {
    on_summary: Option<SummaryHook>,
    on_decided: Option<DecidedHook>,
    on_tick: Option<TickHook>,
    names: Mutex<Names>,
    /// How many files the deciding run decides per transaction.
    page: usize,
}

impl Default for Grouper {
    fn default() -> Grouper {
        Grouper {
            on_summary: None,
            on_decided: None,
            on_tick: None,
            names: Mutex::default(),
            page: PAGE,
        }
    }
}

impl Grouper {
    /// Hands each quick run's [`Summary`] to `hook` as well as to the log.
    #[cfg(test)]
    pub(crate) fn on_summary(mut self, hook: impl Fn(Summary) + Send + Sync + 'static) -> Self {
        self.on_summary = Some(Box::new(hook));
        self
    }

    /// Hands each deciding job's result to `hook`: what the deciding run
    /// did, and what its grouping passes did.
    #[cfg(test)]
    pub(crate) fn on_decided(
        mut self,
        hook: impl Fn(Decided, Summary) + Send + Sync + 'static,
    ) -> Self {
        self.on_decided = Some(Box::new(hook));
        self
    }

    /// Calls `hook` each time the deciding run reports how far it is: at
    /// its start and before every page.
    #[cfg(test)]
    pub(crate) fn on_tick(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_tick = Some(Box::new(hook));
        self
    }

    /// Decides `page` files per transaction.
    #[cfg(test)]
    pub(crate) fn page(mut self, page: usize) -> Self {
        self.page = page.max(1);
        self
    }

    /// A grouping pass, one transaction a step, stopping between steps if
    /// the job is cancelled: every step stands by itself.
    fn pass(&self, job: &JobContext) -> Result<Summary, JobError> {
        let mut summary = Summary::default();
        loop {
            job.check_cancelled()?;
            let (step, more) = job.writer().call(|c| regroup_step(c, BATCH))?;
            summary.add(step);
            if !more {
                return Ok(summary);
            }
        }
    }

    /// The deciding run, making way for jobs of a higher priority.
    fn decide(&self, job: &JobContext) -> Result<Decided, Stop> {
        // A run that panicked left an index that may be half updated: it's
        // only what candidates are found by, and the next run brings it up
        // to date with the file table.
        let mut names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
        decide::run(
            job.writer(),
            &mut names,
            self.page,
            &mut |fraction| -> Result<(), Stop> {
                if let Some(hook) = &self.on_tick {
                    hook();
                }
                Ok(job.progress(fraction)?)
            },
            &mut || -> Result<(), Stop> {
                if job.writer().call(|c| higher_priority_waiting(c))? {
                    return Err(Stop::MakeWay);
                }
                Ok(())
            },
        )
    }
}

impl JobHandler for Grouper {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let mut summary = self.pass(job)?;
        if !is_decide(job.target()) {
            eprintln!("group job {} done: {summary:?}", job.id());
            // The hook first, so a test hears the summary before any job
            // this run asks for can start.
            if let Some(hook) = &self.on_summary {
                hook(summary);
            }
            ask_for_attach(job.id(), job.writer(), |j| job.enqueue(j));
            ask_for_relink(job.id(), job.writer(), |j| job.enqueue(j));
            return job.progress(1.0);
        }
        let decided = match self.decide(job) {
            Ok(decided) => {
                if decided.wrote() {
                    summary.add(self.pass(job)?);
                }
                Some(decided)
            }
            // What was written stays, and so does every file that wasn't
            // reached: one more run, at this job's priority, goes on from
            // there. This job is still running, so it's asked to run once
            // more, and its chain wrapper queues that as it ends.
            Err(Stop::MakeWay) => {
                crate::scan::chain::queue_once(job.writer(), decide_job(), |j| job.enqueue(j))?;
                None
            }
            Err(Stop::Job(e)) => return Err(e),
        };
        eprintln!("deciding job {} done: {decided:?}, {summary:?}", job.id());
        if let (Some(hook), Some(decided)) = (&self.on_decided, decided) {
            hook(decided, summary);
        }
        // Only where tracks changed: what's attached to them is decided on
        // what they hold.
        if summary.changed_tracks() {
            ask_for_attach(job.id(), job.writer(), |j| job.enqueue(j));
            ask_for_relink(job.id(), job.writer(), |j| job.enqueue(j));
        }
        match decided {
            Some(_) => job.progress(1.0),
            None => Ok(()),
        }
    }
}

/// Asks for an attach: tracks may have merged or moved, so the rekordbox
/// data attached to them is decided again (1aD-4). The grouping has
/// committed by now, so a failure to ask is logged and the job goes on
/// (the next grouping run asks again).
fn ask_for_attach(
    id: JobId,
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, JobError>,
) {
    if let Err(e) = crate::attach::request(writer, enqueue) {
        eprintln!("group job {id}: couldn't ask for an attach: {e:?}");
    }
}

/// Asks for a relink: files may have changed track, and a rekordbox
/// track's match is decided on what the files hold and where they are now
/// (1aD-1). Never queued twice while one waits ([`crate::relink::request`]),
/// and a relink never asks for grouping, so the two can't go round in a
/// loop. Like the attach, a failure to ask is logged and the job goes on.
fn ask_for_relink(
    id: JobId,
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, JobError>,
) {
    if let Err(e) = crate::relink::request(writer, enqueue) {
        eprintln!("group job {id}: couldn't ask for a relink: {e:?}");
    }
}

/// Groups every file into tracks, in the background. Returns the job's id;
/// if a grouping job is already queued, returns that one instead of
/// starting another. If one is running (it may have loaded the files
/// already), it's asked to run once more when it ends, like the chained
/// stages are ([`crate::scan::chain::queue_once`]).
#[tauri::command(async)]
#[specta::specta]
pub fn group_files(
    jobs: State<'_, JobQueue>,
    writer: State<'_, Writer>,
) -> Result<JobId, IpcError> {
    Ok(start(&jobs, &writer)?)
}

/// [`group_files`], without Tauri.
pub fn start(jobs: &JobQueue, writer: &Writer) -> Result<JobId, crate::db::DbError> {
    crate::scan::chain::queue_once(writer, group_job(), |j| jobs.enqueue(j))
}

#[cfg(test)]
mod tests;
