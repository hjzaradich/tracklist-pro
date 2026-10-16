//! Where grouping meets the database: loads the files and what was decided
//! about them, asks [`super::plan`] where each file goes, and applies the
//! answer. Also the removal side: letting go of a music folder's files.
//!
//! Tracks are `recording` rows, and a file's track is its `recording_file`
//! row (migration 0003). What's written: those two tables;
//! `grouping_file.placed_key` (the audio each file was placed with) and
//! `grouping_pair.state` (where each decided pair stands); the job's own
//! `version_link` rows (never the user's: one they made or confirmed); and
//! `rekordbox_track.recording_id`, which grouping owns: a matched rekordbox
//! track always points at its file's track. A track that loses its files
//! takes its `analysis` rows with it, on purpose: the ones rekordbox gave
//! are dropped (attach, 1aD-4, works them out again for whichever track
//! holds the data now), and every other source's rows move to the track its
//! files went to ([`settle_analysis`]). Never `library_track`, `crate_entry`
//! or `sync_base`: no Library track is ever merged away here.
//!
//! # In steps
//!
//! A first run places every file, so it isn't one transaction: each step
//! ([`regroup_step`]) plans afresh inside its own transaction and applies at
//! most a batch of the plan. Every step leaves tracks that stand by
//! themselves, so a run that's cut short loses nothing and the next one
//! goes on from there.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::ops::{OpsError, Recorder};

use super::plan::{
    plan, Dest, Input, Key, Link, Member, Move, Outcome, Pair, Placed, Plan, Remembered, State,
    Verdict,
};

/// A set of files holding several kinds of audio is checked pair by pair
/// only up to this many files; a bigger one is held as if it disagreed.
pub const MAX_SET_FILES: usize = 50;

/// At most this many files are placed, moved or brought up to date in one
/// transaction.
pub const BATCH: usize = 20_000;

/// What one grouping pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Files that had no track and got one.
    pub placed: u64,
    /// Files that changed track.
    pub moved: u64,
    /// Tracks made.
    pub recordings_made: u64,
    /// Tracks left with no files and nothing else pointing at them, deleted.
    pub recordings_removed: u64,
    /// Files that should have moved but sit between linked versions.
    pub left_alone: u64,
    /// Moves the database's state refused when they were made (a file that
    /// became pinned, or tracks that became linked, since the plan).
    pub refused: u64,
    /// Version links the job made, and its own that it took away.
    pub links_made: u64,
    pub links_removed: u64,
    /// Sets of duplicates held apart: a pair in them isn't one, the user
    /// decided two of their tracks apart, two are linked versions, or they
    /// are too big to check.
    pub sets_disagreeing: u64,
    /// Sets held apart because two of their tracks are in the Library.
    pub sets_both_in_library: u64,
    /// Sets waiting for a pair to be decided.
    pub sets_waiting: u64,
    /// Pairs inside those sets that nobody decided yet, between files that
    /// aren't due: the deciding job decides them
    /// ([`super::decide::complete`]).
    pub undecided_pairs: u64,
}

impl Summary {
    /// Whether tracks changed: something attached to them may need to be
    /// decided again.
    pub fn changed_tracks(&self) -> bool {
        self.placed + self.moved + self.recordings_made + self.recordings_removed > 0
    }

    /// Adds what a later step of the same pass did.
    pub(crate) fn add(&mut self, step: Summary) {
        self.placed += step.placed;
        self.moved += step.moved;
        self.recordings_made += step.recordings_made;
        self.recordings_removed += step.recordings_removed;
        self.refused += step.refused;
        self.links_made += step.links_made;
        self.links_removed += step.links_removed;
        // What the last step saw is what stands.
        self.left_alone = step.left_alone;
        self.sets_disagreeing = step.sets_disagreeing;
        self.sets_both_in_library = step.sets_both_in_library;
        self.sets_waiting = step.sets_waiting;
        self.undecided_pairs = step.undecided_pairs;
    }
}

/// A track is unreferenced when nothing but its files (`recording_file`)
/// points at it. Everything that names a track by id is listed here, so
/// deleting one never trips a foreign key (a test compares this list with
/// the schema's foreign keys).
///
/// A removal record (`library_removal`) also names a track, but sweeping
/// handles it on purpose (see [`sweep`]), so it's not in this part. A
/// track the user merged into another (`track_merge`, Review) has no files
/// while the merge stands and is kept for its Undo.
const UNREFERENCED: &str =
    "NOT EXISTS (SELECT 1 FROM recording_file WHERE recording_id = recording.id)
     AND NOT EXISTS (SELECT 1 FROM library_track WHERE recording_id = recording.id)
     AND NOT EXISTS (SELECT 1 FROM analysis WHERE recording_id = recording.id)
     AND NOT EXISTS (SELECT 1 FROM version_link
                     WHERE recording_a = recording.id OR recording_b = recording.id)
     AND NOT EXISTS (SELECT 1 FROM rekordbox_track WHERE recording_id = recording.id)
     AND NOT EXISTS (SELECT 1 FROM track_merge WHERE recording_id = recording.id)";

