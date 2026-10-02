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
//! That includes a track whose *linked file* rekordbox doesn't hold but
//! another file of the same track it does (a duplicate): a send would add
//! the linked file as a second entry (rule 3, case A is "already in
//! rekordbox at this Location"), so it's treated as new and
//! [`TrackValues::rekordbox_holds_other_file`] names the other file, for
//! the send flow to warn about. Nothing is re-pointed here (1bE-6 decides
//! that).
//! - *Tags* ([`tag_fields`] has the mapping): per field the linked file's
//!   tag if it has a value, else the first value among the track's other
//!   files that are on disk, in best-file order (role `best` first, then
//!   the lowest file id). Values are as the tag has them: several values
//!   in one field are joined with `, `, a legacy numeric genre becomes its
//!   name, a year must be 1000-9999, and nothing is trimmed or stripped
//!   (a character XML can't carry is the writer's to report).
//! - *File facts* (`Kind`, `Size`, `TotalTime`, `BitRate`, `SampleRate`)
//!   come from the linked file itself, as the last scan and read recorded
//!   them (§5.3: never from rekordbox). `TotalTime` is whole seconds,
//!   truncated. `Kind` is only given for formats rekordbox plays; "M4A
//!   File" for MP4 (AAC and ALAC alike) is unverified (the rb-kit check
//!   should confirm it once).
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
//! **A known track whose linked file is missing** is still sent: rekordbox's
//! own entry goes back unchanged ([`TrackValues::file_missing`]), so a crate
//! or playlist naming it keeps it (owner decision, 2026-10-01). Relink never
//! matches a rekordbox row to a missing file, so the entry is found by the
//! file's own path instead: the one row of the newest read at that path's
//! `Location` key (relink's own key: NFC, NTFS letter case) that is matched
//! to no file. The path is built from the volume's mount point by relink's
//! own rule: where it's mounted now, or where it was last mounted only if no
//! other known volume was. It only echoes rekordbox's entry back at its own
//! `Location`; nothing is attached to any file. Two rows at the `Location`,
//! a row matched to a file, a row kept from an earlier read, or one row
//! wanted by two files leave the track out. A Library
//! track whose linked file is missing and that rekordbox doesn't know, or
//! whose file is unplaceable, comes back as [`Outcome::CannotSend`], for
//! that track only.
//!
//! [`send_values`] is the batch form: a fixed number of queries however
//! many tracks it's given. Asking for one track is a batch of one.

mod shown;
mod tag_fields;

use std::collections::HashMap;

use rusqlite::Connection;

use crate::library::{file_from, LibraryTrackId, StoredFile, FILE_COLUMNS, FILE_JOINS};
use crate::paths::Volumes;

pub use shown::{shown, shown_recordings, shown_removed, Removed, Shown};
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

/// Another file of the same track that rekordbox holds a trusted entry for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiblingEntry {
    pub file_id: i64,
    /// rekordbox's `TrackID` for it (valid within the last read).
    pub track_id: i64,
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
    /// Set when the track isn't known at the linked file but rekordbox
    /// holds another file of it (a duplicate): a send would add the linked
    /// file as a second rekordbox entry beside that one, and the send flow
    /// can warn. The linked file stays as it is (no re-pointing here).
    /// Always `None` for a track rekordbox knows.
    pub rekordbox_holds_other_file: Option<SiblingEntry>,
    /// rekordbox knows the track but its linked file isn't on disk now
    /// (owner decision, 2026-10-01). Every value is rekordbox's own entry,
    /// and the writer sends it unchanged, `Location` exactly as rekordbox
    /// wrote it, so the crates and playlists naming the track stay whole.
    /// Always `false` for a track rekordbox doesn't know: that one is
    /// [`CannotSend::FileMissing`], because there's nothing of rekordbox's
    /// to send.
    pub file_missing: bool,
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
    /// The file's name, without its folder.
    name: String,
}

struct Row {
    library_track: i64,
    recording: i64,
    linked_file: Option<i64>,
    /// `recording.title` / `recording.artist`: nothing fills them in Phase 1a.
    title: Option<String>,
    artist: Option<String>,
}

