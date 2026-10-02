//! The operation log: every user action is recorded with each field it
//! changed, so it can be undone (ROADMAP §1, "Everything in the app is
//! reversible"; §2, `operation` / `change`; 0.2).
//!
//! Ported from musicmanager's `ops/mod.rs` as a copy. What changed:
//! - The log is generic over tables. A change names its table and row
//!   (`entity`, `entity_id`) and says what happened (`set`, `insert`,
//!   `delete`), so a NULL value is never mistaken for a missing row.
//! - An operation stores a `kind` and a `details` JSON object, never UI text;
//!   the frontend describes it from the locale files.
//! - Writes and their log rows are made together by a [`Recorder`], inside
//!   one transaction: either both land or neither does.
//! - Undo refuses, and changes nothing, if a field it would restore was
//!   changed since, instead of silently overwriting that later change.
//! - An undone operation is marked `undone_at`, not deleted.
//! - musicmanager wrote tags into audio files here. This app doesn't write
//!   audio files in the MVP (CLAUDE.md), so the log covers the database only.
//!
//! Undo is multi-step (1bA-12): [`undo_last`] undoes the newest operation
//! that isn't undone yet, so calling it again walks back through the
//! history, one operation at a time, with no limit and across restarts
//! (the marks are in the database). There is no redo. A step that's refused
//! changes nothing and isn't skipped: the operations before it stay out of
//! reach until a new operation is made. That works because history is
//! linear: undoing an operation puts back the state the one before it left.
//!
//! Row ids name rows in the log, so a table that code outside the log
//! inserts into must never reuse an id (`AUTOINCREMENT`; `sync_base`, which
//! a send writes). A test scans the source for such inserts.

mod schema;

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use specta::Type;

use crate::db::{DbError, Writer};
use schema::{decode, first_referencing, quote, ForeignKeyAction, ReferenceCheck, Table};

/// Why recording or undoing an operation failed. Nothing was changed.
#[derive(Debug)]
pub enum OpsError {
    /// The database refused.
    Db(DbError),
    /// An operation kind is a short identifier such as `edit_fields`.
    BadKind(String),
    /// `details` must be a JSON object.
    DetailsNotObject,
    /// The log can't record changes to this table: it isn't a STRICT rowid
    /// table, or it's one of the log's own tables.
    NotRecordable(String),
    /// The table has no such field (or it's the row's id).
    NotAField { entity: String, field: String },
    /// There's no such row.
    RowNotFound { entity: String, id: i64 },
    /// A stored value can't be read back as its column's type. Means the log
    /// was edited by hand or the column changed type.
    BadStoredValue { field: String, value: String },
    /// `change` holds an action other than set, insert or delete.
    BadAction(String),
    /// Rows in `by` reference this row through a foreign key that would
    /// silently change or delete them (CASCADE, SET NULL, SET DEFAULT). The
    /// log can't record that, so it would be irreversible.
    Referenced { entity: String, id: i64, by: String },
    /// A number must be finite: SQLite stores NaN as NULL.
    NotFinite(String),
    /// [`Recorder::read_rows`] takes statements that only read.
    NotReadOnly(String),
}

impl fmt::Display for OpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpsError::Db(e) => write!(f, "{e}"),
            OpsError::BadKind(kind) => write!(f, "{kind:?} is not an operation kind"),
            OpsError::DetailsNotObject => write!(f, "operation details must be a JSON object"),
            OpsError::NotRecordable(entity) => {
                write!(f, "changes to {entity:?} can't be recorded")
            }
            OpsError::NotAField { entity, field } => {
                write!(f, "{entity:?} has no recordable field {field:?}")
            }
            OpsError::RowNotFound { entity, id } => write!(f, "{entity:?} has no row {id}"),
            OpsError::BadStoredValue { field, value } => {
                write!(f, "stored value {value:?} doesn't fit field {field:?}")
            }
            OpsError::BadAction(action) => write!(f, "unknown change action {action:?}"),
            OpsError::Referenced { entity, id, by } => write!(
                f,
                "{entity:?} row {id} is referenced by {by:?}, which would change with it"
            ),
            OpsError::NotFinite(field) => write!(f, "{field:?} is not a finite number"),
            OpsError::NotReadOnly(sql) => write!(f, "{sql:?} is not a read-only statement"),
        }
    }
}

