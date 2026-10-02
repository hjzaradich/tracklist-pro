//! The preflight: a send built in memory and described, nothing written.

use std::collections::HashMap;

use rusqlite::Connection;

use super::{
    tree, ExportRead, LeftOut, LeftOutReason, LosesEntries, OtherFile, Preflight, Refusal,
    RefusalReason, TrackLabel, TreeKind,
};
use crate::library::{self, LibraryTrackId, StoredTrack};
use crate::paths::Volumes;
use crate::rekordbox::source::stored_source;
use crate::rekordbox_write::{
    self, BuildError, Node, Outgoing, Reason, CRATES_FOLDER, PLAYLISTS_FOLDER,
};
use crate::send_values::CannotSend;

/// A send as it would go now: its description, and the file's contents
/// unless the send is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reviewed {
    pub preflight: Preflight,
    pub outgoing: Option<Outgoing>,
}

/// Builds the send from the database as it is now (every Library track,
/// the crate tree, rekordbox's values from the last read) and describes
/// it. Only reads. `None` if rekordbox was never read.
///
/// Call it in one writer job, so everything comes from the same moment.
pub fn review(conn: &Connection, volumes: &impl Volumes) -> rusqlite::Result<Option<Reviewed>> {
    let Some(read) = stored_source(conn)?.last_read else {
        return Ok(None);
    };
    let tracks = library::stored(conn)?;
    let ids: Vec<LibraryTrackId> = tracks.iter().map(|t| t.id).collect();
    let crates = tree::crate_tree(conn)?;
    // Playlists arrive in Phase 3; the folder is still written (rule 6).
    let input = rekordbox_write::gather(conn, volumes, &ids, crates.clone(), Vec::new())?;
    let built = rekordbox_write::build(&input);

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
        other_file: Vec::new(),
        loses_entries: Vec::new(),
        refusal: None,
        nothing_to_send: false,
        can_send: false,
        needs_confirm: false,
    };
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
            preflight.loses_entries = loses_entries(&out, &crates);
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
    } else if let Some(refusal) = export_older_than_last_send(conn, read.modified_ms)? {
        preflight.refusal = Some(refusal);
    }
    let outgoing = outgoing.filter(|_| preflight.refusal.is_none());
    preflight.can_send = preflight.refusal.is_none() && !preflight.nothing_to_send;
    preflight.needs_confirm =
        !preflight.loses_entries.is_empty() || preflight.export.not_stored > 0;
    preflight.token = token(&preflight, outgoing.as_ref());
    Ok(Some(Reviewed {
        preflight,
        outgoing,
    }))
}

/// What a send from an export saved at `export_modified_ms` gets when that
/// is before the last send was recorded: such an export shows rekordbox as
/// it was before that send was imported, so tracks rekordbox now has would
/// go out as new. It's refused as a whole, and the user exports again
/// (owner decision, 2026-10-02). `None` when the export is from the very
/// millisecond of the last send or later, or no send was ever recorded.
///
/// The comparison and what follows from it are decided here and nowhere
/// else.
fn export_older_than_last_send(
    conn: &Connection,
    export_modified_ms: i64,
) -> rusqlite::Result<Option<Refusal>> {
    let older = last_send_ms(conn)?.is_some_and(|sent| export_modified_ms < sent);
    Ok(older.then(|| Refusal {
        reason: RefusalReason::ExportOlderThanLastSend,
        path: Vec::new(),
    }))
}

/// When the last send was recorded, in milliseconds since the Unix epoch;
/// `None` if no send ever was.
///
/// A send's time is written by `record_send` on every track it sent
/// (`library_track.last_exported_at`, kept in `library_removal` when the
/// track is removed) and on every crate and folder it wrote
/// (`sent_playlist.sent_at`); the latest of them all is the last send. It's
/// compared with the export's modified time ([`ExportRead::modified_ms`]),
/// the same clock rekordbox's save and the app's send both run on: an
/// export saved before it is refused, one saved at that very millisecond or
/// later isn't.
pub(super) fn last_send_ms(conn: &Connection) -> rusqlite::Result<Option<i64>> {
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
/// has, in tree order.
fn loses_entries(out: &Outgoing, crates: &[Node]) -> Vec<LosesEntries> {
    let mut lost: HashMap<&[String], u32> = HashMap::new();
    for entry in out.left_out.iter().flat_map(|left| &left.entries) {
        *lost.entry(entry.path.as_slice()).or_default() += 1;
    }
    let mut found = Vec::new();
    let mut walk = |top: &str, kind: TreeKind, nodes: &[Node]| {
        fn visit(
            nodes: &[Node],
            path: &mut Vec<String>,
            kind: TreeKind,
            lost: &HashMap<&[String], u32>,
            found: &mut Vec<LosesEntries>,
        ) {
            for node in nodes {
                match node {
                    Node::Folder { name, children } => {
                        path.push(name.clone());
                        visit(children, path, kind, lost, found);
                    }
                    Node::Playlist { name, entries } => {
                        path.push(name.clone());
                        if let Some(&lost) = lost.get(path.as_slice()) {
                            found.push(LosesEntries {
                                kind,
                                path: path[1..].to_vec(),
                                lost,
                                entries: u32::try_from(entries.len()).unwrap_or(u32::MAX),
                            });
                        }
                    }
                }
                path.pop();
            }
        }
        visit(nodes, &mut vec![top.to_owned()], kind, &lost, &mut found);
    };
    walk(CRATES_FOLDER, TreeKind::Crate, crates);
    walk(PLAYLISTS_FOLDER, TreeKind::Playlist, &[]);
    found
}
