//! 1bC-4: what ranking the files of a track costs on a made-up library of
//! 100,000 files, and that a run with nothing to do, or with a few tracks
//! due, costs in proportion to what changed and not to the library.
//!
//! The files are made up in code (no audio, made-up names): about one track
//! in three has a second file and one in ten a third, in FLAC, MP3 and a
//! format the DJ app doesn't play; every file has a tag block of about the
//! size a real file's is (a few dozen frames, one embedded picture, which
//! the scan records by size only). One track in five is in the Library,
//! linked to its lowest-id file.
//!
//! The measurement is an ignored test. Run it alone, in a release build:
//!
//! ```text
//! cargo test --release --test file_roles_scale -- --ignored --nocapture --exact a_library_of_100k
//! ```
//!
//! It prints three times and asserts none of them: the first full ranking
//! pass (every track is due, as after the migration), an idle run (nothing
//! due), and a run with 100 tracks due. **`a_small_library_goes_through_the_same_steps`**
//! is the smoke test that runs in CI: the same harness on 3,000 files,
//! checking what came out rather than how long it took.

use std::time::{Duration, Instant};

use serde_json::json;
use tracklist_pro_lib::db::{DbError, Writer, DB_FILE_NAME};
use tracklist_pro_lib::file_roles::{self, Summary};
use tracklist_pro_lib::write_guard::WriteGuard;

/// A migrated database in a temp folder with one music folder.
struct Db {
    writer: Writer,
    _dir: tempfile::TempDir,
}

impl Db {
    fn new() -> Db {
        let dir = tempfile::Builder::new()
            .prefix("tlp-roles-")
            .tempdir()
            .unwrap();
        let guard = WriteGuard::app_data(dir.path()).unwrap();
        let path = guard.check(&dir.path().join(DB_FILE_NAME)).unwrap();
        let writer = Writer::open(&path).unwrap();
        writer
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'internal');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'M', 'M');",
                )
            })
            .unwrap();
        Db { writer, _dir: dir }
    }

    fn count(&self, sql: &'static str) -> i64 {
        self.writer
            .call(move |c| c.query_row(sql, [], |r| r.get(0)))
            .unwrap()
    }

    /// Ranks every due track and says how long it took.
    fn settle(&self) -> (Summary, Duration) {
        let begun = Instant::now();
        let summary = file_roles::pass(&self.writer, &mut || Ok::<(), DbError>(())).unwrap();
        (summary, begun.elapsed())
    }
}

/// A tag block of about the size of a real file's: the frames the ranking
/// reads, a picture recorded by size, and a few dozen frames it doesn't.
fn tags(seed: u64) -> String {
    let text =
        |key: &str, text: String| json!({"key": key, "value": {"type": "text", "text": text}});
    let mut items = vec![
        text("TIT2", format!("Made Up Tune {seed}")),
        text("TPE1", format!("Nobody {}", seed % 5000)),
        text("TALB", format!("Album {}", seed % 3000)),
        text("TCON", "Electronic".to_owned()),
        text("TPUB", format!("Label {}", seed % 400)),
        text("TDRC", format!("{}", 1990 + seed % 35)),
        json!({"key": "APIC:front", "value": {"type": "binary", "len": 20_000 + seed % 9_000}}),
    ];
    for n in 0..40 {
        items.push(text(
            &format!("TXXX:note{n}"),
            format!("made-up note {n} of track {seed}"),
        ));
    }
    json!({ "id3v2": items }).to_string()
}

