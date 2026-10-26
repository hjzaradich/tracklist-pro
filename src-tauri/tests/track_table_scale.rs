//! 1cA-3: what the track table's backend query costs on a made-up library of
//! 100,000 tracks: the first page, every sort, a search, a scroll to the
//! middle, and the time to build the table's rows (they are written by the
//! index job, beside the search index's).
//!
//! The library is made up in code (no audio, no real music, made-up words).
//! Of every 100 tracks:
//!
//! - 33 are in the Library, each with a rekordbox entry (so rekordbox has
//!   33,000 of them) and, for 20 of the 33, a place in one of ten crates;
//! - 50 have a BPM and a key (rekordbox's analysis), 90 a genre tag and 95 a
//!   length;
//! - 3 have a file that is gone, 2 a quality finding; the formats are MP3
//!   (60), FLAC (25), WAV (10) and AIFF (5).
//!
//! The measurement is an ignored test; run it alone, in a release build:
//!
//! ```text
//! cargo test --release --test track_table_scale -- --ignored --nocapture --exact a_library_of_100k
//! ```
//!
//! It prints the median of five runs of each query and asserts none (debug
//! builds are not a gate). **`a_small_library_goes_through_the_same_steps`**
//! is the smoke test that runs in CI: the same harness on 2,000 tracks,
//! checking what comes out rather than how long it took.

use std::path::Path;
use std::time::{Duration, Instant};

use tracklist_pro_lib::crates::CrateId;
use tracklist_pro_lib::db::{Writer, DB_FILE_NAME};
use tracklist_pro_lib::fragile::FragileDirs;
use tracklist_pro_lib::paths::Volumes;
use tracklist_pro_lib::search::{pending, settle};
use tracklist_pro_lib::track_table::{
    page, read, SortColumn, TablePage, TableQuery, TableSort, TableSource,
};
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

const KEYS: [&str; 6] = ["1A", "4B", "8A", "8B", "11A", "12B"];

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

/// A word from a vocabulary of `size`, the low numbers far more often.
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

/// A migrated database in a temp folder with one music folder.
struct Db {
    writer: Writer,
    _dir: tempfile::TempDir,
}

impl Db {
    fn new() -> Db {
        let dir = tempfile::Builder::new()
            .prefix("tlp-table-")
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
        Db { writer, _dir: dir }
    }

