//! The custom tag vocabulary (3A-T; ROADMAP 3.1): tag groups, the tags in
//! them, and the tags applied to tracks.
//!
//! - **Every change is one recorded operation** ([`crate::ops`]), so Undo
//!   puts the earlier state back exactly. The import from rekordbox is one
//!   operation too ([`import`]).
//! - **A tag is applied to a track** (`recording`), not to a Library track,
//!   so it stays with the track whether or not it is in the Library.
//! - **Names** are trimmed and can't be empty. A group's name is unique among
//!   groups; a tag's among all tags, by the XML writer's sibling rule
//!   ([`crate::rekordbox_write::sibling_key`]), because a send writes each
//!   tag as a playlist under `Playlists/Tags` ([`crate::send::tag_playlists`])
//!   and a tag name the writer can't carry is refused here, so a tag the app
//!   lets the user make is never the reason a send is refused.
//! - **Colours** are `#rrggbb` or none; the screen draws a plain default.
//! - **Nothing is ever written into a track's comment** (ROADMAP 1.9 rule 1;
//!   writing My Tags is untested, T5).
//!
//! Nothing here touches a file on disk.

pub mod import;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{DbError, ReadPool, Writer};
use crate::ipc::{ErrorKind, ErrorParam, IpcError};
use crate::ops::{self, OpsError};
use crate::rekordbox_write::{build, sibling_key, Node, SendInput};

/// The operation kinds, as the operation log stores them.
pub const CREATE_GROUP_OPERATION: &str = "create_tag_group";
pub const CREATE_TAG_OPERATION: &str = "create_tag";
pub const RENAME_GROUP_OPERATION: &str = "rename_tag_group";
pub const RENAME_TAG_OPERATION: &str = "rename_tag";
pub const COLOUR_GROUP_OPERATION: &str = "colour_tag_group";
pub const COLOUR_TAG_OPERATION: &str = "colour_tag";
pub const APPLY_OPERATION: &str = "apply_tag";
pub const REMOVE_OPERATION: &str = "remove_tag";
pub const IMPORT_OPERATION: &str = "import_tags";

/// A tag group's row id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct TagGroupId(pub i64);

/// A tag's row id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct TagId(pub i64);

/// A group or a tag, for the commands that treat both alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VocabItem {
    Group { id: TagGroupId },
    Tag { id: TagId },
}

/// A tag, as the vocabulary panel lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: TagId,
    pub group_id: TagGroupId,
    pub name: String,
    pub colour: Option<String>,
    /// The rekordbox My Tag it was copied from, as rekordbox spells it.
    pub rekordbox_name: Option<String>,
    /// The last import didn't find that My Tag in rekordbox any more. The
    /// tag stays; the app never deletes one by itself.
    pub gone_from_rekordbox: bool,
    /// How many tracks have it.
    pub track_count: u32,
}

/// A group and its tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TagGroup {
    pub id: TagGroupId,
    pub name: String,
    pub colour: Option<String>,
    pub tags: Vec<Tag>,
}

/// What applying a tag to tracks, or removing it, did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TagsChanged {
    /// How many tracks gained or lost the tag.
    pub changed: u32,
    /// How many were left alone: they had it already (applying), didn't have
    /// it (removing), or no longer exist.
    pub skipped: u32,
    /// The operation this recorded; `None` when nothing changed.
    pub operation_id: Option<i64>,
}

/// Why a vocabulary command was refused. Nothing was changed.
#[derive(Debug)]
pub enum VocabError {
    GroupNotFound,
    TagNotFound,
    /// The name is empty once trimmed.
    NameEmpty,
    /// Another group (or tag) has this name; `existing` is that one's name
    /// as stored.
    NameTaken {
        existing: String,
    },
    /// The tag name holds a character XML can't carry, so a send couldn't
    /// write its playlist.
    NameUnsendable,
    /// A colour is `#rrggbb`.
    ColourInvalid,
    Ops(OpsError),
}

impl From<OpsError> for VocabError {
    fn from(e: OpsError) -> Self {
        VocabError::Ops(e)
    }
}

impl From<rusqlite::Error> for VocabError {
    fn from(e: rusqlite::Error) -> Self {
        VocabError::Ops(OpsError::from(e))
    }
}

