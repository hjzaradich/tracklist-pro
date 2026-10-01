//! Per-track send values (1aD-8; ROADMAP 1.7, 1.8, 1.9, §5.2, §5.3).
//!
//! For a Library track, the values a send would write: each rekordbox XML
//! `TRACK` attribute name → value, each with where it came from. The XML
//! writer (1aE-7 and on) consumes this; nothing here writes a file, an XML
//! or the database, and there's no IPC command.
//!
//! **A track rekordbox knows** (a trusted rekordbox entry for the Library
//! track's linked file; a probable match doesn't count): every attribute
//! is rekordbox's current value exactly as read, string for string and in
//! the order rekordbox wrote them (rule 1). The app changes nothing in 1a.
//! [`TrackValues::in_rekordbox`] is set, so the writer omits the analysis
//! fields ([`ANALYSIS_ATTRIBUTES`] and the `TEMPO` / `POSITION_MARK`
//! children; rule 3, case A). `TrackID` is rekordbox's own and is part of
//! what it knows.
//!
//! **A track rekordbox doesn't know:** values come from the track's files.
//! - *Tags* ([`tag_fields`] has the mapping): per field the linked file's
//!   tag if it has a value, else the first value among the track's other
//!   files that are on disk, in best-file order (role `best` first, then
//!   the lowest file id).
//! - *File facts* (`Kind`, `Size`, `TotalTime`, `BitRate`, `SampleRate`)
//!   come from the linked file itself, as the last scan and read recorded
//!   them (§5.3: never from rekordbox). `TotalTime` is whole seconds,
//!   truncated. `Kind` is only given for formats rekordbox plays.
//! - `Location` is the linked file's full path, plain: the writer encodes
//!   it (rule 5). On an unplugged drive it's the path under the drive
//!   letter it was last seen at.
//! - Never `TrackID`, and never BPM, key, grid or cues from tags or
//!   estimates: rekordbox analyses a new track itself (rule 3). Nor
//!   anything rekordbox owns (play count, rating, colour, dates).
//! - When files disagree on a field the pick still follows the rule above,
//!   and [`TrackValues::disagreements`] lists every file's value for
//!   Review.
//!
//! A Library track whose linked file is missing, unknown or unplaceable
//! comes back as [`Outcome::CannotSend`], for that track only.
//!
//! [`send_values`] is the batch form: a fixed number of queries however
//! many tracks it's given. Asking for one track is a batch of one.

mod tag_fields;

use std::collections::HashMap;

use rusqlite::Connection;

use crate::library::{file_from, LibraryTrackId, StoredFile, FILE_COLUMNS, FILE_JOINS};
use crate::paths::Volumes;

pub use tag_fields::TAG_ATTRIBUTES;

/// The `TRACK` attributes that are rekordbox's analysis: a track already in
/// rekordbox is sent without them (rule 3). The `TEMPO` and `POSITION_MARK`
/// children are analysis too.
pub const ANALYSIS_ATTRIBUTES: [&str; 2] = ["AverageBpm", "Tonality"];

/// Where a value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// rekordbox's current value, from its entry with this `TrackID` (valid
    /// within the last read).
    Rekordbox { track_id: i64 },
    /// A tag in this file, named as `block:key`, e.g. `id3v2:TPE1`.
    FileTag { file_id: i64, tag: String },
    /// A fact about this file: its format, size, duration, bitrate, sample
    /// rate or path.
    FileFact { file_id: i64 },
}

/// One `TRACK` attribute to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    /// The XML attribute name, e.g. `Name`.
    pub attribute: String,
    /// The value, unescaped, as text.
    pub value: String,
    pub provenance: Provenance,
}

/// One file's value for a field its track's files disagree on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileValue {
    pub file_id: i64,
    pub value: String,
}

/// A field whose files hold different values. The one sent is in
/// [`TrackValues::values`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disagreement {
    pub attribute: String,
    /// Every file that has a value for it, in the order they were asked.
    pub values: Vec<FileValue>,
}

