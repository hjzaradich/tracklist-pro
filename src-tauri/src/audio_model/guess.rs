//! The model's guesses at a track's genre and mood (ROADMAP 3.8, 5A-A3),
//! for the "Audio model" panel. Review only: nothing here writes anything,
//! and nothing is applied to the track.
//!
//! - **Probability.** The stored scores are logits; each goes through a
//!   sigmoid, `1 / (1 + e^-x)`, and is shown as a whole percent. A score that
//!   isn't a finite number is skipped.
//! - **Which are shown.** The 50 genre labels: the best at most
//!   [`MOST_GENRES`] whose probability is at least [`THRESHOLD`], best first.
//!   The 7 mood labels: the single best, if it is at least [`THRESHOLD`].
//!   Neither may pass, and then the lists are empty (the screen says so).
//! - **Never loads the model.** It reads the stored scores of the track's
//!   shown file ([`panel`](super::panel)).

use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use tauri::State;

use super::panel::{label_key, standing, PanelState, Standing};
use super::subset::SUBSET;
use crate::db::ReadPool;
use crate::ipc::IpcError;

/// The least probability a guess may have to be shown: 30 %. AudioSet's
/// genre labels are rarely scored high even on a clear example, so the
/// usual 50 % would show almost nothing.
pub const THRESHOLD: f64 = 0.30;

/// The most genre guesses shown.
pub const MOST_GENRES: usize = 3;

/// The first [`SUBSET`] label that is a mood, not a genre.
const FIRST_MOOD: usize = 50;

/// One guess: a label and how sure the model is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Guess {
    /// The label's locale key (see [`label_key`]).
    pub key: String,
    /// 0 to 100.
    pub percent: u8,
}

/// The model's guesses for a track, or why there are none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Guesses {
    pub state: PanelState,
    /// Best first, at most [`MOST_GENRES`].
    pub genres: Vec<Guess>,
    /// The best mood, if one passes.
    pub mood: Option<Guess>,
}

impl Guesses {
    fn none(state: PanelState) -> Guesses {
        Guesses {
            state,
            genres: Vec::new(),
            mood: None,
        }
    }
}

fn sigmoid(logit: f32) -> f64 {
    1.0 / (1.0 + (-f64::from(logit)).exp())
}

/// The guesses in `scores` (57 logits, in [`SUBSET`]'s order).
pub(crate) fn guesses_in(scores: &[f32]) -> (Vec<Guess>, Option<Guess>) {
    let mut passing: Vec<(usize, f64)> = scores
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_finite())
        .map(|(i, &s)| (i, sigmoid(s)))
        .filter(|&(_, p)| p >= THRESHOLD)
        .collect();
    // Best first; the label list's order settles a tie.
    passing.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let guess = |&(i, p): &(usize, f64)| Guess {
        key: label_key(SUBSET[i].1),
        percent: (p * 100.0).round().clamp(0.0, 100.0) as u8,
    };
    let genres = passing
        .iter()
        .filter(|(i, _)| *i < FIRST_MOOD)
        .take(MOST_GENRES)
        .map(guess)
        .collect();
    let mood = passing.iter().find(|(i, _)| *i >= FIRST_MOOD).map(guess);
    (genres, mood)
}

/// The guesses for track `recording`.
pub(crate) fn guesses_for(conn: &Connection, recording: i64) -> rusqlite::Result<Guesses> {
    Ok(match standing(conn, recording)? {
        Standing::Not(state) => Guesses::none(state),
        Standing::Heard(heard) => match heard.scores {
            None => Guesses::none(PanelState::CouldntBeRead),
            Some(scores) => {
                let (genres, mood) = guesses_in(&scores);
                Guesses {
                    state: PanelState::Ready,
                    genres,
                    mood,
                }
            }
        },
    })
}

/// The audio model's genre and mood guesses for a track, from what it
/// stored. Reads only; never loads the model.
#[tauri::command]
#[specta::specta]
pub async fn audio_model_guesses(
    reads: State<'_, ReadPool>,
    recording_id: i64,
) -> Result<Guesses, IpcError> {
    Ok(reads.read(move |c| guesses_for(c, recording_id))?)
}
