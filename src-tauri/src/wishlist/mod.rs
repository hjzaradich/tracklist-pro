//! Wishlists (ROADMAP 2.3): a list of tracks the user wants, pasted as text
//! or read from a CSV file, and what each comes to against the music the
//! user has ([`matching`]).
//!
//! Every write is one operation ([`crate::ops`]), so one Undo takes it
//! back: creating a list (with all its items), renaming or deleting one,
//! adding or removing items, the bought mark and the notes. The match
//! result is never stored; it is worked out when the list is read.
//!
//! No network and no link opening here: the app only reads text and a file
//! the user picked. No music file is opened or written.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tauri::State;

use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::ops::{self, OpsError, Recorder};
pub use matching::{Matcher, WishMatch, WishMatchKind};
pub use parse::{WishSkipReason, WishSkippedLine, WishTrack};

pub mod matching;
pub mod parse;

#[cfg(test)]
mod tests;

/// The operation kinds, as the operation log stores them.
pub const CREATE_OPERATION: &str = "wishlist_create";
pub const RENAME_OPERATION: &str = "wishlist_rename";
pub const DELETE_OPERATION: &str = "wishlist_delete";
pub const ADD_ITEMS_OPERATION: &str = "wishlist_add_items";
pub const REMOVE_ITEM_OPERATION: &str = "wishlist_remove_item";
pub const BOUGHT_OPERATION: &str = "wishlist_bought";
pub const NOTES_OPERATION: &str = "wishlist_notes";

/// The most items one import or one list may hold, the longest name and
/// notes, and the biggest CSV file read.
pub const MAX_ITEMS: usize = 20_000;
pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_NOTES_CHARS: usize = 2_000;
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Why nothing was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WishlistRefusal {
    NameEmpty,
    NameTooLong,
    NotesTooLong,
    NoSuchWishlist,
    NoSuchItem,
    /// More items than [`MAX_ITEMS`].
    TooManyItems,
    /// The file couldn't be read, or isn't text.
    FileUnreadable,
    FileTooBig,
    /// The CSV has no column naming the artist.
    NoArtistColumn,
    /// The CSV has no column naming the title.
    NoTitleColumn,
}

/// What a wishlist command came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum WishlistOutcome {
    /// Done, as one operation (`None` when nothing changed).
    #[serde(rename_all = "camelCase")]
    Done {
        operation_id: Option<i64>,
        /// The list it made or changed.
        wishlist_id: Option<i64>,
        /// Items made.
        added: u32,
        /// Lines or rows an import couldn't take, each with where and why.
        skipped: Vec<WishSkippedLine>,
    },
    /// Nothing was written.
    #[serde(rename_all = "camelCase")]
    Refused { reason: WishlistRefusal },
}

fn refused(reason: WishlistRefusal) -> WishlistOutcome {
    WishlistOutcome::Refused { reason }
}

fn done(operation_id: Option<i64>, wishlist_id: i64) -> WishlistOutcome {
    WishlistOutcome::Done {
        operation_id,
        wishlist_id: Some(wishlist_id),
        added: 0,
        skipped: Vec::new(),
    }
}

/// Where a list came from, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WishlistSource {
    Paste,
    Csv,
}

impl WishlistSource {
    fn as_str(self) -> &'static str {
        match self {
            WishlistSource::Paste => "paste",
            WishlistSource::Csv => "csv",
        }
    }
}

