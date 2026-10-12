//! Any sequence of edits, detects, resolves and undos keeps every value:
//! a field to review always has a conflict holding the three values as
//! they are now, or a decision made on exactly those values; a settled
//! conflict is never changed by a detect; a resolve keeps all three values;
//! an undo brings the open conflict back whole, or changes nothing.

use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

use super::*;

const FIELDS: [&str; 2] = ["Genre", "Rating"];

/// The values a side of a field can take; `None` is no value of its own
/// (the app), no such attribute (rekordbox) or never sent (the base).
/// Some differ only in spelling, so sides often agree.
fn value_of(field: usize, n: usize) -> Option<&'static str> {
    let values: [Option<&str>; 5] = match field {
        0 => [
            None,
            Some("House"),
            Some("Deep House"),
            Some("Tech House"),
            Some("house"),
        ],
        _ => [None, Some("0"), Some("51"), Some("255"), Some("0255")],
    };
    values[n % values.len()]
}

#[derive(Debug, Clone)]
enum Step {
    /// A side (the app's, rekordbox's or the base) of a field changes.
    Set {
        field: usize,
        side: usize,
        value: usize,
    },
    Detect,
    /// One open conflict is settled: the app's, rekordbox's, or by hand.
    Resolve {
        pick: usize,
        choice: usize,
    },
    /// Every open conflict is settled the same way.
    ResolveAll {
        choice: usize,
    },
    Undo,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0..2usize, 0..3usize, 0..5usize)
            .prop_map(|(field, side, value)| Step::Set { field, side, value }),
        4 => Just(Step::Detect),
        2 => (0..4usize, 0..3usize).prop_map(|(pick, choice)| Step::Resolve { pick, choice }),
        1 => (0..2usize).prop_map(|choice| Step::ResolveAll { choice }),
        2 => Just(Step::Undo),
    ]
}

fn choice(field: &str, n: usize) -> Choice {
    match n {
        0 => Choice::KeepApp,
        1 => Choice::KeepRekordbox,
        _ if field == "Rating" => Choice::Edit("153".into()),
        _ => Choice::Edit("Minimal".into()),
    }
}

