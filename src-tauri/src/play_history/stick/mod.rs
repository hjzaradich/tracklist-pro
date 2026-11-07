//! Play history from a gig stick (ROADMAP 3.6, §5.4): the history lists
//! ("HISTORY 001", ...) in the stick's `PIONEER\rekordbox\export.pdb`.
//!
//! - **The stick is only read, and only that one file.** It is opened once,
//!   for reading, and copied into a staging folder in app data
//!   ([`WriteGuard::staging`]); the copy is read ([`devicesql`]) and the
//!   staging folder is removed whatever happens. Nothing else on the stick is
//!   opened but the audio files of the stick's `Contents` folder that a
//!   chosen list plays ([`readable_file`]): never OneLibrary's
//!   `exportLibrary.db`, never a link, never anything that isn't audio.
//! - The user picks which lists to import. Each becomes one session, dated
//!   the day the user gives (the import day unless changed), never from file
//!   dates (§5.4); it can be corrected later like any session.
//! - **Matched by content, not path:** the stick's files are its own copies,
//!   so a play's path (the stick's root plus the path the stick gives) names
//!   no Library file. Instead each stick file a chosen list plays is read
//!   (only read, and only those files) and hashed as the scan hashes
//!   ([`crate::hash`]); a track whose file has the same `audio_hash` (which
//!   survives tag edits), or else the same whole-file `blake3`, is the match.
//!   Audio shared by several tracks matches none: a wrong match is worse than
//!   none. A play that matches nothing keeps its artist and title.
//! - A list whose plays, in order, are already a session on that day is left
//!   out as already imported ([`super::same_session`]).
//! - One import, however many lists, is one operation: one Undo.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tauri::State;

use super::{count, insert_session, m3u8, same_session, NewPlay, IMPORT_OPERATION, MAX_NAME_CHARS};
use crate::db::Writer;
use crate::hash::{self, AUDIO_HASH_LEN};
use crate::ipc::IpcError;
use crate::ops::{self, OpsError};
use crate::write_guard::WriteGuard;

pub mod devicesql;

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;

/// The staging folder the stick's database is copied into.
pub const STAGING: &str = "gig-stick";
const COPY: &str = "export.pdb";
/// The biggest stick audio file hashed; a bigger one stays unmatched.
pub const MAX_AUDIO_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// The most stick files hashed in one import; the rest stay unmatched.
pub const MAX_FILES: usize = 1000;

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
        /// Plays imported, and how many of them matched a track by the
        /// same audio.
        plays: u32,
        matched: u32,
        /// Plays left unmatched because their audio is several tracks'.
        ambiguous: u32,
        /// Plays left unmatched because their file on the stick couldn't be
        /// read (missing, too big, unreadable).
        unreadable: u32,
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
    // ci-reads: not a frontend read
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

/// The stick path `rel` the way it is stored and compared: `\` between its
/// parts, no leading separator, so it doesn't depend on the drive the stick
/// is plugged into.
fn stick_relative(rel: &str) -> String {
    rel.trim_start_matches(['\\', '/']).replace('/', "\\")
}

/// The one folder of a stick whose files are ever opened.
const CONTENTS: &str = "Contents";

/// The full path of the file the stick names `rel` (from its root `root`),
/// only when it may be read: its parts (either slash; empty and `.` parts
/// dropped) are `Contents` (any case) and then names, none of them `..` or
/// holding a `:` (a drive or a stream); it ends in an extension the app
/// accepts as audio; and `Contents` itself and every part below it is a plain folder or
/// file, not a link or junction, the last a regular file. Everything else,
/// the OneLibrary database first of all, is never opened.
fn readable_file(root: &str, rel: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = rel
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let (first, below) = parts.split_first()?;
    if !first.eq_ignore_ascii_case(CONTENTS)
        || below.is_empty()
        || parts.iter().any(|p| *p == ".." || p.contains(':'))
    {
        return None;
    }
    let path = PathBuf::from(on_stick(root, &parts.join("\\")));
    if !crate::tags::is_audio_file(&path) {
        return None;
    }
    let mut at = PathBuf::from(on_stick(root, CONTENTS));
    // `Contents` itself: a plain folder, not a link or junction to elsewhere.
    let top = std::fs::symlink_metadata(&at).ok()?;
    if top.file_type().is_symlink() || is_reparse_point(&top) || !top.is_dir() {
        return None;
    }
    for (i, part) in below.iter().enumerate() {
        at.push(part);
        let meta = std::fs::symlink_metadata(&at).ok()?;
        if meta.file_type().is_symlink() || is_reparse_point(&meta) {
            return None;
        }
        if i + 1 == below.len() && !meta.is_file() {
            return None;
        }
    }
    Some(path)
}

#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_: &std::fs::Metadata) -> bool {
    false
}

/// What a stick file's bytes are, for matching it to a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Content {
    /// Its `audio_hash`, stored form, when its audio could be found.
    pub audio: Option<[u8; AUDIO_HASH_LEN]>,
    /// The BLAKE3 of the whole file.
    pub whole: [u8; 32],
}

