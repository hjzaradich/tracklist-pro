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
}

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// Stores `answer` for `file` by the rule in the module docs.
pub fn save(conn: &Connection, file: i64, answer: &Answer) -> rusqlite::Result<()> {
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
                PATH,
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
        params![file, PATH, answer.outcome.as_str(), reason],
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
            "SELECT fetched_at, outcome, reason, candidates, releases, attempt_reason, attempt_at
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
            ))
        })
        .optional()?;
    Ok(row.and_then(
        |(fetched_at, outcome, reason, candidates, releases, a_reason, a_at)| {
            Some(Cached {
                fetched_at,
                outcome: Outcome::parse(&outcome)?,
                reason,
                candidates: decode(&candidates),
                releases: serde_json::from_str(&releases).unwrap_or_default(),
                attempt: a_reason.zip(a_at),
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
        .filter(|c| c.outcome == Outcome::Found)
        .map(|c| c.candidates)
        .unwrap_or_default())
}
