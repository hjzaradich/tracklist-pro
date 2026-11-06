//! What a smart crate holds: (rule ∪ always) − never, Library tracks only.
//! One SQL condition ([`condition`]) serves the track table (its rows and
//! its live count) and the send, so what the screen lists is what goes out.
//!
//! It reads the track table's index (`track_row`), as the table does: a
//! track the index hasn't caught up with yet joins at the next look or the
//! next send. A send evaluates every smart crate as it starts
//! ([`evaluated`]) and writes each one like a hand-made crate; the playlist
//! it records (`sent_playlist`, `sent_entry`) is that evaluated membership.
//! A refresh is not a user operation: nothing is written to the log.
//!
//! rekordbox-side changes to a smart crate's playlist are ignored, with no
//! question: the app owns these playlists. The membership merge before a
//! send compares hand-made crates only
//! (`crate::conflicts::membership::crate_paths`).

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::Connection;

use super::{compile, SmartRule};
use crate::crates::CrateId;
use crate::library::LibraryTrackId;
use crate::search::flag;

/// The recording ids on one of a smart crate's lists, crate id bound as `mark`.
fn listed(mark: &str, kind: &str) -> String {
    format!(
        "SELECT slt.recording_id FROM crate_entry sce
         JOIN library_track slt ON slt.id = sce.library_track_id
         WHERE sce.crate_id = {mark} AND sce.kind = '{kind}'"
    )
}

/// SQL on `tr` (a `track_row`) that holds for a smart crate's tracks:
/// the Library tracks `rule` matches (none when there is no rule), less the
/// crate's never-include list, plus its always-include list. With no `id`,
/// the rule's Library tracks alone (a rule not saved yet). Values go onto
/// `params`.
pub fn condition(id: Option<CrateId>, rule: Option<&SmartRule>, params: &mut Vec<Value>) -> String {
    let matched = match rule {
        Some(rule) => compile::compile(rule, params),
        None => "0".to_owned(),
    };
    let library = format!("(tr.flags & {}) <> 0", flag::IN_LIBRARY);
    let Some(id) = id else {
        return format!("({library} AND {matched})");
    };
    params.push(Value::Integer(id.0));
    let mark = format!("?{}", params.len());
    format!(
        "(({library} AND {matched} AND tr.recording_id NOT IN ({never}))
          OR tr.recording_id IN ({always}))",
        never = listed(&mark, "never_include"),
        always = listed(&mark, "always_include"),
    )
}

/// A smart crate's stored rule: `None` if there's no such smart crate,
/// `Some(None)` if its rule doesn't read.
pub fn stored_rule(conn: &Connection, id: CrateId) -> rusqlite::Result<Option<Option<SmartRule>>> {
    let mut stmt =
        conn.prepare_cached("SELECT rules FROM crate WHERE id = ?1 AND kind = 'smart'")?;
    let mut rows = stmt.query([id.0])?;
    match rows.next()? {
        Some(row) => {
            let text: Option<String> = row.get(0)?;
            Ok(Some(text.as_deref().and_then(SmartRule::from_json)))
        }
        None => Ok(None),
    }
}

/// The `FROM` and `WHERE` of a smart crate's Library tracks, for the send
/// and the crate list: [`condition`] on the index, and the always-include
/// list read from the crate's own entries too, so a track the index hasn't
/// reached yet still goes out.
fn held_where(id: CrateId, rule: Option<&SmartRule>, params: &mut Vec<Value>) -> String {
    let held = condition(Some(id), rule, params);
    params.push(Value::Integer(id.0));
    format!(
        "FROM library_track lt LEFT JOIN track_row tr ON tr.recording_id = lt.recording_id
         WHERE {held} OR lt.id IN (SELECT library_track_id FROM crate_entry
                                   WHERE crate_id = ?{} AND kind = 'always_include')",
        params.len()
    )
}

/// The Library tracks a smart crate holds now, in the Library list's order
/// (title, artist, id); empty for a crate that isn't a smart one.
pub fn evaluate(conn: &Connection, id: CrateId) -> rusqlite::Result<Vec<LibraryTrackId>> {
    let Some(rule) = stored_rule(conn, id)? else {
        return Ok(Vec::new());
    };
    let mut params = Vec::new();
    let held = held_where(id, rule.as_ref(), &mut params);
    let sql = format!(
        "SELECT lt.id {held}
         ORDER BY tr.title_key IS NULL, tr.title_key, tr.artist_key, lt.recording_id"
    );
    conn.prepare(&sql)?
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok(LibraryTrackId(r.get(0)?))
        })?
        .collect()
}

/// How many tracks a smart crate holds now.
pub fn count(conn: &Connection, id: CrateId) -> rusqlite::Result<u32> {
    let Some(rule) = stored_rule(conn, id)? else {
        return Ok(0);
    };
    let mut params = Vec::new();
    let held = held_where(id, rule.as_ref(), &mut params);
    let n: i64 = conn
        .prepare(&format!("SELECT count(*) {held}"))?
        .query_row(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Every smart crate's tracks, by crate id, as a send writes them.
pub fn evaluated(conn: &Connection) -> rusqlite::Result<HashMap<i64, Vec<LibraryTrackId>>> {
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM crate WHERE kind = 'smart' ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = HashMap::new();
    for id in ids {
        out.insert(id, evaluate(conn, CrateId(id))?);
    }
    Ok(out)
}
