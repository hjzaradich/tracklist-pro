//! The three sides read from the database, and the `conflict` table kept
//! in step with what [`decide`] says about them (1cA-2; ROADMAP 1.10, §2).
//!
//! # Where the sides come from
//!
//! - **rekordbox's:** the track's send values ([`crate::send_values`]),
//!   which for a track rekordbox knows are its entry's attributes exactly
//!   as read. [`load_sides`] takes them from the caller, so the join that
//!   finds a track's entry exists once. A row kept from an older read
//!   (§5.3) is told by its `read_at`, as `send_values` tells it.
//! - **the base:** `sync_base`, what the last send wrote.
//! - **the app's:** [`AppValues`], given by the caller. Nothing stores the
//!   app's own values yet, so it is empty today and every field of a track
//!   rekordbox knows follows rekordbox (table row 4).
//!
//! # Keeping `conflict` in step
//!
//! An **open** conflict is a copy of three values that live elsewhere, so
//! [`detect`] may rewrite it while all three can still be read. A
//! **settled** conflict is the user's decision and the only record of the
//! value that lost; `detect` never changes or deletes one.
//!
//! Per field of each track it is given, the first row that applies:
//!
//! | The field | An open conflict on it | Then |
//! |---|---|---|
//! | belongs to a track with no current rekordbox entry (none, or a row kept from an older read) | any | nothing is touched: there's nothing of rekordbox's to compare with, and the open conflict may hold the only copy of its value |
//! | is answered by a settled conflict that still stands | any | the open one is withdrawn; the answer is the value chosen, **whatever the table says** |
//! | is to review | holds the same three texts | nothing is written |
//! | is to review | holds other texts | its three values are updated in place (same id, same `detected_at`) |
//! | is to review | none | one is opened |
//! | is anything else | any | it is withdrawn (deleted): the sides no longer disagree |
//!
//! So a rerun on unchanged sides writes nothing at all.
//!
//! **A settled conflict stands** while rekordbox's value and the base are
//! the ones it was settled on and the app's value is either the one it was
//! settled on or the value chosen. While it stands the value chosen is what
//! goes out ([`Outcome::Decided`]), and no row of the table is asked: a
//! value given by hand that is the base's would otherwise read as "only
//! rekordbox changed" once the app holds it, and rekordbox's value, the
//! one the user turned down, would come in. Once a side moves, the table
//! applies again; if that makes a conflict, it is a new one, and the
//! settled one stays as it is. Of a field's settled conflicts only the one
//! settled last answers (the highest id: a resolve always inserts, and ids
//! only grow; the clock isn't asked).
//!
//! **A track rekordbox no longer has keeps its open conflicts**, untouched,
//! until the user resolves them or removes the Library track. If its entry
//! comes back in a later read, the same conflicts are decided again by the
//! rules above.
//!
//! `detect` isn't an operation: it's derived state, and Undo must never
//! take a detected conflict away. It inserts rows outside the operation
//! log, which is why `conflict` ids are never reused (migration 0025).
//!
//! A track that can't be sent ([`crate::send_values::CannotSend`]) is left
//! out by [`load_sides`]: nothing about it is decided, and its conflicts
//! stay as they are.

use std::collections::{HashMap, HashSet};

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    compare, decide, kind_of, Decision, Handling, Kind, Rekordbox, Sides, Verdict, NOT_TAKEN_UP,
};
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
use crate::send_values::{self, send_values, Provenance, SendValues};

/// A `conflict` row's id.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct ConflictId(pub i64);

/// Where a conflict stands. Stored in `conflict.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Open,
    /// Settled: the app's value was kept.
    KeptApp,
    /// Settled: rekordbox's value was kept.
    KeptRekordbox,
    /// Settled with a value given by hand (`resolved_value`).
    Edited,
}

