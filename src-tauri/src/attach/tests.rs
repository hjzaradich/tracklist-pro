//! Attaching rekordbox data (1aD-4). Every fixture is synthetic: made-up
//! files and values only.

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use super::*;
use crate::db::Writer;
use crate::grouping::regroup;
use crate::jobs::{self, JobKind, JobQueue};

/// Long enough that a passing test never hits it.
const PATIENCE: Duration = Duration::from_secs(30);

const A: [u8; 34] = [0xA1; 34];
const B: [u8; 34] = [0xB2; 34];

struct Db {
    _dir: tempfile::TempDir,
    writer: Writer,
}

/// A migrated database with one volume and one music folder (id 1).
fn db() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    writer
        .call(|c| {
            c.execute_batch(
                "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'A', 'A');",
            )
        })
        .unwrap();
    Db { _dir: dir, writer }
}

impl Db {
    /// A present file hashed as `hash`. Same hash, same track once grouped.
    fn file(&self, name: &str, hash: [u8; 34]) -> i64 {
        let name = name.to_string();
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime, audio_hash)
                     VALUES (1, ?1, ?1, 100, 1000, ?2)",
                    (name, hash.to_vec()),
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// A rekordbox entry matched to `file` (`None`: unmatched, missing).
    /// Run [`Db::group`] to give it its track.
    fn entry(&self, track_id: i64, file: Option<i64>, bpm: &str, tonality: &str, plays: i64) {
        self.entry_with(track_id, file, bpm, tonality, plays, false);
    }

    fn entry_with(
        &self,
        track_id: i64,
        file: Option<i64>,
        bpm: &str,
        tonality: &str,
        plays: i64,
        probable: bool,
    ) {
        let attributes = serde_json::json!({
            "TrackID": track_id.to_string(),
            "Location": format!("file://localhost/C:/Kit/{track_id}.mp3"),
            "AverageBpm": bpm,
            "Tonality": tonality,
            "PlayCount": plays.to_string(),
            "Comments": "Opener /* Peak */",
        })
        .to_string();
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO rekordbox_track
                         (attributes, location_key, file_id, relink_method, relink_confidence,
                          relink_probable, read_at)
                     VALUES (?1, ?2, ?3, ?4, 1.0, ?5, '2026-01-01T00:00:00.000Z')",
                    (
                        attributes,
                        format!("c:/kit/{track_id}.mp3"),
                        file,
                        file.map(|_| "path"),
                        probable,
                    ),
                )
            })
            .unwrap();
    }

    fn group(&self) {
        self.writer.call(regroup).unwrap();
    }

    fn attach(&self) -> Summary {
        self.writer.call(attach).unwrap()
    }

    fn track(&self, file: i64) -> i64 {
        self.sql_one(&format!(
            "SELECT recording_id FROM recording_file WHERE file_id = {file}"
        ))
    }

    fn sql_one<T: rusqlite::types::FromSql + Send + 'static>(&self, sql: &str) -> T {
        let sql = sql.to_string();
        self.writer
            .call(move |c| c.query_row(&sql, [], |r| r.get(0)))
            .unwrap()
    }

    fn sql(&self, sql: &str) {
        let sql = sql.to_string();
        self.writer.call(move |c| c.execute_batch(&sql)).unwrap();
    }

    /// The track's rekordbox row: (bpm, key).
    fn rekordbox_row(&self, recording: i64) -> Option<(Option<f64>, Option<String>)> {
        use rusqlite::OptionalExtension;
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT bpm, key FROM analysis WHERE recording_id = ?1 AND source = 'rekordbox'",
                    [recording],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
            })
            .unwrap()
    }

    fn rows(&self) -> i64 {
        self.sql_one("SELECT count(*) FROM analysis WHERE source = 'rekordbox'")
    }
}

fn row(bpm: f64, key: &str) -> Option<(Option<f64>, Option<String>)> {
    Some((Some(bpm), Some(key.to_string())))
}

// ---- values -----------------------------------------------------------------

