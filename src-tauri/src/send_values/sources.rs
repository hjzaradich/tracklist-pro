//! Where each tag field of a track comes from, for the Details panel
//! (1cB-17; ROADMAP 1.11, 1.12).
//!
//! The panel shows, for the selected track, each field with its value and
//! the place that value comes from, by the very rules the lists and a send
//! use ([`Loaded::source`], [`Loaded::tag_order`]), so what the panel says
//! can't drift from what the track shows and what a send writes:
//!
//! - **A Library track rekordbox knows:** rekordbox's value. When a file's
//!   tag says something else, that value is listed too, naming its file: the
//!   linked file's, then the track's other files on disk, best first. A field
//!   rekordbox leaves blank has no value (a file's tag doesn't stand in for
//!   it); the files' values are still listed.
//! - **Any other track** (a Library track rekordbox doesn't know, or a track
//!   outside the Library): the shown file's tag, which is the first of the
//!   files that has the field (a Library track's linked file first). Where
//!   another file of the track says something else, that value is listed
//!   too, naming its file.
//!
//! A file that agrees with the value shown is not listed: the panel is a
//! summary, not a table of every file. Nothing is written here.

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::*;

/// A tag field the panel lists, in the order it lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TagField {
    Title,
    Artist,
    Album,
    Genre,
    Composer,
    Remixer,
    Label,
    Year,
    Comment,
}

impl TagField {
    /// The fields, in the order the panel lists them.
    const ALL: [TagField; 9] = [
        TagField::Title,
        TagField::Artist,
        TagField::Album,
        TagField::Genre,
        TagField::Composer,
        TagField::Remixer,
        TagField::Label,
        TagField::Year,
        TagField::Comment,
    ];

    /// The `TRACK` attribute rekordbox and [`TAG_ATTRIBUTES`] call it.
    fn attribute(self) -> &'static str {
        match self {
            TagField::Title => "Name",
            TagField::Artist => "Artist",
            TagField::Album => "Album",
            TagField::Genre => "Genre",
            TagField::Composer => "Composer",
            TagField::Remixer => "Remixer",
            TagField::Label => "Label",
            TagField::Year => "Year",
            TagField::Comment => "Comments",
        }
    }
}

/// Where a value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ValueOrigin {
    /// rekordbox's entry for the track.
    Rekordbox,
    /// A tag in one of the track's files (named in [`SourcedValue::file`]).
    File,
}

/// A file, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NamedFile {
    pub id: i64,
    /// The name without its folder, e.g. `a.mp3`.
    pub name: String,
}

/// A value and the place it comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SourcedValue {
    pub value: String,
    pub origin: ValueOrigin,
    /// The file the value is the tag of; `None` for rekordbox's.
    pub file: Option<NamedFile>,
}

/// One field: the value the track shows and the other values the track's
/// sources hold for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FieldSources {
    pub field: TagField,
    /// The value the track shows. `None` when rekordbox knows the track and
    /// leaves the field blank (a file's tag doesn't stand in for it).
    pub shown: Option<SourcedValue>,
    /// Values that differ from the one shown, each naming its file, in the
    /// order the files are asked.
    pub others: Vec<SourcedValue>,
}

/// What the Details panel lists for one track.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TagSources {
    /// The track is a Library track rekordbox knows: its values are
    /// rekordbox's.
    pub in_rekordbox: bool,
    /// The fields that have a value somewhere, in the panel's order.
    pub fields: Vec<FieldSources>,
    /// The `rekordbox_track` row (`entry_id` of `review_withdraw_relink`)
    /// whose match to the track's linked file the user confirmed, if there
    /// is one to withdraw.
    pub confirmed_entry: Option<i64>,
}

/// A value with something in it: blank text says nothing.
fn text(value: &str) -> Option<&str> {
    Some(value).filter(|v| !v.trim().is_empty())
}

/// The fields of a track from where its values come from: `rekordbox` is
/// the entry's attributes when the track is a Library track rekordbox knows;
/// `files` are the track's files in the order they are asked, each with the
/// tag values it gives.
fn fields_from(
    rekordbox: Option<&[(String, String)]>,
    files: &[(NamedFile, Vec<tag_fields::TagValue>)],
) -> Vec<FieldSources> {
    let mut out = Vec::new();
    for field in TagField::ALL {
        let attribute = field.attribute();
        let from_files: Vec<SourcedValue> = files
            .iter()
            .filter_map(|(file, values)| {
                let value = values.iter().find(|v| v.attribute == attribute)?;
                text(&value.value)?;
                Some(SourcedValue {
                    value: value.value.clone(),
                    origin: ValueOrigin::File,
                    file: Some(file.clone()),
                })
            })
            .collect();
        let (shown, rest) = match rekordbox {
            Some(entry) => {
                let value = entry
                    .iter()
                    .find(|(name, _)| name == attribute)
                    .and_then(|(_, value)| text(value));
                let shown = value.map(|value| SourcedValue {
                    value: value.to_owned(),
                    origin: ValueOrigin::Rekordbox,
                    file: None,
                });
                (shown, from_files)
            }
            None => {
                let mut files = from_files.into_iter();
                (files.next(), files.collect())
            }
        };
        let others: Vec<SourcedValue> = rest
            .into_iter()
            .filter(|other| {
                shown
                    .as_ref()
                    .is_none_or(|shown| shown.value.trim() != other.value.trim())
            })
            .collect();
        if shown.is_some() || !others.is_empty() {
            out.push(FieldSources {
                field,
                shown,
                others,
            });
        }
    }
    out
}