impl std::error::Error for OpsError {}

impl From<rusqlite::Error> for OpsError {
    fn from(e: rusqlite::Error) -> Self {
        OpsError::Db(DbError::Sqlite(e))
    }
}

impl From<DbError> for OpsError {
    fn from(e: DbError) -> Self {
        OpsError::Db(e)
    }
}

/// What [`record`] made: the operation's id (none if nothing changed) and
/// the write's own result.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded<T> {
    pub operation_id: Option<i64>,
    pub value: T,
}

/// Makes writes and records each one as a change of one operation. Only
/// [`record`] hands one out, inside the operation's transaction.
pub struct Recorder<'a> {
    tx: &'a Transaction<'a>,
    operation_id: i64,
    schema: SchemaCache,
    /// The `change` rows of the step that's running, written together (in
    /// order, so ids run in the order the changes were made) as the step
    /// ends, inside its savepoint: if that fails, the step's write goes too.
    pending: Vec<PendingChange>,
    /// How many `change` rows the finished steps wrote.
    written: usize,
    /// The operation's details, and whether [`Recorder::detail`] added to
    /// them.
    details: serde_json::Map<String, serde_json::Value>,
    details_changed: bool,
}

/// One `change` row, before it's written.
struct PendingChange {
    entity: String,
    entity_id: i64,
    action: &'static str,
    field: String,
    before: Option<String>,
    after: Option<String>,
}

/// What the schema says, looked up once per operation: it can't change
/// inside one, and looking it up is most of the cost of a recorded row.
#[derive(Default)]
struct SchemaCache {
    tables: HashMap<String, Rc<Table>>,
    checks: HashMap<(String, ForeignKeyAction, Option<String>), Rc<Vec<ReferenceCheck>>>,
}

impl SchemaCache {
    fn table(&mut self, conn: &Connection, entity: &str) -> Result<Rc<Table>, OpsError> {
        if let Some(table) = self.tables.get(entity) {
            return Ok(Rc::clone(table));
        }
        let table = Rc::new(Table::load(conn, entity)?);
        self.tables.insert(entity.to_owned(), Rc::clone(&table));
        Ok(table)
    }

    /// The first table with a row that references row `id` of `table`
    /// through a foreign key that would silently change it, for `on` and
    /// (if given) only keys on `columns` (see [`Table::reference_checks`]).
    fn referenced_by(
        &mut self,
        conn: &Connection,
        table: &Table,
        id: i64,
        on: ForeignKeyAction,
        columns: Option<&[&str]>,
    ) -> Result<Option<String>, OpsError> {
        let key = (table.name.clone(), on, columns.map(|c| c.join("\u{0}")));
        let checks = match self.checks.get(&key) {
            Some(checks) => Rc::clone(checks),
            None => {
                let checks = Rc::new(table.reference_checks(conn, on, columns)?);
                self.checks.insert(key, Rc::clone(&checks));
                checks
            }
        };
        first_referencing(conn, &checks, id)
    }
}

