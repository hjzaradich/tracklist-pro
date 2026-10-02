//! Undo over the app's real actions: adding to and removing from the
//! Library, the add from rekordbox, and every crate change. Each test
//! compares whole tables, row by row, so "restored" means every column of
//! every row, ids and times included. Everything is synthetic.

use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rusqlite::types::Value;
use rusqlite::Connection;

use super::{
    next_undo, undo_last_via, undo_only, ConflictProblem, NextUndo, OperationDetails, UndoOutcome,
    UndoRefusal, TRIED_AHEAD_UP_TO,
};
use crate::crates::{self, CrateId};
use crate::db::Writer;
use crate::fragile::FragileDirs;
use crate::library::{self, LibraryTrackId};
use crate::paths::Volumes;
use crate::rekordbox_write::{build, gather, record_send, Outgoing};
use crate::send::crate_tree;
use crate::start;
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

/// The tables the app's actions change through the log.
pub(super) const LOGGED_TABLES: [&str; 6] = [
    "library_track",
    "library_removal",
    "crate",
    "crate_entry",
    "sync_base",
    "conflict",
];

/// How many tracks the test Library can hold. The first [`IN_REKORDBOX`]
/// are also in rekordbox, with a trusted match.
const TRACKS: usize = 6;
const IN_REKORDBOX: usize = 3;

/// Crate names to draw from. Some equal each other by the writer's sibling
/// rule (letter case, trailing space), so refusals happen too.
const NAMES: [&str; 5] = ["House", "house", "Techno", "Warm up", "Warm up "];

/// The one test volume, plugged in at `E:\`.
struct Plugged;