/// The audio file `f` holds, as SQL: its `audio_hash`, or its `blake3` when
/// it has none (an empty hash says nothing, like none). The one name for a
/// file's audio wherever it's kept or compared for being the very same:
/// `grouping_file.placed_key`, `grouping_decision`'s keys, the deciding
/// run's "identical", and the trigger that drops a file's pairs (migration
/// 0022 spells the same expression out). Only the rule that files with the
/// same `audio_hash` are one track with no pair decided reads the
/// `audio_hash` alone ([`Member::key`]).
pub(crate) const CONTENT_KEY: &str = "coalesce(nullif(f.audio_hash, x''), f.blake3)";

/// The files and tracks a grouping pass over part of the library looks at
/// ([`regroup_files`]): a set of files closed under what can join them into
/// one set (the files of their tracks, their stored duplicate pairs, the
/// same `audio_hash`), and the tracks of those files. As JSON arrays of
/// ids, the app's own integers, put in the SQL as they are.
pub(crate) struct Scope {
    files: String,
    tracks: String,
}

impl Scope {
    /// `AND {expr} IN (the scope's files)`, or nothing for a whole pass.
    fn files(scope: Option<&Scope>, expr: &str) -> String {
        scope.map_or_else(String::new, |s| {
            format!("AND {expr} IN (SELECT value FROM json_each('{}'))", s.files)
        })
    }

    /// `AND {expr} IN (the scope's tracks)`, or nothing for a whole pass.
    fn tracks(scope: Option<&Scope>, expr: &str) -> String {
        scope.map_or_else(String::new, |s| {
            format!(
                "AND {expr} IN (SELECT value FROM json_each('{}'))",
                s.tracks
            )
        })
    }

    /// The files a pass over `files` must see, and their tracks: grown from
    /// `files` until nothing joins.
    fn of(conn: &Connection, files: &[i64]) -> rusqlite::Result<Scope> {
        let mut found: std::collections::BTreeSet<i64> = files.iter().copied().collect();
        loop {
            let json = serde_json::to_string(&found).expect("numbers always serialize");
            let mut grown = found.clone();
            let mut stmt = conn.prepare_cached(
                "WITH s(id) AS (SELECT value FROM json_each(?1))
                 SELECT rf2.file_id FROM s JOIN recording_file rf ON rf.file_id = s.id
                 JOIN recording_file rf2 ON rf2.recording_id = rf.recording_id
                 UNION SELECT p.file_b FROM s JOIN grouping_pair p ON p.file_a = s.id
                       WHERE p.outcome = 'duplicate'
                 UNION SELECT p.file_a FROM s JOIN grouping_pair p ON p.file_b = s.id
                       WHERE p.outcome = 'duplicate'
                 UNION SELECT f2.id FROM s JOIN file f ON f.id = s.id
                       JOIN file f2 ON f2.audio_hash = f.audio_hash
                       WHERE length(f.audio_hash) > 0",
            )?;
            for id in stmt.query_map([&json], |r| r.get::<_, i64>(0))? {
                grown.insert(id?);
            }
            if grown.len() == found.len() {
                break;
            }
            found = grown;
        }
        let files = serde_json::to_string(&found).expect("numbers always serialize");
        let tracks: Vec<i64> = conn
            .prepare_cached(
                "SELECT DISTINCT recording_id FROM recording_file
                 WHERE file_id IN (SELECT value FROM json_each(?1)) ORDER BY 1",
            )?
            .query_map([&files], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Scope {
            files,
            tracks: serde_json::to_string(&tracks).expect("numbers always serialize"),
        })
    }
}

/// Every file with a track, and every present file without one (of the
/// scope's files, if a scope is given).
fn members(conn: &Connection, scope: Option<&Scope>) -> rusqlite::Result<Vec<Member>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT f.id, rf.recording_id, f.audio_hash, {CONTENT_KEY},
                EXISTS (SELECT 1 FROM library_track lt
                        WHERE lt.linked_file_id = f.id OR lt.source_file_id = f.id),
                g.file_id IS NOT NULL, g.placed_key, g.decided_as IS ?1, f.present
         FROM file f LEFT JOIN recording_file rf ON rf.file_id = f.id
                     LEFT JOIN grouping_file g ON g.file_id = f.id
         WHERE (f.present = 1 OR rf.file_id IS NOT NULL) {}
         ORDER BY f.id",
        Scope::files(scope, "f.id")
    ))?;
    let rows = stmt.query_map([super::decided_as()], |r| {
        let key: Option<Vec<u8>> = r.get(2)?;
        let placed: Option<Vec<u8>> = r.get(6)?;
        Ok(Member {
            file: r.get(0)?,
            recording: r.get(1)?,
            // An empty hash says nothing, like none.
            key: key.filter(|k| !k.is_empty()),
            content: r.get(3)?,
            present: r.get(8)?,
            pinned: r.get(4)?,
            placed: match r.get(5)? {
                true => Placed::At(placed),
                false => Placed::Unknown,
            },
            decided: r.get(7)?,
        })
    })?;
    rows.collect()
}