impl Recorder<'_> {
    /// Adds to the operation's details something only known once the write
    /// has run, such as how many tracks it changed. Like every detail it's
    /// a fact for the UI to describe the operation with, never text.
    pub fn detail(&mut self, key: &str, value: impl Into<serde_json::Value>) {
        self.details.insert(key.to_owned(), value.into());
        self.details_changed = true;
    }

    /// Reads rows inside the operation's transaction, to find what to
    /// change. Only a statement that reads rows is accepted (SQLite also
    /// calls ROLLBACK, SAVEPOINT and ATTACH read-only, but they return no
    /// columns), so nothing gets past the log; writes go through the
    /// recorder.
    pub fn read_rows<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        mut map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, OpsError> {
        let mut stmt = self.tx.prepare_cached(sql)?;
        if !stmt.readonly() || stmt.column_count() == 0 {
            return Err(OpsError::NotReadOnly(sql.to_owned()));
        }
        let rows = stmt.query_map(params, |r| map(r))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Sets one field of one row. Returns false, and records nothing, if it
    /// already held that value.
    pub fn set(
        &mut self,
        entity: &str,
        id: i64,
        field: &str,
        value: impl rusqlite::ToSql,
    ) -> Result<bool, OpsError> {
        check_finite(field, &value)?;
        self.step(|rec| {
            let tx = rec.tx;
            let table = rec.schema.table(tx, entity)?;
            let column = table.column(field)?.clone();
            let before = table.read_field(tx, id, &column)?;
            // Checked before the update, while children still point at the
            // old value; refused below only if the value really changes.
            let referenced_by = rec.schema.referenced_by(
                tx,
                &table,
                id,
                ForeignKeyAction::Update,
                Some(&[field]),
            )?;
            tx.prepare_cached(&format!(
                "UPDATE {} SET {} = ?1 WHERE rowid = ?2",
                table.sql_name(),
                quote(&column.name)
            ))?
            .execute(params![value, id])?;
            // Read back rather than trust the input: the column's type decides
            // what's stored (e.g. 5 in a REAL column is 5.0).
            let after = table.read_field(tx, id, &column)?;
            if after == before {
                return Ok(false);
            }
            if let Some(by) = referenced_by {
                return Err(OpsError::Referenced {
                    entity: table.name.clone(),
                    id,
                    by,
                });
            }
            rec.log(&table.name, id, "set", field, before, after);
            Ok(true)
        })
    }

    /// Inserts a row and returns its id. Every field of the new row is
    /// recorded, including defaults the database filled in.
    pub fn insert(&mut self, entity: &str, values: &[(&str, Value)]) -> Result<i64, OpsError> {
        for (field, value) in values {
            check_finite(field, value)?;
        }
        self.step(|rec| rec.insert_row(entity, values))
    }

    fn insert_row(&mut self, entity: &str, values: &[(&str, Value)]) -> Result<i64, OpsError> {
        let tx = self.tx;
        let table = self.schema.table(tx, entity)?;
        for (field, _) in values {
            table.column(field)?;
        }
        let sql = if values.is_empty() {
            format!("INSERT INTO {} DEFAULT VALUES", table.sql_name())
        } else {
            let names: Vec<String> = values.iter().map(|(f, _)| quote(f)).collect();
            let slots: Vec<String> = (1..=values.len()).map(|i| format!("?{i}")).collect();
            format!(
                "INSERT INTO {} ({}) VALUES ({})",
                table.sql_name(),
                names.join(", "),
                slots.join(", ")
            )
        };
        tx.prepare_cached(&sql)?
            .execute(rusqlite::params_from_iter(values.iter().map(|(_, v)| v)))?;
        let id = tx.last_insert_rowid();
        let row = table
            .read_row(tx, id)?
            .ok_or_else(|| OpsError::RowNotFound {
                entity: table.name.clone(),
                id,
            })?;
        self.log_row(&table, id, "insert", row);
        Ok(id)
    }

    /// Deletes a row, recording every field so undo can put it back.
    ///
    /// Refused while other rows reference it through a foreign key that
    /// would silently delete or change them (CASCADE, SET NULL, SET
    /// DEFAULT): those knock-on changes couldn't be undone. Delete or move
    /// them first, through this recorder.
    pub fn delete(&mut self, entity: &str, id: i64) -> Result<(), OpsError> {
        self.step(|rec| {
            let tx = rec.tx;
            let table = rec.schema.table(tx, entity)?;
            let row = table
                .read_row(tx, id)?
                .ok_or_else(|| OpsError::RowNotFound {
                    entity: table.name.clone(),
                    id,
                })?;
            if let Some(by) =
                rec.schema
                    .referenced_by(tx, &table, id, ForeignKeyAction::Delete, None)?
            {
                return Err(OpsError::Referenced {
                    entity: table.name.clone(),
                    id,
                    by,
                });
            }
            // Delete first, log second: a delete the database refuses (e.g.
            // a RESTRICT key) must not leave a logged delete behind.
            tx.prepare_cached(&format!(
                "DELETE FROM {} WHERE rowid = ?1",
                table.sql_name()
            ))?
            .execute([id])?;
            rec.log_row(&table, id, "delete", row);
            Ok(())
        })
    }

    /// Runs one recorder method as a unit, inside a savepoint: if it fails,
    /// neither its write nor its log rows are kept, even when the caller
    /// carries on past the error.
    fn step<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, OpsError>) -> Result<T, OpsError> {
        let tx = self.tx;
        tx.prepare_cached("SAVEPOINT ops_step")?.execute([])?;
        match f(self).and_then(|value| self.flush().map(|()| value)) {
            Ok(value) => {
                tx.prepare_cached("RELEASE ops_step")?.execute([])?;
                Ok(value)
            }
            Err(e) => {
                self.pending.clear();
                tx.prepare_cached("ROLLBACK TO ops_step")?.execute([])?;
                tx.prepare_cached("RELEASE ops_step")?.execute([])?;
                Err(e)
            }
        }
    }

    /// Records a whole row inserted or deleted: one change per field. A
    /// table with no fields besides its id gets one row-marker change on
    /// `rowid`.
    fn log_row(&mut self, table: &Table, id: i64, action: &'static str, row: Vec<Option<String>>) {
        if table.columns.is_empty() {
            let value = Some(id.to_string());
            let (before, after) = if action == "insert" {
                (None, value)
            } else {
                (value, None)
            };
            return self.log(&table.name, id, action, "rowid", before, after);
        }
        for (column, value) in table.columns.iter().zip(row) {
            let (before, after) = if action == "insert" {
                (None, value)
            } else {
                (value, None)
            };
            self.log(&table.name, id, action, &column.name, before, after);
        }
    }

    fn log(
        &mut self,
        entity: &str,
        id: i64,
        action: &'static str,
        field: &str,
        before: Option<String>,
        after: Option<String>,
    ) {
        self.pending.push(PendingChange {
            entity: entity.to_owned(),
            entity_id: id,
            action,
            field: field.to_owned(),
            before,
            after,
        });
    }

    /// Writes the step's `change` rows, in the order they were made, with
    /// one insert. A step changes one row of one table, and a table has at
    /// most 2,000 columns, so that's at most 14,000 values: under the
    /// 32,766 a statement may bind (SQLite is bundled, so that limit is
    /// ours).
    fn flush(&mut self) -> Result<(), OpsError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let sql = format!(
            "INSERT INTO change (operation_id, entity, entity_id, action, field, before, after)
             VALUES {}",
            vec!["(?, ?, ?, ?, ?, ?, ?)"; self.pending.len()].join(", ")
        );
        let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(self.pending.len() * 7);
        for change in &self.pending {
            values.push(&self.operation_id);
            values.push(&change.entity);
            values.push(&change.entity_id);
            values.push(&change.action);
            values.push(&change.field);
            values.push(&change.before);
            values.push(&change.after);
        }
        self.tx.prepare_cached(&sql)?.execute(&*values)?;
        self.written += self.pending.len();
        self.pending.clear();
        Ok(())
    }
}

