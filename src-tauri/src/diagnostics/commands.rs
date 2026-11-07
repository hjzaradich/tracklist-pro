//! The diagnostics commands the frontend can call. Registered in `ipc.rs`.
//!
//! Nothing here sends, uploads or opens anything: the screen reads the
//! log, previews the report, and saves what it previewed into the app data
//! folder.

use serde::Serialize;
use specta::Type;
use tauri::State;

use super::report::{self, ReportDraft};
use super::{now_ms, EventLog, LogEvent};
use crate::db::ReadPool;
use crate::ipc::{ErrorKind, IpcError};
use crate::write_guard::WriteGuard;

/// The log, newest first, for the Diagnostics screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EventLogView {
    pub events: Vec<LogEvent>,
    /// Events that couldn't be written to the log's file this run. They
    /// are still in this list until the app closes.
    pub lost_writes: u32,
}

/// Where a saved report is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedReport {
    pub path: String,
}

/// The event log, newest first.
#[tauri::command(async)]
#[specta::specta]
pub fn diagnostic_events(log: State<'_, EventLog>) -> EventLogView {
    view(&log)
}

pub(super) fn view(log: &EventLog) -> EventLogView {
    let mut events = log.events();
    events.reverse();
    EventLogView {
        events,
        lost_writes: log.lost_writes(),
    }
}

/// Builds the report and keeps it for [`save_diagnostics_report`]. Returns
/// the exact text that will be saved.
#[tauri::command(async)]
#[specta::specta]
pub fn preview_diagnostics_report(
    reads: State<'_, ReadPool>,
    log: State<'_, EventLog>,
    draft: State<'_, ReportDraft>,
) -> Result<String, IpcError> {
    preview(&reads, &log, &draft)
}

pub(super) fn preview(
    reads: &ReadPool,
    log: &EventLog,
    draft: &ReportDraft,
) -> Result<String, IpcError> {
    let text = report::build(reads, log, now_ms())?;
    draft.set(text.clone());
    Ok(text)
}

/// Saves the previewed report into the app data folder, through the write
/// guard, and returns where it is. Nothing else is done with it.
#[tauri::command(async)]
#[specta::specta]
pub fn save_diagnostics_report(
    guard: State<'_, WriteGuard>,
    draft: State<'_, ReportDraft>,
) -> Result<SavedReport, IpcError> {
    save(&guard, &draft)
}

pub(super) fn save(guard: &WriteGuard, draft: &ReportDraft) -> Result<SavedReport, IpcError> {
    let text = draft
        .take()
        .ok_or_else(|| IpcError::new(ErrorKind::Internal))?;
    let path = match report::save(guard, &text, now_ms()) {
        Ok(path) => path,
        Err(_) => {
            // Still the report on screen: saving can be tried again.
            draft.set(text);
            return Err(IpcError::new(ErrorKind::CannotWrite));
        }
    };
    Ok(SavedReport {
        path: path.to_string_lossy().into_owned(),
    })
}
