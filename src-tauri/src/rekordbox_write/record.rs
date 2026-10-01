//! Recording a send (ROADMAP 1.9 rule 8): `sync_base` for every field
//! sent, and the mark on each Library track, in one transaction.
//!
//! `sync_base` holds, per Library track and field, the value last sent to
//! rekordbox, so the next read can tell "the app changed it", "rekordbox
//! changed it" and "both did" apart (1.10). A field's name is its
//! rekordbox XML attribute name, and its value is the text written,
//! unescaped: `Location` in the encoded form sent. `TrackID` isn't a
//! field (rekordbox reassigns it on import) and isn't recorded. Nor are
//! the analysis fields, whatever the caller hands in: they're rekordbox's,
//! the app never edits them, and they can never conflict (1.10), so a
//! base for one has no use, even when Phase 2 sends them (rule 3, case B).
//!
//! A send isn't an operation in the undo log. Its rows can take the row
//! ids a removed track's bases had, and undoing that removal is then
//! refused (nothing is changed; the track can be added back by hand).

use rusqlite::{params, Connection};

use super::{SentTrack, ANALYSIS_ATTRIBUTES};

/// rekordbox's analysis elements, named as `conflict.field` names them.
const ANALYSIS_ELEMENTS: [&str; 2] = ["TEMPO", "POSITION_MARK"];

/// Whether a field is rekordbox's analysis, which never gets a base.
fn is_analysis(field: &str) -> bool {
    ANALYSIS_ATTRIBUTES.contains(&field) || ANALYSIS_ELEMENTS.contains(&field)
}

/// Records one send: for each track in `sent`, its `sync_base` becomes
/// exactly the fields sent (a field sent before and not this time is
/// dropped; no value is kept twice), and the track's `last_sent_location`
/// and `last_exported_at` are set. All of it or none: any failure, a
/// track that isn't in the Library included, leaves the database as it
/// was. Tracks that weren't sent aren't touched. An analysis field is
/// never recorded, even if `sent` carries one.
///
/// Call it once per send, after the file is in place. Calling it again
/// with the same send is safe (it replaces, never duplicates), so if it
/// fails once the file is in place, call it again: until it succeeds the
/// bases are the previous send's, and what this send changed would look
/// like rekordbox's edits.
pub fn record_send(conn: &mut Connection, sent: &[SentTrack]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let now: String = tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
        r.get(0)
    })?;
    {
        let mut mark = tx.prepare(
            "UPDATE library_track SET last_sent_location = ?2, last_exported_at = ?3
             WHERE id = ?1",
        )?;
        // A field sent again keeps its row, so nothing that refers to the
        // row by id is left pointing at nothing.
        let mut base = tx.prepare(
            "INSERT INTO sync_base (library_track_id, field, value, synced_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (library_track_id, field)
             DO UPDATE SET value = excluded.value, synced_at = excluded.synced_at",
        )?;
        let mut drop_unsent = tx.prepare(
            "DELETE FROM sync_base
             WHERE library_track_id = ?1
               AND field NOT IN (SELECT value FROM json_each(?2))",
        )?;
        for track in sent {
            let id = track.library_track.0;
            let marked = mark.execute(params![id, track.location(), now])?;
            if marked != 1 {
                // Not a Library track. Dropping the transaction rolls
                // everything back.
                return Err(rusqlite::Error::StatementChangedRows(marked));
            }
            let fields: Vec<(&str, &str)> = track
                .fields()
                .filter(|(field, _)| !is_analysis(field))
                .collect();
            for (field, value) in &fields {
                base.execute(params![id, field, value, now])?;
            }
            let fields: Vec<&str> = fields.iter().map(|(field, _)| *field).collect();
            let fields = serde_json::to_string(&fields).expect("strings always serialize");
            drop_unsent.execute(params![id, fields])?;
        }
    }
    tx.commit()
}