/// Runs `write` as one operation of `kind`, with `details` for the UI to
/// describe it, and records every change it makes through the [`Recorder`].
///
/// All in one transaction: if `write` fails, nothing it did is kept and
/// nothing is logged. If it changed nothing, no operation is logged
/// (`operation_id` is `None`), so undo never lands on a no-op.
pub fn record<T>(
    conn: &mut Connection,
    kind: &str,
    details: &serde_json::Value,
    write: impl FnOnce(&mut Recorder<'_>) -> Result<T, OpsError>,
) -> Result<Recorded<T>, OpsError> {
    if !is_kind(kind) {
        return Err(OpsError::BadKind(kind.to_owned()));
    }
    let Some(details) = details.as_object() else {
        return Err(OpsError::DetailsNotObject);
    };
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO operation (kind, details) VALUES (?1, ?2)",
        params![kind, serde_json::Value::Object(details.clone()).to_string()],
    )?;
    let mut recorder = Recorder {
        tx: &tx,
        operation_id: tx.last_insert_rowid(),
        schema: SchemaCache::default(),
        pending: Vec::new(),
        written: 0,
        details: details.clone(),
        details_changed: false,
    };
    // An error drops `tx`, which rolls everything back.
    let value = write(&mut recorder)?;
    let operation_id = (recorder.written > 0).then_some(recorder.operation_id);
    if let Some(id) = operation_id {
        if recorder.details_changed {
            let details = serde_json::Value::Object(std::mem::take(&mut recorder.details));
            tx.execute(
                "UPDATE operation SET details = ?2 WHERE id = ?1",
                params![id, details.to_string()],
            )?;
        }
        tx.commit()?;
    } else {
        tx.rollback()?;
    }
    Ok(Recorded {
        operation_id,
        value,
    })
}