/// How much of the database a batch loads. [`Scope::Names`] is for the
/// lists ([`shown`]): the same decisions, but only the two attributes a list
/// shows, and a file's tags only where the track's values come from them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Send,
    Names,
}

/// What a batch loaded for its tracks.
struct Loaded {
    rows: HashMap<i64, Row>,
    by_recording: HashMap<i64, Vec<i64>>,
    files: HashMap<i64, FileInfo>,
    entry_for: HashMap<i64, (i64, i64)>,
    at_location: HashMap<i64, (i64, i64)>,
    attributes: HashMap<i64, Vec<(String, String)>>,
}

fn json_ids(ids: &mut dyn Iterator<Item = i64>) -> String {
    serde_json::to_string(&ids.collect::<Vec<_>>()).expect("numbers always serialize")
}

/// The values for every Library track in `ids`, in the order given, in a
/// fixed number of queries. An id that isn't a Library track answers
/// [`CannotSend::NotInLibrary`].
pub fn send_values(
    conn: &Connection,
    volumes: &impl Volumes,
    ids: &[LibraryTrackId],
) -> rusqlite::Result<Vec<SendValues>> {
    let loaded = load(conn, volumes, ids, Scope::Send)?;
    let world = World {
        volumes,
        loaded: &loaded,
    };
    Ok(ids
        .iter()
        .map(|&id| SendValues {
            library_track: id,
            outcome: match loaded.rows.get(&id.0) {
                None => Outcome::CannotSend(CannotSend::NotInLibrary),
                Some(row) => world.outcome(row),
            },
        })
        .collect())
}

/// Loads what the decisions of [`World`] need, in a fixed number of queries.
fn load(
    conn: &Connection,
    volumes: &impl Volumes,
    ids: &[LibraryTrackId],
    scope: Scope,
) -> rusqlite::Result<Loaded> {
    let json = json_ids;

    // 1. The Library tracks.
    let wanted = json(&mut ids.iter().map(|id| id.0));
    let rows: HashMap<i64, Row> = conn
        .prepare(
            "SELECT lt.id, lt.recording_id, lt.linked_file_id, r.title, r.artist
             FROM library_track lt LEFT JOIN recording r ON r.id = lt.recording_id
             WHERE lt.id IN (SELECT value FROM json_each(?1))",
        )?
        .query_map([&wanted], |r| {
            Ok(Row {
                library_track: r.get(0)?,
                recording: r.get(1)?,
                linked_file: r.get(2)?,
                title: r.get(3)?,
                artist: r.get(4)?,
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

    // 3. Every file they need, once. A list reads the tags later, and only
    //    of the files it needs them of.
    let file_ids = json(
        &mut by_recording
            .values()
            .flatten()
            .copied()
            .chain(rows.values().filter_map(|r| r.linked_file)),
    );
    let tags_column = match scope {
        Scope::Send => "f.raw_tags",
        Scope::Names => "NULL",
    };
    let mut files: HashMap<i64, FileInfo> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT f.present, f.size, f.sniffed_format, f.bitrate, f.sample_rate,
                f.duration_ms, {tags_column}, {FILE_COLUMNS}, f.rel_path
         FROM file f {FILE_JOINS}
         WHERE f.id IN (SELECT value FROM json_each(?1))"
    ))?;
    for row in stmt.query_map([&file_ids], |r| {
        let stored = file_from(r, 7)?;
        let id: i64 = r.get(7)?;
        let rel_path: String = r.get(13)?;
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
                name: file_name(&rel_path).to_owned(),
            },
        ))
    })? {
        let (id, info) = row?;
        files.insert(id, info);
    }

    // 4. The trusted rekordbox entry for each file (the linked file's
    //    makes a track rekordbox knows; another file's is a sibling), the
    //    most played if it holds several, then the lowest TrackID.
    let mut entry_for: HashMap<i64, (i64, i64)> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT file_id, id, track_id FROM rekordbox_track
         WHERE relink_probable = 0 AND file_id IN (SELECT value FROM json_each(?1))
         ORDER BY file_id, COALESCE(play_count, 0) DESC, track_id",
    )?;
    for row in stmt.query_map([&file_ids], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))? {
        let (file, entry): (i64, (i64, i64)) = row?;
        entry_for.entry(file).or_insert(entry);
    }

    // 4b. A linked file that's missing has no match (relink drops a match
    //     to a gone file), so rekordbox's own entry at the file's own
    //     Location stands in for it: see `entries_at_missing_files`.
    let at_location = entries_at_missing_files(conn, volumes, &rows, &files, &entry_for)?;

    let mut loaded = Loaded {
        rows,
        by_recording,
        files,
        entry_for,
        at_location,
        attributes: HashMap::new(),
    };
    // 4c. A list needs a file's tags only where nothing of rekordbox's
    //     stands in for them.
    if scope == Scope::Names {
        let needed = loaded.files_read_for_tags();
        load_tags(conn, &mut loaded.files, &needed)?;
    }
    loaded.load_attributes(conn, scope)?;
    Ok(loaded)
}

