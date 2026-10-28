//! Settings for how the app looks, in the `ui_setting` table (migration
//! 0034; 1cA-5): one JSON value per key.
//!
//! The first is which columns each track table shows. A choice of columns is
//! the user's taste, not Library data: it is not in the operation log, so it
//! can't be undone, and no Library reset touches it ("Start fresh" only
//! starts an empty Library). Reading never fails on a value: a missing row,
//! or one this version can't read, reads as "no choice", and the screen
//! shows its own default columns.
//!
//! The backend stores column ids as text and does not know them: the
//! frontend owns the list of columns, drops an id it no longer knows and
//! decides where a new column goes (`src/trackTable/columnChoice.ts`). The
//! widths a user dragged columns to (1cB-21) are kept in the same value, in
//! pixels by column id; a value saved without them reads as having none.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;

/// The most column ids a choice keeps, and the longest an id is. The table
/// has a handful of columns; these only stop a runaway value.
const MOST_COLUMNS: usize = 64;
const LONGEST_ID: usize = 40;
/// The widest a column is kept at, in pixels (1cB-21). The frontend owns the
/// minimums; this only stops a runaway value.
const WIDEST: u32 = 4000;

/// The screens that show the track table. A crate is one screen: every crate
/// shows the same columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TableScreen {
    Library,
    AllMusic,
    Crate,
}

impl TableScreen {
    fn key(self) -> &'static str {
        match self {
            TableScreen::Library => "table_columns.library",
            TableScreen::AllMusic => "table_columns.all_music",
            TableScreen::Crate => "table_columns.crate",
        }
    }
}

/// The columns a screen shows, as the user left them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TableColumns {
    /// The ids of the columns shown, in order.
    pub shown: Vec<String>,
    /// The ids of every column the app had when this was saved. A column
    /// added by a later version isn't in it, so it can be told from one the
    /// user chose to hide: it appears only if it is a default column.
    pub known: Vec<String>,
    /// The widths the user dragged columns to (1cB-21), in pixels, by column
    /// id. A column with no entry has its default width.
    pub widths: BTreeMap<String, u32>,
}

/// What is read back from the stored JSON: a value saved before widths
/// existed has none, which reads as "no widths", not as "no choice".
#[derive(Deserialize)]
struct Stored {
    shown: Vec<String>,
    known: Vec<String>,
    #[serde(default)]
    widths: BTreeMap<String, u32>,
}

impl TableColumns {
    /// The same ids, without blanks, repeats and anything past the limits.
    fn tidy(&self) -> TableColumns {
        fn tidy(ids: &[String]) -> Vec<String> {
            let mut kept: Vec<String> = Vec::new();
            for id in ids {
                if !id.is_empty() && id.chars().count() <= LONGEST_ID && !kept.contains(id) {
                    kept.push(id.clone());
                }
            }
            kept.truncate(MOST_COLUMNS);
            kept
        }
        TableColumns {
            shown: tidy(&self.shown),
            known: tidy(&self.known),
            widths: self
                .widths
                .iter()
                .filter(|(id, width)| {
                    !id.is_empty() && id.chars().count() <= LONGEST_ID && **width > 0
                })
                .take(MOST_COLUMNS)
                .map(|(id, width)| (id.clone(), (*width).min(WIDEST)))
                .collect(),
        }
    }
}

/// The columns the user chose for `screen`; `None` if they haven't, or the
/// stored value isn't one this version can read.
pub fn read_table_columns(
    conn: &Connection,
    screen: TableScreen,
) -> rusqlite::Result<Option<TableColumns>> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM ui_setting WHERE key = ?1",
            [screen.key()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(stored
        .and_then(|value| serde_json::from_str::<Stored>(&value).ok())
        .map(|stored| TableColumns {
            shown: stored.shown,
            known: stored.known,
            widths: stored.widths,
        }))
}

/// Stores the columns for `screen`, or forgets the choice (`None`) so the
/// screen follows its default again.
pub fn write_table_columns(
    conn: &Connection,
    screen: TableScreen,
    columns: Option<&TableColumns>,
) -> rusqlite::Result<()> {
    match columns {
        Some(columns) => {
            let value = serde_json::to_string(&columns.tidy()).expect("ids always serialize");
            conn.execute(
                "INSERT INTO ui_setting (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET
                     value = excluded.value,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                (screen.key(), value),
            )?;
        }
        None => {
            conn.execute("DELETE FROM ui_setting WHERE key = ?1", [screen.key()])?;
        }
    }
    Ok(())
}

/// The columns the user chose for a screen's track table; `None` until they
/// choose.
// Async, so the database call runs off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn table_columns(
    reads: State<'_, ReadPool>,
    screen: TableScreen,
) -> Result<Option<TableColumns>, IpcError> {
    Ok(reads.read(move |c| read_table_columns(c, screen))?)
}

/// Sets the columns a screen's track table shows, or (`None`) goes back to
/// the screen's default. Not an operation: it can't be undone.
#[tauri::command]
#[specta::specta]
pub async fn set_table_columns(
    writer: State<'_, Writer>,
    screen: TableScreen,
    columns: Option<TableColumns>,
) -> Result<(), IpcError> {
    Ok(writer.call(move |c| write_table_columns(c, screen, columns.as_ref()))?)
}

#[cfg(test)]
mod tests;
