//! App settings, stored in the `setting` table as one JSON value per key
//! (migration 0005).
//!
//! Reading a setting never fails on its value: a missing row, or a value
//! this version doesn't know (e.g. written by a newer version), reads as the
//! default.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::db::{ReadPool, Writer};
use crate::ipc::IpcError;
use crate::tags::key::KeyNotation;

/// How keys are shown (ROADMAP §1.1). Stored as a JSON string such as
/// `"camelot"` or `"musical_standard"` ([`KeyNotation::as_str`]).
pub const KEY_NOTATION: &str = "key_notation";

/// The key notation the user chose, or Camelot if they haven't.
pub fn read_key_notation(conn: &Connection) -> rusqlite::Result<KeyNotation> {
    // A JSON string, unwrapped; any other JSON value reads as no name.
    let name: Option<Option<String>> = conn
        .query_row(
            "SELECT CASE WHEN json_type(value) = 'text' THEN value ->> '$' END
             FROM setting WHERE key = ?1",
            [KEY_NOTATION],
            |row| row.get(0),
        )
        .optional()?;
    Ok(name
        .flatten()
        .and_then(|name| KeyNotation::from_name(&name))
        .unwrap_or_default())
}

/// Stores the key notation.
pub fn write_key_notation(conn: &Connection, notation: KeyNotation) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, json_quote(?2))
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (KEY_NOTATION, notation.as_str()),
    )?;
    Ok(())
}

/// The key notation the user chose; Camelot until they choose one. The
/// frontend formats keys with it and the generated `KEY_NAMES` table.
// Async, so the database call runs off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn key_notation(reads: State<'_, ReadPool>) -> Result<KeyNotation, IpcError> {
    Ok(reads.read(read_key_notation)?)
}

/// Sets the key notation for the whole app.
#[tauri::command]
#[specta::specta]
pub async fn set_key_notation(
    writer: State<'_, Writer>,
    notation: KeyNotation,
) -> Result<(), IpcError> {
    Ok(writer.call(move |c| write_key_notation(c, notation))?)
}

/// The BPM range the app's own tempo estimates are folded into (ROADMAP
/// 3.7): rekordbox's "BPM range" preference, as the user would set it there.
/// Stored as a JSON array `[min, max]`. No screen sets it yet.
pub const BPM_RANGE: &str = "bpm_range";

/// rekordbox's default range, 98 to 195 BPM.
pub const DEFAULT_BPM_RANGE: BpmRange = BpmRange {
    min: 98.0,
    max: 195.0,
};

/// A tempo range, both ends in it: an estimate is doubled while it's under
/// `min`, then halved while it's over `max`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BpmRange {
    pub min: f64,
    pub max: f64,
}

impl BpmRange {
    /// A range that folding can use: finite, above zero, `min` under `max`.
    pub fn new(min: f64, max: f64) -> Option<BpmRange> {
        (min.is_finite() && max.is_finite() && min > 0.0 && min < max && max < 1000.0)
            .then_some(BpmRange { min, max })
    }
}

/// The BPM range the user set, or rekordbox's default (98 to 195) if they
/// haven't or the value isn't a range.
pub fn read_bpm_range(conn: &Connection) -> rusqlite::Result<BpmRange> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM setting WHERE key = ?1",
            [BPM_RANGE],
            |row| row.get(0),
        )
        .optional()?;
    let range = value
        .and_then(|v| serde_json::from_str::<[f64; 2]>(&v).ok())
        .and_then(|[min, max]| BpmRange::new(min, max));
    Ok(range.unwrap_or(DEFAULT_BPM_RANGE))
}

/// Stores the BPM range. What the tracks show follows it only once their
/// `'local'` rows are folded again, so the app's one caller is
/// [`crate::estimate::set_bpm_range`], which does both.
pub(crate) fn write_bpm_range(conn: &Connection, range: BpmRange) -> rusqlite::Result<()> {
    let value = serde_json::json!([range.min, range.max]).to_string();
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (BPM_RANGE, value),
    )?;
    Ok(())
}