impl From<DbError> for VocabError {
    fn from(e: DbError) -> Self {
        VocabError::Ops(OpsError::Db(e))
    }
}

impl From<VocabError> for IpcError {
    fn from(e: VocabError) -> Self {
        match e {
            VocabError::GroupNotFound => IpcError::new(ErrorKind::TagGroupNotFound),
            VocabError::TagNotFound => IpcError::new(ErrorKind::TagNotFound),
            VocabError::NameEmpty => IpcError::new(ErrorKind::TagNameEmpty),
            VocabError::NameTaken { existing } => IpcError::with(
                ErrorKind::TagNameTaken,
                [("name", ErrorParam::from(existing))],
            ),
            VocabError::NameUnsendable => IpcError::new(ErrorKind::TagNameUnsendable),
            VocabError::ColourInvalid => IpcError::new(ErrorKind::TagColourInvalid),
            VocabError::Ops(e) => IpcError::from(e),
        }
    }
}

/// A command's answer: the refusal, or what it did.
pub type Done<T> = Result<T, VocabError>;

/// Whether the XML writer can write `name` as a playlist's name. The
/// writer's own check, not a copy.
pub(crate) fn sendable(name: &str) -> bool {
    let alone = SendInput {
        playlists: vec![Node::Playlist {
            name: name.to_owned(),
            entries: Vec::new(),
        }],
        ..SendInput::default()
    };
    build(&alone).is_ok()
}

/// A name as it will be stored: trimmed, not empty.
fn trimmed(name: &str) -> Done<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(VocabError::NameEmpty);
    }
    Ok(name.to_owned())
}

/// The names in `table` (`tag_group` or `tag`), with ids, other than `except`.
fn names_in(
    rec: &ops::Recorder<'_>,
    table: &str,
    except: Option<i64>,
) -> Result<Vec<(i64, String)>, OpsError> {
    let sql = match table {
        "tag_group" => "SELECT id, name FROM tag_group WHERE id <> ?1",
        _ => "SELECT id, name FROM tag WHERE id <> ?1",
    };
    rec.read_rows(sql, [except.unwrap_or(0)], |r| Ok((r.get(0)?, r.get(1)?)))
}

/// The name as it will be stored, if no other name in `taken` equals it by
/// the writer's sibling rule.
fn free_name(taken: &[(i64, String)], name: &str) -> Done<String> {
    let name = trimmed(name)?;
    let key = sibling_key(&name);
    match taken.iter().find(|(_, other)| sibling_key(other) == key) {
        Some((_, existing)) => Err(VocabError::NameTaken {
            existing: existing.clone(),
        }),
        None => Ok(name),
    }
}

/// The colour as it will be stored: `#rrggbb` in lower case, or none.
fn checked_colour(colour: Option<&str>) -> Done<Option<String>> {
    let Some(colour) = colour else {
        return Ok(None);
    };
    let lower = colour.trim().to_ascii_lowercase();
    let digits = lower.strip_prefix('#').unwrap_or("");
    if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(Some(lower))
    } else {
        Err(VocabError::ColourInvalid)
    }
}

fn next_position(
    rec: &ops::Recorder<'_>,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<i64, OpsError> {
    Ok(rec
        .read_rows(sql, params, |r| r.get::<_, i64>(0))?
        .first()
        .copied()
        .unwrap_or(0))
}

fn group_exists(rec: &ops::Recorder<'_>, id: TagGroupId) -> Result<bool, OpsError> {
    Ok(!rec
        .read_rows("SELECT 1 FROM tag_group WHERE id = ?1", [id.0], |r| {
            r.get::<_, i64>(0)
        })?
        .is_empty())
}

/// Makes a group, last among the groups. Recorded as one operation.
pub fn create_group_on(conn: &mut Connection, name: &str) -> Result<Done<TagGroupId>, OpsError> {
    let name = name.trim().to_owned();
    ops::record(
        conn,
        CREATE_GROUP_OPERATION,
        &serde_json::json!({ "name": name }),
        |rec| make_group_in(rec, &name),
    )
    .map(|recorded| recorded.value)
}