/// A name as stored: trimmed, not empty, not too long.
fn clean_name(name: &str) -> Result<String, WishlistRefusal> {
    let name = name.trim();
    if name.is_empty() {
        Err(WishlistRefusal::NameEmpty)
    } else if name.chars().count() > MAX_NAME_CHARS {
        Err(WishlistRefusal::NameTooLong)
    } else {
        Ok(name.to_owned())
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn optional(value: &Option<String>) -> Value {
    value.as_deref().map_or(Value::Null, text)
}

fn wishlist_exists(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    conn.query_row("SELECT 1 FROM wishlist WHERE id = ?1", [id], |_| Ok(()))
        .optional()
        .map(|found| found.is_some())
}

fn item_wishlist(conn: &Connection, item: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT wishlist_id FROM wish_item WHERE id = ?1",
        [item],
        |r| r.get(0),
    )
    .optional()
}

/// Inserts `tracks` into `wishlist` from `first` on, as changes of `rec`.
fn insert_items(
    rec: &mut Recorder<'_>,
    wishlist: i64,
    first: i64,
    tracks: &[WishTrack],
) -> Result<(), OpsError> {
    for (position, track) in (first..).zip(tracks) {
        rec.insert(
            "wish_item",
            &[
                ("wishlist_id", Value::Integer(wishlist)),
                ("position", Value::Integer(position)),
                ("raw_artist", text(&track.artist)),
                ("raw_title", text(&track.title)),
                ("raw_version", optional(&track.version)),
                ("raw_isrc", optional(&track.isrc)),
            ],
        )?;
    }
    Ok(())
}

/// Makes a wishlist of `tracks`, one operation. With no track to make it of,
/// nothing is written (the skipped lines still come back).
fn create(
    conn: &mut Connection,
    name: &str,
    source: WishlistSource,
    parsed: parse::Parsed,
) -> Result<WishlistOutcome, OpsError> {
    let name = match clean_name(name) {
        Ok(name) => name,
        Err(reason) => return Ok(refused(reason)),
    };
    if parsed.tracks.len() > MAX_ITEMS {
        return Ok(refused(WishlistRefusal::TooManyItems));
    }
    let added = u32::try_from(parsed.tracks.len()).unwrap_or(u32::MAX);
    if parsed.tracks.is_empty() {
        return Ok(WishlistOutcome::Done {
            operation_id: None,
            wishlist_id: None,
            added: 0,
            skipped: parsed.skipped,
        });
    }
    let recorded = ops::record(
        conn,
        CREATE_OPERATION,
        &json!({ "name": name, "source": source.as_str(), "items": added }),
        |rec| {
            let id = rec.insert(
                "wishlist",
                &[("name", text(&name)), ("source", text(source.as_str()))],
            )?;
            insert_items(rec, id, 0, &parsed.tracks)?;
            Ok(id)
        },
    )?;
    Ok(WishlistOutcome::Done {
        operation_id: recorded.operation_id,
        wishlist_id: Some(recorded.value),
        added,
        skipped: parsed.skipped,
    })
}

/// A wishlist from a pasted tracklist ([`parse::paste`]).
pub fn create_from_paste(
    conn: &mut Connection,
    name: &str,
    pasted: &str,
) -> Result<WishlistOutcome, OpsError> {
    create(conn, name, WishlistSource::Paste, parse::paste(pasted))
}

/// A wishlist from the bytes of a CSV file ([`parse::csv`]).
pub fn create_from_csv(
    conn: &mut Connection,
    name: &str,
    bytes: &[u8],
) -> Result<WishlistOutcome, OpsError> {
    let parsed = match parse::csv(&String::from_utf8_lossy(bytes)) {
        Ok(parsed) => parsed,
        Err(parse::CsvProblem::NoArtistColumn) => {
            return Ok(refused(WishlistRefusal::NoArtistColumn))
        }
        Err(parse::CsvProblem::NoTitleColumn) => {
            return Ok(refused(WishlistRefusal::NoTitleColumn))
        }
        Err(parse::CsvProblem::Unreadable) => return Ok(refused(WishlistRefusal::FileUnreadable)),
    };
    create(conn, name, WishlistSource::Csv, parsed)
}

/// Adds the tracks of a pasted tracklist to the end of a wishlist.
pub fn add_from_paste(
    conn: &mut Connection,
    wishlist: i64,
    pasted: &str,
) -> Result<WishlistOutcome, OpsError> {
    if !wishlist_exists(conn, wishlist)? {
        return Ok(refused(WishlistRefusal::NoSuchWishlist));
    }
    let parsed = parse::paste(pasted);
    let held: i64 = conn.query_row(
        "SELECT count(*) FROM wish_item WHERE wishlist_id = ?1",
        [wishlist],
        |r| r.get(0),
    )?;
    if usize::try_from(held).unwrap_or(usize::MAX) + parsed.tracks.len() > MAX_ITEMS {
        return Ok(refused(WishlistRefusal::TooManyItems));
    }
    let added = u32::try_from(parsed.tracks.len()).unwrap_or(u32::MAX);
    if parsed.tracks.is_empty() {
        return Ok(WishlistOutcome::Done {
            operation_id: None,
            wishlist_id: Some(wishlist),
            added: 0,
            skipped: parsed.skipped,
        });
    }
    let recorded = ops::record(
        conn,
        ADD_ITEMS_OPERATION,
        &json!({ "wishlistId": wishlist, "items": added }),
        |rec| {
            let next: i64 = rec
                .read_rows(
                    "SELECT ifnull(max(position) + 1, 0) FROM wish_item WHERE wishlist_id = ?1",
                    [wishlist],
                    |r| r.get(0),
                )?
                .pop()
                .unwrap_or(0);
            insert_items(rec, wishlist, next, &parsed.tracks)
        },
    )?;
    Ok(WishlistOutcome::Done {
        operation_id: recorded.operation_id,
        wishlist_id: Some(wishlist),
        added,
        skipped: parsed.skipped,
    })
}

/// Renames a wishlist. No operation when the name is the one it has.
pub fn rename(conn: &mut Connection, id: i64, name: &str) -> Result<WishlistOutcome, OpsError> {
    let name = match clean_name(name) {
        Ok(name) => name,
        Err(reason) => return Ok(refused(reason)),
    };
    if !wishlist_exists(conn, id)? {
        return Ok(refused(WishlistRefusal::NoSuchWishlist));
    }
    let recorded = ops::record(
        conn,
        RENAME_OPERATION,
        &json!({ "wishlistId": id, "name": name }),
        |rec| rec.set("wishlist", id, "name", &name),
    )?;
    Ok(done(recorded.operation_id, id))
}

/// Deletes a wishlist with its items. One Undo brings all of it back.
pub fn delete(conn: &mut Connection, id: i64) -> Result<WishlistOutcome, OpsError> {
    if !wishlist_exists(conn, id)? {
        return Ok(refused(WishlistRefusal::NoSuchWishlist));
    }
    let recorded = ops::record(
        conn,
        DELETE_OPERATION,
        &json!({ "wishlistId": id }),
        |rec| {
            let items: Vec<i64> = rec.read_rows(
                "SELECT id FROM wish_item WHERE wishlist_id = ?1 ORDER BY id",
                [id],
                |r| r.get(0),
            )?;
            // The items first, each in the log: the database would delete
            // them with the list, but Undo couldn't bring those back.
            for item in items {
                rec.delete("wish_item", item)?;
            }
            rec.delete("wishlist", id)
        },
    )?;
    Ok(done(recorded.operation_id, id))
}

/// Takes one item off its wishlist.
pub fn remove_item(conn: &mut Connection, item: i64) -> Result<WishlistOutcome, OpsError> {
    let Some(wishlist) = item_wishlist(conn, item)? else {
        return Ok(refused(WishlistRefusal::NoSuchItem));
    };
    let recorded = ops::record(
        conn,
        REMOVE_ITEM_OPERATION,
        &json!({ "wishlistId": wishlist, "itemId": item }),
        |rec| rec.delete("wish_item", item),
    )?;
    Ok(done(recorded.operation_id, wishlist))
}

/// Marks an item bought, or not. No operation when it already is so.
pub fn set_bought(
    conn: &mut Connection,
    item: i64,
    bought: bool,
) -> Result<WishlistOutcome, OpsError> {
    let Some(wishlist) = item_wishlist(conn, item)? else {
        return Ok(refused(WishlistRefusal::NoSuchItem));
    };
    let recorded = ops::record(
        conn,
        BOUGHT_OPERATION,
        &json!({ "wishlistId": wishlist, "itemId": item, "bought": bought }),
        |rec| rec.set("wish_item", item, "bought", bought),
    )?;
    Ok(done(recorded.operation_id, wishlist))
}

/// Sets an item's notes; blank notes clear them. No operation when they
/// are the ones it has.
pub fn set_notes(
    conn: &mut Connection,
    item: i64,
    notes: Option<&str>,
) -> Result<WishlistOutcome, OpsError> {
    let notes = notes.map(str::trim).filter(|n| !n.is_empty());
    if notes.is_some_and(|n| n.chars().count() > MAX_NOTES_CHARS) {
        return Ok(refused(WishlistRefusal::NotesTooLong));
    }
    let Some(wishlist) = item_wishlist(conn, item)? else {
        return Ok(refused(WishlistRefusal::NoSuchItem));
    };
    let recorded = ops::record(
        conn,
        NOTES_OPERATION,
        &json!({ "wishlistId": wishlist, "itemId": item }),
        |rec| rec.set("wish_item", item, "notes", notes),
    )?;
    Ok(done(recorded.operation_id, wishlist))
}

// ---- reads ----------------------------------------------------------------

/// One wishlist with how its items come out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WishlistSummary {
    pub id: i64,
    pub name: String,
    pub source: WishlistSource,
    pub created_at: String,
    pub total: u32,
    pub owned: u32,
    pub owned_other_cut: u32,
    pub rework: u32,
    pub missing: u32,
    /// Items marked bought.
    pub bought: u32,
}