impl Volumes for Plugged {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up in these tests",
            path.display()
        )))
    }

    fn mount_path(&self, _id: &VolumeId) -> Option<PathBuf> {
        Some(PathBuf::from(r"E:\"))
    }
}

const NO_DIRS: FragileDirs = FragileDirs {
    downloads: None,
    temp: Vec::new(),
};

/// One of the app's actions. Numbers pick among what exists when the action
/// runs (the n-th crate, the n-th Library track), wrapping around.
#[derive(Debug, Clone)]
pub(super) enum Action {
    /// "Add to Library" for one track.
    Add(usize),
    /// Add every offered rekordbox track.
    AddFromRekordbox,
    /// "Remove from Library".
    Remove(usize),
    CreateCrate(usize),
    RenameCrate(usize, usize),
    DeleteCrate(usize),
    AddToCrate(usize, Vec<usize>),
    RemoveFromCrate(usize, Vec<usize>),
}

/// Every row of every logged table, each value as text.
pub(super) type State = Vec<(&'static str, Vec<Vec<String>>)>;

/// A database with [`TRACKS`] tracks, each with one file on disk, and an
/// empty Library.
pub(super) struct Lib {
    pub dir: tempfile::TempDir,
    pub writer: Writer,
}

pub(super) fn open(dir: &Path) -> Writer {
    Writer::open(&crate::write_guard::test_path(dir, crate::db::DB_FILE_NAME)).unwrap()
}

fn dump(conn: &Connection, table: &str) -> Vec<Vec<String>> {
    rows(conn, &format!("SELECT * FROM {table} ORDER BY id"))
}

fn rows(conn: &Connection, sql: &str) -> Vec<Vec<String>> {
    let mut stmt = conn.prepare(sql).unwrap();
    let columns = stmt.column_count();
    stmt.query_map([], |r| {
        (0..columns)
            .map(|i| r.get::<_, Value>(i).map(|v| format!("{v:?}")))
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

impl Lib {
    pub fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = open(dir.path());
        let volume = identity(IdentitySignals {
            kind: VolumeKind::External,
            unc_share: None,
            serial: Some(0x1A2B_3C4D),
            filesystem: "NTFS",
            guid: None,
        })
        .unwrap()
        .as_str()
        .to_owned();
        writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES (?1, 'external', 'E:\\')",
                    [volume],
                )?;
                c.execute_batch(
                    "INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');",
                )?;
                for n in 1..=TRACKS {
                    let name = format!("{n}.mp3");
                    c.execute("INSERT INTO recording (title) VALUES (?1)", [&name])?;
                    c.execute(
                        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
                         VALUES (1, ?1, ?1, 1)",
                        [&name],
                    )?;
                    c.execute(
                        "INSERT INTO recording_file (recording_id, file_id, role)
                         VALUES (?1, ?1, 'best')",
                        [n as i64],
                    )?;
                    if n <= IN_REKORDBOX {
                        let attributes = serde_json::json!({
                            "TrackID": n.to_string(),
                            "Location": format!("file://localhost/E:/Music/{name}"),
                        })
                        .to_string();
                        c.execute(
                            "INSERT INTO rekordbox_track
                                 (attributes, location_key, read_at, file_id, relink_method,
                                  relink_probable)
                             VALUES (?1, ?2, '2026-09-30T10:00:00.000Z', ?3, 'path', 0)",
                            (attributes, format!("E:/Music/{name}"), n as i64),
                        )?;
                    }
                }
                Ok(())
            })
            .unwrap();
        Lib { dir, writer }
    }

    pub fn state(&self) -> State {
        self.writer
            .call(|c| Ok(LOGGED_TABLES.map(|table| (table, dump(c, table))).to_vec()))
            .unwrap()
    }

    fn ids(&self, sql: &'static str) -> Vec<i64> {
        self.writer
            .call(move |c| {
                let mut stmt = c.prepare(sql)?;
                let ids = stmt.query_map([], |r| r.get(0))?.collect();
                ids
            })
            .unwrap()
    }

    pub fn library_tracks(&self) -> Vec<i64> {
        self.ids("SELECT id FROM library_track ORDER BY id")
    }

    pub fn crate_ids(&self) -> Vec<i64> {
        self.ids("SELECT id FROM crate ORDER BY id")
    }

    /// Operations made and not undone.
    pub fn operations(&self) -> i64 {
        self.writer
            .call(|c| {
                c.query_row(
                    "SELECT count(*) FROM operation WHERE undone_at IS NULL",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// Runs one action as the app's commands do. Answers whether it changed
    /// anything, i.e. recorded an operation. An action the app refuses (a
    /// taken crate name) or that has nothing to act on changes nothing.
    pub fn apply(&self, action: &Action) -> bool {
        let before = self.operations();
        let pick = |from: &[i64], n: usize| (!from.is_empty()).then(|| from[n % from.len()]);
        let pick_tracks = |from: &[i64], ns: &[usize]| -> Vec<LibraryTrackId> {
            ns.iter()
                .filter_map(|&n| pick(from, n))
                .map(LibraryTrackId)
                .collect()
        };
        match action {
            Action::Add(n) => {
                let recording = (n % TRACKS) as i64 + 1;
                library::promote_with(&self.writer, &Plugged, &NO_DIRS, recording).unwrap();
            }
            Action::AddFromRekordbox => {
                self.writer
                    .call(|c| Ok(start::add_offered(c, None)))
                    .unwrap()
                    .unwrap();
            }
            Action::Remove(n) => {
                if let Some(id) = pick(&self.library_tracks(), *n) {
                    library::remove(&self.writer, LibraryTrackId(id)).unwrap();
                }
            }
            Action::CreateCrate(name) => {
                let name = NAMES[name % NAMES.len()];
                // A taken name is refused, which is fine here.
                let _ = self
                    .writer
                    .call(move |c| Ok(crates::create_on(c, name)))
                    .unwrap()
                    .unwrap();
            }
            Action::RenameCrate(n, name) => {
                if let Some(id) = pick(&self.crate_ids(), *n) {
                    let name = NAMES[name % NAMES.len()];
                    let _ = self
                        .writer
                        .call(move |c| Ok(crates::rename_on(c, CrateId(id), name)))
                        .unwrap()
                        .unwrap();
                }
            }
            Action::DeleteCrate(n) => {
                if let Some(id) = pick(&self.crate_ids(), *n) {
                    self.writer
                        .call(move |c| Ok(crates::delete_on(c, CrateId(id))))
                        .unwrap()
                        .unwrap()
                        .unwrap();
                }
            }
            Action::AddToCrate(n, tracks) => {
                if let Some(id) = pick(&self.crate_ids(), *n) {
                    let tracks = pick_tracks(&self.library_tracks(), tracks);
                    self.writer
                        .call(move |c| Ok(crates::add_on(c, CrateId(id), &tracks)))
                        .unwrap()
                        .unwrap()
                        .unwrap();
                }
            }
            Action::RemoveFromCrate(n, tracks) => {
                if let Some(id) = pick(&self.crate_ids(), *n) {
                    let tracks = pick_tracks(&self.library_tracks(), tracks);
                    self.writer
                        .call(move |c| Ok(crates::remove_on(c, CrateId(id), &tracks)))
                        .unwrap()
                        .unwrap()
                        .unwrap();
                }
            }
        }
        self.operations() > before
    }

    pub fn undo(&self) -> UndoOutcome {
        undo_last_via(&self.writer).unwrap()
    }

    pub fn undone(&self) -> super::OperationInfo {
        match self.undo() {
            UndoOutcome::Undone { operation } => operation,
            other => panic!("expected the undo to work, got {other:?}"),
        }
    }

    pub fn next(&self) -> NextUndo {
        self.writer.call(|c| Ok(next_undo(c))).unwrap().unwrap()
    }

    /// The Library track of track `n` (1-based).
    fn library_track(&self, n: i64) -> LibraryTrackId {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT id FROM library_track WHERE recording_id = ?1",
                    [n],
                    |r| r.get(0),
                )
            })
            .map(LibraryTrackId)
            .unwrap()
    }

    fn add(&self, n: i64) -> LibraryTrackId {
        assert!(self.apply(&Action::Add(n as usize - 1)));
        self.library_track(n)
    }

    fn create(&self, name: &str) -> CrateId {
        let name = name.to_owned();
        self.writer
            .call(move |c| Ok(crates::create_on(c, &name)))
            .unwrap()
            .unwrap()
            .unwrap()
    }

    fn rename(&self, id: CrateId, name: &'static str) {
        self.writer
            .call(move |c| Ok(crates::rename_on(c, id, name)))
            .unwrap()
            .unwrap()
            .unwrap();
    }

    fn add_to(&self, id: CrateId, tracks: &[LibraryTrackId]) -> crates::Changed {
        let tracks = tracks.to_vec();
        self.writer
            .call(move |c| Ok(crates::add_on(c, id, &tracks)))
            .unwrap()
            .unwrap()
            .unwrap()
    }

    fn run(&self, sql: &'static str) {
        self.writer.call(move |c| c.execute_batch(sql)).unwrap();
    }

    /// What a send of the whole Library would write now, built by the real
    /// writer from the real values. Changes nothing.
    fn build_send(&self) -> Outgoing {
        self.writer
            .call(|c| {
                let tracks: Vec<LibraryTrackId> = c
                    .prepare("SELECT id FROM library_track ORDER BY id")?
                    .query_map([], |r| r.get(0).map(LibraryTrackId))?
                    .collect::<rusqlite::Result<_>>()?;
                let input = gather(c, &Plugged, &tracks, crate_tree(c)?, Vec::new())?;
                Ok(build(&input).expect("the send builds"))
            })
            .unwrap()
    }

    /// A send, recorded by the real `record_send` (ROADMAP 1.9 rule 8). The
    /// file itself isn't written: undo only ever meets what was recorded.
    pub fn send(&self) -> Outgoing {
        let out = self.build_send();
        let recorded = out.clone();
        self.writer
            .call(move |c| record_send(c, &recorded.sent, recorded.paths()))
            .unwrap();
        out
    }

    /// The rows of the logged tables that belong to one Library track.
    fn rows_of(&self, track: LibraryTrackId) -> Vec<Vec<Vec<String>>> {
        self.writer
            .call(move |c| {
                Ok([
                    format!("SELECT * FROM library_track WHERE id = {}", track.0),
                    format!(
                        "SELECT * FROM crate_entry WHERE library_track_id = {}",
                        track.0
                    ),
                    format!(
                        "SELECT * FROM sync_base WHERE library_track_id = {}",
                        track.0
                    ),
                    format!(
                        "SELECT * FROM conflict WHERE library_track_id = {}",
                        track.0
                    ),
                ]
                .map(|sql| rows(c, &format!("{sql} ORDER BY id")))
                .to_vec())
            })
            .unwrap()
    }

    /// The logged tables and the log itself.
    fn everything(&self) -> Vec<Vec<Vec<String>>> {
        self.writer
            .call(|c| {
                Ok(LOGGED_TABLES
                    .iter()
                    .chain(&["operation", "change"])
                    .map(|table| dump(c, table))
                    .collect())
            })
            .unwrap()
    }

    fn broken_references(&self) -> i64 {
        self.writer
            .call(|c| {
                c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })
            })
            .unwrap()
    }
}