/// Adds about `n` files in tracks of one to three, and returns how many
/// tracks that made.
fn fill(db: &Db, n: usize) -> usize {
    db.writer
        .call(move |c| {
            let tx = c.transaction()?;
            let mut track = tx.prepare("INSERT INTO recording DEFAULT VALUES")?;
            let mut file = tx.prepare(
                "INSERT INTO file (music_folder_id, rel_path, rel_path_key, sniffed_format, codec,
                                   bitrate, raw_tags)
                 VALUES (1, ?1, ?1, ?2, ?3, ?4, ?5)",
            )?;
            let mut place =
                tx.prepare("INSERT INTO recording_file (recording_id, file_id) VALUES (?1, ?2)")?;
            let mut library = tx.prepare(
                "INSERT INTO library_track (recording_id, linked_file_id) VALUES (?1, ?2)",
            )?;
            let (mut files, mut tracks) = (0usize, 0usize);
            while files < n {
                tracks += 1;
                track.execute([])?;
                let recording = tx.last_insert_rowid();
                let copies = match tracks % 10 {
                    0 => 3,
                    1..=3 => 2,
                    _ => 1,
                };
                let mut first = None;
                for copy in 0..copies {
                    files += 1;
                    let (sniffed, codec, kbps): (&str, &str, Option<i64>) =
                        match (tracks + copy) % 3 {
                            0 => ("flac", "flac", None),
                            1 => ("mp3", "mp3", Some(320)),
                            _ => ("ogg", "vorbis", Some(192)),
                        };
                    file.execute(rusqlite::params![
                        format!("{tracks}-{copy}.{sniffed}"),
                        sniffed,
                        codec,
                        kbps,
                        tags((tracks * 4 + copy) as u64),
                    ])?;
                    let id = tx.last_insert_rowid();
                    place.execute([recording, id])?;
                    first.get_or_insert(id);
                }
                if tracks % 5 == 0 {
                    library.execute([recording, first.unwrap_or_default()])?;
                }
            }
            drop((track, file, place, library));
            tx.commit()?;
            Ok(tracks)
        })
        .unwrap()
}

struct Report {
    files: usize,
    tracks: usize,
    first: (Summary, Duration),
    idle: (Summary, Duration),
    some: (Summary, Duration),
}

fn steps(n: usize, due: usize) -> Report {
    let db = Db::new();
    let tracks = fill(&db, n);
    let files = db.count("SELECT count(*) FROM file") as usize;
    let first = db.settle();
    let idle = db.settle();
    // `due` tracks have a file that goes missing.
    db.writer
        .call(move |c| {
            c.execute(
                "UPDATE file SET present = 0
                 WHERE id IN (SELECT file_id FROM recording_file
                              WHERE recording_id IN (SELECT id FROM recording ORDER BY id LIMIT ?1)
                                AND role = 'best')",
                [due as i64],
            )
        })
        .unwrap();
    let some = db.settle();
    // Checked on the same database.
    assert_eq!(
        db.count(
            "SELECT count(*) FROM (SELECT recording_id FROM recording_file GROUP BY recording_id
                                    HAVING count(*) FILTER (WHERE role = 'best') <> 1)"
        ),
        0,
        "every track has exactly one best"
    );
    assert_eq!(db.count("SELECT count(*) FROM best_file_due"), 0);
    Report {
        files,
        tracks,
        first,
        idle,
        some,
    }
}

fn print(report: &Report) {
    let show = |name: &str, (summary, took): &(Summary, Duration)| {
        println!(
            "{name}: {:.2} s, {} tracks ranked, {} best files changed",
            took.as_secs_f64(),
            summary.ranked,
            summary.best_changed
        );
    };
    println!("{} files in {} tracks", report.files, report.tracks);
    show("first full pass", &report.first);
    show("idle run", &report.idle);
    show("run with tracks due", &report.some);
}

fn check(report: &Report, due: usize) {
    assert_eq!(report.first.0.ranked as usize, report.tracks);
    assert_eq!(report.idle.0, Summary::default(), "nothing was due");
    assert_eq!(report.some.0.ranked as usize, due, "only the due tracks");
}

#[test]
fn a_small_library_goes_through_the_same_steps() {
    let report = steps(3_000, 100);
    print(&report);
    check(&report, 100);
}

#[test]
#[ignore = "a measurement: run alone, in a release build; see the module docs"]
fn a_library_of_100k() {
    let report = steps(100_000, 100);
    print(&report);
    check(&report, 100);
}

#[test]
#[ignore = "a measurement: run alone, in a release build; see the module docs"]
fn a_library_of_10k() {
    let report = steps(10_000, 100);
    print(&report);
    check(&report, 100);
}
