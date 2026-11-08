//! The offline sources: what a Library track's own files say, read from
//! what the scan stored (`file.raw_tags`, the file's name). No audio file
//! is opened and nothing is written.
//!
//! - **The linked file's tags** ([`IdentifySource::Tags`]): `Name`,
//!   `Artist`, `Album`, `Remixer`, `Label` and `Year` as a send maps them
//!   (`send_values::tag_values`), plus the version written in the title tag
//!   ("Velo (Extended Mix)": `Mix` "Extended Mix") and the remixer it names
//!   ("(Kit Remix)": `Remixer` "Kit"), read by the name parser
//!   (`versions::parse_title`). Only markers the parser is sure of count.
//! - **The file's name** ([`IdentifySource::FileName`]), by
//!   `versions::parse_file_name` ("Artist - Title (Mix).mp3"): `Name` (the
//!   title with its version as written, by the convention in [`super`]),
//!   `Artist`, `Mix`, `Remixer`, `Year` (a "(2011)" bracket) and the
//!   catalogue number (a "[ABC123]" bracket). Junk such as a track number
//!   or the extension is left out.
//! - **Sibling files** ([`IdentifySource::Sibling`]): the same reading of
//!   the tags of the track's other files that are on disk and not hidden,
//!   best file first; their names count as [`IdentifySource::FileName`].
//!
//! The catalogue number has no tag source: the tag reader maps no tag to it
//! (`send_values` `TAG_ATTRIBUTES`), so only a file's name can give one.
//! Tags that can't be read give nothing, and the other files still count.

use rusqlite::{Connection, OptionalExtension};

use super::{Candidate, IdentifyField, IdentifySource};
use crate::library::LibraryTrackId;
use crate::send_values::{file_name, tag_values, NamedFile};
use crate::versions::{parse_file_name, parse_title, CreditRole, MarkerKind, ParsedName};

/// The tag attributes read, as a send maps them.
const TAG_FIELDS: [&str; 6] = ["Name", "Artist", "Album", "Remixer", "Label", "Year"];

/// One of the track's files, as the scan stored it.
struct StoredFile {
    file: NamedFile,
    raw_tags: Option<String>,
}