#[test]
fn tonality_becomes_camelot_with_the_confirmed_spellings() {
    for (tonality, camelot) in [
        ("Am", "8A"),
        ("C", "8B"),
        ("F#", "2B"),
        ("Gb", "2B"),
        ("Bb", "6B"),
        ("A#", "6B"),
        ("Abm", "1A"),
        ("F#m", "11A"),
        ("8A", "8A"),
    ] {
        assert_eq!(
            key_value(Some(tonality)).as_deref(),
            Some(camelot),
            "{tonality}"
        );
    }
    assert_eq!(key_value(Some("")), None);
    assert_eq!(key_value(Some("not a key")), None);
    assert_eq!(key_value(None), None);
}

#[test]
fn a_bpm_of_zero_or_nonsense_is_no_bpm() {
    assert_eq!(bpm_value(Some(128.5)), Some(128.5));
    assert_eq!(bpm_value(Some(0.0)), None);
    assert_eq!(bpm_value(Some(-3.0)), None);
    assert_eq!(bpm_value(Some(1000.0)), None);
    assert_eq!(bpm_value(None), None);
}

// ---- attaching ----------------------------------------------------------------

#[test]
fn bpm_and_key_are_written_to_analysis_as_rekordbox_and_the_track_shows_them() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "F#", 0);
    db.group();
    let summary = db.attach();
    assert_eq!(
        summary,
        Summary {
            added: 1,
            updated: 0,
            removed: 0
        }
    );
    let track = db.track(file);
    assert_eq!(db.rekordbox_row(track), row(128.0, "2B"));
    let shown: (f64, String, String) = db
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT bpm, bpm_source, key_source FROM recording_display WHERE recording_id = ?1",
                [track],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .unwrap();
    assert_eq!(shown, (128.0, "rekordbox".into(), "rekordbox".into()));
}

#[test]
fn an_entry_with_a_bpm_but_no_key_gives_a_row_with_only_the_bpm() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "90.00", "", 0);
    db.group();
    db.attach();
    assert_eq!(db.rekordbox_row(db.track(file)), Some((Some(90.0), None)));
}

#[test]
fn an_entry_with_neither_bpm_nor_key_gives_no_row() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "0.00", "", 0);
    db.group();
    assert_eq!(db.attach(), Summary::default());
    assert_eq!(db.rows(), 0);
}

#[test]
fn a_probable_match_attaches_nothing() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry_with(1, Some(file), "128.00", "Am", 0, true);
    db.group();
    assert_eq!(db.attach(), Summary::default());
    assert_eq!(db.rows(), 0);
}

#[test]
fn a_row_with_no_file_and_so_no_track_attaches_nothing() {
    let db = db();
    db.file("a.mp3", A);
    db.entry(1, None, "128.00", "Am", 0);
    db.group();
    assert_eq!(db.attach(), Summary::default());
    assert_eq!(db.rows(), 0);
}

#[test]
fn a_row_that_turns_probable_loses_what_it_gave() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "Am", 0);
    db.group();
    db.attach();
    assert_eq!(db.rows(), 1);
    db.sql("UPDATE rekordbox_track SET relink_probable = 1");
    assert_eq!(
        db.attach(),
        Summary {
            removed: 1,
            ..Summary::default()
        }
    );
    assert_eq!(db.rows(), 0);
}

#[test]
fn a_row_that_becomes_unmatched_loses_what_it_gave() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "Am", 0);
    db.group();
    db.attach();
    db.sql("UPDATE rekordbox_track SET file_id = NULL, recording_id = NULL, relink_method = NULL");
    db.group();
    assert_eq!(db.attach().removed, 1);
    assert_eq!(db.rows(), 0);
}

#[test]
fn a_row_matched_to_another_track_moves_what_it_gave_there() {
    let db = db();
    let first = db.file("a.mp3", A);
    let second = db.file("b.mp3", B);
    db.entry(1, Some(first), "128.00", "Am", 0);
    db.group();
    db.attach();
    let (from, to) = (db.track(first), db.track(second));
    assert_eq!(db.rekordbox_row(from), row(128.0, "8A"));
    db.sql(&format!("UPDATE rekordbox_track SET file_id = {second}"));
    db.group();
    let summary = db.attach();
    assert_eq!((summary.added, summary.removed), (1, 1));
    assert_eq!(db.rekordbox_row(from), None);
    assert_eq!(db.rekordbox_row(to), row(128.0, "8A"));
}

