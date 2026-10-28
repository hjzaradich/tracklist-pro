//! Where the app keeps its own value per field (ROADMAP 1.10, §2
//! `app_value`; 1cB-18): the user's edits, the app's side of the three-way
//! merge.
//!
//! A row is a field of a Library track the app means to change (1.9 rule
//! 1). It holds the value as the user gave it, and the value rekordbox
//! showed for the field when the edit was made (`replaced`, what the edit
//! replaced: from the newest trusted read; `None` when rekordbox had no
//! trusted entry for the track, empty when its entry had no such
//! attribute). A field without a row has none of the app's own and follows
//! rekordbox (decision table row 4). [`read`] hands the rows to the engine
//! as [`AppValues`].
//!
//! # Writing
//!
//! [`set`] and [`clear`] are the only writers, each one operation of the
//! log, so Undo takes either back exactly and is refused, changing nothing,
//! when the row changed since.
//!
//! - **A first edit** records rekordbox's value at that moment as the
//!   replaced value.
//! - **Editing again keeps the replaced value** of the first edit: it's
//!   what rekordbox had before the app held a value of its own, whatever
//!   rekordbox shows now.
//! - **A value equal to rekordbox's current one** (by the field's kind, as
//!   the engine compares) clears the app's own: there's nothing to change,
//!   so the field follows rekordbox again. Only a trusted entry of the
//!   newest read counts; with none, the value is kept.
//! - The value is stored as given, never normalized: `007` stays `007`.
//!   It must be a value of the field's kind ([`super::fits`]).
//! - Analysis fields, `TrackID` and `Location` are refused ([`editable`]),
//!   and the database refuses them too (migration 0035).
//!
//! # After a send
//!
//! A send doesn't touch this table: the app's value stays its own, sent or
//! not, so a send that is never imported can't make it look like rekordbox
//! changed the field back. `replaced` here counts only while the field was
//! never sent. Once it was, the engine ([`super::load_sides`]) takes what
//! the send replaced from the send's own record (`sync_base.replaced`,
//! 1cB-19), which a read forgets once it shows the send taken up, so a
//! later change back in rekordbox reads as rekordbox's change, not as a
//! send not taken up.
//!
//! # Outside the log: taking rekordbox's value in
//!
//! The merge before a send ([`super::merge`], 1cB-3) deletes the app's own
//! value of a field whose rekordbox value it takes in (only rekordbox
//! changed, [`take_in`]). Otherwise, once the base is rekordbox's value,
//! the old app value would read as "only the app changed" and go out over
//! rekordbox's edit. That delete is derived from the three sides, like the
//! rest of detection, so it's outside the log: `app_value` ids are never
//! reused (migration 0035), and Undo of the edit is refused afterwards as
//! "changed since".
//!
//! # Settling a conflict
//!
//! A resolve updates the store in its own operation ([`adopt`], through
//! [`super::resolve`]): keeping rekordbox's value clears the app's own,
//! keeping the app's or giving a value by hand sets it. So the next send
//! carries the value chosen, and the one turned down never goes out again,
//! once the decision no longer answers the field (a side moved).

use rusqlite::types::Value;
use rusqlite::Connection;
use serde_json::json;

use super::resolve::Resolved;
use super::store::{json_ids, load_sides, AppValue, AppValues, RekordboxEntry, Status};
use super::{fits, kind_of, same, Kind};
use crate::library::LibraryTrackId;
use crate::ops::{self, OpsError, Recorded, Recorder};
use crate::paths::Volumes;
use crate::send_values::send_values;
use std::fmt;

/// The operation kind of [`set`].
pub const SET_OPERATION: &str = "set_app_value";
/// The operation kind of [`clear`].
pub const CLEAR_OPERATION: &str = "clear_app_value";

/// Why nothing was stored. Nothing was changed.
#[derive(Debug)]
pub enum AppValueError {
    /// The database or the operation log refused.
    Ops(OpsError),
    /// There's no such Library track.
    NoTrack(LibraryTrackId),
    /// The app never holds a value of its own for this field: an analysis
    /// field, `TrackID` or `Location`.
    NotEditable { field: String },
    /// The value isn't one of the field's kind.
    BadValue { field: String, value: String },
}