impl Status {
    /// The text stored in `conflict.status`.
    pub fn stored(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::KeptApp => "kept_app",
            Status::KeptRekordbox => "kept_rekordbox",
            Status::Edited => "edited",
        }
    }
}

impl FromSql for Status {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "open" => Ok(Status::Open),
            "kept_app" => Ok(Status::KeptApp),
            "kept_rekordbox" => Ok(Status::KeptRekordbox),
            "edited" => Ok(Status::Edited),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

/// One `conflict` row: a field both sides changed, with every side's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub id: ConflictId,
    pub library_track: LibraryTrackId,
    /// The rekordbox XML attribute name, e.g. `Rating`.
    pub field: String,
    pub app_value: Option<String>,
    /// Empty when rekordbox's entry has no such attribute.
    pub rekordbox_value: Option<String>,
    /// What was last sent; `None` when the field was never sent.
    pub base_value: Option<String>,
    pub status: Status,
    /// The value given by hand, when the status is [`Status::Edited`].
    pub resolved_value: Option<String>,
    pub detected_at: String,
    pub resolved_at: Option<String>,
}

impl Conflict {
    /// The value the user chose; `None` while the conflict is open.
    pub fn chosen(&self) -> Option<&str> {
        chosen(
            self.status,
            &self.app_value,
            &self.rekordbox_value,
            &self.resolved_value,
        )
    }
}

/// The value a conflict's status says was chosen, among its values.
fn chosen<'a>(
    status: Status,
    app: &'a Option<String>,
    rekordbox: &'a Option<String>,
    resolved: &'a Option<String>,
) -> Option<&'a str> {
    let value = match status {
        Status::Open => return None,
        Status::KeptApp => app,
        Status::KeptRekordbox => rekordbox,
        Status::Edited => resolved,
    };
    Some(value.as_deref().unwrap_or(""))
}

/// The columns [`conflict_from`] reads, in its order.
pub(super) const CONFLICT_COLUMNS: &str = "id, library_track_id, field, app_value, \
     rekordbox_value, base_value, status, resolved_value, detected_at, resolved_at";

/// A row selected as [`CONFLICT_COLUMNS`].
pub(super) fn conflict_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<Conflict> {
    Ok(Conflict {
        id: ConflictId(row.get(0)?),
        library_track: LibraryTrackId(row.get(1)?),
        field: row.get(2)?,
        app_value: row.get(3)?,
        rekordbox_value: row.get(4)?,
        base_value: row.get(5)?,
        status: row.get(6)?,
        resolved_value: row.get(7)?,
        detected_at: row.get(8)?,
        resolved_at: row.get(9)?,
    })
}

/// Which open conflicts: every one, or only those of one field, of some
/// tracks, or both. "All ratings" is `field: Some("Rating")`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub field: Option<String>,
    pub tracks: Option<Vec<LibraryTrackId>>,
}

impl Filter {
    /// The filter's tracks as a JSON array of ids, for `json_each`.
    pub(super) fn tracks_json(&self) -> Option<String> {
        self.tracks.as_ref().map(|tracks| json_ids(tracks))
    }
}

/// The WHERE clause that applies a [`Filter`] to open conflicts: `?1` is an
/// id (or NULL for any), `?2` the field (or NULL), `?3` the tracks as a
/// JSON array (or NULL).
pub(super) const OPEN_WHERE: &str = "status = 'open'
     AND (?1 IS NULL OR id = ?1)
     AND (?2 IS NULL OR field = ?2)
     AND (?3 IS NULL OR library_track_id IN (SELECT value FROM json_each(?3)))";

/// The open conflicts `filter` matches, oldest first: the review queue.
pub fn open_conflicts(conn: &Connection, filter: &Filter) -> rusqlite::Result<Vec<Conflict>> {
    conn.prepare(&format!(
        "SELECT {CONFLICT_COLUMNS} FROM conflict WHERE {OPEN_WHERE} ORDER BY id"
    ))?
    .query_map(
        params![None::<i64>, filter.field, filter.tracks_json()],
        conflict_from,
    )?
    .collect()
}