/// [`record`], run on the database's one writer connection.
pub fn record_via<T, F>(
    writer: &Writer,
    kind: &str,
    details: serde_json::Value,
    write: F,
) -> Result<Recorded<T>, OpsError>
where
    F: FnOnce(&mut Recorder<'_>) -> Result<T, OpsError> + Send + 'static,
    T: Send + 'static,
{
    let kind = kind.to_owned();
    writer.call(move |conn| Ok(record(conn, &kind, &details, write)))?
}

/// What the UI needs to name an operation, read from its `details`. Each
/// is there only if the operation recorded it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OperationDetails {
    /// The crate's name (after a rename, the new one).
    pub name: Option<String>,
    /// A renamed crate's name before.
    pub from: Option<String>,
    /// How many tracks the operation added or removed.
    pub tracks: Option<u32>,
}

impl OperationDetails {
    /// Reads the details an operation stored. Anything missing or of
    /// another shape is left out: the UI then names the operation plainly.
    fn parse(json: &str) -> OperationDetails {
        let value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_owned);
        OperationDetails {
            name: text("name"),
            from: text("from"),
            tracks: value
                .get("tracks")
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok()),
        }
    }
}

/// An operation, as undo reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OperationInfo {
    /// The operation's row id. Row ids cross IPC as plain numbers (see
    /// `ipc.rs`).
    pub id: i64,
    /// What kind of action it was, e.g. `edit_fields`. The UI describes it
    /// from the locale files.
    pub kind: String,
    pub details: OperationDetails,
}

/// Something changed since the operation that undo would overwrite.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UndoConflict {
    pub entity: String,
    /// The row's id in `entity`.
    pub entity_id: i64,
    /// The field that changed since; null when the whole row is at issue.
    pub field: Option<String>,
    pub problem: ConflictProblem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ConflictProblem {
    /// The field holds a different value from the one the operation left.
    ChangedSince,
    /// The row the operation changed or made has been deleted since.
    RowGone,
    /// The row the operation deleted exists again.
    RowBack,
    /// Other rows now reference the row, and undoing would silently delete
    /// or change them through a foreign key.
    Referenced,
    /// The database's own rules refuse the earlier value now: another row
    /// holds a name that must be unique, or rows that must not lose this
    /// one point at it.
    Blocked,
}

/// Why a step can't be undone, as the UI says it. A code, never text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum UndoRefusal {
    /// A track the step added has been sent to rekordbox since.
    SentSince,
    /// The step would bring back a crate name another crate has now.
    CrateNameTaken { name: String },
    /// Something the step changed has changed again since.
    ChangedSince,
}

/// What undoing the newest operation did.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum UndoOutcome {
    /// Every change of the operation was reversed.
    Undone { operation: OperationInfo },
    /// No operation is left to undo (or, when one operation was asked for,
    /// it isn't the next to undo).
    NothingToUndo,
    /// Undoing would overwrite later changes, so nothing was changed.
    Refused {
        operation: OperationInfo,
        reason: UndoRefusal,
        conflicts: Vec<UndoConflict>,
    },
}

/// What Undo would do now.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NextUndo {
    /// The operation Undo would take back; none when nothing is left.
    pub operation: Option<OperationInfo>,
    /// Why it would be refused, when that's known. `None` doesn't promise
    /// the undo will work: a very large operation isn't tried ahead of time
    /// (see [`TRIED_AHEAD_UP_TO`]).
    pub refusal: Option<UndoRefusal>,
}

/// One recorded change, as read back for undo.
struct ChangeRow {
    entity: String,
    entity_id: i64,
    action: String,
    field: String,
    before: Option<String>,
    after: Option<String>,
}

/// The newest operation that isn't undone: the next one to undo.
fn next_operation(tx: &Transaction<'_>) -> Result<Option<OperationInfo>, OpsError> {
    Ok(tx
        .query_row(
            "SELECT id, kind, details FROM operation
             WHERE undone_at IS NULL ORDER BY id DESC LIMIT 1",
            [],
            |r| {
                Ok(OperationInfo {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    details: OperationDetails::parse(r.get_ref(2)?.as_str()?),
                })
            },
        )
        .optional()?)
}

/// What stands in the way of undoing one operation.
#[derive(Default)]
struct Found {
    conflicts: Vec<UndoConflict>,
    /// A crate name the undo would bring back that another crate has now.
    name_taken: Option<String>,
}