/// Whether a present file has no track yet.
pub fn any_ungrouped(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM file f
                        WHERE f.present = 1
                          AND NOT EXISTS (SELECT 1 FROM recording_file rf WHERE rf.file_id = f.id))",
        [],
        |r| r.get(0),
    )
}

/// A decided pair as stored: what the planner reads, where it stood, and
/// the link it asks for.
struct StoredPair {
    pair: Pair,
    state: Option<State>,
    link_label: Option<String>,
    link_source: Option<String>,
    link_first: Option<String>,
}

fn stored_pairs(conn: &Connection, scope: Option<&Scope>) -> rusqlite::Result<Vec<StoredPair>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT file_a, file_b, outcome, state, link_label, link_source, link_first
         FROM grouping_pair WHERE 1 {} {} ORDER BY file_a, file_b",
        Scope::files(scope, "file_a"),
        Scope::files(scope, "file_b")
    ))?;
    let rows = stmt.query_map([], |r| {
        let (outcome, state): (String, String) = (r.get(2)?, r.get(3)?);
        Ok(StoredPair {
            pair: Pair {
                a: r.get(0)?,
                b: r.get(1)?,
                // The table's CHECK lists these; anything else is a pair
                // that says nothing.
                outcome: Outcome::parse(&outcome).unwrap_or(Outcome::Different),
            },
            state: State::parse(&state),
            link_label: r.get(4)?,
            link_source: r.get(5)?,
            link_first: r.get(6)?,
        })
    })?;
    rows.collect()
}

fn remembered(conn: &Connection) -> rusqlite::Result<Vec<Remembered>> {
    let mut stmt = conn.prepare("SELECT key_a, key_b, decision FROM grouping_decision")?;
    let rows = stmt.query_map([], |r| {
        let decision: String = r.get(2)?;
        Ok((r.get(0)?, r.get(1)?, Verdict::parse(&decision)))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (key_a, key_b, verdict): (Key, Key, Option<Verdict>) = row?;
        if let Some(verdict) = verdict {
            out.push(Remembered {
                key_a,
                key_b,
                verdict,
            });
        }
    }
    Ok(out)
}

/// A version link as stored.
struct StoredLink {
    id: i64,
    a: i64,
    b: i64,
    kind: String,
    label: Option<String>,
    source: String,
    /// The user made it or confirmed it: never touched here.
    settled: bool,
}

fn version_links(conn: &Connection, scope: Option<&Scope>) -> rusqlite::Result<Vec<StoredLink>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, recording_a, recording_b, kind, label, source,
                source = 'user' OR confirmed = 1
         FROM version_link WHERE 1 {} {} ORDER BY id",
        Scope::tracks(scope, "recording_a"),
        Scope::tracks(scope, "recording_b")
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok(StoredLink {
            id: r.get(0)?,
            a: r.get(1)?,
            b: r.get(2)?,
            kind: r.get(3)?,
            label: r.get(4)?,
            source: r.get(5)?,
            settled: r.get(6)?,
        })
    })?;
    rows.collect()
}

fn library(conn: &Connection, scope: Option<&Scope>) -> rusqlite::Result<HashSet<i64>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT recording_id FROM library_track WHERE 1 {}",
        Scope::tracks(scope, "recording_id")
    ))?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// The tracks something other than their files points at: a Library track,
/// analysis, or a matched rekordbox track.
fn referenced(conn: &Connection, scope: Option<&Scope>) -> rusqlite::Result<HashSet<i64>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT recording_id FROM library_track WHERE 1 {in_scope}
         UNION SELECT recording_id FROM analysis WHERE 1 {in_scope}
         UNION SELECT recording_id FROM rekordbox_track
               WHERE recording_id IS NOT NULL {in_scope}",
        in_scope = Scope::tracks(scope, "recording_id")
    ))?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// Everything a plan is made from, as the database has it now.
struct Loaded {
    members: Vec<Member>,
    pairs: Vec<StoredPair>,
    remembered: Vec<Remembered>,
    links: Vec<StoredLink>,
    library: HashSet<i64>,
    referenced: HashSet<i64>,
}

impl Loaded {
    fn read(conn: &Connection, scope: Option<&Scope>) -> rusqlite::Result<Loaded> {
        Ok(Loaded {
            members: members(conn, scope)?,
            pairs: stored_pairs(conn, scope)?,
            remembered: remembered(conn)?,
            links: version_links(conn, scope)?,
            library: library(conn, scope)?,
            referenced: referenced(conn, scope)?,
        })
    }

    fn plan(&self) -> Plan {
        let pairs: Vec<Pair> = self.pairs.iter().map(|p| p.pair).collect();
        let links: Vec<Link> = self
            .links
            .iter()
            .map(|l| Link {
                a: l.a,
                b: l.b,
                settled: l.settled,
            })
            .collect();
        plan(&Input {
            members: &self.members,
            pairs: &pairs,
            remembered: &self.remembered,
            links: &links,
            library: &self.library,
            referenced: &self.referenced,
            max_set_files: MAX_SET_FILES,
        })
    }
}

