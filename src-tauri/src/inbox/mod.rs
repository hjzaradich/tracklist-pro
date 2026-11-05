//! The Inbox (ROADMAP 2.1): files that arrive in a music folder whose role
//! is `inbox`, and what was decided about each.
//!
//! This module is intake and reading. [`intake`] makes the items; [`items`]
//! and [`count`] read them for the screen and the sidebar badge. Labels,
//! actions and the screen come after.
//!
//! No label is stored: "New to you", "Better file" and the rest are worked
//! out when the Inbox is read.
//!
//! Read-only on disk: nothing here opens a music file. A file's size and
//! modified time come from the `file` row the walk wrote, and whether it
//! holds still from the hash stage's own look at it (see [`intake`]).

pub mod intake;
#[cfg(test)]
mod tests;

use rusqlite::{Connection, ToSql};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::ReadPool;
use crate::ipc::IpcError;

/// How a batch of Inbox files came in. Stored in `inbox_batch.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BatchKind {
    /// Files that landed in the Inbox folder itself.
    Loose,
    /// Files that landed in one of its subfolders (a pack, an album).
    Folder,
    /// What was in the folder when it was added. Not news.
    AlreadyThere,
}

impl BatchKind {
    fn parse(name: &str) -> Option<BatchKind> {
        match name {
            "loose" => Some(BatchKind::Loose),
            "folder" => Some(BatchKind::Folder),
            "already_there" => Some(BatchKind::AlreadyThere),
            _ => None,
        }
    }
}

/// What was decided about an Inbox file. Stored in `inbox_item.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    New,
    NotNow,
    Hidden,
    Added,
}

impl ItemStatus {
    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            ItemStatus::New => "new",
            ItemStatus::NotNow => "not_now",
            ItemStatus::Hidden => "hidden",
            ItemStatus::Added => "added",
        }
    }

    fn parse(name: &str) -> Option<ItemStatus> {
        match name {
            "new" => Some(ItemStatus::New),
            "not_now" => Some(ItemStatus::NotNow),
            "hidden" => Some(ItemStatus::Hidden),
            "added" => Some(ItemStatus::Added),
            _ => None,
        }
    }
}

/// One file in the Inbox, as the frontend sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub id: i64,
    /// The `file` row (file.id).
    pub file_id: i64,
    pub music_folder_id: i64,
    pub batch_id: i64,
    pub batch_kind: BatchKind,
    /// The subfolder of a `folder` batch, relative to the music folder.
    pub batch_label: Option<String>,
    pub status: ItemStatus,
    /// When it counted as arrived (UTC, ISO 8601).
    pub arrived_at: String,
    pub decided_at: Option<String>,
    /// The file is on disk as of the last walk. A file gone from disk keeps
    /// its item.
    pub present: bool,
    /// A OneDrive placeholder: its content isn't on this device, and
    /// nothing reads it without the user's opt-in.
    pub online_only: bool,
}

/// Which items [`items`] returns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct InboxFilter {
    /// Only items with this status.
    pub status: Option<ItemStatus>,
    /// Only items of this batch. A batch asked for by id is returned
    /// whatever its kind.
    pub batch_id: Option<i64>,
    /// Also the items that were already in a folder when it was added.
    /// Left out unless asked for: they aren't arrivals.
    pub include_already_there: bool,
}

/// What the sidebar badge counts: items still `new` whose file is present.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InboxCount {
    /// Arrivals nobody has decided about: the badge.
    pub new: i64,
    /// New items that were already in a folder when it was added. Not in
    /// `new`.
    pub already_there: i64,
}

/// The Inbox's items, newest arrival first.
pub fn items(conn: &Connection, filter: &InboxFilter) -> rusqlite::Result<Vec<InboxItem>> {
    let mut sql = String::from(
        "SELECT i.id, i.file_id, f.music_folder_id, i.batch_id, b.kind, b.label, i.status,
                i.arrived_at, i.decided_at, f.present, f.online_only
         FROM inbox_item i
         JOIN inbox_batch b ON b.id = i.batch_id
         JOIN file f ON f.id = i.file_id
         WHERE 1 = 1",
    );
    let mut args: Vec<&dyn ToSql> = Vec::new();
    let status = filter.status.map(ItemStatus::as_str);
    if let Some(status) = &status {
        sql.push_str(&format!(" AND i.status = ?{}", args.len() + 1));
        args.push(status);
    }
    if let Some(batch) = &filter.batch_id {
        sql.push_str(&format!(" AND i.batch_id = ?{}", args.len() + 1));
        args.push(batch);
    } else if !filter.include_already_there {
        sql.push_str(" AND b.kind <> 'already_there'");
    }
    sql.push_str(" ORDER BY i.arrived_at DESC, i.id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(args.as_slice(), |r| {
        let kind: String = r.get(4)?;
        let status: String = r.get(6)?;
        Ok(InboxItem {
            id: r.get(0)?,
            file_id: r.get(1)?,
            music_folder_id: r.get(2)?,
            batch_id: r.get(3)?,
            batch_kind: BatchKind::parse(&kind).ok_or_else(|| unknown(4, &kind))?,
            batch_label: r.get(5)?,
            status: ItemStatus::parse(&status).ok_or_else(|| unknown(6, &status))?,
            arrived_at: r.get(7)?,
            decided_at: r.get(8)?,
            present: r.get(9)?,
            online_only: r.get(10)?,
        })
    })?;
    rows.collect()
}

fn unknown(column: usize, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        format!("unknown value {value:?}").into(),
    )
}

/// How many items are `new` (files gone from disk aren't counted).
pub fn count(conn: &Connection) -> rusqlite::Result<InboxCount> {
    conn.query_row(
        "SELECT
             COALESCE(SUM(b.kind <> 'already_there'), 0),
             COALESCE(SUM(b.kind = 'already_there'), 0)
         FROM inbox_item i
         JOIN inbox_batch b ON b.id = i.batch_id
         JOIN file f ON f.id = i.file_id
         WHERE i.status = 'new' AND f.present = 1",
        [],
        |r| {
            Ok(InboxCount {
                new: r.get(0)?,
                already_there: r.get(1)?,
            })
        },
    )
}

/// The items in the Inbox that match `filter`.
// Async, so the database call runs off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn inbox_items(
    reads: State<'_, ReadPool>,
    filter: InboxFilter,
) -> Result<Vec<InboxItem>, IpcError> {
    Ok(reads.read(move |c| items(c, &filter))?)
}

/// How many Inbox items are new: the sidebar badge.
#[tauri::command]
#[specta::specta]
pub async fn inbox_count(reads: State<'_, ReadPool>) -> Result<InboxCount, IpcError> {
    Ok(reads.read(count)?)
}