fn stored(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredFile> {
    let id: i64 = r.get(0)?;
    let path: String = r.get(1)?;
    Ok(StoredFile {
        file: NamedFile {
            id,
            name: file_name(&path).to_owned(),
        },
        raw_tags: r.get(2)?,
    })
}

/// Every offline candidate for the Library track `track`, or `None` when
/// there's no such Library track.
pub(super) fn candidates(
    conn: &Connection,
    track: LibraryTrackId,
) -> rusqlite::Result<Option<Vec<Candidate>>> {
    let row: Option<(i64, Option<i64>)> = conn
        .query_row(
            "SELECT recording_id, linked_file_id FROM library_track WHERE id = ?1",
            [track.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((recording, linked)) = row else {
        return Ok(None);
    };
    let mut out = Vec::new();
    if let Some(linked) = linked {
        let file = conn
            .query_row(
                "SELECT id, rel_path, raw_tags FROM file WHERE id = ?1",
                [linked],
                stored,
            )
            .optional()?;
        if let Some(file) = file {
            from_tags(
                &file,
                IdentifySource::Tags {
                    file: file.file.clone(),
                },
                &mut out,
            );
            from_name(&file.file, &mut out);
        }
    }
    let mut stmt = conn.prepare_cached(
        "SELECT f.id, f.rel_path, f.raw_tags FROM recording_file rf
         JOIN file f ON f.id = rf.file_id
         WHERE rf.recording_id = ?1 AND f.id IS NOT ?2
           AND f.present = 1 AND f.hidden_reason IS NULL
         ORDER BY rf.role = 'best' DESC, f.id",
    )?;
    let siblings = stmt
        .query_map((recording, linked), stored)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for file in &siblings {
        from_tags(
            file,
            IdentifySource::Sibling {
                file: file.file.clone(),
            },
            &mut out,
        );
        from_name(&file.file, &mut out);
    }
    Ok(Some(out))
}

fn push(out: &mut Vec<Candidate>, field: IdentifyField, value: String, source: &IdentifySource) {
    if !value.trim().is_empty() {
        out.push(Candidate {
            field,
            value,
            source: source.clone(),
        });
    }
}

/// A file's tags, given as `source`.
fn from_tags(file: &StoredFile, source: IdentifySource, out: &mut Vec<Candidate>) {
    let Some(raw) = &file.raw_tags else { return };
    for tag in tag_values(raw, &TAG_FIELDS) {
        if let Some(field) = IdentifyField::of_attribute(tag.attribute) {
            if tag.attribute == "Name" {
                let parsed = parse_title(&tag.value);
                push_version(&parsed, &source, out);
            }
            push(out, field, tag.value, &source);
        }
    }
}

/// A file's name.
fn from_name(file: &NamedFile, out: &mut Vec<Candidate>) {
    let source = IdentifySource::FileName { file: file.clone() };
    let parsed = parse_file_name(&file.name);
    push(out, IdentifyField::Name, titled(&parsed), &source);
    let named = |role| {
        parsed
            .credits
            .iter()
            .filter(|c| c.role == role)
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(" & ")
    };
    let (main, featured) = (named(CreditRole::Main), named(CreditRole::Featured));
    if !main.is_empty() {
        let artist = if featured.is_empty() {
            main
        } else {
            format!("{main} feat. {featured}")
        };
        push(out, IdentifyField::Artist, artist, &source);
    }
    push_version(&parsed, &source, out);
    for aside in &parsed.asides {
        let field = match aside.kind {
            crate::versions::AsideKind::Year => IdentifyField::Year,
            crate::versions::AsideKind::CatalogueNumber => IdentifyField::CatalogueNumber,
            _ => continue,
        };
        push(out, field, unbracketed(&aside.text), &source);
    }
}

/// The version a name says (`Mix`) and the remixer it names.
fn push_version(parsed: &ParsedName, source: &IdentifySource, out: &mut Vec<Candidate>) {
    let sure: Vec<_> = parsed.markers.iter().filter(|m| !m.ambiguous).collect();
    if sure.is_empty() {
        return;
    }
    let mix = distinct_texts(sure.iter().copied())
        .map(unbracketed)
        .collect::<Vec<_>>()
        .join(" ");
    push(out, IdentifyField::Mix, mix, source);
    for marker in sure {
        let reworked = matches!(
            marker.kind,
            MarkerKind::Remix
                | MarkerKind::Rework
                | MarkerKind::Vip
                | MarkerKind::Flip
                | MarkerKind::Bootleg
                | MarkerKind::Edit
        );
        if let (true, Some(by)) = (reworked, &marker.detail) {
            push(out, IdentifyField::Remixer, by.clone(), source);
        }
    }
}

/// The texts of `markers` as written, each once in a row (one bracket can
/// hold several markers: "(Extended Instrumental Mix)").
fn distinct_texts<'a>(
    markers: impl Iterator<Item = &'a crate::versions::Marker>,
) -> impl Iterator<Item = &'a str> {
    let mut last: Option<&str> = None;
    markers.filter_map(move |m| {
        let text = m.text.as_str();
        (last.replace(text) != Some(text)).then_some(text)
    })
}

/// A name's title with its version as written ("Velo (Extended Mix)", "Velo
/// - Radio Edit"): the `Name` convention of [`super`].
fn titled(parsed: &ParsedName) -> String {
    let mut title = parsed.base_title.clone();
    let mut last: Option<&str> = None;
    for marker in &parsed.markers {
        if last.replace(marker.text.as_str()) == Some(marker.text.as_str()) {
            continue;
        }
        if marker.form == crate::versions::MarkerForm::DashSegment {
            title.push_str(" - ");
        } else {
            title.push(' ');
        }
        title.push_str(marker.text.trim());
    }
    title
}

/// A marker or bracket as written, without its brackets or a leading " - ".
fn unbracketed(text: &str) -> String {
    text.trim()
        .trim_start_matches(['-', '(', '[', '{'])
        .trim_end_matches([')', ']', '}'])
        .trim()
        .to_owned()
}
