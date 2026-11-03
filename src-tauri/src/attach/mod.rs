//! Attaching rekordbox data to tracks (1aD-4; ROADMAP 1.2, §2).
//!
//! Relink matches each rekordbox entry to a file, and grouping keeps
//! `rekordbox_track.recording_id` pointing at that file's track. Attach is
//! the step after both: it writes what rekordbox knows about a track's
//! BPM and key into `analysis` with `source = 'rekordbox'`, so the
//! track's displayed values can say where they came from (§2 precedence).
//! Everything else rekordbox holds (grid, cues, play count, last played,
//! rating, colour, comments, My Tags, playlists) stays in
//! `rekordbox_track` and is read per track through [`for_recording`],
//! never copied.
//!
//! - **Trusted matches only.** An entry with `relink_probable = 1`, or
//!   with no track, contributes nothing: a wrong match would put one
//!   track's values on another's file. When a row turns probable, goes
//!   unmatched or lands on another track, the next run removes what it
//!   contributed. Every run decides every track again.
//! - **One rekordbox row per track.** Several entries can point at one
//!   track (a track imported twice into rekordbox). The row comes from the
//!   entry the track shows, the app's one rule for it ([`SHOWN_FROM`]): for
//!   a Library track, the entry the send uses. The other entries stay in
//!   `rekordbox_track` for Review (1b).
//! - **Running again with nothing changed changes nothing:** a row that
//!   already holds the values is left alone, `analyzed_at` included.
//! - Only the database is touched, and only `analysis` rows from
//!   rekordbox. It runs as a background job ([`Attacher`]) asked for
//!   through [`request`] after every grouping run and after every relink
//!   run, and wherever a Library track's link or the Library itself changes
//!   (Review, Undo, a removal) or a file's place among its track's files
//!   does (the roles job), since those change which entry a track shows.
//!   Startup asks for it once after an upgrade that changed that rule
//!   ([`SHOWN_VERSION`]). Each run asks for the search index after it.

mod data;
mod job;

pub use data::{for_recording, RekordboxEntry};
pub use job::{attach_job, attacher, request, Attacher};

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::tags::key;

/// The rekordbox entry a track shows (1cD-X2, audit C12; ROADMAP 1.3): the
/// one rule for every place that picks one entry for a track. Attach (the
/// table's BPM and key), the track's rekordbox panel ([`for_recording`]),
/// the Overview's genre and gaps and the removal list all use it.
///
/// - **Trusted matches only** (`relink_probable = 0`).
/// - **A Library track shows the entry the send uses** for it
///   (`send_values`, step 4): of the entries on its linked file, the most
///   played, then the lowest `TrackID`. With none there it shows none, and
///   its files' own values stand, as the send makes a new track of the
///   file. An entry on another of its files is not its own, so nothing is
///   combined from two entries (foreman's ruling, 2026-10-16). The send's
///   two last resorts for a gone linked file (rekordbox's entry at the
///   file's own Location, a byte-identical copy's entry) are the send's
///   alone: which one applies depends on the drives plugged in when it runs.
/// - **A track outside the Library shows the entry on the file adding it
///   would link** ([`crate::library::linked_file_for`]: of the files with a
///   trusted entry, the one on disk, then its best file, then the lowest
///   file id), and of that file's entries the most played, then the lowest
///   `TrackID`. So adding a track never changes the entry it shows.
///
/// [`SHOWN_FROM`] names the entry `se`, its file's `recording_file` row
/// `se_rf` (`se_rf.recording_id` is the track), its file `se_f` and the
/// track's Library track `se_lt` (if any); [`SHOWN_WHERE`] keeps the
/// entries the track may show; [`SHOWN_ORDER`] puts the one it shows first
/// within a track. [`shown_entry_sql`] is the three as one expression.
pub(crate) const SHOWN_FROM: &str = "rekordbox_track se
         JOIN recording_file se_rf ON se_rf.file_id = se.file_id
         JOIN file se_f ON se_f.id = se.file_id
         LEFT JOIN library_track se_lt ON se_lt.recording_id = se_rf.recording_id";

/// Which entries a track may show (see [`SHOWN_FROM`]).
pub(crate) const SHOWN_WHERE: &str = "se.relink_probable = 0
         AND (se_lt.id IS NULL OR se.file_id = se_lt.linked_file_id)";

/// The entry a track shows first (see [`SHOWN_FROM`]). For a Library track
/// every entry left is on its linked file, so only the last two keys count.
pub(crate) const SHOWN_ORDER: &str = "se_f.present DESC, (se_rf.role = 'best') DESC, se_f.id,
         COALESCE(se.play_count, 0) DESC, se.track_id";

/// The `rekordbox_track.id` of the entry track `recording` (an SQL
/// expression) shows, `NULL` for none: [`SHOWN_FROM`] as one expression.
pub(crate) fn shown_entry_sql(recording: &str) -> String {
    format!(
        "(SELECT se.id FROM {SHOWN_FROM}
          WHERE se_rf.recording_id = {recording} AND {SHOWN_WHERE}
          ORDER BY {SHOWN_ORDER} LIMIT 1)"
    )
}

/// The version of the rule for the entry a track shows ([`SHOWN_FROM`]).
/// Bump it when the rule changes: the first attach run of a build with
/// another version marks every Library track due for the index (the genre
/// and the names the index stores come from that entry), and startup asks
/// for that run. 1: the send's rule (1cD-X2).
pub const SHOWN_VERSION: u32 = 1;

