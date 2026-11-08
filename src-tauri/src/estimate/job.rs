//! The analyze job: decodes a window of each Library track's file and keeps
//! the app's own estimates (ROADMAP 3.7; 3C-E1a).
//!
//! It works like the waveform job ([`crate::waveform`]), and beside it:
//!
//! - **One file at a time, on one thread of the fingerprint jobs' budget**
//!   ([`FirstUp`]), at below-normal thread priority, at the waveform job's
//!   [`PRIORITY`], below every other job. Between two files it looks at the
//!   job table: if a job of a higher priority is queued it stops and queues
//!   the rest as one more run behind it; if one is running it gives its
//!   thread back and waits until nothing above it runs.
//! - **Resumable.** Every file's result is its own transaction, and the job
//!   works from what's still due ([`ledger`]), so a run cut short (closed,
//!   cancelled, made way) is picked up where it stopped and keeps every
//!   whole result. A cancel stops mid-file and writes nothing for that file.
//! - Per file: if another file holds the estimate of the very same audio,
//!   it is copied and nothing is opened. Otherwise the same checks as the
//!   waveform job (still due, its volume plugged in, the OneDrive gate,
//!   unchanged on disk), then [`measure`], then the result is stored with
//!   the track's `'local'` row, unless the file changed meanwhile. A file
//!   whose content can't be decoded gets a row saying why; a decoder panic
//!   counts against that file alone; a file that couldn't be read gets
//!   nothing recorded and is tried again next time.
//! - **When the run is over** every Library track's `'local'` row is
//!   brought in step with the ledger ([`super::sync_all`], a page of tracks
//!   per writer call): a track whose file changed by a relink or a merge, or
//!   a change of BPM range, is shown right without a decode. Then the energy
//!   suggestions are worked out ([`super::suggest`]: in the ledger only,
//!   never an energy on a track), and the search index is asked for, since
//!   the track table reads that.

use std::collections::HashMap;
use std::fs::File;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use super::ledger::{self, Row};
use super::window::measure;
use crate::fingerprint::decode::Stopped;
use crate::fingerprint::{default_threads, lower_priority, FirstUp};
use crate::jobs::{JobContext, JobError, JobHandler, JobKind, NewJob, Priority};
use crate::paths::{RelPath, Volumes};
use crate::quality::{unchanged, Failure};
use crate::scan::folders::{self, StoredFolder};
use crate::scan::{MusicFolderId, ReadGate};
use crate::settings::{read_bpm_range, BpmRange};

/// The job's priority: the waveform job's, the lowest there is, so
/// estimates are made when nothing else is waiting. Neither of the two
/// makes way for the other.
pub const PRIORITY: Priority = crate::waveform::PRIORITY;

/// An analyze job over every file that's due.
pub fn analyze_job() -> NewJob {
    NewJob::new(JobKind::Analyze).priority(PRIORITY)
}

/// How often a job waiting for a free thread checks again.
const THREAD_POLL: Duration = Duration::from_millis(100);

/// How often a job paused behind a running job of a higher priority checks
/// whether it has ended.
const PAUSE_POLL: Duration = Duration::from_millis(500);

/// Jobs of a higher priority than this one's, by where they are.
struct Above {
    queued: bool,
    running: bool,
}

fn jobs_above(conn: &rusqlite::Connection) -> rusqlite::Result<Above> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM job WHERE status = 'queued' AND priority > ?1),
                EXISTS (SELECT 1 FROM job WHERE status = 'running' AND priority > ?1)",
    )?
    .query_row([PRIORITY.0], |r| {
        Ok(Above {
            queued: r.get(0)?,
            running: r.get(1)?,
        })
    })
}

/// A test hook, called with each file id as the job takes it.
type FileHook = Box<dyn Fn(i64) + Send + Sync>;

/// A test hook, called each time the job waits behind a running job.
type PauseHook = Box<dyn Fn() + Send + Sync>;

/// The analyze job's handler.
pub struct Analyzer<V> {
    /// The volumes mounted now, asked once per job.
    volumes: Box<dyn Fn() -> V + Send + Sync>,
    first: FirstUp,
    budget: usize,
    on_file: Option<FileHook>,
    on_decode: Option<FileHook>,
    on_pause: Option<PauseHook>,
}

