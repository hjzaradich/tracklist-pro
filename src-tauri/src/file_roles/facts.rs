//! Reading what the ranking needs about the files of some tracks out of the
//! database, one query for all of them (plus the stored verdicts).
//!
//! Every input is the stored fact, never the live state of the machine:
//! whether a drive is plugged in is not read ([`Place::volume_online`] is
//! always `true` here), so the roles are the same with the drive in or out.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use serde_json::Value;

use crate::best_file::{
    Art, FileFacts, FolderRole, Format, Place, Quality, RekordboxLink, VolumeKind,
};
use crate::quality::store::codec_of;
use crate::send_values::{plays_in_dj_app, tag_values, TAG_ATTRIBUTES};
use crate::tags::TagFields;

/// One file of a track, as the ranking sees it, and the role it has now.
pub(super) struct Loaded {
    pub facts: FileFacts,
    pub role: String,
}

/// The files of each of `recordings`, by track. A track with no files is
/// not in the map. Leaves the stored verdicts as they are: whoever calls
/// settles them first.
pub(super) fn load(
    conn: &Connection,
    recordings: &[i64],
) -> rusqlite::Result<BTreeMap<i64, Vec<Loaded>>> {
    let wanted = serde_json::to_string(recordings).unwrap_or_else(|_| "[]".into());
    let mut rows = Vec::new();
    {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT rf.recording_id, rf.role, f.id, f.present, f.sniffed_format, f.codec,
                    f.bitrate, f.raw_tags, mf.id, mf.role, v.kind,
                    CASE WHEN q.audio_hash IS f.audio_hash
                              AND q.method_version = {version} AND q.failure IS NULL
                         THEN q.cutoff_hz END,
                    CASE WHEN EXISTS (SELECT 1 FROM rekordbox_track rt
                                      WHERE rt.file_id = f.id AND rt.relink_probable = 0) THEN 2
                         WHEN EXISTS (SELECT 1 FROM rekordbox_track rt
                                      WHERE rt.file_id = f.id AND rt.relink_probable = 1) THEN 1
                         ELSE 0 END
             FROM recording_file rf
             JOIN file f ON f.id = rf.file_id
             JOIN music_folder mf ON mf.id = f.music_folder_id
             JOIN volume v ON v.id = mf.volume_id
             LEFT JOIN file_quality q ON q.file_id = f.id
             WHERE rf.recording_id IN (SELECT value FROM json_each(?1))
             ORDER BY rf.recording_id, f.id",
            version = crate::quality::VERSION
        ))?;
        let mapped = stmt.query_map([wanted], |r| {
            Ok(Row {
                recording: r.get(0)?,
                role: r.get(1)?,
                file: r.get(2)?,
                present: r.get(3)?,
                sniffed_format: r.get(4)?,
                codec: r.get(5)?,
                bitrate: r.get(6)?,
                raw_tags: r.get(7)?,
                folder: r.get(8)?,
                folder_role: r.get(9)?,
                volume_kind: r.get(10)?,
                cutoff_hz: r.get(11)?,
                rekordbox: r.get(12)?,
            })
        })?;
        for row in mapped {
            rows.push(row?);
        }
    }
    let file_ids: Vec<i64> = rows.iter().map(|r| r.file).collect();
    let verdicts = crate::quality::stored_verdicts(conn, &file_ids)?;
    let mut by_recording: BTreeMap<i64, Vec<Loaded>> = BTreeMap::new();
    for row in rows {
        let assessment = verdicts.get(&row.file);
        let quality = match assessment {
            Some(assessment) => Quality::from_assessment(
                assessment,
                row.cutoff_hz.and_then(|hz| u32::try_from(hz).ok()),
            ),
            None => Quality::NOT_CHECKED,
        };
        let (tags, art) = match row.raw_tags.as_deref() {
            Some(raw) => (tags_of(raw), art_of(raw)),
            None => (TagFields::default(), None),
        };
        let facts = FileFacts {
            file_id: row.file,
            on_disk: row.present,
            plays_in_dj_app: row.sniffed_format.as_deref().is_some_and(plays_in_dj_app),
            format: Format {
                codec: codec_of(row.codec.as_deref()),
                kbps: row.bitrate.and_then(|k| u32::try_from(k).ok()),
            },
            quality,
            tags,
            art,
            rekordbox: match row.rekordbox {
                2 => RekordboxLink::Trusted,
                1 => RekordboxLink::Probable,
                _ => RekordboxLink::Unknown,
            },
            place: Place {
                // Not whether the drive is plugged in now: that would move
                // the best file whenever a drive is, which the roles must
                // not do.
                volume_online: true,
                folder_role: match row.folder_role.as_str() {
                    "inbox" => FolderRole::Inbox,
                    _ => FolderRole::Scan,
                },
                volume_kind: match row.volume_kind.as_str() {
                    "internal" => VolumeKind::Internal,
                    "network" => VolumeKind::Network,
                    _ => VolumeKind::External,
                },
                folder_order: row.folder,
            },
        };
        by_recording.entry(row.recording).or_default().push(Loaded {
            facts,
            role: row.role,
        });
    }
    Ok(by_recording)
}

