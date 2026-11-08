//! "Use": the chosen values become the app's own, as one operation.

use rusqlite::types::Value;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;

use super::{rank, IdentifyField};
use crate::conflicts::{load_sides, AppValues, RekordboxEntry};
use crate::ipc::{ErrorKind, IpcError};
use crate::library::LibraryTrackId;
use crate::ops::{self, OpsError, Recorder};
use crate::paths::Volumes;
use crate::send_values::send_values;

/// The operation "Use" records; the top bar's Undo names it.
pub const USE_OPERATION: &str = "identify_use";

/// One value the user chose for one field. `field` is an
/// [`IdentifyField`]'s name ("Name", …, "CatalogueNumber").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UseChoice {
    pub field: String,
    pub value: String,
}

/// Why nothing was used. Nothing was changed.
#[derive(Debug)]
pub enum UseError {
    /// Not one of the fields identification sets.
    UnknownField(String),
    /// A blank value: clearing a field isn't Use.
    ValueEmpty(IdentifyField),
    /// A value its field can't take (a year that isn't a number) or holding
    /// a character rekordbox's XML can't carry.
    NotSendable(IdentifyField),
    /// There's no such Library track.
    NoTrack(LibraryTrackId),
    Ops(OpsError),
}

impl From<OpsError> for UseError {
    fn from(e: OpsError) -> Self {
        UseError::Ops(e)
    }
}

impl From<rusqlite::Error> for UseError {
    fn from(e: rusqlite::Error) -> Self {
        UseError::Ops(e.into())
    }
}

impl From<UseError> for IpcError {
    fn from(e: UseError) -> Self {
        match e {
            UseError::UnknownField(_) => IpcError::new(ErrorKind::IdentifyFieldUnknown),
            UseError::ValueEmpty(_) => IpcError::new(ErrorKind::IdentifyValueEmpty),
            UseError::NotSendable(_) => IpcError::new(ErrorKind::IdentifyValueNotSendable),
            UseError::NoTrack(_) => IpcError::new(ErrorKind::LibraryTrackNotFound),
            UseError::Ops(e) => e.into(),
        }
    }
}

/// The choices checked: each field once (a field chosen twice takes the
/// last value), values tidied as suggestions are.
fn checked(choices: &[UseChoice]) -> Result<Vec<(IdentifyField, String)>, UseError> {
    let mut out: Vec<(IdentifyField, String)> = Vec::new();
    for choice in choices {
        let field = IdentifyField::of_name(&choice.field)
            .ok_or_else(|| UseError::UnknownField(choice.field.clone()))?;
        let value = rank::tidy(&choice.value);
        if value.is_empty() {
            return Err(UseError::ValueEmpty(field));
        }
        if !rank::takes(field, &value) {
            return Err(UseError::NotSendable(field));
        }
        out.retain(|(f, _)| *f != field);
        out.push((field, value));
    }
    Ok(out)
}

/// What rekordbox shows for each of `attributes` of `track` now (`None` per
/// field when it has no trusted entry in the newest read, `Some("")` when
/// its entry lacks the attribute): the edit store's rules need it, as
/// `app_values::set_as` reads it.
fn showing(
    conn: &Connection,
    volumes: &impl Volumes,
    track: LibraryTrackId,
    attributes: &[&'static str],
) -> rusqlite::Result<Vec<Option<String>>> {
    let sent = send_values(conn, volumes, &[track])?;
    let entry = load_sides(conn, &sent, &AppValues::default())?
        .into_iter()
        .find(|t| t.library_track == track && t.entry == RekordboxEntry::Current);
    Ok(attributes
        .iter()
        .map(|a| {
            entry.as_ref().map(|t| {
                t.fields
                    .iter()
                    .find(|f| f.field == *a)
                    .and_then(|f| f.rekordbox.clone())
                    .unwrap_or_default()
            })
        })
        .collect())
}

/// Sets the catalogue number of `track` to `value`, in `rec`'s operation.
fn set_catalogue(rec: &mut Recorder<'_>, track: LibraryTrackId, value: &str) -> Result<(), OpsError> {
    let row: Option<i64> = rec
        .read_rows(
            "SELECT id FROM catalogue_number WHERE library_track_id = ?1",
            [track.0],
            |r| r.get(0),
        )?
        .pop();
    match row {
        Some(id) => {
            rec.set("catalogue_number", id, "value", value)?;
        }
        None => {
            rec.insert(
                "catalogue_number",
                &[
                    ("library_track_id", Value::Integer(track.0)),
                    ("value", Value::Text(value.to_owned())),
                ],
            )?;
        }
    }
    Ok(())
}

/// "Use" on the Library track `track`: each choice set as the app's own
/// value, as one operation ([`USE_OPERATION`]). Answers the operation
/// recorded, none when nothing changed (or nothing was chosen).
pub fn use_on(
    conn: &mut Connection,
    volumes: &impl Volumes,
    track: LibraryTrackId,
    choices: &[UseChoice],
) -> Result<Option<i64>, UseError> {
    let chosen = checked(choices)?;
    let found: i64 = conn.query_row(
        "SELECT count(*) FROM library_track WHERE id = ?1",
        [track.0],
        |r| r.get(0),
    )?;
    if found == 0 {
        return Err(UseError::NoTrack(track));
    }
    if chosen.is_empty() {
        return Ok(None);
    }
    let sent: Vec<(&'static str, String)> = chosen
        .iter()
        .filter_map(|(f, v)| f.attribute().map(|a| (a, v.clone())))
        .collect();
    let catalogue = chosen
        .iter()
        .find(|(f, _)| *f == IdentifyField::CatalogueNumber)
        .map(|(_, v)| v.clone());
    let details = json!({
        "libraryTrackId": track.0,
        "fields": chosen.iter().map(|(f, _)| f).collect::<Vec<_>>(),
    });
    let attributes: Vec<&'static str> = sent.iter().map(|(a, _)| *a).collect();
    let recorded = ops::record_with_derived_first(
        conn,
        USE_OPERATION,
        &details,
        // rekordbox's side, read in the operation's own transaction. Reads only.
        |c| showing(c, volumes, track, &attributes),
        |rec, rekordbox| {
            for ((attribute, value), rekordbox) in sent.iter().zip(rekordbox) {
                crate::conflicts::app_values::set_in(rec, track, attribute, value, rekordbox)?;
            }
            if let Some(value) = &catalogue {
                set_catalogue(rec, track, value)?;
            }
            Ok(())
        },
        // The track's row is worked out again (as `genre::own` marks it).
        |c| crate::genre::own::mark_track_due(c, track),
    )?;
    Ok(recorded.operation_id)
}
