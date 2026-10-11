//! A migrated database in a temp dir, with `file` rows made by hand.

use std::sync::Mutex;

use crate::db::{DbError, Writer};
use crate::fingerprint::Fingerprint;
use crate::grouping::{decide, decide_and_group, regroup, Decided, Names, Summary, PAGE};

pub use super::synthetic::{reencoded, track, worn};

/// Two different 34-byte audio hashes, and a third, for tests that need
/// distinct audio.
pub const A: [u8; 34] = [0xA1; 34];
pub const B: [u8; 34] = [0xB2; 34];
pub const C: [u8; 34] = [0xC3; 34];

/// A 34-byte audio hash that is `n` all through: distinct audio per `n`.
pub fn hash(n: u8) -> [u8; 34] {
    [n; 34]
}

/// `items` with one bit flipped in every item: the same audio to a
/// comparison (a difference score of 1), with not one item the same bit
/// for bit, so the matching pass's index has nothing to propose the pair
/// by.
pub fn unproposed(items: &[u32], seed: u64) -> Vec<u32> {
    let mut rng = super::synthetic::Rng(seed ^ 0x51DE);
    items
        .iter()
        .map(|item| item ^ (1 << rng.below(32)))
        .collect()
}

pub struct Db {
    /// Keeps a temp dir alive.
    pub dir: Option<tempfile::TempDir>,
    pub writer: Writer,
    /// The deciding run's index of names, kept between runs as the job's
    /// handler keeps it.
    pub names: Mutex<Names>,
}

/// A file to add: its name on disk, its tags, its audio and its
/// fingerprint.
#[derive(Default, Clone)]
pub struct NewFile<'a> {
    pub name: &'a str,
    pub title: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub hash: Option<[u8; 34]>,
    pub items: Option<Vec<u32>>,
    /// Its whole-file hash, as one byte repeated: what names the audio of a
    /// file with no `hash`.
    pub blake3: Option<u8>,
}