/// What a send would write for one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackValues {
    /// rekordbox already has this track at this file (a trusted match):
    /// the writer sends every value as it is and omits the analysis fields
    /// (rule 3, case A).
    pub in_rekordbox: bool,
    pub values: Vec<Value>,
    /// Always empty for a track rekordbox knows: nothing is picked from
    /// tags there.
    pub disagreements: Vec<Disagreement>,
}

impl TrackValues {
    /// The value of one attribute.
    pub fn get(&self, attribute: &str) -> Option<&Value> {
        self.values.iter().find(|v| v.attribute == attribute)
    }
}

/// Why a Library track can't be sent. Nothing is wrong with the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CannotSend {
    /// There's no Library track with that id.
    NotInLibrary,
    /// It has no linked file (a copy arrives with 2.6; a linked track whose
    /// file was already gone when the Library started).
    NoLinkedFile,
    /// Its linked file isn't on disk now.
    FileMissing { file_id: i64 },
    /// Its linked file's volume or path can't be read back, so there's no
    /// `Location` to send.
    NoLocation { file_id: i64 },
}

/// The answer for one Library track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ready(TrackValues),
    CannotSend(CannotSend),
}

/// One Library track's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendValues {
    pub library_track: LibraryTrackId,
    pub outcome: Outcome,
}

/// A file with what the values need of it.
struct FileInfo {
    stored: Option<StoredFile>,
    present: bool,
    size: Option<i64>,
    format: Option<String>,
    bitrate_kbps: Option<i64>,
    sample_rate: Option<i64>,
    duration_ms: Option<i64>,
    raw_tags: Option<String>,
}

struct Row {
    library_track: i64,
    recording: i64,
    linked_file: Option<i64>,
}

/// The values for every Library track in `ids`, in the order given, in a
/// fixed number of queries. An id that isn't a Library track answers
/// [`CannotSend::NotInLibrary`].
pub fn send_values(
    conn: &Connection,
    volumes: &impl Volumes,
    ids: &[LibraryTrackId],
) -> rusqlite::Result<Vec<SendValues>> {
    let json = |ids: &mut dyn Iterator<Item = i64>| {
        serde_json::to_string(&ids.collect::<Vec<_>>()).expect("numbers always serialize")
    };

    // 1. The Library tracks.
    let wanted = json(&mut ids.iter().map(|id| id.0));
    let rows: HashMap<i64, Row> = conn
        .prepare(
            "SELECT id, recording_id, linked_file_id FROM library_track
             WHERE id IN (SELECT value FROM json_each(?1))",
        )?
        .query_map([&wanted], |r| {
            Ok(Row {
                library_track: r.get(0)?,
                recording: r.get(1)?,
                linked_file: r.get(2)?,
            })
        })?
        .map(|r| r.map(|r| (r.library_track, r)))
        .collect::<rusqlite::Result<_>>()?;

    // 2. Their files, in the order their tags are asked: best first.
    let recordings = json(&mut rows.values().map(|r| r.recording));
    let mut by_recording: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT recording_id, file_id FROM recording_file
         WHERE recording_id IN (SELECT value FROM json_each(?1))
         ORDER BY recording_id, (role = 'best') DESC, file_id",
    )?;
    for row in stmt.query_map([&recordings], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (recording, file): (i64, i64) = row?;
        by_recording.entry(recording).or_default().push(file);
    }

    // 3. Every file they need, once.
    let file_ids = json(
        &mut by_recording
            .values()
            .flatten()
            .copied()
            .chain(rows.values().filter_map(|r| r.linked_file)),
    );
    let mut files: HashMap<i64, FileInfo> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT f.present, f.size, f.sniffed_format, f.bitrate, f.sample_rate,
                f.duration_ms, f.raw_tags, {FILE_COLUMNS}
         FROM file f {FILE_JOINS}
         WHERE f.id IN (SELECT value FROM json_each(?1))"
    ))?;
    for row in stmt.query_map([&file_ids], |r| {
        let stored = file_from(r, 7)?;
        let id: i64 = r.get(7)?;
        Ok((
            id,
            FileInfo {
                stored,
                present: r.get(0)?,
                size: r.get(1)?,
                format: r.get(2)?,
                bitrate_kbps: r.get(3)?,
                sample_rate: r.get(4)?,
                duration_ms: r.get(5)?,
                raw_tags: r.get(6)?,
            },
        ))
    })? {
        let (id, info) = row?;
        files.insert(id, info);
    }

    // 4. The trusted rekordbox entry for each linked file (a track
    //    rekordbox knows), the most played if it holds several, then the
    //    lowest TrackID.
    let linked = json(&mut rows.values().filter_map(|r| r.linked_file));
    let mut entry_for: HashMap<i64, (i64, i64)> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT file_id, id, track_id FROM rekordbox_track
         WHERE relink_probable = 0 AND file_id IN (SELECT value FROM json_each(?1))
         ORDER BY file_id, COALESCE(play_count, 0) DESC, track_id",
    )?;
    for row in stmt.query_map([&linked], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))? {
        let (file, entry): (i64, (i64, i64)) = row?;
        entry_for.entry(file).or_insert(entry);
    }

    // 5. Those entries' attributes, in the order rekordbox wrote them.
    let entries = json(&mut entry_for.values().map(|e| e.0));
    let mut attributes: HashMap<i64, Vec<(String, String)>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT rt.id, j.key, j.atom FROM rekordbox_track rt, json_each(rt.attributes) j
         WHERE rt.id IN (SELECT value FROM json_each(?1))
         ORDER BY rt.id, j.id",
    )?;
    for row in stmt.query_map([&entries], |r| {
        let atom: rusqlite::types::Value = r.get(2)?;
        let atom = match atom {
            rusqlite::types::Value::Text(t) => t,
            rusqlite::types::Value::Integer(n) => n.to_string(),
            rusqlite::types::Value::Real(n) => n.to_string(),
            _ => String::new(),
        };
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, atom))
    })? {
        let (entry, name, value) = row?;
        attributes.entry(entry).or_default().push((name, value));
    }

    let world = World {
        volumes,
        files: &files,
        by_recording: &by_recording,
        entry_for: &entry_for,
        attributes: &attributes,
    };
    Ok(ids
        .iter()
        .map(|&id| SendValues {
            library_track: id,
            outcome: match rows.get(&id.0) {
                None => Outcome::CannotSend(CannotSend::NotInLibrary),
                Some(row) => world.outcome(row),
            },
        })
        .collect())
}

