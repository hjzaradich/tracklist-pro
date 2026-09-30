//! Which files are due a fingerprint, and what happened to each, kept in
//! the shared `file_stage` table through [`crate::scan_state`].
//!
//! A present file is due when the fingerprint stage has no row for it, when
//! its size or modified time differ from the row's, when the row is from
//! another [`VERSION`], or when it was skipped last time (online-only, or
//! couldn't be reached). A file whose content couldn't be fingerprinted
//! keeps a NULL fingerprint and a failed row with its reason, and isn't
//! tried again until it changes.
//!
//! **A tag-only rewrite doesn't decode again** (1aC-10). rekordbox and
//! taggers change a file's size and mtime without touching its audio. Each
//! fingerprint records the `audio_hash` it was computed from
//! (`file.fingerprint_audio_hash`), but only when the hash stage was
//! current for the file at that moment, so the hash could be vouched for.
//! When a file is due and [`carry_forward`] can prove nothing about the audio
//! changed, the fingerprint row moves to the file's new size and mtime and
//! the file isn't opened. Otherwise (no hash, a stale hash stage, a NULL on
//! either side, another audio_hash, another fingerprint version) it's
//! fingerprinted as usual. It never waits for the hash stage.
//!
//! Known limit: a tagger that saves by writing a new file and renaming it
//! over the old one gives the file a new file id. The walk then drops the
//! file's `file_stage` rows (ROADMAP 1.1, `scan::unchanged`), so there's no
//! done fingerprint row to carry and the file is decoded again. That's safe,
//! it just saves nothing; one that saves in place is carried. Another
//! narrow case: the walk's partial hash can carry the hash row of a touched
//! file (ROADMAP 1.1 accepts its blind spot), and this trusts that row.
//!
//! Each result is written with its `file_stage` row in one transaction,
//! and only while the `file` row still has the size and modified time the
//! job read: if a walk changed it meanwhile, nothing is written and the
//! file stays due.

use rusqlite::{params, Connection, OptionalExtension};

use super::decode::Unfingerprintable;
use super::stored::VERSION;
use crate::hash::DEFINITION as HASH_DEFINITION;
use crate::scan::MusicFolderId;
use crate::scan_state::{self, DueFile, Recorded, Scope, Stage};

/// How many due files are listed per query.
const PAGE: usize = 1000;

/// The stage version stored in `file_stage`.
fn version() -> i64 {
    i64::from(VERSION)
}

/// A file due a fingerprint, as its row says now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Due {
    pub id: i64,
    pub folder: MusicFolderId,
    pub rel_path: String,
    pub size: Option<i64>,
    /// Nanoseconds since the Unix epoch.
    pub mtime: Option<i64>,
    /// The walk found it online only (a OneDrive placeholder).
    pub online_only: bool,
}

impl From<DueFile> for Due {
    fn from(f: DueFile) -> Due {
        Due {
            id: f.id,
            folder: MusicFolderId(f.music_folder_id),
            rel_path: f.rel_path,
            size: f.size,
            mtime: f.mtime,
            online_only: f.online_only,
        }
    }
}

impl Due {
    fn recorded(&self, outcome: scan_state::Outcome) -> Recorded {
        Recorded {
            file: self.id,
            size: self.size,
            mtime: self.mtime,
            outcome,
        }
    }
}

/// How the last try at a file ended, as `file_stage` holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Its content can't be fingerprinted.
    Failed(Unfingerprintable),
    /// Not tried, with the reason: e.g. [`scan_state::ONLINE_ONLY`] or
    /// [`scan_state::UNREACHABLE`]. Due again on the next run.
    Skipped(String),
}

/// The ids of every present file that's due, or of those among `only`,
/// lowest first. One pass, page by page: skipped files stay due, so it
/// never starts over.
pub(crate) fn due_ids(conn: &Connection, only: Option<&[i64]>) -> rusqlite::Result<Vec<i64>> {
    let scope = match only {
        Some(ids) => Scope::Files(ids.to_vec()),
        None => Scope::All,
    };
    let mut ids = Vec::new();
    let mut after = 0;
    loop {
        let page = scan_state::due(conn, Stage::Fingerprint, version(), &scope, after, PAGE)?;
        let Some(last) = page.last() else {
            return Ok(ids);
        };
        after = last.id;
        ids.extend(page.iter().map(|f| f.id));
    }
}

/// File `id` as it stands now, if it's present and due.
pub(crate) fn due(conn: &Connection, id: i64) -> rusqlite::Result<Option<Due>> {
    let scope = Scope::Files(vec![id]);
    let page = scan_state::due(conn, Stage::Fingerprint, version(), &scope, 0, 1)?;
    Ok(page.into_iter().next().map(Due::from))
}

/// Stores `blob` as the file's fingerprint and records it done, unless its
/// row changed since `due` was read. Returns whether it was stored.
pub(crate) fn done(conn: &mut Connection, due: &Due, blob: &[u8]) -> rusqlite::Result<bool> {
    write(conn, due, Some(blob), scan_state::Outcome::Done)
}