/// The text after the last `/` or `\`: a file's name without its folder.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or_default()
}

/// Reads `raw_tags` of the `needed` files into `files`.
fn load_tags(
    conn: &Connection,
    files: &mut HashMap<i64, FileInfo>,
    needed: &[i64],
) -> rusqlite::Result<()> {
    if needed.is_empty() {
        return Ok(());
    }
    let wanted = json_ids(&mut needed.iter().copied());
    let mut stmt = conn
        .prepare("SELECT id, raw_tags FROM file WHERE id IN (SELECT value FROM json_each(?1))")?;
    for row in stmt.query_map([&wanted], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
    })? {
        let (id, raw_tags) = row?;
        if let Some(file) = files.get_mut(&id) {
            file.raw_tags = raw_tags;
        }
    }
    Ok(())
}

impl Loaded {
    /// 5. The attributes of the linked files' entries, in the order
    ///    rekordbox wrote them (only `Name` and `Artist` for a list).
    fn load_attributes(&mut self, conn: &Connection, scope: Scope) -> rusqlite::Result<()> {
        let entries = json_ids(
            &mut self
                .rows
                .values()
                .filter_map(|r| r.linked_file)
                .filter_map(|f| {
                    self.entry_for
                        .get(&f)
                        .or_else(|| self.at_location.get(&f))
                        .map(|e| e.0)
                }),
        );
        let only = match scope {
            Scope::Send => "",
            Scope::Names => "AND j.key IN ('Name', 'Artist')",
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT rt.id, j.key, j.atom FROM rekordbox_track rt, json_each(rt.attributes) j
             WHERE rt.id IN (SELECT value FROM json_each(?1)) {only}
             ORDER BY rt.id, j.id"
        ))?;
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
            self.attributes
                .entry(entry)
                .or_default()
                .push((name, value));
        }
        Ok(())
    }

    /// The files whose tags a list reads: those of every track whose values
    /// don't come from rekordbox's entry.
    fn files_read_for_tags(&self) -> Vec<i64> {
        let mut needed: Vec<i64> = Vec::new();
        for row in self.rows.values() {
            if let Source::Known { .. } = self.source(row) {
                continue;
            }
            needed.extend(self.tag_order(row));
        }
        needed.sort_unstable();
        needed.dedup();
        needed
    }

    /// Where a track's values come from. The one place that decides it, for
    /// a send and for a list.
    fn source(&self, row: &Row) -> Source {
        let Some(file_id) = row.linked_file else {
            return Source::Cannot(CannotSend::NoLinkedFile);
        };
        let Some(file) = self.files.get(&file_id) else {
            return Source::Cannot(CannotSend::NoLinkedFile);
        };
        // rekordbox's own Location is sent for a track it knows, so that
        // doesn't need the path to read back, and a missing file doesn't
        // stop it: its entry goes back unchanged, so the crates naming it
        // stay whole.
        if let Some(&(entry, track_id)) = self.entry_for.get(&file_id) {
            return Source::Known {
                entry,
                track_id,
                file_missing: !file.present,
            };
        }
        if !file.present {
            if let Some(&(entry, track_id)) = self.at_location.get(&file_id) {
                return Source::Known {
                    entry,
                    track_id,
                    file_missing: true,
                };
            }
            return Source::Cannot(CannotSend::FileMissing { file_id });
        }
        if file.stored.is_none() {
            return Source::Cannot(CannotSend::NoLocation { file_id });
        }
        Source::Files { file_id }
    }

    /// The files a track's tags are read from: the linked file first, then
    /// the track's other files that are on disk, best first.
    fn tag_order(&self, row: &Row) -> Vec<i64> {
        let linked = row.linked_file.filter(|f| self.files.contains_key(f));
        let mut order: Vec<i64> = linked.into_iter().collect();
        order.extend(
            self.by_recording
                .get(&row.recording)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&f| Some(f) != linked && self.files.get(&f).is_some_and(|f| f.present)),
        );
        order
    }
}