#[test]
fn a_changed_value_updates_the_row_in_place() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "Am", 0);
    db.group();
    db.attach();
    // A fresh read replaces the snapshot; relink and grouping match it again.
    db.sql("DELETE FROM rekordbox_track");
    db.entry(1, Some(file), "130.00", "Am", 0);
    db.group();
    let summary = db.attach();
    assert_eq!((summary.added, summary.updated, summary.removed), (0, 1, 0));
    assert_eq!(db.rekordbox_row(db.track(file)), row(130.0, "8A"));
}

#[test]
fn running_again_with_nothing_changed_changes_nothing() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "Am", 0);
    db.group();
    db.attach();
    db.sql("UPDATE analysis SET analyzed_at = '2000-01-01T00:00:00.000Z'");
    assert_eq!(db.attach(), Summary::default());
    let at: String = db.sql_one("SELECT analyzed_at FROM analysis");
    assert_eq!(at, "2000-01-01T00:00:00.000Z");
}

#[test]
fn other_sources_rows_are_never_touched() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "Am", 0);
    db.group();
    let track = db.track(file);
    db.sql(&format!(
        "INSERT INTO analysis (recording_id, source, bpm, key) VALUES ({track}, 'tag', 126, '9A')"
    ));
    db.attach();
    db.sql("UPDATE rekordbox_track SET relink_probable = 1");
    db.attach();
    let tag: f64 = db.sql_one("SELECT bpm FROM analysis WHERE source = 'tag'");
    assert_eq!(tag, 126.0);
}

// ---- duplicate entries ------------------------------------------------------

/// One track (one audio hash) with three files; the best is the first.
fn three_files(db: &Db) -> (i64, i64, i64) {
    let files = (
        db.file("a.mp3", A),
        db.file("b.mp3", A),
        db.file("c.mp3", A),
    );
    db.group();
    assert_eq!(db.track(files.0), db.track(files.1));
    assert_eq!(
        db.sql_one::<String>(&format!(
            "SELECT role FROM recording_file WHERE file_id = {}",
            files.0
        )),
        "best"
    );
    files
}

#[test]
fn of_duplicate_entries_the_one_on_the_best_file_wins_over_a_higher_play_count() {
    let db = db();
    let (best, other, _) = three_files(&db);
    db.entry(1, Some(other), "100.00", "Am", 50);
    db.entry(2, Some(best), "128.00", "Cm", 0);
    db.group();
    db.attach();
    assert_eq!(db.rekordbox_row(db.track(best)), row(128.0, "5A"));
    assert_eq!(db.rows(), 1);
}

#[test]
fn with_no_entry_on_the_best_file_the_most_played_entry_wins() {
    let db = db();
    let (_, second, third) = three_files(&db);
    db.entry(1, Some(second), "100.00", "Am", 3);
    db.entry(2, Some(third), "128.00", "Cm", 9);
    db.group();
    db.attach();
    assert_eq!(db.rekordbox_row(db.track(second)), row(128.0, "5A"));
}

#[test]
fn with_equal_play_counts_the_lowest_track_id_wins() {
    let db = db();
    let (_, second, third) = three_files(&db);
    db.entry(7, Some(third), "128.00", "Cm", 2);
    db.entry(5, Some(second), "100.00", "Am", 2);
    db.group();
    db.attach();
    assert_eq!(db.rekordbox_row(db.track(second)), row(100.0, "8A"));
}

#[test]
fn a_probable_duplicate_never_takes_the_pick_even_when_it_is_most_played() {
    let db = db();
    let (best, other, _) = three_files(&db);
    db.entry(1, Some(best), "128.00", "Am", 0);
    db.entry_with(2, Some(other), "100.00", "Cm", 99, true);
    db.group();
    db.attach();
    assert_eq!(db.rekordbox_row(db.track(best)), row(128.0, "8A"));
}

#[test]
fn the_other_duplicate_entries_stay_readable_for_review() {
    let db = db();
    let (best, other, _) = three_files(&db);
    db.entry(1, Some(other), "100.00", "Am", 50);
    db.entry(2, Some(best), "128.00", "Cm", 0);
    db.group();
    db.attach();
    let track = db.track(best);
    let entries = db.writer.call(move |c| for_recording(c, track)).unwrap();
    let ids: Vec<(i64, bool)> = entries
        .iter()
        .map(|e| (e.track_id, e.is_analysis_source))
        .collect();
    assert_eq!(ids, [(2, true), (1, false)]);
}

