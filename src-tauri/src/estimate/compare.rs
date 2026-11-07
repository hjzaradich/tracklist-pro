//! The owner's test of the local key (ROADMAP 3.7, "Key"): how the app's own
//! key agrees with rekordbox's, on the Library tracks that have both.
//!
//! Each pair is counted once, as the nearest of: the same key; the relative
//! key (8A and 8B, same Camelot number); one step round the wheel (8A and 7A
//! or 9A); the parallel key (A minor and A major); or anything else. Counted
//! over the whole Library and per genre node (the node the track's genre
//! resolves to; tracks with none are counted together). Numbers only: no
//! track is named.
//!
//! Run by the owner in the app, on their own tracks; never in CI, which
//! tests it on a made-up Library. Like everything that reads the local key,
//! it answers only while [`Feature::LocalKey`] is on.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use tauri::State;

use super::ledger::key_of;
use super::VERSION;
use crate::db::ReadPool;
use crate::ipc::IpcError;
use crate::settings::{read_feature_enabled, Feature};
use crate::tags::key::{self, MusicalKey};

/// How many pairs fell in each kind of agreement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyAgreement {
    pub tracks: u32,
    pub exact: u32,
    pub relative: u32,
    pub one_step: u32,
    pub parallel: u32,
    pub other: u32,
}

/// The counts for one genre node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenreAgreement {
    /// The node's name; `None` for the tracks whose genre resolves to none.
    pub genre: Option<String>,
    pub counts: KeyAgreement,
}

/// The whole comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyComparison {
    pub overall: KeyAgreement,
    /// By node name; the tracks with no node last.
    pub genres: Vec<GenreAgreement>,
}

/// How two keys relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreement {
    Exact,
    Relative,
    OneStep,
    Parallel,
    Other,
}

/// The Camelot number (1 to 12) and whether it's minor (A).
fn camelot(key: MusicalKey) -> (u8, bool) {
    let name = key.to_camelot();
    let (number, letter) = name.split_at(name.len() - 1);
    (number.parse().unwrap_or(0), letter == "A")
}

/// How `ours` relates to `theirs`.
pub fn agreement(ours: MusicalKey, theirs: MusicalKey) -> Agreement {
    let ((a, a_minor), (b, b_minor)) = (camelot(ours), camelot(theirs));
    if ours == theirs {
        Agreement::Exact
    } else if a == b {
        Agreement::Relative
    } else if a_minor == b_minor && (a % 12 + 1 == b || b % 12 + 1 == a) {
        Agreement::OneStep
    } else if ours.pitch_class == theirs.pitch_class {
        Agreement::Parallel
    } else {
        Agreement::Other
    }
}

impl KeyAgreement {
    fn add(&mut self, agreement: Agreement) {
        self.tracks += 1;
        match agreement {
            Agreement::Exact => self.exact += 1,
            Agreement::Relative => self.relative += 1,
            Agreement::OneStep => self.one_step += 1,
            Agreement::Parallel => self.parallel += 1,
            Agreement::Other => self.other += 1,
        }
    }
}

/// The comparison, or `None` while [`Feature::LocalKey`] is off.
pub fn compare(conn: &Connection) -> rusqlite::Result<Option<KeyComparison>> {
    if !read_feature_enabled(conn, Feature::LocalKey)? {
        return Ok(None);
    }
    let shown = crate::search::shown_file_sql("lt.linked_file_id", "lt.recording_id");
    let mut stmt = conn.prepare(&format!(
        "SELECT g.name, a.key, e.key_pitch_class, e.key_mode
         FROM library_track lt
         JOIN file f ON f.id = {shown}
         JOIN file_estimate e ON e.file_id = f.id AND e.audio_hash = f.audio_hash
                             AND e.method_version = {version}
         JOIN analysis a ON a.recording_id = lt.recording_id AND a.source = 'rekordbox'
         LEFT JOIN track_row tr ON tr.recording_id = lt.recording_id
         LEFT JOIN genre g ON g.id = tr.genre_node
         WHERE a.key IS NOT NULL AND e.key_pitch_class IS NOT NULL",
        version = i64::from(VERSION),
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut overall = KeyAgreement::default();
    let mut named: BTreeMap<String, KeyAgreement> = BTreeMap::new();
    let mut none = KeyAgreement::default();
    for row in rows {
        let (genre, theirs, pitch_class, mode) = row?;
        let (Some(theirs), Some(ours)) =
            (key::parse(&theirs), key_of(pitch_class, mode.as_deref()))
        else {
            continue;
        };
        let agreement = agreement(ours, theirs);
        overall.add(agreement);
        match genre {
            Some(name) => named.entry(name).or_default().add(agreement),
            None => none.add(agreement),
        }
    }
    let mut genres: Vec<GenreAgreement> = named
        .into_iter()
        .map(|(name, counts)| GenreAgreement {
            genre: Some(name),
            counts,
        })
        .collect();
    if none.tracks > 0 {
        genres.push(GenreAgreement {
            genre: None,
            counts: none,
        });
    }
    Ok(Some(KeyComparison { overall, genres }))
}

/// How the app's own key agrees with rekordbox's, for the owner's test;
/// `None` while the local key is off.
// Async, so the database call runs off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn compare_local_keys(
    reads: State<'_, ReadPool>,
) -> Result<Option<KeyComparison>, IpcError> {
    Ok(reads.read(compare)?)
}
