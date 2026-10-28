//! Settling conflicts: keep the app's value, keep rekordbox's, or give one
//! by hand; one conflict or many at once (1cA-2; ROADMAP 1.10).
//!
//! Each call is one operation in the log ([`crate::ops`]), so one Undo
//! takes it back, a bulk resolve included: the conflicts are open again
//! with all three values.
//!
//! **A settled conflict is a new row.** The table ties `status`,
//! `resolved_at` and `resolved_value` together with CHECKs, and the log
//! writes (and undoes) one field at a time, so no order of single-field
//! writes gets a row from open to settled. A resolve therefore deletes the
//! open row and inserts the settled one, with the same values and
//! `detected_at`, both through the log: the settled conflict has a new id
//! ([`Resolved::conflict`]; the open one's was [`Resolved::was`]), and Undo
//! puts the open row back under its old id. Ids are never reused (migration
//! 0025).
//!
//! **The app's side takes the value chosen.** Settling records the decision;
//! [`super::detect`] then answers the field with the value chosen
//! ([`super::Outcome::Decided`]) for as long as the decision stands. In the
//! same operation, so one Undo takes both back, [`resolve`] and
//! [`resolve_all`] update the store of the app's own values
//! ([`super::app_values::adopt`]): keeping rekordbox's clears the app's
//! own, keeping the app's or a value by hand sets it. Without that, once a
//! send moved the base, the value turned down would read as "only the app
//! changed" and go out again (1cB-3). [`resolve_with`] and
//! [`resolve_all_with`] take another hook in its place.
//!
//! Undo is refused, changing nothing, if the settled row is gone or
//! changed, or if the field has a newer open conflict (a side moved after
//! the resolve): there can be one open conflict per field.

use std::fmt;

use rusqlite::types::Value;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::store::{
    conflict_from, Conflict, ConflictId, Filter, Status, CONFLICT_COLUMNS, OPEN_WHERE,
};
use super::{app_values, fits, same};
use crate::library::LibraryTrackId;
use crate::ops::{self, OpsError, Recorded, Recorder};

/// The operation kind of [`resolve`].
pub const RESOLVE_OPERATION: &str = "resolve_conflict";
/// The operation kind of [`resolve_all`].
pub const RESOLVE_ALL_OPERATION: &str = "resolve_conflicts";

/// How the user settles a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "choice", content = "value", rename_all = "camelCase")]
pub enum Choice {
    KeepApp,
    KeepRekordbox,
    /// A value given by hand. It must fit the field ([`fits`]). One that is
    /// the same value as a side's is recorded as keeping that side.
    Edit(String),
}

impl Choice {
    /// The code an operation's details name the choice by.
    fn code(&self) -> &'static str {
        match self {
            Choice::KeepApp => "keepApp",
            Choice::KeepRekordbox => "keepRekordbox",
            Choice::Edit(_) => "edit",
        }
    }
}

/// One conflict, settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    /// The settled conflict's row.
    pub conflict: ConflictId,
    /// The open conflict it was. That row is gone; Undo brings it back.
    pub was: ConflictId,
    pub library_track: LibraryTrackId,
    pub field: String,
    /// [`Status::KeptApp`], [`Status::KeptRekordbox`] or [`Status::Edited`].
    pub status: Status,
    /// The value chosen: what the next send writes for the field.
    pub value: String,
}

/// Why nothing was settled. Nothing was changed.
#[derive(Debug)]
pub enum ResolveError {
    /// The database or the operation log refused.
    Ops(OpsError),
    /// There's no open conflict with that id: it was settled or withdrawn.
    NotOpen { conflict: ConflictId },
    /// The value given by hand doesn't fit this field (a rating that isn't
    /// one of rekordbox's six, a color that isn't `0xRRGGBB`, …).
    BadValue { field: String, value: String },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::Ops(e) => write!(f, "{e}"),
            ResolveError::NotOpen { conflict } => {
                write!(f, "conflict {} is not open", conflict.0)
            }
            ResolveError::BadValue { field, value } => {
                write!(f, "{value:?} is not a value of {field:?}")
            }
        }
    }
}

impl std::error::Error for ResolveError {}

impl From<OpsError> for ResolveError {
    fn from(e: OpsError) -> Self {
        ResolveError::Ops(e)
    }
}

/// Settles one open conflict, as one operation, and the store of the
/// app's own values takes the value chosen ([`app_values::adopt`]).
pub fn resolve(
    conn: &mut Connection,
    id: ConflictId,
    choice: &Choice,
) -> Result<Recorded<Resolved>, ResolveError> {
    resolve_with(conn, id, choice, app_values::adopt)
}

