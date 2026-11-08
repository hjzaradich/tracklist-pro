//! What the online look-ups found, kept per file (table `identify_lookup`,
//! migration 0058). A cache: it is fetched again when asked for, and does
//! not travel in bundles.
//!
//! A new answer replaces the file's old one only when it completes (details
//! found, or the services replied with nothing). A failure or a skip leaves a
//! good older answer where it is and only notes itself beside it; it does
//! replace an older failure or skip. So a bad connection never costs the
//! user what an earlier look-up found.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::lookup::{Answer, Outcome, PATH};
use super::{Candidate, IdentifyField, IdentifySource};
use crate::library::LibraryTrackId;

/// A candidate as it is stored.
#[derive(Serialize, Deserialize)]
struct Stored {
    field: IdentifyField,
    value: String,
    service: String,
    via: Option<String>,
}

/// A file's stored answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached {
    pub fetched_at: String,
    pub outcome: Outcome,
    pub reason: Option<String>,
    pub candidates: Vec<Candidate>,
    pub releases: Vec<String>,
    /// A later attempt that failed or was skipped: its reason and when.
    pub attempt: Option<(String, String)>,
    /// The file's audio changed after this was fetched: it no longer says
    /// anything about the file and is not shown as found.
    pub stale: bool,
}

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// Stores `answer` for `file` by the rule in the module docs.
#[cfg(test)]
pub fn save(conn: &Connection, file: i64, answer: &Answer) -> rusqlite::Result<()> {
    save_at(conn, file, answer, current_rev(conn, file)?.unwrap_or(0))
}

#[cfg(test)]
/// The file's `fingerprint_rev` now, if it has a row.
fn current_rev(conn: &Connection, file: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT fingerprint_rev FROM file WHERE id = ?1",
        [file],
        |r| r.get(0),
    )
    .optional()
}

/// The `path` column: the services asked, then (when the file had a
/// fingerprint) `;fp=<its revision>`. Not a column of its own: a migration
/// for it would be 0059.
fn path_for(rev: i64) -> String {
    if rev == 0 {
        PATH.to_owned()
    } else {
        format!("{PATH};fp={rev}")
    }
}

fn rev_of(path: &str) -> i64 {
    path.rsplit_once(";fp=")
        .and_then(|(_, rev)| rev.parse().ok())
        .unwrap_or(0)
}

/// [`save`] for an answer worked out from the file as it was at revision
/// `rev` of its fingerprint (0: it had none). An old answer the audio has
/// changed since is dropped first, so it doesn't outrank this one.
pub fn save_at(conn: &Connection, file: i64, answer: &Answer, rev: i64) -> rusqlite::Result<()> {
    if load(conn, file)?.is_some_and(|old| old.stale) {
        conn.execute("DELETE FROM identify_lookup WHERE file_id = ?1", [file])?;
    }
    let path = path_for(rev);
    let reason = answer.why.map(|w| w.code());
    if answer.outcome.completed() {
        let candidates = encode(&answer.candidates);
        let releases = serde_json::to_string(&answer.releases).unwrap_or_else(|_| "[]".into());
        conn.execute(
            &format!(
                "INSERT INTO identify_lookup
                     (file_id, path, outcome, reason, candidates, releases)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (file_id) DO UPDATE SET
                     path = excluded.path, fetched_at = {NOW},
                     outcome = excluded.outcome, reason = excluded.reason,
                     candidates = excluded.candidates, releases = excluded.releases,
                     attempt_reason = NULL, attempt_at = NULL"
            ),
            params![
                file,
                path,
                answer.outcome.as_str(),
                reason,
                candidates,
                releases
            ],
        )?;
        return Ok(());
    }
    // Replaces a failure or a skip; a good answer only notes the attempt.
    conn.execute(
        &format!(
            "INSERT INTO identify_lookup (file_id, path, outcome, reason)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (file_id) DO UPDATE SET
                 path = excluded.path, fetched_at = {NOW},
                 outcome = excluded.outcome, reason = excluded.reason,
                 candidates = '[]', releases = '[]',
                 attempt_reason = NULL, attempt_at = NULL
             WHERE identify_lookup.outcome IN ('failed', 'skipped')"
        ),
        params![file, path, answer.outcome.as_str(), reason],
    )?;
    conn.execute(
        &format!(
            "UPDATE identify_lookup SET attempt_reason = ?2, attempt_at = {NOW}
             WHERE file_id = ?1 AND outcome IN ('found', 'nothing')"
        ),
        params![file, reason],
    )?;
    Ok(())
}

fn encode(candidates: &[Candidate]) -> String {
    let stored: Vec<Stored> = candidates
        .iter()
        .filter_map(|c| match &c.source {
            IdentifySource::Online { service, via } => Some(Stored {
                field: c.field,
                value: c.value.clone(),
                service: service.clone(),
                via: via.clone(),
            }),
            _ => None,
        })
        .collect();
    serde_json::to_string(&stored).unwrap_or_else(|_| "[]".into())
}

/// Candidates read back; a row that doesn't parse gives none.
fn decode(json: &str) -> Vec<Candidate> {
    serde_json::from_str::<Vec<Stored>>(json)
        .unwrap_or_default()
        .into_iter()
        .map(|s| Candidate {
            field: s.field,
            value: s.value,
            source: IdentifySource::Online {
                service: s.service,
                via: s.via,
            },
        })
        .collect()
}

/// The stored answer for `file`, if it was ever looked up.
pub fn load(conn: &Connection, file: i64) -> rusqlite::Result<Option<Cached>> {
    let row = conn
        .prepare_cached(
            "SELECT fetched_at, outcome, reason, candidates, releases, attempt_reason, attempt_at,
                    path, (SELECT fingerprint_rev FROM file WHERE id = ?1)
             FROM identify_lookup WHERE file_id = ?1",
        )?
        .query_row([file], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<i64>>(8)?,
            ))
        })
        .optional()?;
    Ok(row.and_then(
        |(fetched_at, outcome, reason, candidates, releases, a_reason, a_at, path, now)| {
            let then = rev_of(&path);
            Some(Cached {
                fetched_at,
                outcome: Outcome::parse(&outcome)?,
                reason,
                candidates: decode(&candidates),
                releases: serde_json::from_str(&releases).unwrap_or_default(),
                attempt: a_reason.zip(a_at),
                // Fingerprinted when looked up, and a different audio now.
                stale: then != 0 && now.is_some_and(|now| now != then),
            })
        },
    ))
}

/// The online candidates stored for the linked file of the Library track
/// `track`; none when the track has no linked file or no details were found.
pub fn for_track(conn: &Connection, track: LibraryTrackId) -> rusqlite::Result<Vec<Candidate>> {
    let file: Option<Option<i64>> = conn
        .query_row(
            "SELECT linked_file_id FROM library_track WHERE id = ?1",
            [track.0],
            |r| r.get(0),
        )
        .optional()?;
    let Some(file) = file.flatten() else {
        return Ok(Vec::new());
    };
    Ok(load(conn, file)?
        .filter(|c| c.outcome == Outcome::Found && !c.stale)
        .map(|c| c.candidates)
        .unwrap_or_default())
}
