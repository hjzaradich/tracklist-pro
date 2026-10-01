//! What rekordbox holds about a track, read from `rekordbox_track` by the
//! track's id: nothing is copied (1aD-4).
//!
//! Only trusted entries are listed: one that relink matched to the track's
//! file and didn't mark probable (see [`super`]).

use rusqlite::Connection;
use serde::Serialize;

use crate::rekordbox::my_tags;
use crate::tags::key;

/// One rekordbox entry attached to a track.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RekordboxEntry {
    /// rekordbox's `TrackID`, valid within the read this came from.
    pub track_id: i64,
    /// The file relink matched it to.
    pub file_id: i64,
    /// Whether this is the entry the track's rekordbox BPM and key were
    /// taken from (see [`super`] for how it's picked).
    pub is_analysis_source: bool,
    pub relink_method: Option<String>,
    pub relink_confidence: Option<f64>,
    pub bpm: Option<f64>,
    /// `Tonality` as rekordbox has it.
    pub tonality: Option<String>,
    /// `Tonality` as Camelot, `None` for no key or one that isn't a key.
    pub key: Option<String>,
    pub play_count: Option<i64>,
    /// `LastPlayed` as exported (a date), empty or absent for never.
    pub last_played: Option<String>,
    pub rating: Option<i64>,
    pub colour: Option<String>,
    /// `Comments` exactly as exported, My Tags block included.
    pub comments: Option<String>,
    /// `Comments` without the My Tags block, for showing.
    pub comment_text: Option<String>,
    pub my_tags: Vec<String>,
    /// The beatgrid, a JSON array of `TEMPO` attribute objects.
    pub tempo: String,
    /// The cues, a JSON array of `POSITION_MARK` attribute objects.
    pub position_marks: String,
    /// The rekordbox playlists holding it, a JSON array of paths.
    pub playlists: String,
}

/// Every trusted rekordbox entry attached to `recording`, the one the
/// track's analysis comes from first, then by `TrackID`.
pub fn for_recording(conn: &Connection, recording: i64) -> rusqlite::Result<Vec<RekordboxEntry>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT rt.track_id, rt.file_id, rt.relink_method, rt.relink_confidence,
                rt.bpm, rt.tonality, rt.play_count,
                json_extract(rt.attributes, '$.LastPlayed'),
                rt.rating, rt.colour, rt.comments, rt.my_tags, rt.tempo,
                rt.position_marks, rt.playlists
         FROM rekordbox_track rt
         LEFT JOIN recording_file rf
                ON rf.file_id = rt.file_id AND rf.recording_id = rt.recording_id
         WHERE rt.recording_id = ?1 AND rt.relink_probable = 0
         ORDER BY {}",
        super::PICK_ORDER
    ))?;
    let rows = stmt.query_map([recording], |r| {
        let tonality: Option<String> = r.get(5)?;
        let comments: Option<String> = r.get(10)?;
        let my_tags: String = r.get(11)?;
        Ok(RekordboxEntry {
            track_id: r.get(0)?,
            file_id: r.get(1)?,
            is_analysis_source: false,
            relink_method: r.get(2)?,
            relink_confidence: r.get(3)?,
            bpm: r.get(4)?,
            key: tonality.as_deref().and_then(key::to_camelot),
            tonality,
            play_count: r.get(6)?,
            last_played: r.get(7)?,
            rating: r.get(8)?,
            colour: r.get(9)?,
            comment_text: comments
                .as_deref()
                .map(|c| my_tags::without_tags(c).to_owned()),
            comments,
            // The column is JSON the reader wrote, always an array.
            my_tags: serde_json::from_str(&my_tags).unwrap_or_default(),
            tempo: r.get(12)?,
            position_marks: r.get(13)?,
            playlists: r.get(14)?,
        })
    })?;
    let mut entries: Vec<RekordboxEntry> = rows.collect::<rusqlite::Result<_>>()?;
    // The first one is the pick, and it's the pick only if it gives a row.
    if let Some(first) = entries.first_mut() {
        first.is_analysis_source = super::bpm_value(first.bpm).is_some() || first.key.is_some();
    }
    Ok(entries)
}
