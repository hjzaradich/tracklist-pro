//! The merge before every send (ROADMAP 1.9 rule 2, 1.10; 1cB-3): read →
//! merge → send in one step.
//!
//! [`merge_before_send`] runs detection on the three sides of every track
//! the send may carry (the app's own values from the store, the base with
//! what it replaced, and the newest trusted read), inside the send's own
//! flow, and says per field what the send carries:
//!
//! | Outcome ([`super::Outcome`]) | The send carries | And |
//! |---|---|---|
//! | only the app changed, a send not taken up, or rekordbox has no trusted entry (`SendApp`) | the app's value | the base moves to it |
//! | only rekordbox changed (`TakeRekordbox`) | rekordbox's value | the app's own value is deleted: the app holds nothing of its own for the field any more |
//! | both changed (`Open`) | what [`super::open_at_send`] says | the base stays |
//! | a settled conflict that stands (`Decided`) | the value chosen | the base moves to it |
//! | a row kept from an older read (`Wait`) | rekordbox's value (such a send is refused anyway) | |
//! | every other field, analysis fields included | rekordbox's value, verbatim (rule 1) | |
//!
//! Crate membership is merged in the same transaction, once every track's
//! fields are ([`membership`]): a playlist change made in rekordbox is taken
//! into the crate, and the send writes each crate's playlist as the merge
//! says ([`Merged::membership`]).
//!
//! Detection opens, updates and withdraws conflicts as it always does
//! ([`super::store`]). Taking rekordbox's value in deletes the `app_value`
//! row **outside the operation log**, like the rest of detection: it's
//! derived from the three sides, and Undo must never bring back a value
//! rekordbox has overtaken. So `app_value` ids are never reused (migration
//! 0035), and an Undo of the edit is refused afterwards as "changed
//! since".
//!
//! The merge runs again at the go, before the send is built again. On
//! sides that haven't moved it writes nothing and chooses the same values,
//! so the file is byte for byte the one reviewed.

use std::collections::HashMap;

use rusqlite::Connection;

use super::membership::{self, Memberships};
use super::store::{detect_in, kept_from_older_read, load_sides_with, Outcome, TRACKS_AT_A_TIME};
use super::{app_values, open_at_send, NOT_TAKEN_UP};
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
use crate::send_values::{send_values, SendValues};

/// What the merge before a send found and chose.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merged {
    /// Every track's send values as [`send_values`] gives them, in the
    /// order asked: rekordbox's own (or the files', for a new track),
    /// before the merge's choices.
    pub values: Vec<SendValues>,
    /// Per track, the fields whose value the merge chose, each with the
    /// value the send carries. A field that isn't here carries its own
    /// send value.
    pub chosen: HashMap<LibraryTrackId, Vec<(String, String)>>,
    /// Per track, the fields with an open conflict, crate memberships
    /// included: they wait in Review → Conflicts ([`super::open_at_send`]).
    pub open: HashMap<LibraryTrackId, Vec<String>>,
    /// The fields whose rekordbox value was taken in: their app value was
    /// deleted.
    pub taken_in: Vec<(LibraryTrackId, String)>,
    /// What the merge of crate membership decided ([`membership`]).
    pub membership: Memberships,
}

/// Merges the three sides of the Library tracks `ids` before a send, by
/// the table in the module docs, in one transaction: all of it or none.
/// A thousand tracks at a time, as [`super::detect_from_store`] does.
pub fn merge_before_send(
    conn: &mut Connection,
    volumes: &impl Volumes,
    ids: &[LibraryTrackId],
) -> rusqlite::Result<Merged> {
    let tx = conn.transaction()?;
    let mut merged = Merged::default();
    // Once for the whole merge: it writes conflicts, app values and crate
    // entries, never `rekordbox_track`, so the answer can't change.
    let stale = kept_from_older_read(&tx)?;
    for some in ids.chunks(TRACKS_AT_A_TIME) {
        let values = send_values(&tx, volumes, some)?;
        let app = app_values::read_for_send(&tx, &values)?;
        let sides = load_sides_with(&tx, &values, &app, &stale)?;
        let detected = detect_in(&tx, &sides, NOT_TAKEN_UP)?;
        for field in detected.fields {
            let track = field.library_track;
            let own = app
                .of(track)
                .iter()
                .find(|own| own.field == field.field)
                .map(|own| own.value.as_str());
            let value = match field.outcome {
                Outcome::SendApp => own.map(str::to_owned),
                Outcome::TakeRekordbox => {
                    if own.is_some() {
                        app_values::take_in(&tx, track, &field.field)?;
                        merged.taken_in.push((track, field.field.clone()));
                    }
                    None
                }
                Outcome::Open { .. } => {
                    merged
                        .open
                        .entry(track)
                        .or_default()
                        .push(field.field.clone());
                    own.and_then(open_at_send::value_sent)
                }
                Outcome::Decided { value, .. } => Some(value),
                Outcome::Wait => None,
            };
            if let Some(value) = value {
                merged
                    .chosen
                    .entry(track)
                    .or_default()
                    .push((field.field, value));
            }
        }
        merged.values.extend(values);
    }
    // Crate membership, once every track's entry is known.
    let values = std::mem::take(&mut merged.values);
    membership::merge(&tx, &values, &mut merged)?;
    merged.values = values;
    tx.commit()?;
    Ok(merged)
}
