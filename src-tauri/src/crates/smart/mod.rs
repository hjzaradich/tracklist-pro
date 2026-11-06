//! Smart crates (ROADMAP 3.3): crates whose tracks come from a rule, plus
//! an always-include and a never-include list, sent to rekordbox as plain
//! playlists that each send refreshes.
//!
//! - **The rule** ([`SmartRule`]) is a list of conditions combined with
//!   "all" or "any", each a field from a fixed list ([`Condition`]) with an
//!   operator from a fixed list. It is stored as JSON in `crate.rules` (no
//!   migration: the column and the `smart` kind are there since 0005).
//! - **It compiles to SQL** on the track table's index (`track_row`) in
//!   [`compile`]: the SQL text comes only from fixed strings, and every value
//!   the user chose is a bound parameter.
//! - **What a smart crate holds** is (rule ∪ always) − never, Library tracks
//!   only ([`membership`]). The table lists it, its count is live, and a send
//!   evaluates it and writes it like a hand-made crate.
//! - **Always and never include** are their own undoable operations.
//!   Dropping tracks on a smart crate always-includes them; removing one
//!   never-includes it. A track is on one list at most (`crate_entry` holds a
//!   track once per crate), so moving it from one list to the other changes
//!   its entry's kind.
//! - **A rule that doesn't read** (an older or newer build's) matches no
//!   track; the always-include list still goes out. One bad crate never
//!   stops a page or a send.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::{
    all_in_library, make_in, with_operation, worded, Changed, CrateId, CrateKind, CratesError, Done,
};
use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::library::LibraryTrackId;
use crate::ops::{self, OpsError};

pub mod compile;
pub mod membership;
#[cfg(test)]
mod tests;

/// The operation kinds, as the operation log stores them.
pub const CREATE_SMART_OPERATION: &str = "create_smart_crate";
pub const EDIT_RULE_OPERATION: &str = "edit_smart_crate";
pub const ALWAYS_OPERATION: &str = "always_include";
pub const NEVER_OPERATION: &str = "never_include";

/// The most conditions a rule holds: the builder offers no more, and saving
/// more is refused, so a rule's SQL stays small.
pub const MAX_CONDITIONS: usize = 32;
/// The longest text a condition holds (a genre or a key), in characters.
pub const MAX_TEXT_CHARS: usize = 200;

/// A smart crate's rule: its conditions, all of them or any of them. No
/// condition matches no track (the crate holds its always-include list only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SmartRule {
    pub combine: Combine,
    pub conditions: Vec<Condition>,
}

/// How a rule's conditions combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Combine {
    All,
    Any,
}

/// Comparing a number. `Between` takes `value` and `to`, either way round;
/// with no `to` it is `Is`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum NumberOp {
    Is,
    IsNot,
    AtLeast,
    AtMost,
    Between,
}

/// Comparing a choice (a key, a crate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ChoiceOp {
    Is,
    IsNot,
}

/// Comparing text (a genre), letter case and outer spaces ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TextOp {
    Is,
    IsNot,
    Contains,
}

/// One condition of a rule. A track with no value for the field (no energy,
/// no BPM, no key, no genre, no rating) matches no condition on it, "is
/// not" included.
///
/// To add a field (the tag job's "tag includes", after 3.1 merges): a
/// variant here, its SQL in [`compile::condition`], and its dropdown in
/// `src/crates/smart/`. Nothing else changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "field", rename_all = "camelCase")]
pub enum Condition {
    /// The energy the track shows, 1 to 10.
    Energy {
        op: NumberOp,
        value: u8,
        to: Option<u8>,
    },
    /// The BPM, in whole beats as rounded. With `halfDouble`, a track at
    /// half or double the BPM matches too.
    Bpm {
        op: NumberOp,
        value: u16,
        to: Option<u16>,
        #[serde(rename = "halfDouble", default)]
        half_double: bool,
    },
    /// The key, in Camelot notation (`8A`). Any other text matches nothing.
    Key { op: ChoiceOp, value: String },
    /// The genre as the track shows it.
    Genre { op: TextOp, value: String },
    /// The rating in stars, 0 to 5.
    Rating {
        op: NumberOp,
        value: u8,
        to: Option<u8>,
    },
    /// In a hand-made crate (by id).
    InCrate {
        op: ChoiceOp,
        #[serde(rename = "crateId")]
        crate_id: CrateId,
    },
}

impl SmartRule {
    /// The rule as `crate.rules` stores it.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a rule always serializes")
    }

    /// A stored rule, or `None` if it doesn't read.
    pub fn from_json(text: &str) -> Option<SmartRule> {
        serde_json::from_str(text).ok()
    }

    /// Whether the rule fits the limits ([`MAX_CONDITIONS`], [`MAX_TEXT_CHARS`]).
    fn fits(&self) -> bool {
        self.conditions.len() <= MAX_CONDITIONS
            && self.conditions.iter().all(|c| match c {
                Condition::Key { value, .. } | Condition::Genre { value, .. } => {
                    value.chars().count() <= MAX_TEXT_CHARS
                }
                _ => true,
            })
    }
}

/// Which list an always or never include puts tracks on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inclusion {
    Always,
    Never,
}

impl Inclusion {
    /// The `crate_entry.kind` of the list.
    pub(crate) fn entry_kind(self) -> &'static str {
        match self {
            Inclusion::Always => "always_include",
            Inclusion::Never => "never_include",
        }
    }

    fn operation(self) -> &'static str {
        match self {
            Inclusion::Always => ALWAYS_OPERATION,
            Inclusion::Never => NEVER_OPERATION,
        }
    }
}