impl fmt::Display for AppValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppValueError::Ops(e) => write!(f, "{e}"),
            AppValueError::NoTrack(id) => write!(f, "no Library track {}", id.0),
            AppValueError::NotEditable { field } => {
                write!(f, "{field:?} is never the app's to change")
            }
            AppValueError::BadValue { field, value } => {
                write!(f, "{value:?} is not a value of {field:?}")
            }
        }
    }
}

impl std::error::Error for AppValueError {}

impl From<OpsError> for AppValueError {
    fn from(e: OpsError) -> Self {
        AppValueError::Ops(e)
    }
}

impl From<rusqlite::Error> for AppValueError {
    fn from(e: rusqlite::Error) -> Self {
        AppValueError::Ops(e.into())
    }
}

/// Whether the app may hold a value of its own for `field`: not an
/// analysis field (rekordbox's, 1.9 rule 3), not `TrackID` (not a field)
/// and not `Location` (where the linked file is).
pub fn editable(field: &str) -> bool {
    !field.is_empty()
        && !matches!(
            kind_of(field),
            Kind::Analysis | Kind::TrackId | Kind::Location
        )
}

/// What rekordbox shows for one field of a track: `None` when that isn't
/// known (no trusted entry in the newest read), `Some("")` when its entry
/// has no such attribute.
fn showing(
    conn: &Connection,
    volumes: &impl Volumes,
    track: LibraryTrackId,
    field: &str,
) -> rusqlite::Result<Option<String>> {
    let sent = send_values(conn, volumes, &[track])?;
    let sides = load_sides(conn, &sent, &AppValues::default())?;
    Ok(sides
        .into_iter()
        .find(|t| t.library_track == track && t.entry == RekordboxEntry::Current)
        .map(|t| {
            t.fields
                .into_iter()
                .find(|f| f.field == field)
                .and_then(|f| f.rekordbox)
                .unwrap_or_default()
        }))
}

/// The app's own row for a field, if it has one.
fn own_row(
    rec: &ops::Recorder<'_>,
    track: LibraryTrackId,
    field: &str,
) -> Result<Option<(i64, Option<String>)>, OpsError> {
    Ok(rec
        .read_rows(
            "SELECT id, replaced FROM app_value WHERE library_track_id = ?1 AND field = ?2",
            (track.0, field),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .pop())
}

fn check(conn: &Connection, track: LibraryTrackId, field: &str) -> Result<(), AppValueError> {
    if !editable(field) {
        return Err(AppValueError::NotEditable {
            field: field.to_owned(),
        });
    }
    let found: i64 = conn.query_row(
        "SELECT count(*) FROM library_track WHERE id = ?1",
        [track.0],
        |r| r.get(0),
    )?;
    if found == 0 {
        return Err(AppValueError::NoTrack(track));
    }
    Ok(())
}

/// Sets the app's own value of `field` for a Library track, as one
/// operation, by the rules in the module docs. Returns the app's own value
/// afterwards: `None` when `value` is rekordbox's current one and the field
/// follows rekordbox. Nothing is logged when nothing changed.
pub fn set(
    conn: &mut Connection,
    volumes: &impl Volumes,
    track: LibraryTrackId,
    field: &str,
    value: &str,
) -> Result<Recorded<Option<AppValue>>, AppValueError> {
    check(conn, track, field)?;
    if !fits(field, value) {
        return Err(AppValueError::BadValue {
            field: field.to_owned(),
            value: value.to_owned(),
        });
    }
    let details = json!({ "libraryTrackId": track.0, "field": field });
    let recorded = ops::record_with_derived_first(
        conn,
        SET_OPERATION,
        &details,
        // rekordbox's side, read in the operation's own transaction. Reads
        // only.
        |c| showing(c, volumes, track, field),
        |rec, rekordbox| {
            let own = own_row(rec, track, field)?;
            if let Some(rekordbox) = &rekordbox {
                if same(field, Some(value), Some(rekordbox)) {
                    if let Some((id, _)) = own {
                        rec.delete("app_value", id)?;
                    }
                    return Ok(None);
                }
            }
            let replaced = match own {
                Some((id, replaced)) => {
                    rec.set("app_value", id, "value", value)?;
                    replaced
                }
                None => {
                    rec.insert(
                        "app_value",
                        &[
                            ("library_track_id", Value::Integer(track.0)),
                            ("field", Value::Text(field.to_owned())),
                            ("value", Value::Text(value.to_owned())),
                            ("replaced", rekordbox.clone().into()),
                        ],
                    )?;
                    rekordbox
                }
            };
            Ok(Some(AppValue {
                field: field.to_owned(),
                value: value.to_owned(),
                replaced,
            }))
        },
        |_| Ok(()),
    )?;
    Ok(recorded)
}

/// Clears the app's own value of `field` for a Library track, as one
/// operation: the field follows rekordbox again. Returns whether there was
/// one; nothing is logged when there wasn't.
pub fn clear(
    conn: &mut Connection,
    track: LibraryTrackId,
    field: &str,
) -> Result<Recorded<bool>, AppValueError> {
    check(conn, track, field)?;
    let details = json!({ "libraryTrackId": track.0, "field": field });
    let recorded = ops::record(conn, CLEAR_OPERATION, &details, |rec| {
        let Some((id, _)) = own_row(rec, track, field)? else {
            return Ok(false);
        };
        rec.delete("app_value", id)?;
        Ok(true)
    })?;
    Ok(recorded)
}

/// Takes rekordbox's value in for one field: deletes the app's own value,
/// **outside the operation log** (module docs). Only the merge before a
/// send calls it, inside its own transaction.
pub(super) fn take_in(
    conn: &Connection,
    track: LibraryTrackId,
    field: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM app_value WHERE library_track_id = ?1 AND field = ?2",
        (track.0, field),
    )?;
    Ok(())
}