// ---- reading per track ----------------------------------------------------------

#[test]
fn a_tracks_rekordbox_data_is_read_by_its_id_without_the_my_tags_block() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "F#m", 4);
    db.group();
    let track = db.track(file);
    let entries = db.writer.call(move |c| for_recording(c, track)).unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!((e.track_id, e.file_id, e.play_count), (1, file, Some(4)));
    assert_eq!(e.tonality.as_deref(), Some("F#m"));
    assert_eq!(e.key.as_deref(), Some("11A"));
    assert_eq!(e.comments.as_deref(), Some("Opener /* Peak */"));
    assert_eq!(e.comment_text.as_deref(), Some("Opener"));
    assert!(e.is_analysis_source);
}

#[test]
fn a_probable_entry_is_not_listed_for_the_track() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry_with(1, Some(file), "128.00", "Am", 0, true);
    db.group();
    let track = db.track(file);
    let entries = db.writer.call(move |c| for_recording(c, track)).unwrap();
    assert!(entries.is_empty());
}

// ---- merging tracks -----------------------------------------------------------------

#[test]
fn merging_two_tracks_that_both_have_rekordbox_analysis_succeeds_and_leaves_one_row() {
    let db = db();
    let first = db.file("a.mp3", A);
    let second = db.file("b.mp3", B);
    db.entry(1, Some(first), "128.00", "Am", 0);
    db.entry(2, Some(second), "100.00", "Cm", 0);
    db.group();
    db.attach();
    let (one, two) = (db.track(first), db.track(second));
    assert_ne!(one, two);
    assert_eq!(db.rows(), 2);

    // The second file's audio turns out to be the first's: the tracks merge.
    db.sql(&format!(
        "UPDATE file SET audio_hash = x'{}' WHERE id = {second}",
        "A1".repeat(34)
    ));
    db.group();
    let merged = db.track(first);
    assert_eq!(db.track(second), merged);
    assert_eq!(db.sql_one::<i64>("SELECT count(*) FROM recording"), 1);
    // Only the surviving track's row is left, until attach picks again.
    assert_eq!(db.rows(), 1);
    db.attach();
    assert_eq!(db.rows(), 1);
    // Both entries are on the merged track now, neither on the best file's
    // say-so alone: the best file is the first one, so its entry wins.
    assert_eq!(db.rekordbox_row(merged), row(128.0, "8A"));
}

#[test]
fn a_merge_moves_other_sources_rows_to_the_surviving_track_unless_it_has_its_own() {
    let db = db();
    let first = db.file("a.mp3", A);
    let second = db.file("b.mp3", B);
    db.group();
    let (one, two) = (db.track(first), db.track(second));
    db.sql(&format!(
        "INSERT INTO analysis (recording_id, source, bpm) VALUES ({one}, 'tag', 120);
         INSERT INTO analysis (recording_id, source, bpm) VALUES ({two}, 'tag', 99);
         INSERT INTO analysis (recording_id, source, energy) VALUES ({two}, 'mik', 7);"
    ));
    db.sql(&format!(
        "UPDATE file SET audio_hash = x'{}' WHERE id = {second}",
        "A1".repeat(34)
    ));
    db.group();
    let merged = db.track(first);
    assert_eq!(db.track(second), merged);
    // The survivor's own tag row stays; the other track's mik row came over.
    let tag: f64 = db.sql_one(&format!(
        "SELECT bpm FROM analysis WHERE recording_id = {merged} AND source = 'tag'"
    ));
    let mik: i64 = db.sql_one(&format!(
        "SELECT energy FROM analysis WHERE recording_id = {merged} AND source = 'mik'"
    ));
    assert_eq!((tag, mik), (120.0, 7));
    assert_eq!(db.sql_one::<i64>("SELECT count(*) FROM analysis"), 2);
    assert_eq!(db.sql_one::<i64>("SELECT count(*) FROM recording"), 1);
}

