//! The `rekordbox_track` snapshot: rekordbox's collection as of the last
//! read (1aB-10, ROADMAP 1.2, §2).
//!
//! [`SnapshotRows::from_xml`] turns a parsed export into rows, off the
//! database writer; [`replace_snapshot`] swaps them in, in one transaction,
//! so a read that fails midway leaves the previous snapshot whole.
//!
//! - **Every attribute as read.** A track's `attributes` is a JSON object of
//!   strings holding every `TRACK` attribute in file order, unknown ones
//!   included, because the XML writer fills every attribute it sends from
//!   this snapshot (ROADMAP 1.9 rule 1). `tempo` and `position_marks` keep
//!   each `TEMPO` and `POSITION_MARK` the same way.
//! - **Matched by Location.** `location_key` is the parser's match key
//!   ([`super::Location::match_key`]): decoded by hand, NFC, and for
//!   Windows paths, letter case folded the way NTFS folds it.
//! - **Streaming entries are stored**, with the stream's id as their key,
//!   so they can be listed as "streaming, no file" (ROADMAP 1.2). They're
//!   never matched to a file.
//! - **An incomplete export** ([`super::RekordboxXml::is_complete`] false)
//!   may be missing tracks, so tracks from earlier reads that it doesn't
//!   hold are kept, with their older `read_at`. A complete export replaces
//!   the whole snapshot.
//! - **Nothing is matched here.** `file_id` stays NULL; relinking is 1aC.
//! - A track that can't be stored (no usable `TrackID`, one already taken,
//!   or a `Location` that doesn't decode) is counted, never guessed at.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, Transaction};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{Attrs, EntryTarget, RekordboxXml};

/// One `rekordbox_track` row, ready to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRow {
    pub track_id: i64,
    /// Every `TRACK` attribute, as a JSON object in file order.
    pub attributes: String,
    pub location_key: String,
    /// The `TEMPO` entries, a JSON array of attribute objects.
    pub tempo: String,
    /// The `POSITION_MARK` entries, a JSON array of attribute objects.
    pub position_marks: String,
    /// The My Tag names found at the end of `Comments`, a JSON array
    /// ([`super::my_tags`]).
    pub my_tags: String,
    /// The playlists holding the track, a JSON array of paths, each an array
    /// of names from below ROOT down to the playlist.
    pub playlists: String,
    pub streaming: bool,
}

/// A parsed export as snapshot rows, and what couldn't be stored.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotRows {
    pub rows: Vec<SnapshotRow>,
    /// Whether the export held every track it said it did.
    pub complete: bool,
    /// Tracks left out: no usable `TrackID`, a `TrackID` an earlier track in
    /// the file already has, or a `Location` that doesn't decode.
    pub not_stored: usize,
    /// The Location keys and `TrackID`s of tracks the export holds but that
    /// aren't stored: the ones left out above, and the ones the parser
    /// skipped (deleted, demo tracks, samples). They're still in rekordbox's
    /// read, so an incomplete export replaces earlier rows for them too.
    pub unstored_keys: Vec<String>,
    pub unstored_ids: Vec<i64>,
}

/// What a read did to the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummary {
    /// Tracks stored from this read, streaming ones included.
    pub tracks: u32,
    /// Of those, streaming entries (no file).
    pub streaming: u32,
    /// Tracks kept from earlier reads because this export is incomplete.
    pub kept: u32,
    /// Tracks in the export that couldn't be stored.
    pub not_stored: u32,
    /// Whether the export held every track it said it did.
    pub complete: bool,
}

impl SnapshotRows {
    /// The rows for every track `xml` holds, in file order.
    pub fn from_xml(xml: &RekordboxXml) -> SnapshotRows {
        let playlists = playlist_paths(xml);
        let mut rows = Vec::with_capacity(xml.tracks.len());
        let mut ids = HashSet::with_capacity(xml.tracks.len());
        let mut not_stored = 0;
        let mut unstored_keys = Vec::new();
        let mut unstored_ids = Vec::new();
        let mut unstored = |id: Option<u64>, key: Option<String>| {
            unstored_ids.extend(id.and_then(|id| i64::try_from(id).ok()));
            unstored_keys.extend(key);
        };
        for track in &xml.skipped {
            let key = super::location::decode(&track.location)
                .ok()
                .map(|l| l.match_key());
            unstored(track.track_id, key);
        }
        for (i, track) in xml.tracks.iter().enumerate() {
            let key = || track.location.as_ref().ok().map(|l| l.match_key());
            let (Some(track_id), Ok(location)) =
                (stored_id(track.attrs.get("TrackID")), &track.location)
            else {
                not_stored += 1;
                unstored(track.track_id, key());
                continue;
            };
            if !ids.insert(track_id) {
                not_stored += 1;
                // Its id belongs to the track stored under it.
                unstored(None, key());
                continue;
            }
            rows.push(SnapshotRow {
                track_id,
                attributes: object_json(&track.attrs),
                location_key: location.match_key(),
                tempo: array_json(track.tempos.iter().map(|t| &t.attrs)),
                position_marks: array_json(track.cues.iter().map(|c| &c.attrs)),
                my_tags: super::my_tags::tags_json(track.attrs.get("Comments").unwrap_or("")),
                playlists: playlists.get(&i).map_or_else(
                    || "[]".to_owned(),
                    |paths| serde_json::to_string(paths).expect("strings always serialize"),
                ),
                streaming: track.is_streaming(),
            });
        }
        SnapshotRows {
            rows,
            complete: xml.is_complete(),
            not_stored,
            unstored_keys,
            unstored_ids,
        }
    }
}