/// A smart crate's name, or `None` if `id` isn't a smart crate.
fn smart_name(rec: &ops::Recorder<'_>, id: CrateId) -> Result<Option<String>, OpsError> {
    Ok(rec
        .read_rows(
            "SELECT name FROM crate WHERE id = ?1 AND kind = 'smart'",
            [id.0],
            |r| r.get(0),
        )?
        .pop())
}

/// Makes a smart crate in `parent` (`None`: the top), last among its
/// siblings. Recorded as one operation.
pub fn create_smart_on(
    conn: &mut Connection,
    parent: Option<CrateId>,
    name: &str,
    rule: &SmartRule,
) -> Result<Done<CrateId>, OpsError> {
    if !rule.fits() {
        return Ok(Err(CratesError::RuleTooLong));
    }
    let name = name.trim().to_owned();
    let rules = rule.to_json();
    ops::record(
        conn,
        CREATE_SMART_OPERATION,
        &serde_json::json!({ "name": name, "parentId": parent.map(|p| p.0) }),
        |rec| make_in(rec, CrateKind::Smart, parent, &name, Some(rules)),
    )
    .map(|recorded| worded(recorded.value, Some(CrateKind::Smart)))
}

/// Gives a smart crate a new rule. The rule it has already records nothing
/// (`None`).
pub fn set_rule_on(
    conn: &mut Connection,
    id: CrateId,
    rule: &SmartRule,
) -> Result<Done<Option<i64>>, OpsError> {
    if !rule.fits() {
        return Ok(Err(CratesError::RuleTooLong));
    }
    let rules = rule.to_json();
    ops::record(
        conn,
        EDIT_RULE_OPERATION,
        &serde_json::json!({ "crateId": id.0 }),
        |rec| {
            let Some(name) = smart_name(rec, id)? else {
                return Ok(Err(CratesError::NotFound));
            };
            rec.detail("name", name);
            rec.set("crate", id.0, "rules", rules)?;
            Ok(Ok(()))
        },
    )
    .map(|recorded| recorded.value.map(|()| recorded.operation_id))
}

/// Puts Library tracks on a smart crate's always-include or never-include
/// list. A track on the other list moves; one on this list already is
/// skipped. Recorded as one operation; nothing changed, nothing recorded.
pub fn include_on(
    conn: &mut Connection,
    id: CrateId,
    tracks: &[LibraryTrackId],
    inclusion: Inclusion,
) -> Result<Done<Changed>, OpsError> {
    let kind = inclusion.entry_kind();
    ops::record(
        conn,
        inclusion.operation(),
        &serde_json::json!({ "crateId": id.0 }),
        |rec| {
            let Some(name) = smart_name(rec, id)? else {
                return Ok(Err(CratesError::NotFound));
            };
            if !all_in_library(rec, tracks)? {
                return Ok(Err(CratesError::TrackNotFound));
            }
            let mut entries: std::collections::HashMap<i64, (i64, String)> = rec
                .read_rows(
                    "SELECT library_track_id, id, kind FROM crate_entry WHERE crate_id = ?1",
                    [id.0],
                    |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))),
                )?
                .into_iter()
                .collect();
            let mut changed = 0;
            for track in tracks {
                match entries.get(&track.0) {
                    Some((_, there)) if there == kind => {}
                    Some((entry, _)) => {
                        let entry = *entry;
                        rec.set("crate_entry", entry, "kind", kind)?;
                        entries.insert(track.0, (entry, kind.to_owned()));
                        changed += 1;
                    }
                    None => {
                        let entry = rec.insert(
                            "crate_entry",
                            &[
                                ("crate_id", id.0.into()),
                                ("library_track_id", track.0.into()),
                                ("kind", kind.to_owned().into()),
                            ],
                        )?;
                        entries.insert(track.0, (entry, kind.to_owned()));
                        changed += 1;
                    }
                }
            }
            rec.detail("name", name);
            rec.detail("tracks", changed);
            Ok(Ok(Changed {
                changed,
                skipped: tracks.len() as u32 - changed,
                operation_id: None,
            }))
        },
    )
    .map(with_operation)
}

/// Makes a smart crate, in `parent` or at the top. The name is trimmed.
#[tauri::command]
#[specta::specta]
pub async fn create_smart_crate(
    writer: State<'_, Writer>,
    name: String,
    parent: Option<CrateId>,
    rule: SmartRule,
) -> Result<CrateId, IpcError> {
    Ok(writer.call(move |conn| Ok(create_smart_on(conn, parent, &name, &rule)))???)
}

/// Gives a smart crate a new rule. Answers the operation recorded, or none
/// when the rule is the one it has.
#[tauri::command]
#[specta::specta]
pub async fn set_smart_crate_rule(
    writer: State<'_, Writer>,
    id: CrateId,
    rule: SmartRule,
) -> Result<Option<i64>, IpcError> {
    Ok(writer.call(move |conn| Ok(set_rule_on(conn, id, &rule)))???)
}

/// A smart crate's rule; none when the stored rule doesn't read (it then
/// matches no track).
#[tauri::command]
#[specta::specta]
pub async fn smart_crate_rule(
    reads: State<'_, ReadPool>,
    id: CrateId,
) -> Result<Option<SmartRule>, IpcError> {
    reads
        .read(|conn| membership::stored_rule(conn, id))?
        .ok_or_else(|| CratesError::NotFound.into())
}
