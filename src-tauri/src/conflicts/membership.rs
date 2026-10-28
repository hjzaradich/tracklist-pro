//! Crate membership, merged before every send (ROADMAP 1.10, 1.9 rule 6;
//! 1cB-3): which tracks each crate's rekordbox playlist holds after a send,
//! and whether a playlist edit made in rekordbox changes a crate.
//!
//! # Three sides per crate and track
//!
//! Each pair (a crate's playlist, a Library track) is one "field" of the
//! track, decided by the same table as every other field ([`super::decide`],
//! [`super::store`]), with two values: [`IN`] and [`OUT`].
//!
//! - **The app's:** whether the crate holds the track (`crate_entry`).
//! - **rekordbox's:** whether rekordbox's playlist at the crate's path, in
//!   the newest read, holds the track's own entry (the trusted entry the
//!   send sends the track as, [`crate::send_values`]).
//! - **The base:** whether the last send that wrote that playlist put the
//!   track in it (`sent_entry`, [`crate::rekordbox_write::sent_entries`]).
//!   While a conflict on the pair is open, its base is the conflict's own:
//!   a send doesn't move it ([`super::open_at_send`]). For a playlist last
//!   written before migration 0036 nothing was recorded, so it has no base.
//! - **What a send replaced** (table row 7): the pairs whose membership the
//!   send that last wrote the playlist changed in rekordbox
//!   (`sent_playlist.replaced`), until a read shows that send taken up
//!   ([`crate::rekordbox_write::forget_taken_up`]). A send never imported
//!   then reads as a send not taken up, never as "the DJ changed it back".
//!
//! A playlist is found by its path from `Crates` down, names compared as
//! the writer compares siblings ([`crate::rekordbox_write::sibling_key`]).
//! A conflict on a pair has the field `playlist:` followed by that key path
//! as JSON ([`field_of`]); [`crate_of`] finds its crate.
//!
//! # Which playlists are compared
//!
//! Only one the app itself sent (1.9 rule 6: a `sent_playlist` row) and
//! that a crate of the app still has at that path. So:
//!
//! - **A playlist the DJ made**, even inside `Crates`, is never compared:
//!   the crate of that name (if any) goes out as the app has it.
//! - **A crate renamed, moved or deleted in the app** has no base at its
//!   new path, so it goes out as the app has it; the old playlist is listed
//!   to delete by hand (rule 6). Changes the DJ made to the old playlist are
//!   not taken in.
//! - **A playlist the DJ renamed, moved or deleted in rekordbox** isn't in
//!   the read at its path: nothing is taken in, and the crate goes out as
//!   the app has it (a deleted playlist comes back with the next import).
//!   The same holds for a playlist sent but not imported yet.
//! - **An export saved before the last send that wrote the playlist** can't
//!   show that send, so it decides nothing about it, like a row kept from
//!   an older read (§5.3): the crate goes out as the app has it.
//!
//! An open conflict on a playlist that is no longer compared (its crate or
//! its record is gone) is withdrawn: there's nothing left to merge it into.
//!
//! # What the send does about a pair
//!
//! | Outcome | The send writes | And |
//! |---|---|---|
//! | only the app changed, a send not taken up, the track or the playlist not in rekordbox | the app's membership | |
//! | only rekordbox changed (the DJ added or removed the track there) | rekordbox's | the crate takes it in: its entry is added or deleted |
//! | both changed (only possible without a base) | rekordbox's ([`super::open_at_send`]) | the conflict stays open; its base doesn't move |
//! | a settled conflict that stands | the membership chosen | |
//! | a row kept from an older read | the app's (such a send is refused anyway) | |
//!
//! Two sides of two values can't both change differently when there is a
//! base: a pair is a conflict only on a playlist with no base, or with
//! [`super::NOT_TAKEN_UP`] set to review.
//!
//! **Taking a change in is outside the operation log**, like taking a
//! field's rekordbox value in ([`super::merge`]): it is derived from the
//! three sides, and an Undo must never bring back what rekordbox has
//! overtaken (the next send would take it in again anyway). `crate_entry`
//! ids are never reused (migration 0039), so an Undo of a crate operation
//! whose entry was taken in since is refused as "changed since", and
//! nothing else is undone wrongly. Settling a membership conflict changes
//! the crate in the resolve's own operation ([`adopt`]).
//!
//! **Tracks outside the Library:** an entry of rekordbox's playlist that no
//! Library track is sent as (a track not in the Library, a streaming entry)
//! can't be taken into a crate, and the import replaces the playlist, so
//! the review lists the crate as losing those entries
//! ([`Memberships::outside_library`]) and the send needs the confirm.
//!
//! The order of a playlist's tracks isn't compared: a track taken in goes
//! last, and the send writes the crate's order.

