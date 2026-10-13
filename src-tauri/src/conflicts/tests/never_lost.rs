//! No sequence of edits, reads, sends, resolves and undos loses a value.
//!
//! The test plays both users (edits in the app, edits in rekordbox), the
//! pieces the engine doesn't have yet (a store that takes rekordbox's value
//! or the value chosen, a send, an import that happens or doesn't), and an
//! oracle that never asks [`decide`]. The oracle keeps its own account of
//! the sides and checks the moments a value is overwritten:
//!
//! - **The app's value is overwritten by rekordbox's** (the store acts on
//!   "take rekordbox's"). Allowed only if rekordbox has the same value, or
//!   a conflict row holds the app's value, or the app's value was sent and
//!   rekordbox isn't known to still show what it replaced (ROADMAP 1.10's
//!   "only rekordbox changed").
//! - **rekordbox's value is overwritten by an import.** Allowed only if the
//!   value written is the same, or rekordbox's value is the one last sent
//!   or the one the app's edit is known to have replaced, or a conflict row
//!   holds it.
//! - **The app's value is overwritten by a value the user chose.** Allowed
//!   only if a conflict row holds it.
//!
//! And between those moments:
//!
//! - When each side holds a value the other never had (not the value last
//!   sent, not the value the app's edit replaced), a conflict is open with
//!   exactly the three values as they are, or a decision made on them
//!   answers.
//! - A decision answers its field for as long as no step has changed the
//!   app's, rekordbox's or the base's text since (other than the app taking
//!   the value chosen), by the test's own record of the steps.
//! - A detect never changes a settled conflict, writes nothing for a track
//!   without a current rekordbox entry, and writes nothing when run again.
//! - A resolve keeps all three values; an undo brings the open conflicts
//!   back whole or changes nothing.
//!
//! Values are compared with [`same`] (the comparison per kind has its own
//! tests); nothing else of the engine is trusted.
//!
//! The cases are the same on every run: the generator's seed is fixed and
//! no regressions file is read or written, so a fault is caught on every
//! run or on none.

use proptest::prelude::*;
use proptest::test_runner::{Config, TestCaseError, TestRng, TestRunner};

use super::*;

const FIELDS: [&str; 2] = ["Genre", "Rating"];

/// How many sequences are played, each up to [`STEPS`] steps long.
const CASES: u32 = 320;
const STEPS: usize = 60;

/// A value a side of a field can take. Some differ only in spelling, some
/// are blank, so sides often agree.
fn value_of(field: usize, n: usize) -> &'static str {
    let values: [&str; 6] = match field {
        0 => ["", " ", "House", "house", "Deep House", "Tech House"],
        _ => ["", "0", "51", "153", "255", "0255"],
    };
    values[n % values.len()]
}

#[derive(Debug, Clone)]
enum Step {
    /// The user gives a field a value in the app. `knows`: the store
    /// records the value rekordbox showed, when this is the app's first
    /// own value for the field.
    AppEdit {
        field: usize,
        value: usize,
        knows: bool,
    },
    /// The user gives up the app's own value: the field follows rekordbox.
    AppDrop {
        field: usize,
    },
    /// The field changes in rekordbox; `None`: the attribute goes away.
    RekordboxEdit {
        field: usize,
        value: Option<usize>,
    },
    /// rekordbox's entry for the track: current, kept from an older read,
    /// or gone.
    Entry {
        state: usize,
    },
    Detect,
    /// A read, the merge and a send, in one step; the XML is imported in
    /// rekordbox, or isn't.
    Send {
        imported: bool,
    },
    /// One open conflict is settled; `adopt`: the store takes the value
    /// chosen (`knows`: and records which value of rekordbox's it replaced).
    Resolve {
        pick: usize,
        choice: usize,
        typed: usize,
        adopt: bool,
        knows: bool,
    },
    /// Every open conflict is settled the same way.
    ResolveAll {
        rekordbox: bool,
        adopt: bool,
    },
    Undo,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        5 => (0..2usize, 0..6usize, any::<bool>())
            .prop_map(|(field, value, knows)| Step::AppEdit { field, value, knows }),
        1 => (0..2usize).prop_map(|field| Step::AppDrop { field }),
        5 => (0..2usize, proptest::option::weighted(0.9, 0..6usize))
            .prop_map(|(field, value)| Step::RekordboxEdit { field, value }),
        2 => (0..5usize).prop_map(|state| Step::Entry { state }),
        5 => Just(Step::Detect),
        3 => any::<bool>().prop_map(|imported| Step::Send { imported }),
        4 => (0..4usize, 0..6usize, 0..6usize, any::<bool>(), any::<bool>()).prop_map(
            |(pick, choice, typed, adopt, knows)| Step::Resolve {
                pick,
                choice,
                typed,
                adopt,
                knows,
            }
        ),
        1 => (any::<bool>(), any::<bool>())
            .prop_map(|(rekordbox, adopt)| Step::ResolveAll { rekordbox, adopt }),
        2 => Just(Step::Undo),
    ]
}

