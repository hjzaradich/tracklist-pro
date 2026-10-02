//! The quality job on a real (temp) music folder, after the real walk and
//! the real hash stage.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use super::support::{wait, Library};
use super::{audio, low_passed, noise, pcm, wav};
use crate::hash::hash_job;
use crate::jobs::{JobKind, JobStatus, Priority};
use crate::quality::{quality_job, request, VERSION};
use crate::scan::walk::scan_job;

const RATE: u32 = 44_100;

/// A mono WAV of noise low-passed at `hz`, `seconds` long.
fn lowpassed_wav(seed: u64, seconds: f64, hz: f64) -> Vec<u8> {
    wav(
        &low_passed(&noise(seed, seconds, RATE, 0.1), RATE, hz),
        RATE,
    )
}

fn full_band_wav(seed: u64, seconds: f64) -> Vec<u8> {
    wav(&noise(seed, seconds, RATE, 0.1), RATE)
}

/// Counts the files a qualifier takes.
fn counting(
    library: &Library,
) -> (
    crate::quality::Qualifier<super::support::TempVolume>,
    Arc<AtomicUsize>,
) {
    let taken = Arc::new(AtomicUsize::new(0));
    let count = taken.clone();
    let qualifier = library.qualifier().on_file(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    });
    (qualifier, taken)
}

/// Runs one quality job over every due file to the end, returning how many
/// files it took.
fn measure_all(library: &Library) -> usize {
    let (qualifier, taken) = counting(library);
    let queue = library.queue(qualifier);
    let id = queue.enqueue(quality_job(None)).unwrap();
    assert_eq!(wait(&library.writer, id).status, JobStatus::Done);
    queue.shutdown();
    taken.load(Ordering::SeqCst)
}

/// Runs the walk and the hash stage again, after files changed.
fn walk_and_hash_again(library: &Library) {
    let queue = library.queue(library.qualifier());
    for job in [scan_job(None), hash_job(None)] {
        let id = queue.enqueue(job).unwrap();
        assert_eq!(wait(&library.writer, id).status, JobStatus::Done);
    }
    queue.shutdown();
}

#[test]
fn every_present_file_is_measured_and_its_row_names_the_audio_it_came_from() {
    let library = Library::new();
    library.put("Artist/Low.wav", &lowpassed_wav(1, 12.0, 16_000.0));
    library.put("Artist/Full.wav", &full_band_wav(2, 12.0));
    library.put(
        "Three.flac",
        &audio::flac(&pcm(
            &low_passed(&noise(3, 12.0, RATE, 0.1), RATE, 12_000.0),
            RATE,
        )),
    );
    let ids = library.walk_and_hash();
    assert_eq!(measure_all(&library), 3);
    for (path, id) in &ids {
        let row = library
            .stored(*id)
            .unwrap_or_else(|| panic!("{path} has none"));
        assert_eq!(row.audio_hash, library.audio_hash(*id), "{path}");
        assert_eq!(row.method_version, i64::from(VERSION), "{path}");
        assert_eq!(row.failure, None, "{path}");
        assert_eq!(row.ended.as_deref(), Some("complete"), "{path}");
        assert_eq!(row.decode_errors, 0, "{path}");
        assert_eq!(row.decoded_ms, Some(12_000), "{path}");
        assert_eq!(row.header_ms, Some(12_000), "{path}");
    }
    let cutoff = |path: &str| library.stored(ids[path]).unwrap().cutoff_hz.unwrap();
    assert!((15_300..=16_700).contains(&cutoff("Artist/Low.wav")));
    assert!(cutoff("Artist/Full.wav") >= 21_500);
    assert!((11_300..=12_700).contains(&cutoff("Three.flac")));
}

