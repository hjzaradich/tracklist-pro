//! What the track details' "Audio model" panel can say about one track
//! (ROADMAP 3.8, 5A-A3): the shared first step of [`guess`](super::guess)
//! and [`similar`](super::similar).
//!
//! Both are read-only. Neither loads the model: they read what the embed
//! job already stored, for the file the track shows, its current audio and
//! the current model id only. A result of another model or of audio the file
//! no longer has is ignored, as if it were not there.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;

use super::subset::{self, SUBSET_LEN};
use super::EMBEDDING_LEN;
use crate::settings::{read_feature_enabled, Feature};

/// Why the panel has nothing to show, or that it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PanelState {
    /// The switch is off.
    Off,
    /// Only Library tracks are listened to.
    NotInLibrary,
    /// The model hasn't got to this track yet.
    NotListenedYet,
    /// The model failed on this track, or what it stored is unusable.
    CouldntBeRead,
    /// There is something to show (the list may still be empty).
    Ready,
}

/// What is stored for a track's shown file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Heard {
    /// The file the result is for.
    pub file: i64,
    /// The fingerprint, if it is 192 finite numbers.
    pub fingerprint: Option<Vec<f32>>,
    /// The kept label scores (logits), if there are 57.
    pub scores: Option<Vec<f32>>,
}

/// Where a track stands: [`Heard`], or the state that explains why not.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Standing {
    Not(PanelState),
    Heard(Heard),
}

/// The file Library track `recording` shows, as SQL over `lt`.
pub(super) fn shown() -> String {
    crate::search::shown_file_sql("lt.linked_file_id", "lt.recording_id")
}

/// The fingerprint in `bytes`, if it is whole and finite.
pub(super) fn fingerprint_of(bytes: &[u8]) -> Option<Vec<f32>> {
    subset::from_blob(bytes).filter(|v| v.len() == EMBEDDING_LEN && v.iter().all(|x| x.is_finite()))
}

/// Where track `recording` stands. Never looks at any other track.
pub(crate) fn standing(conn: &Connection, recording: i64) -> rusqlite::Result<Standing> {
    if !read_feature_enabled(conn, Feature::AudioModel)? {
        return Ok(Standing::Not(PanelState::Off));
    }
    let file: Option<i64> = conn
        .query_row(
            &format!(
                "SELECT f.id FROM library_track lt JOIN file f ON f.id = {shown}
                 WHERE lt.recording_id = ?1",
                shown = shown()
            ),
            [recording],
            |r| r.get(0),
        )
        .optional()?;
    let Some(file) = file else {
        return Ok(Standing::Not(PanelState::NotInLibrary));
    };
    let model = subset::model_id();
    let stored: Option<(Vec<u8>, Vec<u8>)> = conn
        .query_row(
            "SELECT e.fingerprint, e.label_scores
             FROM embedding e JOIN file f ON f.id = e.file_id AND f.audio_hash = e.audio_hash
             WHERE e.file_id = ?1 AND e.model_id = ?2",
            (file, &model),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((fingerprint, scores)) = stored {
        return Ok(Standing::Heard(Heard {
            file,
            fingerprint: fingerprint_of(&fingerprint),
            scores: subset::from_blob(&scores).filter(|v| v.len() == SUBSET_LEN),
        }));
    }
    let failed: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM embedding_failure x
                        JOIN file f ON f.id = x.file_id AND f.audio_hash = x.audio_hash
                        WHERE x.file_id = ?1 AND x.model_id = ?2)",
        (file, &model),
        |r| r.get(0),
    )?;
    Ok(Standing::Not(if failed {
        PanelState::CouldntBeRead
    } else {
        PanelState::NotListenedYet
    }))
}

/// The locale key of a kept label: its name in camelCase, e.g. "Hip hop
/// music" is `hipHopMusic`. The screen has its own wording per key.
pub fn label_key(name: &str) -> String {
    let mut key = String::new();
    for (i, word) in name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .enumerate()
    {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if i == 0 {
                key.extend(first.to_lowercase());
            } else {
                key.extend(first.to_uppercase());
            }
            key.push_str(&chars.as_str().to_lowercase());
        }
    }
    key
}