/// Why a file wasn't moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A Library track points at the file: it never moves.
    Pinned,
    /// Its track and the other are linked versions: versions never merge.
    LinkedVersions,
}

/// Moves `file`, which has a track, to track `to`. The one way a file
/// changes track here.
///
/// It checks for itself what the schema's triggers check, and more: a
/// pinned file is refused, and so is a move between two linked versions.
/// The triggers only see a row that is updated or replaced, so code that
/// took a file out of its track and put it into another (a delete, then an
/// insert) would get past them; this never does that, and doesn't lean on
/// them. A move starts over: the file's role in its old track means nothing
/// in the new one.
pub(crate) fn relocate(
    conn: &Connection,
    file: i64,
    to: i64,
) -> rusqlite::Result<Result<i64, Refused>> {
    let from: i64 = conn
        .prepare_cached("SELECT recording_id FROM recording_file WHERE file_id = ?1")?
        .query_row([file], |r| r.get(0))?;
    let pinned: bool = conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM library_track
                            WHERE linked_file_id = ?1 OR source_file_id = ?1)",
        )?
        .query_row([file], |r| r.get(0))?;
    if pinned {
        return Ok(Err(Refused::Pinned));
    }
    let linked: bool = conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM version_link
                            WHERE (recording_a = ?1 AND recording_b = ?2)
                               OR (recording_a = ?2 AND recording_b = ?1))",
        )?
        .query_row([from, to], |r| r.get(0))?;
    if linked {
        return Ok(Err(Refused::LinkedVersions));
    }
    conn.prepare_cached(
        "UPDATE recording_file SET recording_id = ?1, role = 'undecided',
                match_confidence = NULL
         WHERE file_id = ?2",
    )?
    .execute(params![to, file])?;
    Ok(Ok(from))
}

/// Groups every file: each present file ends up in exactly one track (see
/// [`super::plan`] for which). Then, whether or not any file moved: every
/// matched rekordbox track points at its file's track, tracks left with
/// nothing are deleted, every track with files has a best, the job's
/// version links are the ones the decided pairs ask for, and every decided
/// pair says where it stands. Safe to run again at any time; a run with
/// nothing to change writes nothing.
///
/// Several transactions on a big change ([`regroup_step`]); each is all or
/// nothing.
pub fn regroup(conn: &mut Connection) -> rusqlite::Result<Summary> {
    regroup_in_batches(conn, BATCH)
}

/// [`regroup`], at most `batch` files a transaction.
pub(crate) fn regroup_in_batches(conn: &mut Connection, batch: usize) -> rusqlite::Result<Summary> {
    let mut summary = Summary::default();
    loop {
        let (step, more) = regroup_step(conn, batch)?;
        summary.add(step);
        if !more {
            return Ok(summary);
        }
    }
}

/// One transaction of a grouping pass: plans from what the database holds
/// now and applies at most `batch` of it. Returns what it did, and whether
/// there is more to do (then call it again).
pub fn regroup_step(conn: &mut Connection, batch: usize) -> rusqlite::Result<(Summary, bool)> {
    let step = step(conn, batch)?;
    Ok((step.summary, step.more))
}

/// What one transaction of a grouping pass did.
pub struct Step {
    pub summary: Summary,
    /// There is more to do: call again.
    pub more: bool,
    /// The pairs inside waiting sets that nobody decided yet, as the plan
    /// this step was made from lists them ([`Plan::missing`]): what the
    /// deciding job asks the rules about before those sets can merge. What
    /// stands is the last step's.
    pub undecided: Vec<(i64, i64)>,
}

/// A grouping pass over part of the library: the files `files`, and every
/// file that could join a set with them ([`Scope`]: the files of their
/// tracks, their stored duplicate pairs, the same `audio_hash`, grown until
/// nothing joins). For an action that changed those files' tracks and needs
/// where they stand right as it returns (Review's merge and "Keep both",
/// 1bE-9), without planning the whole library on the writer. Inside the
/// scope it decides what a whole pass decides: a set is planned from all of
/// its own files and pairs. Pairs to files outside it, and everything else,
/// are left to the next whole pass (the grouping job). The sweep of tracks
/// left with nothing and the best file of every track run as in a whole
/// pass (one query each).
pub(crate) fn regroup_files(conn: &mut Connection, files: &[i64]) -> rusqlite::Result<Summary> {
    let scope = Scope::of(conn, files)?;
    let mut summary = Summary::default();
    loop {
        let step = step_in(conn, BATCH, Some(&scope))?;
        summary.add(step.summary);
        if !step.more {
            return Ok(summary);
        }
    }
}

/// [`regroup_step`], with the pairs still to decide.
pub fn step(conn: &mut Connection, batch: usize) -> rusqlite::Result<Step> {
    step_in(conn, batch, None)
}

