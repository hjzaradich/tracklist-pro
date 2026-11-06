//! What the plain copy holds, as the XML read would produce it. The
//! database's tracks and playlists are written out as a rekordbox XML
//! document in memory, which the XML reader itself then reads, so every
//! value goes through the same parsing, skipping and warnings as an
//! export's, and nothing downstream can tell the two apart.
//!
//! Only what an export carries is read: the track fields below, and the
//! playlist tree. Beat grids and cue points are not in the database's
//! tables (they live in rekordbox's analysis files), and intelligent
//! playlists hold no entries of their own, so neither is read.

use std::collections::{HashMap, HashSet};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, Row};

use crate::rekordbox::{RekordboxXml, XmlError};

#[derive(Debug)]
pub(super) enum LibraryError {
    /// The tables aren't rekordbox's.
    NotALibrary(rusqlite::Error),
    /// The document didn't read back (a bug here, never the user's file).
    Xml(XmlError),
}

/// Folders deeper than this are left out.
const MAX_DEPTH: usize = 64;

const TRACKS: &str = "
SELECT c.ID, c.FolderPath, c.Title, artist.Name, composer.Name, album.Name, genre.Name,
       c.FileType, c.FileSize, c.Length, c.DiscNo, c.TrackNo, c.ReleaseYear, c.BPM,
       c.StockDate, c.DateCreated, c.BitRate, c.SampleRate, c.Commnt, c.DJPlayCount,
       c.Rating, remixer.Name, k.ScaleName, label.Name, c.Subtitle, color.ColorCode
FROM djmdContent c
LEFT JOIN djmdArtist artist ON artist.ID = c.ArtistID
LEFT JOIN djmdArtist composer ON composer.ID = c.ComposerID
LEFT JOIN djmdAlbum album ON album.ID = c.AlbumID
LEFT JOIN djmdGenre genre ON genre.ID = c.GenreID
LEFT JOIN djmdArtist remixer ON remixer.ID = c.RemixerID
LEFT JOIN djmdKey k ON k.ID = c.KeyID
LEFT JOIN djmdLabel label ON label.ID = c.LabelID
LEFT JOIN djmdColor color ON color.ID = c.ColorID
WHERE c.rb_local_deleted = 0
ORDER BY c.rowid";

const PLAYLISTS: &str = "
SELECT ID, ParentID, Name, Attribute FROM djmdPlaylist
WHERE rb_local_deleted = 0 ORDER BY Seq, rowid";

const ENTRIES: &str = "
SELECT PlaylistID, ContentID FROM djmdSongPlaylist
WHERE rb_local_deleted = 0 ORDER BY TrackNo, rowid";

/// Reads the library on `conn` (the plain copy, read-only).
pub(super) fn read(conn: &Connection) -> Result<RekordboxXml, LibraryError> {
    let document = document(conn).map_err(LibraryError::NotALibrary)?;
    RekordboxXml::parse(document.as_bytes()).map_err(LibraryError::Xml)
}

/// The library as a rekordbox XML document.
fn document(conn: &Connection) -> rusqlite::Result<String> {
    let mut tracks = String::new();
    let mut count = 0usize;
    let mut statement = conn.prepare(TRACKS)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        track(&mut tracks, row)?;
        count += 1;
    }
    let playlists = playlists(conn)?;
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DJ_PLAYLISTS Version=\"1.0.0\">\n\
         <PRODUCT Name=\"rekordbox\" Version=\"\" Company=\"\"/>\n\
         <COLLECTION Entries=\"{count}\">\n{tracks}</COLLECTION>\n\
         <PLAYLISTS>\n{playlists}</PLAYLISTS>\n</DJ_PLAYLISTS>\n"
    ))
}

/// A value as text, whatever its column type.
fn text(row: &Row, i: usize) -> rusqlite::Result<Option<String>> {
    Ok(match row.get_ref(i)? {
        ValueRef::Integer(n) => Some(n.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Null | ValueRef::Blob(_) => None,
    })
}

/// A value as a whole number, whatever its column type.
fn number(row: &Row, i: usize) -> rusqlite::Result<Option<i64>> {
    Ok(match row.get_ref(i)? {
        ValueRef::Integer(n) => Some(n),
        ValueRef::Real(f) => Some(f as i64),
        ValueRef::Text(t) => std::str::from_utf8(t)
            .ok()
            .and_then(|s| s.trim().parse().ok()),
        ValueRef::Null | ValueRef::Blob(_) => None,
    })
}

/// rekordbox's `Kind` for its file type numbers.
fn kind(file_type: i64) -> Option<&'static str> {
    Some(match file_type {
        1 => "MP3 File",
        4 => "M4A File",
        5 => "FLAC File",
        11 => "WAV File",
        12 => "AIFF File",
        _ => return None,
    })
}

