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
//! - [`plan`] and [`regroup`] are the **grouping pass**: from the audio
//!   the files hold, the stored answers and what the user decided, they
//!   put files into tracks, keep the job's version links, and say where
//!   each stored pair stands.
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
//!   them is the user's action ([`crate::review::same`], 1bE-9). So no
//!   Library track, crate entry or sync base is ever re-pointed here. Where
//!   the user kept every two of a set's Library tracks both, a copy outside
//!   the Library in that set joins neither and its pairs are `decided`.
//! - **A file a Library track points at never moves.** A file outside the
//!   Library may join a Library track as one more of its files.
//! - **Linked versions never merge**, and a file never moves between two of
//!   them ([`store::relocate`] checks it itself; the schema's triggers only
//!   see an update or a replace).
//! - **A decision sticks** (`grouping_decision`, written by [`remember`]):
//!   "apart" keeps two tracks apart and the pair is no longer open; "same"
//!   makes their pairs duplicates, and takes away a version link the job
//!   made between the two. A version link the user made or confirmed
//!   settles its two tracks the same way. A decision is kept by the audio
//!   the two tracks held when it was made (every pair of audio between
//!   them), not by file ids, so it survives a retag, a move, a music folder
//!   removed and added again, and the loss of the file it was made on. (A
//!   file with no `audio_hash` is named by its `blake3`: a decision resting
//!   on such a file alone doesn't survive a tag rewrite of it.)
//! - **A merge is never undone by the job.** A new file or a retag never
//!   splits a track; a pair inside a track that the rules no longer call a
//!   duplicate is stored as such (`in_one_track`), and the track stays
//!   whole. Only a file that holds other audio than it joined with leaves.
//!   So "apart" for two files that already share a track changes nothing.
//! - **Nothing is decided on a fingerprint of audio a file no longer
//!   holds** ([`decide`]): a changed file waits for the fingerprint stage,
//!   as a due file and as any other file's partner.
//!
//! One merge needs more than the audio: **a cut-off copy** (quality verdict
//! `truncated`) inside the whole, same-named file is a worse duplicate, not
//! a cut ([`rules`], "A cut-off copy"; [`decide`], "The quality verdict").
//! Everything above holds for it as for any duplicate.
//!
//! # The same again, and a fresh database
//!
//! A run over a database nothing changed in changes nothing, a remembered
//! decision or a version link of the user's included. A new file joins
//! without moving the others, and a file whose fingerprint changed has only
//! its own pairs decided again.
//!
//! Because merges are kept, a set whose pairs disagree (A and B duplicates,
//! B and C duplicates, A and C not) can come out differently depending on
//! the order its files were decided in. Where A's and B's pairs with C are
//! stored by the time a grouping pass reads them, none of the three merge.
//! Where A and B merged before anything was stored about C (C arrived
//! later, or wasn't ready when they were decided: its fingerprint, or
//! matching, came after), A and B stay one track and C stays apart. A fresh
//! database built from the same files may so differ from an old one in such
//! sets, and only there.
//!
//! # When it runs
//!
//! - The **quick job** ([`group_job`]), after every hash stage and after a
//!   walk that found files nothing will read: one grouping pass, so every
//!   file has a track early and exact copies are together before the long
//!   fingerprint run. If its pass finds a set waiting for a pair nobody
//!   decided, it asks for a deciding job.
//! - The **deciding job** ([`decide_job`]), once matching has nothing left
//!   to compare ([`request_decide`]), and when a quality job ends that
//!   found a file cut short ([`request_decide_after_quality`]): a grouping
//!   pass, the quality verdicts brought up to date if a file is due, the
//!   deciding run, a
//!   grouping pass over what it stored, and then the pairs that pass found
//!   waiting sets to miss, with a pass after them
//!   ([`decide_and_group`]). It runs below everything else, like matching,
//!   and makes way for any job of a higher priority between two pages; what
//!   it wrote stays, and the run queued after it goes on from there.
//! - The **roles job** (a third kind of work of the same job kind, see
//!   [`GroupTarget`]): ranks the files of the tracks that changed and sets
//!   which is `best` ([`crate::file_roles`]). It never moves a file.
//!
//! All three only touch the database. No audio file is read or written.
//! Removing a music folder lets go of its files first
//! ([`release_folder_files`]), or their track rows would refuse it.

