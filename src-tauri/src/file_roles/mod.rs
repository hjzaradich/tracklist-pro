//! Which file of a track is its best (ROADMAP 1.7; 1bC-4): the ranking
//! ([`crate::best_file`]) read into `recording_file.role`, kept current, and
//! "a better file is available" for the tracks in the Library.
//!
//! # What the roles are
//!
//! After this step the best-ranked file of a track is `best` and every
//! other file of it is `extra`. `undecided` is only the state of a file that
//! was just placed or moved and has not been ranked yet. A track has one
//! `best` at every moment: the demotion of the old one and the promotion of
//! the new one are in the same transaction, and `ensure_best` (grouping)
//! still gives a track with none a stopgap one at once.
//!
//! The roles are a function of what the database holds, and of nothing
//! else: **not of which drives are plugged in** (every file's volume counts
//! as connected here, [`crate::best_file::Place::volume_online`]), not of
//! the time or of what the roles were. So plugging a drive in or out never
//! moves a best file, and the same database always gives the same roles.
//! Ties end at the lower file id.
//!
//! # Which tracks, and when
//!
//! `best_file_due` (migration 0023) holds the tracks to rank again. Triggers
//! put a track there when something the ranking reads changes: a file placed
//! in it, moved out of it or let go; a file that left, came back, was read
//! again, got other audio or moved folder; a measurement or a stored
//! verdict; rekordbox's match to one of its files; its Library track; its
//! music folder's role or volume. The step clears a track in the same
//! transaction as the roles it writes, so a cancel leaves every track either
//! ranked or still due. Nothing fires on `recording_file.role`, so the step
//! never makes a track due.
//!
//! Bumping [`ROLES_VERSION`] (when the ranking's rules change) makes every
//! track due.
//!
//! The step is the third kind of work of the `Group` job
//! ([`GroupTarget::Roles`]), queued by [`request`] whenever a job of the scan
//! chain, a relink or a read of rekordbox ends and a track is due. It runs
//! below everything else, so a first import is ranked once, after the last
//! stage, and it makes way between batches for any job of a higher priority.
//! Cost: reading and ranking the files of the due tracks, 500 tracks a
//! transaction; an idle run is one `EXISTS` query.
//!
//! # A better file for a Library track
//!
//! The linked file of a Library track is pinned: nothing here changes
//! `library_track.linked_file_id`. When the best-ranked file is another one
//! and is ahead of it in what the audio is, or in being there at all
//! ([`audio_step`]), the track gets a `better_file` row: which file, and the
//! step that put it ahead. A copy that only has more tags, art, a rekordbox
//! link or a better place is not a better file: tags and art are picked per
//! field, apart from the audio. A tie is not better either.

mod facts;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::ops::AddAssign;

use rusqlite::{params, Connection, OptionalExtension};

use crate::best_file::{rank, why_ahead, Step};
use crate::db::{DbError, Writer};
use crate::grouping::GroupTarget;
use crate::jobs::{JobContext, JobError, JobKind, NewJob, Priority};

use facts::Loaded;

/// The version of the ranking's rules as the roles were last made. Bump it
/// when the order, the format lists or the facts read into it change: every
/// track is then ranked again.
pub const ROLES_VERSION: u32 = 1;

/// The setting that remembers which [`ROLES_VERSION`] the roles are of.
const VERSION_KEY: &str = "file_roles_version";

/// How many tracks one transaction ranks.
pub const BATCH: usize = 500;

/// The roles job's priority: below matching's, which is below everything
/// else, so the roles are worked out when nothing else is waiting.
pub const PRIORITY: Priority = Priority(crate::matching::PRIORITY.0 - 10);

/// What a stretch of work did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Tracks ranked.
    pub ranked: u64,
    /// Tracks whose best file is another one than it was.
    pub best_changed: u64,
}

impl AddAssign for Summary {
    fn add_assign(&mut self, other: Summary) {
        self.ranked += other.ranked;
        self.best_changed += other.best_changed;
    }
}

/// The roles job.
pub fn roles_job() -> NewJob {
    NewJob::new(JobKind::Group)
        .target(GroupTarget::Roles.to_json().unwrap_or_default())
        .priority(PRIORITY)
}

/// Whether any track is due.
pub fn any_due(conn: &Connection) -> rusqlite::Result<bool> {
    conn.prepare_cached("SELECT EXISTS (SELECT 1 FROM best_file_due)")?
        .query_row([], |r| r.get(0))
}

