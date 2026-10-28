//! The three-way comparison per field, and the conflicts it finds (ROADMAP
//! 1.10; 1cA-1 and 1cA-2).
//!
//! A field of a Library track has three sides: the app's value, rekordbox's
//! value now (its newest export), and the base (what was last sent,
//! `sync_base`). [`decide`] says what happened to the field since the base.
//! It is a pure decision library: no database, no job, no files, no text a
//! user reads. A verdict and its reason are codes; their wording arrives
//! with the screen that shows them. [`store`] reads the three sides and
//! keeps the conflicts in the `conflict` table; [`resolve`] settles them.
//!
//! # The decision table
//!
//! The first row that matches decides. `=` is the comparison of the field's
//! kind (below), never plain text.
//!
//! | # | Case | [`Verdict`] | [`Reason`] |
//! |---|---|---|---|
//! | 1 | an analysis field (`AverageBpm`, `Tonality`, `TEMPO`, `POSITION_MARK`), or `TrackID` | `Never` | `analysisField`, `notAField` |
//! | 2 | rekordbox's entry is a row kept from an older read | `Wait` | `staleRead` |
//! | 3 | rekordbox has no trusted entry for the track | `SendApp` | `notInRekordbox` |
//! | 4 | the app has no value of its own | `TakeRekordbox` | `appFollows` |
//! | | **The field was sent before (a base)** | | |
//! | 5 | app = base, rekordbox = base | `Unchanged` | `neitherChanged` |
//! | 6 | app ≠ base, rekordbox = base | `SendApp` | `onlyAppChanged` |
//! | 7 | app = base, rekordbox ≠ base, rekordbox = replaced | `NotTakenUp` | `sendNotTakenUp` |
//! | 8 | app = base, rekordbox ≠ base, otherwise | `TakeRekordbox` | `onlyRekordboxChanged` |
//! | 9 | both ≠ base, app = rekordbox | `Same` | `bothChangedAlike` |
//! | 10 | both ≠ base, app ≠ rekordbox | `Conflict` | `bothChanged` |
//! | | **The field was never sent (no base)** | | |
//! | 11 | app = rekordbox | `Same` | `neverSentAlike` |
//! | 12 | app ≠ rekordbox, rekordbox = replaced | `SendApp` | `neverSentOnlyAppChanged` |
//! | 13 | app ≠ rekordbox, otherwise | `Conflict` (no base) | `neverSentBothDiffer` |
//!
//! Rows 5, 6, 8 and 10 are ROADMAP 1.10's own three lines; row 9 is what
//! the `conflict` table has always said (both changed, to different
//! values, "otherwise it isn't a conflict"). The others:
//!
//! - **Row 1.** Analysis fields are rekordbox's: the app never edits or
//!   sends them (1.9 rule 3), and the database refuses a conflict on one
//!   (migration 0014). `TrackID` isn't a field: rekordbox reassigns it on
//!   import.
//! - **Row 2.** An incomplete export keeps rows of earlier reads (§5.3).
//!   Such a row's values are old, so nothing is decided from them, and a
//!   conflict already open on the field is left as it is. (A send from
//!   such a read is refused as a whole, 1.9.)
//! - **Row 3.** A track rekordbox doesn't have (or has only by a probable
//!   match) has nothing of rekordbox's to merge: it goes out as new. A
//!   conflict already open on it is left as it is ([`store`]): it may hold
//!   the only copy of rekordbox's value.
//! - **Row 4.** [`Sides::app`] is `None` when the app holds no value of its
//!   own for the field: it sends rekordbox's current value (1.9 rule 1,
//!   "unless the app means to change it"). The app's own values are the
//!   user's edits, kept by [`app_values`].
//! - **Rows 7 and 12** need [`Sides::replaced`]: the value rekordbox showed
//!   that the app's present value replaced. `None` means it isn't known,
//!   and then these rows never match. For a field never sent it's what the
//!   edit replaced ([`app_values`] records it with each edit); for a field
//!   sent before, what that send replaced, as the send recorded it
//!   ([`crate::rekordbox_write::record_send`], forgotten once a read shows
//!   the send taken up). Row 7 is a send that was never
//!   imported (or answered "No" in rekordbox): the base says the app's
//!   value went out, but rekordbox still shows what it showed before.
//!   Without `replaced` that reads as "rekordbox changed it back" (row 8)
//!   and the app's edit would be reverted without a word.
//! - **Row 13.** With no base nothing proves that either side is unchanged,
//!   so neither may overwrite the other silently.
//!
//! One case stays a conflict although `replaced` could settle it: an edit
//! made on top of a send that wasn't taken up (both differ from the base,
//! rekordbox = replaced) is row 10. Asking is the safe answer.
//!
//! # What is done about a verdict
//!
//! One thing comes before the table: a decision the user made that still
//! stands answers its field whatever the verdict ([`store`]), so a value
//! chosen is never overwritten while no side has moved.
//!
//! [`Verdict::handling`] is the one place that says the rest. `NotTakenUp` is
//! handled as "send the app's value again": nothing is taken in and no
//! conflict is recorded. To make it a conflict instead (the owner's call),
//! change [`NOT_TAKEN_UP`] to [`Handling::Review`]; nothing else needs to
//! change ([`store::detect`] then records it without a base, because the
//! base is the app's own value and the table wants three different
//! values).
//!
//! # Comparing two values
//!
//! [`same`] compares by the field's [`Kind`], reusing the reader's own
//! parsing ([`crate::rekordbox`]), so a value rekordbox merely spells
//! differently is never a change:
//!
//! | Kind | Fields | Compared by | No value |
//! |---|---|---|---|
//! | Location | `Location` | the decoded path's match key (`location::decode`, `Location::match_key`): NFC, NTFS letter case; never as text, because a send re-encodes it | absent, empty |
//! | Rating | `Rating` | stars (`Track::stars`): 0, 51, 102, 153, 204, 255 are 0 to 5; another number by its value | absent and empty are 0 stars |
//! | Colour | `Colour` | the color (`Rgb::parse_hex`), whatever the letter case | absent, empty: no color |
//! | Date | `DateAdded`, `DateModified`, `LastPlayed` | the date (`Date::parse`) | absent, empty |
//! | Number | `Size`, `TotalTime`, `DiscNumber`, `TrackNumber`, `Year`, `BitRate`, `SampleRate`, `PlayCount` | the number, as the reader reads it: `007` is 7; for `DiscNumber`, `TrackNumber` and `Year`, 0 is rekordbox's "none"; a `PlayCount` of 0 is a real count | absent, empty |
//! | Text | everything else, attributes the app doesn't know included | the text in NFC, exactly: letter case and every space count as a change (owner default, 2026-10-12) | absent, empty, all blank |
//!
//! A value that doesn't parse as its kind is compared as exact text, so two
//! unreadable values are only equal when they're identical. `Comments` is
//! one text field, My Tags block included. Playlist membership isn't a
//! field yet. Its base is recorded per track and sent playlist
//! ([`crate::rekordbox_write::sent_entries`]), and [`decide`] takes any
//! field name, so it can become one.

