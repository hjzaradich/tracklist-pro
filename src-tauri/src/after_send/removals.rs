//! Tracks removed from the Library that rekordbox may still hold (ROADMAP
//! 1.9 rule 7).

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;

use crate::library::remove_in_rekordbox;
use crate::rekordbox::location::{decode, Location};
use crate::rekordbox::source::LAST_READ;

/// A removed track to remove in rekordbox by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManualRemoval {
    pub recording_id: i64,
    pub title: Option<String>,
    pub artist: Option<String>,
    /// Where it was sent, as Windows writes it, so the user can find it in
    /// rekordbox. `None` if the sent `Location` can't be read back: the row
    /// then shows only the track's name, and leaves the list once a read made
    /// after the removal has no row matched to the track.
    pub path: Option<String>,
    /// When the user removed it, UTC ISO-8601.
    pub removed_at: String,
}

/// The removed tracks rekordbox still holds, in the order they were
/// removed. See the module notes of [`super`] for when one leaves.
pub fn manual_removals(conn: &Connection) -> rusqlite::Result<Vec<ManualRemoval>> {
    let last_read: Option<String> = conn
        .query_row(
            "SELECT json_extract(value, '$.readAt') FROM setting WHERE key = ?1",
            [LAST_READ],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let mut held = conn.prepare("SELECT 1 FROM rekordbox_track WHERE location_key = ?1 LIMIT 1")?;
    // For a sent Location that can't be read back: the read's rows matched to
    // the removed track (a trusted or probable match to one of its files).
    let mut held_by_track =
        conn.prepare("SELECT 1 FROM rekordbox_track WHERE recording_id = ?1 LIMIT 1")?;
    let mut name = conn.prepare("SELECT title, artist FROM recording WHERE id = ?1")?;
    let mut out = Vec::new();
    for removed in remove_in_rekordbox(conn)? {
        let location = removed
            .last_sent_location
            .as_deref()
            .and_then(|raw| decode(raw).ok());
        let path = match &location {
            Some(Location::File(path)) => Some(path),
            _ => None,
        };
        // Only a read made after the removal can say it's gone, and only if
        // it doesn't hold the track.
        let read_since = last_read
            .as_deref()
            .is_some_and(|read| read >= removed.removed_at.as_str());
        if read_since {
            let held = match path {
                Some(path) => held.exists([path.match_key()])?,
                None => held_by_track.exists([removed.recording_id])?,
            };
            if !held {
                continue;
            }
        }
        let (title, artist) = name
            .query_row([removed.recording_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .unwrap_or((None, None));
        out.push(ManualRemoval {
            recording_id: removed.recording_id,
            title,
            artist,
            path: path.map(|p| p.to_windows().unwrap_or_else(|| p.as_str().to_owned())),
            removed_at: removed.removed_at,
        });
    }
    Ok(out)
}
