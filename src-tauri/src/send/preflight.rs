//! The preflight: a send built in memory and described, no file written.
//! The merge before it (1cB-3) is its only write to the database.

use std::collections::HashMap;

use rusqlite::Connection;

use super::{
    tree, ExportRead, LeftOut, LeftOutReason, LosesEntries, OtherFile, Preflight, Refusal,
    RefusalReason, TrackLabel, TreeKind,
};
use crate::conflicts::{self, open_at_send};
use crate::library::{self, LibraryTrackId, StoredTrack};
use crate::paths::{StoredPath, Volumes};
use crate::rekordbox::location::{decode, Location, PathStyle};
use crate::rekordbox::source::stored_source;
use crate::rekordbox_write::{
    self, sibling_key, BuildError, Node, Outgoing, Reason, SentTrack, CRATES_FOLDER,
    PLAYLISTS_FOLDER,
};
use crate::relink::rules::path_key;
use crate::send_values::CannotSend;
use crate::volume::VolumeKind;

/// A send as it would go now: its description, and the file's contents
/// unless the send is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reviewed {
    pub preflight: Preflight,
    pub outgoing: Option<Outgoing>,
    /// The tracks rekordbox has that go out at another path than their
    /// linked file's, for the prepare step to look up on disk
    /// ([`no_file_at_location`]). Nothing in the preflight or its token
    /// depends on them.
    pub sent_elsewhere: Vec<SentElsewhere>,
    /// The fields of the tracks sent with an open conflict, as the merge
    /// left them ([`open_at_send::open_fields`]). The go records the send
    /// only while they are still these.
    pub open: Vec<(LibraryTrackId, String)>,
    /// The tracks rekordbox has, sent with the export's values, each with
    /// the file a change after the export is looked for in, for the prepare
    /// step to look at on disk ([`changed_on_disk`]). Nothing in the
    /// preflight's token depends on them.
    pub watched: Vec<Watched>,
}

/// A track rekordbox has, sent with the export's values, and the file
/// rekordbox writes its edits into (1.10, "Second channel").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    pub track: TrackLabel,
    /// The file's path as Windows writes it, while its drive is connected;
    /// `None` when it isn't, and the file isn't looked at on disk.
    pub path: Option<String>,
    /// The last scan already recorded a change after the export.
    pub recorded: bool,
}

/// A track rekordbox has, sent with rekordbox's own `Location` although
/// its linked file is at another path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentElsewhere {
    pub track: TrackLabel,
    /// rekordbox's path for it, as Windows writes it: a drive path.
    pub path: String,
}