/// Everything the batch loaded.
struct World<'a, V> {
    volumes: &'a V,
    files: &'a HashMap<i64, FileInfo>,
    by_recording: &'a HashMap<i64, Vec<i64>>,
    entry_for: &'a HashMap<i64, (i64, i64)>,
    attributes: &'a HashMap<i64, Vec<(String, String)>>,
}

impl<V: Volumes> World<'_, V> {
    fn outcome(&self, row: &Row) -> Outcome {
        let Some(file_id) = row.linked_file else {
            return Outcome::CannotSend(CannotSend::NoLinkedFile);
        };
        let Some(file) = self.files.get(&file_id) else {
            return Outcome::CannotSend(CannotSend::NoLinkedFile);
        };
        if !file.present {
            return Outcome::CannotSend(CannotSend::FileMissing { file_id });
        }
        let Some(stored) = &file.stored else {
            return Outcome::CannotSend(CannotSend::NoLocation { file_id });
        };
        if let Some(&(entry, track_id)) = self.entry_for.get(&file_id) {
            return Outcome::Ready(self.known(entry, track_id));
        }
        Outcome::Ready(self.unknown(row, file_id, file, stored))
    }

    /// Every attribute of rekordbox's entry, as read.
    fn known(&self, entry: i64, track_id: i64) -> TrackValues {
        let values = self
            .attributes
            .get(&entry)
            .into_iter()
            .flatten()
            .map(|(name, value)| Value {
                attribute: name.clone(),
                value: value.clone(),
                provenance: Provenance::Rekordbox { track_id },
            })
            .collect();
        TrackValues {
            in_rekordbox: true,
            values,
            disagreements: Vec::new(),
        }
    }

    /// Values from the track's files.
    fn unknown(
        &self,
        row: &Row,
        file_id: i64,
        file: &FileInfo,
        stored: &StoredFile,
    ) -> TrackValues {
        // The linked file first, then the track's other files on disk.
        let mut order = vec![file_id];
        order.extend(
            self.by_recording
                .get(&row.recording)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&f| f != file_id && self.files.get(&f).is_some_and(|f| f.present)),
        );
        let tags: Vec<(i64, Vec<tag_fields::TagValue>)> = order
            .iter()
            .map(|&f| {
                let tags = self.files[&f]
                    .raw_tags
                    .as_deref()
                    .map(tag_fields::read)
                    .unwrap_or_default();
                (f, tags)
            })
            .collect();

        let fact = |attribute: &str, value: String| Value {
            attribute: attribute.to_owned(),
            value,
            provenance: Provenance::FileFact { file_id },
        };
        let mut facts: Vec<Value> = Vec::new();
        if let Some(kind) = file.format.as_deref().and_then(kind) {
            facts.push(fact("Kind", kind.to_owned()));
        }
        facts.extend(file.size.map(|n| fact("Size", n.to_string())));
        facts.extend(
            file.duration_ms
                .map(|ms| fact("TotalTime", (ms / 1000).to_string())),
        );
        facts.extend(file.bitrate_kbps.map(|n| fact("BitRate", n.to_string())));
        facts.extend(file.sample_rate.map(|n| fact("SampleRate", n.to_string())));
        facts.push(fact("Location", stored.shown(self.volumes).path));

        let mut values = Vec::new();
        let mut disagreements = Vec::new();
        for attribute in TAG_ATTRIBUTES {
            let mut candidates: Vec<(i64, &tag_fields::TagValue)> = Vec::new();
            for (file, found) in &tags {
                if let Some(v) = found.iter().find(|v| v.attribute == attribute) {
                    candidates.push((*file, v));
                }
            }
            let Some(&(from, picked)) = candidates.first() else {
                continue;
            };
            values.push(Value {
                attribute: attribute.to_owned(),
                value: picked.value.clone(),
                provenance: Provenance::FileTag {
                    file_id: from,
                    tag: picked.tag.clone(),
                },
            });
            if candidates.iter().any(|(_, v)| v.value != picked.value) {
                disagreements.push(Disagreement {
                    attribute: attribute.to_owned(),
                    values: candidates
                        .iter()
                        .map(|(file_id, v)| FileValue {
                            file_id: *file_id,
                            value: v.value.clone(),
                        })
                        .collect(),
                });
            }
        }
        values.extend(facts);
        // rekordbox's own order of attributes.
        values.sort_by_key(|v| attribute_rank(&v.attribute));
        TrackValues {
            in_rekordbox: false,
            values,
            disagreements,
        }
    }
}