use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::{params, Connection};

use super::merge::Merged;
use super::resolve::Resolved;
use super::store::{detect_in, FieldSides, Outcome, RekordboxEntry, Status, TrackSides};
use super::NOT_TAKEN_UP;
use crate::crates::CrateId;
use crate::library::LibraryTrackId;
use crate::ops::{OpsError, Recorder};
use crate::rekordbox_write::{self, sibling_key, CRATES_FOLDER};
use crate::send_values::{Outcome as Values, Provenance, SendValues};

/// What a membership field's name starts with.
pub const FIELD_PREFIX: &str = "playlist:";
/// The track is in the playlist.
pub const IN: &str = "in";
/// The track isn't in the playlist.
pub const OUT: &str = "out";

/// A playlist's path from `Crates` down, each name as the writer compares
/// siblings.
pub type KeyPath = Vec<String>;

/// The field of a pair, for the playlist at `key`.
pub fn field_of(key: &[String]) -> String {
    format!(
        "{FIELD_PREFIX}{}",
        serde_json::to_string(key).expect("strings always serialize")
    )
}

/// Whether `field` is a playlist membership, not a `TRACK` attribute.
pub fn is_membership(field: &str) -> bool {
    field.starts_with(FIELD_PREFIX)
}

fn key_of(field: &str) -> Option<KeyPath> {
    serde_json::from_str(field.strip_prefix(FIELD_PREFIX)?).ok()
}

fn value(member: bool) -> String {
    (if member { IN } else { OUT }).to_owned()
}

/// What the merge decided about crate membership, for the send to apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Memberships {
    /// Per playlist, tracks the send writes though the crate doesn't hold
    /// them (an open conflict sends rekordbox's membership, a decision the
    /// one chosen): written after the crate's own.
    pub added: HashMap<KeyPath, Vec<LibraryTrackId>>,
    /// Per playlist, tracks the crate holds that the send leaves out.
    pub dropped: HashMap<KeyPath, HashSet<LibraryTrackId>>,
    /// Per playlist rekordbox's newest read has under `Crates` (and that an
    /// older export doesn't stand for), the Library tracks it holds: what a
    /// send replaces there (`sent_playlist.replaced`).
    pub rekordbox: HashMap<KeyPath, HashSet<LibraryTrackId>>,
    /// Per compared playlist, its entries in rekordbox that no Library
    /// track is sent as: the import drops them.
    pub outside_library: HashMap<KeyPath, u32>,
    /// The pairs taken in from rekordbox (the crate changed).
    pub taken_in: Vec<(LibraryTrackId, String)>,
}

/// Every static crate by its playlist's key path (`Crates` first), from
/// `(id, parent, kind, name)` rows.
fn crate_paths(rows: &[(i64, Option<i64>, String, String)]) -> HashMap<KeyPath, i64> {
    let by_id: HashMap<i64, &(i64, Option<i64>, String, String)> =
        rows.iter().map(|row| (row.0, row)).collect();
    let mut paths = HashMap::new();
    for (id, ..) in rows.iter().filter(|row| row.2 == "static") {
        let mut names = Vec::new();
        let mut at = Some(*id);
        while let Some(row) = at.and_then(|id| by_id.get(&id)) {
            names.push(sibling_key(&row.3));
            at = row.1;
            if names.len() > rekordbox_write::MAX_DEPTH {
                break;
            }
        }
        names.push(sibling_key(CRATES_FOLDER));
        names.reverse();
        paths.insert(names, *id);
    }
    paths
}

const CRATE_ROWS: &str = "SELECT id, parent_id, kind, name FROM crate";

/// The crate a membership conflict's field names, if the app still has a
/// crate at that path. For the Conflicts screen, to name it.
pub fn crate_of(conn: &Connection, field: &str) -> rusqlite::Result<Option<CrateId>> {
    let Some(key) = key_of(field) else {
        return Ok(None);
    };
    let rows: Vec<(i64, Option<i64>, String, String)> = conn
        .prepare(CRATE_ROWS)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(crate_paths(&rows).get(&key).copied().map(CrateId))
}