fn json_ids(ids: &[LibraryTrackId]) -> String {
    serde_json::to_string(&ids.iter().map(|id| id.0).collect::<Vec<_>>())
        .expect("numbers always serialize")
}

/// A value of the app's own for one field of one Library track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppValue {
    /// The rekordbox XML attribute name.
    pub field: String,
    /// The value; empty when the app wants the field empty.
    pub value: String,
    /// The value rekordbox showed that this one replaced ([`Sides::replaced`]);
    /// `None` when that isn't known.
    pub replaced: Option<String>,
}

/// The app's own values, per Library track: the fields the app means to
/// change (ROADMAP 1.9 rule 1). A field that isn't here follows rekordbox.
///
/// Nothing stores these yet. Whatever does (the edit store) hands them in
/// through this type; an empty one is today's app.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppValues {
    by_track: HashMap<i64, Vec<AppValue>>,
}

impl AppValues {
    /// Sets the app's value for one field of a track, replacing an earlier
    /// one for that field.
    pub fn set(&mut self, track: LibraryTrackId, value: AppValue) {
        let values = self.by_track.entry(track.0).or_default();
        values.retain(|v| v.field != value.field);
        values.push(value);
    }

    /// The app's own values for a track, in the order they were set.
    pub fn of(&self, track: LibraryTrackId) -> &[AppValue] {
        self.by_track.get(&track.0).map_or(&[], Vec::as_slice)
    }
}

/// rekordbox's entry for a Library track, as far as merging goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RekordboxEntry {
    /// No trusted entry in the newest read: nothing of rekordbox's to merge.
    NotThere,
    /// A row kept from an older, incomplete read.
    Stale,
    /// The entry of the newest read.
    Current,
}

/// One field's sides, owned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldSides {
    /// The rekordbox XML attribute name.
    pub field: String,
    /// The app's own value; `None` when it has none ([`Sides::app`]).
    pub app: Option<String>,
    /// rekordbox's value; `None` when its entry has no such attribute (or
    /// there is no current entry).
    pub rekordbox: Option<String>,
    /// The value last sent; `None` when the field was never sent.
    pub base: Option<String>,
    /// What the app's value replaced ([`Sides::replaced`]).
    pub replaced: Option<String>,
}

/// The sides of every field of one Library track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackSides {
    pub library_track: LibraryTrackId,
    pub entry: RekordboxEntry,
    pub fields: Vec<FieldSides>,
}

impl TrackSides {
    /// One of the track's fields as [`decide`] takes it.
    pub fn sides<'a>(&self, field: &'a FieldSides) -> Sides<'a> {
        Sides {
            app: field.app.as_deref(),
            rekordbox: match self.entry {
                RekordboxEntry::NotThere => Rekordbox::NotThere,
                RekordboxEntry::Stale => Rekordbox::Stale,
                RekordboxEntry::Current => Rekordbox::Value(field.rekordbox.as_deref()),
            },
            base: field.base.as_deref(),
            replaced: field.replaced.as_deref(),
        }
    }
}

/// Whether a field is one that is merged at all (table row 1).
fn merged(field: &str) -> bool {
    !matches!(kind_of(field), Kind::Analysis | Kind::TrackId)
}

