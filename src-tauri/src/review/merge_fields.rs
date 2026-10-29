//! Merging two Library tracks asks field by field (1cB-20; ROADMAP 1.12,
//! "Same track twice in your Library"; owner decision, 2026-10-28).
//!
//! A merge keeps one track and folds the others into it ([`super::same`]).
//! What the user chose for the tracks, their edits and rekordbox's rating,
//! color and comments, must not be lost to that choice, so for each field
//! the tracks' values are compared by the field's kind:
//!
//! - **A value is the user's** when it is the app's own value of the field
//!   (`app_value`, §2: an edit in the app, a conflict settled), or when it
//!   is rekordbox's `Rating`, `Colour` or `Comments` (My Tags included) of
//!   the entry the track is sent as ([`KEPT_BY_REKORDBOX`]). Anything else a
//!   track shows (a tag read from its file, rekordbox's genre or year) is
//!   not, and never makes a field a question by itself.
//! - **Ask:** the tracks hold different values for the field, at least one
//!   is the user's. One row per such field ([`Plan::fields`]); the staying
//!   track's value is chosen by default (the first track that has a value,
//!   if the staying one has none, which only happens with three tracks or
//!   more).
//! - **Carry over:** the staying track has no value, and the others agree on
//!   one that is the user's: it carries over without asking.
//! - Otherwise nothing: the staying track keeps what it has.
//!
//! A value taken from another track becomes the staying track's own value
//! (`app_value`, the same store the conflicts engine reads, 1.10), written
//! inside the merge's own operation ([`super::same::merge`]), so one Undo
//! takes the merge and every value back exactly. The next send then carries
//! it by the rules of 1.10: the value rekordbox shows for the staying track
//! is what it replaced.
//!
//! "No value" is blank text, no stars, no color, a 0 where rekordbox writes 0
//! for none ([`crate::conflicts::is_blank`]); so is a field the app holds an
//! empty value for. Values the staying track couldn't be given (not a value
//! of the field's kind, or a character XML can't carry) are not offered, from
//! the tracks that leave.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::actions::Refused;
use crate::conflict_review::ValueKind;
use crate::conflicts::app_values::{self, TRACK_ATTRIBUTES};
use crate::conflicts::{fits, is_blank, load_sides, same, AppValues, RekordboxEntry};
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
use crate::send_values::{send_values, Outcome};

/// The fields where rekordbox's own value is the user's: they set them in
/// rekordbox, and the app has no other copy (owner decision, 2026-10-28).
pub const KEPT_BY_REKORDBOX: [&str; 3] = ["Rating", "Colour", "Comments"];

/// One track's value of a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FieldValue {
    pub recording_id: i64,
    /// `None`: the track has no value for the field.
    pub value: Option<String>,
}

/// A field the merge asks about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MergeField {
    /// The rekordbox XML attribute name, e.g. `Rating`.
    pub field: String,
    pub kind: ValueKind,
    /// Every track of the item, the one that stays first.
    pub values: Vec<FieldValue>,
    /// The track whose value is chosen by default.
    pub chosen: i64,
}

/// The value the user chose for one field: the track it comes from, and the
/// text the screen showed (the merge is refused if it isn't the track's
/// value any more).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MergeChoice {
    pub field: String,
    pub from: i64,
    pub value: String,
}

/// A Library track of the item, as the plan needs it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Member {
    pub recording: i64,
    pub library_track: LibraryTrackId,
}

/// A value to write into the staying track's own values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Write {
    pub field: String,
    pub value: String,
    /// What rekordbox shows for the field on the staying track (`None`: no
    /// trusted entry in the newest read).
    pub rekordbox: Option<String>,
}

struct Held {
    value: String,
    user: bool,
}

enum What {
    /// Asked: the member (by index) chosen by default.
    Ask(usize),
    /// Not asked: the member whose value carries over.
    Carry(usize),
}

struct Planned {
    field: String,
    held: Vec<Option<Held>>,
    what: What,
    rekordbox: Option<String>,
}

/// What a merge does about each field, for the tracks of one item.
pub(super) struct Plan {
    /// The item's tracks' recordings, the one that stays first.
    recordings: Vec<i64>,
    fields: Vec<Planned>,
}