fn refusal(outcome: UndoOutcome) -> (super::OperationInfo, UndoRefusal) {
    match outcome {
        UndoOutcome::Refused {
            operation, reason, ..
        } => (operation, reason),
        other => panic!("expected the undo to be refused, got {other:?}"),
    }
}

pub(super) fn action() -> impl Strategy<Value = Action> {
    let tracks = || proptest::collection::vec(0..TRACKS, 1..4);
    prop_oneof![
        3 => (0..TRACKS).prop_map(Action::Add),
        1 => Just(Action::AddFromRekordbox),
        2 => (0..TRACKS).prop_map(Action::Remove),
        3 => (0..NAMES.len()).prop_map(Action::CreateCrate),
        2 => (0..4usize, 0..NAMES.len()).prop_map(|(n, name)| Action::RenameCrate(n, name)),
        1 => (0..4usize).prop_map(Action::DeleteCrate),
        3 => (0..4usize, tracks()).prop_map(|(n, t)| Action::AddToCrate(n, t)),
        2 => (0..4usize, tracks()).prop_map(|(n, t)| Action::RemoveFromCrate(n, t)),
    ]
}

/// One step of a history with sends in it.
#[derive(Debug, Clone)]
enum Step {
    Do(Action),
    Send,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        5 => action().prop_map(Step::Do),
        1 => Just(Step::Send),
    ]
}