#[test]
fn a_silent_or_too_short_file_is_stored_with_the_reason_it_has_no_cutoff() {
    let library = Library::new();
    library.put("Silence.wav", &wav(&vec![0.0; RATE as usize * 12], RATE));
    library.put("Blip.wav", &full_band_wav(4, 2.0));
    let ids = library.walk_and_hash();
    measure_all(&library);
    let silent = library.stored(ids["Silence.wav"]).unwrap();
    assert_eq!(
        (silent.cutoff_hz, silent.cutoff_gap.as_deref()),
        (None, Some("silent"))
    );
    let short = library.stored(ids["Blip.wav"]).unwrap();
    assert_eq!(
        (short.cutoff_hz, short.cutoff_gap.as_deref()),
        (None, Some("too_short"))
    );
    // They decoded fine: the durations are there.
    assert_eq!(short.decoded_ms, Some(2_000));
}

#[test]
fn unchanged_audio_is_not_measured_twice() {
    let library = Library::new();
    library.put("a.wav", &full_band_wav(5, 12.0));
    library.put("b.wav", &lowpassed_wav(6, 12.0, 14_000.0));
    let ids = library.walk_and_hash();
    assert_eq!(measure_all(&library), 2);
    let before: Vec<_> = ids.values().map(|id| library.stored(*id)).collect();
    // A second run, and a third after a fresh walk and hash: nothing is due.
    assert_eq!(measure_all(&library), 0);
    walk_and_hash_again(&library);
    assert_eq!(measure_all(&library), 0);
    let after: Vec<_> = ids.values().map(|id| library.stored(*id)).collect();
    assert_eq!(before, after);
}

#[test]
fn a_tag_rewrite_that_moves_size_and_mtime_but_not_the_audio_is_not_measured_again() {
    let library = Library::new();
    let original = full_band_wav(7, 12.0);
    let path = library.put("Tagged.wav", &original);
    let ids = library.walk_and_hash();
    assert_eq!(measure_all(&library), 1);
    let before = library.stored(ids["Tagged.wav"]).unwrap();

    // A LIST chunk before the audio, as a tagger writes: a new size, a new
    // modified time, the same audio.
    let list = b"LIST\x04\0\0\0INFO";
    let mut tagged = original[..36].to_vec();
    tagged.extend_from_slice(list);
    tagged.extend_from_slice(&original[36..]);
    let riff = (tagged.len() as u32 - 8).to_le_bytes();
    tagged[4..8].copy_from_slice(&riff);
    fs::write(&path, &tagged).unwrap();
    walk_and_hash_again(&library);
    let file_size: i64 = library
        .writer
        .call(|c| c.query_row("SELECT size FROM file", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(file_size, tagged.len() as i64, "the walk saw the rewrite");

    assert_eq!(measure_all(&library), 0, "the audio is the same");
    assert_eq!(library.stored(ids["Tagged.wav"]).unwrap(), before);
}

#[test]
fn a_new_modified_time_alone_does_not_make_a_file_due() {
    let library = Library::new();
    let path = library.put("Touched.wav", &full_band_wav(8, 12.0));
    library.walk_and_hash();
    assert_eq!(measure_all(&library), 1);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(300))
        .unwrap();
    walk_and_hash_again(&library);
    assert_eq!(measure_all(&library), 0);
}

#[test]
fn changed_audio_is_measured_again() {
    let library = Library::new();
    let path = library.put("Swapped.wav", &full_band_wav(9, 12.0));
    let ids = library.walk_and_hash();
    assert_eq!(measure_all(&library), 1);
    let id = ids["Swapped.wav"];
    let first = library.stored(id).unwrap();
    assert!(first.cutoff_hz.unwrap() >= 21_500);

    fs::write(&path, lowpassed_wav(10, 12.0, 12_000.0)).unwrap();
    walk_and_hash_again(&library);
    assert_eq!(measure_all(&library), 1, "other audio is due");
    let second = library.stored(id).unwrap();
    assert_ne!(second.audio_hash, first.audio_hash);
    assert_eq!(second.audio_hash, library.audio_hash(id));
    assert!((11_300..=12_700).contains(&second.cutoff_hz.unwrap()));
    assert_eq!(
        library.measured_files(),
        1,
        "the row was replaced, not added to"
    );
}

#[test]
fn a_new_method_version_makes_every_file_due_again() {
    let library = Library::new();
    library.put("a.wav", &full_band_wav(11, 12.0));
    let ids = library.walk_and_hash();
    measure_all(&library);
    library
        .writer
        .call(|c| {
            c.execute(
                "UPDATE file_quality SET method_version = method_version + 1",
                [],
            )
        })
        .unwrap();
    assert_eq!(measure_all(&library), 1);
    assert_eq!(
        library.stored(ids["a.wav"]).unwrap().method_version,
        i64::from(VERSION)
    );
}

#[test]
fn a_file_whose_hash_stage_has_not_run_is_not_measured_yet() {
    let library = Library::new();
    library.put("a.wav", &full_band_wav(12, 12.0));
    // Walked only: no audio hash to record the measurement against.
    let queue = library.queue(library.qualifier());
    let id = queue.enqueue(scan_job(None)).unwrap();
    wait(&library.writer, id);
    queue.shutdown();
    assert_eq!(measure_all(&library), 0);
    assert_eq!(library.measured_files(), 0);
}

#[test]
fn a_cancelled_job_leaves_no_partial_row() {
    let library = Library::new();
    // 60 s of 22 kHz FLAC is about 320 packets.
    library.put(
        "Long.flac",
        &audio::flac(&pcm(&noise(13, 60.0, 22_050, 0.1), 22_050)),
    );
    library.walk_and_hash();
    let (at_packet, packet) = mpsc::channel();
    let (go, wait_for_go) = mpsc::channel::<()>();
    let (at_packet, wait_for_go) = (Mutex::new(at_packet), Mutex::new(wait_for_go));
    let packets = Arc::new(Mutex::new(0u64));
    let count = packets.clone();
    let queue = library.queue(library.qualifier().on_packet(move |n| {
        *count.lock().unwrap() = n;
        if n == 50 {
            at_packet.lock().unwrap().send(()).unwrap();
            wait_for_go.lock().unwrap().recv().unwrap();
        }
    }));
    let job = queue.enqueue(quality_job(None)).unwrap();
    packet.recv().unwrap();
    queue.cancel(job).unwrap();
    go.send(()).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Cancelled);
    queue.shutdown();
    assert!(
        *packets.lock().unwrap() <= 51,
        "it stopped at the next packet"
    );
    assert_eq!(
        library.measured_files(),
        0,
        "nothing for the half-read file"
    );
}