/// One `TRACK`, with the attributes an export gives every track.
fn track(out: &mut String, row: &Row) -> rusqlite::Result<()> {
    let t = |i| text(row, i).map(Option::unwrap_or_default);
    let n = |i| number(row, i).map(|v| v.unwrap_or(0).to_string());
    let bpm = number(row, 13)?.unwrap_or(0).max(0);
    let added = text(row, 14)?
        .filter(|s| !s.is_empty())
        .or(text(row, 15)?)
        .unwrap_or_default();
    let stars = number(row, 20)?.unwrap_or(0).max(0);
    let rating = if stars <= 5 {
        stars * 51
    } else {
        stars.min(255)
    };
    let mut attrs: Vec<(&str, String)> = vec![
        ("TrackID", t(0)?),
        ("Name", t(2)?),
        ("Artist", t(3)?),
        ("Composer", t(4)?),
        ("Album", t(5)?),
        ("Genre", t(6)?),
    ];
    if let Some(kind) = number(row, 7)?.and_then(kind) {
        attrs.push(("Kind", kind.to_owned()));
    }
    attrs.extend([
        ("Size", n(8)?),
        ("TotalTime", n(9)?),
        ("DiscNumber", n(10)?),
        ("TrackNumber", n(11)?),
        ("Year", n(12)?),
        ("AverageBpm", format!("{}.{:02}", bpm / 100, bpm % 100)),
        ("DateAdded", added.chars().take(10).collect()),
        ("BitRate", n(16)?),
        ("SampleRate", n(17)?),
        ("Comments", t(18)?),
        ("PlayCount", n(19)?),
        ("Rating", rating.to_string()),
        ("Location", location(&t(1)?)),
        ("Remixer", t(21)?),
        ("Tonality", t(22)?),
        ("Label", t(23)?),
        ("Mix", t(24)?),
    ]);
    if let Some(color) = number(row, 25)? {
        attrs.push(("Colour", format!("0x{:06X}", color & 0xFF_FFFF)));
    }
    out.push_str("<TRACK");
    for (name, value) in &attrs {
        attribute(out, name, value);
    }
    out.push_str("/>\n");
    Ok(())
}

/// A file path as an export's `Location`: `file://localhost`, then the
/// path's UTF-8 with everything but letters, digits and `-._~/:` escaped.
/// A drive path takes the `/` after the host; others bring their own.
fn location(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut out = String::from("file://localhost");
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        out.push('/');
    }
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// ` name="value"`, escaped for XML. Characters XML can't hold are left out.
fn attribute(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if c < ' ' || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A playlist or folder of the tree.
struct Node {
    id: String,
    name: String,
    folder: bool,
}

/// The tree under rekordbox's `ROOT` folder.
fn playlists(conn: &Connection) -> rusqlite::Result<String> {
    let mut children: HashMap<String, Vec<Node>> = HashMap::new();
    let mut statement = conn.prepare(PLAYLISTS)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        // 0 is a playlist and 1 a folder; intelligent playlists (4) and
        // anything else are left out.
        let folder = match number(row, 3)? {
            Some(0) => false,
            Some(1) => true,
            _ => continue,
        };
        let node = Node {
            id: text(row, 0)?.unwrap_or_default(),
            name: text(row, 2)?.unwrap_or_default(),
            folder,
        };
        let parent = text(row, 1)?.unwrap_or_default();
        children.entry(parent).or_default().push(node);
    }
    let mut entries: HashMap<String, Vec<String>> = HashMap::new();
    let mut statement = conn.prepare(ENTRIES)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let (Some(list), Some(track)) = (text(row, 0)?, text(row, 1)?) else {
            continue;
        };
        entries.entry(list).or_default().push(track);
    }
    let tree = Tree { children, entries };
    let mut inside = String::new();
    let count = tree.write(&mut inside, "root", &mut HashSet::new(), 1);
    Ok(format!(
        "<NODE Type=\"0\" Name=\"ROOT\" Count=\"{count}\">\n{inside}</NODE>\n"
    ))
}

struct Tree {
    children: HashMap<String, Vec<Node>>,
    entries: HashMap<String, Vec<String>>,
}

impl Tree {
    /// Writes the nodes under `parent`; returns how many. A node is
    /// written once, so a loop in the parents ends.
    fn write(
        &self,
        out: &mut String,
        parent: &str,
        seen: &mut HashSet<String>,
        depth: usize,
    ) -> usize {
        let mut count = 0;
        for node in self.children.get(parent).into_iter().flatten() {
            if (node.folder && depth >= MAX_DEPTH) || !seen.insert(node.id.clone()) {
                continue;
            }
            out.push_str("<NODE");
            attribute(out, "Name", &node.name);
            if node.folder {
                let mut inside = String::new();
                let n = self.write(&mut inside, &node.id, seen, depth + 1);
                out.push_str(&format!(" Type=\"0\" Count=\"{n}\">\n{inside}</NODE>\n"));
            } else {
                let tracks = self.entries.get(&node.id).map_or(&[][..], Vec::as_slice);
                out.push_str(&format!(
                    " Type=\"1\" KeyType=\"0\" Entries=\"{}\">\n",
                    tracks.len()
                ));
                for track in tracks {
                    out.push_str("<TRACK");
                    attribute(out, "Key", track);
                    out.push_str("/>\n");
                }
                out.push_str("</NODE>\n");
            }
            count += 1;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_becomes_an_export_location_the_reader_decodes_to_the_same_path() {
        let raw = location("C:/Music/Kit & Kin/Gone #1 (live) ü.mp3");
        assert_eq!(
            raw,
            "file://localhost/C:/Music/Kit%20%26%20Kin/Gone%20%231%20%28live%29%20%C3%BC.mp3"
        );
        let decoded = crate::rekordbox::location::decode(&raw).unwrap();
        assert_eq!(
            decoded.as_file().unwrap().as_str(),
            "C:/Music/Kit & Kin/Gone #1 (live) ü.mp3"
        );
        assert_eq!(
            location("C:\\Music\\a.mp3"),
            "file://localhost/C:/Music/a.mp3"
        );
    }

    #[test]
    fn values_are_escaped_and_characters_xml_cannot_hold_are_left_out() {
        let mut out = String::new();
        attribute(&mut out, "Name", "a&b<c>\"d\"\u{1}\te");
        assert_eq!(out, " Name=\"a&amp;b&lt;c&gt;&quot;d&quot;&#9;e\"");
    }
}