/// The setting that remembers which [`SHOWN_VERSION`] the stored values are
/// of.
const VERSION_KEY: &str = "attach_shown_version";

/// Whether the stored values are of this build's [`SHOWN_VERSION`].
/// Read-only.
pub(crate) fn version_is_current(conn: &Connection) -> rusqlite::Result<bool> {
    use rusqlite::OptionalExtension;
    let stored: Option<i64> = conn
        .query_row(
            "SELECT json_extract(value, '$') FROM setting WHERE key = ?1",
            [VERSION_KEY],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(stored == Some(i64::from(SHOWN_VERSION)))
}

/// What one attach run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Tracks that got a rekordbox row.
    pub added: u64,
    /// Rows whose values changed.
    pub updated: u64,
    /// Rows removed: their track no longer has a trusted entry (or the
    /// entry has no BPM or key).
    pub removed: u64,
}

/// The values one rekordbox row gives a track.
#[derive(Debug, Clone, PartialEq)]
struct Values {
    bpm: Option<f64>,
    key: Option<String>,
}

/// One rekordbox entry attached to a track, as far as picking goes.
struct Candidate {
    recording: i64,
    values: Values,
}

/// The BPM rekordbox gives, if it's one: unanalysed tracks have `0`.
fn bpm_value(raw: Option<f64>) -> Option<f64> {
    raw.filter(|b| b.is_finite() && *b > 0.0 && *b < 1000.0)
}

/// The Camelot key for rekordbox's `Tonality`, through the key module, so
/// the spellings are the app's one set (`F#` and `Gb` are 2B, `Bb` is 6B).
fn key_value(tonality: Option<&str>) -> Option<String> {
    key::to_camelot(tonality?)
}

/// The entry each track takes its rekordbox row from (see the module
/// docs), with the values it gives. Tracks whose entry has neither a BPM
/// nor a key are left out: a row must say something.
fn picks(conn: &Connection) -> rusqlite::Result<Vec<Candidate>> {
    // The entry each track shows first within the track.
    let mut stmt = conn.prepare(&format!(
        "SELECT se_rf.recording_id, se.bpm, se.tonality
         FROM {SHOWN_FROM}
         WHERE {SHOWN_WHERE}
         ORDER BY se_rf.recording_id, {SHOWN_ORDER}"
    ))?;
    let mut rows = stmt.query([])?;
    let mut picked = Vec::new();
    let mut last = None;
    while let Some(r) = rows.next()? {
        let recording: i64 = r.get(0)?;
        if last == Some(recording) {
            continue;
        }
        last = Some(recording);
        let bpm: Option<f64> = r.get(1)?;
        let tonality: Option<String> = r.get(2)?;
        let values = Values {
            bpm: bpm_value(bpm),
            key: key_value(tonality.as_deref()),
        };
        if values.bpm.is_some() || values.key.is_some() {
            picked.push(Candidate { recording, values });
        }
    }
    Ok(picked)
}

/// Brings every track's rekordbox `analysis` row in line with the
/// snapshot, in one transaction. Safe to run again at any time.
pub fn attach(conn: &mut Connection) -> rusqlite::Result<Summary> {
    let tx = conn.transaction()?;
    let mut summary = Summary::default();

    let existing: HashMap<i64, (i64, Values)> = {
        let mut stmt = tx.prepare(
            "SELECT id, recording_id, bpm, key FROM analysis WHERE source = 'rekordbox'",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(1)?,
                (
                    r.get::<_, i64>(0)?,
                    Values {
                        bpm: r.get(2)?,
                        key: r.get(3)?,
                    },
                ),
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    let wanted = picks(&tx)?;
    {
        let mut insert = tx.prepare_cached(
            "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, 'rekordbox', ?2, ?3)",
        )?;
        let mut update = tx.prepare_cached(
            "UPDATE analysis SET bpm = ?2, key = ?3,
                    analyzed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
        )?;
        for c in &wanted {
            match existing.get(&c.recording) {
                None => {
                    insert.execute(params![c.recording, c.values.bpm, &c.values.key])?;
                    summary.added += 1;
                }
                Some((id, have)) if *have != c.values => {
                    update.execute(params![id, c.values.bpm, &c.values.key])?;
                    summary.updated += 1;
                }
                Some(_) => {}
            }
        }
    }

    let keep: std::collections::HashSet<i64> = wanted.iter().map(|c| c.recording).collect();
    {
        let mut remove = tx.prepare_cached("DELETE FROM analysis WHERE id = ?1")?;
        for (recording, (id, _)) in &existing {
            if !keep.contains(recording) {
                remove.execute([id])?;
                summary.removed += 1;
            }
        }
    }
    // A build with a new rule for the entry a track shows: every Library
    // track's stored values are worked out again (`-1` stands for all of
    // them), once, if rekordbox has given anything.
    if !version_is_current(&tx)? {
        tx.execute(
            "INSERT OR IGNORE INTO search_due (recording_id)
             SELECT -1 WHERE EXISTS (SELECT 1 FROM rekordbox_track)",
            [],
        )?;
        tx.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params![VERSION_KEY, SHOWN_VERSION.to_string()],
        )?;
    }
    tx.commit()?;
    Ok(summary)
}

/// Whether there is anything for [`attach`] to do or undo: a rekordbox
/// entry, or a rekordbox row on a track.
pub fn any_to_attach(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM rekordbox_track)
             OR EXISTS (SELECT 1 FROM analysis WHERE source = 'rekordbox')",
        [],
        |r| r.get(0),
    )
}

#[cfg(test)]
mod tests;
