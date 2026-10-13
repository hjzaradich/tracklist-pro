//! 1cA-6: what the search index costs on a made-up library of 100,000
//! files: how big it is, how long a search, a full rebuild, an upkeep after
//! some retags and the settle after a rekordbox read that changed nothing
//! take.
//!
//! The library is made up in code (no audio, no real music, made-up words):
//! a few words are in a large share of the titles, as "the" and "mix"
//! are, and most are rare (60,000 words for titles, 30,000 for artists). Of every 100 tracks:
//!
//! - 33 are in the Library with a rekordbox entry that names them
//!   otherwise (so 33,000 Library tracks at 100,000);
//! - 3 have a hidden extra copy, 2 a file the user hid;
//! - 30 have a comment of three to eight words.
//!
//! The measurement is an ignored test. Run it alone:
//!
//! ```text
//! cargo test --test search_scale -- --ignored --nocapture --exact a_library_of_100k
//! cargo test --test search_scale -- --ignored --nocapture --exact a_library_of_100k_with_prefix_indexes
//! ```
//!
//! It prints the times and the size; it asserts none. The second builds the
//! index with `prefix='2 3'` instead, to compare. **`a_small_library_goes_through_the_same_steps`**
//! is the smoke test that runs in CI: the same harness on 2,000 tracks,
//! checking what came out rather than how long it took.

use std::path::Path;
use std::time::{Duration, Instant};

use tracklist_pro_lib::db::{Writer, DB_FILE_NAME};
use tracklist_pro_lib::paths::Volumes;
use tracklist_pro_lib::search::{find, pending, rebuild, settle};
use tracklist_pro_lib::volume::{Volume, VolumeId};
use tracklist_pro_lib::write_guard::WriteGuard;

/// splitmix64: a seeded, platform-independent random source.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// No drive is looked at: every file of the made-up library is present.
struct NoDrives;

impl Volumes for NoDrives {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up",
            path.display()
        )))
    }

    fn mount_path(&self, _: &VolumeId) -> Option<std::path::PathBuf> {
        None
    }
}

const SYLLABLES: [&str; 24] = [
    "ba", "ko", "ri", "mu", "sel", "dan", "vo", "li", "tra", "ne", "zu", "pha", "gor", "ste",
    "lum", "fi", "rok", "an", "yo", "ce", "dub", "mar", "ix", "ul",
];

const GENRES: [&str; 12] = [
    "House",
    "Deep House",
    "Techno",
    "Drum and Bass",
    "Disco",
    "Garage",
    "Ambient",
    "Trance",
    "Breaks",
    "Electro",
    "Funk",
    "Dub",
];

/// Made-up word number `n`: syllables spelled out in base 24.
fn word(mut n: u64) -> String {
    let mut out = String::new();
    loop {
        out.push_str(SYLLABLES[(n % 24) as usize]);
        n /= 24;
        if n == 0 {
            return out;
        }
    }
}

/// A word from a vocabulary of `size`, the low numbers far more often (a
/// skew, as a language has).
fn pick(rng: &mut Rng, size: u64) -> String {
    let u = rng.below(1_000_000) as f64 / 1_000_000.0;
    word(((u * u) * size as f64) as u64)
}

