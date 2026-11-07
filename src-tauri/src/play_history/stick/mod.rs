//! Play history from a gig stick (ROADMAP 3.6, §5.4): the history lists
//! ("HISTORY 001", ...) in the stick's `PIONEER\rekordbox\export.pdb`.
//!
//! - **The stick is only read, and only that one file.** It is opened once,
//!   for reading, and copied into a staging folder in app data
//!   ([`WriteGuard::staging`]); the copy is read ([`devicesql`]) and the
//!   staging folder is removed whatever happens. Nothing else on the stick is
//!   opened: not its music, and never OneLibrary's `exportLibrary.db`.
//! - The user picks which lists to import. Each becomes one session, dated
//!   the day the user gives (the import day unless changed), never from file
//!   dates (§5.4); it can be corrected later like any session.
//! - A play's path is the stick's root plus the path the stick gives, matched
//!   to the Library's files as an m3u8 play is ([`super::matching`]). A play
//!   that matches nothing keeps its artist and title.
//! - A list whose plays, in order, are already a session on that day is left
//!   out as already imported ([`super::same_session`]).
//! - One import, however many lists, is one operation: one Undo.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tauri::State;

use super::{
    count, insert_session, m3u8, matching, same_session, NewPlay, IMPORT_OPERATION,
    MAX_NAME_CHARS,
};
use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::ops::{self, OpsError};
use crate::relink::Mounted;
use crate::write_guard::WriteGuard;

pub mod devicesql;

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;

/// The staging folder the stick's database is copied into.
pub const STAGING: &str = "gig-stick";
const COPY: &str = "export.pdb";

/// One stick read at a time: they share the staging folder.
static READING: Mutex<()> = Mutex::new(());

/// Why a stick couldn't be read, or nothing was imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum StickRefusal {
    /// The folder has no `PIONEER\rekordbox\export.pdb`.
    NoExport,
    /// It's there but couldn't be read or copied.
    Unreadable,
    /// It's over [`devicesql::MAX_BYTES`].
    TooBig,
    /// It isn't a rekordbox export database.
    NotAnExport,
    /// None of the lists asked for is on the stick with plays.
    NothingChosen,
    /// Not a day of the calendar as `YYYY-MM-DD`.
    BadDate,
}

/// A history list found on the stick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StickList {
    pub id: u32,
    pub name: String,
    pub plays: u32,
}

/// What reading a stick found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum StickListing {
    #[serde(rename_all = "camelCase")]
    Listed {
        lists: Vec<StickList>,
        /// Parts of the stick's database that couldn't be read.
        skipped: u32,
    },
    #[serde(rename_all = "camelCase")]
    Refused { reason: StickRefusal },
}

/// What a stick import came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum StickImport {
    /// Done as one operation (`None` when every list was already imported).
    #[serde(rename_all = "camelCase")]
    Done {
        operation_id: Option<i64>,
        /// The new sessions.
        sessions: Vec<i64>,
        /// Plays imported, and how many of them matched a track.
        plays: u32,
        matched: u32,
        /// Lists left out because they are already a session.
        already_imported: u32,
        /// Parts of the stick's database that couldn't be read.
        skipped: u32,
    },
    #[serde(rename_all = "camelCase")]
    Refused { reason: StickRefusal },
}

/// Where a stick keeps its export database, from its root.
pub fn export_path(root: &Path) -> PathBuf {
    root.join("PIONEER").join("rekordbox").join("export.pdb")
}

/// `path` in the form files are opened through: `\\?\` on Windows, so
/// names are taken literally. `None` if it isn't a full path.
fn open_path(path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    return crate::paths::verbatim_absolute(path).ok();
    #[cfg(not(windows))]
    return path.is_absolute().then(|| path.to_owned());
}

/// Reads the history of the stick whose root folder is `root`, through a
/// copy in staging that is removed before this returns.
pub fn read_stick(guard: &WriteGuard, root: &Path) -> Result<devicesql::Export, StickRefusal> {
    let _one = READING.lock().unwrap_or_else(PoisonError::into_inner);
    let path = open_path(&export_path(root)).ok_or(StickRefusal::NoExport)?;
    let mut source = File::open(&path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => StickRefusal::NoExport,
        _ => StickRefusal::Unreadable,
    })?;
    let meta = source.metadata().map_err(|_| StickRefusal::Unreadable)?;
    if !meta.is_file() {
        return Err(StickRefusal::NoExport);
    }
    if meta.len() > devicesql::MAX_BYTES {
        return Err(StickRefusal::TooBig);
    }
    // Dropped (removing the copy) on every early return below.
    let staging = guard
        .staging(STAGING)
        .map_err(|_| StickRefusal::Unreadable)?;
    let copy = staging
        .copy_in(COPY, &mut (&mut source).take(devicesql::MAX_BYTES + 1))
        .map_err(|_| StickRefusal::Unreadable)?;
    drop(source);
    let bytes = std::fs::read(&copy).map_err(|_| StickRefusal::Unreadable)?;
    // A failure to remove it is retried by the next read's fresh staging.
    let _ = staging.close();
    if bytes.len() as u64 > devicesql::MAX_BYTES {
        return Err(StickRefusal::TooBig);
    }
    devicesql::parse(&bytes).ok_or(StickRefusal::NotAnExport)
}