#[test]
fn a_cancel_keeps_the_files_already_measured_and_the_rest_stay_due() {
    let library = Library::new();
    library.put("a.wav", &full_band_wav(14, 12.0));
    library.put(
        "b.flac",
        &audio::flac(&pcm(&noise(15, 30.0, 22_050, 0.1), 22_050)),
    );
    let ids = library.walk_and_hash();
    let (at_packet, packet) = mpsc::channel();
    let (go, wait_for_go) = mpsc::channel::<()>();
    let (at_packet, wait_for_go) = (Mutex::new(at_packet), Mutex::new(wait_for_go));
    let current = Arc::new(Mutex::new(0));
    let (set, get) = (current.clone(), current);
    let flac = ids["b.flac"];
    let queue = library.queue(
        library
            .qualifier()
            .on_file(move |id| *set.lock().unwrap() = id)
            .on_packet(move |n| {
                if n == 20 && *get.lock().unwrap() == flac {
                    at_packet.lock().unwrap().send(()).unwrap();
                    wait_for_go.lock().unwrap().recv().unwrap();
                }
            }),
    );
    let job = queue.enqueue(quality_job(None)).unwrap();
    packet.recv().unwrap();
    queue.cancel(job).unwrap();
    go.send(()).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Cancelled);
    queue.shutdown();
    // Files are taken lowest id first: the WAV was finished before the FLAC.
    assert!(library.stored(ids["a.wav"]).is_some());
    assert!(library.stored(flac).is_none());
    assert_eq!(measure_all(&library), 1, "only the cancelled one is due");
}