pub mod decide;
pub mod plan;
pub mod rules;
mod store;
mod target;

pub use decide::{Decided, Names, MAX_TITLE_BUCKET, PAGE};
pub use plan::Verdict;
pub use store::{
    any_ungrouped, regroup, regroup_step, release_folder_files, remember, CantRemember, Refused,
    Step, Summary, BATCH, MAX_SET_FILES,
};
pub use target::GroupTarget;

pub(crate) use store::sync_rekordbox;

pub(crate) use store::CONTENT_KEY;

use std::sync::{Mutex, PoisonError};

use tauri::State;

use crate::db::{DbError, Writer};
use crate::ipc::IpcError;
use crate::jobs::{JobContext, JobError, JobHandler, JobId, JobKind, JobQueue, NewJob, Priority};

/// The version of what the deciding run stores. Bump it when the rules
/// ([`rules::decide`]), the pairs asked about or what is stored for one
/// change: every file is then due again, and its pairs are decided afresh.
///
/// 2: 1bD-4 (dotted spellings, label-word artists, the folder a file sits
/// in, harmless brackets, a file name whose second part is only a label).
///
/// 3: 1bC-6 (a cut-off copy inside the whole, same-named file is a
/// duplicate, not a cut; each file's record keeps the quality verdict it
/// was decided at).
pub const RULES_VERSION: u32 = 3;

/// What a file's record says it was decided under
/// (`grouping_file.decided_as`): the rules' version and the comparison's.
/// A record with any other value is due.
pub fn decided_as() -> i64 {
    (i64::from(RULES_VERSION) << 16) | i64::from(crate::matching::VERSION)
}

/// The deciding job's priority: matching's, below the scan chain's
/// background work and everything the user asks for.
pub const DECIDE_PRIORITY: Priority = crate::matching::PRIORITY;

/// The quick grouping job, at background priority: nobody is waiting on it.
pub fn group_job() -> NewJob {
    NewJob::new(JobKind::Group).priority(Priority::BACKGROUND)
}

/// The deciding job: a grouping pass, the deciding run, and a grouping
/// pass over what it stored.
pub fn decide_job() -> NewJob {
    NewJob::new(JobKind::Group)
        .target(GroupTarget::Decide.to_json().unwrap_or_default())
        .priority(DECIDE_PRIORITY)
}

/// Whether a grouping job's target makes it a deciding job.
pub fn is_decide(target: Option<&serde_json::Value>) -> bool {
    GroupTarget::of(target) == GroupTarget::Decide
}

/// Queues a deciding job if a file is due one and ready
/// ([`decide::any_due`]) and matching has nothing left to compare (a file
/// matching hasn't been through can't be decided yet), unless one is
/// already queued (a running one is asked to run once more). Called by the
/// scan chain wherever it asks for matching, and when a matching job ends.
pub(crate) fn request_decide<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    let due = writer.call(|c| decide_due(c))?;
    if due {
        crate::scan::chain::queue_once(writer, decide_job(), enqueue)?;
    }
    Ok(())
}

/// Whether [`request_decide`] would queue a deciding job: a file is due one
/// and ready, and matching has nothing left to compare. For a caller that
/// asks it on a read connection.
pub(crate) fn decide_due(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    Ok(!crate::matching::any_due(conn)? && decide::any_due(conn)?)
}