/// A decision that, by the steps played since, no side has moved from.
#[derive(Debug, Clone, PartialEq)]
struct Standing {
    conflict: ConflictId,
    value: String,
}

/// The oracle's account of one field.
#[derive(Debug, Clone, Default)]
struct Account {
    /// The app's own value; `None`: it follows rekordbox.
    app: Option<String>,
    /// What the store says the app's value replaced.
    replaced: Option<String>,
    /// rekordbox's attribute; `None`: it has none.
    rekordbox: Option<String>,
    /// The value last sent.
    base: Option<String>,
    standing: Option<Standing>,
}

impl Account {
    fn rekordbox_text(&self) -> &str {
        self.rekordbox.as_deref().unwrap_or("")
    }
}

/// The store took the value chosen for a field: what it held before.
#[derive(Debug, Clone)]
struct Adopted {
    field: usize,
    app: Option<String>,
    replaced: Option<String>,
    chosen: String,
}

/// One logged resolve: the open conflicts it settled and what the store
/// took with it.
#[derive(Debug, Clone)]
struct Done {
    reopened: Vec<Conflict>,
    adopted: Vec<Adopted>,
}

struct World<'a> {
    lib: &'a Lib,
    id: LibraryTrackId,
    entry: RekordboxEntry,
    fields: [Account; 2],
    /// The resolves that were logged and not undone, newest last.
    resolves: Vec<Done>,
}

fn fail<T>(why: String) -> Result<T, TestCaseError> {
    Err(TestCaseError::fail(why))
}