/// [`step`] over the whole library, or over `scope` only.
fn step_in(conn: &mut Connection, batch: usize, scope: Option<&Scope>) -> rusqlite::Result<Step> {
    let batch = batch.max(1);
    let tx = conn.transaction()?;
    let loaded = Loaded::read(&tx, scope)?;
    let mut plan = loaded.plan();
    let mut summary = Summary {
        left_alone: plan.left_alone as u64,
        sets_disagreeing: plan.sets_disagreeing as u64,
        sets_both_in_library: plan.sets_both_in_library as u64,
        sets_waiting: plan.sets_waiting as u64,
        undecided_pairs: plan.missing.len() as u64,
        ..Summary::default()
    };
    // What each file is recorded as placed with: the audio it holds.
    let keys: HashMap<i64, Option<&[u8]>> = loaded
        .members
        .iter()
        .map(|m| (m.file, m.content.as_deref()))
        .collect();

    // The job's own links that no pair asks for any more go first: they
    // would hold back a move that is right now.
    let unsupported: HashSet<(i64, i64)> = plan.unsupported_links.iter().copied().collect();
    let mut links: Vec<&StoredLink> = Vec::new();
    for link in &loaded.links {
        if !link.settled && unsupported.contains(&(link.a.min(link.b), link.a.max(link.b))) {
            tx.execute("DELETE FROM version_link WHERE id = ?1", [link.id])?;
            summary.links_removed += 1;
        } else {
            links.push(link);
        }
    }

    // Moves, then the records of files that stay put, up to the batch.
    let mut made: HashMap<usize, i64> = HashMap::new();
    // Where the files of each track that lost some went, and how many.
    let mut heirs: BTreeMap<i64, BTreeMap<i64, u32>> = BTreeMap::new();
    let moves = &plan.moves[..plan.moves.len().min(batch)];
    {
        let mut make = tx.prepare_cached("INSERT INTO recording DEFAULT VALUES")?;
        let mut place = tx
            .prepare_cached("INSERT INTO recording_file (recording_id, file_id) VALUES (?1, ?2)")?;
        for Move { file, from, to } in moves {
            let to = match *to {
                Dest::Recording(id) => id,
                Dest::New(n) => match made.get(&n) {
                    Some(&id) => id,
                    None => {
                        make.execute([])?;
                        let id = tx.last_insert_rowid();
                        made.insert(n, id);
                        summary.recordings_made += 1;
                        id
                    }
                },
            };
            match from {
                None => {
                    place.execute(params![to, file])?;
                    summary.placed += 1;
                }
                Some(_) => match relocate(&tx, *file, to)? {
                    Ok(old) => {
                        *heirs.entry(old).or_default().entry(to).or_default() += 1;
                        summary.moved += 1;
                    }
                    Err(_) => {
                        summary.refused += 1;
                        continue;
                    }
                },
            }
            record_placed(&tx, *file, keys.get(file).copied().flatten())?;
        }
    }
    let all_moved = moves.len() == plan.moves.len();
    let room = batch - moves.len();
    let settle = match all_moved {
        true => &plan.settle[..plan.settle.len().min(room)],
        false => &plan.settle[..0],
    };
    for file in settle {
        record_placed(&tx, *file, keys.get(file).copied().flatten())?;
    }
    let done = all_moved && settle.len() == plan.settle.len();

    sync_rekordbox(&tx)?;
    settle_analysis(&tx, &heirs)?;
    settle_removals(&tx, &heirs)?;
    if done {
        // With every file where the plan puts it: the links the pairs ask
        // for, and where each pair stands.
        let dest = |dest: Dest| match dest {
            Dest::Recording(id) => Some(id),
            Dest::New(n) => made.get(&n).copied(),
        };
        sync_links(&tx, &loaded, &plan, &links, dest, &mut summary)?;
        let mut set_state = tx.prepare_cached(
            "UPDATE grouping_pair SET state = ?3 WHERE file_a = ?1 AND file_b = ?2",
        )?;
        for (stored, state) in loaded.pairs.iter().zip(&plan.states) {
            if let Some(state) = state.filter(|state| Some(*state) != stored.state) {
                set_state.execute(params![stored.pair.a, stored.pair.b, state.as_str()])?;
            }
        }
    }
    summary.recordings_removed = sweep(&tx)?;
    ensure_best(&tx)?;
    tx.commit()?;
    // A step that got nowhere (every move refused) mustn't be asked for
    // again and again.
    let got_somewhere = summary.placed + summary.moved > 0 || !settle.is_empty();
    Ok(Step {
        summary,
        more: !done && got_somewhere,
        undecided: std::mem::take(&mut plan.missing),
    })
}

/// Records the audio `file` holds ([`CONTENT_KEY`]) as the audio it was
/// placed with.
fn record_placed(conn: &Connection, file: i64, key: Option<&[u8]>) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO grouping_file (file_id, placed_key) VALUES (?1, ?2)
         ON CONFLICT (file_id) DO UPDATE SET placed_key = excluded.placed_key",
    )?
    .execute(params![file, key])?;
    Ok(())
}

