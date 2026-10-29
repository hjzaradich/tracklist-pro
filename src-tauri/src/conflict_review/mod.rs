//! Conflicts in Review (1cB-1, 1cB-2; ROADMAP 1.10 "Conflicts queue", 1.12).
//!
//! The command layer over the conflict engine ([`crate::conflicts`]): what
//! Review lists, and the three ways it settles a conflict. The engine decides
//! what a conflict is and how it is settled; nothing here compares, detects
//! or writes a conflict itself.
//!
//! - [`review_conflict_fields`]: the fields that have open conflicts, with how
//!   many and the kind of value each holds. A field with none isn't listed.
//! - [`review_conflicts`]: a page of one field's open conflicts, the oldest
//!   first, each with the track's name and the three values (the app's,
//!   rekordbox's and the base: `base` is `None` when the field was never
//!   sent). Values are rekordbox's own text, as stored; the screen shows them
//!   by [`ValueKind`].
//! - [`review_resolve_conflict`]: keep the app's value, keep rekordbox's, or
//!   a value given by hand (which must be a value of the field's kind).
//! - [`review_resolve_conflicts`]: keep one side for every open conflict of
//!   a field. One operation, so one Undo reopens them all.
//!
//! Each settling is one operation in the log (`conflicts::resolve`,
//! `conflicts::resolve_all`), so the top bar's Undo takes it back.
//!
//! The settling goes through [`settle_one`] and [`settle_all`], the two places
//! that call the engine. The engine's `resolve` and `resolve_all` also adopt
//! the value chosen into the app's own values (and a crate membership into the
//! crate) inside the same operation, so one Undo takes it all back.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::conflicts::{
    self, kind_of, Choice, ConflictId, Filter, Kind, ResolveError, Resolved, Status,
};
use crate::db::{ReadPool, Writer};
use crate::ipc::{ErrorKind, IpcError};
use crate::library::LibraryTrackId;
use crate::scan::system_volumes;
use crate::send_values::shown;

#[cfg(test)]
mod tests;

/// The most conflicts one page holds, whatever the caller asks for.
pub const MAX_PAGE: u32 = 500;

/// How a field's values are read and shown (and what a value given by hand
/// looks like). The engine's own `Kind`, without its two never-listed
/// members (an analysis field and `TrackID` never conflict).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ValueKind {
    Location,
    /// rekordbox's six values, shown as stars.
    Rating,
    /// `0xRRGGBB`.
    Colour,
    /// `2026-09-28`.
    Date,
    /// Whole numbers.
    Number,
    Text,
    /// A crate membership (`playlist:...`): `in` or `out`, shown in words.
    Membership,
}

impl ValueKind {
    pub(crate) fn of(field: &str) -> ValueKind {
        if conflicts::membership::is_membership(field) {
            return ValueKind::Membership;
        }
        match kind_of(field) {
            Kind::Location => ValueKind::Location,
            Kind::Rating => ValueKind::Rating,
            Kind::Colour => ValueKind::Colour,
            Kind::Date => ValueKind::Date,
            Kind::Number => ValueKind::Number,
            Kind::Text | Kind::Analysis | Kind::TrackId => ValueKind::Text,
        }
    }
}

/// A field with open conflicts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConflictField {
    /// The rekordbox XML attribute name, e.g. `Rating`.
    pub field: String,
    pub kind: ValueKind,
    /// How many open conflicts the field has.
    pub count: u32,
    /// For a crate membership: the crate's name inside its folders' names,
    /// joined with " / " (`conflicts::membership::crate_labels`); `None`
    /// for any other field.
    pub crate_name: Option<String>,
}

/// A track named as Review names tracks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConflictTrack {
    pub library_track: LibraryTrackId,
    pub title: Option<String>,
    pub artist: Option<String>,
    /// The name of the track's linked file, if it has one.
    pub file_name: Option<String>,
}

/// One open conflict, with its three values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConflictRow {
    pub id: ConflictId,
    pub track: ConflictTrack,
    pub field: String,
    pub kind: ValueKind,
    /// The app's value; `None` when the app holds no value.
    pub app: Option<String>,
    /// rekordbox's value; `None` or empty when its entry has no such
    /// attribute.
    pub rekordbox: Option<String>,
    /// What was last sent; `None` when the field was never sent.
    pub base: Option<String>,
}

