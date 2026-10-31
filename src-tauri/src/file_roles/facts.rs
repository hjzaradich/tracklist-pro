//! Reading what the ranking needs about the files of some tracks out of the
//! database, one query for all of them (plus the stored verdicts).
//!
//! Every input is the stored fact, never the live state of the machine:
//! whether a drive is plugged in is not read ([`Place::volume_online`] is
//! always `true` here), so the roles are the same with the drive in or out.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Connection;

use super::choice::{content_key, Choices, Held};
use crate::best_file::{
    Art, FileFacts, FolderRole, Format, Place, Quality, RekordboxLink, VolumeKind,
};
use crate::quality::store::codec_of;
use crate::send_values::{plays_in_dj_app, tag_values_and_largest_binary, TAG_ATTRIBUTES};
use crate::tags::TagFields;

/// One file of a track, as the ranking sees it, and the role it has now.
pub(crate) struct Loaded {
    pub facts: FileFacts,
    pub role: String,
}

/// The files of each of `recordings`, by track. A track with no files is
/// not in the map. Leaves the stored verdicts as they are: whoever calls
/// settles them first.
pub(crate) fn load(
    conn: &Connection,
    recordings: &[i64],
) -> rusqlite::Result<BTreeMap<i64, Vec<Loaded>>> {
    let wanted = serde_json::to_string(recordings).unwrap_or_else(|_| "[]".into());
    read(
        conn,
        "FROM recording_file rf
         JOIN file f ON f.id = rf.file_id
         JOIN music_folder mf ON mf.id = f.music_folder_id
         JOIN volume v ON v.id = mf.volume_id
         LEFT JOIN file_quality q ON q.file_id = f.id
         WHERE rf.recording_id IN (SELECT value FROM json_each(?1))
         ORDER BY rf.recording_id, f.id",
        wanted,
    )
}

/// One file that is in no track yet (scanned, not yet grouped), as the
/// ranking sees it, with track 0 and the role `undecided`. `None` if there
/// is no such file or the file has a track (use [`load`]).
pub(crate) fn load_unplaced(conn: &Connection, file: i64) -> rusqlite::Result<Option<Loaded>> {
    let mut found = read(
        conn,
        "FROM (SELECT 0 AS recording_id, 'undecided' AS role) rf
         JOIN file f ON f.id = CAST(?1 AS INTEGER)
         JOIN music_folder mf ON mf.id = f.music_folder_id
         JOIN volume v ON v.id = mf.volume_id
         LEFT JOIN file_quality q ON q.file_id = f.id
         WHERE NOT EXISTS (SELECT 1 FROM recording_file x WHERE x.file_id = f.id)",
        file.to_string(),
    )?;
    Ok(found.remove(&0).and_then(|mut files| files.pop()))
}

/// Reads the rows `tail` (a `FROM … ORDER BY` over `rf`, `f`, `mf`, `v` and
/// `q`, with `?1` bound to `param`) selects, and the stored verdicts of
/// their files.
fn read(
    conn: &Connection,
    tail: &str,
    param: String,
) -> rusqlite::Result<BTreeMap<i64, Vec<Loaded>>> {
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
                         ELSE 0 END,
                    f.audio_hash, f.blake3
             {tail}",
            version = crate::quality::VERSION
        ))?;
        let mapped = stmt.query_map([param], |r| {
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
                audio_hash: r.get(13)?,
                blake3: r.get(14)?,
            })
        })?;
        for row in mapped {
            rows.push(row?);
        }
    }
    let file_ids: Vec<i64> = rows.iter().map(|r| r.file).collect();
    let verdicts = crate::quality::stored_verdicts(conn, &file_ids)?;
    let mut by_recording: BTreeMap<i64, Vec<(Loaded, Held)>> = BTreeMap::new();
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
            Some(raw) => tags_and_art_of(raw),
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
            chosen: false,
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
        let held = Held {
            on_disk: row.present,
            key: content_key(row.audio_hash, row.blake3.as_ref()),
            blake3: row.blake3,
        };
        let loaded = Loaded {
            facts,
            role: row.role,
        };
        by_recording
            .entry(row.recording)
            .or_default()
            .push((loaded, held));
    }
    // What the user chose (`file_choice`) is decisive: a track's files that
    // a choice covers are marked, so the ranking puts them first.
    // A track in the Library has no choice: the feature is for tracks outside
    // it (owner decision, 2026-10-28), so the ranking, and Review's rules
    // that read it, work as if there were none. The row stays and applies
    // again if the track leaves the Library.
    let choices = Choices::load(conn)?;
    let in_library = if choices.is_empty() {
        HashSet::new()
    } else {
        library_tracks(conn, by_recording.keys().copied())?
    };
    Ok(by_recording
        .into_iter()
        .map(|(recording, files)| {
            let (mut loaded, held): (Vec<Loaded>, Vec<Held>) = files.into_iter().unzip();
            if !in_library.contains(&recording) {
                for (file, chosen) in loaded.iter_mut().zip(choices.chosen(&held)) {
                    file.facts.chosen = chosen;
                }
            }
            (recording, loaded)
        })
        .collect())
}

/// Which of `recordings` are in the Library.
fn library_tracks(
    conn: &Connection,
    recordings: impl Iterator<Item = i64>,
) -> rusqlite::Result<HashSet<i64>> {
    let wanted =
        serde_json::to_string(&recordings.collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
    conn.prepare_cached(
        "SELECT DISTINCT recording_id FROM library_track
         WHERE recording_id IN (SELECT value FROM json_each(?1))",
    )?
    .query_map([wanted], |r| r.get(0))?
    .collect()
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
    audio_hash: Option<Vec<u8>>,
    blake3: Option<Vec<u8>>,
}

/// The descriptive tags a file's `raw_tags` give, and its largest embedded
/// picture, from one read of the stored tags. Album artist and ISRC aren't
/// mapped yet (they stay `None`).
///
/// The art is the largest picture's size in bytes. The scan records nothing
/// else about a picture: width, height and hash stay `None`.
pub(super) fn tags_and_art_of(raw_tags: &str) -> (TagFields, Option<Art>) {
    let mut tags = TagFields::default();
    let number = |text: &str| text.trim().parse::<i64>().ok();
    let (found, largest) = tag_values_and_largest_binary(raw_tags, &TAG_ATTRIBUTES, is_picture_key);
    for found in found {
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
    let art = largest.map(|largest| Art {
        bytes: u32::try_from(largest).unwrap_or(u32::MAX),
        width: None,
        height: None,
        hash: None,
    });
    (tags, art)
}

/// [`tags_and_art_of`]'s tags.
#[cfg(test)]
pub(super) fn tags_of(raw_tags: &str) -> TagFields {
    tags_and_art_of(raw_tags).0
}

/// [`tags_and_art_of`]'s art.
#[cfg(test)]
pub(super) fn art_of(raw_tags: &str) -> Option<Art> {
    tags_and_art_of(raw_tags).1
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
