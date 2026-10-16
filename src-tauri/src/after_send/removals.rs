//! Tracks removed from the Library that rekordbox may still hold (ROADMAP
//! 1.9 rule 7).

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;

use crate::library::{removed_tracks, LibraryTrackId};
use crate::paths::Volumes;
use crate::rekordbox::location::{decode, Location};
use crate::rekordbox::source::LAST_READ;
use crate::send_values::{sent_as, shown_removed, Removed, SentAs};

/// Why a track is to be removed in rekordbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RemovalReason {
    /// The user removed the track from the Library.
    Removed,
    /// The track is still in the Library, linked to another file since the
    /// user switched it in Review: this is the entry of the file that is
    /// gone, which the track is no longer sent as (`replaced_entry`).
    Replaced,
    /// The Library track it was sent as was merged into another Library
    /// track that is the same recording (Review, 1bE-9): this entry is the
    /// one of the track that left (`replaced_entry`, reason `merge`).
    Merged,
}

/// A track to remove in rekordbox by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManualRemoval {
    pub reason: RemovalReason,
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
    /// When the user removed it (or switched its file), UTC ISO-8601.
    pub removed_at: String,
}

/// The tracks to remove in rekordbox by hand: the removed tracks rekordbox
/// still holds, in the order they were removed, then the entries a switch
/// of a linked file left behind ([`replaced_entries`]). See the module notes
/// of [`super`] for when one leaves.
///
/// `volumes` says where each drive is, for the send's own answer to which
/// entries Library tracks are sent as; no file is opened.
pub fn manual_removals(
    conn: &Connection,
    volumes: &impl Volumes,
) -> rusqlite::Result<Vec<ManualRemoval>> {
    let (mut out, listed) = removed_from_library(conn)?;
    // One rekordbox entry is listed once: an entry a switch left behind
    // that a removed track is already listed at isn't listed again.
    out.extend(
        replaced_entries(conn, volumes)?
            .into_iter()
            .filter(|(_, key)| !listed.contains(key))
            .map(|(removal, _)| removal),
    );
    Ok(out)
}

/// The rekordbox entries a switch of a Library track's linked file or a
/// merge of two Library tracks left behind (`replaced_entry`;
/// [`crate::review::actions`], [`crate::review::same`]), in the order they
/// were replaced: each is listed while the store holds an entry at its
/// `Location` (any row, as for a removed track: an incomplete read can't say
/// an entry is gone) and **no Library track is sent as an entry there**. So
/// an entry is never both sent and listed for removal: once relink trusts
/// the entry on a Library track's linked file, or the user confirms it
/// there, the track is sent as that entry again and the record is moot. It
/// is asked of the send itself ([`sent_as`]), so the two can't disagree.
/// Each comes with its Location key.
fn replaced_entries(
    conn: &Connection,
    volumes: &impl Volumes,
) -> rusqlite::Result<Vec<(ManualRemoval, String)>> {
    struct Record {
        recording: i64,
        location: String,
        key: String,
        at: String,
        merged: bool,
    }
    let records: Vec<Record> = conn
        .prepare(
            "SELECT r.recording_id, r.location, r.location_key, r.replaced_at,
                    r.reason = 'merge'
             FROM replaced_entry r
             WHERE EXISTS (SELECT 1 FROM rekordbox_track t WHERE t.location_key = r.location_key)
             ORDER BY r.id",
        )?
        .query_map([], |r| {
            Ok(Record {
                recording: r.get(0)?,
                location: r.get(1)?,
                key: r.get(2)?,
                at: r.get(3)?,
                merged: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    // Nearly always: nothing was ever replaced.
    if records.is_empty() {
        return Ok(Vec::new());
    }

    // The entries at those Locations.
    let keys = serde_json::to_string(&records.iter().map(|r| &r.key).collect::<Vec<_>>())
        .expect("strings always serialize");
    let mut entries_at: HashMap<String, Vec<i64>> = HashMap::new();
    for row in conn
        .prepare(
            "SELECT location_key, id FROM rekordbox_track
             WHERE location_key IN (SELECT value FROM json_each(?1))",
        )?
        .query_map([&keys], |r| Ok((r.get(0)?, r.get(1)?)))?
    {
        let (key, id): (String, i64) = row?;
        entries_at.entry(key).or_default().push(id);
    }
    // Every Library track that could be sent as one of them: one linked to
    // a file such an entry is matched to, and one whose linked file is gone
    // (it is sent as the entry at that file's own Location).
    let could: Vec<LibraryTrackId> = conn
        .prepare(
            "SELECT lt.id FROM library_track lt
             JOIN file f ON f.id = lt.linked_file_id
             WHERE f.present = 0
                OR f.id IN (SELECT file_id FROM rekordbox_track
                            WHERE location_key IN (SELECT value FROM json_each(?1)))
             ORDER BY lt.id",
        )?
        .query_map([&keys], |r| r.get(0).map(LibraryTrackId))?
        .collect::<rusqlite::Result<_>>()?;
    let asked: Vec<(LibraryTrackId, Option<i64>)> = could.iter().map(|id| (*id, None)).collect();
    let sent: HashSet<i64> = sent_as(conn, volumes, &asked)?
        .into_iter()
        .filter_map(|sent| match sent {
            SentAs::Entry(row) => Some(row),
            SentAs::New | SentAs::Nothing => None,
        })
        .collect();

    let mut out = Vec::new();
    let mut keys = Vec::new();
    let mut asked = Vec::new();
    for record in records {
        let none: &[i64] = &[];
        let rows = entries_at.get(&record.key).map_or(none, Vec::as_slice);
        if rows.iter().any(|row| sent.contains(row)) {
            continue;
        }
        let path = match decode(&record.location).ok() {
            Some(Location::File(path)) => Some(
                path.to_windows()
                    .unwrap_or_else(|| path.as_str().to_owned()),
            ),
            _ => None,
        };
        keys.push(record.key.clone());
        asked.push(Removed {
            recording_id: record.recording,
            location_key: Some(record.key),
            path_name: path
                .as_deref()
                .map(|p| crate::send_values::file_name(p).to_owned()),
        });
        out.push(ManualRemoval {
            reason: if record.merged {
                RemovalReason::Merged
            } else {
                RemovalReason::Replaced
            },
            recording_id: record.recording,
            title: None,
            artist: None,
            path,
            removed_at: record.at,
        });
    }
    // Named by what rekordbox calls the entry: it is the one to look for.
    for (removal, shown) in out.iter_mut().zip(shown_removed(conn, &asked)?) {
        (removal.title, removal.artist) = shown.into_options();
    }
    Ok(out.into_iter().zip(keys).collect())
}

/// The removed tracks rekordbox still holds, in the order they were
/// removed, and the Location keys of the entries they are listed at.
fn removed_from_library(
    conn: &Connection,
) -> rusqlite::Result<(Vec<ManualRemoval>, HashSet<String>)> {
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
                reason: RemovalReason::Removed,
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
            reason: RemovalReason::Removed,
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
    let listed = asked.into_iter().filter_map(|a| a.location_key).collect();
    Ok((out, listed))
}