/// The sides of every track in `sent` that can be sent: rekordbox's from
/// the send values themselves, the base from `sync_base`, the app's from
/// `app`. A fixed number of queries however many tracks. Every field any
/// side names is there, and every field with an open conflict, so
/// [`detect`] decides each open conflict again. Analysis fields and
/// `TrackID` are left out: they're never merged.
///
/// `sent` must be the tracks' send values from the same read of the
/// database (the same transaction, or nothing written in between).
pub fn load_sides(
    conn: &Connection,
    sent: &[SendValues],
    app: &AppValues,
) -> rusqlite::Result<Vec<TrackSides>> {
    let ready: Vec<(LibraryTrackId, &send_values::TrackValues)> = sent
        .iter()
        .filter_map(|s| match &s.outcome {
            send_values::Outcome::Ready(values) => Some((s.library_track, values)),
            send_values::Outcome::CannotSend(_) => None,
        })
        .collect();
    let ids = json_ids(&ready.iter().map(|(id, _)| *id).collect::<Vec<_>>());

    let mut bases: HashMap<i64, Vec<(String, String)>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT library_track_id, field, value FROM sync_base
         WHERE library_track_id IN (SELECT value FROM json_each(?1))
         ORDER BY library_track_id, field",
    )?;
    for row in stmt.query_map([&ids], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))? {
        let (track, base): (i64, (String, String)) = row?;
        bases.entry(track).or_default().push(base);
    }

    let mut open: HashMap<i64, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT library_track_id, field FROM conflict
         WHERE status = 'open' AND library_track_id IN (SELECT value FROM json_each(?1))
         ORDER BY library_track_id, field",
    )?;
    for row in stmt.query_map([&ids], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (track, field): (i64, String) = row?;
        open.entry(track).or_default().push(field);
    }

    // Rows an incomplete read kept from an earlier one, by `TrackID` (the
    // current read never gives a kept row's id to another track).
    let stale: HashSet<i64> = conn
        .prepare(
            "SELECT track_id FROM rekordbox_track
             WHERE read_at < (SELECT max(read_at) FROM rekordbox_track)",
        )?
        .query_map([], |r| r.get::<_, Option<i64>>(0))?
        .filter_map(Result::transpose)
        .collect::<rusqlite::Result<_>>()?;

    let mut tracks = Vec::with_capacity(ready.len());
    for (id, values) in ready {
        let entry = if !values.in_rekordbox {
            RekordboxEntry::NotThere
        } else {
            let track_id = values.values.iter().find_map(|v| match v.provenance {
                Provenance::Rekordbox { track_id } => Some(track_id),
                _ => None,
            });
            match track_id {
                Some(track_id) if stale.contains(&track_id) => RekordboxEntry::Stale,
                _ => RekordboxEntry::Current,
            }
        };
        let mut fields: Vec<FieldSides> = Vec::new();
        if values.in_rekordbox {
            for value in values.values.iter().filter(|v| merged(&v.attribute)) {
                slot(&mut fields, &value.attribute).rekordbox = Some(value.value.clone());
            }
        }
        for (field, value) in bases.remove(&id.0).unwrap_or_default() {
            if merged(&field) {
                slot(&mut fields, &field).base = Some(value);
            }
        }
        for own in app.of(id).iter().filter(|own| merged(&own.field)) {
            let field = slot(&mut fields, &own.field);
            field.app = Some(own.value.clone());
            field.replaced.clone_from(&own.replaced);
        }
        for field in open.remove(&id.0).unwrap_or_default() {
            slot(&mut fields, &field);
        }
        tracks.push(TrackSides {
            library_track: id,
            entry,
            fields,
        });
    }
    Ok(tracks)
}

/// The entry for `name` in `fields`, added at the end if it isn't there.
fn slot<'a>(fields: &'a mut Vec<FieldSides>, name: &str) -> &'a mut FieldSides {
    let at = match fields.iter().position(|f| f.field == name) {
        Some(at) => at,
        None => {
            fields.push(FieldSides {
                field: name.to_owned(),
                ..FieldSides::default()
            });
            fields.len() - 1
        }
    };
    &mut fields[at]
}

/// What a send does about one field, after [`detect`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum Outcome {
    /// The app's value goes out.
    SendApp,
    /// rekordbox's value is taken in: the app's own value is behind it.
    TakeRekordbox,
    /// A conflict is open on the field: the user decides.
    Open { conflict: ConflictId },
    /// The user decided: this value goes out.
    Decided { conflict: ConflictId, value: String },
    /// rekordbox's row is from an older read: nothing is decided.
    Wait,
}

