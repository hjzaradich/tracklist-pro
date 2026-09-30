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
//! Single-step undo only: [`undo_last`] undoes the most recent operation,
//! and once that's undone there's nothing more to undo. Multi-step undo is
//! 1bA-12.

mod schema;

use std::fmt;

use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use specta::Type;

use crate::db::{DbError, Writer};
use schema::{decode, quote, ForeignKeyAction, Table};

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
    changes: usize,
}

impl Recorder<'_> {
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
            let table = Table::load(rec.tx, entity)?;
            let column = table.column(field)?.clone();
            let before = table.read_field(rec.tx, id, &column)?;
            // Checked before the update, while children still point at the
            // old value; refused below only if the value really changes.
            let referenced_by =
                table.referenced_by(rec.tx, id, ForeignKeyAction::Update, Some(&[field]))?;
            rec.tx.execute(
                &format!(
                    "UPDATE {} SET {} = ?1 WHERE rowid = ?2",
                    table.sql_name(),
                    quote(&column.name)
                ),
                params![value, id],
            )?;
            // Read back rather than trust the input: the column's type decides
            // what's stored (e.g. 5 in a REAL column is 5.0).
            let after = table.read_field(rec.tx, id, &column)?;
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
            rec.log(&table.name, id, "set", field, before, after)?;
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
        let table = Table::load(self.tx, entity)?;
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
        self.tx.execute(
            &sql,
            rusqlite::params_from_iter(values.iter().map(|(_, v)| v)),
        )?;
        let id = self.tx.last_insert_rowid();
        let row = table
            .read_row(self.tx, id)?
            .ok_or_else(|| OpsError::RowNotFound {
                entity: table.name.clone(),
                id,
            })?;
        self.log_row(&table, id, "insert", row)?;
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
            let table = Table::load(rec.tx, entity)?;
            let row = table
                .read_row(rec.tx, id)?
                .ok_or_else(|| OpsError::RowNotFound {
                    entity: table.name.clone(),
                    id,
                })?;
            if let Some(by) = table.referenced_by(rec.tx, id, ForeignKeyAction::Delete, None)? {
                return Err(OpsError::Referenced {
                    entity: table.name.clone(),
                    id,
                    by,
                });
            }
            // Delete first, log second: a delete the database refuses (e.g.
            // a RESTRICT key) must not leave a logged delete behind.
            rec.tx.execute(
                &format!("DELETE FROM {} WHERE rowid = ?1", table.sql_name()),
                [id],
            )?;
            rec.log_row(&table, id, "delete", row)
        })
    }

    /// Runs one recorder method as a unit, inside a savepoint: if it fails,
    /// neither its write nor its log rows are kept, even when the caller
    /// carries on past the error.
    fn step<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, OpsError>) -> Result<T, OpsError> {
        self.tx.execute_batch("SAVEPOINT ops_step")?;
        let changes = self.changes;
        match f(self) {
            Ok(value) => {
                self.tx.execute_batch("RELEASE ops_step")?;
                Ok(value)
            }
            Err(e) => {
                self.changes = changes;
                self.tx
                    .execute_batch("ROLLBACK TO ops_step; RELEASE ops_step")?;
                Err(e)
            }
        }
    }

    /// Records a whole row inserted or deleted: one change per field. A
    /// table with no fields besides its id gets one row-marker change on
    /// `rowid`.
    fn log_row(
        &mut self,
        table: &Table,
        id: i64,
        action: &str,
        row: Vec<Option<String>>,
    ) -> Result<(), OpsError> {
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
            self.log(&table.name, id, action, &column.name, before, after)?;
        }
        Ok(())
    }

    fn log(
        &mut self,
        entity: &str,
        id: i64,
        action: &str,
        field: &str,
        before: Option<String>,
        after: Option<String>,
    ) -> Result<(), OpsError> {
        self.tx.execute(
            "INSERT INTO change (operation_id, entity, entity_id, action, field, before, after)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![self.operation_id, entity, id, action, field, before, after],
        )?;
        self.changes += 1;
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
    if !details.is_object() {
        return Err(OpsError::DetailsNotObject);
    }
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO operation (kind, details) VALUES (?1, ?2)",
        params![kind, details.to_string()],
    )?;
    let mut recorder = Recorder {
        tx: &tx,
        operation_id: tx.last_insert_rowid(),
        changes: 0,
    };
    // An error drops `tx`, which rolls everything back.
    let value = write(&mut recorder)?;
    let operation_id = (recorder.changes > 0).then_some(recorder.operation_id);
    if operation_id.is_some() {
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
}