/// One item of a wishlist with its result.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WishItem {
    pub id: i64,
    pub wishlist_id: i64,
    pub position: i64,
    pub artist: String,
    pub title: String,
    pub version: Option<String>,
    pub isrc: Option<String>,
    pub bought: bool,
    pub notes: Option<String>,
    /// The Inbox file that answered it (auto tick-off, 2.5).
    pub acquired_file_id: Option<i64>,
    pub acquired_at: Option<String>,
    pub result: WishMatch,
}

/// The wishlists, oldest first, with a count per result. One matcher serves
/// them all.
pub fn wishlists(conn: &Connection) -> rusqlite::Result<Vec<WishlistSummary>> {
    let mut lists: Vec<WishlistSummary> = conn
        .prepare_cached("SELECT id, name, source, created_at FROM wishlist ORDER BY id")?
        .query_map([], |r| {
            Ok(WishlistSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                source: if r.get::<_, String>(2)? == "csv" {
                    WishlistSource::Csv
                } else {
                    WishlistSource::Paste
                },
                created_at: r.get(3)?,
                total: 0,
                owned: 0,
                owned_other_cut: 0,
                rework: 0,
                missing: 0,
                bought: 0,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let at: HashMap<i64, usize> = lists.iter().enumerate().map(|(i, l)| (l.id, i)).collect();
    let items = read_items(conn, None)?;
    if items.is_empty() {
        return Ok(lists);
    }
    let matcher = Matcher::build(conn)?;
    for (wishlist, track, bought) in items {
        let Some(&i) = at.get(&wishlist) else {
            continue;
        };
        let list = &mut lists[i];
        list.total += 1;
        list.bought += u32::from(bought);
        match matcher.find(&track).kind {
            WishMatchKind::Owned => list.owned += 1,
            WishMatchKind::OwnedOtherCut => list.owned_other_cut += 1,
            WishMatchKind::Rework => list.rework += 1,
            WishMatchKind::Missing => list.missing += 1,
        }
    }
    Ok(lists)
}

/// The items (of one wishlist, or all) as the matcher reads them, in order.
fn read_items(
    conn: &Connection,
    wishlist: Option<i64>,
) -> rusqlite::Result<Vec<(i64, WishTrack, bool)>> {
    conn.prepare_cached(
        "SELECT wishlist_id, raw_artist, raw_title, raw_version, raw_isrc, bought
         FROM wish_item WHERE ?1 IS NULL OR wishlist_id = ?1
         ORDER BY wishlist_id, position",
    )?
    .query_map([wishlist], |r| {
        Ok((
            r.get(0)?,
            WishTrack {
                artist: r.get(1)?,
                title: r.get(2)?,
                version: r.get(3)?,
                isrc: r.get(4)?,
            },
            r.get(5)?,
        ))
    })?
    .collect()
}

/// The items of a wishlist in the order they were listed, each with its
/// result. Empty for a list that isn't there.
pub fn items(conn: &Connection, wishlist: i64) -> rusqlite::Result<Vec<WishItem>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, position, raw_artist, raw_title, raw_version, raw_isrc, bought, notes,
                acquired_file_id, acquired_at
         FROM wish_item WHERE wishlist_id = ?1 ORDER BY position",
    )?;
    let rows: Vec<WishItem> = stmt
        .query_map([wishlist], |r| {
            Ok(WishItem {
                id: r.get(0)?,
                wishlist_id: wishlist,
                position: r.get(1)?,
                artist: r.get(2)?,
                title: r.get(3)?,
                version: r.get(4)?,
                isrc: r.get(5)?,
                bought: r.get(6)?,
                notes: r.get(7)?,
                acquired_file_id: r.get(8)?,
                acquired_at: r.get(9)?,
                // Filled in below, once the matcher has been built.
                result: matching::missing(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    if rows.is_empty() {
        return Ok(rows);
    }
    let matcher = Matcher::build(conn)?;
    Ok(rows
        .into_iter()
        .map(|mut item| {
            item.result = matcher.find(&WishTrack {
                artist: item.artist.clone(),
                title: item.title.clone(),
                version: item.version.clone(),
                isrc: item.isrc.clone(),
            });
            item
        })
        .collect())
}

// ---- commands -------------------------------------------------------------

/// The CSV file the user picked, read in full. Only read, never written.
fn read_file(path: &str) -> Result<Vec<u8>, WishlistRefusal> {
    let path = Path::new(path);
    let size = std::fs::metadata(path)
        .map_err(|_| WishlistRefusal::FileUnreadable)?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(WishlistRefusal::FileTooBig);
    }
    std::fs::read(path).map_err(|_| WishlistRefusal::FileUnreadable)
}

/// The wishlists with a count per result.
#[tauri::command]
#[specta::specta]
pub async fn wishlists_list(reads: State<'_, ReadPool>) -> Result<Vec<WishlistSummary>, IpcError> {
    Ok(reads.read(wishlists)?)
}

/// The items of a wishlist, each with its result and reason.
#[tauri::command]
#[specta::specta]
pub async fn wishlist_items(
    reads: State<'_, ReadPool>,
    wishlist_id: i64,
) -> Result<Vec<WishItem>, IpcError> {
    Ok(reads.read(move |c| items(c, wishlist_id))?)
}

/// A new wishlist from a pasted tracklist.
#[tauri::command(async)]
#[specta::specta]
pub fn wishlist_create_from_paste(
    writer: State<'_, Writer>,
    name: String,
    pasted: String,
) -> Result<WishlistOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(create_from_paste(c, &name, &pasted)))??)
}

/// A new wishlist from a CSV file at `path` (picked by the user in the
/// file dialog). The file is only read.
#[tauri::command(async)]
#[specta::specta]
pub fn wishlist_create_from_csv(
    writer: State<'_, Writer>,
    name: String,
    path: String,
) -> Result<WishlistOutcome, IpcError> {
    let bytes = match read_file(&path) {
        Ok(bytes) => bytes,
        Err(reason) => return Ok(refused(reason)),
    };
    Ok(writer.call(move |c| Ok(create_from_csv(c, &name, &bytes)))??)
}

/// More tracks, pasted, for the end of a wishlist.
#[tauri::command(async)]
#[specta::specta]
pub fn wishlist_add_from_paste(
    writer: State<'_, Writer>,
    wishlist_id: i64,
    pasted: String,
) -> Result<WishlistOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(add_from_paste(c, wishlist_id, &pasted)))??)
}