impl Found {
    /// The one reason the UI gives, most specific first.
    fn refusal(&self) -> Option<UndoRefusal> {
        if let Some(name) = &self.name_taken {
            return Some(UndoRefusal::CrateNameTaken { name: name.clone() });
        }
        if self.conflicts.is_empty() {
            return None;
        }
        // A send marks every track it sent and gives it bases, which then
        // hold the Library track in place.
        let sent = self.conflicts.iter().any(|c| {
            c.entity == "library_track"
                && (c.problem == ConflictProblem::Blocked
                    || matches!(
                        c.field.as_deref(),
                        Some("last_sent_location" | "last_exported_at")
                    ))
        });
        Some(if sent {
            UndoRefusal::SentSince
        } else {
            UndoRefusal::ChangedSince
        })
    }
}

/// Reverses every change of `operation` inside `tx` and marks it undone,
/// unless something stands in the way. Then `tx` holds a half-undone state
/// and must be rolled back.
fn unwind(tx: &Transaction<'_>, operation: &OperationInfo) -> Result<Found, OpsError> {
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

    // Newest first, so changes made in sequence within the operation (insert
    // a row, then set one of its fields) unwind in reverse. Each step first
    // checks the database still holds what the operation left.
    let mut found = Found::default();
    let mut schema = SchemaCache::default();
    let mut i = 0;
    while i < changes.len() {
        let change = &changes[i];
        // A row's insert or delete is one change per field, logged together.
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
            "set" => undo_set(tx, &mut schema, &table, change, &mut found)?,
            "insert" => undo_insert(tx, &mut schema, &table, group, &mut found.conflicts)?,
            "delete" => undo_delete(tx, &table, group, &mut found)?,
            other => return Err(OpsError::BadAction(other.to_owned())),
        }
        i += run;
    }
    if !found.conflicts.is_empty() {
        return Ok(found);
    }

    // A crate name must differ from its siblings' the way a send compares
    // them, which is looser than the database's own unique index. History
    // alone can't break that (the later crate is undone first); a write
    // outside the log could.
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

fn undo_step(conn: &mut Connection, only: Option<i64>) -> Result<UndoOutcome, OpsError> {
    let tx = conn.transaction()?;
    let Some(operation) = next_operation(&tx)? else {
        return Ok(UndoOutcome::NothingToUndo);
    };
    if only.is_some_and(|id| id != operation.id) {
        return Ok(UndoOutcome::NothingToUndo);
    }
    let found = unwind(&tx, &operation)?;
    match found.refusal() {
        // Dropping `tx` rolls back the steps already taken.
        Some(reason) => Ok(UndoOutcome::Refused {
            operation,
            reason,
            conflicts: found.conflicts,
        }),
        None => {
            tx.commit()?;
            Ok(UndoOutcome::Undone { operation })
        }
    }
}

/// Undoes the newest operation that isn't undone yet, exactly: a set gets
/// its old value back, an inserted row is deleted, a deleted row is put back
/// with its old id. Call it again to undo the one before.
///
/// Refused, changing nothing, if anything the operation wrote has changed
/// since: undoing would silently throw that later change away. A refused
/// operation isn't skipped; the next call tries it again.
pub fn undo_last(conn: &mut Connection) -> Result<UndoOutcome, OpsError> {
    undo_step(conn, None)
}

/// [`undo_last`], but only if `operation` is the one it would undo.
/// Otherwise nothing is undone ([`UndoOutcome::NothingToUndo`]): an Undo
/// offered for one action never takes back a different one.
pub fn undo_only(conn: &mut Connection, operation: i64) -> Result<UndoOutcome, OpsError> {
    undo_step(conn, Some(operation))
}

/// [`undo_last`], run on the database's one writer connection.
pub fn undo_last_via(writer: &Writer) -> Result<UndoOutcome, OpsError> {
    writer.call(|conn| Ok(undo_last(conn)))?
}

/// The most `change` rows an operation may have for [`next_undo`] to try
/// its undo ahead of time. Trying costs about what the undo itself does, on
/// the one writer, each time the UI asks; a bigger operation (adding a whole
/// rekordbox collection) is only named, and a refusal shows when Undo is
/// used.
pub const TRIED_AHEAD_UP_TO: i64 = 2_000;