/// The lists of a read stick, for the user to pick from.
pub fn listing(export: &devicesql::Export) -> StickListing {
    StickListing::Listed {
        lists: export
            .lists
            .iter()
            .map(|l| StickList {
                id: l.id,
                name: l.name.clone(),
                plays: count(l.plays.len()),
            })
            .collect(),
        skipped: export.skipped,
    }
}

/// The stick path `rel` (from the root) as a full path under `root`.
fn on_stick(root: &str, rel: &str) -> String {
    let root = root.trim_end_matches(['\\', '/']);
    let rel = rel.trim_start_matches(['\\', '/']).replace('/', "\\");
    format!("{root}\\{rel}")
}

/// Imports the lists `chosen` (by id) of the stick `export`, read from the
/// folder `root`, as sessions played on `played_on`: one operation.
pub fn import_stick(
    conn: &mut Connection,
    mounted: &Mounted,
    root: &str,
    export: &devicesql::Export,
    chosen: &[u32],
    played_on: &str,
) -> Result<StickImport, OpsError> {
    let refused = |reason| Ok(StickImport::Refused { reason });
    let played_on = played_on.trim();
    if !m3u8::real_day(played_on) {
        return refused(StickRefusal::BadDate);
    }
    let picked: Vec<_> = export
        .lists
        .iter()
        .filter(|l| chosen.contains(&l.id) && !l.plays.is_empty())
        .collect();
    if picked.is_empty() {
        return refused(StickRefusal::NothingChosen);
    }

    let index = matching::PathIndex::load(conn, mounted)?;
    let mut sets: Vec<(String, Vec<NewPlay>)> = Vec::new();
    let mut already = 0_u32;
    for list in picked {
        let plays: Vec<NewPlay> = list
            .plays
            .iter()
            .map(|p| {
                let path = on_stick(root, &p.path);
                // A path that climbs out of the stick names nothing on it.
                let inside = !p.path.split(['/', '\\']).any(|part| part == "..");
                NewPlay {
                    recording: inside.then(|| index.recording_for(&path, None)).flatten(),
                    entry: m3u8::Entry {
                        path,
                        artist: p.artist.clone(),
                        title: p.title.clone(),
                    },
                }
            })
            .collect();
        if same_session(conn, &plays, played_on)?.is_some() {
            already += 1;
            continue;
        }
        sets.push((list.name.chars().take(MAX_NAME_CHARS).collect(), plays));
    }

    let total = count(sets.iter().map(|(_, p)| p.len()).sum());
    let matched = count(
        sets.iter()
            .flat_map(|(_, p)| p)
            .filter(|p| p.recording.is_some())
            .count(),
    );
    if sets.is_empty() {
        return Ok(StickImport::Done {
            operation_id: None,
            sessions: Vec::new(),
            plays: 0,
            matched: 0,
            already_imported: already,
            skipped: export.skipped,
        });
    }
    let names: Vec<&str> = sets.iter().map(|(n, _)| n.as_str()).collect();
    let recorded = ops::record(
        conn,
        IMPORT_OPERATION,
        &json!({ "source": "stick", "names": names, "plays": total }),
        |rec| {
            sets.iter()
                .map(|(name, plays)| insert_session(rec, "stick", name, played_on, plays))
                .collect::<Result<Vec<i64>, OpsError>>()
        },
    )?;
    Ok(StickImport::Done {
        operation_id: recorded.operation_id,
        sessions: recorded.value,
        plays: total,
        matched,
        already_imported: already,
        skipped: export.skipped,
    })
}

// ---- commands ---------------------------------------------------------------

/// The history lists of the stick whose root folder is `root` (picked by the
/// user in the folder dialog). The stick is only read.
#[tauri::command(async)]
#[specta::specta]
pub fn history_stick_lists(
    guard: State<'_, WriteGuard>,
    root: String,
) -> Result<StickListing, IpcError> {
    Ok(match read_stick(&guard, Path::new(&root)) {
        Ok(export) => listing(&export),
        Err(reason) => StickListing::Refused { reason },
    })
}

/// Imports the lists `list_ids` of the stick at `root` as sessions played on
/// `played_on`. The stick is read again, so what's imported is what's on it now.
#[tauri::command(async)]
#[specta::specta]
pub fn history_import_stick(
    reads: State<'_, ReadPool>,
    writer: State<'_, Writer>,
    guard: State<'_, WriteGuard>,
    root: String,
    list_ids: Vec<u32>,
    played_on: String,
) -> Result<StickImport, IpcError> {
    let export = match read_stick(&guard, Path::new(&root)) {
        Ok(export) => export,
        Err(reason) => return Ok(StickImport::Refused { reason }),
    };
    let known = reads.read(crate::relink::identities)?;
    let mounted = Mounted::ask(&known, &crate::scan::system_volumes());
    Ok(writer.call(move |c| {
        Ok(import_stick(
            c, &mounted, &root, &export, &list_ids, &played_on,
        ))
    })??)
}
