//! Tracks removed from the Library that rekordbox may still hold (ROADMAP
//! 1.9 rule 7).

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;

use crate::library::removed_tracks;
use crate::rekordbox::location::{decode, Location};
use crate::rekordbox::source::LAST_READ;
use crate::send_values::{shown_removed, Removed};

/// A removed track to remove in rekordbox by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManualRemoval {
    pub recording_id: i64,
    /// What rekordbox shows for it if it has a row at the sent `Location`,
    /// else the file's tags, else the file's name ([`shown_removed`]).
    pub title: Option<String>,
    pub artist: Option<String>,
    /// Where it was sent, as Windows writes it, so the user can find it in
    /// rekordbox; for a track that was never sent, where rekordbox's own
    /// entry for it is. `None` if the sent `Location` can't be read back: the row
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
    // For a track that was never sent: rekordbox's own entry for it, found
    // through a trusted match to one of the track's files (the lowest
    // TrackID if it holds several). A probable match doesn't count: it
    // isn't known to be this track.
    let mut own_entry = conn.prepare(
        "SELECT rt.location FROM rekordbox_track rt
         JOIN recording_file rf ON rf.file_id = rt.file_id
         WHERE rf.recording_id = ?1 AND rt.relink_probable = 0
         ORDER BY rt.track_id, rt.id LIMIT 1",
    )?;
    let mut out = Vec::new();
    // What to name each row by, asked of [`shown_removed`] for all at once.
    let mut asked = Vec::new();
    for removed in removed_tracks(conn)? {
        if removed.last_sent_location.is_none() {
            // Never sent, so only the latest read can say rekordbox has
            // it: listed while that read holds an entry for it, at that
            // entry's own Location.
            let entry: Option<String> = own_entry
                .query_row([removed.recording_id], |r| r.get(0))
                .optional()?;
            let Some(entry) = entry else {
                continue;
            };
            let decoded = decode(&entry).ok();
            let path = match &decoded {
                Some(Location::File(path)) => Some(
                    path.to_windows()
                        .unwrap_or_else(|| path.as_str().to_owned()),
                ),
                _ => None,
            };
            asked.push(Removed {
                recording_id: removed.recording_id,
                location_key: decoded.as_ref().map(Location::match_key),
                path_name: path
                    .as_deref()
                    .map(|p| crate::send_values::file_name(p).to_owned()),
            });
            out.push(ManualRemoval {
                recording_id: removed.recording_id,
                title: None,
                artist: None,
                path,
                removed_at: removed.removed_at,
            });
            continue;
        }
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
        let shown_path = path.map(|p| p.to_windows().unwrap_or_else(|| p.as_str().to_owned()));
        asked.push(Removed {
            recording_id: removed.recording_id,
            location_key: path.map(|p| p.match_key()),
            path_name: shown_path
                .as_deref()
                .map(|p| crate::send_values::file_name(p).to_owned()),
        });
        out.push(ManualRemoval {
            recording_id: removed.recording_id,
            title: None,
            artist: None,
            path: shown_path,
            removed_at: removed.removed_at,
        });
    }
    // The names, for all of them at once.
    for (removal, shown) in out.iter_mut().zip(shown_removed(conn, &asked)?) {
        (removal.title, removal.artist) = shown.into_options();
    }
    Ok(out)
}