#[cfg(windows)]
#[test]
fn a_corrupt_frame_is_counted_as_a_decode_error_and_does_not_stop_the_job() {
    let library = Library::new();
    let clean = audio::mp3(&pcm(&noise(16, 30.0, RATE, 0.1), RATE), 192);
    let mut damaged = clean.clone();
    for at in (1000..damaged.len()).step_by(997) {
        damaged[at] ^= 0x81;
    }
    library.put("a.mp3", &clean);
    library.put("b.mp3", &damaged);
    library.put("c.wav", &full_band_wav(17, 12.0));
    let ids = library.walk_and_hash();
    assert_eq!(measure_all(&library), 3);
    let good = library.stored(ids["a.mp3"]).unwrap();
    let bad = library.stored(ids["b.mp3"]).unwrap();
    assert_eq!(good.decode_errors, 0);
    assert_eq!(good.error_kinds, None);
    assert!(bad.decode_errors >= 1, "{bad:?}");
    let kinds: serde_json::Value =
        serde_json::from_str(bad.error_kinds.as_deref().unwrap()).unwrap();
    assert!(kinds["decode"].as_i64().unwrap() >= 1, "{kinds}");
    // The damaged file was still measured, and the one after it too.
    assert!(bad.cutoff_hz.is_some());
    assert!(bad.decoded_ms.unwrap() < bad.header_ms.unwrap());
    assert!(library.stored(ids["c.wav"]).is_some());
}

#[test]
fn a_file_cut_short_is_stored_with_a_decoded_duration_shorter_than_its_header() {
    let library = Library::new();
    let whole = audio::flac(&pcm(&noise(18, 20.0, RATE, 0.1), RATE));
    library.put("Cut.flac", &whole[..whole.len() * 2 / 5]);
    let ids = library.walk_and_hash();
    measure_all(&library);
    let row = library.stored(ids["Cut.flac"]).unwrap();
    assert_eq!(row.header_ms, Some(20_000));
    let decoded = row.decoded_ms.unwrap();
    assert!((7_000..=9_000).contains(&decoded), "{row:?}");
}

#[test]
fn a_file_the_hash_stage_could_not_find_audio_in_is_not_measured() {
    // No audio_hash means no audio to key a measurement to (and the read
    // stage already marks such files truncated or broken).
    let library = Library::new();
    library.put("Garbage.flac", &vec![0x5A; 50_000]);
    library.put("Empty.mp3", b"");
    let whole = full_band_wav(30, 20.0);
    library.put("Cut.wav", &whole[..44 + RATE as usize * 2 * 8]);
    library.walk_and_hash();
    assert_eq!(measure_all(&library), 0);
    assert_eq!(library.measured_files(), 0);
}

#[test]
fn a_file_that_cannot_be_decoded_gets_a_failure_row_and_is_not_tried_again() {
    let library = Library::new();
    // An Opus file: it has audio, which Symphonia has no decoder for.
    library.put("Opus.opus", &crate::tags::test_audio::ogg_opus());
    library.put("Good.wav", &full_band_wav(19, 12.0));
    let ids = library.walk_and_hash();
    assert_eq!(measure_all(&library), 2);
    let opus = library.stored(ids["Opus.opus"]).unwrap();
    assert_eq!(opus.failure.as_deref(), Some("unsupported_codec"));
    assert_eq!(opus.cutoff_hz, None);
    assert_eq!(opus.decoded_ms, None);
    assert!(library.stored(ids["Good.wav"]).unwrap().failure.is_none());
    // Nothing about them changed, so none is due.
    assert_eq!(measure_all(&library), 0);
}

#[test]
fn a_file_on_an_unplugged_drive_gets_no_row_and_is_measured_when_it_is_back() {
    let library = Library::new();
    library.put("a.wav", &full_band_wav(20, 12.0));
    let ids = library.walk_and_hash();
    library.volume.set_online(false);
    assert_eq!(
        measure_all(&library),
        1,
        "it was taken, and found unreachable"
    );
    assert_eq!(library.measured_files(), 0);
    library.volume.set_online(true);
    assert_eq!(measure_all(&library), 1);
    assert!(library.stored(ids["a.wav"]).is_some());
}