/// Makes the job's version links the ones the decided pairs ask for: adds
/// the missing ones, takes away its own that no pair asks for or that a
/// pair now asks for differently. The user's links are never touched, and a
/// pair of tracks they linked gets no second link.
fn sync_links(
    tx: &Transaction<'_>,
    loaded: &Loaded,
    plan: &Plan,
    links: &[&StoredLink],
    dest: impl Fn(Dest) -> Option<i64>,
    summary: &mut Summary,
) -> rusqlite::Result<()> {
    /// A link as a pair asks for it.
    #[derive(PartialEq)]
    struct Asked<'a> {
        a: i64,
        b: i64,
        kind: &'a str,
        label: Option<&'a str>,
        source: &'a str,
    }
    let mut asked: BTreeMap<(i64, i64), Asked<'_>> = BTreeMap::new();
    for wanted in &plan.wanted_links {
        let (Some(a), Some(b)) = (dest(wanted.a), dest(wanted.b)) else {
            continue;
        };
        let stored = &loaded.pairs[wanted.pair];
        let Some(source) = stored.link_source.as_deref() else {
            continue;
        };
        // The direction a label has (a mashup contains its part); else the
        // lower track first.
        let (first, second) = match stored.link_first.as_deref() {
            Some("a") => (a, b),
            Some("b") => (b, a),
            _ => (a.min(b), a.max(b)),
        };
        asked.insert(
            (a.min(b), a.max(b)),
            Asked {
                a: first,
                b: second,
                kind: stored.pair.outcome.as_str(),
                label: stored.link_label.as_deref(),
                source,
            },
        );
    }
    for link in links {
        let key = (link.a.min(link.b), link.a.max(link.b));
        if link.settled {
            asked.remove(&key);
            continue;
        }
        let as_asked = Asked {
            a: link.a,
            b: link.b,
            kind: &link.kind,
            label: link.label.as_deref(),
            source: &link.source,
        };
        if asked.get(&key) == Some(&as_asked) {
            asked.remove(&key);
        } else {
            tx.execute("DELETE FROM version_link WHERE id = ?1", [link.id])?;
            summary.links_removed += 1;
        }
    }
    let mut add = tx.prepare_cached(
        "INSERT INTO version_link (recording_a, recording_b, kind, label, source)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for link in asked.values() {
        add.execute(params![link.a, link.b, link.kind, link.label, link.source])?;
        summary.links_made += 1;
    }
    Ok(())
}

/// Points each matched rekordbox track at its file's track. Grouping owns
/// `rekordbox_track.recording_id`: a match (relink) sets `file_id`, a fresh
/// read of the snapshot clears `recording_id`, and this puts it right
/// again after placements, moves and both of those. A row with no matched
/// file has no track either.
pub(crate) fn sync_rekordbox(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE rekordbox_track
         SET recording_id = (SELECT rf.recording_id FROM recording_file rf
                             WHERE rf.file_id = rekordbox_track.file_id)
         WHERE recording_id IS NOT (SELECT rf.recording_id FROM recording_file rf
                                    WHERE rf.file_id = rekordbox_track.file_id)",
        [],
    )?;
    Ok(())
}

/// The track most of a lost track's files went to (ties to the lower id).
fn heir_of(went_to: &BTreeMap<i64, u32>) -> Option<i64> {
    went_to
        .iter()
        .max_by_key(|&(&id, &count)| (count, std::cmp::Reverse(id)))
        .map(|(&id, _)| id)
}

/// Settles the removal record (`library_removal`, the user removed the
/// track from the Library) of tracks that lost all their files to a merge.
/// The record moves to the heir, unless the heir is in the Library (it
/// isn't removed any more, so the record is dropped). If the heir has a
/// record too, the earlier one is kept. A track that still has files keeps
/// its record.
fn settle_removals(
    conn: &Connection,
    heirs: &BTreeMap<i64, BTreeMap<i64, u32>>,
) -> rusqlite::Result<()> {
    for (&old, went_to) in heirs {
        let has_files: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM recording_file WHERE recording_id = ?1)",
            [old],
            |r| r.get(0),
        )?;
        if has_files {
            continue;
        }
        let Some(heir) = heir_of(went_to) else {
            continue;
        };
        let in_library: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM library_track WHERE recording_id = ?1)",
            [heir],
            |r| r.get(0),
        )?;
        let removed_at = |recording: i64| -> rusqlite::Result<Option<String>> {
            conn.query_row(
                "SELECT removed_at FROM library_removal WHERE recording_id = ?1",
                [recording],
                |r| r.get(0),
            )
            .optional()
        };
        let Some(old_at) = removed_at(old)? else {
            continue;
        };
        if in_library {
            conn.execute("DELETE FROM library_removal WHERE recording_id = ?1", [old])?;
            continue;
        }
        if let Some(heir_at) = removed_at(heir)? {
            // Both were removed: the earlier record stays, on the heir.
            let drop = if old_at < heir_at { heir } else { old };
            conn.execute(
                "DELETE FROM library_removal WHERE recording_id = ?1",
                [drop],
            )?;
            if drop == heir {
                conn.execute(
                    "UPDATE library_removal SET recording_id = ?2 WHERE recording_id = ?1",
                    [old, heir],
                )?;
            }
        } else {
            conn.execute(
                "UPDATE library_removal SET recording_id = ?2 WHERE recording_id = ?1",
                [old, heir],
            )?;
        }
    }
    Ok(())
}

