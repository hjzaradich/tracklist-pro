//! The conflict engine's tests. Every name here is invented.
//!
//! - `table`: one test per row of the decision table (module docs), the
//!   order of the rows, and what is done about each verdict.
//! - `compare`: the comparison of each kind of field, and what "no value"
//!   is for each.

use super::*;

mod compare;
mod table;

/// A field sent before: the app's value, rekordbox's, and the base. What
/// the app's value replaced isn't known.
pub(super) fn sides<'a>(app: &'a str, rekordbox: &'a str, base: &'a str) -> Sides<'a> {
    Sides {
        app: Some(app),
        rekordbox: Rekordbox::Value(Some(rekordbox)),
        base: Some(base),
        replaced: None,
    }
}

/// A field that was never sent.
pub(super) fn never_sent<'a>(app: &'a str, rekordbox: &'a str) -> Sides<'a> {
    Sides {
        app: Some(app),
        rekordbox: Rekordbox::Value(Some(rekordbox)),
        base: None,
        replaced: None,
    }
}

/// The same sides, with the value the app's own replaced known.
pub(super) fn replacing<'a>(sides: Sides<'a>, replaced: &'a str) -> Sides<'a> {
    Sides {
        replaced: Some(replaced),
        ..sides
    }
}
