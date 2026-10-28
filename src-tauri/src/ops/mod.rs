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
//! **Automatic operations** ([`AutomaticKind`], made with
//! [`record_automatic`]) are changes the app makes on its own, such as a
//! Library track following its file to a new place. They are in the log
//! like any other, but Undo never lands on one: the user's next Undo takes
//! back their own last action (`next_operation` skips them by kind). When
//! that action is older than automatic operations on the very rows it
//! changed, undoing it takes those back with it, in the same transaction:
//! all of it or nothing. An automatic operation on other rows stays as it
//! is. So history is still linear for every row.
//!
//! What counts is where the rows end up: as they were before the user's
//! action. The automatic operations since are taken back as one net change
//! per field (what the oldest found, what the newest left), not replayed
//! step by step, so a value that only they passed through never has to be
//! written again. Undoing the add of a Library track that has since
//! followed its file deletes the track; it is not first linked back to the
//! file it followed from, whose row may be gone with its music folder.
//! Undo still refuses, changing nothing, when a field doesn't hold what the
//! newest automatic operation left (something else changed it since), and
//! when a value it does have to write back is refused by the database (the
//! row it names is gone).
//!
//! **What the log knows of the features above it.** Two things, both on
//! the Undo button's path only ([`undo_for_user`], [`refused_by_a_rule`]),
//! never in recording or in the unwinding itself: once an undo has
//! committed, the files of the Library tracks it changed are ranked again
//! ([`crate::file_roles`]), so what is derived from a link is right as the
//! undo returns (after an undone "Keep both" or merge of two Library tracks
//! a grouping pass runs first, [`crate::review::same`], so the pair is back
//! in Review); and one undo is refused by a feature's own rule, the
//! switch of a linked file that made a track a new one for rekordbox and
//! was sent since ([`crate::review::actions`]).
//!
//! Row ids name rows in the log, so a table that code outside the log
//! inserts into must never reuse an id (`AUTOINCREMENT`; `sync_base`, which
//! a send writes, and `conflict`, which detecting conflicts writes). A test
//! scans the source for such inserts. The same holds for the ids the log
//! stores as values: `file` and `recording`, which scans, folder removals
//! and the grouping job write outside the log, never reuse an id either
//! (migration 0031), so a recorded file or track id names its own row or
//! none, and an Undo whose row is gone is refused. A test checks every
//! table a logged table points at.

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
///
/// For a user's action. A kind that is an automatic operation's
/// ([`AutomaticKind`]) is refused: those are made with
/// [`record_automatic`], so no user action is ever skipped by Undo.
pub fn record<T>(
    conn: &mut Connection,
    kind: &str,
    details: &serde_json::Value,
    write: impl FnOnce(&mut Recorder<'_>) -> Result<T, OpsError>,
) -> Result<Recorded<T>, OpsError> {
    if AutomaticKind::of(kind).is_some() {
        return Err(OpsError::BadKind(kind.to_owned()));
    }
    record_as(conn, kind, details, write)
}

/// An operation the app makes on its own, with nobody asking for it. It is
/// logged like a user's action, but Undo skips it (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomaticKind {
    /// A Library track's linked file moved to the same audio at a new
    /// place ([`crate::library::follow`]).
    FollowFile,
}

impl AutomaticKind {
    /// Every automatic kind.
    pub const ALL: [AutomaticKind; 1] = [AutomaticKind::FollowFile];

    /// The kind as the log stores it.
    pub const fn as_str(self) -> &'static str {
        match self {
            AutomaticKind::FollowFile => "follow_file",
        }
    }

    /// The automatic kind stored as `kind`, if it is one.
    pub fn of(kind: &str) -> Option<AutomaticKind> {
        AutomaticKind::ALL.into_iter().find(|k| k.as_str() == kind)
    }

    /// The stored kinds as a SQL list, for `kind IN (...)`. They are this
    /// file's own constants, never input.
    fn sql_list() -> String {
        let kinds: Vec<String> = AutomaticKind::ALL
            .iter()
            .map(|k| format!("'{}'", k.as_str()))
            .collect();
        kinds.join(", ")
    }
}