proptest! {
    // Each case builds a database, so fewer cases than the default 256.
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn after_any_actions_as_many_undos_restore_the_tables_step_by_step_back_to_the_start(
        actions in proptest::collection::vec(action(), 1..14),
    ) {
        let lib = Lib::new();
        // The tables before each action that changed something.
        let mut before = Vec::new();
        for action in &actions {
            let state = lib.state();
            if lib.apply(action) {
                before.push(state);
            } else {
                prop_assert_eq!(lib.state(), state, "{:?} recorded nothing", action);
            }
        }
        while let Some(state) = before.pop() {
            let outcome = lib.undo();
            let undone = matches!(outcome, UndoOutcome::Undone { .. });
            prop_assert!(undone, "got {:?}", outcome);
            prop_assert_eq!(lib.state(), state);
        }
        prop_assert_eq!(lib.undo(), UndoOutcome::NothingToUndo);
        prop_assert_eq!(lib.next().operation, None);
    }

    #[test]
    fn with_sends_between_actions_each_undo_restores_or_is_refused_whole_and_never_meets_a_reused_id(
        steps in proptest::collection::vec(step(), 1..16),
    ) {
        let lib = Lib::new();
        // The tables before each recorded action, and whether a send has
        // been recorded since.
        let mut before: Vec<(State, bool)> = Vec::new();
        for step in &steps {
            match step {
                Step::Do(action) => {
                    let state = lib.state();
                    if lib.apply(action) {
                        before.push((state, false));
                    }
                }
                Step::Send => {
                    lib.send();
                    for entry in &mut before {
                        entry.1 = true;
                    }
                }
            }
        }
        while let Some((state, sent_since)) = before.pop() {
            let now = lib.everything();
            // What the top bar would say is what Undo then does.
            let told = lib.next();
            prop_assert_eq!(lib.everything(), now.clone(), "asking what's next changed something");
            match lib.undo() {
                UndoOutcome::Undone { operation } => {
                    prop_assert_eq!(told.operation.map(|o| o.id), Some(operation.id));
                    prop_assert_eq!(told.refusal, None);
                    if !sent_since {
                        prop_assert_eq!(lib.state(), state);
                    }
                    prop_assert_eq!(lib.broken_references(), 0);
                }
                UndoOutcome::Refused { operation, reason, conflicts } => {
                    // Only an add of a track that was sent since is refused.
                    prop_assert!(sent_since, "{:?} refused with no send since", operation);
                    prop_assert!(
                        operation.kind == library::PROMOTE_OPERATION
                            || operation.kind == start::ADD_OPERATION,
                        "{:?} refused: {:?}", operation, conflicts
                    );
                    prop_assert_eq!(&reason, &UndoRefusal::SentSince);
                    prop_assert_eq!(told.refusal, Some(reason));
                    prop_assert!(
                        conflicts.iter().all(|c| c.problem != ConflictProblem::RowBack),
                        "a row id was reused: {:?}", conflicts
                    );
                    // Nothing changed, and asking again doesn't skip it.
                    prop_assert_eq!(lib.everything(), now.clone());
                    prop_assert_eq!(refusal(lib.undo()).0.id, operation.id);
                    prop_assert_eq!(lib.everything(), now);
                    break;
                }
                UndoOutcome::NothingToUndo => prop_assert!(false, "an operation went missing"),
            }
        }
    }
}

#[test]
fn three_actions_then_three_undos_restore_the_start() {
    let lib = Lib::new();
    let start = lib.state();
    let track = lib.add(1);
    let after_add = lib.state();
    let house = lib.create("House");
    let after_create = lib.state();
    assert_eq!(lib.add_to(house, &[track]).changed, 1);

    assert_eq!(lib.undone().kind, crates::ADD_OPERATION);
    assert_eq!(lib.state(), after_create);
    assert_eq!(lib.undone().kind, crates::CREATE_OPERATION);
    assert_eq!(lib.state(), after_add);
    assert_eq!(lib.undone().kind, library::PROMOTE_OPERATION);
    assert_eq!(lib.state(), start);
    assert_eq!(lib.undo(), UndoOutcome::NothingToUndo);
}

#[test]
fn undo_carries_on_through_the_history_after_the_app_is_restarted() {
    let lib = Lib::new();
    let start = lib.state();
    let track = lib.add(1);
    let house = lib.create("House");
    lib.add_to(house, &[track]);
    let after_create = {
        // One step is undone before the restart.
        lib.undone();
        lib.state()
    };

    // The app closes and opens the same database again.
    let Lib { dir, writer } = lib;
    drop(writer);
    let lib = Lib {
        writer: open(dir.path()),
        dir,
    };
    assert_eq!(lib.state(), after_create);
    assert_eq!(
        lib.next().operation.map(|o| o.kind).as_deref(),
        Some(crates::CREATE_OPERATION)
    );
    assert_eq!(lib.undone().kind, crates::CREATE_OPERATION);
    assert_eq!(lib.undone().kind, library::PROMOTE_OPERATION);
    assert_eq!(lib.state(), start);
    assert_eq!(lib.undo(), UndoOutcome::NothingToUndo);
}