/// A page of one field's open conflicts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConflictPage {
    pub field: String,
    /// How many open conflicts the field has in all.
    pub total: u32,
    pub items: Vec<ConflictRow>,
}

/// What a bulk settling keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    App,
    Rekordbox,
}

impl Side {
    fn choice(self) -> Choice {
        match self {
            Side::App => Choice::KeepApp,
            Side::Rekordbox => Choice::KeepRekordbox,
        }
    }
}

/// How many open conflicts a field has.
pub fn open_in(conn: &Connection, field: &str) -> rusqlite::Result<u32> {
    conn.query_row(
        "SELECT COUNT(*) FROM conflict WHERE status = 'open' AND field = ?1",
        [field],
        |r| r.get(0),
    )
}

/// How many open conflicts wait, in all.
pub fn open_count(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row(
        "SELECT COUNT(*) FROM conflict WHERE status = 'open'",
        [],
        |r| r.get(0),
    )
}

/// The fields with open conflicts, by field name.
pub fn fields(conn: &Connection) -> rusqlite::Result<Vec<ConflictField>> {
    let mut found: Vec<ConflictField> = conn
        .prepare(
            "SELECT field, COUNT(*) FROM conflict WHERE status = 'open'
             GROUP BY field ORDER BY field",
        )?
        .query_map([], |r| {
            let field: String = r.get(0)?;
            Ok(ConflictField {
                kind: ValueKind::of(&field),
                field,
                count: r.get(1)?,
                crate_name: None,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let memberships: Vec<String> = found
        .iter()
        .filter(|f| f.kind == ValueKind::Membership)
        .map(|f| f.field.clone())
        .collect();
    if !memberships.is_empty() {
        let mut names = conflicts::membership::crate_labels(conn, &memberships)?;
        for field in &mut found {
            field.crate_name = names.remove(&field.field);
        }
    }
    Ok(found)
}

/// A page of the open conflicts of `field`, the oldest first, in one read
/// transaction so the total and the page agree.
pub fn page(
    conn: &Connection,
    volumes: &impl crate::paths::Volumes,
    field: &str,
    offset: u32,
    limit: u32,
) -> rusqlite::Result<ConflictPage> {
    let conn = conn.unchecked_transaction()?;
    let total = open_in(&conn, field)?;
    let limit = limit.min(MAX_PAGE);
    let kind = ValueKind::of(field);
    let mut items: Vec<ConflictRow> = conn
        .prepare(
            "SELECT id, library_track_id, app_value, rekordbox_value, base_value
             FROM conflict WHERE status = 'open' AND field = ?1
             ORDER BY id LIMIT ?2 OFFSET ?3",
        )?
        .query_map(params![field, limit, offset], |r| {
            Ok(ConflictRow {
                id: ConflictId(r.get(0)?),
                // Named below, for all the page's tracks at once.
                track: ConflictTrack {
                    library_track: LibraryTrackId(r.get(1)?),
                    title: None,
                    artist: None,
                    file_name: None,
                },
                field: field.to_owned(),
                kind,
                app: r.get(2)?,
                rekordbox: r.get(3)?,
                base: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let ids: Vec<LibraryTrackId> = items.iter().map(|row| row.track.library_track).collect();
    let mut names = shown(&conn, volumes, &ids)?;
    for row in &mut items {
        if let Some(shown) = names.remove(&row.track.library_track) {
            (row.track.title, row.track.artist) = shown.into_options();
        }
    }
    // The linked file's name: for a title or artist conflict the name Review
    // picks for the track is one of the values in dispute, so the screen
    // shows the file beside it.
    let ids_json = serde_json::to_string(&ids.iter().map(|id| id.0).collect::<Vec<_>>())
        .expect("numbers always serialize");
    let files: HashMap<i64, String> = conn
        .prepare(
            "SELECT lt.id, f.rel_path FROM library_track lt
             JOIN file f ON f.id = lt.linked_file_id
             WHERE lt.id IN (SELECT value FROM json_each(?1))",
        )?
        .query_map([ids_json], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for row in &mut items {
        row.track.file_name = files
            .get(&row.track.library_track.0)
            .and_then(|path| path.rsplit('/').next())
            .map(str::to_owned);
    }
    Ok(ConflictPage {
        field: field.to_owned(),
        total,
        items,
    })
}

/// Settles one conflict, as one operation. The one place Review settles a
/// single conflict.
pub fn settle_one(
    conn: &mut Connection,
    id: ConflictId,
    choice: &Choice,
) -> Result<Resolved, ResolveError> {
    conflicts::resolve(conn, id, choice).map(|recorded| recorded.value)
}

/// Keeps one side for every open conflict of `field`, as one operation.
/// Answers how many were settled. The one place Review settles in bulk.
pub fn settle_all(conn: &mut Connection, field: &str, side: Side) -> Result<u32, ResolveError> {
    let filter = Filter {
        field: Some(field.to_owned()),
        tracks: None,
    };
    conflicts::resolve_all(conn, &filter, &side.choice())
        .map(|recorded| u32::try_from(recorded.value.len()).unwrap_or(u32::MAX))
}

impl From<ResolveError> for IpcError {
    fn from(e: ResolveError) -> Self {
        match e {
            ResolveError::Ops(e) => IpcError::from(e),
            ResolveError::NotOpen { .. } => IpcError::new(ErrorKind::ConflictGone),
            ResolveError::BadValue { .. } => IpcError::new(ErrorKind::ConflictBadValue),
        }
    }
}

/// The settled conflict, as the screen needs to know it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Settled {
    pub library_track: LibraryTrackId,
    pub field: String,
    pub status: Status,
    /// The value that is now the field's.
    pub value: String,
}

impl From<Resolved> for Settled {
    fn from(r: Resolved) -> Self {
        Settled {
            library_track: r.library_track,
            field: r.field,
            status: r.status,
            value: r.value,
        }
    }
}

/// The fields that have open conflicts, with how many each has.
#[tauri::command]
#[specta::specta]
pub async fn review_conflict_fields(
    reads: State<'_, ReadPool>,
) -> Result<Vec<ConflictField>, IpcError> {
    Ok(reads.read(fields)?)
}

/// A page of one field's open conflicts, the oldest first.
#[tauri::command]
#[specta::specta]
pub async fn review_conflicts(
    reads: State<'_, ReadPool>,
    field: String,
    offset: u32,
    limit: u32,
) -> Result<ConflictPage, IpcError> {
    let volumes = system_volumes();
    Ok(reads.read(|conn| page(conn, &volumes, &field, offset, limit))?)
}

/// Settles one open conflict: keep the app's value, keep rekordbox's, or a
/// value given by hand. Refused, with nothing changed, if the conflict isn't
/// open any more or the value doesn't fit the field. A file location is
/// never given by hand: the app holds no value of its own for it. Nor is a
/// crate membership: the crate holds the track or it doesn't, so the choices
/// are the app's and rekordbox's.
#[tauri::command]
#[specta::specta]
pub async fn review_resolve_conflict(
    writer: State<'_, Writer>,
    id: ConflictId,
    choice: Choice,
) -> Result<Settled, IpcError> {
    let settled = writer.call(move |conn| {
        if matches!(choice, Choice::Edit(_)) {
            let field: Option<String> = conn
                .query_row("SELECT field FROM conflict WHERE id = ?1", [id.0], |r| {
                    r.get(0)
                })
                .optional()?;
            if field.is_some_and(|field| {
                kind_of(&field) == Kind::Location || conflicts::membership::is_membership(&field)
            }) {
                return Ok(Err(IpcError::new(ErrorKind::ConflictBadValue)));
            }
        }
        Ok(settle_one(conn, id, &choice).map_err(IpcError::from))
    })??;
    Ok(settled.into())
}

/// Keeps one side for every open conflict of a field ("keep rekordbox's for
/// all ratings"). One operation, so one Undo reopens them all. `expected` is
/// how many the screen showed: if there are more or fewer now, nothing is
/// settled and the screen asks again. Answers how many were settled.
#[tauri::command]
#[specta::specta]
pub async fn review_resolve_conflicts(
    writer: State<'_, Writer>,
    field: String,
    side: Side,
    expected: u32,
) -> Result<u32, IpcError> {
    writer.call(move |conn| {
        // The writer runs one job at a time, so no other write comes between
        // the count and the settling.
        if open_in(conn, &field)? != expected {
            return Ok(Err(IpcError::new(ErrorKind::ConflictsChanged)));
        }
        Ok(settle_all(conn, &field, side).map_err(IpcError::from))
    })?
}