impl<'a> World<'a> {
    /// A fresh start on the same database: no conflict, nothing to undo.
    fn new(lib: &'a Lib, id: LibraryTrackId) -> World<'a> {
        lib.run("DELETE FROM change; DELETE FROM operation; DELETE FROM conflict;");
        World {
            lib,
            id,
            entry: RekordboxEntry::Current,
            fields: [Account::default(), Account::default()],
            resolves: Vec::new(),
        }
    }

    fn sides(&self) -> TrackSides {
        TrackSides {
            library_track: self.id,
            entry: self.entry,
            fields: FIELDS
                .iter()
                .zip(&self.fields)
                .map(|(name, account)| FieldSides {
                    field: (*name).to_owned(),
                    app: account.app.clone(),
                    rekordbox: account.rekordbox.clone(),
                    base: account.base.clone(),
                    replaced: account.replaced.clone(),
                })
                .collect(),
        }
    }

    // ---- What the oracle knows about a field ------------------------------

    /// The app holds a value rekordbox doesn't have.
    fn app_differs(&self, i: usize) -> bool {
        let f = &self.fields[i];
        f.app.is_some() && !same(FIELDS[i], f.app.as_deref(), f.rekordbox.as_deref())
    }

    /// The app's value is the one last sent.
    fn app_was_sent(&self, i: usize) -> bool {
        let f = &self.fields[i];
        f.app.is_some() && f.base.is_some() && same(FIELDS[i], f.app.as_deref(), f.base.as_deref())
    }

    /// rekordbox shows the value the app's edit is known to have replaced.
    fn still_shows_replaced(&self, i: usize) -> bool {
        let f = &self.fields[i];
        f.replaced.is_some() && same(FIELDS[i], f.replaced.as_deref(), f.rekordbox.as_deref())
    }

    /// rekordbox holds a value of its own: not the app's, not the one last
    /// sent, not the one the app's edit replaced.
    fn rekordbox_is_new(&self, i: usize) -> bool {
        let f = &self.fields[i];
        let at_base =
            f.base.is_some() && same(FIELDS[i], f.rekordbox.as_deref(), f.base.as_deref());
        self.app_differs(i) && !at_base && !self.still_shows_replaced(i)
    }

    /// Whether a conflict row of the field holds `value` on the app's side
    /// (`app`) or on rekordbox's.
    fn held(&self, i: usize, app: bool, value: &str) -> bool {
        self.lib.rows().iter().any(|row| {
            let side = if app {
                &row.app_value
            } else {
                &row.rekordbox_value
            };
            row.field == FIELDS[i] && same(FIELDS[i], side.as_deref(), Some(value))
        })
    }

    /// Whether an open conflict's values are the field's values right now.
    fn is_current(&self, row: &Conflict) -> bool {
        let Some(i) = FIELDS.iter().position(|name| *name == row.field) else {
            return false;
        };
        let f = &self.fields[i];
        values(row)
            == (
                f.app.as_deref(),
                Some(f.rekordbox_text()),
                f.base.as_deref(),
            )
    }

    // ---- The steps --------------------------------------------------------

    fn apply(&mut self, step: &Step) -> Result<(), TestCaseError> {
        match *step {
            Step::AppEdit {
                field,
                value,
                knows,
            } => {
                let value = value_of(field, value);
                let f = &mut self.fields[field];
                if f.app.as_deref() != Some(value) {
                    f.standing = None;
                }
                if f.app.is_none() {
                    f.replaced = knows.then(|| f.rekordbox_text().to_owned());
                }
                f.app = Some(value.to_owned());
                Ok(())
            }
            Step::AppDrop { field } => {
                let f = &mut self.fields[field];
                f.app = None;
                f.replaced = None;
                f.standing = None;
                Ok(())
            }
            Step::RekordboxEdit { field, value } => {
                let value = value.map(|n| value_of(field, n).to_owned());
                let f = &mut self.fields[field];
                if f.rekordbox != value {
                    f.standing = None;
                }
                f.rekordbox = value;
                Ok(())
            }
            Step::Entry { state } => {
                self.entry = match state {
                    3 => RekordboxEntry::Stale,
                    4 => RekordboxEntry::NotThere,
                    _ => RekordboxEntry::Current,
                };
                Ok(())
            }
            Step::Detect => self.detect().map(|_| ()),
            Step::Send { imported } => self.send(imported),
            Step::Resolve {
                pick,
                choice,
                typed,
                adopt,
                knows,
            } => {
                let open = self.lib.open(&Filter::default());
                if open.is_empty() {
                    return Ok(());
                }
                let target = open[pick % open.len()].clone();
                let i = FIELDS.iter().position(|n| *n == target.field).unwrap();
                let text = |value: &Option<String>| value.clone().unwrap_or_default();
                let choice = match choice {
                    0 => Choice::KeepApp,
                    1 => Choice::KeepRekordbox,
                    2 => Choice::Edit(text(&target.app_value)),
                    3 => Choice::Edit(text(&target.rekordbox_value)),
                    4 if target.base_value.is_some() => Choice::Edit(text(&target.base_value)),
                    _ => Choice::Edit(value_of(i, typed).to_owned()),
                };
                let resolved = self
                    .lib
                    .resolve(target.id, choice)
                    .map_err(|e| TestCaseError::fail(e.to_string()))?
                    .value;
                self.kept(&target, &resolved)?;
                let adopted = self.decided(&target, &resolved, adopt, knows)?;
                self.resolves.push(Done {
                    reopened: vec![target],
                    adopted: adopted.into_iter().collect(),
                });
                Ok(())
            }
            Step::ResolveAll { rekordbox, adopt } => {
                let open = self.lib.open(&Filter::default());
                let choice = if rekordbox {
                    Choice::KeepRekordbox
                } else {
                    Choice::KeepApp
                };
                let resolved = self
                    .lib
                    .resolve_all(&Filter::default(), choice)
                    .map_err(|e| TestCaseError::fail(e.to_string()))?
                    .value;
                prop_assert_eq!(resolved.len(), open.len());
                let mut adopted = Vec::new();
                for (target, resolved) in open.iter().zip(&resolved) {
                    self.kept(target, resolved)?;
                    adopted.extend(self.decided(target, resolved, adopt, true)?);
                }
                prop_assert!(self.lib.open(&Filter::default()).is_empty());
                if !open.is_empty() {
                    self.resolves.push(Done {
                        reopened: open,
                        adopted,
                    });
                }
                Ok(())
            }
            Step::Undo => self.undo(),
        }
    }

    /// A resolve replaced the open conflict by a settled one that still
    /// holds all three values.
    fn kept(&self, was: &Conflict, resolved: &Resolved) -> Result<(), TestCaseError> {
        let rows = self.lib.rows();
        prop_assert!(rows.iter().all(|c| c.id != was.id));
        let Some(settled) = rows.iter().find(|c| c.id == resolved.conflict) else {
            return fail(format!("no settled row for {resolved:?}"));
        };
        prop_assert_eq!(values(settled), values(was));
        prop_assert_eq!(&settled.field, &was.field);
        prop_assert_eq!(&settled.detected_at, &was.detected_at);
        prop_assert_ne!(settled.status, Status::Open);
        prop_assert_eq!(settled.chosen(), Some(resolved.value.as_str()));
        Ok(())
    }

    /// The oracle's side of a resolve. A decision made on the values as
    /// they are now is one no side has moved from; the store then takes
    /// the value chosen, if the step says so.
    fn decided(
        &mut self,
        was: &Conflict,
        resolved: &Resolved,
        adopt: bool,
        knows: bool,
    ) -> Result<Option<Adopted>, TestCaseError> {
        let i = FIELDS.iter().position(|n| *n == was.field).unwrap();
        if !self.is_current(was) {
            // Settled on values a side has left since: nothing to hold it to.
            self.fields[i].standing = None;
            return Ok(None);
        }
        self.fields[i].standing = Some(Standing {
            conflict: resolved.conflict,
            value: resolved.value.clone(),
        });
        if !adopt {
            return Ok(None);
        }
        // The value chosen overwrites the app's: the settled row holds it.
        let before = self.fields[i].clone();
        if let Some(app) = &before.app {
            if !same(FIELDS[i], Some(app), Some(&resolved.value)) {
                prop_assert!(
                    self.held(i, true, app),
                    "{}: the app's {:?} is overwritten by the value chosen and no row holds it",
                    FIELDS[i],
                    app
                );
            }
        }
        let f = &mut self.fields[i];
        f.app = Some(resolved.value.clone());
        if knows {
            f.replaced = Some(before.rekordbox_text().to_owned());
        }
        Ok(Some(Adopted {
            field: i,
            app: before.app,
            replaced: before.replaced,
            chosen: resolved.value.clone(),
        }))
    }

    fn undo(&mut self) -> Result<(), TestCaseError> {
        // The store's part of a resolve is undone with it, and that is
        // refused once the app's value has changed again: not played.
        if let Some(done) = self.resolves.last() {
            let changed_since = done
                .adopted
                .iter()
                .any(|a| self.fields[a.field].app.as_deref() != Some(a.chosen.as_str()));
            if changed_since {
                return Ok(());
            }
        }
        let before = self.lib.rows();
        match self.lib.undo() {
            UndoOutcome::Undone { .. } => {
                let Some(done) = self.resolves.pop() else {
                    return fail("an undo with nothing resolved".to_owned());
                };
                let rows = self.lib.rows();
                for open in &done.reopened {
                    prop_assert!(rows.contains(open), "{:?} is not back", open);
                    let i = FIELDS.iter().position(|n| *n == open.field).unwrap();
                    self.fields[i].standing = None;
                }
                for adopted in done.adopted {
                    let f = &mut self.fields[adopted.field];
                    f.app = adopted.app;
                    f.replaced = adopted.replaced;
                }
            }
            UndoOutcome::Refused { .. } => {
                prop_assert!(!self.resolves.is_empty());
                prop_assert_eq!(self.lib.rows(), before);
            }
            UndoOutcome::NothingToUndo => {
                prop_assert!(self.resolves.is_empty());
                prop_assert_eq!(self.lib.rows(), before);
            }
        }
        Ok(())
    }

    /// A read and the merge: detect, check what it did and said, and let
    /// the store take rekordbox's value where it says so.
    fn detect(&mut self) -> Result<Detected, TestCaseError> {
        let sides = self.sides();
        let before = self.lib.rows();
        let detected = self.lib.detect(std::slice::from_ref(&sides));
        let rows = self.lib.rows();

        // A detect never changes or deletes a settled conflict, and writes
        // nothing at all for a track without a current rekordbox entry.
        let settled = |rows: &[Conflict]| -> Vec<Conflict> {
            rows.iter()
                .filter(|c| c.status != Status::Open)
                .cloned()
                .collect()
        };
        prop_assert_eq!(settled(&rows), settled(&before));
        if self.entry != RekordboxEntry::Current {
            prop_assert_eq!(&rows, &before);
        }
        // And detecting again changes nothing.
        let again = self.lib.detect(std::slice::from_ref(&sides));
        prop_assert_eq!(&self.lib.rows(), &rows);
        prop_assert_eq!(&again.fields, &detected.fields);
        prop_assert!(again.opened.is_empty() && again.updated.is_empty());
        prop_assert!(again.withdrawn.is_empty());
        if self.entry != RekordboxEntry::Current {
            return Ok(detected);
        }

        for (i, name) in FIELDS.iter().enumerate() {
            let outcome = detected
                .fields
                .iter()
                .find(|o| o.field == *name)
                .map(|o| o.outcome.clone());

            // What an answer points at is what it says.
            match &outcome {
                Some(Outcome::Open { conflict }) => {
                    let Some(row) = rows.iter().find(|c| c.id == *conflict) else {
                        return fail(format!("{name}: open, but the row is gone"));
                    };
                    prop_assert_eq!(row.status, Status::Open);
                    prop_assert!(self.is_current(row), "{}: {:?} is out of date", name, row);
                }
                Some(Outcome::Decided { conflict, value }) => {
                    let Some(row) = rows.iter().find(|c| c.id == *conflict) else {
                        return fail(format!("{name}: decided by a row that's gone"));
                    };
                    let f = &self.fields[i];
                    prop_assert_eq!(row.chosen(), Some(value.as_str()));
                    prop_assert!(
                        same(name, row.rekordbox_value.as_deref(), f.rekordbox.as_deref()),
                        "{}: decided on {:?}, but rekordbox has {:?}",
                        name,
                        row.rekordbox_value,
                        f.rekordbox
                    );
                    prop_assert!(
                        same(name, row.app_value.as_deref(), f.app.as_deref())
                            || same(name, row.chosen(), f.app.as_deref()),
                        "{}: decided on {:?}, but the app has {:?}",
                        name,
                        row,
                        f.app
                    );
                }
                _ => {}
            }

            // A decision no side has moved from answers the field.
            if let Some(standing) = &self.fields[i].standing {
                prop_assert_eq!(
                    &outcome,
                    &Some(Outcome::Decided {
                        conflict: standing.conflict,
                        value: standing.value.clone(),
                    }),
                    "{}: the decision was not honoured",
                    name
                );
            }

            // The store takes rekordbox's value: the app's is overwritten.
            if outcome == Some(Outcome::TakeRekordbox) {
                let app = self.fields[i].app.clone().unwrap_or_default();
                let allowed = !self.app_differs(i)
                    || self.held(i, true, &app)
                    || (self.app_was_sent(i) && !self.still_shows_replaced(i));
                prop_assert!(
                    allowed,
                    "{}: the app's {:?} is overwritten by rekordbox's {:?} (base {:?}) and no \
                     conflict row holds it",
                    name,
                    app,
                    self.fields[i].rekordbox,
                    self.fields[i].base
                );
                let f = &mut self.fields[i];
                f.app = None;
                f.replaced = None;
                f.standing = None;
            }

            // Each side holds a value the other never had: a conflict
            // holds both, or a decision made on them answers.
            if self.app_differs(i) && !self.app_was_sent(i) && self.rekordbox_is_new(i) {
                prop_assert!(
                    matches!(
                        outcome,
                        Some(Outcome::Open { .. } | Outcome::Decided { .. })
                    ),
                    "{}: both sides changed ({:?}), but the answer is {:?}",
                    name,
                    self.fields[i],
                    outcome
                );
            }
        }
        Ok(detected)
    }

    /// A send: read and merge first, then every field goes out with the
    /// value the merge gave it (a field with an open conflict is held
    /// back), and the import overwrites rekordbox's values, or never runs.
    fn send(&mut self, imported: bool) -> Result<(), TestCaseError> {
        if self.entry != RekordboxEntry::Current {
            // No send from an old read; a track that is new to rekordbox
            // has nothing to merge.
            return Ok(());
        }
        let detected = self.detect()?;
        for (i, name) in FIELDS.iter().enumerate() {
            let outcome = detected
                .fields
                .iter()
                .find(|o| o.field == *name)
                .map(|o| o.outcome.clone());
            let was = self.fields[i].clone();
            let rekordbox_was_new = self.rekordbox_is_new(i);
            let sent: Option<String> = match &outcome {
                Some(Outcome::Open { .. } | Outcome::Wait) => continue,
                Some(Outcome::Decided { value, .. }) => {
                    // The value chosen goes out, and is the app's from now.
                    if let Some(app) = &was.app {
                        if !same(name, Some(app), Some(value)) {
                            prop_assert!(
                                self.held(i, true, app),
                                "{}: the app's {:?} goes for the value chosen and no row holds it",
                                name,
                                app
                            );
                        }
                    }
                    self.fields[i].app = Some(value.clone());
                    Some(value.clone())
                }
                // The app's own value, or rekordbox's back when it follows.
                _ => was.app.clone().or_else(|| was.rekordbox.clone()),
            };
            if self.fields[i].base != sent {
                self.fields[i].standing = None;
            }
            self.fields[i].base.clone_from(&sent);
            let (Some(value), true) = (sent, imported) else {
                continue;
            };
            if !same(name, Some(&value), was.rekordbox.as_deref()) {
                let allowed = !rekordbox_was_new || self.held(i, false, was.rekordbox_text());
                prop_assert!(
                    allowed,
                    "{}: rekordbox's {:?} is overwritten by {:?} and no conflict row holds it",
                    name,
                    was.rekordbox,
                    value
                );
            }
            let f = &mut self.fields[i];
            if f.rekordbox.as_deref() != Some(value.as_str()) {
                f.standing = None;
            }
            f.rekordbox = Some(value);
            // rekordbox shows the app's value now: nothing is "replaced".
            f.replaced = None;
        }
        Ok(())
    }
}

#[test]
fn no_sequence_of_edits_reads_sends_resolves_and_undos_loses_a_value() {
    // One database for every case: each starts by emptying the conflicts
    // and the operation log.
    let lib = Lib::new();
    let id = lib.bare();
    let config = Config {
        cases: CASES,
        failure_persistence: None,
        ..Config::default()
    };
    let rng = TestRng::deterministic_rng(config.rng_algorithm);
    let mut runner = TestRunner::new_with_rng(config, rng);

    let played = runner.run(&proptest::collection::vec(step(), 1..STEPS), |steps| {
        let mut world = World::new(&lib, id);
        for step in &steps {
            world.apply(step)?;
        }
        // Whatever happened, a last read leaves the table in step.
        world.detect()?;
        Ok(())
    });

    if let Err(failed) = played {
        panic!("{failed}");
    }
}