impl<V: Volumes + Sync + 'static> Analyzer<V> {
    /// A handler on `first`'s thread budget, which the fingerprint jobs
    /// share.
    pub fn new(volumes: impl Fn() -> V + Send + Sync + 'static, first: FirstUp) -> Analyzer<V> {
        Analyzer {
            volumes: Box::new(volumes),
            first,
            budget: default_threads(),
            on_file: None,
            on_decode: None,
            on_pause: None,
        }
    }

    /// Calls `hook` with each file id as the job takes it.
    #[cfg(test)]
    pub(crate) fn on_file(mut self, hook: impl Fn(i64) + Send + Sync + 'static) -> Self {
        self.on_file = Some(Box::new(hook));
        self
    }

    /// Calls `hook` each time the job waits behind a running job of a
    /// higher priority, after it has given its thread back.
    #[cfg(test)]
    pub(crate) fn on_pause(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_pause = Some(Box::new(hook));
        self
    }

    /// How many threads the budget holds. At least 1.
    #[cfg(test)]
    pub(crate) fn budget(mut self, budget: usize) -> Self {
        self.budget = budget.max(1);
        self
    }

    /// Calls `hook` with each file id as the job starts decoding it.
    #[cfg(test)]
    pub(crate) fn on_decode(mut self, hook: impl Fn(i64) + Send + Sync + 'static) -> Self {
        self.on_decode = Some(Box::new(hook));
        self
    }
}

/// What a job knows while it runs.
struct Run<'a, V> {
    job: &'a JobContext,
    gate: ReadGate,
    volumes: V,
    folders: HashMap<MusicFolderId, StoredFolder>,
    ids: Vec<i64>,
    range: BpmRange,
}

/// Why the loop over the files ended.
enum Ended {
    AllDone,
    MadeWay,
}

impl<V: Volumes + Sync + 'static> JobHandler for Analyzer<V> {
    fn run(&self, job: &JobContext) -> Result<(), JobError> {
        let folders: HashMap<MusicFolderId, StoredFolder> = job
            .writer()
            .call(|c| folders::stored(c))?
            .into_iter()
            .map(|f| (f.id, f))
            .collect();
        let volumes = (self.volumes)();
        let reachable: Vec<i64> = folders
            .values()
            .filter(|f| f.is_reachable(&volumes))
            .map(|f| f.id.0)
            .collect();
        let ids = job
            .writer()
            .call(move |c| ledger::due_ids(c, Some(&reachable)))?;
        let gate = job.writer().call(|c| ReadGate::for_job(c))?;
        let range = job.writer().call(|c| read_bpm_range(c))?;
        let run = Run {
            job,
            gate,
            volumes,
            folders,
            ids,
            range,
        };
        job.progress(0.0)?;
        let ended = if run.ids.is_empty() {
            Ended::AllDone
        } else {
            // A thread of its own, so lowering its priority ends with the job.
            thread::scope(|s| {
                s.spawn(|| {
                    panic::catch_unwind(AssertUnwindSafe(|| self.work(&run)))
                        .unwrap_or_else(|_| Err(JobError::failed("the analyze thread panicked")))
                })
                .join()
                .unwrap_or_else(|_| Err(JobError::failed("the analyze thread panicked")))
            })?
        };
        match ended {
            Ended::MadeWay => {
                crate::scan::chain::queue_once(job.writer(), analyze_job(), |j| job.enqueue(j))?;
                Ok(())
            }
            Ended::AllDone => {
                // Any track's row that no measurement of this run reached
                // (a relink, a merge, a new range): a page per writer call.
                super::sync_all(job.writer(), range)?;
                // The energy suggestions, in the ledger only (the owner's
                // rule): the first batch, one after the Library changed by
                // more than 5%, or the newcomers' ([`super::suggest`]).
                super::suggest::refresh(job.writer())?;
                // The track table reads the index, which the 'local' rows
                // (this run's, and the ones just put right) reach only when
                // it is built again. It is asked for only if some track is
                // due, so a run that changed nothing queues nothing.
                if let Err(e) = crate::search::request(job.writer(), |j| job.enqueue(j)) {
                    eprintln!(
                        "estimate job {}: couldn't ask for the index: {e:?}",
                        job.id()
                    );
                }
                // New files are measured by now: the audio model (when its
                // switch is on) listens to them next.
                if let Err(e) = crate::audio_model::request(job.writer(), None, |j| job.enqueue(j))
                {
                    eprintln!(
                        "estimate job {}: couldn't ask for the audio model: {e:?}",
                        job.id()
                    );
                }
                job.progress(1.0)
            }
        }
    }
}