/// An optional feature (ROADMAP §1.1, CLAUDE.md "Network and privacy"): off
/// until the user turns it on. Stored in `setting` as JSON `true` or
/// `false` under `feature_<name>`; no row means off. Like the other
/// settings, a write is not an operation in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Feature {
    /// Wishlists (2.3 to 2.5).
    Wishlist,
    /// Reading rekordbox's library database (2.7).
    RekordboxDatabase,
}

impl Feature {
    fn key(self) -> &'static str {
        match self {
            Feature::Wishlist => "feature_wishlist",
            Feature::RekordboxDatabase => "feature_rekordbox_database",
        }
    }
}

/// Whether the user turned `feature` on. Off for no row, or any value but
/// `true`.
pub fn read_feature_enabled(conn: &Connection, feature: Feature) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT json_type(value) = 'true' FROM setting WHERE key = ?1",
            [feature.key()],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// Turns `feature` on or off.
pub fn write_feature_enabled(
    conn: &Connection,
    feature: Feature,
    on: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, json(?2))
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        (feature.key(), if on { "true" } else { "false" }),
    )?;
    Ok(())
}

/// Whether the user turned an optional feature on.
// Async, so the database call runs off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn feature_enabled(
    reads: State<'_, ReadPool>,
    feature: Feature,
) -> Result<bool, IpcError> {
    Ok(reads.read(move |c| read_feature_enabled(c, feature))?)
}

