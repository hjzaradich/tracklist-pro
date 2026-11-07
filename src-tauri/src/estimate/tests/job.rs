//! The analyze job on a real (temp) music folder, after the real walk and
//! the real hash stage.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use super::support::{wait, Library, TempVolume};
use super::{audio, clicks, silence};
use crate::estimate::ledger::{self, Stored};
use crate::estimate::{analyze_job, Analyzer, PRIORITY, VERSION};
use crate::hash::Hasher;
use crate::jobs::{JobKind, JobQueue, JobStatus, Priority};
use crate::scan::walk::Walker;
use crate::waveform::tests::join_library_for_test;

const RATE: u32 = 22_050;

fn wav(bpm: f64, seconds: f64) -> Vec<u8> {
    audio::wav(&clicks(bpm, seconds, RATE))
}

fn analyzer(library: &Library) -> Analyzer<TempVolume> {
    let volume = library.volume.clone();
    Analyzer::new(move || volume.clone(), library.first.clone())
}

fn queue(library: &Library, analyzer: Analyzer<TempVolume>) -> JobQueue {
    let (walk_volume, hash_volume) = (library.volume.clone(), library.volume.clone());
    JobQueue::builder(library.writer.clone())
        .workers(1)
        .handler(JobKind::Scan, Walker::new(move || walk_volume.clone()))
        .handler(JobKind::Hash, Hasher::new(move || hash_volume.clone()))
        .handler(JobKind::Analyze, analyzer)
        .start()
        .unwrap()
}

/// The real walk and hash, then every file is a Library track's file.
fn walk_and_hash(library: &Library) -> BTreeMap<String, i64> {
    let ids = library.walk_and_hash();
    library.writer.call(|c| join_library_for_test(c)).unwrap();
    ids
}

/// Runs one analyze job to the end, returning how many files it decoded.
fn analyze_all(library: &Library) -> usize {
    let decoded = Arc::new(AtomicUsize::new(0));
    let count = decoded.clone();
    let handler = analyzer(library).on_decode(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    });
    let queue = queue(library, handler);
    let id = queue.enqueue(analyze_job()).unwrap();
    assert_eq!(wait(&library.writer, id).status, JobStatus::Done);
    queue.shutdown();
    decoded.load(Ordering::SeqCst)
}

fn stored(library: &Library, file: i64) -> Option<Stored> {
    library.writer.call(move |c| ledger::current(c, file)).unwrap()
}

/// The track file `file` belongs to.
fn track_of(library: &Library, file: i64) -> i64 {
    library
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT recording_id FROM recording_file WHERE file_id = ?1",
                [file],
                |r| r.get(0),
            )
        })
        .unwrap()
}

/// The track's `'local'` row: its id, BPM, key and energy.
fn local_row(library: &Library, track: i64) -> Option<(i64, Option<f64>, Option<String>, Option<i64>)> {
    use rusqlite::OptionalExtension;
    library
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT id, bpm, key, energy FROM analysis
                 WHERE recording_id = ?1 AND source = 'local'",
                [track],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
        })
        .unwrap()
}