/// [`detect`]'s answer for one field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FieldOutcome {
    pub library_track: LibraryTrackId,
    pub field: String,
    pub decision: Decision,
    pub outcome: Outcome,
}

/// What [`detect`] found and did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    /// Conflicts opened by this run.
    pub opened: Vec<ConflictId>,
    /// Open conflicts whose values this run brought up to date.
    pub updated: Vec<ConflictId>,
    /// Open conflicts this run withdrew: the sides no longer disagree, or
    /// a settled conflict answers the field.
    pub withdrawn: Vec<ConflictId>,
    /// What the send does about each field that needs more than rekordbox's
    /// own value back, in the order given: every field the app holds its
    /// own value for that isn't simply in agreement, and every field with
    /// an open conflict or a decision that stands. A field that follows
    /// rekordbox isn't listed.
    pub fields: Vec<FieldOutcome>,
}

impl Detected {
    fn extend(&mut self, other: Detected) {
        self.opened.extend(other.opened);
        self.updated.extend(other.updated);
        self.withdrawn.extend(other.withdrawn);
        self.fields.extend(other.fields);
    }
}

/// A `conflict` row as [`detect`] needs it.
struct Stored {
    id: i64,
    app: Option<String>,
    rekordbox: Option<String>,
    base: Option<String>,
    status: Status,
    resolved_value: Option<String>,
}

impl Stored {
    /// The value chosen, of a settled row.
    fn chosen(&self) -> Option<&str> {
        chosen(
            self.status,
            &self.app,
            &self.rekordbox,
            &self.resolved_value,
        )
    }
}

/// The three values a conflict on these sides records: the app's,
/// rekordbox's (empty when its entry has no such attribute) and the base
/// (`None` when the field was never sent).
///
/// The table wants three different values. Whenever [`decide`] says
/// `Conflict` they are; a base that is one side's own text (only when
/// [`super::NOT_TAKEN_UP`] is set to review) is no third value, and is left
/// out.
fn recorded<'a>(sides: &Sides<'a>) -> (&'a str, &'a str, Option<&'a str>) {
    let app = sides.app.unwrap_or("");
    let rekordbox = match sides.rekordbox {
        Rekordbox::Value(value) => value.unwrap_or(""),
        Rekordbox::NotThere | Rekordbox::Stale => "",
    };
    let base = sides.base.filter(|&base| base != app && base != rekordbox);
    (app, rekordbox, base)
}

/// Whether a settled conflict still answers its field: rekordbox's value
/// and the base are the ones it was settled on, and the app's value is the
/// one it was settled on or the one chosen. `base` is the base a conflict
/// on these sides has.
fn stands(field: &str, settled: &Stored, sides: &Sides<'_>, base: Option<&str>) -> bool {
    let (app, rekordbox, _) = recorded(sides);
    let same = |a: Option<&str>, b: Option<&str>| compare::same(field, a, b);
    settled.base.is_some() == base.is_some()
        && same(settled.base.as_deref(), base)
        && same(settled.rekordbox.as_deref(), Some(rekordbox))
        && (same(settled.app.as_deref(), Some(app)) || same(settled.chosen(), Some(app)))
}

/// Brings the `conflict` table in step with the sides of `tracks`, by the
/// rules in the module docs, in one transaction: all of it or none. Tracks
/// that aren't given, and fields of a track that aren't named, are left
/// alone. Not an operation: nothing is logged, and a rerun on the same
/// sides writes nothing.
pub fn detect(conn: &mut Connection, tracks: &[TrackSides]) -> rusqlite::Result<Detected> {
    let tx = conn.transaction()?;
    let detected = detect_in(&tx, tracks, NOT_TAKEN_UP)?;
    tx.commit()?;
    Ok(detected)
}