/// The order rekordbox writes the attributes in.
const ORDER: [&str; 17] = [
    "Name",
    "Artist",
    "Composer",
    "Album",
    "Genre",
    "Kind",
    "Size",
    "TotalTime",
    "DiscNumber",
    "TrackNumber",
    "Year",
    "BitRate",
    "SampleRate",
    "Comments",
    "Location",
    "Remixer",
    "Label",
];

fn attribute_rank(attribute: &str) -> usize {
    ORDER
        .iter()
        .position(|a| *a == attribute)
        .unwrap_or(ORDER.len())
}

/// rekordbox's `Kind` for a file format (`file.sniffed_format`), for the
/// formats it plays. Others (Ogg, Opus, WavPack, WMA, …) get none.
fn kind(format: &str) -> Option<&'static str> {
    Some(match format {
        "mp3" => "MP3 File",
        "flac" => "FLAC File",
        "wav" | "rf64" => "WAV File",
        "aiff" | "aifc" => "AIFF File",
        "mp4" => "M4A File",
        _ => return None,
    })
}

/// A single Library track's values (a batch of one).
pub fn send_values_one(
    conn: &Connection,
    volumes: &impl Volumes,
    id: LibraryTrackId,
) -> rusqlite::Result<SendValues> {
    Ok(send_values(conn, volumes, &[id])?
        .pop()
        .expect("one answer per id"))
}

#[cfg(test)]
mod tests;