#[test]
fn a_file_changed_since_the_walk_is_left_for_the_next_walk() {
    let library = Library::new();
    let path = library.put("a.wav", &full_band_wav(21, 12.0));
    library.walk_and_hash();
    fs::write(&path, full_band_wav(22, 13.0)).unwrap();
    assert_eq!(measure_all(&library), 1);
    assert_eq!(
        library.measured_files(),
        0,
        "its row still describes the old file"
    );
    walk_and_hash_again(&library);
    assert_eq!(measure_all(&library), 1);
    assert_eq!(library.measured_files(), 1);
}

#[test]
fn the_job_only_reads_the_files_it_measures() {
    let library = Library::new();
    let bytes = full_band_wav(23, 12.0);
    let path = library.put("a.wav", &bytes);
    library.walk_and_hash();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    measure_all(&library);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn a_job_waits_for_a_free_thread_of_the_shared_budget_and_stops_if_cancelled_while_it_waits() {
    let library = Library::new();
    library.put("a.wav", &full_band_wav(24, 12.0));
    library.walk_and_hash();
    // Something else (a fingerprint run) holds the only thread.
    let held = library.first.try_threads(1, 1).expect("the budget is free");
    let (qualifier, taken) = counting(&library);
    let queue = library.queue(qualifier);
    let job = queue.enqueue(quality_job(None)).unwrap();
    // Running means it has started, and is parked on the budget.
    loop {
        let status = library
            .writer
            .call(move |c| crate::jobs::store::get(c, job))
            .unwrap()
            .unwrap()
            .status;
        if status == JobStatus::Running {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    queue.cancel(job).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Cancelled);
    assert_eq!(
        taken.load(Ordering::SeqCst),
        0,
        "no file was taken without a thread"
    );
    drop(held);
    queue.shutdown();
    // With the thread free it measures.
    assert_eq!(measure_all(&library), 1);
}

#[test]
fn a_quality_job_is_a_background_job_of_its_own_kind() {
    let job = quality_job(None);
    assert_eq!(job.kind, JobKind::Quality);
    assert_eq!(job.priority, Priority::BACKGROUND);
    assert_eq!(job.target, None);
    let some = quality_job(Some(vec![3, 4]));
    assert_eq!(some.target, Some(serde_json::json!({ "file_ids": [3, 4] })));
}

#[test]
fn a_job_for_named_files_measures_only_those() {
    let library = Library::new();
    library.put("a.wav", &full_band_wav(25, 12.0));
    library.put("b.wav", &full_band_wav(26, 12.0));
    let ids = library.walk_and_hash();
    let (qualifier, taken) = counting(&library);
    let queue = library.queue(qualifier);
    let job = queue
        .enqueue(quality_job(Some(vec![ids["b.wav"]])))
        .unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    queue.shutdown();
    assert_eq!(taken.load(Ordering::SeqCst), 1);
    assert!(library.stored(ids["b.wav"]).is_some());
    assert!(library.stored(ids["a.wav"]).is_none());
}

#[test]
fn a_job_is_queued_only_when_a_file_is_due_one() {
    let library = Library::new();
    library.put("a.wav", &full_band_wav(27, 12.0));
    let queue = library.queue(library.qualifier());
    let queued = Arc::new(AtomicUsize::new(0));
    let ask = |library: &Library| {
        let (q, count) = (&queue, queued.clone());
        request(&library.writer, |job| {
            count.fetch_add(1, Ordering::SeqCst);
            q.enqueue(job)
        })
        .unwrap();
        queued.load(Ordering::SeqCst)
    };
    // Nothing walked, then walked but not hashed: nothing to measure.
    assert_eq!(ask(&library), 0);
    let id = queue.enqueue(scan_job(None)).unwrap();
    wait(&library.writer, id);
    assert_eq!(ask(&library), 0);
    let id = queue.enqueue(hash_job(None)).unwrap();
    wait(&library.writer, id);
    queue.shutdown();
    // Hashed: due. (The queue is shut down, so the job just waits.)
    let queue = library.queue(library.qualifier());
    let queued2 = queued.clone();
    request(&library.writer, |job| {
        queued2.fetch_add(1, Ordering::SeqCst);
        queue.enqueue(job)
    })
    .unwrap();
    assert_eq!(queued.load(Ordering::SeqCst), 1);
    queue.shutdown();
}
