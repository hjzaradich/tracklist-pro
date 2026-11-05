//! How far an add to a crate has got, for the screen to show (1cE-P2).
//!
//! An add of tracks from outside the Library is ONE operation inside one
//! writer job (one Undo), so nothing in it can report through the job queue:
//! the queue writes progress through the same writer and would wait behind
//! the add. This is an in-memory sink in Tauri state instead. The add's
//! loops publish into it; [`crate_add_progress`] reads it without the writer,
//! so it answers while the add holds the writer.

use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::ipc::IpcError;

/// How many tracks go by between two publishes. Cheap enough to publish
/// often, rare enough to cost nothing next to a row of the add.
pub(super) const EVERY: usize = 256;

/// What an add is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AddPhase {
    /// No add has begun, or one has begun and its work hasn't started
    /// (it may be waiting behind another write).
    Idle,
    /// Judging each track the way "Add to Library" would, before any write.
    Checking,
    /// Writing: linking tracks into the Library and putting them in the crate.
    Adding,
    /// The last add is over. `done` and `total` are where it got to.
    Done,
}

/// A reading of the sink: `done` of `total` tracks, in `phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddProgressState {
    pub phase: AddPhase,
    pub done: u32,
    pub total: u32,
}

const IDLE: AddProgressState = AddProgressState {
    phase: AddPhase::Idle,
    done: 0,
    total: 0,
};

impl Default for AddProgressState {
    fn default() -> Self {
        IDLE
    }
}

#[derive(Default)]
struct Inner {
    state: Mutex<AddProgressState>,
    /// A test's way to stop the add at a publish, to look from outside.
    #[cfg(test)]
    on_publish: Mutex<Option<Arc<dyn Fn(AddProgressState) + Send + Sync>>>,
}

/// The sink. Cloning shares it.
#[derive(Clone, Default)]
pub struct AddProgress(Arc<Inner>);

/// Held by the command for the length of an add; when it ends (by any road)
/// the sink says [`AddPhase::Done`].
pub struct Tracking(AddProgress);

impl Drop for Tracking {
    fn drop(&mut self) {
        self.0.set(|s| s.phase = AddPhase::Done);
    }
}

impl AddProgress {
    fn lock(&self) -> MutexGuard<'_, AddProgressState> {
        self.0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set(&self, change: impl FnOnce(&mut AddProgressState)) {
        let now = {
            let mut state = self.lock();
            change(&mut state);
            *state
        };
        #[cfg(test)]
        {
            let hook = self
                .0
                .on_publish
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some(hook) = hook {
                hook(now);
            }
        }
        #[cfg(not(test))]
        let _ = now;
    }

    /// A new add begins: forget the last one. Call before the add is handed
    /// to the writer, so a wait behind another write doesn't show the last
    /// add's numbers.
    pub fn begin(&self) -> Tracking {
        self.set(|s| *s = IDLE);
        Tracking(self.clone())
    }

    /// A phase begins, over `total` tracks.
    pub(super) fn phase(&self, phase: AddPhase, total: usize) {
        self.set(|s| {
            *s = AddProgressState {
                phase,
                done: 0,
                total: total.try_into().unwrap_or(u32::MAX),
            }
        });
    }

    /// `done` tracks of the phase are behind it. Publishes every [`EVERY`]
    /// tracks (and at the first).
    pub(super) fn at(&self, done: usize) {
        if done % EVERY == 0 {
            self.set(|s| s.done = done.try_into().unwrap_or(u32::MAX));
        }
    }

    /// The phase is over: all of it is done.
    pub(super) fn all_done(&self) {
        self.set(|s| s.done = s.total);
    }

    /// The reading now.
    pub fn read(&self) -> AddProgressState {
        *self.lock()
    }

    #[cfg(test)]
    pub(super) fn on_publish(&self, hook: impl Fn(AddProgressState) + Send + Sync + 'static) {
        *self
            .0
            .on_publish
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(hook));
    }
}

/// How far the add under way has got. Never waits for the writer.
#[tauri::command]
#[specta::specta]
pub async fn crate_add_progress(
    progress: State<'_, AddProgress>,
) -> Result<AddProgressState, IpcError> {
    Ok(progress.read())
}
