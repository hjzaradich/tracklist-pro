//! The waveform job on a real (temp) music folder, after the real walk and
//! the real hash stage.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime};

use super::support::{wait, Library, TempVolume, PATIENCE};
use super::{audio, shaped, source, split};
use crate::db::DbError;
use crate::hash::{hash_job, Hasher};
use crate::jobs::{JobContext, JobError, JobId, JobKind, JobQueue, JobStatus, NewJob, Priority};
use crate::scan::walk::{scan_job, ScannedFile, Walker};
use crate::waveform::{
    peaks, request, request_first, waveform_job, waveform_job_for, waveform_of, Waveform,
    Wanted, Waveformer, PRIORITY, VERSION,
};

const RATE: u32 = 44_100;

fn wav(seconds: f64) -> Vec<u8> {
    audio::wav(&shaped(seconds, RATE))
}

/// A waveform handler over this library's volume.
fn waveformer(library: &Library) -> Waveformer<TempVolume> {
    let volume = library.volume.clone();
    Waveformer::new(move || volume.clone(), library.first.clone())
}

/// A queue with one worker that walks, hashes and draws waveforms.
fn queue(library: &Library, waveformer: Waveformer<TempVolume>) -> JobQueue {
    let (walk_volume, hash_volume) = (library.volume.clone(), library.volume.clone());
    JobQueue::builder(library.writer.clone())
        .workers(1)
        .handler(
            JobKind::Scan,
            Walker::new(move || walk_volume.clone(), |_: Vec<ScannedFile>| {}),
        )
        .handler(JobKind::Hash, Hasher::new(move || hash_volume.clone()))
        .handler(JobKind::Waveform, waveformer)
        .start()
        .unwrap()
}

/// Runs one waveform job to the end, returning how many files it decoded.
fn draw_all(library: &Library) -> usize {
    let taken = Arc::new(AtomicUsize::new(0));
    let count = taken.clone();
    let handler = waveformer(library).on_file(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    });
    let queue = queue(library, handler);
    let id = queue.enqueue(waveform_job()).unwrap();
    assert_eq!(wait(&library.writer, id).status, JobStatus::Done);
    queue.shutdown();
    taken.load(Ordering::SeqCst)
}

/// Runs the walk and the hash stage again, after files changed.
fn walk_and_hash_again(library: &Library) {
    let queue = queue(library, waveformer(library));
    for job in [scan_job(None), hash_job(None)] {
        let id = queue.enqueue(job).unwrap();
        assert_eq!(wait(&library.writer, id).status, JobStatus::Done);
    }
    queue.shutdown();
}

/// What the command would answer for file `id` now.
fn answer(library: &Library, id: i64) -> Waveform {
    library.writer.call(move |c| waveform_of(c, id)).unwrap()
}

fn ready(library: &Library, id: i64) -> (Vec<u8>, Vec<u8>, u32) {
    match answer(library, id) {
        Waveform::Ready {
            peaks,
            levels,
            duration_ms,
        } => (peaks, levels, duration_ms),
        other => panic!("file {id} is {other:?}"),
    }
}

fn stored_rows(library: &Library) -> i64 {
    library
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM file_waveform", [], |r| r.get(0)))
        .unwrap()
}

#[test]
fn peaks_are_computed_once_and_reused() {
    let library = Library::new();
    library.put("a.wav", &wav(8.0));
    library.put("Folder/b.wav", &wav(6.0));
    let ids = library.walk_and_hash();
    assert_eq!(draw_all(&library), 2);
    let before: Vec<_> = ids.values().map(|&id| ready(&library, id)).collect();
    assert_eq!(ready(&library, ids["a.wav"]).2, 8_000);
    assert_eq!(ready(&library, ids["Folder/b.wav"]).2, 6_000);
    // The loud second half is higher than the quiet first, in what is kept.
    let (kept, ..) = ready(&library, ids["a.wav"]);
    assert!(kept[10] < kept[kept.len() - 10]);

    // A second run, and a third after a fresh walk and hash: nothing is due,
    // so nothing is decoded.
    assert_eq!(draw_all(&library), 0);
    walk_and_hash_again(&library);
    assert_eq!(draw_all(&library), 0);
    let after: Vec<_> = ids.values().map(|&id| ready(&library, id)).collect();
    assert_eq!(before, after);
}

