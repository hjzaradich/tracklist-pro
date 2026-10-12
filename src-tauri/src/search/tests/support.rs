//! A small library to search in.

use std::path::{Path, PathBuf};

use crate::db::Writer;
use crate::paths::Volumes;
use crate::search::{find, settle, Found};
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

fn volume_id() -> VolumeId {
    identity(IdentitySignals {
        kind: VolumeKind::External,
        unc_share: None,
        serial: Some(0x1A2B_3C4D),
        filesystem: "NTFS",
        guid: None,
    })
    .unwrap()
}

/// The test volume, mounted at `E:\`.
#[derive(Clone)]
pub struct Mount;

impl Volumes for Mount {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up in these tests",
            path.display()
        )))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (*id == volume_id()).then(|| PathBuf::from(r"E:\"))
    }
}

/// A database with one volume and one music folder, `Music` (id 1).
pub struct Lib {
    pub _dir: tempfile::TempDir,
    pub writer: Writer,
}

/// The tag name a file's `raw_tags` give each field under.
fn frame(field: &str) -> &'static str {
    match field {
        "Name" => "TIT2",
        "Artist" => "TPE1",
        "Album" => "TALB",
        "Remixer" => "TPE4",
        "Label" => "TPUB",
        "Genre" => "TCON",
        "Comments" => "COMM::eng",
        other => panic!("no frame for {other}"),
    }
}

impl Lib {
    pub fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        let identity = volume_id().as_str().to_owned();
        writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES (?1, 'external', 'E:\')",
                    [identity],
                )?;
                c.execute_batch(
                    "INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');",
                )
            })
            .unwrap();
        Lib { _dir: dir, writer }
    }

    pub fn insert(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    pub fn exec(&self, sql: &'static str) {
        self.writer.call(move |c| c.execute_batch(sql)).unwrap();
    }

    /// A track with one best file, `name` inside the music folder, whose
    /// tags give `title` (and `artist`).
    pub fn track(&self, title: Option<&str>, artist: Option<&str>, name: &str) -> (i64, i64) {
        let recording = self.insert("INSERT INTO recording DEFAULT VALUES", ());
        let file = self.file(recording, name, "best");
        let mut fields = Vec::new();
        fields.extend(title.map(|t| ("Name", t)));
        fields.extend(artist.map(|a| ("Artist", a)));
        if !fields.is_empty() {
            self.tag(file, &fields);
        }
        (recording, file)
    }

    /// Another file of `track`.
    pub fn file(&self, track: i64, name: &str, role: &str) -> i64 {
        let file = self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, ?1, ?1)",
            (name.to_owned(),),
        );
        self.insert(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, ?3)",
            (track, file, role.to_owned()),
        );
        file
    }

    /// Sets the tags of `file` to `fields` (`Name`, `Artist`, `Album`,
    /// `Remixer`, `Label`, `Genre`, `Comments`).
    pub fn tag(&self, file: i64, fields: &[(&str, &str)]) {
        let items: Vec<_> = fields
            .iter()
            .map(|(field, text)| {
                serde_json::json!({"key": frame(field), "value": {"type": "text", "text": text}})
            })
            .collect();
        let raw = serde_json::json!({ "id3v2": items }).to_string();
        self.insert("UPDATE file SET raw_tags = ?1 WHERE id = ?2", (raw, file));
    }

    /// A Library track for `track`, linked to `file`.
    pub fn library(&self, track: i64, file: i64) -> i64 {
        self.insert(
            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (?1, 'linked', ?2, 'ok')",
            (track, file),
        )
    }

    /// rekordbox's entry for `file`, with the attributes given.
    pub fn rekordbox_entry(&self, file: i64, attributes: &[(&str, &str)]) -> i64 {
        let mut object = serde_json::Map::new();
        object.insert("TrackID".into(), "7".into());
        object.insert(
            "Location".into(),
            "file://localhost/E:/Music/known.mp3".into(),
        );
        for (name, value) in attributes {
            object.insert((*name).into(), (*value).into());
        }
        let attributes = serde_json::Value::Object(object).to_string();
        self.insert(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable)
             VALUES (?1, 'E:/MUSIC/KNOWN.MP3', '2026-09-30T10:00:00.000Z', ?2, 'path', 0)",
            (attributes, file),
        )
    }

    /// Works out every row that is due, two tracks to a transaction.
    pub fn settle(&self) -> usize {
        self.writer.call(|c| settle(c, &Mount, 2)).unwrap()
    }

    pub fn pending(&self) -> bool {
        self.writer.call(|c| crate::search::pending(c)).unwrap()
    }

    /// What `text` finds, once the index is current.
    pub fn found(&self, text: &str) -> Option<Found> {
        self.settle();
        self.found_as_is(text)
    }

    /// What `text` finds, whether the index is current or not.
    pub fn found_as_is(&self, text: &str) -> Option<Found> {
        let text = text.to_owned();
        self.writer.call(move |c| find(c, &text)).unwrap()
    }

    /// The tracks `text` finds, once the index is current.
    pub fn finds(&self, text: &str) -> Vec<i64> {
        self.found(text).map(|f| f.tracks).unwrap_or_default()
    }

    /// Everything the index holds, as `fts5vocab` lists it: (word, track,
    /// column, offset). Two indexes with the same list answer every query
    /// alike.
    pub fn index_contents(&self) -> Vec<(String, i64, String, i64)> {
        self.settle();
        self.writer
            .call(|c| {
                c.execute_batch(
                    "DROP TABLE IF EXISTS temp.search_vocab;
                     CREATE VIRTUAL TABLE temp.search_vocab
                         USING fts5vocab(main, search_track, instance);",
                )?;
                c.prepare(
                    "SELECT term, doc, col, offset FROM temp.search_vocab ORDER BY 1, 2, 3, 4",
                )?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect()
            })
            .unwrap()
    }
}