/// A `TrackID` the table can store: whole digits, written the way the
/// number is (no leading zeros, since the table checks the attribute reads
/// back as the same number), and within SQLite's integers.
fn stored_id(value: Option<&str>) -> Option<i64> {
    let value = value?;
    let id: i64 = super::attrs::digits(value)?;
    (id.to_string() == value).then_some(id)
}

/// An element's attributes as a JSON object, in file order. A JSON object
/// has no order of its own; this text keeps the file's, and SQLite stores
/// the text as given.
fn object_json(attrs: &Attrs) -> String {
    struct Ordered<'a>(&'a Attrs);
    impl Serialize for Ordered<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut map = s.serialize_map(Some(self.0.len()))?;
            for (name, value) in self.0.iter() {
                map.serialize_entry(name, value)?;
            }
            map.end()
        }
    }
    serde_json::to_string(&Ordered(attrs)).expect("strings always serialize")
}

fn array_json<'a>(elements: impl Iterator<Item = &'a Attrs>) -> String {
    let parts: Vec<String> = elements.map(object_json).collect();
    format!("[{}]", parts.join(","))
}

/// Each track's playlists (by its index in `xml.tracks`), in tree order,
/// each listed once however many times it holds the track.
fn playlist_paths(xml: &RekordboxXml) -> HashMap<usize, Vec<Vec<String>>> {
    let mut paths: HashMap<usize, Vec<Vec<String>>> = HashMap::new();
    for (folders, playlist) in xml.playlists.playlists() {
        let mut path: Vec<String> = folders.iter().map(|&f| f.to_owned()).collect();
        path.push(playlist.name.clone());
        let mut seen = HashSet::new();
        for entry in &playlist.entries {
            if let EntryTarget::Track(i) = entry.target {
                if seen.insert(i) {
                    paths.entry(i).or_default().push(path.clone());
                }
            }
        }
    }
    paths
}

/// Replaces the snapshot with `rows`, in one transaction: on any failure
/// the previous snapshot stays as it was. For an incomplete export, rows
/// from earlier reads whose Location it doesn't hold at all (stored or not)
/// are kept, unless this read gives their `TrackID` to a track (then the
/// kept row goes; the ids are only valid within one read).
///
/// `before_commit` runs inside the transaction after the rows are in, with
/// the rows' `read_at`, so a caller can record the read alongside them.
pub fn replace_snapshot(
    conn: &mut Connection,
    rows: &SnapshotRows,
    before_commit: impl FnOnce(&Transaction<'_>, &SnapshotSummary, &str) -> rusqlite::Result<()>,
) -> rusqlite::Result<SnapshotSummary> {
    let tx = conn.transaction()?;
    let read_at: String =
        tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
            r.get(0)
        })?;
    let kept = if rows.complete {
        tx.execute("DELETE FROM rekordbox_track", [])?;
        0
    } else {
        keep_missing(&tx, rows)?
    };
    {
        let mut insert = tx.prepare(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, tempo, position_marks, my_tags, playlists, read_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for row in &rows.rows {
            insert.execute(params![
                row.attributes,
                row.location_key,
                row.tempo,
                row.position_marks,
                row.my_tags,
                row.playlists,
                read_at,
            ])?;
        }
    }
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let summary = SnapshotSummary {
        tracks: count(rows.rows.len()),
        streaming: count(rows.rows.iter().filter(|r| r.streaming).count()),
        kept: count(kept),
        not_stored: count(rows.not_stored),
        complete: rows.complete,
    };
    before_commit(&tx, &summary, &read_at)?;
    tx.commit()?;
    Ok(summary)
}

/// Deletes every earlier row this read replaces, keeping the ones whose
/// Location it doesn't hold. Returns how many were kept.
fn keep_missing(tx: &Transaction<'_>, rows: &SnapshotRows) -> rusqlite::Result<usize> {
    let keys: HashSet<&str> = rows
        .rows
        .iter()
        .map(|r| r.location_key.as_str())
        .chain(rows.unstored_keys.iter().map(String::as_str))
        .collect();
    let ids: HashSet<i64> = rows
        .rows
        .iter()
        .map(|r| r.track_id)
        .chain(rows.unstored_ids.iter().copied())
        .collect();
    let earlier: Vec<(i64, String, i64)> = tx
        .prepare("SELECT id, location_key, track_id FROM rekordbox_track")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut delete = tx.prepare("DELETE FROM rekordbox_track WHERE id = ?1")?;
    let mut kept = 0;
    for (id, key, track_id) in earlier {
        if keys.contains(key.as_str()) || ids.contains(&track_id) {
            delete.execute([id])?;
        } else {
            kept += 1;
        }
    }
    Ok(kept)
}

#[cfg(test)]
mod tests;
