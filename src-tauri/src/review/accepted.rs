//! The view of what "Accept all" switched (1bE-3; ROADMAP 1.12): for each
//! switched track, its name and the old and new file's names. Read-only. The
//! switch itself ([`super::actions::accept_clear_best_files`]) returns ids
//! only; the screen asks for the words here when the user opens "View".

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::compare::file_names;
use super::queue::{name_of, names};
use super::ReviewTrackName;
use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::paths::Volumes;
use crate::scan::system_volumes;

/// One track "Accept all" switched, as the switch returned it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SwitchedRef {
    pub recording_id: i64,
    pub from_file_id: i64,
    pub to_file_id: i64,
}

/// A switched track with its name and the names of its old and new file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SwitchedView {
    pub recording_id: i64,
    pub track: ReviewTrackName,
    pub from_name: String,
    pub to_name: String,
}

/// The names for `switched`, in the same order. Nothing is changed.
pub fn switched_view(
    conn: &Connection,
    volumes: &impl Volumes,
    switched: &[SwitchedRef],
) -> rusqlite::Result<Vec<SwitchedView>> {
    let conn = conn.unchecked_transaction()?;
    let files: Vec<i64> = switched
        .iter()
        .flat_map(|s| [s.from_file_id, s.to_file_id])
        .collect();
    let file_names = file_names(&conn, volumes, &files)?;
    let track_names = names(&conn, volumes, switched.iter().map(|s| s.recording_id))?;
    let name = |file: i64| file_names.get(&file).cloned().unwrap_or_default();
    Ok(switched
        .iter()
        .map(|s| SwitchedView {
            recording_id: s.recording_id,
            track: name_of(&track_names, s.recording_id),
            from_name: name(s.from_file_id),
            to_name: name(s.to_file_id),
        })
        .collect())
}

/// The names of what "Accept all" switched, for its "View".
#[tauri::command]
#[specta::specta]
pub async fn review_switched_view(
    reads: State<'_, ReadPool>,
    switched: Vec<SwitchedRef>,
) -> Result<Vec<SwitchedView>, IpcError> {
    let volumes = system_volumes();
    Ok(reads.read(|conn| switched_view(conn, &volumes, &switched))?)
}