/// Settles the `analysis` of tracks that lost files to a merge or a move,
/// so deleting them can't be held back by it (`analysis.recording_id` is
/// `ON DELETE RESTRICT`). A track left with no files hands every source
/// but rekordbox over to the track most of its files went to (ties to the
/// lower id), unless that track has a row of the same source already, in
/// which case the heir's own is kept. Whatever is left on it, rekordbox's
/// rows included, goes: attach recomputes those. A track that still has
/// files keeps its rows.
fn settle_analysis(
    conn: &Connection,
    heirs: &BTreeMap<i64, BTreeMap<i64, u32>>,
) -> rusqlite::Result<()> {
    let mut has_files = conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM recording_file WHERE recording_id = ?1)")?;
    let mut hand_over = conn.prepare_cached(
        "UPDATE OR IGNORE analysis SET recording_id = ?2
         WHERE recording_id = ?1 AND source <> 'rekordbox'",
    )?;
    let mut drop_rest = conn.prepare_cached("DELETE FROM analysis WHERE recording_id = ?1")?;
    for (&old, went_to) in heirs {
        if has_files.query_row([old], |r| r.get::<_, bool>(0))? {
            continue;
        }
        if let Some(heir) = heir_of(went_to) {
            hand_over.execute([old, heir])?;
        }
        drop_rest.execute([old])?;
    }
    drop_fileless_rekordbox_analysis(conn)
}

/// Drops the rows rekordbox gave to tracks that hold no file (no entry can
/// point at such a track, so attach has nothing to keep them for).
fn drop_fileless_rekordbox_analysis(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM analysis
         WHERE source = 'rekordbox'
           AND NOT EXISTS (SELECT 1 FROM recording_file rf
                           WHERE rf.recording_id = analysis.recording_id)",
        [],
    )?;
    Ok(())
}

/// Deletes every track that has no files and nothing else pointing at it,
/// wherever it came from (a move emptied it, or the last thing pointing at
/// it went). Returns how many were deleted.
///
/// A track the user removed from the Library (`library_removal`) that
/// has no files can't be offered again, so its record goes with it, unless
/// it was sent to rekordbox before: then the record is what tells the user
/// to remove it there by hand (ROADMAP 1.9 rule 7), so it and the track stay.
fn sweep(conn: &Connection) -> rusqlite::Result<u64> {
    conn.execute(
        &format!(
            "DELETE FROM library_removal
             WHERE last_sent_location IS NULL
               AND recording_id IN (SELECT id FROM recording WHERE {UNREFERENCED})"
        ),
        [],
    )?;
    let deleted = conn.execute(
        &format!(
            "DELETE FROM recording WHERE {UNREFERENCED}
             AND NOT EXISTS (SELECT 1 FROM library_removal WHERE recording_id = recording.id)"
        ),
        [],
    )?;
    Ok(deleted as u64)
}

/// Gives every track with files but no best file one, for the moment: the
/// first file still undecided (on disk before one that's gone), else the
/// first file marked extra, in the same order. A best that's already there
/// is never changed. Which file is really best is the roles job's
/// ([`crate::file_roles`]), which runs for every track this changes.
fn ensure_best(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE recording_file SET role = 'best'
         WHERE file_id IN (
             SELECT (SELECT rf.file_id
                     FROM recording_file rf JOIN file f ON f.id = rf.file_id
                     WHERE rf.recording_id = t.recording_id AND rf.role <> 'best'
                     ORDER BY rf.role = 'extra', f.present DESC, rf.file_id
                     LIMIT 1)
             FROM (SELECT DISTINCT recording_id FROM recording_file) t
             WHERE NOT EXISTS (SELECT 1 FROM recording_file b
                               WHERE b.recording_id = t.recording_id AND b.role = 'best'))",
        [],
    )?;
    Ok(())
}

/// Lets go of the files of a music folder that's being removed, so their
/// `recording_file` rows don't hold the `file` rows back: deletes each
/// file's row in its track, whatever its role, then deletes any track left
/// with no files and nothing else pointing at it, and gives each track that
/// lost its best file a new one. A track that still has files in another
/// folder stays with them.
///
/// Runs inside the caller's transaction, so it rolls back with it. Rows
/// pointing straight at a file (a Library track's `linked_file_id`) aren't
/// touched: they still refuse the removal. What grouping keeps per file
/// (`grouping_file`, `grouping_pair`) goes with the file row; a remembered
/// decision stays, since it names the audio, not the file.
pub fn release_folder_files(conn: &Connection, folder: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM recording_file
         WHERE file_id IN (SELECT id FROM file WHERE music_folder_id = ?1)",
        [folder],
    )?;
    drop_fileless_rekordbox_analysis(conn)?;
    sweep(conn)?;
    ensure_best(conn)
}

