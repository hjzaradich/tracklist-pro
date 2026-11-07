//! The analyze job on a real (temp) music folder, after the real walk and
//! the real hash stage.

use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use super::support::{wait, Library, TempVolume};
use super::{audio, clicks, scaled, silence};
use crate::estimate::ledger::{self, Stored};
use crate::estimate::support::{queued, store_row};
use crate::estimate::{analyze_job, Analyzer, PRIORITY, VERSION};
use crate::hash::{hash_job, Hasher};
use crate::jobs::{JobContext, JobError, JobKind, JobQueue, JobStatus, NewJob, Priority};
use crate::scan::walk::{scan_job, Walker};
use crate::settings::BpmRange;
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
pub(super) fn walk_and_hash(library: &Library) -> BTreeMap<String, i64> {
    let ids = library.walk_and_hash();
    library.writer.call(|c| join_library_for_test(c)).unwrap();
    ids
}

/// Runs one analyze job to the end, returning how many files it decoded.
pub(super) fn analyze_all(library: &Library) -> usize {
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

pub(super) fn stored(library: &Library, file: i64) -> Option<Stored> {
    library
        .writer
        .call(move |c| ledger::current(c, file))
        .unwrap()
}

/// The track file `file` belongs to.
pub(super) fn track_of(library: &Library, file: i64) -> i64 {
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

/// A `'local'` analysis row: its id, BPM, key and energy.
pub(super) type LocalRow = (i64, Option<f64>, Option<String>, Option<i64>);

/// The track's `'local'` row.
pub(super) fn local_row(library: &Library, track: i64) -> Option<LocalRow> {
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
    let (a, b) = (
        track_of(&library, ids["a.wav"]),
        track_of(&library, ids["Folder/b.wav"]),
    );
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
fn a_run_stores_the_energy_features_and_works_the_suggestions_out_in_the_ledger_only() {
    let library = Library::new();
    library.put(
        "slow quiet.wav",
        &audio::wav(&scaled(clicks(90.0, 30.0, RATE), 0.1)),
    );
    library.put("fast loud.wav", &wav(180.0, 30.0));
    library.put("mid.wav", &wav(128.0, 30.0));
    let ids = walk_and_hash(&library);
    assert_eq!(analyze_all(&library), 3);

    let suggestion = |name: &str| {
        let track = track_of(&library, ids[name]);
        library
            .writer
            .call(move |c| crate::estimate::suggest::suggestion_for(c, track))
            .unwrap()
            .expect("a suggestion")
    };
    let (slow, fast) = (suggestion("slow quiet.wav"), suggestion("fast loud.wav"));
    assert!(slow.value < fast.value, "{slow:?} {fast:?}");
    assert_eq!(slow.basis.len(), 4, "loudness, onsets, flux and tempo");

    // In the ledger only: a measured file has its features, and no track has
    // an energy of the app's anywhere.
    let features: i64 = library
        .writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM file_estimate
                 WHERE loudness_lufs <= 0 AND onset_density > 0 AND spectral_flux > 0
                   AND energy_suggestion BETWEEN 1 AND 10",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(features, 3);
    let energies = count(
        &library,
        "SELECT count(*) FROM analysis WHERE energy IS NOT NULL",
    );
    assert_eq!(energies, 0);
    // Nothing left to do.
    assert_eq!(analyze_all(&library), 0);
    assert!(!library
        .writer
        .call(|c| crate::estimate::due(c, None))
        .unwrap());
}

#[test]
fn a_file_measured_again_by_the_job_loses_its_old_suggestion_and_is_placed_afresh() {
    let library = Library::new();
    let path = library.put(
        "swapped.wav",
        &audio::wav(&scaled(clicks(90.0, 30.0, RATE), 0.1)),
    );
    library.put("fast loud.wav", &wav(180.0, 30.0));
    library.put("mid.wav", &wav(128.0, 30.0));
    let ids = walk_and_hash(&library);
    assert_eq!(analyze_all(&library), 3);
    let track = track_of(&library, ids["swapped.wav"]);
    let value = |library: &Library| -> u8 {
        library
            .writer
            .call(move |c| crate::estimate::suggest::suggestion_for(c, track))
            .unwrap()
            .expect("a suggestion")
            .value
    };
    let quiet = value(&library);

    // The quiet file's audio is replaced by something louder and denser than
    // the middle one: measured again, it is placed again among the same three.
    fs::write(&path, wav(150.0, 30.0)).unwrap();
    {
        let queue = queue(&library, analyzer(&library));
        for job in [scan_job(None), hash_job(None)] {
            let job = queue.enqueue(job).unwrap();
            assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
        }
        queue.shutdown();
    }
    assert_eq!(analyze_all(&library), 1);
    let loud = value(&library);
    assert!(loud > quiet, "{quiet} then {loud}");
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
fn a_raised_version_keeps_the_local_bpm_until_the_new_measurement_replaces_it_in_place() {
    let library = Library::new();
    library.put("a.wav", &wav(128.0, 30.0));
    let ids = walk_and_hash(&library);
    analyze_all(&library);
    let track = track_of(&library, ids["a.wav"]);
    let (row_id, bpm, ..) = local_row(&library, track).unwrap();
    assert!(near(bpm, 128.0));
    // As if the method had been raised since: the file's ledger row is the
    // old version's.
    bump_version(&library);
    let stale = |library: &Library| library.writer.call(|c| ledger::any_stale(c)).unwrap();
    // Nothing takes the row away, however often the rows are folded again.
    assert!(!stale(&library), "a waiting file is not a mismatch");
    crate::estimate::resync(&library.writer, store_row(&library.writer)).unwrap();
    crate::estimate::request(&library.writer, None, store_row(&library.writer)).unwrap();
    let kept = local_row(&library, track).unwrap();
    assert_eq!((kept.0, kept.1.map(|b| b.round())), (row_id, Some(128.0)));
    assert!(library
        .writer
        .call(|c| crate::estimate::due(c, None))
        .unwrap());

    // The new measurement replaces it, in place.
    assert_eq!(analyze_all(&library), 1);
    let (again, bpm, ..) = local_row(&library, track).unwrap();
    assert_eq!(again, row_id);
    assert!(near(bpm, 128.0), "{bpm:?}");
    let version: i64 = library
        .writer
        .call(|c| c.query_row("SELECT method_version FROM file_estimate", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(version, i64::from(VERSION));
    assert!(!stale(&library));
}

/// As if the method had been raised since every file was measured.
fn bump_version(library: &Library) {
    library
        .writer
        .call(|c| {
            c.execute(
                "UPDATE file_estimate SET method_version = ?1",
                [i64::from(VERSION) - 1],
            )
        })
        .unwrap();
}

#[test]
fn a_relink_onto_a_file_waiting_to_be_measured_again_shows_that_files_own_older_bpm() {
    let (library, ids, ta, _tb) = two_measured();
    let (row_id, bpm, ..) = local_row(&library, ta).unwrap();
    assert!(near(bpm, 128.0));
    bump_version(&library);
    assert!(!is_stale(&library));

    let b = ids["b.wav"];
    library
        .writer
        .call(move |c| {
            c.execute(
                "UPDATE library_track SET linked_file_id = ?1 WHERE recording_id = ?2",
                [b, ta],
            )
        })
        .unwrap();
    assert!(is_stale(&library), "still shows the old file's BPM");
    ask(&library);
    let (again, bpm, ..) = local_row(&library, ta).unwrap();
    assert_eq!(again, row_id, "the same row, updated in place");
    assert!(near(bpm, 174.0), "b.wav's own older measurement: {bpm:?}");
    assert!(!is_stale(&library));

    // And its new measurement replaces that, in place (a.wav is shown by
    // no track now, so only b.wav is measured).
    assert_eq!(analyze_all(&library), 1);
    let (still, bpm, ..) = local_row(&library, ta).unwrap();
    assert_eq!(still, row_id);
    assert!(near(bpm, 174.0), "{bpm:?}");
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
    assert_eq!(
        (bpm, source.as_deref(), table),
        (Some(120.0), Some("rekordbox"), Some(120.0))
    );
}

#[test]
fn a_cancel_mid_run_keeps_only_whole_results() {
    let library = Library::new();
    for (name, bpm) in [("a.wav", 120.0), ("b.wav", 128.0), ("c.wav", 140.0)] {
        library.put(name, &wav(bpm, 30.0));
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
    let measured = ids
        .values()
        .filter(|&&id| stored(&library, id).is_some())
        .count();
    assert_eq!(measured, 1, "the first file only");
    let rows: i64 = library
        .writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM analysis WHERE source = 'local'",
                [],
                |r| r.get(0),
            )
        })
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

// ---- keeping the local rows right -----------------------------------------

/// A time to wait for a job that should finish.
const PATIENCE: Duration = Duration::from_secs(60);

fn count(library: &Library, sql: &'static str) -> i64 {
    library
        .writer
        .call(move |c| c.query_row(sql, [], |r| r.get(0)))
        .unwrap()
}

/// Two measured files, a at 128 and b at 87 (shown as 174), and their tracks.
fn two_measured() -> (Library, BTreeMap<String, i64>, i64, i64) {
    let library = Library::new();
    library.put("a.wav", &wav(128.0, 30.0));
    library.put("b.wav", &wav(87.0, 30.0));
    let ids = walk_and_hash(&library);
    assert_eq!(analyze_all(&library), 2);
    let (ta, tb) = (
        track_of(&library, ids["a.wav"]),
        track_of(&library, ids["b.wav"]),
    );
    (library, ids, ta, tb)
}

fn ask(library: &Library) {
    crate::estimate::request(&library.writer, None, store_row(&library.writer)).unwrap();
}

fn is_stale(library: &Library) -> bool {
    library.writer.call(|c| ledger::any_stale(c)).unwrap()
}

#[test]
fn a_relink_onto_a_file_already_measured_shows_that_files_bpm_at_once_without_a_decode() {
    let (library, ids, ta, _tb) = two_measured();
    let (row_id, bpm, ..) = local_row(&library, ta).unwrap();
    assert!(near(bpm, 128.0));
    assert!(!is_stale(&library));

    // The relink points the track at the other, measured, file.
    let b = ids["b.wav"];
    library
        .writer
        .call(move |c| {
            c.execute(
                "UPDATE library_track SET linked_file_id = ?1 WHERE recording_id = ?2",
                [b, ta],
            )
        })
        .unwrap();
    assert!(is_stale(&library), "still shows the old file's BPM");
    assert!(near(local_row(&library, ta).unwrap().1, 128.0));

    ask(&library);
    let (again, bpm, ..) = local_row(&library, ta).unwrap();
    assert_eq!(again, row_id, "the same row, updated in place");
    assert!(near(bpm, 174.0), "{bpm:?}");
    assert!(!is_stale(&library));
    assert_eq!(
        queued(&library.writer, "analyze"),
        0,
        "there is nothing to measure"
    );
    assert_eq!(queued(&library.writer, "index"), 1, "the table is rebuilt");
    // Asking again changes nothing and queues nothing more.
    ask(&library);
    assert_eq!(queued(&library.writer, "index"), 1);
}

#[test]
fn a_relink_onto_a_file_not_measured_yet_drops_the_old_bpm_and_asks_for_the_job() {
    let (library, ids, ta, _tb) = two_measured();
    let b = ids["b.wav"];
    library
        .writer
        .call(move |c| {
            c.execute("DELETE FROM file_estimate WHERE file_id = ?1", [b])?;
            c.execute(
                "UPDATE library_track SET linked_file_id = ?1 WHERE recording_id = ?2",
                [b, ta],
            )
        })
        .unwrap();
    ask(&library);
    assert_eq!(local_row(&library, ta), None, "no BPM beats the wrong one");
    assert_eq!(queued(&library.writer, "analyze"), 1);
    // The job measures it and the track shows the file's own.
    assert_eq!(analyze_all(&library), 1);
    assert!(near(local_row(&library, ta).unwrap().1, 174.0));
}

#[test]
fn audio_replaced_in_place_is_measured_again_and_the_row_follows() {
    let library = Library::new();
    let path = library.put("Swapped.wav", &wav(128.0, 30.0));
    let ids = walk_and_hash(&library);
    let id = ids["Swapped.wav"];
    assert_eq!(analyze_all(&library), 1);
    let track = track_of(&library, id);
    let (row_id, bpm, ..) = local_row(&library, track).unwrap();
    assert!(near(bpm, 128.0));

    fs::write(&path, wav(87.0, 30.0)).unwrap();
    {
        let queue = queue(&library, analyzer(&library));
        for job in [scan_job(None), hash_job(None)] {
            let job = queue.enqueue(job).unwrap();
            assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
        }
        queue.shutdown();
    }
    // The stored answer is of the old audio: not current, and due.
    assert_eq!(stored(&library, id), None);
    assert!(is_stale(&library));
    assert_eq!(analyze_all(&library), 1, "other audio is measured again");
    let (again, bpm, ..) = local_row(&library, track).unwrap();
    assert_eq!(again, row_id, "the same row, updated in place");
    assert!(near(bpm, 174.0), "{bpm:?}");
    assert_eq!(count(&library, "SELECT count(*) FROM file_estimate"), 1);
    let row_hash: Vec<u8> = library
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT audio_hash FROM file_estimate WHERE file_id = ?1",
                [id],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(row_hash, library.audio_hash(id));
}

#[test]
fn a_run_asks_for_the_index_and_the_track_table_shows_the_new_bpm() {
    let library = Library::new();
    library.put("a.wav", &wav(128.0, 30.0));
    let ids = walk_and_hash(&library);
    let track = track_of(&library, ids["a.wav"]);
    let volume = library.volume.clone();
    let queue = JobQueue::builder(library.writer.clone())
        .workers(1)
        .handler(JobKind::Analyze, analyzer(&library))
        .handler(
            JobKind::Index,
            crate::search::indexer_with(move || volume.clone(), 100),
        )
        .start()
        .unwrap();
    let job = queue.enqueue(analyze_job()).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    let index: Vec<i64> = library
        .writer
        .call(|c| {
            let mut s = c.prepare("SELECT id FROM job WHERE kind = 'index'")?;
            let rows = s.query_map([], |r| r.get(0))?;
            rows.collect()
        })
        .unwrap();
    assert_eq!(index.len(), 1, "the run asked for the index");
    for id in index {
        assert_eq!(
            wait(&library.writer, crate::jobs::JobId(id)).status,
            JobStatus::Done
        );
    }
    queue.shutdown();
    let in_table: Option<f64> = library
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT bpm FROM track_row WHERE recording_id = ?1",
                [track],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert!(near(in_table, 128.0), "{in_table:?}");
}

#[test]
fn setting_the_bpm_range_folds_every_estimate_again_without_a_decode() {
    let library = Library::new();
    library.put("b.wav", &wav(87.0, 30.0));
    let ids = walk_and_hash(&library);
    assert_eq!(analyze_all(&library), 1);
    let track = track_of(&library, ids["b.wav"]);
    assert!(near(local_row(&library, track).unwrap().1, 174.0));

    let range = BpmRange::new(70.0, 140.0).unwrap();
    crate::estimate::set_bpm_range(&library.writer, range, store_row(&library.writer)).unwrap();
    assert!(near(local_row(&library, track).unwrap().1, 87.0));
    assert_eq!(queued(&library.writer, "index"), 1);
    assert_eq!(
        analyze_all(&library),
        0,
        "the ledger kept the detector's answer"
    );
    assert!(near(local_row(&library, track).unwrap().1, 87.0));
    // And back.
    crate::estimate::set_bpm_range(
        &library.writer,
        crate::settings::DEFAULT_BPM_RANGE,
        store_row(&library.writer),
    )
    .unwrap();
    assert!(near(local_row(&library, track).unwrap().1, 174.0));
}

#[test]
fn the_end_of_run_sync_goes_a_page_of_tracks_at_a_time() {
    let (library, _ids, ta, tb) = two_measured();
    let range = crate::settings::DEFAULT_BPM_RANGE;
    let clear = |library: &Library| {
        library
            .writer
            .call(|c| c.execute("DELETE FROM analysis WHERE source = 'local'", []))
            .unwrap()
    };
    clear(&library);
    let page = |after: i64| {
        library
            .writer
            .call(move |c| ledger::sync_page(c, None, after, 1, range))
            .unwrap()
    };
    let (first, second) = (ta.min(tb), ta.max(tb));
    assert_eq!(page(0), (1, Some(first)), "one track per page");
    assert!(local_row(&library, first).is_some());
    assert!(local_row(&library, second).is_none());
    assert_eq!(page(first), (1, Some(second)));
    assert_eq!(page(second), (0, None), "the last page is empty");
    assert!(local_row(&library, second).is_some());

    clear(&library);
    assert_eq!(
        crate::estimate::sync_all(&library.writer, range).unwrap(),
        2,
        "all of them, over however many pages"
    );
}

// ---- making way, the same audio, a cancel while decoding ---------------------

#[test]
fn files_that_hold_the_very_same_audio_are_decoded_once_and_share_the_estimate() {
    let library = Library::new();
    let bytes = wav(128.0, 30.0);
    library.put("a.wav", &bytes);
    library.put("Copy/b.wav", &bytes);
    let ids = walk_and_hash(&library);
    assert_eq!(analyze_all(&library), 1, "decoded once");
    let (a, b) = (ids["a.wav"], ids["Copy/b.wav"]);
    assert_eq!(stored(&library, a), stored(&library, b));
    assert!(stored(&library, a).unwrap().bpm.is_some());
    for file in [a, b] {
        assert!(near(
            local_row(&library, track_of(&library, file)).unwrap().1,
            128.0
        ));
    }
}

/// A queue of three workers that runs analyze jobs and one other kind of
/// job, `Export` (a higher priority than the estimates), whose handler is
/// `export`.
fn queue_with_a_blocking_job(
    library: &Library,
    analyzer: Analyzer<TempVolume>,
    export: impl Fn(&JobContext) -> Result<(), JobError> + Send + Sync + 'static,
) -> JobQueue {
    JobQueue::builder(library.writer.clone())
        .workers(3)
        .handler(JobKind::Analyze, analyzer)
        .handler(JobKind::Export, export)
        .start()
        .unwrap()
}

#[test]
fn the_job_gives_its_thread_back_and_waits_while_a_job_of_higher_priority_is_running() {
    let library = Library::new();
    for (i, name) in ["a.wav", "b.wav", "c.wav"].into_iter().enumerate() {
        library.put(name, &wav(100.0 + 10.0 * i as f64, 30.0));
    }
    walk_and_hash(&library);

    let taken = Arc::new(AtomicUsize::new(0));
    let (counted, first_up) = (taken.clone(), library.first.clone());
    // Taking the first file: the thread is held. Hold the job there until
    // the test has started the other job.
    let (holding, held) = mpsc::channel::<usize>();
    let (go, going) = mpsc::channel::<()>();
    let going = Mutex::new(going);
    let (paused, pause_seen) = mpsc::channel::<()>();
    let paused = Mutex::new(paused);
    let handler = analyzer(&library)
        .budget(1)
        .on_file(move |_| {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                holding.send(first_up.busy()).unwrap();
                going.lock().unwrap().recv_timeout(PATIENCE).unwrap();
            }
        })
        .on_pause(move || {
            paused.lock().unwrap().send(()).ok();
        });
    let (up, export_up) = mpsc::channel();
    let up = Mutex::new(up);
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let blocking = move |_: &JobContext| {
        up.lock().unwrap().send(()).ok();
        released
            .lock()
            .unwrap()
            .recv_timeout(PATIENCE)
            .map_err(|_| JobError::failed("never released"))
    };
    let queue = queue_with_a_blocking_job(&library, handler, blocking);

    let job = queue.enqueue(analyze_job()).unwrap();
    assert_eq!(
        held.recv_timeout(PATIENCE).unwrap(),
        1,
        "it holds the thread"
    );
    // A job that outranks it starts running.
    let export = queue
        .enqueue(NewJob::new(JobKind::Export).priority(Priority::USER))
        .unwrap();
    export_up.recv_timeout(PATIENCE).unwrap();
    go.send(()).unwrap();

    // It finishes the file it has, then gives the thread back and waits.
    pause_seen.recv_timeout(PATIENCE).unwrap();
    assert_eq!(
        library.first.busy(),
        0,
        "the thread went back to the budget"
    );
    assert_eq!(
        taken.load(Ordering::SeqCst),
        1,
        "and no other file was taken"
    );
    assert_eq!(count(&library, "SELECT count(*) FROM file_estimate"), 1);

    // The other job ends: the estimates carry on.
    release.send(()).unwrap();
    assert_eq!(wait(&library.writer, export).status, JobStatus::Done);
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    queue.shutdown();
    assert_eq!(taken.load(Ordering::SeqCst), 3);
    assert_eq!(count(&library, "SELECT count(*) FROM file_estimate"), 3);
    assert_eq!(library.first.busy(), 0);
}

#[test]
fn the_job_makes_way_for_a_waiting_job_of_higher_priority_and_queues_the_rest() {
    let library = Library::new();
    for (i, name) in ["a.wav", "b.wav", "c.wav"].into_iter().enumerate() {
        library.put(name, &wav(100.0 + 10.0 * i as f64, 30.0));
    }
    walk_and_hash(&library);
    let taken = Arc::new(AtomicUsize::new(0));
    let (counted, writer) = (taken.clone(), library.writer.clone());
    // While the first file is taken, a job the user asked for is queued
    // (the single worker is busy, so it waits).
    let handler = analyzer(&library).on_file(move |_| {
        if counted.fetch_add(1, Ordering::SeqCst) == 0 {
            writer
                .call(|c| c.execute("INSERT INTO job (kind, priority) VALUES ('export', 10)", []))
                .unwrap();
        }
    });
    // The one worker takes the job that outranks it next; it holds the
    // worker, so what the analyze job queued behind it can be seen waiting.
    let (up, export_up) = mpsc::channel();
    let up = Mutex::new(up);
    let (release, released) = mpsc::channel::<()>();
    let released = Mutex::new(released);
    let blocking = move |_: &JobContext| {
        up.lock().unwrap().send(()).ok();
        released
            .lock()
            .unwrap()
            .recv_timeout(PATIENCE)
            .map_err(|_| JobError::failed("never released"))
    };
    let queue = JobQueue::builder(library.writer.clone())
        .workers(1)
        // As the app registers it: the wrapper runs a job once more when it
        // asked for that while it ran.
        .handler(JobKind::Analyze, crate::scan::chain::after_analyze(handler))
        .handler(JobKind::Export, blocking)
        .start()
        .unwrap();
    let job = queue.enqueue(analyze_job()).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    export_up.recv_timeout(PATIENCE).unwrap();
    assert_eq!(taken.load(Ordering::SeqCst), 1, "it stopped after one file");
    assert_eq!(
        count(&library, "SELECT count(*) FROM file_estimate"),
        1,
        "and kept what it had done"
    );
    // Queued as the job ends, behind the one that holds the worker.
    let start = std::time::Instant::now();
    while queued(&library.writer, "analyze") == 0 {
        assert!(
            start.elapsed() < PATIENCE,
            "the rest was never queued again"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(queued(&library.writer, "analyze"), 1);
    release.send(()).unwrap();
    queue.shutdown();
}

#[test]
fn a_cancel_while_a_file_is_decoding_stops_it_and_stores_nothing_for_it() {
    let library = Library::new();
    library.put("a.wav", &wav(128.0, 60.0));
    let ids = walk_and_hash(&library);
    let (at_decode, decoding) = mpsc::channel();
    let (go, wait_for_go) = mpsc::channel::<()>();
    let (at_decode, wait_for_go) = (Mutex::new(at_decode), Mutex::new(wait_for_go));
    let handler = analyzer(&library).on_decode(move |_| {
        at_decode.lock().unwrap().send(()).unwrap();
        wait_for_go.lock().unwrap().recv().unwrap();
    });
    let queue = queue(&library, handler);
    let job = queue.enqueue(analyze_job()).unwrap();
    decoding.recv_timeout(PATIENCE).unwrap();
    queue.cancel(job).unwrap();
    go.send(()).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Cancelled);
    queue.shutdown();
    assert_eq!(stored(&library, ids["a.wav"]), None);
    assert_eq!(local_row(&library, track_of(&library, ids["a.wav"])), None);
    // Not recorded as unreadable either: the next run measures it.
    assert_eq!(analyze_all(&library), 1);
}