struct Row {
    recording: i64,
    role: String,
    file: i64,
    present: bool,
    sniffed_format: Option<String>,
    codec: Option<String>,
    bitrate: Option<i64>,
    raw_tags: Option<String>,
    folder: i64,
    folder_role: String,
    volume_kind: String,
    cutoff_hz: Option<i64>,
    rekordbox: i64,
}

/// The descriptive tags a file's `raw_tags` give. Album artist and ISRC
/// aren't mapped yet (they stay `None`).
pub(super) fn tags_of(raw_tags: &str) -> TagFields {
    let mut tags = TagFields::default();
    let number = |text: &str| text.trim().parse::<i64>().ok();
    for found in tag_values(raw_tags, &TAG_ATTRIBUTES) {
        let text = Some(found.value.clone());
        match found.attribute {
            "Name" => tags.title = text,
            "Artist" => tags.artist = text,
            "Album" => tags.album = text,
            "Genre" => tags.genre = text,
            "Comments" => tags.comment = text,
            "Composer" => tags.composer = text,
            "Remixer" => tags.remixer = text,
            "Label" => tags.label = text,
            "Year" => tags.year = number(&found.value),
            "TrackNumber" => tags.track_no = number(&found.value),
            "DiscNumber" => tags.disc_no = number(&found.value),
            _ => {}
        }
    }
    tags
}

/// The largest embedded picture's size in bytes. The scan records nothing
/// else about a picture: width, height and hash stay `None`.
pub(super) fn art_of(raw_tags: &str) -> Option<Art> {
    let Ok(Value::Object(blocks)) = serde_json::from_str::<Value>(raw_tags) else {
        return None;
    };
    let largest = blocks
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter(|item| {
            item.get("key")
                .and_then(Value::as_str)
                .is_some_and(is_picture_key)
        })
        .filter_map(|item| {
            let value = item.get("value")?;
            (value.get("type")?.as_str()? == "binary")
                .then(|| value.get("len")?.as_u64())
                .flatten()
        })
        .filter(|len| *len > 0)
        .max()?;
    Some(Art {
        bytes: u32::try_from(largest).unwrap_or(u32::MAX),
        width: None,
        height: None,
        hash: None,
    })
}

/// Whether a stored tag item's key is a picture: ID3's `APIC` (and `PIC`),
/// Vorbis and FLAC's `METADATA_BLOCK_PICTURE`, MP4's `covr`, APE's
/// `Cover Art (…)`. An item key may carry a description after a colon.
fn is_picture_key(key: &str) -> bool {
    let name = key.split(':').next().unwrap_or(key);
    matches!(name, "APIC" | "PIC" | "METADATA_BLOCK_PICTURE" | "covr")
        || name.to_ascii_lowercase().starts_with("cover art")
}

/// The same files as a map by file id, for looking two up.
pub(super) fn by_file(files: &[Loaded]) -> HashMap<i64, &FileFacts> {
    files.iter().map(|l| (l.facts.file_id, &l.facts)).collect()
}
