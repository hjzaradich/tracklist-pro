//! The Inbox (ROADMAP 2.1): files that arrive in a music folder whose role
//! is `inbox`, and what was decided about each.
//!
//! This module is intake and reading. [`intake`] makes the items; [`items`]
//! and [`count`] read them for the screen and the sidebar badge, each item
//! with its label and reason ([`labels`]). Actions and the screen come after.
//!
//! No label is stored: "New to you", "Better file" and the rest are worked
//! out when the Inbox is read.
//!
//! Read-only on disk: nothing here opens a music file. A file's size and
//! modified time come from the `file` row the walk wrote, and whether it
//! holds still from the hash stage's own look at it (see [`intake`]).

pub mod actions;
pub mod intake;
#[cfg(test)]
mod label_tests;
pub mod labels;
#[cfg(test)]
mod tests;

use rusqlite::{Connection, ToSql};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::suggest::Reason;
use labels::{label_files, Label, Labeled};

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
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub id: i64,
    /// The `file` row (file.id).
    pub file_id: i64,
    /// The file's own name (the last part of its stored relative path), not
    /// a full path.
    pub file_name: String,
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
    /// What the file is to the user, worked out now (see [`labels`]).
    pub label: Label,
    /// Why, as an i18n key with parameters.
    pub reason: Reason,
    /// The Library track the label is about, if it is about one.
    pub library_track_id: Option<i64>,
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
    /// Only items with this label.
    pub label: Option<Label>,
}

/// How many of the `new` arrivals have each label.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LabelCounts {
    pub checking: i64,
    pub new_to_you: i64,
    pub already_have: i64,
    pub better_file: i64,
    pub other_cut: i64,
    pub rework: i64,
}

impl LabelCounts {
    fn add(&mut self, label: Label) {
        let slot = match label {
            Label::Checking => &mut self.checking,
            Label::NewToYou => &mut self.new_to_you,
            Label::AlreadyHave => &mut self.already_have,
            Label::BetterFile => &mut self.better_file,
            Label::OtherCut => &mut self.other_cut,
            Label::Rework => &mut self.rework,
        };
        *slot += 1;
    }
}

/// An Inbox folder, as far as intake is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InboxFolder {
    pub music_folder_id: i64,
    /// Its first look (the baseline: what was there is not news) isn't
    /// taken yet, so nothing arrives from it. It waits for a walk that
    /// lists everything, or the third that doesn't.
    pub waiting_for_first_look: bool,
}

/// What the sidebar badge counts: items still `new` whose file is present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InboxCount {
    /// Arrivals nobody has decided about: the badge.
    pub new: i64,
    /// New items that were already in a folder when it was added. Not in
    /// `new`.
    pub already_there: i64,
    /// The arrivals counted in `new`, by label. They add up to `new`.
    pub by_label: LabelCounts,
    /// Every Inbox folder.
    pub folders: Vec<InboxFolder>,
}

/// The Inbox's items, newest arrival first.
pub fn items(conn: &Connection, filter: &InboxFilter) -> rusqlite::Result<Vec<InboxItem>> {
    let mut sql = String::from(
        "SELECT i.id, i.file_id, f.music_folder_id, i.batch_id, b.kind, b.label, i.status,
                i.arrived_at, i.decided_at, f.present, f.online_only, f.rel_path
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
        // A row with a kind or status this build doesn't know is left out;
        // it must not fail the page.
        let (Some(batch_kind), Some(status)) =
            (BatchKind::parse(&kind), ItemStatus::parse(&status))
        else {
            return Ok(None);
        };
        let placeholder = Labeled::checking();
        Ok(Some(InboxItem {
            id: r.get(0)?,
            file_id: r.get(1)?,
            file_name: {
                let rel: String = r.get(11)?;
                rel.rsplit(['/', '\\']).next().unwrap_or(&rel).to_string()
            },
            music_folder_id: r.get(2)?,
            batch_id: r.get(3)?,
            batch_kind,
            batch_label: r.get(5)?,
            status,
            arrived_at: r.get(7)?,
            decided_at: r.get(8)?,
            present: r.get(9)?,
            online_only: r.get(10)?,
            // Filled in below, for all the rows at once.
            label: placeholder.label,
            reason: placeholder.reason,
            library_track_id: None,
        }))
    })?;
    let mut items: Vec<InboxItem> = rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let ids: Vec<i64> = items.iter().map(|i| i.file_id).collect();
    let mut labeled = label_files(conn, &ids)?;
    for item in &mut items {
        let found = labeled
            .remove(&item.file_id)
            .unwrap_or_else(Labeled::checking);
        item.label = found.label;
        item.reason = found.reason;
        item.library_track_id = found.library_track_id;
    }
    if let Some(wanted) = filter.label {
        items.retain(|i| i.label == wanted);
    }
    Ok(items)
}

/// How many items are `new` (files gone from disk aren't counted), how many
/// of the arrivals have each label, and which Inbox folders still wait for
/// their first look.
pub fn count(conn: &Connection) -> rusqlite::Result<InboxCount> {
    let (new, already_there) = conn.query_row(
        "SELECT
             COALESCE(SUM(b.kind <> 'already_there'), 0),
             COALESCE(SUM(b.kind = 'already_there'), 0)
         FROM inbox_item i
         JOIN inbox_batch b ON b.id = i.batch_id
         JOIN file f ON f.id = i.file_id
         WHERE i.status = 'new' AND f.present = 1",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )?;
    let arrivals: Vec<i64> = conn
        .prepare_cached(
            "SELECT i.file_id
             FROM inbox_item i
             JOIN inbox_batch b ON b.id = i.batch_id
             JOIN file f ON f.id = i.file_id
             WHERE i.status = 'new' AND f.present = 1 AND b.kind <> 'already_there'",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut labeled = label_files(conn, &arrivals)?;
    let mut by_label = LabelCounts::default();
    for file in &arrivals {
        by_label.add(labeled.remove(file).map_or(Label::Checking, |l| l.label));
    }
    let folders = conn
        .prepare_cached(
            "SELECT m.id, NOT EXISTS (SELECT 1 FROM inbox_batch b
                                      WHERE b.music_folder_id = m.id
                                        AND b.kind = 'already_there')
             FROM music_folder m WHERE m.role = 'inbox' ORDER BY m.id",
        )?
        .query_map([], |r| {
            Ok(InboxFolder {
                music_folder_id: r.get(0)?,
                waiting_for_first_look: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(InboxCount {
        new,
        already_there,
        by_label,
        folders,
    })
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