/// The writes of [`create_group_on`], inside the operation `rec` records.
/// Every refusal comes before the first write.
pub(crate) fn make_group_in(
    rec: &mut ops::Recorder<'_>,
    name: &str,
) -> Result<Done<TagGroupId>, OpsError> {
    let name = match free_name(&names_in(rec, "tag_group", None)?, name) {
        Ok(name) => name,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let position = next_position(
        rec,
        "SELECT ifnull(max(position), 0) + 1 FROM tag_group",
        [],
    )?;
    let id = rec.insert(
        "tag_group",
        &[("name", name.into()), ("position", position.into())],
    )?;
    Ok(Ok(TagGroupId(id)))
}

/// Makes a tag in `group`, last among its tags. Recorded as one operation.
pub fn create_tag_on(
    conn: &mut Connection,
    group: TagGroupId,
    name: &str,
) -> Result<Done<TagId>, OpsError> {
    let name = name.trim().to_owned();
    ops::record(
        conn,
        CREATE_TAG_OPERATION,
        &serde_json::json!({ "name": name, "groupId": group.0 }),
        |rec| make_tag_in(rec, group, &name, None),
    )
    .map(|recorded| recorded.value)
}

/// The writes of [`create_tag_on`]; `rekordbox_name` links the tag to the My
/// Tag it is copied from.
pub(crate) fn make_tag_in(
    rec: &mut ops::Recorder<'_>,
    group: TagGroupId,
    name: &str,
    rekordbox_name: Option<&str>,
) -> Result<Done<TagId>, OpsError> {
    if !group_exists(rec, group)? {
        return Ok(Err(VocabError::GroupNotFound));
    }
    let name = match free_name(&names_in(rec, "tag", None)?, name) {
        Ok(name) => name,
        Err(refusal) => return Ok(Err(refusal)),
    };
    if !sendable(&name) {
        return Ok(Err(VocabError::NameUnsendable));
    }
    let position = next_position(
        rec,
        "SELECT ifnull(max(position), 0) + 1 FROM tag WHERE group_id = ?1",
        [group.0],
    )?;
    let mut values = vec![
        ("group_id", group.0.into()),
        ("name", name.into()),
        ("position", position.into()),
    ];
    if let Some(from) = rekordbox_name {
        values.push(("rekordbox_name", from.to_owned().into()));
    }
    Ok(Ok(TagId(rec.insert("tag", &values)?)))
}

/// What a group or tag is called now, if it exists.
fn name_of(rec: &ops::Recorder<'_>, item: VocabItem) -> Result<Option<String>, OpsError> {
    let (sql, id) = match item {
        VocabItem::Group { id } => ("SELECT name FROM tag_group WHERE id = ?1", id.0),
        VocabItem::Tag { id } => ("SELECT name FROM tag WHERE id = ?1", id.0),
    };
    Ok(rec.read_rows(sql, [id], |r| r.get(0))?.into_iter().next())
}

fn not_found(item: VocabItem) -> VocabError {
    match item {
        VocabItem::Group { .. } => VocabError::GroupNotFound,
        VocabItem::Tag { .. } => VocabError::TagNotFound,
    }
}

/// Renames a group or tag and returns the operation it recorded; renaming it
/// to what it is called already records nothing (`None`).
pub fn rename_on(
    conn: &mut Connection,
    item: VocabItem,
    name: &str,
) -> Result<Done<Option<i64>>, OpsError> {
    let name = name.trim().to_owned();
    let kind = match item {
        VocabItem::Group { .. } => RENAME_GROUP_OPERATION,
        VocabItem::Tag { .. } => RENAME_TAG_OPERATION,
    };
    ops::record(conn, kind, &serde_json::json!({ "name": name }), |rec| {
        let Some(from) = name_of(rec, item)? else {
            return Ok(Err(not_found(item)));
        };
        let (table, id) = match item {
            VocabItem::Group { id } => ("tag_group", id.0),
            VocabItem::Tag { id } => ("tag", id.0),
        };
        let name = match free_name(&names_in(rec, table, Some(id))?, &name) {
            Ok(name) => name,
            Err(refusal) => return Ok(Err(refusal)),
        };
        if table == "tag" && !sendable(&name) {
            return Ok(Err(VocabError::NameUnsendable));
        }
        rec.detail("from", from);
        rec.set(table, id, "name", name)?;
        Ok(Ok(()))
    })
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// Sets a group's or tag's colour (`#rrggbb`), or clears it. Returns the
/// operation recorded, or none when the colour is the one it has.
pub fn colour_on(
    conn: &mut Connection,
    item: VocabItem,
    colour: Option<&str>,
) -> Result<Done<Option<i64>>, OpsError> {
    let colour = match checked_colour(colour) {
        Ok(colour) => colour,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let kind = match item {
        VocabItem::Group { .. } => COLOUR_GROUP_OPERATION,
        VocabItem::Tag { .. } => COLOUR_TAG_OPERATION,
    };
    ops::record(
        conn,
        kind,
        &serde_json::json!({ "colour": colour }),
        |rec| {
            let Some(name) = name_of(rec, item)? else {
                return Ok(Err(not_found(item)));
            };
            let (table, id) = match item {
                VocabItem::Group { id } => ("tag_group", id.0),
                VocabItem::Tag { id } => ("tag", id.0),
            };
            rec.detail("name", name);
            rec.set(table, id, "colour", colour)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// Applies a tag to tracks (`recordings`, `TableRow.recordingId`). A track
/// that has it, or doesn't exist, is skipped; one listed twice counts once.
/// Recorded as one operation; if nothing changed, nothing is recorded.
pub fn apply_on(
    conn: &mut Connection,
    tag: TagId,
    recordings: &[i64],
) -> Result<Done<TagsChanged>, OpsError> {
    change_on(conn, tag, recordings, true)
}

/// Takes a tag off tracks; the reverse of [`apply_on`].
pub fn remove_on(
    conn: &mut Connection,
    tag: TagId,
    recordings: &[i64],
) -> Result<Done<TagsChanged>, OpsError> {
    change_on(conn, tag, recordings, false)
}

fn change_on(
    conn: &mut Connection,
    tag: TagId,
    recordings: &[i64],
    apply: bool,
) -> Result<Done<TagsChanged>, OpsError> {
    let kind = if apply {
        APPLY_OPERATION
    } else {
        REMOVE_OPERATION
    };
    let given: Vec<i64> = recordings.to_vec();
    ops::record(conn, kind, &serde_json::json!({ "tagId": tag.0 }), |rec| {
        let Some(name) = name_of(rec, VocabItem::Tag { id: tag })? else {
            return Ok(Err(VocabError::TagNotFound));
        };
        let json = serde_json::to_string(&given).expect("numbers always serialize");
        let exist: HashSet<i64> = rec
            .read_rows(
                "SELECT id FROM recording WHERE id IN (SELECT value FROM json_each(?1))",
                [&json],
                |r| r.get(0),
            )?
            .into_iter()
            .collect();
        let have: std::collections::HashMap<i64, i64> = rec
            .read_rows(
                "SELECT recording_id, id FROM tag_applied
                 WHERE tag_id = ?1 AND recording_id IN (SELECT value FROM json_each(?2))",
                (tag.0, &json),
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            .into_iter()
            .collect();
        let mut seen = HashSet::new();
        let mut changed = 0u32;
        for &recording in &given {
            if !exist.contains(&recording) || !seen.insert(recording) {
                continue;
            }
            match (apply, have.get(&recording)) {
                (true, None) => {
                    rec.insert(
                        "tag_applied",
                        &[("recording_id", recording.into()), ("tag_id", tag.0.into())],
                    )?;
                    changed += 1;
                }
                (false, Some(&row)) => {
                    rec.delete("tag_applied", row)?;
                    changed += 1;
                }
                _ => {}
            }
        }
        rec.detail("name", name);
        rec.detail("tracks", changed);
        Ok(Ok(TagsChanged {
            changed,
            skipped: u32::try_from(given.len())
                .unwrap_or(u32::MAX)
                .saturating_sub(changed),
            operation_id: None,
        }))
    })
    .map(|recorded| {
        recorded.value.map(|changed| TagsChanged {
            operation_id: recorded.operation_id,
            ..changed
        })
    })
}

/// Every group with its tags, in the order they were made (or imported).
/// Reads rows one at a time, so a bad row is a database error and not bad
/// data: every column is checked in the schema.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<TagGroup>> {
    let mut groups: Vec<TagGroup> = conn
        .prepare_cached("SELECT id, name, colour FROM tag_group ORDER BY position, id")?
        .query_map([], |r| {
            Ok(TagGroup {
                id: TagGroupId(r.get(0)?),
                name: r.get(1)?,
                colour: r.get(2)?,
                tags: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare_cached(
        "SELECT t.id, t.group_id, t.name, t.colour, t.rekordbox_name, t.gone_from_rekordbox,
                (SELECT count(*) FROM tag_applied a WHERE a.tag_id = t.id)
         FROM tag t ORDER BY t.position, t.id",
    )?;
    let tags = stmt.query_map([], |r| {
        Ok(Tag {
            id: TagId(r.get(0)?),
            group_id: TagGroupId(r.get(1)?),
            name: r.get(2)?,
            colour: r.get(3)?,
            rekordbox_name: r.get(4)?,
            gone_from_rekordbox: r.get(5)?,
            track_count: r.get(6)?,
        })
    })?;
    for tag in tags {
        let tag = tag?;
        if let Some(group) = groups.iter_mut().find(|g| g.id == tag.group_id) {
            group.tags.push(tag);
        }
    }
    Ok(groups)
}

/// The tags a track has, by id.
pub fn tags_of(conn: &Connection, recording: i64) -> rusqlite::Result<Vec<TagId>> {
    conn.prepare_cached("SELECT tag_id FROM tag_applied WHERE recording_id = ?1 ORDER BY id")?
        .query_map([recording], |r| Ok(TagId(r.get(0)?)))?
        .collect()
}

// The commands. `async` runs the writes off the main thread, like the crates'.

/// Makes a tag group.
#[tauri::command]
#[specta::specta]
pub async fn create_tag_group(
    writer: State<'_, Writer>,
    name: String,
) -> Result<TagGroupId, IpcError> {
    Ok(writer.call(move |conn| Ok(create_group_on(conn, &name)))???)
}

/// Makes a tag in a group.
#[tauri::command]
#[specta::specta]
pub async fn create_tag(
    writer: State<'_, Writer>,
    group: TagGroupId,
    name: String,
) -> Result<TagId, IpcError> {
    Ok(writer.call(move |conn| Ok(create_tag_on(conn, group, &name)))???)
}

/// Renames a group or tag. Answers the operation recorded, or none.
#[tauri::command]
#[specta::specta]
pub async fn rename_tag_item(
    writer: State<'_, Writer>,
    item: VocabItem,
    name: String,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(rename_on(conn, item, &name)))???)
}

/// Sets or clears (`null`) a group's or tag's colour.
#[tauri::command]
#[specta::specta]
pub async fn colour_tag_item(
    writer: State<'_, Writer>,
    item: VocabItem,
    colour: Option<String>,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(colour_on(conn, item, colour.as_deref())))???)
}

/// Applies a tag to tracks.
#[tauri::command]
#[specta::specta]
pub async fn apply_tag(
    writer: State<'_, Writer>,
    tag: TagId,
    recordings: Vec<i64>,
) -> Result<TagsChanged, IpcError> {
    Ok(writer.call(move |conn| Ok(apply_on(conn, tag, &recordings)))???)
}

/// Takes a tag off tracks.
#[tauri::command]
#[specta::specta]
pub async fn remove_tag(
    writer: State<'_, Writer>,
    tag: TagId,
    recordings: Vec<i64>,
) -> Result<TagsChanged, IpcError> {
    Ok(writer.call(move |conn| Ok(remove_on(conn, tag, &recordings)))???)
}

/// Every group with its tags.
#[tauri::command]
#[specta::specta]
pub async fn list_tags(reads: State<'_, ReadPool>) -> Result<Vec<TagGroup>, IpcError> {
    Ok(reads.read(list)?)
}

/// The tags a track has.
#[tauri::command]
#[specta::specta]
pub async fn tags_of_track(
    reads: State<'_, ReadPool>,
    recording_id: i64,
) -> Result<Vec<TagId>, IpcError> {
    Ok(reads.read(|conn| tags_of(conn, recording_id))?)
}

/// Copies rekordbox's My Tags into the vocabulary, as one operation. New tags
/// go in the group of the earlier import, else in a group called `group_name`.
#[tauri::command]
#[specta::specta]
pub async fn import_rekordbox_tags(
    writer: State<'_, Writer>,
    group_name: String,
) -> Result<import::TagsImported, IpcError> {
    Ok(writer.call(move |conn| Ok(import::import_on(conn, &group_name)))???)
}