fn words(rng: &mut Rng, how_many: u64, size: u64) -> String {
    (0..how_many)
        .map(|_| pick(rng, size))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `base` to `base + spread - 1` words.
fn some_words(rng: &mut Rng, base: u64, spread: u64, size: u64) -> String {
    let how_many = base + rng.below(spread);
    words(rng, how_many, size)
}

/// A migrated database in a temp folder with one music folder.
struct Db {
    writer: Writer,
    path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl Db {
    fn new() -> Db {
        let dir = tempfile::Builder::new()
            .prefix("tlp-search-")
            .tempdir()
            .unwrap();
        let guard = WriteGuard::app_data(dir.path()).unwrap();
        let path = guard.check(&dir.path().join(DB_FILE_NAME)).unwrap();
        let writer = Writer::open(&path).unwrap();
        writer
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');",
                )
            })
            .unwrap();
        Db {
            writer,
            path: dir.path().join(DB_FILE_NAME),
            _dir: dir,
        }
    }

    /// Replaces the index table with one that has `prefix` indexes.
    fn with_prefix_indexes(&self, prefix: &'static str) {
        self.writer
            .call(move |c| {
                c.execute_batch(&format!(
                    "DROP TABLE search_track;
                     CREATE VIRTUAL TABLE search_track USING fts5(
                         title, artist, album, remixer, label, genre, comment, file_name,
                         content = '', contentless_delete = 1,
                         tokenize = 'unicode61 remove_diacritics 2', prefix = '{prefix}');"
                ))
            })
            .unwrap();
    }

    /// Makes `tracks` tracks, one file each (a few with more), as the scan
    /// and the grouping job leave them. Every 100th title starts with
    /// "Zeppelinish". Returns the ids of those tracks.
    fn make(&self, tracks: u64) -> Vec<i64> {
        self.writer
            .call(move |c| {
                let tx = c.transaction()?;
                let mut rng = Rng(0x5EA2C4);
                let mut planted = Vec::new();
                {
                    let mut recording = tx.prepare("INSERT INTO recording DEFAULT VALUES")?;
                    let mut file = tx.prepare(
                        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, raw_tags, hidden_reason)
                         VALUES (1, ?1, ?1, ?2, ?3)",
                    )?;
                    let mut link = tx.prepare(
                        "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, ?3)",
                    )?;
                    let mut library = tx.prepare(
                        "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                         VALUES (?1, 'linked', ?2, 'ok')",
                    )?;
                    let mut rekordbox = tx.prepare(
                        "INSERT INTO rekordbox_track
                             (attributes, location_key, read_at, file_id, relink_method, relink_probable)
                         VALUES (?1, ?2, '2026-09-30T10:00:00.000Z', ?3, 'path', 0)",
                    )?;
                    for n in 0..tracks {
                        let mut title = some_words(&mut rng, 2, 3, 60_000);
                        if n % 100 == 0 {
                            title = format!("Zeppelinish {title}");
                        }
                        let artist = some_words(&mut rng, 1, 2, 30_000);
                        let item = |key: &str, text: &str| {
                            serde_json::json!({"key": key, "value": {"type": "text", "text": text}})
                        };
                        let mut items = vec![
                            item("TIT2", &title),
                            item("TPE1", &artist),
                            item("TCON", GENRES[rng.below(12) as usize]),
                        ];
                        if rng.below(2) == 0 {
                            items.push(item("TALB", &some_words(&mut rng, 1, 2, 40_000)));
                        }
                        if rng.below(3) == 0 {
                            items.push(item("TPUB", &words(&mut rng, 1, 1_500)));
                        }
                        if rng.below(10) == 0 {
                            items.push(item("TPE4", &words(&mut rng, 2, 30_000)));
                        }
                        if n % 10 < 3 {
                            items.push(item("COMM::eng", &some_words(&mut rng, 3, 6, 60_000)));
                        }
                        let raw = serde_json::json!({ "id3v2": items }).to_string();
                        let name = format!("G{}/{artist} - {title}.mp3", n % 40);
                        recording.execute([])?;
                        let recording_id = tx.last_insert_rowid();
                        let hidden = (n % 50 == 7).then_some("user");
                        file.execute(rusqlite::params![name, raw, hidden])?;
                        let file_id = tx.last_insert_rowid();
                        link.execute(rusqlite::params![recording_id, file_id, "best"])?;
                        if n % 100 == 0 {
                            planted.push(recording_id);
                        }
                        if n % 33 == 5 {
                            // A hidden extra copy.
                            file.execute(rusqlite::params![
                                format!("G{}/{artist} - {title} (copy).mp3", n % 40),
                                raw,
                                Option::<&str>::None
                            ])?;
                            let extra = tx.last_insert_rowid();
                            link.execute(rusqlite::params![recording_id, extra, "extra"])?;
                        }
                        if n % 100 < 33 {
                            library.execute(rusqlite::params![recording_id, file_id])?;
                            let attributes = serde_json::json!({
                                "TrackID": (n + 1).to_string(),
                                "Name": some_words(&mut rng, 2, 3, 60_000),
                                "Artist": &artist,
                                "Location": format!("file://localhost/E:/M/{n}.mp3"),
                            })
                            .to_string();
                            rekordbox.execute(rusqlite::params![
                                attributes,
                                format!("E:/M/{n}.MP3"),
                                file_id
                            ])?;
                        }
                    }
                }
                tx.commit()?;
                Ok(planted)
            })
            .unwrap()
    }

    fn size_of_index(&self) -> u64 {
        self.writer
            .call(|c| {
                c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
                c.query_row(
                    "SELECT coalesce(sum(pgsize), 0) FROM dbstat WHERE name LIKE 'search_track%'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
            })
            .map(|n| n as u64)
            .unwrap_or_else(|_| std::fs::metadata(&self.path).map_or(0, |m| m.len()))
    }
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = work();
    (out, start.elapsed())
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn secs(d: Duration) -> String {
    format!("{:.2} s", d.as_secs_f64())
}

/// The median of five runs of a search, with how many tracks it found.
fn search(db: &Db, text: &str) -> (usize, usize, Duration) {
    let mut times = Vec::new();
    let mut found = (0, 0);
    for _ in 0..5 {
        let text = text.to_owned();
        let (f, took) = timed(|| db.writer.call(move |c| find(c, &text)).unwrap());
        let f = f.expect("a search");
        found = (f.tracks.len(), f.hidden.len());
        times.push(took);
    }
    times.sort();
    (found.0, found.1, times[2])
}

/// Everything the measurement and the smoke test do. `check` is true for
/// the smoke test.
fn run(tracks: u64, prefix: Option<&'static str>, check: bool) {
    let db = Db::new();
    if let Some(prefix) = prefix {
        db.with_prefix_indexes(prefix);
    }
    let (planted, made) = timed(|| db.make(tracks));
    println!("{tracks} tracks made in {}", secs(made));
    assert!(db.writer.call(|c| pending(c)).unwrap());

    let (settled, built) = timed(|| db.writer.call(|c| settle(c, &NoDrives, 500)).unwrap());
    println!(
        "index built from nothing: {settled} tracks in {} ({:.0} tracks/s)",
        secs(built),
        settled as f64 / built.as_secs_f64()
    );
    let size = db.size_of_index();
    println!("index size: {:.1} MB", size as f64 / 1e6);

    let top = word(0);
    let probes = [
        ("1 letter", top[..1].to_owned()),
        ("2 letters", top[..2].to_owned()),
        ("3 letters", word(5)[..3].to_owned()),
        ("a common word", top.clone()),
        ("a rare word", word(50_000)),
        ("two words", format!("{} {}", word(1), word(2))),
        ("hostile", "\"NEAR(a b)\" title: -x*".to_owned()),
    ];
    for (what, text) in &probes {
        let (tracks, hidden, took) = search(&db, text);
        println!(
            "search {what} ({text}): {tracks} tracks, {hidden} hidden files, {}",
            ms(took)
        );
    }

    // Upkeep: a thousand retags.
    let (_, retagged) = timed(|| {
        db.writer
            .call(|c| {
                c.execute_batch(
                    "UPDATE file SET raw_tags = replace(raw_tags, 'TIT2', 'TIT2')
                         WHERE id % 100 = 3;
                     UPDATE file SET raw_tags = replace(raw_tags, '\"text\"', '\"text\" ')
                         WHERE id % 100 = 3",
                )
            })
            .unwrap()
    });
    let due: i64 = db
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM search_due", [], |r| r.get(0)))
        .unwrap();
    let (n, upkeep) = timed(|| db.writer.call(|c| settle(c, &NoDrives, 500)).unwrap());
    println!(
        "upkeep after retagging {due} tracks (marking took {}): {n} tracks in {}",
        ms(retagged),
        ms(upkeep)
    );

    // A rekordbox read that changed nothing leaves the mark the triggers
    // note a replaced snapshot with: every Library track is due.
    let (_, replaced) = timed(|| {
        db.writer
            .call(|c| c.execute("INSERT INTO search_due (recording_id) VALUES (-1)", []))
            .unwrap()
    });
    let (n, resettle) = timed(|| db.writer.call(|c| settle(c, &NoDrives, 500)).unwrap());
    println!(
        "resettle after a rekordbox read that changed nothing (marking took {}): \
         {n} tracks in {}",
        ms(replaced),
        secs(resettle)
    );

    let (n, again) = timed(|| db.writer.call(|c| rebuild(c, &NoDrives, 500)).unwrap());
    println!("rebuild: {n} tracks in {}", secs(again));

    if check {
        // What was planted is found, through the same index after a rebuild.
        let zeppelin = db
            .writer
            .call(|c| find(c, "zeppelinish"))
            .unwrap()
            .expect("a search");
        let mut expected = planted.clone();
        expected.sort_unstable();
        assert_eq!(zeppelin.tracks, expected);
        assert!(!db.writer.call(|c| pending(c)).unwrap());
        let hostile = db
            .writer
            .call(|c| find(c, "\"NEAR(a b)\" title: -x*"))
            .unwrap();
        assert!(hostile.is_some());
        // Every Library track is found by the title rekordbox has for it.
        let (library, name): (i64, String) = db
            .writer
            .call(|c| {
                c.query_row(
                    "SELECT lt.recording_id, json_extract(rt.attributes, '$.Name')
                     FROM library_track lt JOIN rekordbox_track rt ON rt.file_id = lt.linked_file_id
                     ORDER BY lt.id LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .unwrap();
        let found = db
            .writer
            .call(move |c| find(c, &name))
            .unwrap()
            .expect("a search");
        assert!(found.tracks.contains(&library));
    }
}

#[test]
fn a_small_library_goes_through_the_same_steps() {
    run(2_000, None, true);
}

#[test]
#[ignore = "a measurement: run it alone, with --nocapture (see the file's doc)"]
fn a_library_of_100k() {
    run(100_000, None, false);
}

#[test]
#[ignore = "a measurement: run it alone, with --nocapture (see the file's doc)"]
fn a_library_of_100k_with_prefix_indexes() {
    run(100_000, Some("2 3"), false);
}

#[test]
#[ignore = "a measurement: run it alone, with --nocapture (see the file's doc)"]
fn a_library_of_10k() {
    run(10_000, None, false);
}