/// [`resolve`], with `adopt` run inside the same operation once the
/// conflict is settled, in place of [`app_values::adopt`]: what it writes
/// there is undone with the resolve.
pub fn resolve_with(
    conn: &mut Connection,
    id: ConflictId,
    choice: &Choice,
    mut adopt: impl FnMut(&mut Recorder<'_>, &Resolved) -> Result<(), OpsError>,
) -> Result<Recorded<Resolved>, ResolveError> {
    let details = serde_json::json!({ "conflicts": 1, "choice": choice.code() });
    let recorded = ops::record(conn, RESOLVE_OPERATION, &details, |rec| {
        let Some(open) = read_open(rec, Some(id), &Filter::default())?.pop() else {
            return Ok(Err(ResolveError::NotOpen { conflict: id }));
        };
        let settled = match settle(&open, choice) {
            Ok(settled) => settled,
            Err(refused) => return Ok(Err(refused)),
        };
        let now = now(rec)?;
        let resolved = write(rec, &open, settled, &now)?;
        rec.detail("field", resolved.field.clone());
        adopt(rec, &resolved)?;
        Ok(Ok(resolved))
    })?;
    Ok(Recorded {
        operation_id: recorded.operation_id,
        value: recorded.value?,
    })
}

/// Settles every open conflict `filter` matches the same way, as one
/// operation: "take rekordbox for all ratings" is `field: Some("Rating")`
/// with [`Choice::KeepRekordbox`]. With nothing to settle, nothing is
/// logged. The store of the app's own values takes each value chosen
/// ([`app_values::adopt`]).
///
/// With [`Choice::Edit`] every matched conflict gets that value; if it
/// doesn't fit one of their fields, none is settled.
pub fn resolve_all(
    conn: &mut Connection,
    filter: &Filter,
    choice: &Choice,
) -> Result<Recorded<Vec<Resolved>>, ResolveError> {
    resolve_all_with(conn, filter, choice, app_values::adopt)
}

/// [`resolve_all`], with `adopt` run inside the same operation after each
/// conflict is settled, in place of [`app_values::adopt`] (see
/// [`resolve_with`]).
pub fn resolve_all_with(
    conn: &mut Connection,
    filter: &Filter,
    choice: &Choice,
    mut adopt: impl FnMut(&mut Recorder<'_>, &Resolved) -> Result<(), OpsError>,
) -> Result<Recorded<Vec<Resolved>>, ResolveError> {
    let mut details = serde_json::json!({ "choice": choice.code() });
    if let Some(field) = &filter.field {
        details["field"] = field.as_str().into();
    }
    let recorded = ops::record(conn, RESOLVE_ALL_OPERATION, &details, |rec| {
        let open = read_open(rec, None, filter)?;
        // Every conflict is checked before any is written.
        let mut settled = Vec::with_capacity(open.len());
        for conflict in &open {
            match settle(conflict, choice) {
                Ok(how) => settled.push(how),
                Err(refused) => return Ok(Err(refused)),
            }
        }
        let now = now(rec)?;
        let mut resolved = Vec::with_capacity(open.len());
        for (conflict, how) in open.iter().zip(settled) {
            let one = write(rec, conflict, how, &now)?;
            adopt(rec, &one)?;
            resolved.push(one);
        }
        rec.detail("conflicts", resolved.len());
        Ok(Ok(resolved))
    })?;
    Ok(Recorded {
        operation_id: recorded.operation_id,
        value: recorded.value?,
    })
}

/// The open conflicts `filter` matches (only `id`, when given), oldest
/// first, read inside the operation.
fn read_open(
    rec: &Recorder<'_>,
    id: Option<ConflictId>,
    filter: &Filter,
) -> Result<Vec<Conflict>, OpsError> {
    rec.read_rows(
        &format!("SELECT {CONFLICT_COLUMNS} FROM conflict WHERE {OPEN_WHERE} ORDER BY id"),
        params![id.map(|id| id.0), filter.field, filter.tracks_json()],
        conflict_from,
    )
}

/// The time now, as the database writes times.
fn now(rec: &Recorder<'_>) -> Result<String, OpsError> {
    let mut now = rec.read_rows("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", (), |r| {
        r.get::<_, String>(0)
    })?;
    Ok(now.pop().unwrap_or_default())
}

/// How one conflict ends: its status, the value given by hand (only for
/// [`Status::Edited`]) and the value chosen.
struct Settled {
    status: Status,
    resolved_value: Option<String>,
    value: String,
}

/// What `choice` makes of an open conflict, or why it can't be made.
fn settle(open: &Conflict, choice: &Choice) -> Result<Settled, ResolveError> {
    let app = open.app_value.as_deref().unwrap_or("");
    let rekordbox = open.rekordbox_value.as_deref().unwrap_or("");
    let kept = |status, value: &str| Settled {
        status,
        resolved_value: None,
        value: value.to_owned(),
    };
    Ok(match choice {
        Choice::KeepApp => kept(Status::KeptApp, app),
        Choice::KeepRekordbox => kept(Status::KeptRekordbox, rekordbox),
        Choice::Edit(value) => {
            if !fits(&open.field, value) {
                return Err(ResolveError::BadValue {
                    field: open.field.clone(),
                    value: value.clone(),
                });
            }
            if same(&open.field, Some(value), Some(app)) {
                kept(Status::KeptApp, app)
            } else if same(&open.field, Some(value), Some(rekordbox)) {
                kept(Status::KeptRekordbox, rekordbox)
            } else {
                Settled {
                    status: Status::Edited,
                    resolved_value: Some(value.clone()),
                    value: value.clone(),
                }
            }
        }
    })
}

/// Replaces the open row by its settled one, through the log.
fn write(
    rec: &mut Recorder<'_>,
    open: &Conflict,
    settled: Settled,
    now: &str,
) -> Result<Resolved, OpsError> {
    rec.delete("conflict", open.id.0)?;
    let id = rec.insert(
        "conflict",
        &[
            ("library_track_id", Value::Integer(open.library_track.0)),
            ("field", Value::Text(open.field.clone())),
            ("app_value", open.app_value.clone().into()),
            ("rekordbox_value", open.rekordbox_value.clone().into()),
            ("base_value", open.base_value.clone().into()),
            ("status", Value::Text(settled.status.stored().to_owned())),
            ("resolved_value", settled.resolved_value.into()),
            ("detected_at", Value::Text(open.detected_at.clone())),
            ("resolved_at", Value::Text(now.to_owned())),
        ],
    )?;
    Ok(Resolved {
        conflict: ConflictId(id),
        was: open.id,
        library_track: open.library_track,
        field: open.field.clone(),
        status: settled.status,
        value: settled.value,
    })
}