/// Works out the plan for `members`, the track that stays first. Reads only.
pub(super) fn plan(
    conn: &rusqlite::Connection,
    volumes: &impl Volumes,
    members: &[Member],
) -> rusqlite::Result<Plan> {
    let ids: Vec<LibraryTrackId> = members.iter().map(|m| m.library_track).collect();
    let sent = send_values(conn, volumes, &ids)?;
    let own = app_values::read(conn, &ids)?;
    let sides = load_sides(conn, &sent, &AppValues::default())?;
    // What rekordbox shows for the staying track, as `app_values::set` reads
    // it: only a current entry counts.
    let staying = sides
        .iter()
        .find(|t| t.library_track == ids[0] && t.entry == RekordboxEntry::Current);

    let mut candidates: BTreeSet<&str> = KEPT_BY_REKORDBOX.into_iter().collect();
    for id in &ids {
        for value in own.of(*id) {
            if app_values::editable(&value.field)
                && TRACK_ATTRIBUTES.contains(&value.field.as_str())
            {
                candidates.insert(&value.field);
            }
        }
    }

    let held = |at: usize, field: &str| -> Option<Held> {
        let (value, user) = match own.of(ids[at]).iter().find(|v| v.field == field) {
            Some(own) => (own.value.as_str(), true),
            None => match &sent[at].outcome {
                Outcome::Ready(values) => {
                    let found = values.get(field)?;
                    (
                        found.value.as_str(),
                        values.in_rekordbox && KEPT_BY_REKORDBOX.contains(&field),
                    )
                }
                Outcome::CannotSend(_) => return None,
            },
        };
        if is_blank(field, value) {
            return None;
        }
        // The staying track's own value is shown whatever it is; one that
        // would move to it must be a value it can hold.
        let movable = fits(field, value) && crate::rekordbox_write::uncarriable(value).is_none();
        (at == 0 || movable).then(|| Held {
            value: value.to_owned(),
            user,
        })
    };

    let mut fields = Vec::new();
    for field in candidates {
        let helds: Vec<Option<Held>> = (0..ids.len()).map(|at| held(at, field)).collect();
        let present: Vec<usize> = (0..ids.len()).filter(|&at| helds[at].is_some()).collect();
        let Some(&first) = present.first() else {
            continue;
        };
        if !present
            .iter()
            .any(|&at| helds[at].as_ref().is_some_and(|h| h.user))
        {
            continue;
        }
        let value_of = |at: usize| helds[at].as_ref().map(|h| h.value.as_str());
        let alike = present
            .iter()
            .all(|&at| same(field, value_of(at), value_of(first)));
        let what = match (alike, helds[0].is_some()) {
            (false, true) => What::Ask(0),
            (false, false) => What::Ask(first),
            (true, false) => What::Carry(first),
            (true, true) => continue,
        };
        fields.push(Planned {
            field: field.to_owned(),
            held: helds,
            what,
            rekordbox: staying.map(|t| {
                t.fields
                    .iter()
                    .find(|f| f.field == field)
                    .and_then(|f| f.rekordbox.clone())
                    .unwrap_or_default()
            }),
        });
    }
    Ok(Plan {
        recordings: members.iter().map(|m| m.recording).collect(),
        fields,
    })
}

impl Plan {
    /// The fields to ask about, by the name rekordbox gives them.
    pub fn fields(&self) -> Vec<MergeField> {
        self.fields
            .iter()
            .filter_map(|planned| {
                let What::Ask(chosen) = planned.what else {
                    return None;
                };
                Some(MergeField {
                    field: planned.field.clone(),
                    kind: ValueKind::of(&planned.field),
                    values: self
                        .recordings
                        .iter()
                        .zip(&planned.held)
                        .map(|(&recording_id, held)| FieldValue {
                            recording_id,
                            value: held.as_ref().map(|h| h.value.clone()),
                        })
                        .collect(),
                    chosen: self.recordings[chosen],
                })
            })
            .collect()
    }

    /// The values to give the staying track, `choices` applied over the
    /// defaults. [`Refused::ItemGone`] when a choice names a field the merge
    /// doesn't ask about, a track that isn't in the item, or a value the
    /// track doesn't hold any more: the screen showed something else.
    pub fn writes(&self, choices: &[MergeChoice]) -> Result<Vec<Write>, Refused> {
        let mut picked: Vec<Option<usize>> = vec![None; self.fields.len()];
        for choice in choices {
            let at = self
                .fields
                .iter()
                .position(|p| p.field == choice.field && matches!(p.what, What::Ask(_)))
                .ok_or(Refused::ItemGone)?;
            let from = self
                .recordings
                .iter()
                .position(|&r| r == choice.from)
                .ok_or(Refused::ItemGone)?;
            let holds = self.fields[at].held[from]
                .as_ref()
                .is_some_and(|h| h.value == choice.value);
            if !holds {
                return Err(Refused::ItemGone);
            }
            picked[at] = Some(from);
        }
        let mut writes = Vec::new();
        for (planned, picked) in self.fields.iter().zip(picked) {
            let from = picked.unwrap_or(match planned.what {
                What::Ask(at) | What::Carry(at) => at,
            });
            if from == 0 {
                continue;
            }
            let Some(held) = &planned.held[from] else {
                return Err(Refused::ItemGone);
            };
            writes.push(Write {
                field: planned.field.clone(),
                value: held.value.clone(),
                rekordbox: planned.rekordbox.clone(),
            });
        }
        Ok(writes)
    }
}