/// The content of each stick file the lists `chosen` play, by the path the
/// stick gives (`None`: it couldn't be read). Each file is opened only to be
/// read, once however often it's played; at most [`MAX_FILES`] files, each
/// at most [`MAX_AUDIO_BYTES`].
pub fn hash_stick_files(
    root: &str,
    export: &devicesql::Export,
    chosen: &[u32],
) -> HashMap<String, Option<Content>> {
    let mut out: HashMap<String, Option<Content>> = HashMap::new();
    let mut buf = vec![0_u8; hash::BUFFER];
    let plays = export
        .lists
        .iter()
        .filter(|l| chosen.contains(&l.id))
        .flat_map(|l| &l.plays);
    for play in plays {
        if out.contains_key(&play.path) {
            continue;
        }
        let content = (out.len() < MAX_FILES)
            .then(|| {
                let path = readable_file(root, &play.path)?;
                hash::hash_path(&path, &mut buf, &mut |read| read > MAX_AUDIO_BYTES).ok()
            })
            .flatten()
            .flatten()
            .map(|h| Content {
                audio: h.audio.ok().map(|a| a.to_bytes()),
                whole: h.blake3,
            });
        out.insert(play.path.clone(), content);
    }
    out
}

/// The track whose file has the same content, if exactly one has.
enum Found {
    One(i64),
    Several,
    None,
}

fn track_with(conn: &Connection, content: &Content) -> rusqlite::Result<Found> {
    let by = |column: &str, value: &[u8]| -> rusqlite::Result<Found> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT DISTINCT rf.recording_id FROM file f
             JOIN recording_file rf ON rf.file_id = f.id
             WHERE f.{column} = ?1 LIMIT 2"
        ))?;
        let tracks = stmt
            .query_map([value], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(match tracks[..] {
            [one] => Found::One(one),
            [] => Found::None,
            _ => Found::Several,
        })
    };
    if let Some(audio) = &content.audio {
        match by("audio_hash", audio)? {
            Found::None => {}
            found => return Ok(found),
        }
    }
    by("blake3", &content.whole)
}

/// Imports the lists `chosen` (by id) of the stick `export` as sessions
/// played on `played_on`: one operation. `contents` is what
/// [`hash_stick_files`] found. A play's path is kept as the stick gives it
/// ([`stick_relative`]), never with the drive: the same stick plugged in
/// elsewhere is still the same stick.
pub fn import_stick(
    conn: &mut Connection,
    export: &devicesql::Export,
    chosen: &[u32],
    played_on: &str,
    contents: &HashMap<String, Option<Content>>,
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

    // Each stick file's track, looked up once.
    let mut tracks: HashMap<&str, Found> = HashMap::new();
    for play in picked.iter().flat_map(|l| &l.plays) {
        if tracks.contains_key(play.path.as_str()) {
            continue;
        }
        let found = match contents.get(&play.path).copied().flatten() {
            Some(content) => track_with(conn, &content)?,
            None => Found::None,
        };
        tracks.insert(&play.path, found);
    }

    let mut sets: Vec<(String, Vec<NewPlay>, u32, u32)> = Vec::new();
    let mut already = 0_u32;
    for list in picked {
        let (mut ambiguous, mut unreadable) = (0, 0);
        let plays: Vec<NewPlay> = list
            .plays
            .iter()
            .map(|p| {
                let recording = match tracks.get(p.path.as_str()) {
                    Some(Found::One(track)) => Some(*track),
                    Some(Found::Several) => {
                        ambiguous += 1;
                        None
                    }
                    _ => {
                        if !matches!(contents.get(&p.path), Some(Some(_))) {
                            unreadable += 1;
                        }
                        None
                    }
                };
                NewPlay {
                    recording,
                    entry: m3u8::Entry {
                        path: stick_relative(&p.path),
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
        let name = list.name.chars().take(MAX_NAME_CHARS).collect();
        sets.push((name, plays, ambiguous, unreadable));
    }

    let total = count(sets.iter().map(|s| s.1.len()).sum());
    let matched = count(
        sets.iter()
            .flat_map(|s| &s.1)
            .filter(|p| p.recording.is_some())
            .count(),
    );
    let ambiguous = sets.iter().map(|s| s.2).sum();
    let unreadable = sets.iter().map(|s| s.3).sum();
    if sets.is_empty() {
        return Ok(StickImport::Done {
            operation_id: None,
            sessions: Vec::new(),
            plays: 0,
            matched: 0,
            ambiguous: 0,
            unreadable: 0,
            already_imported: already,
            skipped: export.skipped,
        });
    }
    let names: Vec<&str> = sets.iter().map(|s| s.0.as_str()).collect();
    let recorded = ops::record(
        conn,
        IMPORT_OPERATION,
        &json!({ "source": "stick", "names": names, "plays": total }),
        |rec| {
            sets.iter()
                .map(|(name, plays, ..)| insert_session(rec, "stick", name, played_on, plays))
                .collect::<Result<Vec<i64>, OpsError>>()
        },
    )?;
    Ok(StickImport::Done {
        operation_id: recorded.operation_id,
        sessions: recorded.value,
        plays: total,
        matched,
        ambiguous,
        unreadable,
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
/// `played_on`. The stick is read again, so what's imported is what's on it
/// now; its files are hashed before the database is asked.
#[tauri::command(async)]
#[specta::specta]
pub fn history_import_stick(
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
    let contents = hash_stick_files(&root, &export, &list_ids);
    Ok(writer.call(move |c| Ok(import_stick(c, &export, &list_ids, &played_on, &contents)))??)
}
