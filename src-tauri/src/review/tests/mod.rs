//! Review queue tests. Everything is made up: a volume, a music folder, file
//! and track names, and the rows grouping, the roles job and the quality job
//! would have written, set by SQL, in a migrated database in a temp dir.

mod actions;
mod compare;
mod held_reason;
mod ipc;
mod plan;
mod queue;
mod same;
mod split;

use std::path::{Path, PathBuf};

use super::*;
use crate::db::Writer;
use crate::paths::Volumes;
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

/// The made-up volume, mounted at `E:\`.
pub(crate) struct Mount;

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
pub(crate) struct Lib {
    _dir: tempfile::TempDir,
    pub writer: Writer,
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
                     VALUES (?1, 'external', 'E:\\')",
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

    pub fn exec(&self, sql: &str) {
        let sql = sql.to_owned();
        self.writer
            .call(move |c| c.execute_batch(&sql))
            .unwrap_or_else(|e| panic!("{e:?}"));
    }

    fn insert(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// A track with a made-up title (and the artist "Someone").
    pub fn track(&self, title: &str) -> i64 {
        self.insert(
            "INSERT INTO recording (title, artist) VALUES (?1, 'Someone')",
            (title.to_owned(),),
        )
    }

    /// A file of `track`: `format` is `mp3` (320 kbps) or `flac`.
    pub fn file(&self, track: i64, name: &str, format: &str, present: bool) -> i64 {
        let (codec, bitrate) = match format {
            "flac" => ("flac", None),
            _ => ("mp3", Some(320)),
        };
        let file = self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, sniffed_format,
                               codec, bitrate, duration_ms)
             VALUES (1, ?1, ?1, ?2, ?3, ?4, ?5, 215000)",
            (
                name.to_owned(),
                present,
                format.to_owned(),
                codec.to_owned(),
                bitrate,
            ),
        );
        self.insert(
            "INSERT INTO recording_file (recording_id, file_id) VALUES (?1, ?2)",
            (track, file),
        );
        file
    }

    /// A track with one mp3 on disk.
    pub fn simple(&self, title: &str) -> (i64, i64) {
        let track = self.track(title);
        let file = self.file(track, &format!("{title}.mp3"), "mp3", true);
        (track, file)
    }

    /// Puts `track` in the Library, linked to `file`.
    pub fn library(&self, track: i64, file: i64) {
        self.insert(
            "INSERT INTO library_track (recording_id, kind, linked_file_id)
             VALUES (?1, 'linked', ?2)",
            (track, file),
        );
    }

    /// A stored grouping pair, the lower file id first.
    pub fn pair(&self, x: i64, y: i64, outcome: &str, state: &str) {
        let (a, b) = (x.min(y), x.max(y));
        self.insert(
            "INSERT INTO grouping_pair (file_a, file_b, outcome, reasons, state)
             VALUES (?1, ?2, ?3,
                     '[{\"code\":\"sameAudioNamesDisagree\"},{\"code\":\"namesDiffer\"}]', ?4)",
            (a, b, outcome.to_owned(), state.to_owned()),
        );
    }

    pub fn open_pair(&self, x: i64, y: i64) {
        self.pair(x, y, "review", "open");
    }

    pub fn better(&self, track: i64, file: i64, step: &str) {
        self.insert(
            "INSERT INTO better_file (recording_id, file_id, step) VALUES (?1, ?2, ?3)",
            (track, file, step.to_owned()),
        );
    }

    /// A current stored verdict, as the quality job would leave it.
    pub fn verdict(&self, file: i64, verdict: &str) {
        let sql = format!(
            "INSERT INTO file_verdict (file_id, rules_version, inputs, verdict, findings)
             SELECT f.id, {}, {}, ?2, '[]' FROM file f
             LEFT JOIN file_quality q ON q.file_id = f.id WHERE f.id = ?1",
            crate::quality::RULES_VERSION,
            crate::quality::store::INPUTS
        );
        let verdict = verdict.to_owned();
        self.writer
            .call(move |c| c.execute(&sql, (file, verdict)))
            .unwrap();
    }

    /// A rekordbox entry, matched to `file` (probably or trustedly) or to
    /// none.
    pub fn rekordbox(&self, file: Option<i64>, method: &str, probable: bool) -> i64 {
        let n = self.num("SELECT count(*) FROM rekordbox_track") + 1;
        let method = file.map(|_| method.to_owned());
        self.insert(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable)
             VALUES (json_object('TrackID', ?1, 'Name', 'Old name', 'Artist', 'Someone',
                                 'TotalTime', '215', 'Location', 'file://localhost/E:/Old/old.mp3'),
                     'E:/Old/old.mp3', '2026-09-30T10:00:00.000Z', ?2, ?3, ?4)",
            (n.to_string(), file, method, probable),
        )
    }

    pub fn num(&self, sql: &str) -> i64 {
        let sql = sql.to_owned();
        self.writer
            .call(move |c| c.query_row(&sql, [], |r| r.get(0)))
            .unwrap()
    }

    pub fn counts(&self) -> ReviewCounts {
        self.writer.call(|c| counts(c)).unwrap()
    }

    pub fn page(&self, group: ReviewGroup, offset: u32, limit: u32) -> ReviewPage {
        self.writer
            .call(move |c| items(c, &Mount, group, offset, limit))
            .unwrap()
    }

    /// Every item of `group`, as (kind, a, b).
    pub fn ids(&self, group: ReviewGroup) -> Vec<(ReviewKind, i64, Option<i64>)> {
        self.page(group, 0, 1000)
            .items
            .into_iter()
            .map(|i| (i.id.kind, i.id.a, i.id.b))
            .collect()
    }

    pub fn compare(&self, id: ReviewItemId) -> Option<ReviewComparison> {
        self.writer
            .call(move |c| comparison(c, &Mount, id))
            .unwrap()
    }
}

pub(crate) fn id(kind: ReviewKind, a: i64, b: Option<i64>) -> ReviewItemId {
    ReviewItemId { kind, a, b }
}