/// Turns an optional feature on or off.
#[tauri::command]
#[specta::specta]
pub async fn set_feature_enabled(
    writer: State<'_, Writer>,
    feature: Feature,
    on: bool,
) -> Result<(), IpcError> {
    Ok(writer.call(move |c| write_feature_enabled(c, feature, on))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// A migrated database in a temp dir, with its writer and read pool.
    fn db() -> (tempfile::TempDir, Writer, ReadPool) {
        let dir = tempfile::tempdir().unwrap();
        let db_file = crate::db::db_path(dir.path());
        let db_file = db_file.file_name().unwrap().to_str().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(dir.path(), db_file)).unwrap();
        let reads = ReadPool::open(writer.guarded_path()).unwrap();
        (dir, writer, reads)
    }

    fn stored(writer: &Writer) -> Option<String> {
        writer
            .call(|c| {
                c.query_row(
                    "SELECT value FROM setting WHERE key = ?1",
                    [KEY_NOTATION],
                    |r| r.get(0),
                )
                .optional()
            })
            .unwrap()
    }

    #[test]
    fn a_new_database_shows_keys_in_camelot() {
        let (_dir, _writer, reads) = db();
        assert_eq!(reads.read(read_key_notation).unwrap(), KeyNotation::Camelot);
    }

    #[test]
    fn each_notation_is_stored_as_its_json_name_and_reads_back() {
        let (_dir, writer, reads) = db();
        for notation in KeyNotation::ALL {
            writer
                .call(move |c| write_key_notation(c, notation))
                .unwrap();
            assert_eq!(reads.read(read_key_notation).unwrap(), notation);
            assert_eq!(
                stored(&writer),
                Some(format!("\"{}\"", notation.as_str())),
                "{notation:?}"
            );
        }
    }

    #[test]
    fn changing_the_notation_keeps_one_row_and_updates_its_time() {
        let (_dir, writer, _reads) = db();
        writer
            .call(|c| {
                write_key_notation(c, KeyNotation::MusicalFlats)?;
                c.execute(
                    "UPDATE setting SET updated_at = '2000-01-01T00:00:00.000Z'",
                    [],
                )?;
                write_key_notation(c, KeyNotation::MusicalSharps)
            })
            .unwrap();
        let (rows, updated): (i64, String) = writer
            .call(|c| {
                c.query_row(
                    "SELECT count(*), max(updated_at) FROM setting WHERE key = ?1",
                    [KEY_NOTATION],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .unwrap();
        assert_eq!(rows, 1);
        assert!(updated.as_str() > "2000-01-01T00:00:00.000Z", "{updated}");
    }

    #[test]
    fn an_unknown_or_malformed_stored_value_reads_as_camelot_instead_of_failing() {
        let (_dir, writer, reads) = db();
        for value in [r#""open_key""#, r#""musical""#, "42", "null", r#"{"a":1}"#] {
            writer
                .call(move |c| {
                    c.execute(
                        "INSERT OR REPLACE INTO setting (key, value) VALUES (?1, ?2)",
                        (KEY_NOTATION, value),
                    )
                })
                .unwrap();
            assert_eq!(
                reads.read(read_key_notation).unwrap(),
                KeyNotation::Camelot,
                "{value}"
            );
        }
    }

    // Through the real app: its startup (DB, state) and its command list,
    // on Tauri's mock runtime, called the way the frontend calls them.
    use crate::ipc::testing::{app, invoke};

    #[test]
    fn the_frontend_gets_camelot_before_any_notation_is_chosen() {
        let (_data, app) = app();
        assert_eq!(
            invoke(&app, "key_notation", json!({})),
            Ok(json!("camelot"))
        );
    }

    #[test]
    fn the_frontend_can_set_each_notation_by_name_and_read_it_back() {
        let (_data, app) = app();
        for notation in KeyNotation::ALL {
            let name = notation.as_str();
            assert_eq!(
                invoke(&app, "set_key_notation", json!({ "notation": name })),
                Ok(Value::Null)
            );
            assert_eq!(invoke(&app, "key_notation", json!({})), Ok(json!(name)));
        }
    }

    #[test]
    fn setting_an_unknown_notation_is_refused_and_keeps_the_current_one() {
        let (_data, app) = app();
        invoke(
            &app,
            "set_key_notation",
            json!({ "notation": "musical_flats" }),
        )
        .unwrap();
        assert!(invoke(&app, "set_key_notation", json!({ "notation": "open_key" })).is_err());
        assert!(invoke(&app, "set_key_notation", json!({})).is_err());
        assert_eq!(
            invoke(&app, "key_notation", json!({})),
            Ok(json!("musical_flats"))
        );
    }

    #[test]
    fn the_bpm_range_is_rekordboxs_default_until_set_and_reads_back_once_set() {
        let (_dir, writer, _reads) = db();
        let read = |w: &Writer| w.call(|c| read_bpm_range(c)).unwrap();
        assert_eq!(read(&writer), DEFAULT_BPM_RANGE);
        assert_eq!(
            (DEFAULT_BPM_RANGE.min, DEFAULT_BPM_RANGE.max),
            (98.0, 195.0)
        );
        let range = BpmRange::new(70.0, 140.0).unwrap();
        writer.call(move |c| write_bpm_range(c, range)).unwrap();
        assert_eq!(read(&writer), range);
        let raw: String = writer
            .call(|c| {
                c.query_row(
                    "SELECT value FROM setting WHERE key = 'bpm_range'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap(),
            json!([70.0, 140.0])
        );
    }

    #[test]
    fn a_stored_bpm_range_that_is_not_a_range_reads_as_the_default() {
        let (_dir, writer, _reads) = db();
        for bad in [
            "[140, 70]",
            "[0, 100]",
            "\"fast\"",
            "[98]",
            "[98, 195, 3]",
            "{}",
        ] {
            writer
                .call(move |c| {
                    c.execute(
                        "INSERT INTO setting (key, value) VALUES ('bpm_range', ?1)
                         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                        [bad],
                    )
                })
                .unwrap();
            assert_eq!(
                writer.call(|c| read_bpm_range(c)).unwrap(),
                DEFAULT_BPM_RANGE,
                "{bad}"
            );
        }
    }
}
