//! Relink tests. Everything is synthetic: made-up volumes, folders, file
//! names and durations, in a migrated database in a temp dir. No file is
//! read, since relink reads only the database.

mod confirmed;
mod general;
mod job;
mod path;
mod recheck;
mod same_name;
mod titles;
mod unique_duration;
mod window;

use rusqlite::params;

use super::{relink, Mounted, Summary};
use crate::db::Writer;
use crate::rekordbox::location;
use crate::volume::{identity, IdentitySignals, VolumeKind};

/// A made-up volume identity, built the way real ones are.
pub(super) fn serial(n: u32) -> String {
    identity(IdentitySignals {
        kind: VolumeKind::External,
        unc_share: None,
        serial: Some(n),
        filesystem: "NTFS",
        guid: None,
    })
    .unwrap()
    .as_str()
    .to_owned()
}

/// A library database to relink in.
pub(super) struct Lib {
    _dir: tempfile::TempDir,
    pub writer: Writer,
    next_track_id: std::cell::Cell<i64>,
}

/// A row's match: file, method, confidence.
pub(super) type Match = (i64, String, f64);

impl Lib {
    pub fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        Lib {
            _dir: dir,
            writer,
            next_track_id: std::cell::Cell::new(1),
        }
    }

    fn exec(&self, sql: &'static str, values: Vec<rusqlite::types::Value>) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, rusqlite::params_from_iter(values))?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// A volume last mounted at `last_mount` (e.g. `E:\`).
    pub fn volume(&self, identity: &str, last_mount: Option<&str>) -> i64 {
        self.exec(
            "INSERT INTO volume (identity, kind, last_mount_path) VALUES (?1, 'external', ?2)",
            vec![
                identity.to_owned().into(),
                last_mount.map(str::to_owned).into(),
            ],
        )
    }

    /// A music folder at `rel` (`/`-separated, `""` for the top) on
    /// `volume`.
    pub fn folder(&self, volume: i64, rel: &str) -> i64 {
        use unicode_normalization::UnicodeNormalization;
        let key: String = rel.nfc().collect();
        self.exec(
            "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (?1, ?2, ?3)",
            vec![volume.into(), rel.to_owned().into(), key.into()],
        )
    }

    /// A present file at `rel` in `folder`, lasting `duration_ms` (`None`:
    /// not read yet).
    pub fn file(&self, folder: i64, rel: &str, duration_ms: Option<i64>) -> i64 {
        use unicode_normalization::UnicodeNormalization;
        let key: String = rel.nfc().collect();
        self.exec(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, duration_ms)
             VALUES (?1, ?2, ?3, 1000, ?4)",
            vec![
                folder.into(),
                rel.to_owned().into(),
                key.into(),
                duration_ms.into(),
            ],
        )
    }

    /// Sets a file's size, as the walk would.
    pub fn size(&self, file: i64, size: i64) {
        self.writer
            .call(move |c| c.execute("UPDATE file SET size = ?2 WHERE id = ?1", [file, size]))
            .unwrap();
    }

    /// Stage 2 read the file: it lasts `duration_ms`.
    pub fn read(&self, file: i64, duration_ms: i64) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET duration_ms = ?2 WHERE id = ?1",
                    [file, duration_ms],
                )
            })
            .unwrap();
    }

    /// Stage 2 read the file's tags: `raw_tags` as stored.
    pub fn tags(&self, file: i64, raw_tags: &str) {
        let raw_tags = raw_tags.to_owned();
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET raw_tags = ?2 WHERE id = ?1",
                    params![file, raw_tags],
                )
            })
            .unwrap();
    }

    /// Gives the file an ID3v2 title tag.
    pub fn title(&self, file: i64, title: &str) {
        let tags = serde_json::json!({
            "id3v2": [{"key": "TIT2", "value": {"type": "text", "text": title}}]
        });
        self.tags(file, &tags.to_string());
    }

    /// The walk found the file online-only (a OneDrive placeholder), so
    /// stage 2 never read it.
    pub fn online_only(&self, file: i64) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET online_only = 1, duration_ms = NULL WHERE id = ?1",
                    [file],
                )
            })
            .unwrap();
    }

    /// Whether a row's match is only probable.
    pub fn probable(&self, track: i64) -> bool {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT relink_probable FROM rekordbox_track WHERE id = ?1",
                    [track],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// Stage 3 hashed the file's audio: `hash` stands for its audio_hash.
    pub fn audio_hash(&self, file: i64, hash: u8) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET audio_hash = ?2 WHERE id = ?1",
                    params![file, vec![hash; 16]],
                )
            })
            .unwrap();
    }

    /// A new track (`recording`) holding `files`, the first one as its
    /// best file. Returns the track's id.
    pub fn group(&self, files: &[i64]) -> i64 {
        let files = files.to_vec();
        self.writer
            .call(move |c| {
                c.execute("INSERT INTO recording DEFAULT VALUES", [])?;
                let recording = c.last_insert_rowid();
                for (n, file) in files.iter().enumerate() {
                    let role = if n == 0 { "best" } else { "undecided" };
                    c.execute(
                        "INSERT INTO recording_file (recording_id, file_id, role)
                         VALUES (?1, ?2, ?3)",
                        params![recording, file, role],
                    )?;
                }
                Ok(recording)
            })
            .unwrap()
    }

    /// A row's `recording_id`.
    pub fn recording(&self, track: i64) -> Option<i64> {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT recording_id FROM rekordbox_track WHERE id = ?1",
                    [track],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// The walk found the file gone.
    pub fn gone(&self, file: i64) {
        self.writer
            .call(move |c| c.execute("UPDATE file SET present = 0 WHERE id = ?1", [file]))
            .unwrap();
    }

    /// A rekordbox track at `location` (as rekordbox writes it) lasting
    /// `total_time` whole seconds, named `Track <n>`.
    pub fn track(&self, location: &str, total_time: Option<&str>) -> i64 {
        self.track_with(location, total_time, &[])
    }

    /// [`Lib::track`] with more attributes.
    pub fn track_with(
        &self,
        location: &str,
        total_time: Option<&str>,
        more: &[(&str, &str)],
    ) -> i64 {
        let id = self.next_track_id.get();
        self.next_track_id.set(id + 1);
        let mut attrs = serde_json::Map::new();
        attrs.insert("TrackID".into(), id.to_string().into());
        if !more.iter().any(|(k, _)| *k == "Name") {
            attrs.insert("Name".into(), format!("Track {id}").into());
        }
        if let Some(t) = total_time {
            attrs.insert("TotalTime".into(), t.into());
        }
        for (k, v) in more {
            attrs.insert((*k).into(), (*v).into());
        }
        attrs.insert("Location".into(), location.into());
        let key = location::decode(location).unwrap().match_key();
        self.exec(
            "INSERT INTO rekordbox_track (attributes, location_key, read_at)
             VALUES (?1, ?2, '2026-09-29T12:00:00.000Z')",
            vec![
                serde_json::Value::Object(attrs).to_string().into(),
                key.into(),
            ],
        )
    }

    /// Marks a row matched as an earlier run would have.
    pub fn matched_before(&self, track: i64, file: i64, method: &str, confidence: f64) {
        let method = method.to_owned();
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE rekordbox_track
                     SET file_id = ?2, relink_method = ?3, relink_confidence = ?4 WHERE id = ?1",
                    params![track, file, method, confidence],
                )
            })
            .unwrap();
    }

    /// The user confirmed `location`'s file is `file`.
    pub fn confirm(&self, location: &str, file: i64, method: &str) {
        let key = location::decode(location).unwrap().match_key();
        self.exec(
            "INSERT INTO relink (location_key, file_id, method, confidence) VALUES (?1, ?2, ?3, 0.5)",
            vec![key.into(), file.into(), method.to_owned().into()],
        );
    }

    pub fn relink(&self, mounted: &Mounted) -> Summary {
        let mounted = mounted.clone();
        self.writer.call(move |c| relink(c, &mounted)).unwrap()
    }

    /// A row's match.
    pub fn matched(&self, track: i64) -> Option<Match> {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT file_id, relink_method, relink_confidence FROM rekordbox_track
                     WHERE id = ?1",
                    [track],
                    |r| {
                        Ok(match r.get::<_, Option<i64>>(0)? {
                            Some(file) => Some((file, r.get(1)?, r.get(2)?)),
                            None => None,
                        })
                    },
                )
            })
            .unwrap()
    }

    /// The method a row was matched by, if any.
    pub fn method(&self, track: i64) -> Option<String> {
        self.matched(track).map(|(_, m, _)| m)
    }

    /// Every row's match, for comparing whole states.
    pub fn all_matches(&self) -> Vec<(i64, Option<Match>)> {
        self.writer
            .call(|c| {
                c.prepare(
                    "SELECT id, file_id, relink_method, relink_confidence FROM rekordbox_track
                     ORDER BY id",
                )?
                .query_map([], |r| {
                    let m = match r.get::<_, Option<i64>>(1)? {
                        Some(file) => Some((file, r.get(2)?, r.get(3)?)),
                        None => None,
                    };
                    Ok((r.get(0)?, m))
                })?
                .collect()
            })
            .unwrap()
    }
}

/// A library with one volume plugged in at `E:\` and a music folder at
/// `E:\Music`. Returns the library, the folder and what's mounted.
pub(super) fn e_music() -> (Lib, i64, Mounted) {
    let lib = Lib::new();
    let e = serial(1);
    let vol = lib.volume(&e, Some(r"E:\"));
    let music = lib.folder(vol, "Music");
    (lib, music, Mounted::new([(e, r"E:\")]))
}

/// `file://localhost/` plus `path`, with spaces escaped as rekordbox does.
pub(super) fn loc(path: &str) -> String {
    format!("file://localhost/{}", path.replace(' ', "%20"))
}

pub(super) fn path(file: i64) -> Option<Match> {
    Some((file, "path".to_owned(), 1.0))
}
