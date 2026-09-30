//! A migrated database in a temp dir, with `file` rows made by hand.

use crate::db::Writer;
use crate::grouping::{regroup, Summary};

/// Two different 34-byte audio hashes, and a third, for tests that need
/// distinct audio.
pub const A: [u8; 34] = [0xA1; 34];
pub const B: [u8; 34] = [0xB2; 34];
pub const C: [u8; 34] = [0xC3; 34];

pub struct Db {
    /// Keeps a temp dir alive.
    pub dir: Option<tempfile::TempDir>,
    pub writer: Writer,
}

/// One volume and two music folders (ids 1 and 2).
pub fn db() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let mut db = db_in(&path);
    db.dir = Some(dir);
    db
}

/// [`db`], with the database in `data`.
pub fn db_in(data: &std::path::Path) -> Db {
    let writer = Writer::open(&crate::write_guard::test_path(
        data,
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    writer
        .call(|c| {
            c.execute_batch(
                "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'A', 'A');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'B', 'B');",
            )
        })
        .unwrap();
    Db { dir: None, writer }
}

impl Db {
    /// A present file in `folder`, hashed as `hash` (`None`: not hashed).
    pub fn file(&self, folder: i64, name: &str, hash: Option<[u8; 34]>) -> i64 {
        let name = name.to_string();
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime, audio_hash)
                     VALUES (?1, ?2, ?2, 100, 1000, ?3)",
                    (folder, name, hash.map(|h| h.to_vec())),
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    pub fn set_hash(&self, file: i64, hash: Option<[u8; 34]>) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET audio_hash = ?2 WHERE id = ?1",
                    (file, hash.map(|h| h.to_vec())),
                )
            })
            .unwrap();
    }

    pub fn set_present(&self, file: i64, present: bool) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET present = ?2 WHERE id = ?1",
                    (file, present),
                )
            })
            .unwrap();
    }

    pub fn group(&self) -> Summary {
        self.writer.call(regroup).unwrap()
    }

    /// The track holding `file`.
    pub fn track(&self, file: i64) -> Option<i64> {
        self.query(
            "SELECT recording_id FROM recording_file WHERE file_id = ?1",
            file,
        )
    }

    pub fn role(&self, file: i64) -> String {
        self.query("SELECT role FROM recording_file WHERE file_id = ?1", file)
            .unwrap()
    }

    fn query<T: rusqlite::types::FromSql + Send + 'static>(
        &self,
        sql: &'static str,
        arg: i64,
    ) -> Option<T> {
        use rusqlite::OptionalExtension;
        self.writer
            .call(move |c| c.query_row(sql, [arg], |r| r.get(0)).optional())
            .unwrap()
    }

    pub fn count(&self, sql: &str) -> i64 {
        let sql = sql.to_string();
        self.writer
            .call(move |c| c.query_row(&sql, [], |r| r.get(0)))
            .unwrap()
    }

    pub fn tracks(&self) -> i64 {
        self.count("SELECT count(*) FROM recording")
    }

    /// Runs SQL that must succeed.
    pub fn sql(&self, sql: &str) {
        let sql = sql.to_string();
        self.writer.call(move |c| c.execute_batch(&sql)).unwrap();
    }

    /// Every file's track, as sorted groups of file ids.
    pub fn groups(&self) -> Vec<Vec<i64>> {
        let rows: Vec<(i64, i64)> = self
            .writer
            .call(|c| {
                let mut s = c.prepare("SELECT recording_id, file_id FROM recording_file")?;
                let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .unwrap();
        let mut by: std::collections::BTreeMap<i64, Vec<i64>> = Default::default();
        for (t, f) in rows {
            by.entry(t).or_default().push(f);
        }
        let mut groups: Vec<Vec<i64>> = by.into_values().collect();
        for g in &mut groups {
            g.sort();
        }
        groups.sort();
        groups
    }
}