/// The BPM the track shows, and from which source: by the view, and by the
/// track table's own subquery.
fn shown_bpm(library: &Library, track: i64) -> (Option<f64>, Option<String>, Option<f64>) {
    let subquery = crate::search::BPM_AND_KEY;
    library
        .writer
        .call(move |c| {
            let (bpm, source) = c.query_row(
                "SELECT bpm, bpm_source FROM recording_display WHERE recording_id = ?1",
                [track],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let table: Option<f64> = c.query_row(
                &format!("SELECT {subquery} FROM recording r WHERE r.id = ?1"),
                [track],
                |r| r.get(0),
            )?;
            Ok((bpm, source, table))
        })
        .unwrap()
}

fn near(bpm: Option<f64>, expected: f64) -> bool {
    bpm.is_some_and(|b| (b - expected).abs() <= 0.5)
}

#[test]
fn each_library_track_gets_a_local_bpm_once_and_a_rerun_decodes_nothing() {
    let library = Library::new();
    library.put("a.wav", &wav(128.0, 30.0));
    library.put("Folder/b.wav", &wav(87.0, 30.0));
    let ids = walk_and_hash(&library);
    assert_eq!(analyze_all(&library), 2);
    let (a, b) = (track_of(&library, ids["a.wav"]), track_of(&library, ids["Folder/b.wav"]));
    let row_a = local_row(&library, a).unwrap();
    assert!(near(row_a.1, 128.0), "{row_a:?}");
    // The ledger keeps the detector's answer; the track shows it folded.
    let ledger_b = stored(&library, ids["Folder/b.wav"]).unwrap();
    assert!(near(ledger_b.bpm, 87.0), "{ledger_b:?}");
    assert!(near(local_row(&library, b).unwrap().1, 174.0));
    // No key and no energy, ever, from the estimate.
    for track in [a, b] {
        let (_, _, key, energy) = local_row(&library, track).unwrap();
        assert_eq!((key, energy), (None, None));
    }
    let (bpm, source, table) = shown_bpm(&library, a);
    assert_eq!((bpm, source.as_deref()), (row_a.1, Some("local")));
    assert_eq!(table, row_a.1);

    // Measured at this version and of this audio: nothing is due.
    assert_eq!(analyze_all(&library), 0);
    assert!(!library
        .writer
        .call(|c| crate::estimate::due(c, None))
        .unwrap());
    assert_eq!(local_row(&library, a).unwrap(), row_a);
}

#[test]
fn silence_and_a_very_short_file_are_skipped_with_a_reason_and_get_no_analysis_row() {
    let library = Library::new();
    library.put("quiet.wav", &audio::wav(&silence(30.0, RATE)));
    library.put("blip.wav", &wav(128.0, 4.0));
    let ids = walk_and_hash(&library);
    assert_eq!(analyze_all(&library), 2);
    for (name, why) in [("quiet.wav", "silent"), ("blip.wav", "too_short")] {
        let row = stored(&library, ids[name]).unwrap();
        assert_eq!(row.skipped.as_deref(), Some(why), "{name}");
        assert_eq!((row.bpm, row.analysed_ms), (None, None));
        assert_eq!(local_row(&library, track_of(&library, ids[name])), None);
    }
    // Skipped is an answer: not tried again until the audio changes.
    assert_eq!(analyze_all(&library), 0);
}

#[test]
fn a_new_method_version_measures_again_and_updates_the_local_row_in_place() {
    let library = Library::new();
    library.put("a.wav", &wav(128.0, 30.0));
    let ids = walk_and_hash(&library);
    analyze_all(&library);
    let track = track_of(&library, ids["a.wav"]);
    let (row_id, ..) = local_row(&library, track).unwrap();
    // As if measured by another method: the row and the value are stale.
    library
        .writer
        .call(move |c| {
            c.execute(
                "UPDATE file_estimate SET method_version = ?1, bpm = 61.0",
                [i64::from(VERSION) + 1],
            )?;
            c.execute("UPDATE analysis SET bpm = 122.0 WHERE id = ?1", [row_id])
        })
        .unwrap();
    assert!(library
        .writer
        .call(|c| crate::estimate::due(c, None))
        .unwrap());
    assert_eq!(analyze_all(&library), 1);
    let (again, bpm, ..) = local_row(&library, track).unwrap();
    assert_eq!(again, row_id, "the same row, updated in place");
    assert!(near(bpm, 128.0), "{bpm:?}");
    let count: i64 = library
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM analysis", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn a_bpm_from_rekordbox_hides_the_local_one_which_stays() {
    let library = Library::new();
    library.put("a.wav", &wav(128.0, 30.0));
    let ids = walk_and_hash(&library);
    let track = track_of(&library, ids["a.wav"]);
    library
        .writer
        .call(move |c| {
            c.execute(
                "INSERT INTO analysis (recording_id, source, bpm, key) VALUES (?1, 'rekordbox', 120.0, '8A')",
                [track],
            )
        })
        .unwrap();
    analyze_all(&library);
    assert!(near(local_row(&library, track).unwrap().1, 128.0));
    let (bpm, source, table) = shown_bpm(&library, track);
    assert_eq!((bpm, source.as_deref(), table), (Some(120.0), Some("rekordbox"), Some(120.0)));
}

#[test]
fn a_cancel_mid_run_keeps_only_whole_results() {
    let library = Library::new();
    for name in ["a.wav", "b.wav", "c.wav"] {
        library.put(name, &wav(128.0, 30.0));
    }
    let ids = walk_and_hash(&library);
    let (at_second, second) = mpsc::channel();
    let (go, wait_for_go) = mpsc::channel::<()>();
    let (at_second, wait_for_go) = (Mutex::new(at_second), Mutex::new(wait_for_go));
    let taken = Arc::new(AtomicUsize::new(0));
    let count = taken.clone();
    let handler = analyzer(&library).on_file(move |_| {
        if count.fetch_add(1, Ordering::SeqCst) == 1 {
            at_second.lock().unwrap().send(()).unwrap();
            wait_for_go.lock().unwrap().recv().unwrap();
        }
    });
    let queue = queue(&library, handler);
    let job = queue.enqueue(analyze_job()).unwrap();
    second.recv().unwrap();
    queue.cancel(job).unwrap();
    go.send(()).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Cancelled);
    queue.shutdown();
    let measured = ids.values().filter(|&&id| stored(&library, id).is_some()).count();
    assert_eq!(measured, 1, "the first file only");
    let rows: i64 = library
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM analysis WHERE source = 'local'", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(rows, 1);
    // What was stored stays: the next run does the other two.
    assert_eq!(analyze_all(&library), 2);
}

#[test]
fn a_file_outside_the_library_is_not_analysed() {
    let library = Library::new();
    library.put("a.wav", &wav(128.0, 30.0));
    library.walk_and_hash();
    assert!(!library
        .writer
        .call(|c| crate::estimate::due(c, None))
        .unwrap());
    assert_eq!(analyze_all(&library), 0);
}

#[test]
fn the_job_runs_at_the_waveform_jobs_priority_the_lowest_there_is() {
    assert_eq!(analyze_job().kind, JobKind::Analyze);
    assert_eq!(analyze_job().priority, PRIORITY);
    assert_eq!(PRIORITY, crate::waveform::PRIORITY);
    assert!(PRIORITY < Priority::BACKGROUND);
}