/// What a settled conflict makes of the store, inside the resolve's own
/// operation (so one Undo takes both back): keeping rekordbox's value
/// clears the app's own; keeping the app's, or a value given by hand, makes
/// it the app's own. A field the app never holds a value of its own for
/// ([`editable`]) is left alone, as is a value already stored.
///
/// A value newly stored records as what it replaced the rekordbox value
/// the conflict showed; an edit already stored keeps its own.
pub fn adopt(rec: &mut Recorder<'_>, resolved: &Resolved) -> Result<(), OpsError> {
    if !editable(&resolved.field) {
        return Ok(());
    }
    let own = own_row(rec, resolved.library_track, &resolved.field)?;
    match (resolved.status, own) {
        (Status::KeptRekordbox, Some((id, _))) => rec.delete("app_value", id),
        (Status::KeptRekordbox | Status::Open, _) => Ok(()),
        // Nothing is logged when it's the value stored.
        (Status::KeptApp | Status::Edited, Some((id, _))) => rec
            .set("app_value", id, "value", resolved.value.as_str())
            .map(|_| ()),
        (Status::KeptApp | Status::Edited, None) => {
            let replaced: Option<String> = rec
                .read_rows(
                    "SELECT rekordbox_value FROM conflict WHERE id = ?1",
                    [resolved.conflict.0],
                    |r| r.get(0),
                )?
                .pop()
                .flatten();
            rec.insert(
                "app_value",
                &[
                    ("library_track_id", Value::Integer(resolved.library_track.0)),
                    ("field", Value::Text(resolved.field.clone())),
                    ("value", Value::Text(resolved.value.clone())),
                    ("replaced", replaced.into()),
                ],
            )?;
            Ok(())
        }
    }
}

/// The app's own values of these Library tracks, as the engine takes them,
/// in the order they were first set.
pub fn read(conn: &Connection, ids: &[LibraryTrackId]) -> rusqlite::Result<AppValues> {
    let mut stmt = conn.prepare(
        "SELECT library_track_id, field, value, replaced FROM app_value
         WHERE library_track_id IN (SELECT value FROM json_each(?1))
         ORDER BY library_track_id, id",
    )?;
    let mut values = AppValues::default();
    for row in stmt.query_map([json_ids(ids)], |r| {
        Ok((
            LibraryTrackId(r.get(0)?),
            AppValue {
                field: r.get(1)?,
                value: r.get(2)?,
                replaced: r.get(3)?,
            },
        ))
    })? {
        let (track, value) = row?;
        values.set(track, value);
    }
    Ok(values)
}
