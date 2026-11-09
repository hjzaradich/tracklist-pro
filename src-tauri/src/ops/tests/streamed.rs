//! Run 6C U: Undo reads an operation's changes a piece at a time
//! ([`UNDO_PIECE`] rows), not all at once. The Undo that read them all at
//! once is kept here, word for word, as the oracle: over varied operations
//! larger than one piece, the two leave the database in the same state and
//! give the same conflicts, refusals and errors, whatever the piece size.

use super::*;

// --- The oracle: Undo as it was before Run 6C U --------------------------

/// The changes of one operation, oldest first.
fn oracle_changes_of(tx: &Transaction<'_>, operation: i64) -> Result<Vec<ChangeRow>, OpsError> {
    let mut stmt = tx.prepare(
        "SELECT entity, entity_id, action, field, before, after FROM change
         WHERE operation_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([operation], |r| {
        Ok(ChangeRow {
            entity: r.get(0)?,
            entity_id: r.get(1)?,
            action: r.get(2)?,
            field: r.get(3)?,
            before: r.get(4)?,
            after: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn oracle_unwind_with_automatic(
    tx: &Transaction<'_>,
    operation: &OperationInfo,
) -> Result<Found, OpsError> {
    let automatic = automatic_since(tx, operation)?;
    if automatic.is_empty() {
        return oracle_unwind(tx, operation, &HashMap::new());
    }
    // Oldest first.
    let mut theirs: Vec<ChangeRow> = Vec::new();
    for op in automatic.iter().rev() {
        theirs.extend(oracle_changes_of(tx, op.id)?);
    }
    // An automatic operation that made or deleted rows has no net change
    // per field: those are taken back one after the other, newest first.
    if theirs.iter().any(|c| c.action != "set") {
        for op in &automatic {
            let found = oracle_unwind(tx, op, &HashMap::new())?;
            if !found.conflicts.is_empty() {
                return Ok(Found {
                    conflicts: found.conflicts,
                    ..Found::default()
                });
            }
        }
        return oracle_unwind(tx, operation, &HashMap::new());
    }

    let mut found = Found::default();
    let mut keys: Vec<FieldKey> = Vec::new();
    let mut net: HashMap<FieldKey, Net> = HashMap::new();
    for change in &theirs {
        let key = (
            change.entity.clone(),
            change.entity_id,
            change.field.clone(),
        );
        match net.get_mut(&key) {
            Some(n) => {
                if n.after != change.before {
                    found.conflicts.push(conflict(
                        change,
                        Some(&change.field),
                        ConflictProblem::ChangedSince,
                    ));
                }
                n.after.clone_from(&change.after);
            }
            None => {
                keys.push(key.clone());
                net.insert(
                    key,
                    Net {
                        before: change.before.clone(),
                        after: change.after.clone(),
                    },
                );
            }
        }
    }
    if !found.conflicts.is_empty() {
        return Ok(found);
    }

    // What the user's operation wrote itself.
    let mine = oracle_changes_of(tx, operation.id)?;
    let covered = |key: &FieldKey| {
        mine.iter().any(|c| {
            c.entity == key.0
                && c.entity_id == key.1
                && (c.action == "insert" || (c.action == "set" && c.field == key.2))
        })
    };
    let mut schema = SchemaCache::default();
    let mut taken_to_hold: HashMap<FieldKey, Option<String>> = HashMap::new();
    for key in keys {
        let n = &net[&key];
        let table = schema.table(tx, &key.0)?;
        let net_change = ChangeRow {
            entity: key.0.clone(),
            entity_id: key.1,
            action: "set".to_owned(),
            field: key.2.clone(),
            before: n.before.clone(),
            after: n.after.clone(),
        };
        if covered(&key) {
            // Still what the newest of them left? Then the user's operation
            // is undone as if the field held what the oldest found.
            let column = table.column(&key.2)?;
            match table.read_field(tx, key.1, column) {
                Ok(now) if now == n.after => {
                    taken_to_hold.insert(key, n.before.clone());
                }
                Ok(_) => found.conflicts.push(conflict(
                    &net_change,
                    Some(&net_change.field),
                    ConflictProblem::ChangedSince,
                )),
                Err(OpsError::RowNotFound { .. }) => {
                    found
                        .conflicts
                        .push(conflict(&net_change, None, ConflictProblem::RowGone));
                }
                Err(e) => return Err(e),
            }
        } else {
            // Not the user's to undo: put back for real, as a set is.
            undo_set(
                tx,
                &mut schema,
                &table,
                &net_change,
                &mut found,
                &HashMap::new(),
            )?;
        }
    }
    if !found.conflicts.is_empty() {
        return Ok(Found {
            conflicts: found.conflicts,
            ..Found::default()
        });
    }

    let found = oracle_unwind(tx, operation, &taken_to_hold)?;
    if !found.conflicts.is_empty() {
        return Ok(found);
    }
    for op in &automatic {
        tx.execute(
            "UPDATE operation SET undone_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            [op.id],
        )?;
    }
    Ok(found)
}

fn oracle_unwind(
    tx: &Transaction<'_>,
    operation: &OperationInfo,
    taken_to_hold: &HashMap<FieldKey, Option<String>>,
) -> Result<Found, OpsError> {
    let changes: Vec<ChangeRow> = {
        let mut stmt = tx.prepare(
            "SELECT entity, entity_id, action, field, before, after FROM change
             WHERE operation_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([operation.id], |r| {
            Ok(ChangeRow {
                entity: r.get(0)?,
                entity_id: r.get(1)?,
                action: r.get(2)?,
                field: r.get(3)?,
                before: r.get(4)?,
                after: r.get(5)?,
            })
        })?;
        rows.collect::<Result<_, _>>()?
    };

    let mut found = Found::default();
    let mut schema = SchemaCache::default();
    let mut i = 0;
    while i < changes.len() {
        let change = &changes[i];
        let run = changes[i..]
            .iter()
            .take_while(|c| {
                change.action != "set"
                    && c.action == change.action
                    && c.entity == change.entity
                    && c.entity_id == change.entity_id
            })
            .count()
            .max(1);
        let group = &changes[i..i + run];
        let table = schema.table(tx, &change.entity)?;
        match change.action.as_str() {
            "set" => undo_set(tx, &mut schema, &table, change, &mut found, taken_to_hold)?,
            "insert" => undo_insert(tx, &mut schema, &table, group, &mut found, taken_to_hold)?,
            "delete" => undo_delete(tx, &table, group, &mut found)?,
            other => return Err(OpsError::BadAction(other.to_owned())),
        }
        i += run;
    }
    if !found.conflicts.is_empty() {
        return Ok(found);
    }

    let mut crates: Vec<i64> = changes
        .iter()
        .filter(|c| c.entity == "crate")
        .map(|c| c.entity_id)
        .collect();
    crates.sort_unstable();
    crates.dedup();
    if let Some((id, name)) = crate::crates::name_clash(tx, &crates)? {
        found.conflicts.push(UndoConflict {
            entity: "crate".to_owned(),
            entity_id: id,
            field: Some("name".to_owned()),
            problem: ConflictProblem::Blocked,
        });
        found.name_taken = Some(name);
        return Ok(found);
    }

    tx.execute(
        "UPDATE operation SET undone_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        [operation.id],
    )?;
    Ok(found)
}

// --- Comparing the two -------------------------------------------------

/// Every row of every table, as text, sorted per table. When an operation
/// was undone is the clock's, so only whether it was is kept.
fn snapshot(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
            let names: Vec<String> = stmt.column_names().map(str::to_owned).collect();
            let mut rows: Vec<String> = stmt
                .query_map([], |r| {
                    let mut row = Vec::new();
                    for (i, name) in names.iter().enumerate() {
                        let value: Value = r.get(i)?;
                        row.push(if name == "undone_at" {
                            format!("{}", value != Value::Null)
                        } else {
                            format!("{value:?}")
                        });
                    }
                    Ok(row.join(" | "))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            rows.sort();
            (table, rows)
        })
        .collect()
}

/// What one Undo did: what it found in the way (or the error it stopped
/// on), and the database it left inside its transaction.
#[derive(Debug, PartialEq)]
struct Outcome {
    found: Result<FoundSeen, String>,
    state: Vec<(String, Vec<String>)>,
}

#[derive(Debug, PartialEq)]
struct FoundSeen {
    conflicts: Vec<UndoConflict>,
    name_taken: Option<String>,
    held_by_a_send: bool,
    source_gone: bool,
    refusal: Option<UndoRefusal>,
}

#[derive(Clone, Copy, Debug)]
enum Way {
    /// The oracle, reading every change at once.
    AllAtOnce,
    /// Undo as it is, a piece of this many rows at a time.
    InPieces(usize),
}

/// The Undo of the newest user operation, the given way, in a transaction
/// that is then rolled back: the database is as it was for the next way.
fn undo_the(conn: &mut Connection, way: Way) -> Outcome {
    let tx = conn.transaction().unwrap();
    let operation = next_operation(&tx).unwrap().expect("an operation to undo");
    let found = match way {
        Way::AllAtOnce => oracle_unwind_with_automatic(&tx, &operation),
        Way::InPieces(piece) => unwind_with_automatic_in(&tx, &operation, piece),
    };
    let found = found
        .map(|f| FoundSeen {
            refusal: f.refusal(),
            conflicts: f.conflicts,
            name_taken: f.name_taken,
            held_by_a_send: f.held_by_a_send,
            source_gone: f.source_gone,
        })
        .map_err(|e| format!("{e:?}"));
    let state = snapshot(&tx);
    tx.rollback().unwrap();
    Outcome { found, state }
}

/// Undoes the newest user operation the old way and in pieces of many
/// sizes, asserts every way gives the same, and returns that.
fn the_same_every_way(conn: &mut Connection) -> Outcome {
    let before = snapshot(conn);
    let oracle = undo_the(conn, Way::AllAtOnce);
    for piece in [1, 2, 3, 4, 5, 7, 11, 64, UNDO_PIECE] {
        let streamed = undo_the(conn, Way::InPieces(piece));
        assert_eq!(
            streamed.found, oracle.found,
            "what Undo found, in pieces of {piece}"
        );
        assert!(
            streamed.state == oracle.state,
            "the database after Undo differs in pieces of {piece}"
        );
    }
    assert_eq!(snapshot(conn), before, "every way was rolled back");
    oracle
}

fn changes_in(conn: &Connection, operation: i64) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM change WHERE operation_id = ?1",
        [operation],
        |r| r.get(0),
    )
    .unwrap()
}

fn sample_text(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT t FROM sample WHERE id = ?1", [id], |r| r.get(0))
        .unwrap()
}

// --- The operations ----------------------------------------------------

/// What a varied operation made, for the cases to change behind its back.
struct Varied {
    operation: i64,
    /// `sample` rows it inserted, oldest first.
    inserted: Vec<i64>,
    /// `sample` rows that were there, whose fields it set.
    set: Vec<i64>,
    /// `parent` rows it deleted, by id and code.
    deleted: Vec<(i64, String)>,
    /// A `parent` row it inserted.
    new_parent: i64,
    /// The crate it renamed and wrote notes on, and the crate it made.
    renamed: i64,
    made: i64,
    /// A crate it didn't touch.
    other: i64,
}

/// One user operation over every kind of row change, `size` times over:
/// inserts of `sample` rows (one logged change per field, so their groups
/// straddle the pieces), sets of fields of rows that were there, a set of
/// a field of a row it inserted itself, deletes of `parent` rows, a crate
/// made, and a crate renamed with new notes.
fn varied(conn: &mut Connection, size: i64) -> Varied {
    let renamed = crate_row(conn, "Warmup");
    let other = crate_row(conn, "Peak");
    let set: Vec<i64> = (0..size)
        .map(|n| {
            conn.execute(
                "INSERT INTO sample (i, r, t, b) VALUES (?1, ?1 * 0.5, 'row ' || ?1, x'00')",
                [n],
            )
            .unwrap();
            conn.last_insert_rowid()
        })
        .collect();
    let deleted: Vec<(i64, String)> = (0..size.min(8))
        .map(|n| {
            let code = format!("code {n}");
            (parent_row(conn, &code), code)
        })
        .collect();
    let (to_set, to_delete) = (set.clone(), deleted.clone());
    let recorded = record(conn, "tidy", &no_details(), move |rec| {
        let mut inserted = Vec::new();
        let mut made = 0;
        let mut new_parent = 0;
        for (n, id) in to_set.iter().enumerate() {
            let n = n as i64;
            inserted.push(rec.insert(
                "sample",
                &[
                    ("i", Value::Integer(n)),
                    ("r", Value::Real(n as f64 / 4.0)),
                    ("t", Value::Text(format!("new {n}"))),
                    ("b", Value::Blob(vec![1, 2, n as u8])),
                ],
            )?);
            rec.set("sample", *id, "t", format!("changed {n}"))?;
            if n % 3 == 0 {
                rec.set("sample", *id, "i", n * 10)?;
                rec.set(
                    "sample",
                    *inserted.last().unwrap(),
                    "t",
                    format!("then {n}"),
                )?;
            }
            if let Some((parent, _)) = to_delete.get(n as usize) {
                rec.delete("parent", *parent)?;
            }
            if n == size / 2 {
                made = rec.insert(
                    "crate",
                    &[
                        ("kind", Value::Text("static".into())),
                        ("name", Value::Text("Made".into())),
                    ],
                )?;
                new_parent = rec.insert(
                    "parent",
                    &[
                        ("code", Value::Text("new code".into())),
                        ("name", Value::Text("New".into())),
                    ],
                )?;
                rec.set("crate", renamed, "name", "Warmup 2")?;
                rec.set("crate", renamed, "notes", "my notes")?;
            }
        }
        Ok((inserted, made, new_parent))
    })
    .unwrap();
    let (inserted, made, new_parent) = recorded.value;
    Varied {
        operation: recorded.operation_id.unwrap(),
        inserted,
        set,
        deleted,
        new_parent,
        renamed,
        made,
        other,
    }
}

/// An automatic operation's sets, as the app makes them on its own.
fn automatic_sets(conn: &mut Connection, sets: &[(&str, i64, &str, Value)]) -> i64 {
    let sets: Vec<(String, i64, String, Value)> = sets
        .iter()
        .map(|(e, id, f, v)| ((*e).to_owned(), *id, (*f).to_owned(), v.clone()))
        .collect();
    record_automatic(conn, AutomaticKind::FollowFile, &no_details(), |rec| {
        for (entity, id, field, value) in &sets {
            rec.set(entity, *id, field, value)?;
        }
        Ok(())
    })
    .unwrap()
    .operation_id
    .unwrap()
}

fn refusal(outcome: &Outcome) -> Option<UndoRefusal> {
    outcome.found.as_ref().unwrap().refusal.clone()
}

fn problems(outcome: &Outcome) -> Vec<ConflictProblem> {
    let mut problems: Vec<ConflictProblem> = outcome
        .found
        .as_ref()
        .unwrap()
        .conflicts
        .iter()
        .map(|c| c.problem)
        .collect();
    problems.dedup();
    problems
}

// --- The cases ---------------------------------------------------------

#[test]
fn a_varied_operation_larger_than_a_piece_undoes_the_same_in_pieces_as_all_at_once() {
    let (_dir, mut conn) = db();
    let op = varied(&mut conn, 30);
    assert!(changes_in(&conn, op.operation) > 200);
    let outcome = the_same_every_way(&mut conn);
    assert_eq!(refusal(&outcome), None);
    // And it really was undone: back to before the operation.
    undone(undo_last(&mut conn).unwrap());
    assert_eq!(sample_text(&conn, op.set[4]).as_deref(), Some("row 4"));
    assert_eq!(count(&conn, "sample"), 30);
    assert_eq!(crate_name(&conn, op.renamed).as_deref(), Some("Warmup"));
    assert_eq!(crate_name(&conn, op.made), None);
    assert_eq!(count(&conn, "parent"), 8);
}

#[test]
fn a_varied_operation_larger_than_many_pieces_undoes_the_same_at_the_real_piece_size() {
    let (_dir, mut conn) = db();
    let op = varied(&mut conn, 500);
    assert!(changes_in(&conn, op.operation) > 3 * UNDO_PIECE as i64);
    assert_eq!(refusal(&the_same_every_way(&mut conn)), None);
    // A row changed behind its back, in the middle: refused, the same way.
    conn.execute("UPDATE sample SET t = 'drift' WHERE id = ?1", [op.set[200]])
        .unwrap();
    let outcome = the_same_every_way(&mut conn);
    assert_eq!(refusal(&outcome), Some(UndoRefusal::ChangedSince));
    assert_eq!(problems(&outcome), [ConflictProblem::ChangedSince]);
}

#[test]
fn rows_changed_behind_an_operations_back_refuse_its_undo_the_same_in_pieces() {
    let (_dir, mut conn) = db();
    let op = varied(&mut conn, 30);
    conn.execute_batch(&format!(
        "UPDATE sample SET t = 'drift' WHERE id = {};
         UPDATE sample SET r = 99.5 WHERE id = {};
         DELETE FROM sample WHERE id = {};
         DELETE FROM sample WHERE id = {};",
        op.set[5], op.inserted[7], op.inserted[12], op.set[20]
    ))
    .unwrap();
    let outcome = the_same_every_way(&mut conn);
    assert_eq!(refusal(&outcome), Some(UndoRefusal::ChangedSince));
    // Each of the four is found (a row the operation inserted, then set a
    // field of, is gone for both changes).
    let conflicts = &outcome.found.as_ref().unwrap().conflicts;
    let found = |entity_id: i64, problem: ConflictProblem| {
        conflicts
            .iter()
            .any(|c| c.entity_id == entity_id && c.problem == problem)
    };
    assert!(
        found(op.set[5], ConflictProblem::ChangedSince),
        "{conflicts:?}"
    );
    assert!(found(op.inserted[7], ConflictProblem::ChangedSince));
    assert!(found(op.inserted[12], ConflictProblem::RowGone));
    assert!(found(op.set[20], ConflictProblem::RowGone));
}

#[test]
fn rows_the_database_refuses_or_that_came_back_refuse_its_undo_the_same_in_pieces() {
    let (_dir, mut conn) = db();
    let op = varied(&mut conn, 30);
    // A deleted row's code is taken (the unique index refuses it back), a
    // deleted row is back, and a row the operation made is referenced.
    let (_, code) = &op.deleted[2];
    conn.execute(
        "INSERT INTO parent (code, name) VALUES (?1, 'Other')",
        [code],
    )
    .unwrap();
    let (back, _) = &op.deleted[5];
    conn.execute(
        "INSERT INTO parent (id, code, name) VALUES (?1, 'back', 'Back')",
        [back],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO child_cascade (parent_id) VALUES (?1)",
        [op.new_parent],
    )
    .unwrap();
    let outcome = the_same_every_way(&mut conn);
    let mut seen = problems(&outcome);
    seen.sort_by_key(|p| format!("{p:?}"));
    seen.dedup();
    assert_eq!(
        seen,
        [
            ConflictProblem::Blocked,
            ConflictProblem::Referenced,
            ConflictProblem::RowBack
        ]
    );
}

#[test]
fn a_crate_name_taken_since_refuses_the_undo_the_same_in_pieces() {
    let (_dir, mut conn) = db();
    let _op = varied(&mut conn, 30);
    // Another crate has the renamed crate's old name now.
    crate_row(&conn, "Warmup");
    let outcome = the_same_every_way(&mut conn);
    assert_eq!(
        refusal(&outcome),
        Some(UndoRefusal::CrateNameTaken {
            name: "Warmup".to_owned()
        })
    );
}

#[test]
fn automatic_operations_since_are_taken_back_with_it_the_same_in_pieces() {
    let (_dir, mut conn) = db();
    let op = varied(&mut conn, 30);
    // On fields it set, on a row it inserted, and on a field it didn't
    // write; twice on one of them.
    automatic_sets(
        &mut conn,
        &[
            ("crate", op.renamed, "notes", Value::Text("moved".into())),
            (
                "sample",
                op.inserted[4],
                "t",
                Value::Text("followed".into()),
            ),
            ("sample", op.set[1], "r", Value::Real(8.0)),
        ],
    );
    automatic_sets(
        &mut conn,
        &[(
            "crate",
            op.renamed,
            "notes",
            Value::Text("moved twice".into()),
        )],
    );
    // On a row it didn't touch: left alone.
    automatic_sets(
        &mut conn,
        &[("crate", op.other, "notes", Value::Text("elsewhere".into()))],
    );
    let outcome = the_same_every_way(&mut conn);
    assert_eq!(refusal(&outcome), None);
    undone(undo_last(&mut conn).unwrap());
    assert_eq!(crate_notes(&conn, op.renamed).as_deref(), Some("old notes"));
    assert_eq!(crate_notes(&conn, op.other).as_deref(), Some("elsewhere"));
    let r: f64 = conn
        .query_row("SELECT r FROM sample WHERE id = ?1", [op.set[1]], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(r, 0.5);
}

#[test]
fn automatic_operations_changed_behind_their_back_refuse_the_same_in_pieces() {
    // Their chain broken: one didn't find what the one before it left.
    let (_dir, mut conn) = db();
    let op = varied(&mut conn, 30);
    automatic_sets(
        &mut conn,
        &[("crate", op.renamed, "notes", Value::Text("moved".into()))],
    );
    conn.execute(
        "UPDATE crate SET notes = 'by hand' WHERE id = ?1",
        [op.renamed],
    )
    .unwrap();
    automatic_sets(
        &mut conn,
        &[(
            "crate",
            op.renamed,
            "notes",
            Value::Text("moved twice".into()),
        )],
    );
    let outcome = the_same_every_way(&mut conn);
    assert_eq!(refusal(&outcome), Some(UndoRefusal::ChangedSince));

    // A field they set that holds something else now, and a row they set
    // that is gone.
    let (_dir, mut conn) = db();
    let op = varied(&mut conn, 30);
    automatic_sets(
        &mut conn,
        &[
            (
                "sample",
                op.inserted[9],
                "t",
                Value::Text("followed".into()),
            ),
            ("sample", op.set[2], "r", Value::Real(8.0)),
        ],
    );
    conn.execute_batch(&format!(
        "UPDATE sample SET t = 'by hand' WHERE id = {};
         DELETE FROM sample WHERE id = {};",
        op.inserted[9], op.set[2]
    ))
    .unwrap();
    let outcome = the_same_every_way(&mut conn);
    assert_eq!(refusal(&outcome), Some(UndoRefusal::ChangedSince));
    assert_eq!(
        problems(&outcome),
        [ConflictProblem::ChangedSince, ConflictProblem::RowGone]
    );
}

#[test]
fn an_automatic_operation_that_made_rows_is_taken_back_the_same_in_pieces() {
    let (_dir, mut conn) = db();
    let op = varied(&mut conn, 30);
    let target = op.set[6];
    record_automatic(&mut conn, AutomaticKind::FollowFile, &no_details(), |rec| {
        for n in 0..6 {
            rec.insert("sample", &[("t", Value::Text(format!("auto {n}")))])?;
        }
        rec.set("sample", target, "t", "followed")
    })
    .unwrap();
    assert_eq!(refusal(&the_same_every_way(&mut conn)), None);
    // And refused alike when one of the rows it made changed since.
    let made: i64 = conn
        .query_row("SELECT max(id) FROM sample", [], |r| r.get(0))
        .unwrap();
    conn.execute("UPDATE sample SET t = 'by hand' WHERE id = ?1", [made])
        .unwrap();
    assert_eq!(
        refusal(&the_same_every_way(&mut conn)),
        Some(UndoRefusal::ChangedSince)
    );
}

#[test]
fn a_change_undo_cannot_read_stops_it_with_the_same_error_in_pieces() {
    // A logged action Undo doesn't know, deep in the operation.
    let (_dir, mut conn) = db();
    let op = varied(&mut conn, 30);
    conn.execute_batch(&format!(
        "PRAGMA ignore_check_constraints = ON;
         UPDATE change SET action = 'merge'
             WHERE id = (SELECT min(id) + 40 FROM change WHERE operation_id = {});
         PRAGMA ignore_check_constraints = OFF;",
        op.operation
    ))
    .unwrap();
    let outcome = the_same_every_way(&mut conn);
    assert_eq!(outcome.found, Err("BadAction(\"merge\")".to_owned()));
}
