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
//!   and the database refuses them too (migration 0035). So is any name
//!   that isn't one of rekordbox's `TRACK` attributes ([`TRACK_ATTRIBUTES`]),
//!   and a value holding a character XML 1.0 can't carry (the send would
//!   leave the track out).
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
use super::{fits, kind_of, membership, same, Kind};
use crate::energy_stars;
use crate::library::LibraryTrackId;
use crate::ops::{self, OpsError, Recorded, Recorder};
use crate::paths::Volumes;
use crate::send_values::{send_values, SendValues};
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
    /// field, `TrackID`, `Location`, or a name that isn't one of rekordbox's
    /// `TRACK` attributes.
    NotEditable { field: String },
    /// The value isn't one of the field's kind, or holds a character XML
    /// can't carry.
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

/// The attributes rekordbox writes on a `TRACK` (§5.3): the only names a
/// field edit may give.
pub const TRACK_ATTRIBUTES: [&str; 28] = [
    "TrackID",
    "Name",
    "Artist",
    "Composer",
    "Album",
    "Grouping",
    "Genre",
    "Kind",
    "Size",
    "TotalTime",
    "DiscNumber",
    "TrackNumber",
    "Year",
    "AverageBpm",
    "DateModified",
    "DateAdded",
    "BitRate",
    "SampleRate",
    "Comments",
    "PlayCount",
    "LastPlayed",
    "Rating",
    "Location",
    "Remixer",
    "Tonality",
    "Label",
    "Mix",
    "Colour",
];

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
    if !TRACK_ATTRIBUTES.contains(&field) {
        return Err(AppValueError::NotEditable {
            field: field.to_owned(),
        });
    }
    if !fits(field, value) || crate::rekordbox_write::uncarriable(value).is_some() {
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
        |rec, rekordbox| set_in(rec, track, field, value, rekordbox),
        |_| Ok(()),
    )?;
    Ok(recorded)
}

/// [`set`]'s writes, as changes of the operation `rec` records, for a
/// caller that is part of a bigger operation (the merge of two Library
/// tracks, which takes the value the user chose from the other track). The
/// caller checked the field and the value ([`editable`], [`fits`]).
/// `rekordbox` is what rekordbox shows for the field now (`None`: no trusted
/// entry). Returns the app's own value afterwards, `None` when `value` is
/// rekordbox's and the field follows it.
pub(crate) fn set_in(
    rec: &mut Recorder<'_>,
    track: LibraryTrackId,
    field: &str,
    value: &str,
    rekordbox: Option<String>,
) -> Result<Option<AppValue>, OpsError> {
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
}

/// What the merge of two Library tracks does for a value the user chose
/// (1cB-20; ROADMAP 1.12): [`set_in`], and the choice becomes the user's
/// decision against what rekordbox shows now, so the next send carries it as
/// an app change (the rules of 1.10) instead of reading rekordbox's change
/// since the last send as a change to take in or to ask about again. In the
/// merge's own operation:
///
/// - the value records rekordbox's present value as what it replaced (also
///   when an edit was stored already: this is a new decision);
/// - a base (`sync_base`) that rekordbox's value has moved off since the
///   send is dropped, as if the field were never sent: the base is the
///   value last sent, and the user has now chosen against rekordbox's;
/// - an open conflict on the field is withdrawn: the user answered it, and
///   what is left to disagree is detected again from the three sides.
pub(crate) fn set_chosen(
    rec: &mut Recorder<'_>,
    track: LibraryTrackId,
    field: &str,
    value: &str,
    rekordbox: Option<String>,
) -> Result<(), OpsError> {
    let own = set_in(rec, track, field, value, rekordbox.clone())?;
    if let (Some(rekordbox), Some(own)) = (&rekordbox, &own) {
        if own.replaced.as_ref() != Some(rekordbox) {
            if let Some((id, _)) = own_row(rec, track, field)? {
                rec.set("app_value", id, "replaced", rekordbox.as_str())?;
            }
        }
    }
    if let Some(rekordbox) = &rekordbox {
        let base: Vec<(i64, String)> = rec.read_rows(
            "SELECT id, value FROM sync_base WHERE library_track_id = ?1 AND field = ?2",
            (track.0, field),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        for (id, sent) in base {
            if !same(field, Some(&sent), Some(rekordbox)) {
                rec.delete("sync_base", id)?;
            }
        }
    }
    let open: Vec<i64> = rec.read_rows(
        "SELECT id FROM conflict
         WHERE library_track_id = ?1 AND field = ?2 AND status = 'open'",
        (track.0, field),
        |r| r.get(0),
    )?;
    for id in open {
        rec.delete("conflict", id)?;
    }
    Ok(())
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
/// the conflict showed; an edit already stored keeps its own. A crate
/// membership changes the crate instead ([`membership::adopt`]).
pub fn adopt(rec: &mut Recorder<'_>, resolved: &Resolved) -> Result<(), OpsError> {
    if membership::is_membership(&resolved.field) {
        return membership::adopt(rec, resolved);
    }
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

/// [`read`] for the tracks `sent` and the merge before a send: the app's
/// own values, and while the user has energy sent as the star rating, each
/// track's `Rating` derived from its energy ([`energy_stars::derive`]),
/// which stands in place of an own `Rating` of the track's. Only the
/// merge and detection read it so; the store itself, and the merge of two
/// Library tracks (1.12), see the rows as they are.
pub fn read_for_send(conn: &Connection, sent: &[SendValues]) -> rusqlite::Result<AppValues> {
    let ids: Vec<LibraryTrackId> = sent.iter().map(|s| s.library_track).collect();
    let mut values = read(conn, &ids)?;
    energy_stars::derive(conn, sent, &mut values)?;
    Ok(values)
}

/// Drops the app's own row for a field, inside an operation the caller
/// records, if it has one. `adopt` for a field that the app sends from
/// something else (`Rating`, from the track's energy; [`energy_stars`]) uses
/// it, so no stale value of the app's own comes back when that stops.
pub(crate) fn forget(
    rec: &mut Recorder<'_>,
    track: LibraryTrackId,
    field: &str,
) -> Result<(), OpsError> {
    match own_row(rec, track, field)? {
        Some((id, _)) => rec.delete("app_value", id),
        None => Ok(()),
    }
}