pub mod app_values;
mod compare;
pub mod merge;
pub mod open_at_send;
pub mod resolve;
pub mod store;

#[cfg(test)]
mod tests;

use serde::Serialize;
use specta::Type;

pub use compare::{fits, kind_of, same, Kind, ANALYSIS_FIELDS};
pub use merge::{merge_before_send, Merged};
pub use resolve::{
    resolve, resolve_all, resolve_all_with, resolve_with, Choice, ResolveError, Resolved,
    RESOLVE_ALL_OPERATION, RESOLVE_OPERATION,
};
pub use store::{
    detect, detect_for, detect_from_store, load_sides, open_conflicts, AppValue, AppValues,
    Conflict, ConflictId, Detected, FieldOutcome, FieldSides, Filter, Outcome, RekordboxEntry,
    Status, TrackSides,
};

/// rekordbox's side of one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rekordbox<'a> {
    /// rekordbox has no trusted entry for the track in its newest read.
    NotThere,
    /// Its entry is a row kept from an older, incomplete read.
    Stale,
    /// Its entry's value in the newest read; `None` when the entry has no
    /// such attribute.
    Value(Option<&'a str>),
}

/// The sides of one field of one Library track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sides<'a> {
    /// The app's own value. `None`: the app has none of its own and sends
    /// rekordbox's. `Some("")` is the app wanting the field empty.
    pub app: Option<&'a str>,
    pub rekordbox: Rekordbox<'a>,
    /// The value last sent (`sync_base`). `None`: never sent.
    pub base: Option<&'a str>,
    /// The value on rekordbox's side that the app's present value replaced.
    /// `None`: not known.
    pub replaced: Option<&'a str>,
}