/// Records that the file's content can't be fingerprinted: its fingerprint
/// is NULL (an older version's is cleared) and `why` is kept.
pub(crate) fn failed(
    conn: &mut Connection,
    due: &Due,
    why: Unfingerprintable,
) -> rusqlite::Result<()> {
    write(conn, due, None, scan_state::Outcome::Failed(why.reason())).map(|_| ())
}

/// Records that the file wasn't tried, e.g. [`scan_state::Outcome::unreachable`].
/// Its fingerprint is left as it is.
pub(crate) fn skipped(
    conn: &mut Connection,
    due: &Due,
    outcome: scan_state::Outcome,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    if row_unchanged(&tx, due)? {
        scan_state::record(&tx, Stage::Fingerprint, version(), &[due.recorded(outcome)])?;
    }
    tx.commit()
}

/// How the last try at file `id` ended, if it's been tried.
pub fn outcome(conn: &Connection, id: i64) -> rusqlite::Result<Option<Outcome>> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT status, reason FROM file_stage WHERE file_id = ?1 AND stage = ?2",
            params![id, Stage::Fingerprint.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.map(|(status, reason)| {
        let reason = reason.unwrap_or_default();
        match status.as_str() {
            "done" => Outcome::Done,
            "failed" => {
                Unfingerprintable::parse(&reason).map_or(Outcome::Skipped(reason), Outcome::Failed)
            }
            _ => Outcome::Skipped(reason),
        }
    }))
}

/// The condition that the hash stage is current for file `f`: it has an
/// `audio_hash`, and a done `hash` row at the current version, size and
/// mtime (?HASH is the hash stage's version).
const HASH_CURRENT: &str = "f.audio_hash IS NOT NULL
    AND EXISTS (SELECT 1 FROM file_stage h
                WHERE h.file_id = f.id AND h.stage = 'hash' AND h.status = 'done'
                  AND h.version = ?5 AND h.size IS f.size AND h.mtime IS f.mtime)";

/// Moves file `due.id`'s done fingerprint row to the size and mtime `due`
/// read, without decoding, if that's provable: the file row is unchanged
/// since `due`; the hash stage is current for it; the fingerprint's recorded
/// audio_hash equals the file's audio_hash (neither NULL); and the
/// fingerprint stage's row is a done row of the current [`VERSION`]. Returns
/// whether it did. Never waits for, or runs, the hash stage.
pub(crate) fn carry_forward(conn: &mut Connection, due: &Due) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    let provable: bool = tx
        .prepare_cached(&format!(
            "SELECT EXISTS (
                 SELECT 1 FROM file f
                 JOIN file_stage p ON p.file_id = f.id AND p.stage = 'fingerprint'
                 WHERE f.id = ?1 AND f.present = 1 AND f.size IS ?2 AND f.mtime IS ?3
                   AND f.fingerprint IS NOT NULL
                   AND f.fingerprint_audio_hash IS NOT NULL
                   AND f.fingerprint_audio_hash = f.audio_hash
                   AND p.status = 'done' AND p.version = ?4
                   AND {HASH_CURRENT})"
        ))?
        .query_row(
            params![
                due.id,
                due.size,
                due.mtime,
                version(),
                i64::from(HASH_DEFINITION)
            ],
            |r| r.get(0),
        )?;
    if provable {
        scan_state::record(
            &tx,
            Stage::Fingerprint,
            version(),
            &[due.recorded(scan_state::Outcome::Done)],
        )?;
    }
    tx.commit()?;
    Ok(provable)
}

/// Whether file `due.id` is present with the size and mtime `due` read.
fn row_unchanged(conn: &Connection, due: &Due) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM file
                        WHERE id = ?1 AND present = 1 AND size IS ?2 AND mtime IS ?3)",
    )?
    .query_row(params![due.id, due.size, due.mtime], |r| r.get(0))
}

/// Sets the fingerprint and records `outcome`, in one transaction, only if
/// the row is unchanged.
fn write(
    conn: &mut Connection,
    due: &Due,
    blob: Option<&[u8]>,
    outcome: scan_state::Outcome,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    // The audio_hash it came from, if the hash stage vouches for it now.
    let changed = tx
        .prepare_cached(&format!(
            "UPDATE file AS f SET fingerprint = ?2,
                 fingerprint_audio_hash =
                     CASE WHEN ?2 IS NOT NULL AND {HASH_CURRENT} THEN f.audio_hash END
             WHERE f.id = ?1 AND f.present = 1 AND f.size IS ?3 AND f.mtime IS ?4"
        ))?
        .execute(params![
            due.id,
            blob,
            due.size,
            due.mtime,
            i64::from(HASH_DEFINITION)
        ])?;
    if changed == 1 {
        scan_state::record(&tx, Stage::Fingerprint, version(), &[due.recorded(outcome)])?;
    }
    tx.commit()?;
    Ok(changed == 1)
}