/// What [`undo_last`] would undo now, and why it would be refused, if it
/// would. Changes nothing: the undo is tried inside a transaction that is
/// always rolled back, so this says exactly what [`undo_last`] would.
pub fn next_undo(conn: &mut Connection) -> Result<NextUndo, OpsError> {
    let tx = conn.transaction()?;
    let Some(operation) = next_operation(&tx)? else {
        return Ok(NextUndo {
            operation: None,
            refusal: None,
        });
    };
    let changes: i64 = tx.query_row(
        "SELECT count(*) FROM change WHERE operation_id = ?1",
        [operation.id],
        |r| r.get(0),
    )?;
    let refusal = if changes <= TRIED_AHEAD_UP_TO {
        unwind(&tx, &operation)?.refusal()
    } else {
        None
    };
    tx.rollback()?;
    Ok(NextUndo {
        operation: Some(operation),
        refusal,
    })
}

fn conflict(change: &ChangeRow, field: Option<&str>, problem: ConflictProblem) -> UndoConflict {
    UndoConflict {
        entity: change.entity.clone(),
        entity_id: change.entity_id,
        field: field.map(str::to_owned),
        problem,
    }
}

fn undo_set(
    tx: &Transaction<'_>,
    schema: &mut SchemaCache,
    table: &Table,
    change: &ChangeRow,
    found: &mut Found,
) -> Result<(), OpsError> {
    let conflicts = &mut found.conflicts;
    let column = table.column(&change.field)?;
    let now = match table.read_field(tx, change.entity_id, column) {
        Ok(now) => now,
        Err(OpsError::RowNotFound { .. }) => {
            conflicts.push(conflict(change, None, ConflictProblem::RowGone));
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    if now != change.after {
        conflicts.push(conflict(
            change,
            Some(&change.field),
            ConflictProblem::ChangedSince,
        ));
        return Ok(());
    }
    let field = [change.field.as_str()];
    if schema
        .referenced_by(
            tx,
            table,
            change.entity_id,
            ForeignKeyAction::Update,
            Some(&field),
        )?
        .is_some()
    {
        conflicts.push(conflict(
            change,
            Some(&change.field),
            ConflictProblem::Referenced,
        ));
        return Ok(());
    }
    let written = tx
        .prepare_cached(&format!(
            "UPDATE {} SET {} = ?1 WHERE rowid = ?2",
            table.sql_name(),
            quote(&column.name)
        ))?
        .execute(params![
            decode(column, change.before.as_deref())?,
            change.entity_id
        ]);
    if refused_by_a_constraint(written)? {
        conflicts.push(conflict(
            change,
            Some(&change.field),
            ConflictProblem::Blocked,
        ));
        if is_crate_name(change) {
            found.name_taken = change.before.clone();
        }
    }
    Ok(())
}

/// Whether the database's rules (a unique index, a foreign key, a check)
/// refused a write of the undo. Any other failure is an error.
fn refused_by_a_constraint(written: rusqlite::Result<usize>) -> Result<bool, OpsError> {
    match written {
        Ok(_) => Ok(false),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Ok(true)
        }
        Err(e) => Err(e.into()),
    }
}

fn is_crate_name(change: &ChangeRow) -> bool {
    change.entity == "crate" && change.field == "name"
}

fn undo_insert(
    tx: &Transaction<'_>,
    schema: &mut SchemaCache,
    table: &Table,
    group: &[ChangeRow],
    conflicts: &mut Vec<UndoConflict>,
) -> Result<(), OpsError> {
    let first = &group[0];
    let Some(row) = table.read_row(tx, first.entity_id)? else {
        conflicts.push(conflict(first, None, ConflictProblem::RowGone));
        return Ok(());
    };
    let mut changed_since = false;
    for change in group.iter().filter(|c| c.field != "rowid") {
        // A derived column follows other data (database triggers), so its
        // moving on isn't someone else's edit.
        if is_derived(&table.name, &change.field) {
            continue;
        }
        let index = table
            .columns
            .iter()
            .position(|c| c.name == change.field)
            .ok_or_else(|| OpsError::NotAField {
                entity: table.name.clone(),
                field: change.field.clone(),
            })?;
        if row[index] != change.after {
            changed_since = true;
            conflicts.push(conflict(
                change,
                Some(&change.field),
                ConflictProblem::ChangedSince,
            ));
        }
    }
    if changed_since {
        return Ok(());
    }
    if schema
        .referenced_by(tx, table, first.entity_id, ForeignKeyAction::Delete, None)?
        .is_some()
    {
        conflicts.push(conflict(first, None, ConflictProblem::Referenced));
        return Ok(());
    }
    let written = tx
        .prepare_cached(&format!(
            "DELETE FROM {} WHERE rowid = ?1",
            table.sql_name()
        ))?
        .execute([first.entity_id]);
    if refused_by_a_constraint(written)? {
        conflicts.push(conflict(first, None, ConflictProblem::Blocked));
    }
    Ok(())
}

fn undo_delete(
    tx: &Transaction<'_>,
    table: &Table,
    group: &[ChangeRow],
    found: &mut Found,
) -> Result<(), OpsError> {
    let conflicts = &mut found.conflicts;
    let first = &group[0];
    if table.read_row(tx, first.entity_id)?.is_some() {
        conflicts.push(conflict(first, None, ConflictProblem::RowBack));
        return Ok(());
    }
    let mut names = vec!["rowid".to_owned()];
    let mut values = vec![Value::Integer(first.entity_id)];
    for change in group.iter().filter(|c| c.field != "rowid") {
        let column = table.column(&change.field)?;
        names.push(quote(&column.name));
        values.push(decode(column, change.before.as_deref())?);
    }
    let slots: Vec<String> = (1..=values.len()).map(|i| format!("?{i}")).collect();
    let written = tx
        .prepare_cached(&format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table.sql_name(),
            names.join(", "),
            slots.join(", ")
        ))?
        .execute(rusqlite::params_from_iter(values));
    if refused_by_a_constraint(written)? {
        conflicts.push(conflict(first, None, ConflictProblem::Blocked));
        if let Some(name) = group.iter().find(|c| is_crate_name(c)) {
            found.name_taken = name.before.clone();
        }
    }
    Ok(())
}