#[test]
fn peaks_are_computed_again_when_the_audio_changes() {
    let library = Library::new();
    let path = library.put("Swapped.wav", &wav(8.0));
    let ids = library.walk_and_hash();
    let id = ids["Swapped.wav"];
    assert_eq!(draw_all(&library), 1);
    let first = ready(&library, id);

    fs::write(&path, wav(9.0)).unwrap();
    walk_and_hash_again(&library);
    // The old peaks are of other audio: not ready until the new ones are.
    assert_eq!(answer(&library, id), Waveform::NotReady);
    assert_eq!(draw_all(&library), 1, "other audio is due");
    let second = ready(&library, id);
    assert_eq!(second.2, 9_000);
    assert_ne!(first, second);
    let row_hash: Vec<u8> = library
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT audio_hash FROM file_waveform WHERE file_id = ?1",
                [id],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(row_hash, library.audio_hash(id));
    assert_eq!(stored_rows(&library), 1, "the row was replaced");
}

#[test]
fn a_tag_rewrite_that_moves_size_and_mtime_but_not_the_audio_is_not_decoded_again() {
    let library = Library::new();
    let original = wav(8.0);
    let path = library.put("Tagged.wav", &original);
    let ids = library.walk_and_hash();
    assert_eq!(draw_all(&library), 1);
    let before = ready(&library, ids["Tagged.wav"]);

    // A LIST chunk before the audio, as a tagger writes: a new size, a new
    // modified time, the same audio.
    let mut tagged = original[..36].to_vec();
    tagged.extend_from_slice(b"LIST\x04\0\0\0INFO");
    tagged.extend_from_slice(&original[36..]);
    let riff = (tagged.len() as u32 - 8).to_le_bytes();
    tagged[4..8].copy_from_slice(&riff);
    fs::write(&path, &tagged).unwrap();
    walk_and_hash_again(&library);
    let size: i64 = library
        .writer
        .call(|c| c.query_row("SELECT size FROM file", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(size, tagged.len() as i64, "the walk saw the rewrite");

    assert_eq!(draw_all(&library), 0, "the audio is the same");
    assert_eq!(ready(&library, ids["Tagged.wav"]), before);
}

#[test]
fn a_new_modified_time_alone_does_not_make_a_file_due() {
    let library = Library::new();
    let path = library.put("Touched.wav", &wav(6.0));
    library.walk_and_hash();
    assert_eq!(draw_all(&library), 1);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(300))
        .unwrap();
    walk_and_hash_again(&library);
    assert_eq!(draw_all(&library), 0);
}

#[test]
fn a_new_method_version_makes_every_file_due_again() {
    let library = Library::new();
    library.put("a.wav", &wav(6.0));
    let ids = library.walk_and_hash();
    draw_all(&library);
    library
        .writer
        .call(|c| {
            c.execute(
                "UPDATE file_waveform SET method_version = method_version + 1",
                [],
            )
        })
        .unwrap();
    assert_eq!(answer(&library, ids["a.wav"]), Waveform::NotReady);
    assert_eq!(draw_all(&library), 1);
    let version: i64 = library
        .writer
        .call(|c| c.query_row("SELECT method_version FROM file_waveform", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(version, i64::from(VERSION));
}

#[test]
fn a_job_put_back_after_a_kill_draws_only_the_files_still_due() {
    let library = Library::new();
    for (i, name) in ["a.wav", "b.wav", "c.wav"].into_iter().enumerate() {
        library.put(name, &wav(6.0 + i as f64));
    }
    let ids = library.walk_and_hash();
    assert_eq!(draw_all(&library), 3);
    // The run died before it stored the second file.
    let lost = ids["b.wav"];
    library
        .writer
        .call(move |c| c.execute("DELETE FROM file_waveform WHERE file_id = ?1", [lost]))
        .unwrap();
    assert_eq!(draw_all(&library), 1);
    assert!(matches!(answer(&library, lost), Waveform::Ready { .. }));
    assert_eq!(stored_rows(&library), 3);
}

#[test]
fn a_file_the_job_has_not_reached_is_not_ready_and_so_is_one_that_is_not_known() {
    let library = Library::new();
    library.put("a.wav", &wav(6.0));
    let ids = library.walk_and_hash();
    assert_eq!(answer(&library, ids["a.wav"]), Waveform::NotReady);
    assert_eq!(answer(&library, 9_999), Waveform::NotReady);
    draw_all(&library);
    assert!(matches!(
        answer(&library, ids["a.wav"]),
        Waveform::Ready { .. }
    ));
}

#[test]
fn a_file_that_cannot_be_decoded_gets_a_reason_and_is_not_tried_again() {
    let library = Library::new();
    // An Opus file: it has audio, which Symphonia has no decoder for.
    library.put("Opus.opus", &crate::tags::test_audio::ogg_opus());
    library.put("Good.wav", &wav(6.0));
    let ids = library.walk_and_hash();
    assert_eq!(draw_all(&library), 2);
    assert_eq!(answer(&library, ids["Opus.opus"]), Waveform::Unavailable);
    assert!(matches!(
        answer(&library, ids["Good.wav"]),
        Waveform::Ready { .. }
    ));
    let failure: String = library
        .writer
        .call(|c| {
            c.query_row(
                "SELECT failure FROM file_waveform WHERE failure IS NOT NULL",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(failure, "unsupported_codec");
    assert_eq!(draw_all(&library), 0);
}

#[test]
fn files_of_every_format_the_decoder_supports_are_drawn() {
    let library = Library::new();
    let pcm = shaped(6.0, RATE);
    library.put("a.wav", &audio::wav(&pcm));
    library.put("b.aiff", &audio::aiff(&pcm));
    library.put("c.flac", &audio::flac(&pcm));
    library.put("d.m4a", &audio::m4a_alac(&pcm));
    #[cfg(windows)]
    library.put("e.mp3", &audio::mp3(&pcm, 192));
    let ids = library.walk_and_hash();
    assert_eq!(draw_all(&library), ids.len());
    let wav = ready(&library, ids["a.wav"]);
    for name in ["b.aiff", "c.flac", "d.m4a"] {
        assert_eq!(ready(&library, ids[name]), wav, "{name}");
    }
    #[cfg(windows)]
    {
        let (_, _, ms) = ready(&library, ids["e.mp3"]);
        assert!(ms.abs_diff(6_000) <= 100, "{ms} ms");
    }
}

#[test]
fn a_file_on_an_unplugged_drive_is_not_due_and_is_drawn_when_it_is_back() {
    let library = Library::new();
    library.put("a.wav", &wav(6.0));
    let ids = library.walk_and_hash();
    library.volume.set_online(false);
    assert_eq!(draw_all(&library), 0, "not taken: nothing can be done");
    assert_eq!(stored_rows(&library), 0);
    library.volume.set_online(true);
    assert_eq!(draw_all(&library), 1);
    assert!(matches!(
        answer(&library, ids["a.wav"]),
        Waveform::Ready { .. }
    ));
}

#[test]
fn a_file_changed_since_the_walk_is_left_for_the_next_walk() {
    let library = Library::new();
    let path = library.put("a.wav", &wav(6.0));
    library.walk_and_hash();
    fs::write(&path, wav(7.0)).unwrap();
    assert_eq!(draw_all(&library), 1);
    assert_eq!(
        stored_rows(&library),
        0,
        "its row still describes the old file"
    );
    walk_and_hash_again(&library);
    assert_eq!(draw_all(&library), 1);
    assert_eq!(stored_rows(&library), 1);
}

#[test]
fn a_file_without_an_audio_hash_is_not_drawn() {
    let library = Library::new();
    library.put("Garbage.flac", &vec![0x5A; 50_000]);
    library.put("Empty.mp3", b"");
    library.walk_and_hash();
    assert_eq!(draw_all(&library), 0);
    assert_eq!(stored_rows(&library), 0);
}

#[test]
fn the_job_makes_way_for_a_waiting_job_of_higher_priority_between_two_files() {
    let library = Library::new();
    for (i, name) in ["a.wav", "b.wav", "c.wav"].into_iter().enumerate() {
        library.put(name, &wav(6.0 + i as f64));
    }
    library.walk_and_hash();
    let taken = Arc::new(AtomicUsize::new(0));
    let (count, writer) = (taken.clone(), library.writer.clone());
    // While the first file is taken, a job the user asked for is queued
    // (the single worker is busy, so it waits).
    let handler = waveformer(&library).on_file(move |_| {
        if count.fetch_add(1, Ordering::SeqCst) == 0 {
            writer
                .call(|c| c.execute("INSERT INTO job (kind, priority) VALUES ('export', 10)", []))
                .unwrap();
        }
    });
    let queue = queue(&library, handler);
    let id = queue.enqueue(waveform_job()).unwrap();
    assert_eq!(wait(&library.writer, id).status, JobStatus::Done);
    queue.shutdown();
    assert_eq!(taken.load(Ordering::SeqCst), 1, "it stopped after one file");
    assert_eq!(stored_rows(&library), 1, "and kept what it had done");
}

#[test]
fn the_job_is_the_lowest_priority_there_is() {
    assert_eq!(waveform_job().priority, PRIORITY);
    assert_eq!(waveform_job().kind, JobKind::Waveform);
    assert!(PRIORITY < crate::file_roles::PRIORITY);
    assert!(PRIORITY < crate::matching::PRIORITY);
    assert!(PRIORITY < crate::jobs::Priority::BACKGROUND);
}

#[test]
fn a_waveform_is_kept_in_the_database_and_nothing_is_written_beside_the_music() {
    let library = Library::new();
    library.put("a.wav", &wav(12.0));
    library.put("Folder/b.wav", &wav(30.0));
    library.walk_and_hash();
    let before = listing(&library.music);
    draw_all(&library);
    assert_eq!(listing(&library.music), before);
    // At most 1024 buckets of two bytes.
    let longest: i64 = library
        .writer
        .call(|c| {
            c.query_row("SELECT max(length(peaks)) FROM file_waveform", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_eq!(longest, 2 * peaks::BUCKETS as i64);
}

/// Every file under `dir` with its size and modified time.
fn listing(dir: &Path) -> BTreeMap<String, (u64, SystemTime)> {
    let mut found = BTreeMap::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let entry = entry.unwrap();
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                folders.push(entry.path());
            } else {
                found.insert(
                    entry.path().display().to_string(),
                    (meta.len(), meta.modified().unwrap()),
                );
            }
        }
    }
    found
}

/// Queues by storing the row, as the queue does.
fn store_row(library: &Library) -> impl Fn(crate::jobs::NewJob) -> Result<JobId, DbError> + '_ {
    |job| {
        library.writer.call(move |c| {
            c.execute(
                "INSERT INTO job (kind, target, priority) VALUES (?1, ?2, ?3)",
                (
                    job.kind.as_str(),
                    job.target.map(|t| t.to_string()),
                    job.priority.0,
                ),
            )?;
            Ok(JobId(c.last_insert_rowid()))
        })
    }
}

#[test]
fn a_job_is_asked_for_only_while_a_file_is_due_and_never_twice() {
    let library = Library::new();
    let queued = |library: &Library| -> i64 {
        library
            .writer
            .call(|c| {
                c.query_row(
                    "SELECT count(*) FROM job WHERE kind = 'waveform' AND status = 'queued'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap()
    };
    // Nothing found yet: nothing to ask for.
    request(&library.writer, None, store_row(&library)).unwrap();
    assert_eq!(queued(&library), 0);

    library.put("a.wav", &wav(6.0));
    library.walk_and_hash();
    request(&library.writer, None, store_row(&library)).unwrap();
    request(&library.writer, None, store_row(&library)).unwrap();
    assert_eq!(queued(&library), 1);

    // Once drawn, nothing is owed.
    library
        .writer
        .call(|c| c.execute("DELETE FROM job", []))
        .unwrap();
    draw_all(&library);
    library
        .writer
        .call(|c| c.execute("DELETE FROM job", []))
        .unwrap();
    request(&library.writer, None, store_row(&library)).unwrap();
    assert_eq!(queued(&library), 0);
}

/// Prints what drawing a six-minute track costs, for the pull request:
/// `cargo test measures_ -- --ignored --nocapture`. There is no timing
/// assertion; the numbers are for people.
#[test]
#[ignore = "prints a measurement"]
fn measures_the_time_and_bytes_a_six_minute_track_takes() {
    let pcm = shaped(360.0, RATE);
    let mut cases = vec![("WAV", audio::wav(&pcm))];
    #[cfg(windows)]
    cases.push(("MP3 192 kbps", audio::mp3(&pcm, 192)));
    for (format, bytes) in cases {
        let started = std::time::Instant::now();
        let computed = peaks::compute(source(&bytes), &mut |_| true).unwrap();
        let took = started.elapsed();
        let (peaks, _) = split(&computed.peaks);
        eprintln!(
            "six minutes of {format} ({} MiB): decoded and reduced in {took:?}; {} bytes stored ({} buckets)",
            bytes.len() >> 20,
            computed.peaks.len(),
            peaks.len(),
        );
    }
}

/// Runs one waveform job to the end, returning how many files it took and
/// how many it decoded (the rest copied the waveform of the same audio).
fn draw_counting(library: &Library) -> (usize, usize) {
    let (taken, decoded) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (t, d) = (taken.clone(), decoded.clone());
    let handler = waveformer(library)
        .on_file(move |_| {
            t.fetch_add(1, Ordering::SeqCst);
        })
        .on_decode(move |_| {
            d.fetch_add(1, Ordering::SeqCst);
        });
    let queue = queue(library, handler);
    let id = queue.enqueue(waveform_job()).unwrap();
    assert_eq!(wait(&library.writer, id).status, JobStatus::Done);
    queue.shutdown();
    (taken.load(Ordering::SeqCst), decoded.load(Ordering::SeqCst))
}

#[test]
fn files_that_hold_the_very_same_audio_are_decoded_once_and_share_its_waveform() {
    let library = Library::new();
    let tune = wav(8.0);
    library.put("a.wav", &tune);
    library.put("Copies/b.wav", &tune);
    library.put("Copies/Deeper/c.wav", &tune);
    library.put("Other.wav", &wav(9.0));
    let ids = library.walk_and_hash();
    assert_eq!(
        library.audio_hash(ids["a.wav"]),
        library.audio_hash(ids["Copies/b.wav"])
    );
    assert_eq!(draw_counting(&library), (4, 2), "four taken, two decoded");
    let first = ready(&library, ids["a.wav"]);
    assert_eq!(ready(&library, ids["Copies/b.wav"]), first);
    assert_eq!(ready(&library, ids["Copies/Deeper/c.wav"]), first);
    assert_eq!(ready(&library, ids["Other.wav"]).2, 9_000);
    assert_eq!(stored_rows(&library), 4);
    // Nothing is due after that.
    assert_eq!(draw_counting(&library), (0, 0));
}

#[test]
fn a_copy_of_a_file_that_cannot_be_decoded_shares_the_reason_without_another_try() {
    let library = Library::new();
    let opus = crate::tags::test_audio::ogg_opus();
    library.put("One.opus", &opus);
    library.put("Two.opus", &opus);
    let ids = library.walk_and_hash();
    assert_eq!(draw_counting(&library), (2, 1));
    for name in ["One.opus", "Two.opus"] {
        assert_eq!(answer(&library, ids[name]), Waveform::Unavailable, "{name}");
    }
}

#[test]
fn a_file_with_other_audio_than_its_neighbours_is_not_given_their_waveform() {
    let library = Library::new();
    library.put("a.wav", &wav(8.0));
    library.put("b.wav", &wav(9.0));
    let ids = library.walk_and_hash();
    assert_eq!(draw_counting(&library), (2, 2));
    assert_ne!(ready(&library, ids["a.wav"]), ready(&library, ids["b.wav"]));
}

/// A queue of three workers that runs waveform jobs and one other kind of
/// job, `Export` (a higher priority than the waveforms), whose handler is
/// `export`.
fn queue_with_a_blocking_job(
    library: &Library,
    waveformer: Waveformer<TempVolume>,
    export: impl Fn(&JobContext) -> Result<(), JobError> + Send + Sync + 'static,
) -> JobQueue {
    JobQueue::builder(library.writer.clone())
        .workers(3)
        .handler(JobKind::Waveform, waveformer)
        .handler(JobKind::Export, export)
        .start()
        .unwrap()
}

/// A job that is running until `release` is sent on, and says so on `up`.
fn blocking_job(
    up: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
) -> impl Fn(&JobContext) -> Result<(), JobError> + Send + Sync + 'static {
    let release = Mutex::new(release);
    move |_: &JobContext| {
        up.send(()).ok();
        let release = release.lock().unwrap();
        release
            .recv_timeout(PATIENCE)
            .map_err(|_| JobError::failed("never released"))
    }
}

fn blocking_export() -> NewJob {
    NewJob::new(JobKind::Export).priority(Priority::USER)
}

#[test]
fn the_job_gives_its_thread_back_and_waits_while_a_job_of_higher_priority_is_running() {
    let library = Library::new();
    for (i, name) in ["a.wav", "b.wav", "c.wav"].into_iter().enumerate() {
        library.put(name, &wav(6.0 + i as f64));
    }
    library.walk_and_hash();

    let taken = Arc::new(AtomicUsize::new(0));
    let (count, first_up) = (taken.clone(), library.first.clone());
    // Taking the first file: the thread is held. Hold the job there until
    // the test has started the other job.
    let (holding, held) = mpsc::channel::<usize>();
    let (go, going) = mpsc::channel::<()>();
    let going = Mutex::new(going);
    let (paused, pause_seen) = mpsc::channel::<()>();
    let handler = waveformer(&library)
        .budget(1)
        .on_file(move |_| {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                holding.send(first_up.busy()).unwrap();
                going.lock().unwrap().recv_timeout(PATIENCE).unwrap();
            }
        })
        .on_pause(move || {
            paused.send(()).ok();
        });
    let (up, export_up) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let queue = queue_with_a_blocking_job(&library, handler, blocking_job(up, released));

    let draw = queue.enqueue(waveform_job()).unwrap();
    assert_eq!(
        held.recv_timeout(PATIENCE).unwrap(),
        1,
        "it holds the thread"
    );
    // A job that outranks it starts running (a fingerprint run an idle
    // worker took at once looks like this).
    let export = queue.enqueue(blocking_export()).unwrap();
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
    assert_eq!(stored_rows(&library), 1);

    // The other job ends: the waveforms carry on.
    release.send(()).unwrap();
    assert_eq!(wait(&library.writer, export).status, JobStatus::Done);
    assert_eq!(wait(&library.writer, draw).status, JobStatus::Done);
    queue.shutdown();
    assert_eq!(taken.load(Ordering::SeqCst), 3);
    assert_eq!(stored_rows(&library), 3);
    assert_eq!(library.first.busy(), 0);
}

#[test]
fn a_file_asked_for_is_drawn_at_once_even_while_the_other_waveforms_wait_behind_a_running_job() {
    let library = Library::new();
    for (i, name) in ["a.wav", "b.wav", "c.wav"].into_iter().enumerate() {
        library.put(name, &wav(6.0 + i as f64));
    }
    let ids = library.walk_and_hash();

    let (paused, pause_seen) = mpsc::channel::<()>();
    let handler = waveformer(&library).on_pause(move || {
        paused.send(()).ok();
    });
    let wanted = handler.wanted_handle();
    let (up, export_up) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let queue = queue_with_a_blocking_job(&library, handler, blocking_job(up, released));
    let export = queue.enqueue(blocking_export()).unwrap();
    export_up.recv_timeout(PATIENCE).unwrap();
    let all = queue.enqueue(waveform_job()).unwrap();
    pause_seen.recv_timeout(PATIENCE).unwrap();
    assert_eq!(stored_rows(&library), 0, "the waveforms wait");

    // The player loads the last file: it is asked for.
    request_first(&library.writer, &wanted, ids["c.wav"], |job| {
        queue.enqueue(job)
    })
    .unwrap();
    let asked = JobId(
        library
            .writer
            .call(|c| {
                c.query_row("SELECT max(id) FROM job WHERE kind = 'waveform'", [], |r| {
                    r.get(0)
                })
            })
            .unwrap(),
    );
    assert_eq!(wait(&library.writer, asked).status, JobStatus::Done);
    assert!(matches!(
        answer(&library, ids["c.wav"]),
        Waveform::Ready { .. }
    ));
    assert_eq!(answer(&library, ids["a.wav"]), Waveform::NotReady);
    assert_eq!(answer(&library, ids["b.wav"]), Waveform::NotReady);

    release.send(()).unwrap();
    assert_eq!(wait(&library.writer, export).status, JobStatus::Done);
    assert_eq!(wait(&library.writer, all).status, JobStatus::Done);
    queue.shutdown();
    assert_eq!(stored_rows(&library), 3);
}

#[test]
fn a_file_is_asked_for_only_while_it_is_due_and_the_job_is_for_it_alone_at_user_priority() {
    let library = Library::new();
    library.put("a.wav", &wav(6.0));
    let ids = library.walk_and_hash();
    let id = ids["a.wav"];
    let wanted = Wanted::default();
    let queued = |library: &Library| -> Vec<(i64, Option<String>)> {
        library
            .writer
            .call(|c| {
                let mut s = c.prepare(
                    "SELECT priority, target FROM job WHERE kind = 'waveform' AND status = 'queued'",
                )?;
                let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .unwrap()
    };
    // A file nobody knows, then the file twice: one job, for it alone.
    request_first(&library.writer, &wanted, 9_999, store_row(&library)).unwrap();
    assert_eq!(queued(&library), []);
    request_first(&library.writer, &wanted, id, store_row(&library)).unwrap();
    request_first(&library.writer, &wanted, id, store_row(&library)).unwrap();
    let jobs = queued(&library);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].0, Priority::USER.0);
    let target: serde_json::Value = serde_json::from_str(jobs[0].1.as_deref().unwrap()).unwrap();
    assert_eq!(target, waveform_job_for(vec![id]).target.unwrap());

    // Once it is drawn there is nothing to ask for.
    library
        .writer
        .call(|c| c.execute("DELETE FROM job", []))
        .unwrap();
    draw_all(&library);
    request_first(&library.writer, &wanted, id, store_row(&library)).unwrap();
    assert_eq!(queued(&library), []);
}

/// What a test sees of the decodes: how many run at once (the most there
/// ever were), and which files were decoded, in order. The first decode,
/// of `hold`, stays in progress until `release` is sent on.
struct Decodes {
    active: Arc<AtomicUsize>,
    most: Arc<AtomicUsize>,
    decoded: Arc<Mutex<Vec<i64>>>,
}

fn watching_decodes(
    handler: Waveformer<TempVolume>,
    hold: i64,
    started: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    waiting: mpsc::Sender<i64>,
) -> (Waveformer<TempVolume>, Decodes) {
    let seen = Decodes {
        active: Arc::new(AtomicUsize::new(0)),
        most: Arc::new(AtomicUsize::new(0)),
        decoded: Arc::new(Mutex::new(Vec::new())),
    };
    let (active, most, decoded) = (seen.active.clone(), seen.most.clone(), seen.decoded.clone());
    let release = Mutex::new(release);
    let begin = move |id: i64| {
        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
        most.fetch_max(now, Ordering::SeqCst);
        decoded.lock().unwrap().push(id);
        if id == hold {
            started.send(()).unwrap();
            release.lock().unwrap().recv_timeout(PATIENCE).unwrap();
        }
    };
    let end = seen.active.clone();
    let handler = handler
        .on_decode(begin)
        .on_decoded(move |_| {
            end.fetch_sub(1, Ordering::SeqCst);
        })
        .on_decode_wait(move |id| {
            waiting.send(id).ok();
        });
    (handler, seen)
}

fn waveform_jobs(library: &Library) -> Vec<JobId> {
    library
        .writer
        .call(|c| {
            let mut s = c.prepare("SELECT id FROM job WHERE kind = 'waveform' ORDER BY id")?;
            let rows = s.query_map([], |r| Ok(JobId(r.get(0)?)))?;
            rows.collect()
        })
        .unwrap()
}

#[test]
fn a_file_asked_for_never_decodes_beside_the_background_run() {
    let library = Library::new();
    library.put("a.wav", &wav(6.0));
    library.put("c.wav", &wav(7.0));
    let ids = library.walk_and_hash();
    // The background run takes the lower id first.
    let (a, c) = (ids["a.wav"].min(ids["c.wav"]), ids["a.wav"].max(ids["c.wav"]));

    let (started, started_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let (waiting, waiting_rx) = mpsc::channel();
    let (handler, seen) =
        watching_decodes(waveformer(&library), a, started, release_rx, waiting);
    let wanted = handler.wanted_handle();
    let queue = JobQueue::builder(library.writer.clone())
        .workers(2)
        .handler(JobKind::Waveform, handler)
        .start()
        .unwrap();

    // The background run is in the middle of decoding the first file…
    let background = queue.enqueue(waveform_job()).unwrap();
    started_rx.recv_timeout(PATIENCE).unwrap();
    // …when the player asks for the other. It has to wait for its turn.
    request_first(&library.writer, &wanted, c, |job| queue.enqueue(job)).unwrap();
    assert_eq!(waiting_rx.recv_timeout(PATIENCE).unwrap(), c);
    assert_eq!(*seen.decoded.lock().unwrap(), [a], "nothing else is decoding");
    assert_eq!(seen.active.load(Ordering::SeqCst), 1);

    release.send(()).unwrap();
    assert_eq!(wait(&library.writer, background).status, JobStatus::Done);
    for job in waveform_jobs(&library) {
        assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    }
    queue.shutdown();
    assert_eq!(seen.most.load(Ordering::SeqCst), 1, "never two at once");
    let mut decoded = seen.decoded.lock().unwrap().clone();
    decoded.sort_unstable();
    assert_eq!(decoded, [a, c], "each decoded once");
    assert!(matches!(answer(&library, a), Waveform::Ready { .. }));
    assert!(matches!(answer(&library, c), Waveform::Ready { .. }));
}

#[test]
fn requests_for_files_the_player_skipped_past_drop_out_and_do_not_pile_up() {
    let library = Library::new();
    for (i, name) in ["a.wav", "b.wav", "c.wav", "d.wav"].into_iter().enumerate() {
        library.put(name, &wav(6.0 + i as f64));
    }
    let ids = library.walk_and_hash();
    let (a, b, c, d) = (ids["a.wav"], ids["b.wav"], ids["c.wav"], ids["d.wav"]);

    let (started, started_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let (waiting, _waiting_rx) = mpsc::channel();
    let (handler, seen) =
        watching_decodes(waveformer(&library), a, started, release_rx, waiting);
    let wanted = handler.wanted_handle();
    let queue = JobQueue::builder(library.writer.clone())
        .workers(4)
        .handler(JobKind::Waveform, handler)
        .start()
        .unwrap();

    // The first file is being decoded; the player skips on through three more.
    request_first(&library.writer, &wanted, a, |job| queue.enqueue(job)).unwrap();
    started_rx.recv_timeout(PATIENCE).unwrap();
    for file in [b, c, d] {
        request_first(&library.writer, &wanted, file, |job| queue.enqueue(job)).unwrap();
    }

    release.send(()).unwrap();
    let jobs = waveform_jobs(&library);
    assert_eq!(jobs.len(), 4);
    for job in jobs {
        assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    }
    queue.shutdown();
    // The ones skipped past were never decoded: the one in progress and the
    // one the player stopped on were.
    assert_eq!(*seen.decoded.lock().unwrap(), [a, d]);
    assert_eq!(seen.most.load(Ordering::SeqCst), 1);
    assert!(matches!(answer(&library, d), Waveform::Ready { .. }));
    assert_eq!(answer(&library, b), Waveform::NotReady);
    assert_eq!(answer(&library, c), Waveform::NotReady);
}
