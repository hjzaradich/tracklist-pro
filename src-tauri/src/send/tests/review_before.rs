//! The send review as it was before 1cD-S1 made it cheaper: the first
//! Library load, every step on one connection, every label worked out for
//! every track. Kept word for word, for tests only, so `same_as_before` can
//! hold the review the app runs to it, on the same database. Only its
//! results' types are its own, so that the app's can change.

use std::collections::HashMap;

use rusqlite::Connection;

use super::{
    export_older_than_last_send, is_local_drive, left_out_reason, listing, loses_entries,
    modified_at, nothing_is_at, refusal, sent_elsewhere, LIST_FROM,
};
use crate::conflicts::{self, open_at_send};
use crate::library::{self, LibraryTrackId, StoredTrack};
use crate::paths::{StoredPath, Volumes};
use crate::rekordbox::source::stored_source;
use crate::rekordbox_write::{self, Outgoing, SentTrack};
use crate::send::{
    tree, ExportRead, LeftOut, OtherFile, Preflight, Refusal, RefusalReason, TrackLabel,
};

/// A send as it would go, as the review worked it out then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Before {
    pub preflight: Preflight,
    pub outgoing: Option<Outgoing>,
    pub sent_elsewhere: Vec<ElsewhereBefore>,
    pub open: Vec<(LibraryTrackId, String)>,
    pub watched: Vec<WatchedBefore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedBefore {
    pub track: TrackLabel,
    pub path: Option<String>,
    pub recorded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElsewhereBefore {
    pub track: TrackLabel,
    pub path: String,
}

pub fn review(conn: &mut Connection, volumes: &impl Volumes) -> rusqlite::Result<Option<Before>> {
    let Some(read) = stored_source(conn)?.last_read else {
        return Ok(None);
    };
    let tracks = library::stored(conn, volumes)?;
    let ids: Vec<LibraryTrackId> = tracks.iter().map(|t| t.id).collect();
    let merged = conflicts::merge_before_send(conn, volumes, &ids)?;
    let stars_changed = crate::energy_stars::stars_changed_by(conn, &merged)?;
    // The tree after the merge: it may have taken playlist changes made in
    // rekordbox into crates (1.10).
    let mut crates = tree::crate_tree(conn)?;
    tree::as_merged(&mut crates, &merged.membership);
    let membership = merged.membership.clone();
    // Playlists arrive in Phase 3; the folder is still written (rule 6).
    let input = rekordbox_write::gather_merged(conn, merged, crates.clone(), Vec::new())?;
    let mut built = rekordbox_write::build(&input);
    if let Ok(out) = &mut built {
        out.note_replaced(&membership.rekordbox);
    }
    let conn: &Connection = conn;

    let labels: HashMap<LibraryTrackId, &StoredTrack> = tracks.iter().map(|t| (t.id, t)).collect();
    let label = |id: LibraryTrackId| {
        let track = labels.get(&id);
        TrackLabel {
            library_track: id,
            title: track.and_then(|t| t.title.clone()),
            artist: track.and_then(|t| t.artist.clone()),
            file_name: track
                .and_then(|t| t.file.as_ref())
                .map(|f| f.shown(volumes).name),
        }
    };
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);

    let mut preflight = Preflight {
        token: String::new(),
        export: ExportRead {
            path: read.path.clone(),
            modified_ms: read.modified_ms,
            read_at: read.read_at.clone(),
            not_stored: read.summary.not_stored,
        },
        new_tracks: 0,
        known_tracks: 0,
        left_out: Vec::new(),
        file_missing: Vec::new(),
        no_file_at_location: Vec::new(),
        other_file: Vec::new(),
        no_kind: Vec::new(),
        loses_entries: Vec::new(),
        conflicts_waiting: 0,
        stars_changed,
        changed_after_export: Vec::new(),
        refusal: None,
        nothing_to_send: false,
        can_send: false,
        export_older_than_last_send: false,
        needs_confirm: false,
    };
    let mut elsewhere = Vec::new();
    let mut open = Vec::new();
    let outgoing = match built {
        Err(e) => {
            preflight.refusal = Some(refusal(&e));
            None
        }
        Ok(out) => {
            let known = out.sent.iter().filter(|t| t.in_rekordbox).count();
            preflight.known_tracks = count(known);
            preflight.new_tracks = count(out.sent.len() - known);
            preflight.nothing_to_send = out.sent.is_empty() && crates.is_empty();
            preflight.left_out = out
                .left_out
                .iter()
                .map(|left| {
                    let (reason, attribute) = left_out_reason(&left.reason);
                    LeftOut {
                        track: label(left.library_track),
                        reason,
                        attribute,
                    }
                })
                .collect();
            preflight.file_missing = out
                .sent
                .iter()
                .filter(|sent| sent.file_missing)
                .map(|sent| label(sent.library_track))
                .collect();
            for sent in &out.sent {
                let linked = labels
                    .get(&sent.library_track)
                    .and_then(|t| t.file.as_ref())
                    .map(|f| f.shown(volumes).path);
                if let Some(path) = sent_elsewhere(sent, linked.as_deref()) {
                    elsewhere.push(ElsewhereBefore {
                        track: label(sent.library_track),
                        path,
                    });
                }
            }
            for sent in &out.sent {
                let Some(other) = &sent.rekordbox_holds_other_file else {
                    continue;
                };
                let rekordbox_file =
                    library::stored_file(conn, other.file_id)?.map(|f| f.shown(volumes).path);
                preflight.other_file.push(OtherFile {
                    track: label(sent.library_track),
                    library_file: labels
                        .get(&sent.library_track)
                        .and_then(|t| t.file.as_ref())
                        .map(|f| f.shown(volumes).path),
                    rekordbox_file,
                });
            }
            // A new track gets a `Kind` only for a format rekordbox plays
            // (`send_values`); a track rekordbox has is its own entry.
            preflight.no_kind = out
                .sent
                .iter()
                .filter(|sent| {
                    !sent.in_rekordbox && !sent.attributes.iter().any(|(name, _)| name == "Kind")
                })
                .map(|sent| label(sent.library_track))
                .collect();
            preflight.loses_entries = loses_entries(&out, &crates, &membership.outside_library);
            // Fields of the tracks sent that wait in Review → Conflicts:
            // they go with rekordbox's value until the user decides. A crate
            // membership waits whether or not its track is sent (one the
            // send leaves out of a playlist may be in no crate sent).
            let sent: Vec<LibraryTrackId> = out.sent.iter().map(|s| s.library_track).collect();
            open = open_at_send::open_fields(conn, &sent)?;
            preflight.conflicts_waiting = count(open.len());
            Some(out)
        }
    };
    // An incomplete export keeps rows from earlier reads, and a send never
    // uses an old read: this outranks any other refusal.
    if !read.summary.complete {
        preflight.refusal = Some(Refusal {
            reason: RefusalReason::IncompleteExport,
            path: Vec::new(),
        });
    }
    // An export saved before the last send isn't refused, but the user is
    // asked about it before the go.
    preflight.export_older_than_last_send = export_older_than_last_send(conn, read.modified_ms)?;
    let outgoing = outgoing.filter(|_| preflight.refusal.is_none());
    preflight.can_send = preflight.refusal.is_none() && !preflight.nothing_to_send;
    preflight.needs_confirm =
        !preflight.loses_entries.is_empty() || preflight.export.not_stored > 0;
    preflight.token = token(&preflight, outgoing.as_ref());
    // A warning only, after the token: it changes nothing about the go
    // (1.10, "Second channel"). The prepare step looks at the disk too.
    let mut watched = Vec::new();
    if let Some(out) = &outgoing {
        for (id, file, recorded) in changed_after_export(conn, read.modified_ms, &out.sent)? {
            let linked = labels.get(&id).and_then(|t| t.file.as_ref());
            let file = match linked.filter(|f| f.file_id == file) {
                Some(f) => Some(f.clone()),
                None => library::stored_file(conn, file)?,
            };
            let path = file.and_then(|f| {
                StoredPath::new(f.volume, f.rel)
                    .resolve(volumes)
                    .ok()
                    .map(|abs| crate::scan::display_path(&abs))
            });
            watched.push(WatchedBefore {
                track: label(id),
                path,
                recorded,
            });
        }
        preflight.changed_after_export = watched
            .iter()
            .filter(|w| w.recorded)
            .map(|w| w.track.clone())
            .collect();
    }
    // A refused send sends nothing anywhere.
    let sent_elsewhere = match outgoing {
        Some(_) => elsewhere,
        None => Vec::new(),
    };
    Ok(Some(Before {
        preflight,
        outgoing,
        sent_elsewhere,
        open,
        watched,
    }))
}
fn first_ns_after(ms: i64) -> i64 {
    ms.saturating_add(1).saturating_mul(1_000_000)
}
fn changed_after_export(
    conn: &Connection,
    export_modified_ms: i64,
    sent: &[SentTrack],
) -> rusqlite::Result<Vec<(LibraryTrackId, i64, bool)>> {
    let after = first_ns_after(export_modified_ms);
    let files = |sql: &str| -> rusqlite::Result<HashMap<i64, (i64, bool)>> {
        conn.prepare(sql)?
            .query_map([after], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect()
    };
    let entries = files(
        "SELECT r.track_id, f.id, coalesce(f.mtime >= ?1, 0)
         FROM rekordbox_track r JOIN file f ON f.id = r.file_id
         WHERE r.relink_probable = 0",
    )?;
    let linked = files(
        "SELECT l.id, f.id, coalesce(f.mtime >= ?1, 0)
         FROM library_track l JOIN file f ON f.id = l.linked_file_id",
    )?;
    Ok(sent
        .iter()
        .filter(|s| s.in_rekordbox)
        .filter_map(|s| {
            i64::try_from(s.track_id)
                .ok()
                .and_then(|id| entries.get(&id))
                .or_else(|| linked.get(&s.library_track.0))
                .map(|&(file, recorded)| (s.library_track, file, recorded))
        })
        .collect())
}
pub fn no_file_at_location(sent_elsewhere: &[ElsewhereBefore]) -> Vec<TrackLabel> {
    // By drive (`C:\`, letter case folded): whether it's a local drive
    // that's connected.
    let mut local: HashMap<String, bool> = HashMap::new();
    sent_elsewhere
        .iter()
        .filter(|sent| {
            let Some(drive) = sent.path.get(..3) else {
                return false;
            };
            *local
                .entry(drive.to_ascii_uppercase())
                .or_insert_with(|| is_local_drive(drive))
                && nothing_is_at(&sent.path)
        })
        .map(|sent| sent.track.clone())
        .collect()
}
pub fn changed_on_disk(watched: &[WatchedBefore], export_modified_ms: i64) -> Vec<TrackLabel> {
    let after = first_ns_after(export_modified_ms);
    // By drive (a path's first three characters, letter case folded):
    // whether it's a local drive that's connected.
    let mut local: HashMap<String, bool> = HashMap::new();
    // The files to look at, by folder: those the scan didn't flag.
    let mut wanted: HashMap<&str, Vec<&str>> = HashMap::new();
    for w in watched.iter().filter(|w| !w.recorded) {
        let Some(path) = w.path.as_deref() else {
            continue;
        };
        let Some(drive) = path.get(..3) else {
            continue;
        };
        let connected = *local
            .entry(drive.to_ascii_uppercase())
            .or_insert_with(|| is_local_drive(drive));
        if let (true, Some((folder, name))) = (connected, path.rsplit_once('\\')) {
            wanted.entry(folder).or_default().push(name);
        }
    }
    let mut modified: HashMap<(&str, &str), i64> = HashMap::new();
    for (&folder, names) in &wanted {
        let listed = if names.len() >= LIST_FROM {
            listing(folder)
        } else {
            None
        };
        for &name in names {
            let at = match &listed {
                Some(listing) => listing
                    .get(name)
                    .copied()
                    .or_else(|| modified_at(&format!("{folder}\\{name}"))),
                None => modified_at(&format!("{folder}\\{name}")),
            };
            if let Some(at) = at {
                modified.insert((folder, name), at);
            }
        }
    }
    watched
        .iter()
        .filter(|w| {
            w.recorded
                || w.path
                    .as_deref()
                    .and_then(|path| path.rsplit_once('\\'))
                    .and_then(|key| modified.get(&key))
                    .is_some_and(|&at| at >= after)
        })
        .map(|w| w.track.clone())
        .collect()
}
fn token(preflight: &Preflight, outgoing: Option<&Outgoing>) -> String {
    let mut hasher = blake3::Hasher::new();
    let described = serde_json::to_vec(preflight).expect("a preflight always serializes");
    hasher.update(&(described.len() as u64).to_le_bytes());
    hasher.update(&described);
    if let Some(out) = outgoing {
        hasher.update(out.xml());
    }
    hasher.finalize().to_hex().to_string()
}