#[test]
fn a_track_whose_folder_is_released_does_not_keep_its_rekordbox_row_or_itself() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "Am", 0);
    db.group();
    db.attach();
    assert_eq!(db.rows(), 1);
    // A fresh read without the entry, then the folder is removed.
    db.sql("DELETE FROM rekordbox_track");
    db.writer
        .call(|c| crate::grouping::release_folder_files(c, 1))
        .unwrap();
    assert_eq!(db.rows(), 0);
    assert_eq!(db.sql_one::<i64>("SELECT count(*) FROM recording"), 0);
}

// ---- the job --------------------------------------------------------------------------

#[test]
fn asking_with_no_rekordbox_data_queues_nothing() {
    let db = db();
    db.file("a.mp3", A);
    db.group();
    let queue = JobQueue::builder(db.writer.clone()).start().unwrap();
    let asked = request(&db.writer, |j| queue.enqueue(j)).unwrap();
    queue.shutdown();
    assert_eq!(asked, None);
    assert_eq!(db.sql_one::<i64>("SELECT count(*) FROM job"), 0);
}

#[test]
fn asking_again_while_an_attach_job_is_waiting_returns_that_job() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "Am", 0);
    db.group();
    let (release, hold) = mpsc::channel::<()>();
    let hold = Mutex::new(hold);
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        // Keeps the only worker busy, so attach jobs stay queued.
        .handler(JobKind::Scan, move |_: &jobs::JobContext| {
            let _ = hold.lock().unwrap().recv();
            Ok(())
        })
        .handler(JobKind::Attach, Attacher::default())
        .start()
        .unwrap();
    queue.enqueue(jobs::NewJob::new(JobKind::Scan)).unwrap();
    let first = request(&db.writer, |j| queue.enqueue(j)).unwrap();
    let second = request(&db.writer, |j| queue.enqueue(j)).unwrap();
    assert!(first.is_some());
    assert_eq!(first, second);
    release.send(()).unwrap();
    queue.shutdown();
}

/// A queue running `attach` (and, with `grouper`, grouping), with every
/// attach summary sent to the returned receiver.
fn queue_with(db: &Db, grouper: bool) -> (JobQueue, mpsc::Receiver<Summary>) {
    let (send, heard) = mpsc::channel();
    let send = Mutex::new(send);
    let mut builder = JobQueue::builder(db.writer.clone()).handler(
        JobKind::Attach,
        Attacher::default().on_summary(move |s| {
            let _ = send.lock().unwrap().send(s);
        }),
    );
    if grouper {
        builder = builder.handler(JobKind::Group, crate::grouping::Grouper::default());
    }
    (builder.start().unwrap(), heard)
}

#[test]
fn the_job_attaches_and_reports_what_it_did() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "Am", 0);
    db.group();
    let (queue, heard) = queue_with(&db, false);
    request(&db.writer, |j| queue.enqueue(j)).unwrap().unwrap();
    let summary = heard.recv_timeout(PATIENCE).unwrap();
    queue.shutdown();
    assert_eq!(summary.added, 1);
    assert_eq!(db.rekordbox_row(db.track(file)), row(128.0, "8A"));
}

#[test]
fn a_grouping_run_asks_for_an_attach() {
    let db = db();
    let file = db.file("a.mp3", A);
    // Matched but not yet grouped: grouping gives the entry its track, and
    // the attach it asks for writes the row.
    db.entry(1, Some(file), "128.00", "Am", 0);
    let (queue, heard) = queue_with(&db, true);
    queue.enqueue(crate::grouping::group_job()).unwrap();
    let summary = heard.recv_timeout(PATIENCE).unwrap();
    queue.shutdown();
    assert_eq!(summary.added, 1);
    assert_eq!(db.rows(), 1);
}

#[test]
fn a_second_run_with_nothing_changed_reports_nothing_done() {
    let db = db();
    let file = db.file("a.mp3", A);
    db.entry(1, Some(file), "128.00", "Am", 0);
    db.group();
    let (queue, heard) = queue_with(&db, false);
    queue.enqueue(attach_job()).unwrap();
    assert_eq!(heard.recv_timeout(PATIENCE).unwrap().added, 1);
    queue.enqueue(attach_job()).unwrap();
    assert_eq!(heard.recv_timeout(PATIENCE).unwrap(), Summary::default());
    queue.shutdown();
}