/// Columns the database keeps current itself with triggers, such as
/// `library_track.source_status`, which follows the linked file. Undoing an
/// insert doesn't treat a change to one as a later edit, and undoing a
/// delete lets the triggers set it again.
const DERIVED_FIELDS: &[(&str, &str)] = &[("library_track", "source_status")];

fn is_derived(entity: &str, field: &str) -> bool {
    DERIVED_FIELDS.contains(&(entity, field))
}

/// Refuses NaN and infinite numbers: SQLite would store NaN as NULL, so the
/// log would record a different value from the one asked for.
fn check_finite(field: &str, value: &dyn rusqlite::ToSql) -> Result<(), OpsError> {
    use rusqlite::types::{ToSqlOutput, ValueRef};
    let real = match value.to_sql()? {
        ToSqlOutput::Borrowed(ValueRef::Real(f)) | ToSqlOutput::Owned(Value::Real(f)) => Some(f),
        _ => None,
    };
    match real {
        Some(f) if !f.is_finite() => Err(OpsError::NotFinite(field.to_owned())),
        _ => Ok(()),
    }
}

/// An operation kind: lowercase letters, digits and `_`, starting with a
/// letter. It's a code, not text; the UI names it from the locale files.
fn is_kind(kind: &str) -> bool {
    kind.starts_with(|c: char| c.is_ascii_lowercase())
        && kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Undoes the newest operation that isn't undone yet, unless something it
/// wrote has changed since. With `operation_id`, only if that operation is
/// the one to undo. If the undo hits an error, nothing was changed.
// `async` runs it off the main thread, so the UI never waits on the writer.
#[tauri::command(async)]
#[specta::specta]
pub fn undo_last_operation(
    writer: tauri::State<'_, Writer>,
    operation_id: Option<i64>,
) -> Result<UndoOutcome, crate::ipc::IpcError> {
    Ok(writer.call(move |conn| Ok(undo_step(conn, operation_id)))??)
}

/// What Undo would undo now, and why it would be refused, if it would.
/// Changes nothing.
#[tauri::command(async)]
#[specta::specta]
pub fn next_undo_operation(
    writer: tauri::State<'_, Writer>,
) -> Result<NextUndo, crate::ipc::IpcError> {
    Ok(writer.call(|conn| Ok(next_undo(conn)))??)
}

#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod tests;