/// The files that are, as grouping stands now, duplicates of `file` in its
/// track, by id. What relink asks before it trusts a track for a file that
/// is gone (step 4, 1bC-7).
///
/// Sharing a track isn't enough for that. A merge is never undone, so two
/// files can share one while the rules call them a cut (`in_one_track`),
/// or without ever having been compared (a file that joined through a
/// third one while the other was away). So a file is returned only on the
/// stored answer for the two files themselves, and only while that answer
/// is current:
///
/// - a pair of the two is stored as a duplicate, and a grouping pass found
///   them in one track (`merged`); and
/// - grouping is **current** for both: every pair of the file is decided
///   under these rules ([`decided_as`]), and it sits in its track with the
///   audio it holds (`placed_key` is its `audio_hash`, or its `blake3` when
///   it has none). A file that is due (not ready counts as due), that holds
///   other audio than it was placed with, or that has no record at all,
///   isn't.
///
/// A file that isn't on disk is asked about like any other: its record is
/// what was known when it went. Its pair goes when the other file is
/// decided again without it, so after that nothing is returned for the two.
///
/// What grouping can't see is the caller's to check: a file that changed on
/// disk since it was hashed still has its old `audio_hash` here.
pub fn current_duplicates(conn: &rusqlite::Connection, file: i64) -> rusqlite::Result<Vec<i64>> {
    // A file is current when its record says so: `f` and `g` are its rows.
    let current = format!(
        "g.decided_as IS ?2 AND g.placed_key IS NOT NULL AND g.placed_key IS {}",
        store::CONTENT_KEY
    );
    let mut stmt = conn.prepare_cached(&format!(
        "WITH mate (id) AS (
             SELECT file_b FROM grouping_pair
             WHERE file_a = ?1 AND outcome = 'duplicate' AND state = 'merged'
             UNION ALL
             SELECT file_a FROM grouping_pair
             WHERE file_b = ?1 AND outcome = 'duplicate' AND state = 'merged')
         SELECT f.id
         FROM file f JOIN grouping_file g ON g.file_id = f.id
                     JOIN recording_file rf ON rf.file_id = f.id
         WHERE f.id IN (SELECT id FROM mate)
           AND {current}
           AND rf.recording_id = (SELECT recording_id FROM recording_file WHERE file_id = ?1)
           -- Nothing is said about a file that isn't current itself.
           AND EXISTS (SELECT 1 FROM file f JOIN grouping_file g ON g.file_id = f.id
                       WHERE f.id = ?1 AND {current})
         ORDER BY f.id"
    ))?;
    let mates = stmt.query_map((file, decided_as()), |r| r.get(0))?;
    mates.collect()
}

/// What follows a quality job: a file whose verdict now says "truncated"
/// where the verdict it was decided at didn't (or the other way round) is
/// made due again ([`decide::due_again_where_truncation_changed`]), and
/// then a deciding job is asked for as everywhere else
/// ([`request_decide`]). A file made due here stays due, so if matching
/// still has work, the request that follows it finds the file.
///
/// The quality job has just brought its verdicts up to date, so nothing is
/// settled here but the few files looked at closely (an out-of-date
/// verdict never counts as a changed one).
pub(crate) fn request_decide_after_quality<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<JobId, E>,
) -> Result<(), E> {
    writer.call(decide::due_again_where_truncation_changed)?;
    request_decide(writer, enqueue)
}

/// Before a deciding run lists what is due: the quality verdicts are
/// brought up to date, a page a transaction ([`crate::quality::settle`];
/// `between` may stop it between two), and every file that is truncated
/// now and wasn't when it was decided, or the other way round, is made due
/// again.
///
/// Every verdict is only brought up to date when a file is due: that reads
/// every file once, to find the files that turned truncated while nobody
/// looked, and a job with nothing to decide has no use for it. Whether or
/// not it ran, no file is made due, and no pair decided, on a verdict that
/// is out of date: the files looked at for a changed verdict, and the
/// files of every page, are settled where they are read ([`decide`], "The
/// quality verdict").
fn catch_up_with_verdicts<E: From<DbError>>(
    writer: &Writer,
    between: &mut dyn FnMut() -> Result<(), E>,
) -> Result<(), E> {
    if writer.call(|c| decide::any_recorded_due(c))? {
        let mut after = 0;
        loop {
            between()?;
            let page = writer.call(move |c| crate::quality::store::settle_page(c, None, after))?;
            match page.last {
                Some(last) => after = last,
                None => break,
            }
        }
    }
    writer.call(decide::due_again_where_truncation_changed)?;
    Ok(())
}

/// A grouping pass, one transaction a step ([`store::step`]). `between` is
/// called before every step and may stop the pass there: every step stands
/// by itself. Returns what the pass did, and the pairs its waiting sets
/// still miss ([`Step::undecided`]).
pub fn pass<E: From<DbError>>(
    writer: &Writer,
    between: &mut dyn FnMut() -> Result<(), E>,
) -> Result<(Summary, Vec<(i64, i64)>), E> {
    let mut summary = Summary::default();
    loop {
        between()?;
        let step = writer.call(|c| store::step(c, BATCH))?;
        summary.add(step.summary);
        if !step.more {
            return Ok((summary, step.undecided));
        }
    }
}