/// Where a track's values come from.
enum Source {
    /// rekordbox's entry (a row of `rekordbox_track`).
    Known {
        entry: i64,
        track_id: i64,
        file_missing: bool,
    },
    /// The track's files: the linked file is on disk and has a path.
    Files { file_id: i64 },
    /// Nothing to send for it.
    Cannot(CannotSend),
}

/// For each linked file that's missing and has no match, rekordbox's own
/// entry at the file's own `Location`, as (row id, `TrackID`). Relink never
/// matches a row to a missing file, so without this a track rekordbox
/// knows would be left out of its crates once its file went missing.
///
/// It only echoes rekordbox's entry back at its own `Location`: nothing of
/// rekordbox's is attached to a file, so this is no match in relink's sense.
/// An entry is used only if it
/// - is from the current read (an incomplete read keeps older rows, whose
///   `TrackID`s belong to an earlier read),
/// - is the only row at that `Location` (two rows are ambiguous),
/// - is matched to no file (a probable match always has a file): a row
///   matched to a file belongs to that file's track, and
/// - isn't wanted by a second file: a row whose key two files lead to goes
///   to neither, whatever the order.
///
/// The file's path is its volume's mount point and its path on the volume,
/// keyed as relink keys it (NFC, NTFS case folding). The mount point is
/// relink's own rule ([`crate::relink::volume_mounts`]): where the volume is
/// mounted now, or where it was last mounted only if no other known volume
/// was, so two volumes that shared a drive letter never find each other's
/// entries.
fn entries_at_missing_files<V: Volumes>(
    conn: &Connection,
    volumes: &V,
    rows: &HashMap<i64, Row>,
    files: &HashMap<i64, FileInfo>,
    entry_for: &HashMap<i64, (i64, i64)>,
) -> rusqlite::Result<HashMap<i64, (i64, i64)>> {
    let missing: Vec<(i64, &StoredFile)> = rows
        .values()
        .filter_map(|r| r.linked_file)
        .collect::<std::collections::BTreeSet<i64>>()
        .into_iter()
        .filter_map(|file_id| {
            let file = files.get(&file_id)?;
            if file.present || entry_for.contains_key(&file_id) {
                return None;
            }
            Some((file_id, file.stored.as_ref()?))
        })
        .collect();
    if missing.is_empty() {
        return Ok(HashMap::new());
    }
    let mounted = crate::relink::Mounted::ask(&crate::relink::identities(conn)?, volumes);
    let mounts: HashMap<String, String> = crate::relink::volume_mounts(conn, &mounted)?
        .into_iter()
        .filter_map(|v| Some((v.identity, v.mount?)))
        .collect();
    let mut wanted: Vec<(i64, String)> = Vec::new();
    for (file_id, stored) in missing {
        let Some(mount) = mounts.get(stored.volume.as_str()) else {
            continue;
        };
        let rel: Vec<&str> = stored.rel.components().collect();
        let path = format!("{mount}/{}", rel.join("/"));
        wanted.push((file_id, crate::relink::rules::path_key(&path)));
    }
    if wanted.is_empty() {
        return Ok(HashMap::new());
    }
    let keys = serde_json::to_string(&wanted.iter().map(|(_, k)| k).collect::<Vec<_>>())
        .expect("strings always serialize");
    // location_key -> every row of the current read at it.
    let mut at: HashMap<String, Vec<RowAt>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT location_key, id, track_id, file_id FROM rekordbox_track
         WHERE read_at = (SELECT max(read_at) FROM rekordbox_track)
           AND location_key IN (SELECT value FROM json_each(?1))",
    )?;
    for row in stmt.query_map([&keys], |r| {
        Ok((
            r.get::<_, String>(0)?,
            RowAt {
                id: r.get(1)?,
                track_id: r.get(2)?,
                matched: r.get(3)?,
            },
        ))
    })? {
        let (key, entry) = row?;
        at.entry(key).or_default().push(entry);
    }
    let mut files_per_key: HashMap<&str, usize> = HashMap::new();
    for (_, key) in &wanted {
        *files_per_key.entry(key).or_default() += 1;
    }
    let mut found = HashMap::new();
    for (file_id, key) in &wanted {
        if files_per_key[key.as_str()] > 1 {
            continue;
        }
        if let Some([only]) = at.get(key).map(Vec::as_slice) {
            if only.matched.is_none() {
                found.insert(*file_id, (only.id, only.track_id));
            }
        }
    }
    Ok(found)
}