#[tauri::command(async)]
#[specta::specta]
pub fn wishlist_rename(
    writer: State<'_, Writer>,
    wishlist_id: i64,
    name: String,
) -> Result<WishlistOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(rename(c, wishlist_id, &name)))??)
}

#[tauri::command(async)]
#[specta::specta]
pub fn wishlist_delete(
    writer: State<'_, Writer>,
    wishlist_id: i64,
) -> Result<WishlistOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(delete(c, wishlist_id)))??)
}

#[tauri::command(async)]
#[specta::specta]
pub fn wishlist_remove_item(
    writer: State<'_, Writer>,
    item_id: i64,
) -> Result<WishlistOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(remove_item(c, item_id)))??)
}

#[tauri::command(async)]
#[specta::specta]
pub fn wishlist_set_bought(
    writer: State<'_, Writer>,
    item_id: i64,
    bought: bool,
) -> Result<WishlistOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(set_bought(c, item_id, bought)))??)
}

#[tauri::command(async)]
#[specta::specta]
pub fn wishlist_set_notes(
    writer: State<'_, Writer>,
    item_id: i64,
    notes: Option<String>,
) -> Result<WishlistOutcome, IpcError> {
    Ok(writer.call(move |c| Ok(set_notes(c, item_id, notes.as_deref())))??)
}