struct World {
    lib: Lib,
    id: LibraryTrackId,
    /// Per field: the app's value, rekordbox's, the base.
    sides: [[Option<&'static str>; 3]; 2],
    /// The open conflicts each resolve that was logged settled, newest last.
    resolves: Vec<Vec<Conflict>>,
}

impl World {
    fn new() -> World {
        let lib = Lib::new();
        let id = lib.bare();
        World {
            lib,
            id,
            sides: [[None; 3]; 2],
            resolves: Vec::new(),
        }
    }

    fn track(&self) -> TrackSides {
        track(
            self.id,
            FIELDS
                .iter()
                .zip(self.sides)
                .map(|(name, [app, rekordbox, base])| field(name, app, rekordbox, base))
                .collect(),
        )
    }

    fn apply(&mut self, step: &Step) -> Result<(), TestCaseError> {
        match *step {
            Step::Set { field, side, value } => {
                self.sides[field][side] = value_of(field, value);
                Ok(())
            }
            Step::Detect => self.detect(),
            Step::Resolve { pick, choice: n } => {
                let open = self.lib.open(&Filter::default());
                if open.is_empty() {
                    return Ok(());
                }
                let target = open[pick % open.len()].clone();
                let resolved = self
                    .lib
                    .resolve(target.id, choice(&target.field, n))
                    .map_err(|e| TestCaseError::fail(e.to_string()))?
                    .value;
                self.kept(&target, &resolved)?;
                self.resolves.push(vec![target]);
                Ok(())
            }
            Step::ResolveAll { choice: n } => {
                let open = self.lib.open(&Filter::default());
                let resolved = self
                    .lib
                    .resolve_all(&Filter::default(), choice("", n))
                    .map_err(|e| TestCaseError::fail(e.to_string()))?
                    .value;
                prop_assert_eq!(resolved.len(), open.len());
                for (target, resolved) in open.iter().zip(&resolved) {
                    self.kept(target, resolved)?;
                }
                prop_assert!(self.lib.open(&Filter::default()).is_empty());
                if !open.is_empty() {
                    self.resolves.push(open);
                }
                Ok(())
            }
            Step::Undo => {
                let before = self.lib.rows();
                match self.lib.undo() {
                    UndoOutcome::Undone { .. } => {
                        let Some(reopened) = self.resolves.pop() else {
                            return Err(TestCaseError::fail("an undo with nothing resolved"));
                        };
                        let rows = self.lib.rows();
                        for open in reopened {
                            prop_assert!(rows.contains(&open), "{:?} is not back", open);
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
        }
    }

    /// A resolve replaced the open conflict by a settled one that still
    /// holds all three values.
    fn kept(&self, was: &Conflict, resolved: &Resolved) -> Result<(), TestCaseError> {
        let rows = self.lib.rows();
        prop_assert!(rows.iter().all(|c| c.id != was.id));
        let Some(settled) = rows.iter().find(|c| c.id == resolved.conflict) else {
            return Err(TestCaseError::fail(format!(
                "no settled row for {resolved:?}"
            )));
        };
        prop_assert_eq!(values(settled), values(was));
        prop_assert_eq!(&settled.field, &was.field);
        prop_assert_eq!(&settled.detected_at, &was.detected_at);
        prop_assert_ne!(settled.status, Status::Open);
        prop_assert_eq!(settled.chosen(), Some(resolved.value.as_str()));
        Ok(())
    }

    fn detect(&mut self) -> Result<(), TestCaseError> {
        let sides = self.track();
        let settled_before = self.lib.settled();
        let detected = self.lib.detect(std::slice::from_ref(&sides));

        // A detect never changes or deletes a settled conflict.
        let settled = self.lib.settled();
        prop_assert_eq!(&settled, &settled_before);

        let open = self.lib.open(&Filter::default());
        for f in &sides.fields {
            let name = f.field.as_str();
            let now = (
                f.app.as_deref(),
                Some(f.rekordbox.as_deref().unwrap_or("")),
                f.base.as_deref(),
            );
            let open_here: Vec<&Conflict> = open.iter().filter(|c| c.field == name).collect();
            let listed = detected.fields.iter().find(|o| o.field == name);
            let handling = decide(name, &sides.sides(f)).verdict.handling();
            if handling != Handling::Review {
                prop_assert!(open_here.is_empty(), "{}: {:?}", name, open_here);
                continue;
            }
            match listed.map(|o| &o.outcome) {
                // A conflict is open, holding the three values as they are.
                Some(Outcome::Open { conflict }) => {
                    prop_assert_eq!(open_here.len(), 1);
                    prop_assert_eq!(open_here[0].id, *conflict);
                    prop_assert_eq!(values(open_here[0]), now);
                }
                // Or the user decided, on these very values.
                Some(Outcome::Decided { conflict, value }) => {
                    prop_assert!(open_here.is_empty());
                    let Some(row) = settled.iter().find(|c| c.id == *conflict) else {
                        return Err(TestCaseError::fail(format!(
                            "{name}: decided by a row that's gone"
                        )));
                    };
                    prop_assert_eq!(&row.field, &f.field);
                    prop_assert_eq!(row.chosen(), Some(value.as_str()));
                    prop_assert!(same(name, row.rekordbox_value.as_deref(), now.1));
                    prop_assert_eq!(row.base_value.is_some(), now.2.is_some());
                    prop_assert!(same(name, row.base_value.as_deref(), now.2));
                    prop_assert!(
                        same(name, row.app_value.as_deref(), now.0)
                            || same(name, row.chosen(), now.0)
                    );
                }
                other => prop_assert!(false, "{}: to review, but {:?}", name, other),
            }
        }

        // And detecting again changes nothing.
        let rows = self.lib.rows();
        let again = self.lib.detect(std::slice::from_ref(&sides));
        prop_assert_eq!(self.lib.rows(), rows);
        prop_assert_eq!(&again.fields, &detected.fields);
        prop_assert!(again.opened.is_empty() && again.updated.is_empty());
        prop_assert!(again.withdrawn.is_empty());
        Ok(())
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    #[test]
    fn no_sequence_of_edits_detects_resolves_and_undos_loses_a_value(
        steps in proptest::collection::vec(step(), 1..40),
    ) {
        let mut world = World::new();
        for step in &steps {
            world.apply(step)?;
        }
        // Whatever happened, a last detect leaves the table in step.
        world.detect()?;
    }
}