/// Why a decision couldn't be remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CantRemember {
    /// Both sides are the same file.
    SameFile,
    /// A file isn't known, or has neither an `audio_hash` nor a `blake3`
    /// yet: there is nothing to name its audio by.
    NoAudioKnown,
}

/// Remembers what the user decided about the tracks of files `a` and `b`
/// ([`Verdict`]), and the next grouping run acts on it.
///
/// A decision is kept by audio, not by file or track: one row for every
/// pair of audio between the two tracks as they are now (each file's
/// [`CONTENT_KEY`]), replacing an earlier decision about the same two
/// pieces of audio. So it covers every pair of files between the two
/// tracks, and still holds when the file it was made on is overwritten or
/// its music folder is removed, as long as each track keeps a file it held
/// then.
///
/// What it doesn't do:
///
/// - **"Apart" for two files that already share a track changes nothing**:
///   a merge is never undone by the job, and the pair goes on reading as
///   merged. Taking a file out of a track is an action of its own (Review,
///   1bE), not a remembered decision. Only the one row for the two files'
///   audio is written then.
/// - **A file with no `audio_hash` is named by its `blake3`**, which a tag
///   rewrite changes: a decision resting on such a file alone doesn't
///   survive one.
/// - A file with no audio known at all is refused, and files in the two
///   tracks with none are left out of the rows.
///
/// Every row is written through the operation `rec` records (a new row is
/// an insert, a changed decision a set), so the user's Undo takes the
/// decision back; nothing else writes `grouping_decision`, so its ids are
/// only ever made through the log. Review's "Keep both" calls it
/// ([`crate::review::same`]).
pub fn remember(
    rec: &mut Recorder<'_>,
    a: i64,
    b: i64,
    verdict: Verdict,
) -> Result<Result<(), CantRemember>, OpsError> {
    if a == b {
        return Ok(Err(CantRemember::SameFile));
    }
    let content = |rec: &Recorder<'_>, file: i64| -> Result<Option<Key>, OpsError> {
        let found: Option<Option<Key>> = rec
            .read_rows(
                &format!("SELECT {CONTENT_KEY} FROM file f WHERE f.id = ?1"),
                [file],
                |r| r.get(0),
            )?
            .pop();
        Ok(found.flatten().filter(|key| !key.is_empty()))
    };
    let (Some(key_a), Some(key_b)) = (content(rec, a)?, content(rec, b)?) else {
        return Ok(Err(CantRemember::NoAudioKnown));
    };
    let track = |rec: &Recorder<'_>, file: i64| -> Result<Option<i64>, OpsError> {
        Ok(rec
            .read_rows(
                "SELECT recording_id FROM recording_file WHERE file_id = ?1",
                [file],
                |r| r.get(0),
            )?
            .pop())
    };
    // Every kind of audio the file's track holds, the file's own included.
    let held_with =
        |rec: &Recorder<'_>, track: Option<i64>, own: Key| -> Result<Vec<Key>, OpsError> {
            let mut held = vec![own];
            if let Some(track) = track {
                let keys = rec.read_rows(
                    &format!(
                        "SELECT DISTINCT {CONTENT_KEY} FROM recording_file rf
                     JOIN file f ON f.id = rf.file_id
                     WHERE rf.recording_id = ?1"
                    ),
                    [track],
                    |r| r.get::<_, Option<Key>>(0),
                )?;
                held.extend(keys.into_iter().flatten().filter(|key| !key.is_empty()));
            }
            held.sort_unstable();
            held.dedup();
            Ok(held)
        };
    let (track_a, track_b) = (track(rec, a)?, track(rec, b)?);
    let (side_a, side_b) = match track_a.is_some() && track_a == track_b {
        // One track: nothing to keep apart or bring together between two.
        true => (vec![key_a], vec![key_b]),
        false => (
            held_with(rec, track_a, key_a)?,
            held_with(rec, track_b, key_b)?,
        ),
    };
    let mut pairs: Vec<(&Key, &Key)> = Vec::new();
    for key_a in &side_a {
        for key_b in &side_b {
            pairs.push(match key_a <= key_b {
                true => (key_a, key_b),
                false => (key_b, key_a),
            });
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    for (low, high) in pairs {
        let stored: Option<(i64, String)> = rec
            .read_rows(
                "SELECT id, decision FROM grouping_decision WHERE key_a = ?1 AND key_b = ?2",
                params![low, high],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            .pop();
        match stored {
            None => {
                rec.insert(
                    "grouping_decision",
                    &[
                        ("key_a", Value::Blob(low.clone())),
                        ("key_b", Value::Blob(high.clone())),
                        ("decision", Value::Text(verdict.as_str().to_owned())),
                    ],
                )?;
            }
            Some((id, decision)) if decision != verdict.as_str() => {
                let now: String = rec
                    .read_rows("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
                        r.get(0)
                    })?
                    .pop()
                    .unwrap_or_default();
                rec.set("grouping_decision", id, "decision", verdict.as_str())?;
                rec.set("grouping_decision", id, "decided_at", now)?;
            }
            Some(_) => {}
        }
    }
    Ok(Ok(()))
}