/// Makes every track due if the roles are of another [`ROLES_VERSION`]
/// than this build's.
fn ensure_version(conn: &Connection) -> rusqlite::Result<()> {
    let stored: Option<i64> = conn
        .query_row(
            "SELECT json_extract(value, '$') FROM setting WHERE key = ?1",
            [VERSION_KEY],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    if stored == Some(i64::from(ROLES_VERSION)) {
        return Ok(());
    }
    conn.execute(
        "INSERT OR IGNORE INTO best_file_due (recording_id) SELECT id FROM recording",
        [],
    )?;
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![VERSION_KEY, ROLES_VERSION.to_string()],
    )?;
    Ok(())
}

/// Queues the roles job if a track is due, unless one is already queued (a
/// running one is asked to run once more). Called wherever a stage of the
/// scan chain, a relink or a read of rekordbox ends
/// ([`crate::scan::chain`]), and after a track is added to the Library.
pub(crate) fn request<E: From<DbError>>(
    writer: &Writer,
    enqueue: impl FnOnce(NewJob) -> Result<crate::jobs::JobId, E>,
) -> Result<(), E> {
    let due = writer.call(|c| {
        ensure_version(c)?;
        any_due(c)
    })?;
    if due {
        crate::scan::chain::queue_once(writer, roles_job(), enqueue)?;
    }
    Ok(())
}

/// What one transaction did, and whether there is more.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Batch {
    pub summary: Summary,
    pub more: bool,
}

/// Ranks the files of up to `batch` due tracks and writes their roles and
/// their "better file", in one transaction. Settles the stored verdicts of
/// those tracks' files first (a stale verdict reads as none). Safe to run at
/// any time; with nothing due it writes nothing.
pub fn settle_batch(conn: &mut Connection, batch: usize) -> rusqlite::Result<Batch> {
    let due: Vec<i64> = {
        let mut stmt = conn.prepare_cached(
            "SELECT recording_id FROM best_file_due ORDER BY recording_id LIMIT ?1",
        )?;
        let rows = stmt.query_map([batch.max(1) as i64], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<Vec<i64>>>()?
    };
    if due.is_empty() {
        return Ok(Batch::default());
    }
    let wanted = serde_json::to_string(&due).unwrap_or_else(|_| "[]".into());
    let files: Vec<i64> = {
        let mut stmt = conn.prepare_cached(
            "SELECT file_id FROM recording_file
             WHERE recording_id IN (SELECT value FROM json_each(?1))",
        )?;
        let rows = stmt.query_map([&wanted], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<Vec<i64>>>()?
    };
    crate::quality::settle_files(conn, &files)?;

    let tx = conn.transaction()?;
    let mut by_recording = facts::load(&tx, &due)?;
    let linked = linked_files(&tx, &wanted)?;
    let mut summary = Summary::default();
    for recording in &due {
        let files = by_recording.remove(recording).unwrap_or_default();
        summary.ranked += 1;
        if rank_track(&tx, *recording, &files, linked.get(recording).copied())? {
            summary.best_changed += 1;
        }
    }
    tx.execute(
        "DELETE FROM best_file_due WHERE recording_id IN (SELECT value FROM json_each(?1))",
        [&wanted],
    )?;
    tx.commit()?;
    Ok(Batch {
        summary,
        more: any_due(conn)?,
    })
}

/// The linked file of each of the given tracks that is in the Library.
fn linked_files(conn: &Connection, wanted: &str) -> rusqlite::Result<HashMap<i64, i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT recording_id, linked_file_id FROM library_track
         WHERE linked_file_id IS NOT NULL
           AND recording_id IN (SELECT value FROM json_each(?1))",
    )?;
    let rows = stmt.query_map([wanted], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let linked = rows.collect::<rusqlite::Result<HashMap<i64, i64>>>()?;
    Ok(linked)
}

/// Ranks one track's files and writes the roles and the better file.
/// Returns whether its best file is another one than before.
fn rank_track(
    conn: &Connection,
    recording: i64,
    files: &[Loaded],
    linked: Option<i64>,
) -> rusqlite::Result<bool> {
    let facts: Vec<_> = files.iter().map(|l| l.facts.clone()).collect();
    let ranking = rank(&facts);
    let Some(best) = ranking.best() else {
        write_better(conn, recording, None)?;
        return Ok(false);
    };

    // One best at every moment: the old one steps down first.
    let mut demote =
        conn.prepare_cached("UPDATE recording_file SET role = 'extra' WHERE file_id = ?1")?;
    for file in files {
        if file.facts.file_id != best && file.role != "extra" {
            demote.execute([file.facts.file_id])?;
        }
    }
    let mut promote =
        conn.prepare_cached("UPDATE recording_file SET role = 'best' WHERE file_id = ?1")?;
    let best_before = files
        .iter()
        .find(|f| f.role == "best")
        .map(|f| f.facts.file_id);
    if best_before != Some(best) {
        promote.execute([best])?;
    }

    let better = linked.filter(|linked| *linked != best).and_then(|linked| {
        let by_file = facts::by_file(files);
        let (best_facts, linked_facts) = (by_file.get(&best)?, by_file.get(&linked)?);
        let step = audio_step(&why_ahead(best_facts, linked_facts)?)?;
        Some((best, step))
    });
    write_better(conn, recording, better)?;
    Ok(best_before != Some(best))
}

/// The step of the ranking that put a file ahead, if it is about the audio
/// or the file being there: on disk, verdict, plays in the DJ app, cutoff,
/// decode errors, format. Not tags, art, a rekordbox link, a better place or
/// a tie.
pub fn audio_step(reason: &crate::best_file::Reason) -> Option<Step> {
    let step = reason.step()?;
    matches!(
        step,
        Step::OnDisk
            | Step::Verdict
            | Step::Playable
            | Step::Cutoff
            | Step::DecodeErrors
            | Step::Format
    )
    .then_some(step)
}

/// Writes a track's `better_file` row, or takes it away, changing nothing
/// that is already right.
fn write_better(
    conn: &Connection,
    recording: i64,
    better: Option<(i64, Step)>,
) -> rusqlite::Result<()> {
    match better {
        Some((file, step)) => {
            conn.prepare_cached(
                "INSERT INTO better_file (recording_id, file_id, step) VALUES (?1, ?2, ?3)
                 ON CONFLICT (recording_id) DO UPDATE
                 SET file_id = excluded.file_id, step = excluded.step
                 WHERE file_id IS NOT excluded.file_id OR step IS NOT excluded.step",
            )?
            .execute(params![recording, file, step_name(step)])?;
        }
        None => {
            conn.prepare_cached("DELETE FROM better_file WHERE recording_id = ?1")?
                .execute([recording])?;
        }
    }
    Ok(())
}

/// The name a step is stored and sent under.
fn step_name(step: Step) -> &'static str {
    match step {
        Step::OnDisk => "onDisk",
        Step::Verdict => "verdict",
        Step::Playable => "playable",
        Step::Cutoff => "cutoff",
        Step::DecodeErrors => "decodeErrors",
        Step::Format => "format",
        Step::Tags => "tags",
        Step::Art => "art",
        Step::Rekordbox => "rekordbox",
        Step::Place => "place",
        Step::Tied => "tied",
    }
}

