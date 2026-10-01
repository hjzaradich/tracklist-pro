//! The command the after-send screen calls.

use tauri::State;

use super::{lists, AfterSendLists};
use crate::db::ReadPool;
use crate::ipc::IpcError;

/// The stale playlists and the manual removals, from the database as it is
/// now. Reads only.
#[tauri::command]
#[specta::specta]
pub async fn after_send_lists(reads: State<'_, ReadPool>) -> Result<AfterSendLists, IpcError> {
    Ok(reads.read(lists)?)
}