/// Builds the send from the database as it is now (every Library track,
/// the crate tree, rekordbox's values from the last read, merged with the
/// app's own: read → merge → send in one step, ROADMAP 1.9 rule 2) and
/// describes it. `None` if rekordbox was never read.
///
/// The merge ([`conflicts::merge_before_send`]) is the only write: it
/// opens, updates and withdraws conflicts and takes rekordbox's values in,
/// as detection does. Those writes stay whether or not the user goes:
/// preparing a send is a read and a merge (1.10), so it takes in what
/// rekordbox's newer values overtook. Run again on sides that haven't
/// moved, it writes nothing, so the go builds the very send that was
/// reviewed.
///
/// Call it in one writer job, so everything comes from the same moment.
///
/// [`Preflight::no_file_at_location`] is left empty here: it's about the
/// disk, not about the send, so the prepare step fills it afterwards from
/// [`Reviewed::sent_elsewhere`], outside the writer job, and the token
/// doesn't cover it. A file that comes back between the review and the
/// go changes neither the send nor whether it may be written.
pub fn review(conn: &mut Connection, volumes: &impl Volumes) -> rusqlite::Result<Option<Reviewed>> {
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
                    elsewhere.push(SentElsewhere {
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
            watched.push(Watched {
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
    Ok(Some(Reviewed {
        preflight,
        outgoing,
        sent_elsewhere,
        open,
        watched,
    }))
}

/// rekordbox's path for `sent`, if it's a track rekordbox has that goes
/// out with rekordbox's own `Location` while its linked file (`linked`, as
/// Windows writes its path) is somewhere else: the app paired rekordbox's
/// entry with a file at another path, and a send never changes a known
/// track's `Location`. Whether rekordbox finds a file at its path is then
/// a question for the disk ([`no_file_at_location`]). Reads nothing.
///
/// `None` for every other track, and for these:
/// - a track sent although its linked file is missing
///   ([`SentTrack::file_missing`]). It has its own list in the review
///   ([`Preflight::file_missing`]) and is never in both, whichever path
///   rekordbox has for it;
/// - a `Location` that isn't a Windows drive path (`C:\…`): a network
///   path (`\\server\share\…`) or a macOS path is never looked up.
pub(super) fn sent_elsewhere(sent: &SentTrack, linked: Option<&str>) -> Option<String> {
    if !sent.in_rekordbox || sent.file_missing {
        return None;
    }
    let Ok(Location::File(theirs)) = decode(sent.location()) else {
        return None;
    };
    if theirs.match_key() == path_key(&linked?.replace('\\', "/")) {
        return None;
    }
    if theirs.style() != PathStyle::WindowsDrive {
        return None;
    }
    theirs.to_windows()
}

/// The tracks among `sent_elsewhere` with no file at rekordbox's path:
/// rekordbox will show them as missing, as it did before the send. This
/// asks the disk, once per track and once per drive, so the prepare step
/// calls it after its writer job, never inside one, and never again for
/// the go.
///
/// The disk is asked because rekordbox's file may well be in a folder the
/// app doesn't scan: a file there means rekordbox finds it. A track is
/// listed only when the answer is a plain "nothing is there". It isn't
/// listed when that can't be told:
/// - its drive isn't connected now;
/// - its drive is a network drive, mapped to a letter or not (asking one
///   that doesn't answer can take a long time);
/// - the lookup fails for any other reason than the file not being there.
///
/// Off Windows nothing is ever listed: no drive is connected there
/// (ROADMAP §1.1).
pub fn no_file_at_location(sent_elsewhere: &[SentElsewhere]) -> Vec<TrackLabel> {
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

/// Whether `drive` (`C:\`) is connected now and isn't a network drive.
fn is_local_drive(drive: &str) -> bool {
    let Ok(root) = crate::paths::verbatim_absolute(std::path::Path::new(drive)) else {
        return false;
    };
    crate::volume::volume_for(&root).is_ok_and(|volume| volume.kind != VolumeKind::Network)
}

/// Whether the disk says plainly that nothing is at `path`. Only looks.
fn nothing_is_at(path: &str) -> bool {
    let Ok(path) = crate::paths::verbatim_absolute(std::path::Path::new(path)) else {
        return false;
    };
    matches!(std::fs::metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// The first nanosecond after the millisecond `ms`, as `file.mtime`
/// counts time: a file modified then or later changed after it.
fn first_ns_after(ms: i64) -> i64 {
    ms.saturating_add(1).saturating_mul(1_000_000)
}

/// The tracks among `sent` that rekordbox has, in send order, each with the
/// file a change after the export saved at `export_modified_ms` is looked
/// for in, and whether the last scan recorded one (1.10, "Second
/// channel"). rekordbox writes a DJ's edits into the file (§5.1), so such a
/// track may hold an edit made in rekordbox after the export, which a send
/// of the export's values would overwrite.
///
/// The file is the one rekordbox's entry is matched to (a trusted match
/// only), since that is the file rekordbox writes into; the track's linked
/// file only when the entry has none. They differ when a track is sent as a
/// copy's entry. A track with neither isn't listed, and nor is a new track:
/// its values come from its own file, not from the export.
///
/// The sign recorded here is the file's modified time as the last scan
/// stored it (`file.mtime`), later than the export's millisecond. The app
/// never writes an audio file, so nothing it does moves it. Reads the
/// database only; the prepare step also looks at the disk
/// ([`changed_on_disk`]). Nothing here feeds the send: the file's tags are
/// never read for it.
pub(super) fn changed_after_export(
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

/// The tracks among `watched` whose file changed after the export saved at
/// `export_modified_ms`, in send order: the ones the last scan recorded,
/// and the ones whose file's modified time on disk now is later than the
/// export's millisecond (1.10, "Second channel"). So a DJ who edits in
/// rekordbox after exporting and sends at once is warned, though no scan
/// has seen the change.
///
/// This asks the disk, so the prepare step calls it after its writer job,
/// never inside one, and the go never asks again (the token doesn't cover
/// the answer). A folder holding [`LIST_FROM`] or more of the files is
/// listed once, which gives every file's modified time without opening it
/// (as the scan's walk does); other files are looked up one by one. A file
/// isn't looked at, and the scan's answer stands, when its drive isn't
/// connected now or is a network drive, or the lookup fails. Off Windows no
/// drive is connected, so only the scan's answer counts (ROADMAP §1.1).
pub fn changed_on_disk(watched: &[Watched], export_modified_ms: i64) -> Vec<TrackLabel> {
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

/// How many files of one folder make [`changed_on_disk`] list the folder
/// instead of looking the files up one by one.
pub(super) const LIST_FROM: usize = 8;

/// The modified time of every file in `folder`, by name, in nanoseconds
/// since the Unix epoch, from one listing; `None` if it can't be listed.
fn listing(folder: &str) -> Option<HashMap<String, i64>> {
    let folder = crate::paths::verbatim_absolute(std::path::Path::new(folder)).ok()?;
    let mut found = HashMap::new();
    for entry in std::fs::read_dir(folder).ok()?.flatten() {
        if let Ok(at) = entry.metadata().and_then(|m| m.modified()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            found.insert(name, crate::scan::walk::nanos(at));
        }
    }
    Some(found)
}

/// The modified time of the file at `path`, in nanoseconds since the Unix
/// epoch; `None` when the disk can't tell.
fn modified_at(path: &str) -> Option<i64> {
    let path = crate::paths::verbatim_absolute(std::path::Path::new(path)).ok()?;
    let at = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    Some(crate::scan::walk::nanos(at))
}

/// Whether an export saved at `export_modified_ms` is older than the last
/// recorded send: saved before it, to the millisecond. `false` when it's
/// from the very millisecond of the last send or later, or no send was
/// ever recorded.
///
/// Such an export shows rekordbox as it was before that send. If the send
/// was imported, tracks rekordbox now has would go out as new and the
/// dialogs rekordbox raises wouldn't be the ones the review counted; if it
/// never was, the export is as good as a new one. The app can't tell the
/// two apart, so the user is asked ([`may_send_from_this_export`]).
fn export_older_than_last_send(
    conn: &Connection,
    export_modified_ms: i64,
) -> rusqlite::Result<bool> {
    Ok(last_send_ms(conn)?.is_some_and(|sent| export_modified_ms < sent))
}

/// Whether the go may be taken as far as the export's age goes, given the
/// user's answer to "has a send been imported into rekordbox since this
/// export was saved?" (`None`: no answer).
///
/// An export that isn't older than the last send raises no question, and
/// whatever came as an answer is ignored. One that is goes only on a plain
/// "no" (`Some(false)`): "yes" means rekordbox has changed since the
/// export, and no answer is not a no. This is the only place that decides
/// it (owner decision, 2026-10-10; before, such a send was always refused).
pub(super) fn may_send_from_this_export(
    preflight: &Preflight,
    imported_since_export: Option<bool>,
) -> bool {
    !preflight.export_older_than_last_send || imported_since_export == Some(false)
}

/// When the last send was recorded, in milliseconds since the Unix epoch;
/// `None` if no send ever was.
///
/// A send's time is written by `record_send` on every track it sent
/// (`library_track.last_exported_at`, kept in `library_removal` when the
/// track is removed) and on every crate and folder it wrote
/// (`sent_playlist.sent_at`); the latest of them all is the last send. It's
/// compared with the export's modified time ([`ExportRead::modified_ms`]),
/// the same clock rekordbox's save and the app's send both run on
/// ([`export_older_than_last_send`]).
pub(crate) fn last_send_ms(conn: &Connection) -> rusqlite::Result<Option<i64>> {
    // Whole seconds, then the three digits after the point: exact, where
    // julianday arithmetic would round.
    conn.query_row(
        "SELECT max(CAST(strftime('%s', at) AS INTEGER) * 1000 + CAST(substr(at, 21, 3) AS INTEGER))
         FROM (SELECT last_exported_at AS at FROM library_track
               UNION ALL SELECT last_exported_at FROM library_removal
               UNION ALL SELECT sent_at FROM sent_playlist)
         WHERE at IS NOT NULL",
        [],
        |r| r.get(0),
    )
}

/// Names a preflight by everything it says and every byte it would
/// write, so a write given the token back can prove the send it builds is
/// the one reviewed.
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

pub(super) fn refusal(error: &BuildError) -> Refusal {
    let (reason, path) = match error {
        BuildError::RepeatedName { path } => (RefusalReason::SameName, path.clone()),
        BuildError::EmptyName { path } => (RefusalReason::NoName, path.clone()),
        BuildError::UncarriableName { path } => (RefusalReason::UnsendableName, path.clone()),
        BuildError::TooDeep { path } => (RefusalReason::TooDeep, path.clone()),
        BuildError::RepeatedTrack(_) | BuildError::TrackIdsExhausted | BuildError::ReadBack(_) => {
            eprintln!("send: the file can't be built: {error}");
            (RefusalReason::Internal, Vec::new())
        }
    };
    Refusal { reason, path }
}

fn left_out_reason(reason: &Reason) -> (LeftOutReason, Option<String>) {
    match reason {
        Reason::NoValues(CannotSend::NotInLibrary | CannotSend::NoLinkedFile)
        | Reason::NotGiven => (LeftOutReason::NoFile, None),
        Reason::NoValues(CannotSend::FileMissing { .. }) => (LeftOutReason::FileMissing, None),
        Reason::NoValues(CannotSend::NoLocation { .. }) | Reason::NoLocation => {
            (LeftOutReason::NoPath, None)
        }
        Reason::Location(_) => (LeftOutReason::PathNotSendable, None),
        Reason::UncarriableCharacter { attribute } => {
            (LeftOutReason::UnsendableCharacter, Some(attribute.clone()))
        }
        Reason::DuplicateLocation => (LeftOutReason::SameFileAsAnother, None),
        Reason::BadTrackId | Reason::TrackIdOnNewTrack | Reason::DuplicateTrackId => {
            (LeftOutReason::RekordboxEntry, None)
        }
        Reason::BadAttributeName { attribute } | Reason::RepeatedAttribute { attribute } => {
            (LeftOutReason::RekordboxEntry, Some(attribute.clone()))
        }
    }
}

/// Every crate and playlist that's written with fewer entries than it
/// has, in tree order. A crate's playlist in rekordbox loses entries too
/// when it holds tracks no Library track is sent as (`outside`, by key path;
/// 1.10, `conflicts::membership`): they count among those left out, and
/// among its entries.
fn loses_entries(
    out: &Outgoing,
    crates: &[Node],
    outside: &HashMap<Vec<String>, u32>,
) -> Vec<LosesEntries> {
    let mut lost: HashMap<&[String], u32> = HashMap::new();
    for entry in out.left_out.iter().flat_map(|left| &left.entries) {
        *lost.entry(entry.path.as_slice()).or_default() += 1;
    }
    struct Counts<'a> {
        lost: HashMap<&'a [String], u32>,
        outside: &'a HashMap<Vec<String>, u32>,
    }
    fn visit(
        nodes: &[Node],
        path: &mut Vec<String>,
        kind: TreeKind,
        counts: &Counts<'_>,
        found: &mut Vec<LosesEntries>,
    ) {
        for node in nodes {
            match node {
                Node::Folder { name, children } => {
                    path.push(name.clone());
                    visit(children, path, kind, counts, found);
                }
                Node::Playlist { name, entries } => {
                    path.push(name.clone());
                    let key: Vec<String> = path.iter().map(|n| sibling_key(n)).collect();
                    let left_out = counts.lost.get(path.as_slice()).copied().unwrap_or(0);
                    let outside = counts.outside.get(&key).copied().unwrap_or(0);
                    if left_out + outside > 0 {
                        let entries = u32::try_from(entries.len()).unwrap_or(u32::MAX);
                        found.push(LosesEntries {
                            kind,
                            path: path[1..].to_vec(),
                            lost: left_out.saturating_add(outside),
                            entries: entries.saturating_add(outside),
                        });
                    }
                }
            }
            path.pop();
        }
    }
    let counts = Counts { lost, outside };
    let mut found = Vec::new();
    visit(
        crates,
        &mut vec![CRATES_FOLDER.to_owned()],
        TreeKind::Crate,
        &counts,
        &mut found,
    );
    visit(
        &[],
        &mut vec![PLAYLISTS_FOLDER.to_owned()],
        TreeKind::Playlist,
        &counts,
        &mut found,
    );
    found
}