/// [`detect`] with [`NOT_TAKEN_UP`] set the other way: a send that wasn't
/// taken up goes to review. For the tests of that setting.
#[cfg(test)]
pub(super) fn detect_reviewing_what_was_not_taken_up(
    conn: &mut Connection,
    tracks: &[TrackSides],
) -> rusqlite::Result<Detected> {
    let tx = conn.transaction()?;
    let detected = detect_in(&tx, tracks, Handling::Review)?;
    tx.commit()?;
    Ok(detected)
}

/// How many tracks [`detect_for`] reads and decides at a time, so a large
/// Library's values are never all in memory at once.
pub const TRACKS_AT_A_TIME: usize = 1_000;

/// [`detect`] for the Library tracks `ids`, reading their sides itself:
/// their send values, [`load_sides`], then [`detect`], a thousand tracks at
/// a time, all in one transaction, so no read of rekordbox's export can
/// land in between.
pub fn detect_for(
    conn: &mut Connection,
    volumes: &impl Volumes,
    ids: &[LibraryTrackId],
    app: &AppValues,
) -> rusqlite::Result<Detected> {
    let tx = conn.transaction()?;
    let mut detected = Detected::default();
    for some in ids.chunks(TRACKS_AT_A_TIME) {
        let sent = send_values(&tx, volumes, some)?;
        let sides = load_sides(&tx, &sent, app)?;
        detected.extend(detect_in(&tx, &sides, NOT_TAKEN_UP)?);
    }
    tx.commit()?;
    Ok(detected)
}

/// `conflict` rows by Library track and field.
type Rows = HashMap<(i64, String), Stored>;

/// Deletes the open conflict of `key`, if there is one.
fn withdraw_open(
    delete: &mut rusqlite::Statement<'_>,
    open: &mut Rows,
    key: &(i64, String),
    withdrawn: &mut Vec<ConflictId>,
) -> rusqlite::Result<()> {
    if let Some(row) = open.remove(key) {
        delete.execute([row.id])?;
        withdrawn.push(ConflictId(row.id));
    }
    Ok(())
}

