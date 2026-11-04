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
//!   entry the track shows, the app's one rule for it ([`shown_entry_sql`]): for
//!   a Library track, the entry the send uses (bar its entry at a gone
//!   file's own Location). The other entries stay in
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

/// The `rekordbox_track.id` of the rekordbox entry track `recording` (an
/// SQL expression) shows, `NULL` for none (1cD-X2, audit C12; ROADMAP §2
/// `analysis`): the one rule for every place that picks one entry for a
/// track. Attach (the table's BPM and key), the track's rekordbox panel
/// ([`for_recording`]), the Overview's genre and gaps and the removal list
/// all use it.
///
/// - **Trusted matches only** (`relink_probable = 0`).
/// - **A Library track shows the entry the send uses** for it
///   (`send_values`, `Loaded::source`): of the entries on its linked file,
///   the most played, then the lowest `TrackID`; with none there, a
///   byte-identical copy's entry ([`copy_entry_sql`], 1bE-11), whether the
///   linked file is on disk or gone. With neither it shows none, and its
///   files' own values stand, as the send makes a new track of the file.
///   An entry on another of its files is otherwise not its own, so nothing
///   is combined from two entries (foreman's ruling, 2026-10-16). One last
///   resort is the send's alone: rekordbox's entry at a gone linked file's
///   own `Location` (1bE-10), which depends on the drives plugged in when
///   the send runs.
/// - **A track outside the Library shows the entry on the file adding it
///   would link** ([`crate::library::linked_file_for`]: of the files with a
///   trusted entry, the one on disk, then its best file, then the lowest
///   file id), and of that file's entries the most played, then the lowest
///   `TrackID`. So adding a track never changes the entry it shows.
pub(crate) fn shown_entry_sql(recording: &str) -> String {
    format!(
        "COALESCE((SELECT se.id FROM {SHOWN_FROM}
                   WHERE se_rf.recording_id = {recording} AND {SHOWN_WHERE}
                   ORDER BY {SHOWN_ORDER} LIMIT 1),
                  {copy})",
        copy = copy_entry_sql(recording)
    )
}

/// The entries on a track's own files: `se`, its file's `recording_file`
/// row `se_rf` (`se_rf.recording_id` is the track), its file `se_f` and the
/// track's Library track `se_lt` (if any).
const SHOWN_FROM: &str = "rekordbox_track se
         JOIN recording_file se_rf ON se_rf.file_id = se.file_id
         JOIN file se_f ON se_f.id = se.file_id
         LEFT JOIN library_track se_lt ON se_lt.recording_id = se_rf.recording_id";

/// Which of them a track may show first: for a Library track, the ones on
/// its linked file.
const SHOWN_WHERE: &str = "se.relink_probable = 0
         AND (se_lt.id IS NULL OR se.file_id = se_lt.linked_file_id)";

/// The one it shows. For a Library track every entry left is on its linked
/// file, so only the last two keys count.
const SHOWN_ORDER: &str = "se_f.present DESC, (se_rf.role = 'best') DESC, se_f.id,
         COALESCE(se.play_count, 0) DESC, se.track_id";

/// The entry of a byte-identical copy of Library track `recording`'s linked
/// file, as the send takes it (`send_values::entries_of_copies`, 1bE-11): a
/// file of the track on disk, other than the linked file, with the same
/// `blake3` and the hash stage current for both; exactly one
/// `rekordbox_track` row matched to it, trusted; exactly one such copy in
/// the track; and no Library track linked to it. `NULL` otherwise, and for
/// a track outside the Library. The send's last condition (no Library track
/// sent as that entry from its gone file's own `Location`) needs the
/// drives, so it is left out: it only ever narrows the send's answer.
fn copy_entry_sql(recording: &str) -> String {
    let version = crate::relink::hash_version().to_string();
    format!(
        "(SELECT CASE WHEN count(*) = 1
                       AND max(EXISTS (SELECT 1 FROM library_track co
                                       WHERE co.linked_file_id = cc.id)) = 0
                      THEN max(ce.id) END
          FROM library_track cl
          JOIN file cf ON cf.id = cl.linked_file_id
          JOIN recording_file crf ON crf.recording_id = cl.recording_id
          JOIN file cc ON cc.id = crf.file_id
          JOIN rekordbox_track ce ON ce.file_id = cc.id AND ce.relink_probable = 0
          WHERE cl.recording_id = {recording}
            AND cc.id <> cf.id AND cc.present = 1
            AND length(cf.blake3) > 0 AND cc.blake3 = cf.blake3
            AND {linked_current} AND {copy_current}
            AND (SELECT count(*) FROM rekordbox_track cx WHERE cx.file_id = cc.id) = 1)",
        linked_current = crate::relink::hash_current_of("cf").replace("?1", &version),
        copy_current = crate::relink::hash_current_of("cc").replace("?1", &version),
    )
}

/// The version of the rule for the entry a track shows ([`shown_entry_sql`]).
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
    // The entry each track that rekordbox has a row for shows.
    let mut stmt = conn.prepare(&format!(
        "SELECT t.recording_id, se.bpm, se.tonality
         FROM (SELECT DISTINCT rf.recording_id FROM rekordbox_track rt
               JOIN recording_file rf ON rf.file_id = rt.file_id) t
         JOIN rekordbox_track se ON se.id = {}
         ORDER BY t.recording_id",
        shown_entry_sql("t.recording_id")
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
    // A copy's entry (1bE-11) turns on the files' hashes, whose changes
    // mark nothing due for the index: a Library track with no entry on its
    // linked file and a trusted one on another of its files is worked out
    // again after every run (a row that comes out the same costs a read).
    tx.execute(
        "INSERT OR IGNORE INTO search_due (recording_id)
         SELECT lt.recording_id FROM library_track lt
         WHERE NOT EXISTS (SELECT 1 FROM rekordbox_track x
                           WHERE x.file_id = lt.linked_file_id AND x.relink_probable = 0)
           AND EXISTS (SELECT 1 FROM recording_file rf
                       JOIN rekordbox_track y ON y.file_id = rf.file_id
                       WHERE rf.recording_id = lt.recording_id
                         AND rf.file_id IS NOT lt.linked_file_id AND y.relink_probable = 0)",
        [],
    )?;
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