/// The sources of the tag fields of the track `recording`.
pub fn tag_sources(
    conn: &Connection,
    volumes: &impl Volumes,
    recording: i64,
) -> rusqlite::Result<TagSources> {
    let library: Option<i64> = conn
        .query_row(
            "SELECT id FROM library_track WHERE recording_id = ?1",
            [recording],
            |r| r.get(0),
        )
        .optional()?;
    match library {
        Some(id) => of_library_track(conn, volumes, LibraryTrackId(id)),
        None => of_files(conn, recording),
    }
}

/// A Library track: the rules of a send decide where its values come from.
fn of_library_track(
    conn: &Connection,
    volumes: &impl Volumes,
    id: LibraryTrackId,
) -> rusqlite::Result<TagSources> {
    // A send's scope reads every file's tags and every attribute of the entry.
    let loaded = load(conn, volumes, &[id], Scope::Send, &HashMap::new())?;
    let Some(row) = loaded.rows.get(&id.0) else {
        return Ok(TagSources::default());
    };
    let entry = match loaded.source(row) {
        Source::Known { entry, .. } => Some(entry),
        Source::Files { .. } | Source::Cannot(_) => None,
    };
    let files: Vec<(NamedFile, Vec<tag_fields::TagValue>)> =
        read_tags(&loaded.files, &loaded.tag_order(row), &TAG_ATTRIBUTES)
            .into_iter()
            .map(|(file, values)| {
                let name = loaded.files[&file].name.clone();
                (NamedFile { id: file, name }, values)
            })
            .collect();
    let attributes = entry.and_then(|entry| loaded.attributes.get(&entry));
    Ok(TagSources {
        in_rekordbox: entry.is_some(),
        fields: fields_from(attributes.map(Vec::as_slice), &files),
        confirmed_entry: confirmed_entry(conn, &loaded, row)?,
    })
}

/// The entry the user confirmed to the track's linked file, if the entry is
/// trusted on it and a confirmation of its `Location` stands: what
/// `relink::decide::withdraw` takes away.
fn confirmed_entry(conn: &Connection, loaded: &Loaded, row: &Row) -> rusqlite::Result<Option<i64>> {
    let Some(&(entry, _)) = row.linked_file.and_then(|f| loaded.entry_for.get(&f)) else {
        return Ok(None);
    };
    let confirmed: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM rekordbox_track t
                        JOIN relink r ON r.location_key = t.location_key AND r.verdict = 'same'
                        WHERE t.id = ?1)",
        [entry],
        |r| r.get(0),
    )?;
    Ok(confirmed.then_some(entry))
}

/// A track outside the Library: its own files' tags, best file first.
fn of_files(conn: &Connection, recording: i64) -> rusqlite::Result<TagSources> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.rel_path, f.raw_tags
         FROM recording_file rf JOIN file f ON f.id = rf.file_id
         WHERE rf.recording_id = ?1
         ORDER BY (rf.role = 'best') DESC, f.id",
    )?;
    let files: Vec<(NamedFile, Vec<tag_fields::TagValue>)> = stmt
        .query_map([recording], |r| {
            let rel_path: String = r.get(1)?;
            let raw_tags: Option<String> = r.get(2)?;
            Ok((
                NamedFile {
                    id: r.get(0)?,
                    name: file_name(&rel_path).to_owned(),
                },
                raw_tags
                    .map(|raw| tag_fields::read_only(&raw, &TAG_ATTRIBUTES))
                    .unwrap_or_default(),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(TagSources {
        in_rekordbox: false,
        fields: fields_from(None, &files),
        confirmed_entry: None,
    })
}

/// What the Details panel lists for the track `recording_id`: each tag field
/// with its value and where it comes from, and the confirmation of its
/// rekordbox entry, if one can be withdrawn. A track that doesn't exist has
/// no fields.
#[tauri::command]
#[specta::specta]
pub async fn track_tag_sources(
    reads: tauri::State<'_, crate::db::ReadPool>,
    recording_id: i64,
) -> Result<TagSources, crate::ipc::IpcError> {
    let volumes = crate::scan::system_volumes();
    Ok(reads.read(|conn| tag_sources(conn, &volumes, recording_id))?)
}