#[test]
fn undoing_the_add_of_a_track_sent_since_is_refused_changes_nothing_and_blocks_the_older_steps() {
    let lib = Lib::new();
    let house = lib.create("House");
    // Track 4 isn't in rekordbox, so a send carries it as a new track.
    let track = lib.add(4);
    lib.add_to(house, &[track]);
    assert_eq!(lib.send().sent.len(), 1);

    // The crate step still undoes after the send.
    assert_eq!(lib.undone().kind, crates::ADD_OPERATION);
    // The add doesn't: the track has gone to rekordbox.
    let before = lib.everything();
    assert_eq!(lib.next().refusal, Some(UndoRefusal::SentSince));
    for _ in 0..2 {
        let (operation, reason) = refusal(lib.undo());
        assert_eq!(operation.kind, library::PROMOTE_OPERATION);
        assert_eq!(reason, UndoRefusal::SentSince);
        assert_eq!(lib.everything(), before, "a refused undo changed something");
    }
    // The crate made before it is out of reach: undo doesn't skip.
    assert_eq!(lib.crate_ids(), [house.0]);

    // A new action can be undone; then the refused step is next again.
    lib.create("Techno");
    assert_eq!(lib.undone().kind, crates::CREATE_OPERATION);
    assert_eq!(refusal(lib.undo()).1, UndoRefusal::SentSince);
    assert_eq!(lib.crate_ids(), [house.0]);
}

#[test]
fn undoing_an_add_from_rekordbox_is_refused_once_one_of_its_tracks_was_sent() {
    let lib = Lib::new();
    assert!(lib.apply(&Action::AddFromRekordbox));
    let house = lib.create("House");
    lib.add_to(house, &[lib.library_track(1)]);
    assert_eq!(lib.send().sent.len(), 1);
    lib.undone();
    lib.undone();

    let before = lib.everything();
    let (operation, reason) = refusal(lib.undo());
    assert_eq!(operation.kind, start::ADD_OPERATION);
    assert_eq!(reason, UndoRefusal::SentSince);
    assert_eq!(lib.everything(), before);
}

#[test]
fn an_action_that_changes_nothing_leaves_the_next_undo_as_it_was() {
    let lib = Lib::new();
    let track = lib.add(1);
    let outside = lib.add(2);
    let house = lib.create("House");
    lib.add_to(house, &[track]);
    let next = lib.next();
    assert_eq!(
        next.operation.as_ref().map(|o| o.kind.as_str()),
        Some(crates::ADD_OPERATION)
    );

    // Already in the crate; already called that; already in the Library;
    // nothing left to add from rekordbox... each records nothing.
    assert_eq!(lib.add_to(house, &[track]).operation_id, None);
    lib.rename(house, "House");
    assert!(!lib.apply(&Action::Add(0)));
    let taken_out = lib
        .writer
        .call(move |c| Ok(crates::remove_on(c, house, &[outside])))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(taken_out.operation_id, None);
    // A refused action too.
    assert!(!lib.apply(&Action::CreateCrate(1)));

    assert_eq!(lib.next(), next);
}

#[test]
fn a_removed_track_comes_back_exactly_when_undone_after_a_send_and_goes_out_as_rekordboxs_own() {
    let lib = Lib::new();
    let house = lib.create("House");
    // Track 1 is in rekordbox; track 4 is not.
    let known = lib.add(1);
    lib.add_to(house, &[known]);
    let first = lib.send();
    let sent_before = first
        .sent
        .iter()
        .find(|t| t.library_track == known)
        .expect("the known track was sent")
        .clone();
    assert!(sent_before.in_rekordbox);
    // A conflict on it, waiting in the review queue.
    lib.writer
        .call(move |c| {
            c.execute(
                "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value,
                                       base_value)
                 VALUES (?1, 'Rating', '204', '153', '0')",
                [known.0],
            )
        })
        .unwrap();
    // A track that was never sent joins the crate.
    let new = lib.add(4);
    lib.add_to(house, &[new]);
    let before = lib.rows_of(known);
    assert!(before.iter().all(|table| !table.is_empty()), "{before:?}");

    library::remove(&lib.writer, known).unwrap();
    // The send makes bases for the new track. They must not take the ids
    // the removed track's bases had.
    let second = lib.send();
    assert_eq!(
        second
            .sent
            .iter()
            .map(|t| t.library_track)
            .collect::<Vec<_>>(),
        [new]
    );

    // The removal is undone, although a send happened since.
    assert_eq!(lib.next().refusal, None);
    assert_eq!(lib.undone().kind, library::REMOVE_OPERATION);
    assert_eq!(lib.rows_of(known), before);
    let removals: i64 = lib
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM library_removal", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(removals, 0);
    assert_eq!(lib.broken_references(), 0);

    // The next send carries it as the track rekordbox has, value for value,
    // not as a new one.
    let third = lib.build_send();
    let sent_after = third
        .sent
        .iter()
        .find(|t| t.library_track == known)
        .expect("the restored track is sent");
    assert!(sent_after.in_rekordbox);
    assert_eq!(sent_after.attributes, sent_before.attributes);
}