/// A `rekordbox_track` row at a `Location`.
struct RowAt {
    id: i64,
    track_id: i64,
    /// The file it is matched to, if any (a probable match always has one).
    matched: Option<i64>,
}

/// Everything the batch loaded, with the volumes paths are placed on.
struct World<'a, V> {
    volumes: &'a V,
    loaded: &'a Loaded,
}

/// Each of `order`'s files with the `wanted` tag values it gives.
fn read_tags(
    files: &HashMap<i64, FileInfo>,
    order: &[i64],
    wanted: &[&str],
) -> Vec<(i64, Vec<tag_fields::TagValue>)> {
    order
        .iter()
        .map(|&f| {
            let tags = files[&f]
                .raw_tags
                .as_deref()
                .map(|raw| tag_fields::read_only(raw, wanted))
                .unwrap_or_default();
            (f, tags)
        })
        .collect()
}

/// Every file's value for `attribute`, in the order the files were asked.
/// The first is the one a send writes.
fn candidates<'a>(
    tags: &'a [(i64, Vec<tag_fields::TagValue>)],
    attribute: &str,
) -> Vec<(i64, &'a tag_fields::TagValue)> {
    let mut found = Vec::new();
    for (file, values) in tags {
        if let Some(v) = values.iter().find(|v| v.attribute == attribute) {
            found.push((*file, v));
        }
    }
    found
}

impl<V: Volumes> World<'_, V> {
    fn outcome(&self, row: &Row) -> Outcome {
        let loaded = self.loaded;
        match loaded.source(row) {
            Source::Cannot(why) => Outcome::CannotSend(why),
            Source::Known {
                entry,
                track_id,
                file_missing,
            } => Outcome::Ready(self.known(entry, track_id, file_missing)),
            Source::Files { file_id } => {
                let file = &loaded.files[&file_id];
                let Some(stored) = &file.stored else {
                    return Outcome::CannotSend(CannotSend::NoLocation { file_id });
                };
                Outcome::Ready(self.unknown(row, file_id, file, stored))
            }
        }
    }

    /// Every attribute of rekordbox's entry, as read.
    fn known(&self, entry: i64, track_id: i64, file_missing: bool) -> TrackValues {
        let values = self
            .loaded
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
            rekordbox_holds_other_file: None,
            file_missing,
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
        let loaded = self.loaded;
        let tags = read_tags(&loaded.files, &loaded.tag_order(row), &TAG_ATTRIBUTES);

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
            let candidates = candidates(&tags, attribute);
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
        let rekordbox_holds_other_file = loaded
            .by_recording
            .get(&row.recording)
            .into_iter()
            .flatten()
            .filter(|&&f| f != file_id)
            .find_map(|f| {
                loaded.entry_for.get(f).map(|&(_, track_id)| SiblingEntry {
                    file_id: *f,
                    track_id,
                })
            });
        TrackValues {
            in_rekordbox: false,
            values,
            disagreements,
            rekordbox_holds_other_file,
            file_missing: false,
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