/// [`record`], for an operation the app makes on its own: logged, and
/// undone only together with an earlier user action on the same rows.
pub fn record_automatic<T>(
    conn: &mut Connection,
    kind: AutomaticKind,
    details: &serde_json::Value,
    write: impl FnOnce(&mut Recorder<'_>) -> Result<T, OpsError>,
) -> Result<Recorded<T>, OpsError> {
    record_as(conn, kind.as_str(), details, write)
}

/// [`record`], and then, in the same transaction and only if the
/// operation recorded something, `derived`: writes to derived state that
/// follows from the operation (a snapshot another job works out again),
/// which are not in the log and which Undo doesn't take back. The operation
/// and those writes land together or not at all. `derived` gets the
/// operation's transaction: it must write only tables the log never records.
pub fn record_with_derived<T>(
    conn: &mut Connection,
    kind: &str,
    details: &serde_json::Value,
    write: impl FnOnce(&mut Recorder<'_>) -> Result<T, OpsError>,
    derived: impl FnOnce(&Connection) -> rusqlite::Result<()>,
) -> Result<Recorded<T>, OpsError> {
    if AutomaticKind::of(kind).is_some() {
        return Err(OpsError::BadKind(kind.to_owned()));
    }
    record_then(conn, kind, details, write, Some(derived))
}

/// [`record_with_derived`], with writes to derived state made first, in the
/// same transaction, before the operation's own: `first` makes what the
/// recorded writes need and the log never names as a row (the new track a
/// split in Review moves a file into, made as the grouping job makes tracks,
/// 1bE-2), and hands it to `write`. If `write` records nothing, `first` is
/// rolled back with it. `first` must write only tables the log never
/// records.
pub fn record_with_derived_first<P, T>(
    conn: &mut Connection,
    kind: &str,
    details: &serde_json::Value,
    first: impl FnOnce(&Connection) -> rusqlite::Result<P>,
    write: impl FnOnce(&mut Recorder<'_>, P) -> Result<T, OpsError>,
    derived: impl FnOnce(&Connection) -> rusqlite::Result<()>,
) -> Result<Recorded<T>, OpsError> {
    if AutomaticKind::of(kind).is_some() {
        return Err(OpsError::BadKind(kind.to_owned()));
    }
    record_around(conn, kind, details, first, write, Some(derived))
}

fn record_as<T>(
    conn: &mut Connection,
    kind: &str,
    details: &serde_json::Value,
    write: impl FnOnce(&mut Recorder<'_>) -> Result<T, OpsError>,
) -> Result<Recorded<T>, OpsError> {
    record_then(
        conn,
        kind,
        details,
        write,
        None::<fn(&Connection) -> rusqlite::Result<()>>,
    )
}

fn record_then<T>(
    conn: &mut Connection,
    kind: &str,
    details: &serde_json::Value,
    write: impl FnOnce(&mut Recorder<'_>) -> Result<T, OpsError>,
    derived: Option<impl FnOnce(&Connection) -> rusqlite::Result<()>>,
) -> Result<Recorded<T>, OpsError> {
    record_around(
        conn,
        kind,
        details,
        |_| Ok(()),
        |rec, ()| write(rec),
        derived,
    )
}

fn record_around<P, T>(
    conn: &mut Connection,
    kind: &str,
    details: &serde_json::Value,
    first: impl FnOnce(&Connection) -> rusqlite::Result<P>,
    write: impl FnOnce(&mut Recorder<'_>, P) -> Result<T, OpsError>,
    derived: Option<impl FnOnce(&Connection) -> rusqlite::Result<()>>,
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
    let prepared = first(&tx)?;
    let value = write(&mut recorder, prepared)?;
    let operation_id = (recorder.written > 0).then_some(recorder.operation_id);
    if let Some(id) = operation_id {
        if let Some(derived) = derived {
            derived(&tx)?;
        }
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
    /// A track the step added has been sent to rekordbox since, or a field
    /// whose conflict the step settled.
    SentSince,
    /// The step would put a Library track back, but the track or the file
    /// it was linked to is no longer in the database (its music folder was
    /// removed, or grouping merged the track away).
    SourceGone,
    /// The step would bring back a crate name another crate has now.
    CrateNameTaken { name: String },
    /// Something the step changed has changed again since.
    ChangedSince,
    /// The step switched a Library track's linked file so that the track
    /// goes to rekordbox as a new track, and the track has been sent since:
    /// the copy's entry is in rekordbox, and taking the switch back would
    /// leave it there with nothing to list it.
    SwitchSentSince,
    /// The step merged Library tracks that are the same recording, and the
    /// track that stayed is linked to another file since (it followed its
    /// file to one the merge moved, or was switched): undoing the merge would
    /// leave two Library tracks on one file.
    MergeLinkChanged,
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

/// The newest of the user's operations that isn't undone: the next one to
/// undo. An automatic operation ([`AutomaticKind`]) is never it.
fn next_operation(tx: &Transaction<'_>) -> Result<Option<OperationInfo>, OpsError> {
    Ok(tx
        .query_row(
            &format!(
                "SELECT id, kind, details FROM operation
                 WHERE undone_at IS NULL AND kind NOT IN ({})
                 ORDER BY id DESC LIMIT 1",
                AutomaticKind::sql_list()
            ),
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

/// The automatic operations made after `operation` that aren't undone and
/// changed a row `operation` changed too, newest first: what has to be
/// taken back before `operation` can be.
fn automatic_since(
    tx: &Transaction<'_>,
    operation: &OperationInfo,
) -> Result<Vec<OperationInfo>, OpsError> {
    let newer: Vec<(OperationInfo, String, i64)> = {
        let mut stmt = tx.prepare(&format!(
            "SELECT o.id, o.kind, o.details, c.entity, c.entity_id
             FROM operation o JOIN change c ON c.operation_id = o.id
             WHERE o.id > ?1 AND o.undone_at IS NULL AND o.kind IN ({})
             ORDER BY o.id DESC, c.id",
            AutomaticKind::sql_list()
        ))?;
        let rows = stmt.query_map([operation.id], |r| {
            Ok((
                OperationInfo {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    details: OperationDetails::parse(r.get_ref(2)?.as_str()?),
                },
                r.get(3)?,
                r.get(4)?,
            ))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    // Nearly always: nothing automatic happened since.
    if newer.is_empty() {
        return Ok(Vec::new());
    }
    let mine: std::collections::HashSet<(String, i64)> = {
        let mut stmt =
            tx.prepare("SELECT DISTINCT entity, entity_id FROM change WHERE operation_id = ?1")?;
        let rows = stmt.query_map([operation.id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut found: Vec<OperationInfo> = Vec::new();
    for (automatic, entity, entity_id) in newer {
        if found.last().is_some_and(|last| last.id == automatic.id) {
            continue;
        }
        if mine.contains(&(entity, entity_id)) {
            found.push(automatic);
        }
    }
    Ok(found)
}

/// A field of a row, as the log names it: table, row id, field.
type FieldKey = (String, i64, String);

/// What the automatic operations since did to one field, end to end.
struct Net {
    /// What the oldest of them found.
    before: Option<String>,
    /// What the newest of them left.
    after: Option<String>,
}

/// The changes of one operation, oldest first.
fn changes_of(tx: &Transaction<'_>, operation: i64) -> Result<Vec<ChangeRow>, OpsError> {
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

/// [`unwind`] for a user's operation, with the automatic operations made
/// since on the rows it changed ([`automatic_since`]): the rows end up as
/// they were before the user's operation, and all of those operations are
/// marked undone. If anything stands in the way, the whole undo is refused
/// and `tx` must be rolled back.
///
/// The automatic operations are taken back as one net change per field,
/// not one after the other (see the module docs):
/// - every field they set must hold what the newest of them left, and each
///   must have found what the one before it left; else something changed it
///   since, and the undo is refused as "changed since";
/// - a field the user's operation wrote itself (it set it, or inserted the
///   row) is then taken to hold what the oldest of them found, and the
///   user's operation is undone from there: the value in between is never
///   written;
/// - a field the user's operation didn't write gets back what the oldest of
///   them found, for real. If the database refuses that value, the undo is
///   refused.
fn unwind_with_automatic(
    tx: &Transaction<'_>,
    operation: &OperationInfo,
) -> Result<Found, OpsError> {
    let automatic = automatic_since(tx, operation)?;
    if automatic.is_empty() {
        return unwind(tx, operation, &HashMap::new());
    }
    // Oldest first.
    let mut theirs: Vec<ChangeRow> = Vec::new();
    for op in automatic.iter().rev() {
        theirs.extend(changes_of(tx, op.id)?);
    }
    // An automatic operation that made or deleted rows has no net change
    // per field: those are taken back one after the other, newest first.
    if theirs.iter().any(|c| c.action != "set") {
        for op in &automatic {
            let found = unwind(tx, op, &HashMap::new())?;
            if !found.conflicts.is_empty() {
                return Ok(Found {
                    conflicts: found.conflicts,
                    ..Found::default()
                });
            }
        }
        return unwind(tx, operation, &HashMap::new());
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
    let mine = changes_of(tx, operation.id)?;
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

    let found = unwind(tx, operation, &taken_to_hold)?;
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

/// What stands in the way of undoing one operation.
#[derive(Default)]
struct Found {
    conflicts: Vec<UndoConflict>,
    /// A crate name the undo would bring back that another crate has now.
    name_taken: Option<String>,
    /// A Library track the operation added can't be taken out again: rows
    /// that must not lose it point at it, which only a send makes.
    held_by_a_send: bool,
    /// A Library track the operation removed can't be put back: the track
    /// or file it points at is gone.
    source_gone: bool,
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
        let marked_sent = self.conflicts.iter().any(|c| {
            c.entity == "library_track"
                && c.problem == ConflictProblem::ChangedSince
                && matches!(
                    c.field.as_deref(),
                    Some("last_sent_location" | "last_exported_at")
                )
        });
        Some(if marked_sent || self.held_by_a_send {
            UndoRefusal::SentSince
        } else if self.source_gone {
            UndoRefusal::SourceGone
        } else {
            UndoRefusal::ChangedSince
        })
    }
}

/// Reverses every change of `operation` inside `tx` and marks it undone,
/// unless something stands in the way. Then `tx` holds a half-undone state
/// and must be rolled back.
///
/// `taken_to_hold` names fields that are taken to hold another value than
/// the database has: what they held before the automatic operations since
/// ([`unwind_with_automatic`], which has checked what they hold now).
fn unwind(
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

/// Why a feature's own rule refuses to undo `operation`, whatever its rows
/// hold: asked before anything is unwound, by [`undo_step`] and by
/// [`next_undo`] alike. Three rules: a switch of a linked file that made the
/// track a new one for rekordbox, sent since
/// ([`crate::review::actions::sent_as_new_since`]); a merge of Library
/// tracks whose staying track is linked to another file since
/// ([`crate::review::same::merge_link_changed_since`]); and a settled
/// conflict whose field a send carried since
/// ([`crate::conflicts::resolve::sent_since`]). It is the counterpart of
/// "sent since" for an add ([`Found::refusal`]), which the rows themselves
/// show; a switch writes nothing a send changes, so it has to be asked.
fn refused_by_a_rule(
    tx: &Transaction<'_>,
    operation: &OperationInfo,
) -> Result<Option<UndoRefusal>, OpsError> {
    if crate::review::actions::sent_as_new_since(tx, operation.id)? {
        return Ok(Some(UndoRefusal::SwitchSentSince));
    }
    if crate::review::same::merge_link_changed_since(tx, operation.id)? {
        return Ok(Some(UndoRefusal::MergeLinkChanged));
    }
    if crate::conflicts::resolve::sent_since(tx, operation.id)? {
        return Ok(Some(UndoRefusal::SentSince));
    }
    Ok(crate::review::split::undo_refused(tx, operation.id)?.then_some(UndoRefusal::ChangedSince))
}

fn undo_step(conn: &mut Connection, only: Option<i64>) -> Result<UndoOutcome, OpsError> {
    let tx = conn.transaction()?;
    let Some(operation) = next_operation(&tx)? else {
        return Ok(UndoOutcome::NothingToUndo);
    };
    if only.is_some_and(|id| id != operation.id) {
        return Ok(UndoOutcome::NothingToUndo);
    }
    if let Some(reason) = refused_by_a_rule(&tx, &operation)? {
        return Ok(UndoOutcome::Refused {
            operation,
            reason,
            conflicts: Vec::new(),
        });
    }
    let found = unwind_with_automatic(&tx, &operation)?;
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

/// Undoes the newest of the user's operations that isn't undone yet,
/// exactly: a set gets its old value back, an inserted row is deleted, a
/// deleted row is put back with its old id. Call it again to undo the one
/// before. An automatic operation is never the one undone; those made since
/// on the same rows are taken back with it (see the module docs).
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

/// [`undo_last`] (or [`undo_only`], with `only`) as the Undo button runs
/// it: once the undo has committed, the files of the Library tracks it
/// changed are ranked again where that is due, so what follows a track's
/// linked file (its "better file", and with it the Review queue) is right
/// as the undo returns, not when the roles job next runs. An undo that put
/// a switched link back brings the "Linked file gone, copy found" item back
/// with it, and an undone relink confirmation brings its "Confirm relink"
/// item back ([`crate::relink::decide::after_undo`]).
///
/// The ranking is derived state: if it fails here the undo stands, the
/// failure is logged, and the roles job ranks the tracks, which are still
/// due.
pub fn undo_for_user(conn: &mut Connection, only: Option<i64>) -> Result<UndoOutcome, OpsError> {
    let outcome = undo_step(conn, only)?;
    if let UndoOutcome::Undone { operation } = &outcome {
        // An undone "Keep both" or merge of two Library tracks: the grouping
        // pass puts the pair back in the queue (derived state, as below).
        crate::review::same::after_undo(conn, operation.id, &operation.kind);
        // An undone "these are the same track", "not the same" or split:
        // the same, for the item's files (1bE-2).
        crate::review::split::after_undo(conn, operation.id, &operation.kind);
        if let Err(e) = rank_again_after(conn, operation.id) {
            eprintln!(
                "undo of operation {}: couldn't rank its tracks again: {e}",
                operation.id
            );
        }
        // An undone confirmation or "not the same": the entry's rows follow
        // the `relink` table again (derived state too; the relink run the
        // command asks for decides them from scratch).
        if let Err(e) = crate::relink::decide::after_undo(conn, operation.id) {
            eprintln!(
                "undo of operation {}: couldn't bring its relink matches back: {e}",
                operation.id
            );
        }
    }
    Ok(outcome)
}

/// Ranks again the tracks whose Library track `operation` changed, the
/// tracks it moved files between (a merge of two Library tracks), and
/// those linked to the audio a dismissal it made or took back was about
/// (which better file a track is offered skips dismissed ones), where they
/// are due (a database trigger made them so): at most one batch of the roles
/// job, the rest is the job's.
fn rank_again_after(conn: &mut Connection, operation: i64) -> rusqlite::Result<()> {
    let due: Vec<i64> = conn
        .prepare_cached(&format!(
            "SELECT recording_id FROM best_file_due WHERE recording_id IN (
                 SELECT lt.recording_id FROM change c
                 JOIN library_track lt ON lt.id = c.entity_id
                 WHERE c.operation_id = ?1 AND c.entity = 'library_track'
                 UNION
                 SELECT CAST(coalesce(c.before, c.after) AS INTEGER) FROM change c
                 WHERE c.operation_id = ?1 AND c.entity = 'library_track'
                   AND c.field = 'recording_id'
                 UNION
                 SELECT CAST(c.before AS INTEGER) FROM change c
                 WHERE c.operation_id = ?1 AND c.entity = 'recording_file'
                   AND c.field = 'recording_id'
                 UNION
                 SELECT CAST(c.after AS INTEGER) FROM change c
                 WHERE c.operation_id = ?1 AND c.entity = 'recording_file'
                   AND c.field = 'recording_id'
                 UNION
                 SELECT CAST(json_extract(o.details, '$.recordingId') AS INTEGER)
                 FROM operation o WHERE o.id = ?1 AND o.kind = '{}'
                 UNION
                 SELECT lt.recording_id FROM library_track lt
                 JOIN file f ON f.id = lt.linked_file_id
                 WHERE lower(hex({})) IN (
                     SELECT coalesce(c.before, c.after) FROM change c
                     WHERE c.operation_id = ?1 AND c.entity = 'review_decision'
                       AND c.field = 'linked_key'))
             ORDER BY recording_id LIMIT ?2",
            crate::hidden::USE_THIS_FILE_OPERATION,
            crate::grouping::CONTENT_KEY
        ))?
        .query_map(params![operation, crate::file_roles::BATCH as i64], |r| {
            r.get(0)
        })?
        .collect::<rusqlite::Result<_>>()?;
    crate::file_roles::settle_tracks(conn, &due).map(|_| ())
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
    let refusal = match refused_by_a_rule(&tx, &operation)? {
        Some(refusal) => Some(refusal),
        None if changes <= TRIED_AHEAD_UP_TO => unwind_with_automatic(&tx, &operation)?.refusal(),
        None => None,
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
    taken_to_hold: &HashMap<FieldKey, Option<String>>,
) -> Result<(), OpsError> {
    // A derived column is kept current by whatever it follows, not by the
    // log: what it held before the operation means nothing now, so Undo
    // leaves it to that (a moved file's role is the ranking's, worked out
    // again for the tracks the undo changed).
    if is_derived(&table.name, &change.field) {
        return Ok(());
    }
    let conflicts = &mut found.conflicts;
    let column = table.column(&change.field)?;
    // Nearly always empty: no key is built then.
    let held = (!taken_to_hold.is_empty())
        .then(|| {
            taken_to_hold.get(&(
                change.entity.clone(),
                change.entity_id,
                change.field.clone(),
            ))
        })
        .flatten();
    let now = match held {
        Some(held) => held.clone(),
        None => match table.read_field(tx, change.entity_id, column) {
            Ok(now) => now,
            Err(OpsError::RowNotFound { .. }) => {
                conflicts.push(conflict(change, None, ConflictProblem::RowGone));
                return Ok(());
            }
            Err(e) => return Err(e),
        },
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
    // A file moved back to the track it came from starts over there, as any
    // move does: its role and match confidence are the ranking's (derived),
    // and a track has one best file, so the role goes first.
    if starts_over(&table.name, &change.field) {
        tx.prepare_cached(
            "UPDATE recording_file SET role = 'undecided', match_confidence = NULL
             WHERE rowid = ?1",
        )?
        .execute([change.entity_id])?;
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
    found: &mut Found,
    taken_to_hold: &HashMap<FieldKey, Option<String>>,
) -> Result<(), OpsError> {
    let conflicts = &mut found.conflicts;
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
        let held = (!taken_to_hold.is_empty())
            .then(|| {
                taken_to_hold.get(&(
                    change.entity.clone(),
                    change.entity_id,
                    change.field.clone(),
                ))
            })
            .flatten();
        let now = held.unwrap_or(&row[index]);
        if *now != change.after {
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
        found.held_by_a_send |= first.entity == "library_track";
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
        found.source_gone |= first.entity == "library_track";
    }
    Ok(())
}

/// The derived columns of the logged tables, by table: a column that
/// follows other data and that something outside the log keeps current, so
/// its moving on is nobody's edit. **The one list of them.** Undoing an
/// insert doesn't treat a change to one as "changed since" (every other
/// column of the row still must hold what the operation left), and undoing
/// a delete puts back what the log holds and lets whatever keeps the column
/// current set it again.
///
/// - `library_track.source_status` follows the linked file's `present`
///   (database triggers).
/// - `relink.audio_hash` is the audio a file held when the user confirmed a
///   match, and `relink.file_key` what a confirmation is kept by. When the
///   file wasn't hashed yet they are stored empty, and the relink job fills
///   them in once it is (never over a value that is there): undoing the
///   confirmation must still work after that.
/// - `relink.file_id` only points at the file the user looked at; removing
///   that file's music folder lets go of it (the foreign key sets it to
///   NULL), and the decision stands by what the file held. Undoing it must
///   still work after that.
///
/// - `recording_file.role` and `recording_file.match_confidence` are the
///   ranking's (the roles job, [`crate::file_roles`]): a file that changes
///   track starts over in its new one, and the ranking works it out again.
///   Only an action that moves files between tracks writes them, through
///   the one function that moves a file (`review::same::move_file`): a merge
///   of two Library tracks (Review, 1bE-9), "these are the same track" and a
///   split (Review, 1bE-2).
///
/// Undoing a **set** of a derived column leaves the column as it is: no
/// "changed since" check, nothing of the value before written back. The
/// operations that set one move files, and their Undo ranks the tracks they
/// changed again; a file the Undo moves back starts over in its track, as
/// every move does (`starts_over`: role `undecided`, no match confidence).
/// No other operation sets a derived column (a test holds the source to
/// that).
///
/// A test holds every entry to what the "changed since" check skips; a new
/// entry needs its own case there.
pub(crate) const DERIVED_FIELDS: &[(&str, &str)] = &[
    ("library_track", "source_status"),
    ("relink", "audio_hash"),
    ("relink", "file_key"),
    ("relink", "file_id"),
    ("recording_file", "role"),
    ("recording_file", "match_confidence"),
];

fn is_derived(entity: &str, field: &str) -> bool {
    DERIVED_FIELDS.contains(&(entity, field))
}

/// A set of this field moves a file to another track, so undoing it moves the
/// file back, and the file starts over there: its derived role and match
/// confidence (`DERIVED_FIELDS`) are reset first, as every move resets them
/// (a split in Review moves a file that may be its new track's best back
/// into a track that has one, 1bE-2). The ranking then works them out again.
fn starts_over(entity: &str, field: &str) -> bool {
    (entity, field) == ("recording_file", "recording_id")
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
    jobs: tauri::State<'_, crate::jobs::JobQueue>,
    operation_id: Option<i64>,
) -> Result<UndoOutcome, crate::ipc::IpcError> {
    let outcome = writer.call(move |conn| Ok(undo_for_user(conn, operation_id)))??;
    if let UndoOutcome::Undone { operation } = &outcome {
        // Whatever else the undo made due is worked out in the background.
        if crate::relink::decide::is_operation(&operation.kind) {
            crate::review::actions::ask_for_relink(&writer, &jobs);
        }
        if crate::review::same::is_operation(&operation.kind)
            || crate::review::split::is_operation(&operation.kind)
        {
            crate::review::same::ask_for_merge_follow_up(&writer, &jobs);
        } else {
            crate::review::actions::ask_for_follow_up_work(&writer, &jobs);
        }
    }
    Ok(outcome)
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
pub(crate) mod history_tests;
#[cfg(test)]
mod tests;