#[test]
fn undo_through_a_reused_crate_name_never_clashes_because_the_later_crate_goes_first() {
    let lib = Lib::new();
    let start = lib.state();
    let first = lib.create("House");
    lib.rename(first, "Techno");
    lib.create("House");

    lib.undone();
    lib.undone();
    lib.undone();
    assert_eq!(lib.state(), start);
}

#[test]
fn undoing_a_rename_is_refused_with_its_own_reason_when_another_crate_has_the_old_name_now() {
    for planted in ["House", "house ", "HOUSE"] {
        let lib = Lib::new();
        let first = lib.create("House");
        lib.rename(first, "Techno");
        // Not through the log: no command makes this, but nothing in the
        // database stops a later one from doing so.
        lib.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO crate (kind, name) VALUES ('static', ?1)",
                    [planted],
                )
            })
            .unwrap();
        let before = lib.everything();

        let taken = UndoRefusal::CrateNameTaken {
            name: "House".to_owned(),
        };
        assert_eq!(lib.next().refusal, Some(taken.clone()), "{planted:?}");
        let (operation, reason) = refusal(lib.undo());
        assert_eq!(operation.kind, crates::RENAME_OPERATION);
        assert_eq!(reason, taken, "{planted:?}");
        assert_eq!(lib.everything(), before, "{planted:?}");
    }
}

#[test]
fn undoing_a_crate_delete_is_refused_with_its_own_reason_when_another_crate_has_its_name_now() {
    for planted in ["House", "house"] {
        let lib = Lib::new();
        let first = lib.create("House");
        lib.writer
            .call(move |c| Ok(crates::delete_on(c, first)))
            .unwrap()
            .unwrap()
            .unwrap();
        lib.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO crate (id, kind, name) VALUES (50, 'static', ?1)",
                    [planted],
                )
            })
            .unwrap();
        let before = lib.everything();

        let (operation, reason) = refusal(lib.undo());
        assert_eq!(operation.kind, crates::DELETE_OPERATION);
        assert_eq!(
            reason,
            UndoRefusal::CrateNameTaken {
                name: "House".to_owned()
            },
            "{planted:?}"
        );
        assert_eq!(lib.everything(), before, "{planted:?}");
    }
}

#[test]
fn the_next_undo_names_the_action_with_what_the_ui_needs_to_say_it() {
    let lib = Lib::new();
    assert_eq!(
        lib.next(),
        NextUndo {
            operation: None,
            refusal: None
        }
    );
    let details = |lib: &Lib| {
        let operation = lib.next().operation.expect("something to undo");
        (operation.kind, operation.details)
    };
    let named = |name: &str, from: Option<&str>, tracks: Option<u32>| OperationDetails {
        name: Some(name.to_owned()),
        from: from.map(str::to_owned),
        tracks,
    };

    assert!(lib.apply(&Action::AddFromRekordbox));
    assert_eq!(
        details(&lib),
        (
            start::ADD_OPERATION.to_owned(),
            OperationDetails {
                tracks: Some(IN_REKORDBOX as u32),
                ..OperationDetails::default()
            }
        )
    );
    let house = lib.create("House");
    assert_eq!(
        details(&lib),
        (
            crates::CREATE_OPERATION.to_owned(),
            named("House", None, None)
        )
    );
    lib.rename(house, "Techno");
    assert_eq!(
        details(&lib),
        (
            crates::RENAME_OPERATION.to_owned(),
            named("Techno", Some("House"), None)
        )
    );
    let tracks = [lib.library_track(1), lib.library_track(2)];
    lib.add_to(house, &tracks);
    assert_eq!(
        details(&lib),
        (
            crates::ADD_OPERATION.to_owned(),
            named("Techno", None, Some(2))
        )
    );
    // One of the two isn't in the crate any more: one is removed.
    assert!(lib.apply(&Action::RemoveFromCrate(0, vec![0])));
    assert!(lib.apply(&Action::RemoveFromCrate(0, vec![0, 1])));
    assert_eq!(
        details(&lib),
        (
            crates::REMOVE_OPERATION.to_owned(),
            named("Techno", None, Some(1))
        )
    );
    assert!(lib.apply(&Action::DeleteCrate(0)));
    assert_eq!(
        details(&lib),
        (
            crates::DELETE_OPERATION.to_owned(),
            named("Techno", None, None)
        )
    );
    assert!(lib.apply(&Action::Remove(0)));
    assert_eq!(
        details(&lib),
        (
            library::REMOVE_OPERATION.to_owned(),
            OperationDetails::default()
        )
    );
}