/// The `raw_tags` of an MP3 with these two tags.
fn raw_tags(title: Option<&str>, artist: Option<&str>) -> String {
    let frame = |key: &str, text: &str| serde_json::json!({ "key": key, "value": { "type": "text", "text": text } });
    let frames: Vec<serde_json::Value> = [("TIT2", title), ("TPE1", artist)]
        .into_iter()
        .filter_map(|(key, text)| Some(frame(key, text?)))
        .collect();
    serde_json::json!({ "id3v2": frames }).to_string()
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
    Db {
        dir: None,
        writer,
        names: Mutex::default(),
    }
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

    /// A present file in folder 1 with tags, audio and a fingerprint, as
    /// the read, hash and fingerprint stages leave one: the fingerprint is
    /// of the audio the file holds.
    pub fn add(&self, file: NewFile<'_>) -> i64 {
        let name = file.name.to_string();
        let tags = raw_tags(file.title, file.artist);
        let hash = file.hash.map(|h| h.to_vec());
        let blake3 = file.blake3.map(|b| vec![b; 32]);
        let blob = file.items.map(|items| Fingerprint::new(items).to_blob());
        let id = self
            .writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime,
                                       audio_hash, raw_tags, fingerprint, blake3)
                     VALUES (1, ?1, ?1, 100, 1000, ?2, ?3, ?4, ?5)",
                    (name, hash, tags, blob, blake3),
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap();
        self.fingerprint_stage_ran(id);
        id
    }

    /// What the fingerprint stage records with a fingerprint it wrote: the
    /// `audio_hash` it came from, and its own row for the file as it is on
    /// disk (its size and modified time). After this, `file`'s fingerprint
    /// is of the audio it holds.
    pub fn fingerprint_stage_ran(&self, file: i64) {
        self.sql(&format!(
            "UPDATE file SET fingerprint_audio_hash = audio_hash
             WHERE id = {file} AND fingerprint IS NOT NULL;
             INSERT OR REPLACE INTO file_stage (file_id, stage, version, size, mtime, status)
             SELECT id, 'fingerprint', 1, size, mtime, 'done' FROM file
             WHERE id = {file} AND fingerprint IS NOT NULL;"
        ));
    }

    /// Overwrites `file` on disk as the walk and the hash stage see it: a
    /// new size and modified time, and the audio it holds now (`hash`, or
    /// for a file the hash stage can't hash, `blake3`). Its fingerprint is
    /// left as it was: the fingerprint stage hasn't been through it.
    pub fn overwrite(&self, file: i64, hash: Option<[u8; 34]>, blake3: Option<u8>) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET size = size + 1, mtime = mtime + 1, audio_hash = ?2,
                                     blake3 = ?3
                     WHERE id = ?1",
                    (file, hash.map(|h| h.to_vec()), blake3.map(|b| vec![b; 32])),
                )
            })
            .unwrap();
    }

    /// Gives `file` other tags, as a rewrite by rekordbox or the user does.
    pub fn retag(&self, file: i64, title: Option<&str>, artist: Option<&str>) {
        let tags = raw_tags(title, artist);
        self.writer
            .call(move |c| c.execute("UPDATE file SET raw_tags = ?2 WHERE id = ?1", (file, tags)))
            .unwrap();
    }

    /// Gives `file` another fingerprint, as the fingerprint job does once
    /// it reaches a file whose audio changed.
    pub fn set_items(&self, file: i64, items: Vec<u32>) {
        let blob = Fingerprint::new(items).to_blob();
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET fingerprint = ?2 WHERE id = ?1",
                    (file, blob),
                )
            })
            .unwrap();
        self.fingerprint_stage_ran(file);
    }

    /// Gives `file` another `audio_hash`, with a fingerprint (if it has
    /// one) that is of it: both stages have been through the file. For a
    /// file between the two stages, see [`Db::overwrite`].
    pub fn set_hash(&self, file: i64, hash: Option<[u8; 34]>) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET audio_hash = ?2 WHERE id = ?1",
                    (file, hash.map(|h| h.to_vec())),
                )
            })
            .unwrap();
        self.fingerprint_stage_ran(file);
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

    /// Adds `file`'s track to the Library, linked to `file`. Returns the
    /// track.
    pub fn to_library(&self, file: i64) -> i64 {
        let track = self.track(file).expect("the file has a track");
        self.sql(&format!(
            "INSERT INTO library_track (recording_id, linked_file_id) VALUES ({track}, {file})"
        ));
        track
    }

    pub fn group(&self) -> Summary {
        self.writer.call(regroup).unwrap()
    }

    /// The matching pass, as the chain runs it once the fingerprints are in.
    pub fn compare_fingerprints(&self) {
        crate::matching::refresh(&self.writer).unwrap();
    }

    /// The deciding run alone, `page` files a transaction: no grouping
    /// pass, and nothing decided for the sets a pass would find waiting.
    pub fn decide_in_pages(&self, page: usize) -> Decided {
        self.decide_ticking(page, || {})
    }

    /// [`Db::decide_in_pages`], calling `tick` wherever the run reports how
    /// far it is: at its start and before every page.
    pub fn decide_ticking(&self, page: usize, mut tick: impl FnMut()) -> Decided {
        let mut names = self.names.lock().unwrap();
        decide::run::<DbError>(
            &self.writer,
            &mut names,
            page,
            &mut |_| {
                tick();
                Ok(())
            },
            &mut || Ok(()),
        )
        .unwrap()
    }

    /// Everything a deciding job does ([`decide_and_group`]), without the
    /// matching pass before it.
    pub fn deciding_job(&self) -> (Decided, Summary) {
        let mut names = self.names.lock().unwrap();
        let mut done = (Decided::default(), Summary::default());
        decide_and_group::<DbError>(
            &self.writer,
            &mut names,
            PAGE,
            &mut done,
            &mut |_| Ok(()),
            &mut || Ok(()),
            &mut || Ok(()),
        )
        .unwrap();
        done
    }

    /// The quick job's pass, the matching pass, and a deciding job, as the
    /// chain runs them once the fingerprints are in.
    pub fn decide(&self) -> (Decided, Summary) {
        let mut summary = self.group();
        self.compare_fingerprints();
        let (decided, passes) = self.deciding_job();
        summary.add(passes);
        (decided, summary)
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

    /// The stored pair of files `a` and `b`: its outcome and where it
    /// stands.
    pub fn pair(&self, a: i64, b: i64) -> Option<(String, String)> {
        use rusqlite::OptionalExtension;
        let (low, high) = (a.min(b), a.max(b));
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT outcome, state FROM grouping_pair WHERE file_a = ?1 AND file_b = ?2",
                    [low, high],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
            })
            .unwrap()
    }

    /// The pair's outcome and where it stands, as `("duplicate", "merged")`.
    pub fn stands(&self, a: i64, b: i64) -> (String, String) {
        self.pair(a, b)
            .unwrap_or_else(|| panic!("no pair stored for files {a} and {b}"))
    }

    /// The reasons stored for the pair, as JSON.
    pub fn reasons(&self, a: i64, b: i64) -> String {
        self.writer
            .call(move |c| decide::reasons_of(c, a, b))
            .unwrap()
            .unwrap_or_else(|| panic!("no pair stored for files {a} and {b}"))
    }

    /// The version link between the tracks of files `a` and `b`: its kind,
    /// label, source, and whether `a`'s track is its first.
    pub fn link(&self, a: i64, b: i64) -> Option<(String, Option<String>, String, bool)> {
        use rusqlite::OptionalExtension;
        let (ta, tb) = (self.track(a).unwrap(), self.track(b).unwrap());
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT kind, label, source, recording_a = ?1 FROM version_link
                     WHERE (recording_a = ?1 AND recording_b = ?2)
                        OR (recording_a = ?2 AND recording_b = ?1)",
                    [ta, tb],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
            })
            .unwrap()
    }

    /// The files whose every pair is decided.
    pub fn decided(&self) -> Vec<i64> {
        self.writer
            .call(|c| {
                let mut s = c.prepare(
                    "SELECT file_id FROM grouping_file WHERE decided_as IS ?1 ORDER BY file_id",
                )?;
                let rows = s.query_map([crate::grouping::decided_as()], |r| r.get(0))?;
                rows.collect()
            })
            .unwrap()
    }

    /// Whether matching stored a comparison of files `a` and `b`.
    pub fn compared(&self, a: i64, b: i64) -> bool {
        self.writer
            .call(move |c| crate::matching::store::of_pair(c, a, b))
            .unwrap()
            .is_some()
    }

    /// Everything grouping keeps, as text: tracks and their files, pairs,
    /// links, records, decisions and stored comparisons, with the times
    /// they were written. Two equal dumps are a database grouping didn't
    /// touch.
    pub fn dump(&self) -> String {
        const TABLES: [&str; 7] = [
            "SELECT 'track ' || id || ' ' || created_at FROM recording ORDER BY id",
            "SELECT 'file ' || file_id || ' ' || recording_id || ' ' || role
             FROM recording_file ORDER BY file_id",
            "SELECT 'pair ' || file_a || '-' || file_b || ' ' || outcome || ' ' || state || ' '
                    || reasons || ' ' || decided_at
             FROM grouping_pair ORDER BY file_a, file_b",
            "SELECT 'link ' || id || ' ' || recording_a || ' ' || recording_b || ' ' || kind
                    || ' ' || coalesce(label, '-') || ' ' || source || ' ' || created_at
             FROM version_link ORDER BY id",
            "SELECT 'record ' || file_id || ' ' || hex(coalesce(placed_key, x'')) || ' '
                    || coalesce(decided_as, -1) || ' ' || rev
             FROM grouping_file ORDER BY file_id",
            "SELECT 'decision ' || id || ' ' || hex(key_a) || ' ' || hex(key_b) || ' ' || decision
             FROM grouping_decision ORDER BY id",
            "SELECT 'compared ' || file_a || '-' || file_b || ' ' || compared_at
             FROM fingerprint_match ORDER BY file_a, file_b",
        ];
        self.writer
            .call(|c| {
                let mut out = Vec::new();
                for sql in TABLES {
                    let mut s = c.prepare(sql)?;
                    let rows = s.query_map([], |r| r.get::<_, String>(0))?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                Ok(out.join("\n"))
            })
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
