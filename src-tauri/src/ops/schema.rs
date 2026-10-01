//! Which tables and fields the operation log can record, and how a field's
//! value is stored as text in `change.before` / `change.after`.
//!
//! `change` names a table and column as text, and they end up in SQL, so
//! both are checked against the live schema before use. Only STRICT rowid
//! tables are allowed: STRICT means each column holds exactly one type, so a
//! stored text value turns back into exactly the value it came from; rowid
//! means `change.entity_id` identifies the row.

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};

use super::OpsError;

/// Tables the log never records changes to: its own tables, and the
/// migration bookkeeping.
const NOT_RECORDABLE: &[&str] = &["operation", "change", "schema_migration"];

/// A column's type in a STRICT table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ColumnType {
    Integer,
    Real,
    Text,
    Blob,
}

/// One column that changes are recorded for.
#[derive(Debug, Clone)]
pub(super) struct Column {
    pub name: String,
    pub ty: ColumnType,
}

/// A table the log can record changes to, with the columns it records: every
/// ordinary column except the rowid alias (`id INTEGER PRIMARY KEY`), which
/// is `change.entity_id` itself.
#[derive(Debug, Clone)]
pub(super) struct Table {
    pub name: String,
    pub columns: Vec<Column>,
}

impl Table {
    /// Looks `entity` up in the live schema and checks it can be recorded.
    pub fn load(conn: &Connection, entity: &str) -> Result<Table, OpsError> {
        let refuse = || OpsError::NotRecordable(entity.to_owned());
        if NOT_RECORDABLE.contains(&entity) || entity.starts_with("sqlite_") {
            return Err(refuse());
        }
        let shape: Option<(bool, bool)> = conn
            .query_row(
                "SELECT wr, strict FROM pragma_table_list
                 WHERE schema = 'main' AND type = 'table' AND name = ?1",
                [entity],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match shape {
            // WITHOUT ROWID: `entity_id` can't name a row.
            Some((false, true)) => {}
            _ => return Err(refuse()),
        }

        let mut stmt = conn.prepare(
            "SELECT name, upper(type), pk, hidden FROM pragma_table_xinfo(?1, 'main') ORDER BY cid",
        )?;
        let rows = stmt
            .query_map([entity], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let pk_columns = rows.iter().filter(|(_, _, pk, _)| *pk > 0).count();
        let mut columns = Vec::new();
        for (name, ty, pk, hidden) in rows {
            // Generated columns can't be written; they follow the others.
            if hidden != 0 {
                continue;
            }
            if pk == 1 && pk_columns == 1 && ty == "INTEGER" {
                continue; // The rowid alias.
            }
            let ty = match ty.as_str() {
                "INT" | "INTEGER" => ColumnType::Integer,
                "REAL" => ColumnType::Real,
                "TEXT" => ColumnType::Text,
                "BLOB" => ColumnType::Blob,
                // ANY: a stored text value couldn't say which type it was.
                _ => return Err(refuse()),
            };
            columns.push(Column { name, ty });
        }
        Ok(Table {
            name: entity.to_owned(),
            columns,
        })
    }

    /// The column called `field`, if changes to it can be recorded.
    pub fn column(&self, field: &str) -> Result<&Column, OpsError> {
        self.columns
            .iter()
            .find(|c| c.name == field)
            .ok_or_else(|| OpsError::NotAField {
                entity: self.name.clone(),
                field: field.to_owned(),
            })
    }

    /// The table's name, quoted for SQL and qualified with `main`, so a
    /// TEMP table of the same name can't stand in for it. Safe: it came
    /// from the schema.
    pub fn sql_name(&self) -> String {
        format!("main.{}", quote(&self.name))
    }

    /// The queries that say whether row `id` of this table is referenced
    /// by a row of a table that would silently change with it, through a
    /// foreign key whose `on` action (`on_delete` or `on_update`) is
    /// CASCADE, SET NULL or SET DEFAULT. The log can't record those
    /// knock-on changes, so a write that would cause them is refused. With
    /// `columns`, only foreign keys that point at one of those columns
    /// count (an update of just those columns). One query per such key,
    /// found from the schema alone. A bulk operation works this out once per table (the schema
    /// can't change inside it) and runs [`first_referencing`] per row.
    pub fn reference_checks(
        &self,
        conn: &Connection,
        on: ForeignKeyAction,
        columns: Option<&[&str]>,
    ) -> Result<Vec<ReferenceCheck>, OpsError> {
        let action_column = match on {
            ForeignKeyAction::Delete => "on_delete",
            ForeignKeyAction::Update => "on_update",
        };
        let sql = format!(
            "SELECT t.name, fk.id, fk.\"from\", fk.\"to\"
             FROM pragma_table_list AS t
             JOIN pragma_foreign_key_list(t.name, 'main') AS fk
             WHERE t.schema = 'main' AND t.type = 'table' AND fk.\"table\" = ?1
               AND upper(fk.{action_column}) IN ('CASCADE', 'SET NULL', 'SET DEFAULT')
             ORDER BY t.name, fk.id, fk.seq"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map([&self.name], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // One foreign key can span several columns: group by (table, id).
        let mut keys: Vec<(String, Vec<ColumnPair>)> = Vec::new();
        let mut last: Option<(String, i64)> = None;
        for (child, fk_id, from, to) in rows {
            if last.as_ref() != Some(&(child.clone(), fk_id)) {
                keys.push((child.clone(), Vec::new()));
                last = Some((child, fk_id));
            }
            keys.last_mut().expect("just pushed").1.push((from, to));
        }

        let primary_key = self.primary_key(conn)?;
        let mut checks = Vec::new();
        for (child, pairs) in keys {
            // `to` is NULL when the key points at the parent's primary key.
            let parent_columns: Vec<String> = if pairs.iter().all(|(_, to)| to.is_some()) {
                pairs.iter().map(|(_, to)| to.clone().unwrap()).collect()
            } else {
                primary_key.clone()
            };
            if let Some(only) = columns {
                if !parent_columns.iter().any(|c| only.contains(&c.as_str())) {
                    continue;
                }
            }
            let matches: Vec<String> = pairs
                .iter()
                .zip(&parent_columns)
                .map(|((from, _), to)| format!("c.{} = p.{}", quote(from), quote(to)))
                .collect();
            let sql = format!(
                "SELECT EXISTS (SELECT 1 FROM main.{} AS c, {} AS p
                                WHERE p.rowid = ?1 AND {})",
                quote(&child),
                self.sql_name(),
                matches.join(" AND ")
            );
            checks.push(ReferenceCheck { child, sql });
        }
        Ok(checks)
    }

    /// The primary key's columns, in key order; `rowid` when there's none.
    fn primary_key(&self, conn: &Connection) -> Result<Vec<String>, OpsError> {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info(?1, 'main') WHERE pk > 0 ORDER BY pk")?;
        let names = stmt
            .query_map([&self.name], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(if names.is_empty() {
            vec!["rowid".to_owned()]
        } else {
            names
        })
    }

    /// Every recorded field of row `id`, or `None` if there's no such row.
    pub fn read_row(
        &self,
        conn: &Connection,
        id: i64,
    ) -> Result<Option<Vec<Option<String>>>, OpsError> {
        if self.columns.is_empty() {
            let exists = conn
                .query_row(
                    &format!("SELECT 1 FROM {} WHERE rowid = ?1", self.sql_name()),
                    [id],
                    |_| Ok(()),
                )
                .optional()?;
            return Ok(exists.map(|()| Vec::new()));
        }
        let list: Vec<String> = self.columns.iter().map(|c| quote(&c.name)).collect();
        let sql = format!(
            "SELECT {} FROM {} WHERE rowid = ?1",
            list.join(", "),
            self.sql_name()
        );
        Ok(conn
            .prepare_cached(&sql)?
            .query_row([id], |r| {
                (0..self.columns.len())
                    .map(|i| r.get::<_, Value>(i).map(|v| encode(&v)))
                    .collect()
            })
            .optional()?)
    }

    /// One field of row `id` as stored text, or `Err(RowNotFound)`.
    pub fn read_field(
        &self,
        conn: &Connection,
        id: i64,
        column: &Column,
    ) -> Result<Option<String>, OpsError> {
        let sql = format!(
            "SELECT {} FROM {} WHERE rowid = ?1",
            quote(&column.name),
            self.sql_name()
        );
        conn.prepare_cached(&sql)?
            .query_row([id], |r| r.get::<_, Value>(0))
            .optional()?
            .map(|v| encode(&v))
            .ok_or_else(|| OpsError::RowNotFound {
                entity: self.name.clone(),
                id,
            })
    }
}

/// One query that says whether a row is referenced by a table that would
/// silently change with it (see [`Table::reference_checks`]).
#[derive(Debug, Clone)]
pub(super) struct ReferenceCheck {
    child: String,
    sql: String,
}

/// The first of `checks` whose table has a row referencing row `id`, by
/// table name.
pub(super) fn first_referencing(
    conn: &Connection,
    checks: &[ReferenceCheck],
    id: i64,
) -> Result<Option<String>, OpsError> {
    for check in checks {
        let referenced: bool = conn
            .prepare_cached(&check.sql)?
            .query_row([id], |r| r.get(0))?;
        if referenced {
            return Ok(Some(check.child.clone()));
        }
    }
    Ok(None)
}

/// One column of a foreign key: the child's column, and the parent's (none
/// when the key points at the parent's primary key).
type ColumnPair = (String, Option<String>);

/// Which change to a parent row a foreign key's action is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ForeignKeyAction {
    Delete,
    Update,
}

/// Quotes an identifier for SQL.
pub(super) fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// A value as `change.before` / `change.after` store it. NULL stays NULL.
///
/// Numbers use their shortest exact form (`2`, `128.5`), so text compares
/// equal exactly when the values do and parses back to the same value. BLOBs
/// are lowercase hex.
pub(super) fn encode(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Integer(i) => Some(i.to_string()),
        Value::Real(f) => Some(f.to_string()),
        Value::Text(s) => Some(s.clone()),
        Value::Blob(b) => Some(b.iter().map(|byte| format!("{byte:02x}")).collect()),
    }
}

/// The stored text of a field back as the value it came from.
pub(super) fn decode(column: &Column, text: Option<&str>) -> Result<Value, OpsError> {
    let Some(text) = text else {
        return Ok(Value::Null);
    };
    let bad = || OpsError::BadStoredValue {
        field: column.name.clone(),
        value: text.to_owned(),
    };
    Ok(match column.ty {
        ColumnType::Integer => Value::Integer(text.parse().map_err(|_| bad())?),
        ColumnType::Real => Value::Real(text.parse().map_err(|_| bad())?),
        ColumnType::Text => Value::Text(text.to_owned()),
        ColumnType::Blob => {
            if text.len() % 2 != 0 || !text.is_ascii() {
                return Err(bad());
            }
            let bytes = (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|_| bad())?;
            Value::Blob(bytes)
        }
    })
}