    /// Makes `tracks` tracks, one file each, as the scan and the grouping job
    /// leave them, with the shares in the file's doc.
    fn make(&self, tracks: u64) {
        self.writer
            .call(move |c| {
                let tx = c.transaction()?;
                let mut rng = Rng(0x7AB1E5);
                for n in 1..=10 {
                    tx.execute(
                        "INSERT INTO crate (name, kind) VALUES (?1, 'static')",
                        [format!("Crate {n}")],
                    )?;
                }
                {
                    let mut recording = tx.prepare("INSERT INTO recording DEFAULT VALUES")?;
                    let mut file = tx.prepare(
                        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, raw_tags,
                                           sniffed_format, duration_ms, present)
                         VALUES (1, ?1, ?1, ?2, ?3, ?4, ?5)",
                    )?;
                    let mut verdict = tx.prepare(
                        "INSERT INTO file_verdict (file_id, rules_version, inputs, verdict)
                         VALUES (?1, 1, '[]', ?2)",
                    )?;
                    let mut link = tx.prepare(
                        "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
                    )?;
                    let mut library = tx.prepare(
                        "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                         VALUES (?1, 'linked', ?2, ?3)",
                    )?;
                    let mut rekordbox = tx.prepare(
                        "INSERT INTO rekordbox_track
                             (attributes, location_key, read_at, file_id, relink_method, relink_probable)
                         VALUES (?1, ?2, '2026-09-30T10:00:00.000Z', ?3, 'path', 0)",
                    )?;
                    let mut analysis = tx.prepare(
                        "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, 'rekordbox', ?2, ?3)",
                    )?;
                    let mut entry = tx.prepare(
                        "INSERT INTO crate_entry (crate_id, library_track_id, kind) VALUES (?1, ?2, 'member')",
                    )?;
                    for n in 0..tracks {
                        let title_words = 2 + rng.below(3);
                        let title = words(&mut rng, title_words, 60_000);
                        let artist_words = 1 + rng.below(2);
                        let artist = words(&mut rng, artist_words, 30_000);
                        let item = |key: &str, text: &str| {
                            serde_json::json!({"key": key, "value": {"type": "text", "text": text}})
                        };
                        let mut items = vec![item("TIT2", &title), item("TPE1", &artist)];
                        if rng.below(100) < 90 {
                            items.push(item("TCON", GENRES[rng.below(12) as usize]));
                        }
                        let raw = serde_json::json!({ "id3v2": items }).to_string();
                        let (format, ext) = match rng.below(100) {
                            0..=59 => ("mp3", "mp3"),
                            60..=84 => ("flac", "flac"),
                            85..=94 => ("wav", "wav"),
                            _ => ("aiff", "aif"),
                        };
                        let name = format!("G{}/{artist} - {title}.{ext}", n % 40);
                        let gone = n % 100 >= 97;
                        let duration = (rng.below(100) < 95).then(|| 120_000 + rng.below(400_000) as i64);
                        recording.execute([])?;
                        let recording_id = tx.last_insert_rowid();
                        file.execute(rusqlite::params![name, raw, format, duration, !gone])?;
                        let file_id = tx.last_insert_rowid();
                        if n % 100 == 50 || n % 100 == 51 {
                            verdict.execute(rusqlite::params![file_id, "low_bitrate"])?;
                        }
                        link.execute(rusqlite::params![recording_id, file_id])?;
                        if rng.below(100) < 50 {
                            analysis.execute(rusqlite::params![
                                recording_id,
                                80.0 + rng.below(1000) as f64 / 10.0,
                                KEYS[rng.below(6) as usize]
                            ])?;
                        }
                        if n % 100 < 33 {
                            library.execute(rusqlite::params![
                                recording_id,
                                file_id,
                                if gone { "missing" } else { "ok" }
                            ])?;
                            let library_id = tx.last_insert_rowid();
                            let rating = ["0", "51", "102", "153", "204", "255"][rng.below(6) as usize];
                            let attributes = serde_json::json!({
                                "TrackID": (n + 1).to_string(),
                                "Name": &title,
                                "Artist": &artist,
                                "Genre": GENRES[rng.below(12) as usize],
                                "Rating": rating,
                                "TotalTime": (120 + rng.below(400)).to_string(),
                                "Location": format!("file://localhost/E:/M/{n}.mp3"),
                            })
                            .to_string();
                            rekordbox.execute(rusqlite::params![
                                attributes,
                                format!("E:/M/{n}.MP3"),
                                file_id
                            ])?;
                            if n % 100 < 20 {
                                entry.execute(rusqlite::params![1 + (n % 10) as i64, library_id])?;
                            }
                        }
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .unwrap();
    }

    fn page(&self, query: TableQuery) -> TablePage {
        self.writer
            .call(move |c| {
                let found = read(c, &NoDrives, &query)?.expect("the crate exists");
                let dirs = FragileDirs {
                    downloads: None,
                    temp: Vec::new(),
                };
                Ok(page(found, query.offset, &NoDrives, &dirs))
            })
            .unwrap()
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

fn query(source: TableSource) -> TableQuery {
    TableQuery {
        source,
        search: None,
        sort: None,
        offset: 0,
        limit: 200,
        with_total: true,
    }
}

fn sorted(source: TableSource, column: SortColumn, descending: bool) -> TableQuery {
    TableQuery {
        sort: Some(TableSort { column, descending }),
        ..query(source)
    }
}

/// The median of `runs` runs of a query, with the page of the last.
fn measure(db: &Db, q: &TableQuery, runs: usize) -> (TablePage, Duration) {
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..runs {
        let (found, took) = timed(|| db.page(q.clone()));
        times.push(took);
        last = Some(found);
    }
    times.sort();
    (last.unwrap(), times[times.len() / 2])
}

/// Everything the measurement and the smoke test do. `check` is true for
/// the smoke test.
fn run(tracks: u64, check: bool) {
    let db = Db::new();
    let (_, made) = timed(|| db.make(tracks));
    println!("{tracks} tracks made in {}", secs(made));
    assert!(db.writer.call(|c| pending(c)).unwrap());

    let (settled, built) = timed(|| db.writer.call(|c| settle(c, &NoDrives, 500)).unwrap());
    println!(
        "index and table rows built from nothing: {settled} tracks in {} ({:.0} tracks/s)",
        secs(built),
        settled as f64 / built.as_secs_f64()
    );

    let middle = u32::try_from(tracks / 2).unwrap();
    let mut probes: Vec<(&str, TableQuery)> = vec![
        ("Library, first page", query(TableSource::Library)),
        ("All music, first page", query(TableSource::AllMusic)),
        (
            "All music, sort by title, descending",
            sorted(TableSource::AllMusic, SortColumn::Title, true),
        ),
        (
            "All music, sort by artist",
            sorted(TableSource::AllMusic, SortColumn::Artist, false),
        ),
        (
            "All music, sort by BPM, descending",
            sorted(TableSource::AllMusic, SortColumn::Bpm, true),
        ),
        (
            "All music, sort by key",
            sorted(TableSource::AllMusic, SortColumn::Key, false),
        ),
        (
            "All music, sort by time",
            sorted(TableSource::AllMusic, SortColumn::Time, false),
        ),
        (
            "All music, sort by genre",
            sorted(TableSource::AllMusic, SortColumn::Genre, false),
        ),
        (
            "All music, sort by date added, descending",
            sorted(TableSource::AllMusic, SortColumn::DateAdded, true),
        ),
        (
            "All music, sort by file",
            sorted(TableSource::AllMusic, SortColumn::File, false),
        ),
        (
            "All music, sort by folder",
            sorted(TableSource::AllMusic, SortColumn::Folder, false),
        ),
        (
            "Library, sort by rating, descending",
            sorted(TableSource::Library, SortColumn::Rating, true),
        ),
        (
            "All music, scroll to the middle",
            TableQuery {
                offset: middle,
                with_total: false,
                ..query(TableSource::AllMusic)
            },
        ),
        (
            "All music, scroll to the middle, sort by BPM",
            TableQuery {
                offset: middle,
                with_total: false,
                ..sorted(TableSource::AllMusic, SortColumn::Bpm, false)
            },
        ),
        (
            "All music, scroll to the end, sort by genre, descending",
            TableQuery {
                offset: u32::try_from(tracks).unwrap() - 100,
                with_total: false,
                ..sorted(TableSource::AllMusic, SortColumn::Genre, true)
            },
        ),
        (
            "Crate 3, first page",
            query(TableSource::Crate { id: CrateId(3) }),
        ),
        (
            "Crate 3, sort by title",
            sorted(
                TableSource::Crate { id: CrateId(3) },
                SortColumn::Title,
                false,
            ),
        ),
    ];
    let top = word(0);
    probes.push((
        "All music, search a letter",
        TableQuery {
            search: Some(top[..1].to_owned()),
            ..query(TableSource::AllMusic)
        },
    ));
    probes.push((
        "All music, search a letter, sort by BPM",
        TableQuery {
            search: Some(top[..1].to_owned()),
            ..sorted(TableSource::AllMusic, SortColumn::Bpm, true)
        },
    ));
    probes.push((
        "All music, search a word",
        TableQuery {
            search: Some(word(5)),
            ..query(TableSource::AllMusic)
        },
    ));

    let mut slowest = Duration::ZERO;
    for (what, q) in &probes {
        let (found, took) = measure(&db, q, if check { 1 } else { 5 });
        println!(
            "{what}: {} rows of {:?} in {}",
            found.rows.len(),
            found.total,
            ms(took)
        );
        slowest = slowest.max(took);
        if check {
            assert!(found.rows.len() <= 200, "{what}");
        }
    }
    println!("slowest query: {}", ms(slowest));

    if check {
        let all = db.page(query(TableSource::AllMusic));
        assert_eq!(all.total, Some(u32::try_from(tracks).unwrap()));
        let library = db.page(query(TableSource::Library));
        assert_eq!(
            library.total,
            Some(u32::try_from(tracks / 100 * 33).unwrap())
        );
        // Windows laid end to end are the list, with no row twice or skipped.
        let mut stitched = Vec::new();
        for offset in (0..tracks as u32).step_by(200) {
            let part = db.page(TableQuery {
                offset,
                with_total: false,
                ..sorted(TableSource::AllMusic, SortColumn::Bpm, false)
            });
            stitched.extend(part.rows.iter().map(|r| r.recording_id));
        }
        let mut seen = stitched.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(stitched.len(), tracks as usize);
        assert_eq!(seen.len(), tracks as usize);
        // The rows without a BPM come last.
        let first = db.page(sorted(TableSource::AllMusic, SortColumn::Bpm, false));
        assert!(first.rows[0].bpm.is_some());
        let last = db.page(TableQuery {
            offset: u32::try_from(tracks).unwrap() - 10,
            ..sorted(TableSource::AllMusic, SortColumn::Bpm, false)
        });
        assert!(last.rows.iter().all(|r| r.bpm.is_none()));
        assert!(!db.writer.call(|c| pending(c)).unwrap());
    }
}

#[test]
fn a_small_library_goes_through_the_same_steps() {
    run(2_000, true);
}

#[test]
#[ignore = "a measurement: run it alone, in a release build, with --nocapture (see the file's doc)"]
fn a_library_of_100k() {
    run(100_000, false);
}

#[test]
#[ignore = "a measurement: run it alone, in a release build, with --nocapture (see the file's doc)"]
fn a_library_of_10k() {
    run(10_000, false);
}