/// How many times in a row the deciding job decides the pairs a pass found
/// missing and passes again. One round is the rule; a merge it allows can
/// show another waiting set its missing pairs. Whatever is left after
/// these is found by the next job's pass.
const COMPLETING_ROUNDS: usize = 8;

/// Everything a deciding job does, in order: a grouping pass; the quality
/// verdicts, brought up to date if a file is due, and the files made due
/// that are truncated now and weren't when they were decided
/// ([`decide`], "The quality verdict"); the deciding
/// run over every file that is due and ready ([`decide::run`]); a pass over
/// what it stored; then, while a pass lists pairs that waiting sets miss,
/// those pairs ([`decide::complete`]) and one more pass.
///
/// `done` is added to as the work goes, so it says what was done even when
/// `tick`, `stop_here` (between two pages) or `between` (between two steps
/// of a pass) stops it early.
pub fn decide_and_group<E: From<DbError>>(
    writer: &Writer,
    names: &mut Names,
    page: usize,
    done: &mut (Decided, Summary),
    tick: &mut dyn FnMut(f64) -> Result<(), E>,
    stop_here: &mut dyn FnMut() -> Result<(), E>,
    between: &mut dyn FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let (summary, mut undecided) = pass(writer, between)?;
    done.1.add(summary);
    catch_up_with_verdicts(writer, between)?;
    done.0 = decide::run(writer, names, page, tick, stop_here)?;
    if done.0.wrote() {
        let (summary, missing) = pass(writer, between)?;
        done.1.add(summary);
        undecided = missing;
    }
    for _ in 0..COMPLETING_ROUNDS {
        if undecided.is_empty() {
            break;
        }
        let before = done.0.completed;
        decide::complete(writer, &undecided, page, &mut done.0, tick, stop_here)?;
        if done.0.completed == before {
            break;
        }
        let (summary, missing) = pass(writer, between)?;
        done.1.add(summary);
        undecided = missing;
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

    /// The deciding job's work, making way for jobs of a higher priority
    /// between two pages and stopping between two steps of a pass if the
    /// job is cancelled.
    fn decide(&self, job: &JobContext, done: &mut (Decided, Summary)) -> Result<(), Stop> {
        // A run that panicked left an index that may be half updated: it's
        // only what candidates are found by, and the next run brings it up
        // to date with the file table.
        let mut names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
        decide_and_group(
            job.writer(),
            &mut names,
            self.page,
            done,
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
            &mut || -> Result<(), Stop> { Ok(job.check_cancelled()?) },
        )
    }
}

impl JobHandler for Grouper {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        // The roles are a Group job's third kind of work ([`GroupTarget`]).
        if GroupTarget::of(job.target()) == GroupTarget::Roles {
            return crate::file_roles::run(job);
        }
        if !is_decide(job.target()) {
            let (summary, _) = pass(job.writer(), &mut || job.check_cancelled())?;
            eprintln!("group job {} done: {summary:?}", job.id());
            // The hook first, so a test hears the summary before any job
            // this run asks for can start.
            if let Some(hook) = &self.on_summary {
                hook(summary);
            }
            // A set waits for a pair nobody decided, and no file is due
            // that would bring a deciding job by itself (a deciding job was
            // cut short, or the user called two tracks the same).
            if summary.undecided_pairs > 0 {
                let asked =
                    crate::scan::chain::queue_once(job.writer(), decide_job(), |j| job.enqueue(j));
                if let Err(e) = asked {
                    eprintln!(
                        "group job {}: couldn't ask for a deciding job: {e:?}",
                        job.id()
                    );
                }
            }
            ask_for_attach(job.id(), job.writer(), |j| job.enqueue(j));
            ask_for_relink(job.id(), job.writer(), |j| job.enqueue(j));
            return job.progress(1.0);
        }
        let mut done = (Decided::default(), Summary::default());
        let finished = match self.decide(job, &mut done) {
            Ok(()) => true,
            // What was written stays, and so does every file that wasn't
            // reached: one more run, at this job's priority, goes on from
            // there. This job is still running, so it's asked to run once
            // more, and its chain wrapper queues that as it ends.
            Err(Stop::MakeWay) => {
                crate::scan::chain::queue_once(job.writer(), decide_job(), |j| job.enqueue(j))?;
                false
            }
            Err(Stop::Job(e)) => return Err(e),
        };
        let (decided, summary) = done;
        let decided = finished.then_some(decided);
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
pub(crate) mod tests;