/// The step stored under `name`, if it is one a better file can name.
pub(crate) fn step_of(name: &str) -> Option<Step> {
    Some(match name {
        "onDisk" => Step::OnDisk,
        "verdict" => Step::Verdict,
        "playable" => Step::Playable,
        "cutoff" => Step::Cutoff,
        "decodeErrors" => Step::DecodeErrors,
        "format" => Step::Format,
        _ => return None,
    })
}

/// Ranks every due track, a batch a transaction. `between` is called before
/// each batch and may stop the pass there: every batch stands by itself.
pub fn pass<E: From<DbError>>(
    writer: &Writer,
    between: &mut dyn FnMut() -> Result<(), E>,
) -> Result<Summary, E> {
    let mut total = Summary::default();
    loop {
        between()?;
        let step = writer.call(|c| settle_batch(c, BATCH))?;
        total += step.summary;
        if !step.more {
            return Ok(total);
        }
    }
}

/// Whether a job of a higher priority than the roles job's is waiting.
fn higher_priority_waiting(conn: &Connection) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM job WHERE status = 'queued' AND priority > ?1)",
    )?
    .query_row([PRIORITY.0], |r| r.get(0))
}

/// The roles job: ranks every due track. Stops between batches if the job
/// is cancelled, or makes way for a job of a higher priority; the chain
/// queues it again as it ends while a track is still due.
pub fn run(job: &JobContext) -> Result<(), JobError> {
    let mut total = Summary::default();
    let writer = job.writer();
    loop {
        job.check_cancelled()?;
        if writer.call(|c| higher_priority_waiting(c))? {
            break;
        }
        let step = writer.call(|c| {
            ensure_version(c)?;
            settle_batch(c, BATCH)
        })?;
        total += step.summary;
        if !step.more {
            break;
        }
    }
    eprintln!("roles job {} done: {total:?}", job.id());
    // Which rekordbox entry gives a track its BPM and key is decided on its
    // best file: attach again where a best file changed (a no-op without
    // rekordbox data).
    if total.best_changed > 0 {
        if let Err(e) = crate::attach::request(writer, |j| job.enqueue(j)) {
            eprintln!("roles job {}: couldn't ask for an attach: {e:?}", job.id());
        }
    }
    job.progress(1.0)
}