#[test]
fn asking_what_undo_would_do_changes_nothing_in_the_tables_or_the_log() {
    let lib = Lib::new();
    let house = lib.create("House");
    let track = lib.add(4);
    lib.add_to(house, &[track]);
    // Once with an undo that would work...
    let before = lib.everything();
    assert_eq!(lib.next().refusal, None);
    assert_eq!(lib.everything(), before);
    // ...and once with one that would be refused part-way through.
    lib.send();
    lib.undone();
    let before = lib.everything();
    for _ in 0..3 {
        assert_eq!(lib.next().refusal, Some(UndoRefusal::SentSince));
    }
    assert_eq!(lib.everything(), before);
    // The answer is what Undo then does.
    assert_eq!(refusal(lib.undo()).1, UndoRefusal::SentSince);
}

#[test]
fn a_very_large_operation_is_named_but_not_tried_ahead_of_time() {
    let lib = Lib::new();
    // More change rows than are tried ahead: many crates in one operation.
    let crates_made = 400;
    let operation = super::record_via(
        &lib.writer,
        "create_crate",
        serde_json::json!({}),
        move |rec| {
            for n in 0..crates_made {
                rec.insert(
                    "crate",
                    &[
                        ("kind", Value::Text("static".into())),
                        ("name", Value::Text(format!("Crate {n}"))),
                    ],
                )?;
            }
            Ok(())
        },
    )
    .unwrap()
    .operation_id
    .unwrap();
    let rows: i64 = lib
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM change", [], |r| r.get(0)))
        .unwrap();
    assert!(rows > TRIED_AHEAD_UP_TO, "{rows}");
    // Something that will get the undo refused.
    lib.run("UPDATE crate SET name = 'Renamed by hand' WHERE name = 'Crate 7'");

    let next = lib.next();
    assert_eq!(next.operation.map(|o| o.id), Some(operation));
    assert_eq!(next.refusal, None, "the refusal is found when Undo is used");
    assert_eq!(refusal(lib.undo()).1, UndoRefusal::ChangedSince);
}

/// Local only: how long asking "what would Undo do" takes for the largest
/// operation that is tried ahead of time. Prints; asserts nothing about time.
#[test]
#[ignore = "a measurement, run by hand"]
fn measure_asking_what_undo_would_do_for_the_largest_operation_tried_ahead() {
    let lib = Lib::new();
    super::record_via(&lib.writer, "create_crate", serde_json::json!({}), |rec| {
        // 7 recorded fields per crate.
        for n in 0..(TRIED_AHEAD_UP_TO / 7) {
            rec.insert(
                "crate",
                &[
                    ("kind", Value::Text("static".into())),
                    ("name", Value::Text(format!("Crate {n}"))),
                ],
            )?;
        }
        Ok(())
    })
    .unwrap();
    for _ in 0..5 {
        let started = std::time::Instant::now();
        assert_eq!(lib.next().refusal, None);
        println!("asked in {:?}", started.elapsed());
    }
}

#[test]
fn an_undo_offered_for_one_action_does_nothing_once_that_action_is_not_the_next_to_undo() {
    let lib = Lib::new();
    let track = lib.add(1);
    let house = lib.create("House");
    let added = lib.add_to(house, &[track]).operation_id.unwrap();
    // The top bar undoes it first.
    lib.undone();
    let before = lib.everything();
    let outcome = lib
        .writer
        .call(move |c| Ok(undo_only(c, added)))
        .unwrap()
        .unwrap();
    assert_eq!(outcome, UndoOutcome::NothingToUndo);
    assert_eq!(lib.everything(), before);
}

// --- Row ids the log relies on -------------------------------------------

/// Every `INSERT INTO <table>` in `source`, for the tables in `tables`.
fn inserts_into<'a>(source: &str, tables: &[&'a str]) -> Vec<&'a str> {
    let flat: String = source
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .replace("main.", "")
        .replace('"', "");
    tables
        .iter()
        .copied()
        .filter(|table| {
            ["", "or replace ", "or ignore "].iter().any(|or| {
                flat.match_indices(&format!("insert {or}into {table}"))
                    .any(|(at, found)| {
                        // `crate` mustn't match `crate_entry`.
                        !flat[at + found.len()..]
                            .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                    })
            })
        })
        .collect()
}

