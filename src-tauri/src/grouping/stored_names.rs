//! The keys of each file's names, kept in the database (`grouping_names`,
//! migration 0045; 1cD-G1), so the index the deciding run finds files by
//! ([`super::decide::Names`]) can be built after a restart without parsing
//! anyone's tags.
//!
//! **A row is used only while it is current**: made at the file's `rev` now
//! and by today's [`RULES_VERSION`]. The database changes `rev` whenever a
//! file's tags or its name change, whichever code changed them, so nothing
//! has to remember to update a row: an old one is simply not read, and the
//! file's keys are worked out again. A rules version covers the parser the
//! keys come from: `grouping::tests::stored_names` pins the keys, so a parser
//! change that would leave old ones standing fails until the version moves.
//!
//! **A row is kept only if it is still true when it is written.** [`keep`]
//! writes it only while the file's `rev` is the one read before its tags were,
//! so a file retagged in between keeps no keys of its old names.
//!
//! The rows are only ever what candidates are found by. Every decision reads
//! the file's own tags ([`super::decide`]).

use std::collections::HashMap;

use rusqlite::{params, Connection};

use super::decide::Entry;
use super::RULES_VERSION;

/// Whether row `n` of file `g.file_id` is current, as SQL.
fn current_row() -> String {
    format!("n.rev = g.rev AND n.rules_version = {RULES_VERSION}")
}

/// One present file, as the index asks about it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Present {
    pub file: i64,
    /// Its record's `rev` (`None`: it has no record yet).
    pub rev: Option<i64>,
    /// It has a current row: its keys need no working out.
    pub stored: bool,
}

/// Every present file, by id.
pub(super) fn present(conn: &Connection) -> rusqlite::Result<Vec<Present>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT f.id, g.rev, n.file_id IS NOT NULL
         FROM file f LEFT JOIN grouping_file g ON g.file_id = f.id
         LEFT JOIN grouping_names n ON n.file_id = f.id AND {}
         WHERE f.present = 1 ORDER BY f.id",
        current_row()
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok(Present {
            file: r.get(0)?,
            rev: r.get(1)?,
            stored: r.get(2)?,
        })
    })?;
    rows.collect()
}

/// The keys of each of `ids` that is present and has a current row. A row
/// that can't be read is left out, like one that isn't there.
pub(super) fn current(conn: &Connection, ids: &[i64]) -> rusqlite::Result<HashMap<i64, Entry>> {
    let ids = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT n.file_id, n.entry
         FROM grouping_names n
         JOIN grouping_file g ON g.file_id = n.file_id
         JOIN file f ON f.id = n.file_id
         WHERE n.file_id IN (SELECT value FROM json_each(?1)) AND f.present = 1 AND {}",
        current_row()
    ))?;
    let rows = stmt.query_map([ids], |r| {
        Ok((r.get(0)?, r.get_ref(1)?.as_str().ok().and_then(decode)))
    })?;
    let mut entries = HashMap::new();
    for row in rows {
        if let (file, Some(entry)) = row? {
            entries.insert(file, entry);
        }
    }
    Ok(entries)
}

/// Keys worked out from a file's tags, and the `rev` read before those tags
/// were.
pub(super) struct WorkedOut {
    pub file: i64,
    pub rev: i64,
    pub entry: String,
}

/// Stores `worked_out` in one transaction, each only if its file's `rev` is
/// still the one it was worked out at.
pub(super) fn keep(conn: &mut Connection, worked_out: &[WorkedOut]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    {
        let mut keep = tx.prepare_cached(
            "INSERT INTO grouping_names (file_id, rev, rules_version, entry)
             SELECT file_id, rev, ?1, ?2 FROM grouping_file WHERE file_id = ?3 AND rev = ?4
             ON CONFLICT (file_id) DO UPDATE
             SET rev = excluded.rev, rules_version = excluded.rules_version,
                 entry = excluded.entry",
        )?;
        for found in worked_out {
            keep.execute(params![RULES_VERSION, found.entry, found.file, found.rev])?;
        }
    }
    tx.commit()
}

/// `entry` as a row holds it.
pub(super) fn encode(entry: &Entry) -> Option<String> {
    serde_json::to_string(&(&entry.titles, &entry.parts, &entry.artists)).ok()
}

fn decode(stored: &str) -> Option<Entry> {
    let (titles, parts, artists) = serde_json::from_str(stored).ok()?;
    Some(Entry {
        titles,
        parts,
        artists,
    })
}