/// [`detect`], inside the caller's transaction. `not_taken_up` is how
/// [`Verdict::NotTakenUp`] is handled ([`NOT_TAKEN_UP`], but for the tests
/// of its other setting).
fn detect_in(
    conn: &Connection,
    tracks: &[TrackSides],
    not_taken_up: Handling,
) -> rusqlite::Result<Detected> {
    let ids = json_ids(&tracks.iter().map(|t| t.library_track).collect::<Vec<_>>());
    // Per track and field: the open conflict, and the settled one settled
    // last (the rows come lowest id first, so the last one read stays).
    let mut open: Rows = HashMap::new();
    let mut settled: Rows = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT library_track_id, field, id, app_value, rekordbox_value, base_value, status,
                    resolved_value
             FROM conflict
             WHERE library_track_id IN (SELECT value FROM json_each(?1))
             ORDER BY id",
        )?;
        let rows = stmt.query_map([&ids], |r| {
            Ok((
                (r.get::<_, i64>(0)?, r.get::<_, String>(1)?),
                Stored {
                    id: r.get(2)?,
                    app: r.get(3)?,
                    rekordbox: r.get(4)?,
                    base: r.get(5)?,
                    status: r.get(6)?,
                    resolved_value: r.get(7)?,
                },
            ))
        })?;
        for row in rows {
            let (key, stored) = row?;
            match stored.status {
                Status::Open => open.insert(key, stored),
                _ => settled.insert(key, stored),
            };
        }
    }

    let mut insert = conn.prepare(
        "INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    let mut update = conn.prepare(
        "UPDATE conflict SET app_value = ?2, rekordbox_value = ?3, base_value = ?4
         WHERE id = ?1 AND status = 'open'",
    )?;
    let mut delete = conn.prepare("DELETE FROM conflict WHERE id = ?1 AND status = 'open'")?;

    let mut detected = Detected::default();
    for track in tracks {
        for field in &track.fields {
            let sides = track.sides(field);
            let decision = decide(&field.field, &sides);
            let key = (track.library_track.0, field.field.clone());
            let handling = decision.verdict.handling_when(not_taken_up);
            let merged = decision.verdict != Verdict::Never;
            // The base a conflict on this field has: the field's own,
            // unless a field that isn't a `Conflict` is sent to review
            // (see `recorded`).
            let base = if handling == Handling::Review && decision.verdict != Verdict::Conflict {
                recorded(&sides).2
            } else {
                sides.base
            };
            let outcome = if !merged {
                None
            } else if track.entry != RekordboxEntry::Current {
                // Nothing of rekordbox's to compare with: no row is touched,
                // and a conflict that is open stays open.
                match (handling, open.get(&key)) {
                    (Handling::Wait, _) => Some(Outcome::Wait),
                    (_, Some(row)) => Some(Outcome::Open {
                        conflict: ConflictId(row.id),
                    }),
                    (_, None) => Some(Outcome::SendApp),
                }
            } else if let Some(row) = settled
                .get(&key)
                .filter(|row| stands(&field.field, row, &sides, base))
            {
                // The user's decision answers the field, whatever the table
                // would say now.
                withdraw_open(&mut delete, &mut open, &key, &mut detected.withdrawn)?;
                Some(Outcome::Decided {
                    conflict: ConflictId(row.id),
                    value: row.chosen().unwrap_or("").to_owned(),
                })
            } else {
                match handling {
                    // Only without a current entry, handled above.
                    Handling::Wait => Some(Outcome::Wait),
                    // The sides don't disagree (any more).
                    Handling::Nothing | Handling::SendApp | Handling::TakeRekordbox => {
                        withdraw_open(&mut delete, &mut open, &key, &mut detected.withdrawn)?;
                        match handling {
                            Handling::SendApp => Some(Outcome::SendApp),
                            Handling::TakeRekordbox => Some(Outcome::TakeRekordbox),
                            _ => None,
                        }
                    }
                    Handling::Review => {
                        let (app, rekordbox, base) = recorded(&sides);
                        if let Some(row) = open.get_mut(&key) {
                            let current = row.app.as_deref() == Some(app)
                                && row.rekordbox.as_deref() == Some(rekordbox)
                                && row.base.as_deref() == base;
                            if !current {
                                update.execute(params![row.id, app, rekordbox, base])?;
                                row.app = Some(app.to_owned());
                                row.rekordbox = Some(rekordbox.to_owned());
                                row.base = base.map(str::to_owned);
                                detected.updated.push(ConflictId(row.id));
                            }
                            Some(Outcome::Open {
                                conflict: ConflictId(row.id),
                            })
                        } else {
                            insert.execute(params![key.0, key.1, app, rekordbox, base])?;
                            let id = conn.last_insert_rowid();
                            open.insert(
                                key,
                                Stored {
                                    id,
                                    app: Some(app.to_owned()),
                                    rekordbox: Some(rekordbox.to_owned()),
                                    base: base.map(str::to_owned),
                                    status: Status::Open,
                                    resolved_value: None,
                                },
                            );
                            detected.opened.push(ConflictId(id));
                            Some(Outcome::Open {
                                conflict: ConflictId(id),
                            })
                        }
                    }
                }
            };
            let listed = field.app.is_some()
                || matches!(
                    outcome,
                    Some(Outcome::Open { .. } | Outcome::Decided { .. })
                );
            if let (Some(outcome), true) = (outcome, listed) {
                detected.fields.push(FieldOutcome {
                    library_track: track.library_track,
                    field: field.field.clone(),
                    decision,
                    outcome,
                });
            }
        }
    }
    Ok(detected)
}