/// A playlist the app sent, as its record says.
struct SentPlaylist {
    /// The last send that wrote it; `None` before migration 0036 (no base).
    send: Option<i64>,
    /// The tracks whose membership that send changed in rekordbox.
    replaced: Option<HashSet<i64>>,
    /// When it was written, in milliseconds since the Unix epoch.
    sent_ms: i64,
}

/// One pair's sides, kept for what the send does about its outcome.
struct Pair {
    crate_id: i64,
    key: KeyPath,
    app: bool,
    /// rekordbox's membership: from the read, or the open conflict's.
    rekordbox: Option<bool>,
}

/// Merges crate membership before a send, inside the merge's transaction
/// (module docs). `values` are every Library track's send values, read in
/// that transaction.
pub(super) fn merge(
    tx: &Connection,
    values: &[SendValues],
    merged: &mut Merged,
) -> rusqlite::Result<()> {
    let crates_key = sibling_key(CRATES_FOLDER);
    let crate_rows: Vec<(i64, Option<i64>, String, String)> = tx
        .prepare(CRATE_ROWS)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let crates = crate_paths(&crate_rows);
    let mut members: HashMap<i64, BTreeSet<i64>> = HashMap::new();
    for row in tx
        .prepare("SELECT crate_id, library_track_id FROM crate_entry WHERE kind = 'member'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
    {
        let (crate_id, track): (i64, i64) = row?;
        members.entry(crate_id).or_default().insert(track);
    }

    let mut sent: HashMap<KeyPath, SentPlaylist> = HashMap::new();
    for row in tx
        .prepare(
            "SELECT path_key, send_id, replaced,
                    CAST(strftime('%s', sent_at) AS INTEGER) * 1000
                        + CAST(substr(sent_at, 21, 3) AS INTEGER)
             FROM sent_playlist WHERE kind = 'playlist'",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?
    {
        let (key, send, replaced, sent_ms) = row?;
        let Ok(key) = serde_json::from_str::<KeyPath>(&key) else {
            continue;
        };
        let replaced = replaced.and_then(|r| serde_json::from_str::<HashSet<i64>>(&r).ok());
        sent.insert(
            key,
            SentPlaylist {
                send,
                replaced,
                sent_ms: sent_ms.unwrap_or(i64::MAX),
            },
        );
    }

    // rekordbox's side: each Library track's own entry, and what each
    // playlist of the newest read holds.
    let ids: Vec<LibraryTrackId> = values.iter().map(|v| v.library_track).collect();
    let stale: HashSet<i64> = tx
        .prepare(
            "SELECT track_id FROM rekordbox_track
             WHERE read_at < (SELECT max(read_at) FROM rekordbox_track)",
        )?
        .query_map([], |r| r.get::<_, Option<i64>>(0))?
        .filter_map(Result::transpose)
        .collect::<rusqlite::Result<_>>()?;
    let mut entries: HashMap<i64, RekordboxEntry> = HashMap::new();
    let mut library_of: HashMap<i64, i64> = HashMap::new();
    for track in values {
        let entry = match &track.outcome {
            Values::Ready(v) if v.in_rekordbox => {
                let id = v.values.iter().find_map(|v| match v.provenance {
                    Provenance::Rekordbox { track_id } => Some(track_id),
                    _ => None,
                });
                match id {
                    Some(id) if stale.contains(&id) => RekordboxEntry::Stale,
                    Some(id) => {
                        library_of.insert(id, track.library_track.0);
                        RekordboxEntry::Current
                    }
                    None => RekordboxEntry::NotThere,
                }
            }
            _ => RekordboxEntry::NotThere,
        };
        entries.insert(track.library_track.0, entry);
    }
    let mut in_rekordbox: HashMap<KeyPath, BTreeSet<i64>> = HashMap::new();
    let mut outside: HashMap<KeyPath, u32> = HashMap::new();
    for row in tx
        .prepare(
            "SELECT track_id, playlists FROM rekordbox_track
             WHERE read_at = (SELECT max(read_at) FROM rekordbox_track) AND playlists <> '[]'",
        )?
        .query_map([], |r| {
            Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, String>(1)?))
        })?
    {
        let (track_id, playlists) = row?;
        let track_id = track_id.unwrap_or(-1);
        let Ok(playlists) = serde_json::from_str::<Vec<Vec<String>>>(&playlists) else {
            continue;
        };
        for path in playlists {
            let key: KeyPath = path.iter().map(|n| sibling_key(n)).collect();
            if key.first() != Some(&crates_key) {
                continue;
            }
            match library_of.get(&track_id) {
                Some(&library) => {
                    in_rekordbox.entry(key).or_default().insert(library);
                }
                None => *outside.entry(key).or_default() += 1,
            }
        }
    }
    let tree: HashSet<KeyPath> = crate::after_send::load_tree(tx)?
        .map(|tree| playlist_keys(&tree.crates, &[crates_key.clone()]))
        .unwrap_or_default();
    let export_ms = crate::rekordbox::source::stored_source(tx)?
        .last_read
        .map_or(i64::MIN, |read| read.modified_ms);
    for key in &tree {
        // An export older than the send that wrote it stands for nothing.
        if sent.get(key).is_none_or(|s| export_ms >= s.sent_ms) {
            let held = in_rekordbox.get(key).into_iter().flatten();
            merged
                .membership
                .rekordbox
                .insert(key.clone(), held.map(|&t| LibraryTrackId(t)).collect());
        }
    }

    let mut bases: HashMap<KeyPath, HashSet<i64>> = HashMap::new();
    for entry in rekordbox_write::sent_entries(tx, &ids)? {
        bases.entry(entry.key).or_default().insert(entry.library_track.0);
    }
    // Open membership conflicts: their base, and rekordbox's value.
    let mut open: HashMap<(i64, String), (i64, Option<String>, Option<String>)> = HashMap::new();
    for row in tx
        .prepare(
            "SELECT id, library_track_id, field, rekordbox_value, base_value FROM conflict
             WHERE status = 'open' AND substr(field, 1, length(?1)) = ?1",
        )?
        .query_map([FIELD_PREFIX], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
    {
        let (id, track, field, rekordbox, base): (i64, i64, String, _, _) = row?;
        open.insert((track, field), (id, rekordbox, base));
    }

    // The pairs of every playlist compared.
    let mut sides = Vec::new();
    let mut pairs: HashMap<(i64, String), Pair> = HashMap::new();
    let mut compared: HashSet<String> = HashSet::new();
    let mut keys: Vec<&KeyPath> = crates.keys().filter(|k| sent.contains_key(*k)).collect();
    keys.sort();
    for key in keys {
        let crate_id = crates[key];
        let record = &sent[key];
        let field = field_of(key);
        compared.insert(field.clone());
        let held = merged.membership.rekordbox.contains_key(key);
        let rekordbox = in_rekordbox.get(key);
        let base = bases.get(key);
        let app = members.get(&crate_id);
        if held {
            if let Some(&n) = outside.get(key) {
                merged.membership.outside_library.insert(key.clone(), n);
            }
        }
        let mut tracks: BTreeSet<i64> = BTreeSet::new();
        tracks.extend(app.into_iter().flatten());
        tracks.extend(base.into_iter().flatten());
        tracks.extend(rekordbox.into_iter().flatten());
        tracks.extend(open.keys().filter(|(_, f)| *f == field).map(|(t, _)| *t));
        for track in tracks {
            let Some(&own) = entries.get(&track) else {
                continue;
            };
            let entry = match (tree.contains(key), held) {
                // Not in the read at this path: nothing of rekordbox's.
                (false, _) => RekordboxEntry::NotThere,
                // An export older than the send: it decides nothing.
                (true, false) => RekordboxEntry::Stale,
                (true, true) => own,
            };
            let app_in = app.is_some_and(|a| a.contains(&track));
            let rekordbox_in = rekordbox.is_some_and(|r| r.contains(&track));
            let open_here = open.get(&(track, field.clone()));
            let (base, replaced) = match open_here {
                // The conflict's base: a send doesn't move it.
                Some((_, _, base)) => (base.clone(), None),
                None if record.send.is_some() => {
                    let was = base.is_some_and(|b| b.contains(&track));
                    let changed = record.replaced.as_ref().is_some_and(|r| r.contains(&track));
                    (Some(value(was)), changed.then(|| value(!was)))
                }
                None => (None, None),
            };
            let shown = if entry == RekordboxEntry::Current {
                Some(rekordbox_in)
            } else {
                open_here.and_then(|(_, r, _)| r.as_deref().map(|r| r == IN))
            };
            pairs.insert(
                (track, field.clone()),
                Pair {
                    crate_id,
                    key: key.clone(),
                    app: app_in,
                    rekordbox: shown,
                },
            );
            sides.push(TrackSides {
                library_track: LibraryTrackId(track),
                entry,
                fields: vec![FieldSides {
                    field: field.clone(),
                    app: Some(value(app_in)),
                    rekordbox: Some(value(rekordbox_in)),
                    base,
                    replaced,
                }],
            });
        }
    }
    // An open conflict on a playlist no longer compared: withdrawn.
    for ((_, field), (id, _, _)) in &open {
        if !compared.contains(field) {
            tx.execute(
                "DELETE FROM conflict WHERE id = ?1 AND status = 'open'",
                [id],
            )?;
        }
    }

    let detected = detect_in(tx, &sides, NOT_TAKEN_UP)?;
    let mut take_in = tx.prepare(
        "INSERT INTO crate_entry (crate_id, library_track_id) VALUES (?1, ?2)
         ON CONFLICT (crate_id, library_track_id) DO NOTHING",
    )?;
    let mut take_out =
        tx.prepare("DELETE FROM crate_entry WHERE crate_id = ?1 AND library_track_id = ?2")?;
    for outcome in detected.fields {
        let track = outcome.library_track;
        let Some(pair) = pairs.get(&(track.0, outcome.field.clone())) else {
            continue;
        };
        let sent_in = match outcome.outcome {
            Outcome::SendApp | Outcome::Wait => pair.app,
            Outcome::TakeRekordbox => {
                let rekordbox_in = pair.rekordbox.unwrap_or(pair.app);
                if rekordbox_in != pair.app {
                    if rekordbox_in {
                        take_in.execute(params![pair.crate_id, track.0])?;
                    } else {
                        take_out.execute(params![pair.crate_id, track.0])?;
                    }
                    merged.membership.taken_in.push((track, outcome.field));
                }
                // The crate now holds what rekordbox holds.
                continue;
            }
            Outcome::Open { .. } => {
                merged
                    .open
                    .entry(track)
                    .or_default()
                    .push(outcome.field.clone());
                pair.rekordbox.unwrap_or(pair.app)
            }
            Outcome::Decided { value, .. } => value == IN,
        };
        if sent_in && !pair.app {
            merged
                .membership
                .added
                .entry(pair.key.clone())
                .or_default()
                .push(track);
        } else if !sent_in && pair.app {
            merged
                .membership
                .dropped
                .entry(pair.key.clone())
                .or_default()
                .insert(track);
        }
    }
    Ok(())
}

/// The key path of every playlist below `nodes`, under `path`.
fn playlist_keys(nodes: &[crate::after_send::TreeNode], path: &[String]) -> HashSet<KeyPath> {
    let mut out = HashSet::new();
    for node in nodes {
        let mut here = path.to_vec();
        here.push(sibling_key(&node.name));
        match &node.children {
            Some(children) => out.extend(playlist_keys(children, &here)),
            None => {
                out.insert(here);
            }
        }
    }
    out
}

/// What settling a membership conflict makes of its crate, inside the
/// resolve's own operation (so one Undo takes both back): the crate holds
/// the track if the membership chosen is [`IN`], and not if it's [`OUT`].
/// A crate the app no longer has at that path is left alone.
pub(super) fn adopt(rec: &mut Recorder<'_>, resolved: &Resolved) -> Result<(), OpsError> {
    if resolved.status == Status::Open {
        return Ok(());
    }
    let Some(key) = key_of(&resolved.field) else {
        return Ok(());
    };
    let rows: Vec<(i64, Option<i64>, String, String)> = rec.read_rows(CRATE_ROWS, (), |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })?;
    let Some(&crate_id) = crate_paths(&rows).get(&key) else {
        return Ok(());
    };
    let entry: Option<i64> = rec
        .read_rows(
            "SELECT id FROM crate_entry WHERE crate_id = ?1 AND library_track_id = ?2",
            (crate_id, resolved.library_track.0),
            |r| r.get(0),
        )?
        .pop();
    match (resolved.value == IN, entry) {
        (true, None) => {
            rec.insert(
                "crate_entry",
                &[
                    ("crate_id", Value::Integer(crate_id)),
                    ("library_track_id", Value::Integer(resolved.library_track.0)),
                ],
            )?;
        }
        (false, Some(id)) => rec.delete("crate_entry", id)?,
        _ => {}
    }
    Ok(())
}