/// What undoing the last operation did.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum UndoOutcome {
    /// Every change of the operation was reversed.
    Undone { operation: OperationInfo },
    /// No operation, or the last one is already undone.
    NothingToUndo,
    /// Undoing would overwrite later changes, so nothing was changed.
    Refused {
        operation: OperationInfo,
        conflicts: Vec<UndoConflict>,
    },
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

/// Undoes the most recent operation exactly: a set gets its old value back,
/// an inserted row is deleted, a deleted row is put back with its old id.
///
/// Refused, changing nothing, if anything the operation wrote has changed
/// since: undoing would silently throw that later change away.
pub fn undo_last(conn: &mut Connection) -> Result<UndoOutcome, OpsError> {
    let tx = conn.transaction()?;
    let latest: Option<(i64, String, Option<String>)> = tx
        .query_row(
            "SELECT id, kind, undone_at FROM operation ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((id, kind, None)) = latest else {
        return Ok(UndoOutcome::NothingToUndo);
    };
    let operation = OperationInfo { id, kind };

    let changes: Vec<ChangeRow> = {
        let mut stmt = tx.prepare(
            "SELECT entity, entity_id, action, field, before, after FROM change
             WHERE operation_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([id], |r| {
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
    let mut conflicts = Vec::new();
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
        let table = Table::load(&tx, &change.entity)?;
        match change.action.as_str() {
            "set" => undo_set(&tx, &table, change, &mut conflicts)?,
            "insert" => undo_insert(&tx, &table, group, &mut conflicts)?,
            "delete" => undo_delete(&tx, &table, group, &mut conflicts)?,
            other => return Err(OpsError::BadAction(other.to_owned())),
        }
        i += run;
    }

    if !conflicts.is_empty() {
        // Dropping `tx` rolls back the steps already taken.
        return Ok(UndoOutcome::Refused {
            operation,
            conflicts,
        });
    }
    tx.execute(
        "UPDATE operation SET undone_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        [id],
    )?;
    tx.commit()?;
    Ok(UndoOutcome::Undone { operation })
}

/// [`undo_last`], run on the database's one writer connection.
pub fn undo_last_via(writer: &Writer) -> Result<UndoOutcome, OpsError> {
    writer.call(|conn| Ok(undo_last(conn)))?
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
    table: &Table,
    change: &ChangeRow,
    conflicts: &mut Vec<UndoConflict>,
) -> Result<(), OpsError> {
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
    if table
        .referenced_by(tx, change.entity_id, ForeignKeyAction::Update, Some(&field))?
        .is_some()
    {
        conflicts.push(conflict(
            change,
            Some(&change.field),
            ConflictProblem::Referenced,
        ));
        return Ok(());
    }
    tx.execute(
        &format!(
            "UPDATE {} SET {} = ?1 WHERE rowid = ?2",
            table.sql_name(),
            quote(&column.name)
        ),
        params![decode(column, change.before.as_deref())?, change.entity_id],
    )?;
    Ok(())
}

fn undo_insert(
    tx: &Transaction<'_>,
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
    if table
        .referenced_by(tx, first.entity_id, ForeignKeyAction::Delete, None)?
        .is_some()
    {
        conflicts.push(conflict(first, None, ConflictProblem::Referenced));
        return Ok(());
    }
    tx.execute(
        &format!("DELETE FROM {} WHERE rowid = ?1", table.sql_name()),
        [first.entity_id],
    )?;
    Ok(())
}

fn undo_delete(
    tx: &Transaction<'_>,
    table: &Table,
    group: &[ChangeRow],
    conflicts: &mut Vec<UndoConflict>,
) -> Result<(), OpsError> {
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
    tx.execute(
        &format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table.sql_name(),
            names.join(", "),
            slots.join(", ")
        ),
        rusqlite::params_from_iter(values),
    )?;
    Ok(())
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

/// Undoes the most recent operation, unless something it wrote has changed
/// since. If the undo hits an error, nothing was changed.
// `async` runs it off the main thread, so the UI never waits on the writer.
#[tauri::command(async)]
#[specta::specta]
pub fn undo_last_operation(
    writer: tauri::State<'_, Writer>,
) -> Result<UndoOutcome, crate::ipc::IpcError> {
    Ok(undo_last_via(&writer)?)
}

#[cfg(test)]
mod tests;