/// The app's own code, without its tests: every `.rs` file under `src`
/// except the modules declared under `#[cfg(test)]` (a file or a folder),
/// each file cut at its inline `#[cfg(test)] mod tests {`.
fn app_sources() -> Vec<(PathBuf, String)> {
    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut all = Vec::new();
    files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut all);

    // `#[cfg(test)]` then `mod name;` puts `name.rs` or `name/` out of the
    // app: beside a `mod.rs`, `lib.rs` or `main.rs`, else in the folder
    // named after the declaring file.
    let mut test_only: Vec<PathBuf> = Vec::new();
    let mut texts = Vec::new();
    for path in all {
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\r\n", "\n");
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let home = if ["mod", "lib", "main"].contains(&stem.as_str()) {
            path.parent().unwrap().to_path_buf()
        } else {
            path.with_extension("")
        };
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        for pair in lines.windows(2) {
            let declared = pair[1]
                .trim_start_matches("pub(super) ")
                .trim_start_matches("pub(crate) ")
                .trim_start_matches("pub ");
            if pair[0] != "#[cfg(test)]" {
                continue;
            }
            if let Some(name) = declared
                .strip_prefix("mod ")
                .and_then(|rest| rest.strip_suffix(';'))
            {
                test_only.push(home.join(format!("{name}.rs")));
                test_only.push(home.join(name));
            }
        }
        texts.push((path, text));
    }
    texts
        .into_iter()
        .filter(|(path, _)| !test_only.iter().any(|skip| path.starts_with(skip)))
        .map(|(path, text)| {
            let code = text
                .split("#[cfg(test)]\nmod tests {")
                .next()
                .unwrap()
                .to_owned();
            (path, code)
        })
        .collect()
}

#[test]
fn the_source_scan_reads_the_apps_code_and_leaves_its_tests_out() {
    let sources = app_sources();
    let has = |end: &str| {
        sources
            .iter()
            .any(|(path, _)| path.to_string_lossy().replace('\\', "/").ends_with(end))
    };
    assert!(has("src/rekordbox_write/record.rs"));
    assert!(has("src/ops/mod.rs"));
    assert!(has("src/library/mod.rs"));
    for tests in [
        "src/ops/tests.rs",
        "src/ops/history_tests.rs",
        "src/rekordbox_write/record_tests.rs",
        "src/db/migrations/schema/library.rs",
        "src/relink/tests/send_after_gone.rs",
    ] {
        assert!(!has(tests), "{tests} was read as app code");
    }
}

#[test]
fn the_insert_scan_finds_inserts_however_they_are_written_and_tells_table_names_apart() {
    let tables = ["crate", "crate_entry", "sync_base"];
    assert_eq!(
        inserts_into("INSERT INTO sync_base (a) VALUES (1)", &tables),
        ["sync_base"]
    );
    assert_eq!(
        inserts_into(
            "insert  or replace\n into main.\"crate_entry\" (a)",
            &tables
        ),
        ["crate_entry"]
    );
    assert_eq!(inserts_into("INSERT INTO crate(kind)", &tables), ["crate"]);
    assert_eq!(
        inserts_into(
            "SELECT * FROM crate; INSERT INTO sent_playlist (path)",
            &tables
        ),
        Vec::<&str>::new()
    );
}

#[test]
fn code_outside_the_log_only_inserts_into_logged_tables_whose_ids_are_never_reused() {
    // A send isn't an operation, and it inserts sync bases. If such a table
    // reused ids, a new row could take the id of a row an operation
    // deleted, and undoing that operation would be refused for no reason
    // the user could see. So: AUTOINCREMENT, or go through the log.
    let lib = Lib::new();
    let never_reused: Vec<&str> = lib
        .writer
        .call(|c| {
            let mut found = Vec::new();
            for table in LOGGED_TABLES {
                let sql: String = c.query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |r| r.get(0),
                )?;
                if sql.to_uppercase().contains("AUTOINCREMENT") {
                    found.push(table);
                }
            }
            Ok(found)
        })
        .unwrap();
    assert_eq!(never_reused, ["sync_base"]);

    let sources = app_sources();
    assert!(
        sources.len() > 50,
        "only {} source files found",
        sources.len()
    );
    let mut seen = Vec::new();
    for (path, code) in &sources {
        for table in inserts_into(code, &LOGGED_TABLES) {
            seen.push(table);
            assert!(
                never_reused.contains(&table),
                "{} inserts into `{table}` outside the operation log, and `{table}` reuses row \
                 ids. Make the insert through `ops::record`, or give the table AUTOINCREMENT in \
                 a migration.",
                path.display()
            );
        }
    }
    // The scan does see the send's insert, so it isn't passing by blindness.
    assert_eq!(seen, ["sync_base"]);
}

#[test]
fn every_table_the_app_records_changes_to_is_in_the_list_these_tests_check() {
    // `rec.insert("table", …)`, `rec.delete("table", …)`, `rec.set("table", …)`.
    for (path, code) in app_sources() {
        for call in ["rec.insert(", "rec.delete(", "rec.set("] {
            for (at, _) in code.match_indices(call) {
                let rest = code[at + call.len()..].trim_start();
                let Some(name) = rest.strip_prefix('"').and_then(|r| r.split('"').next()) else {
                    continue; // A variable: `library::remove_on` loops over a list.
                };
                assert!(
                    LOGGED_TABLES.contains(&name),
                    "{} records changes to `{name}`; add it to LOGGED_TABLES",
                    path.display()
                );
            }
        }
    }
}