impl<V: Volumes + Sync + 'static> Analyzer<V> {
    /// The thread: takes the files one at a time.
    fn work(&self, run: &Run<'_, V>) -> Result<Ended, JobError> {
        lower_priority();
        // The thread of the budget, held only while the job is measuring.
        let mut lease = None;
        for (done, &id) in run.ids.iter().enumerate() {
            loop {
                run.job.check_cancelled()?;
                let above = run.job.writer().call(|c| jobs_above(c))?;
                if above.queued {
                    return Ok(Ended::MadeWay);
                }
                if above.running {
                    lease = None;
                    if let Some(hook) = &self.on_pause {
                        hook();
                    }
                    thread::sleep(PAUSE_POLL);
                    continue;
                }
                if lease.is_none() {
                    lease = self.first.try_threads(1, self.budget);
                    if lease.is_none() {
                        thread::sleep(THREAD_POLL);
                        continue;
                    }
                }
                break;
            }
            if let Some(hook) = &self.on_file {
                hook(id);
            }
            self.one(run, id)?;
            run.job.progress((done + 1) as f64 / run.ids.len() as f64)?;
        }
        Ok(Ended::AllDone)
    }

    /// Measures file `id`, if it's still due, and records how it went.
    fn one(&self, run: &Run<'_, V>, id: i64) -> Result<(), JobError> {
        let job = run.job;
        let range = run.range;
        let Some(due) = job.writer().call(move |c| ledger::due(c, id))? else {
            return Ok(());
        };
        let same = due.clone();
        if job.writer().call(move |c| ledger::reuse(c, &same, range))? {
            return Ok(());
        }
        let Some(path) = run.path(&due.folder, &due.rel_path) else {
            return Ok(());
        };
        // OneDrive (ROADMAP 1.1): an online-only file is never opened unless
        // the user opted in, since reading it downloads it.
        if !run.gate.allows(due.online_only) {
            return Ok(());
        }
        match run.gate.may_open(&path) {
            Ok(true) => {}
            Ok(false) | Err(_) => return Ok(()),
        }
        let Ok(file) = File::open(&path) else {
            return Ok(());
        };
        if !unchanged(&file, &due) {
            return Ok(());
        }
        let Ok(after) = file.try_clone() else {
            return Ok(());
        };
        if let Some(hook) = &self.on_decode {
            hook(id);
        }
        let mut keep_going = |_: Option<f64>| !job.is_cancelled();
        let measured = panic::catch_unwind(AssertUnwindSafe(|| {
            measure(Box::new(file), &mut keep_going)
        }));
        let row = match measured {
            Ok(Ok(m)) => Row::Measured(m),
            Ok(Err(Stopped::Cancelled)) => return Err(JobError::Cancelled),
            Ok(Err(Stopped::Failed(why))) => match Failure::of(why) {
                Some(failure) => Row::Failed(failure),
                None => return Ok(()),
            },
            Err(_) => Row::Failed(Failure::DecoderCrashed),
        };
        // Rewritten while it was decoding: leave it for the next walk.
        if !unchanged(&after, &due) {
            return Ok(());
        }
        job.writer()
            .call(move |c| ledger::record(c, &due, &row, range))?;
        Ok(())
    }
}

impl<V: Volumes> Run<'_, V> {
    /// Where the file is now, as a `\\?\` path; `None` if its volume is
    /// offline or its row can't be placed.
    fn path(&self, folder: &MusicFolderId, rel_path: &str) -> Option<PathBuf> {
        let folder = self.folders.get(folder)?;
        let rel = RelPath::parse(rel_path).ok()?;
        folder.stored_path().join(&rel).resolve(&self.volumes).ok()
    }
}
