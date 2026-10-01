//! Recording a send (ROADMAP 1.9 rule 8): `sync_base` for every field
//! sent, and the mark on each Library track, in one transaction.
//!
//! `sync_base` holds, per Library track and field, the value last sent to
//! rekordbox, so the next read can tell "the app changed it", "rekordbox
//! changed it" and "both did" apart (1.10). A field's name is its
//! rekordbox XML attribute name, and its value is the text written,
//! unescaped: `Location` in the encoded form sent. `TrackID` isn't a
//! field (rekordbox reassigns it on import), and the analysis fields are
//! never sent, so neither is recorded.

use rusqlite::{params, Connection};

use super::SentTrack;

/// Records one send: for each track in `sent`, its `sync_base` becomes
/// exactly the fields sent (a field sent before and not this time is
/// dropped; no value is kept twice), and the track's `last_sent_location`
/// and `last_exported_at` are set. All of it or none: any failure, a
/// track that isn't in the Library included, leaves the database as it
/// was. Tracks that weren't sent aren't touched.
///
/// Call it once per send, after the file is in place.
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
            if mark.execute(params![id, track.location(), now])? != 1 {
                // Dropping the transaction rolls everything back.
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            for (field, value) in track.fields() {
                base.execute(params![id, field, value, now])?;
            }
            let fields: Vec<&str> = track.fields().map(|(field, _)| field).collect();
            let fields = serde_json::to_string(&fields).expect("strings always serialize");
            drop_unsent.execute(params![id, fields])?;
        }
    }
    tx.commit()
}