/// What happened to a field since its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// Not a field that is merged (row 1).
    Never,
    /// rekordbox's value is from an older read: nothing is decided (row 2).
    Wait,
    /// Neither side changed.
    Unchanged,
    /// Only the app changed, or rekordbox has nothing to merge: the app's
    /// value goes out.
    SendApp,
    /// Only rekordbox changed, or the app has no value of its own:
    /// rekordbox's value comes in.
    TakeRekordbox,
    /// Both sides hold the same value, and it isn't the base's.
    Same,
    /// The app's value was sent, and rekordbox still shows what it showed
    /// before (row 7).
    NotTakenUp,
    /// Both sides changed, to different values.
    Conflict,
}

/// What is done about a [`Verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Handling {
    /// Nothing: the sides agree, or the field isn't merged.
    Nothing,
    /// The app's value stands and is sent.
    SendApp,
    /// rekordbox's value is taken in.
    TakeRekordbox,
    /// The user decides: a conflict is recorded, with every side's value.
    Review,
    /// Nothing is decided and nothing recorded is touched.
    Wait,
}

/// How [`Verdict::NotTakenUp`] is handled: the app's value is sent again.
/// [`Handling::Review`] makes it a conflict instead.
pub const NOT_TAKEN_UP: Handling = Handling::SendApp;

impl Verdict {
    /// What is done about it. The one place that decides.
    pub fn handling(self) -> Handling {
        self.handling_when(NOT_TAKEN_UP)
    }

    /// [`Verdict::handling`] with `NotTakenUp` handled as given, so both
    /// settings of [`NOT_TAKEN_UP`] can be run.
    pub(crate) fn handling_when(self, not_taken_up: Handling) -> Handling {
        match self {
            Verdict::Never | Verdict::Unchanged | Verdict::Same => Handling::Nothing,
            Verdict::Wait => Handling::Wait,
            Verdict::SendApp => Handling::SendApp,
            Verdict::TakeRekordbox => Handling::TakeRekordbox,
            Verdict::NotTakenUp => not_taken_up,
            Verdict::Conflict => Handling::Review,
        }
    }
}

/// Which row of the decision table decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    AnalysisField,
    NotAField,
    StaleRead,
    NotInRekordbox,
    AppFollows,
    NeitherChanged,
    OnlyAppChanged,
    SendNotTakenUp,
    OnlyRekordboxChanged,
    BothChangedAlike,
    BothChanged,
    NeverSentAlike,
    NeverSentOnlyAppChanged,
    NeverSentBothDiffer,
}

/// [`decide`]'s answer for one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub verdict: Verdict,
    pub reason: Reason,
}

/// What happened to `field` since its base, by the decision table in the
/// module docs.
pub fn decide(field: &str, sides: &Sides<'_>) -> Decision {
    use Reason as R;
    use Verdict as V;
    let is = |verdict, reason| Decision { verdict, reason };
    match kind_of(field) {
        Kind::Analysis => return is(V::Never, R::AnalysisField),
        Kind::TrackId => return is(V::Never, R::NotAField),
        _ => {}
    }
    let rekordbox = match sides.rekordbox {
        Rekordbox::Stale => return is(V::Wait, R::StaleRead),
        Rekordbox::NotThere => return is(V::SendApp, R::NotInRekordbox),
        Rekordbox::Value(value) => compare::key(field, value),
    };
    let Some(app) = sides.app else {
        return is(V::TakeRekordbox, R::AppFollows);
    };
    let app = compare::key(field, Some(app));
    // rekordbox still shows the value the app's own replaced.
    let still_replaced = sides
        .replaced
        .is_some_and(|replaced| compare::key(field, Some(replaced)) == rekordbox);
    let Some(base) = sides.base else {
        return if app == rekordbox {
            is(V::Same, R::NeverSentAlike)
        } else if still_replaced {
            is(V::SendApp, R::NeverSentOnlyAppChanged)
        } else {
            is(V::Conflict, R::NeverSentBothDiffer)
        };
    };
    let base = compare::key(field, Some(base));
    match (app == base, rekordbox == base) {
        (true, true) => is(V::Unchanged, R::NeitherChanged),
        (false, true) => is(V::SendApp, R::OnlyAppChanged),
        (true, false) if still_replaced => is(V::NotTakenUp, R::SendNotTakenUp),
        (true, false) => is(V::TakeRekordbox, R::OnlyRekordboxChanged),
        (false, false) if app == rekordbox => is(V::Same, R::BothChangedAlike),
        (false, false) => is(V::Conflict, R::BothChanged),
    }
}
