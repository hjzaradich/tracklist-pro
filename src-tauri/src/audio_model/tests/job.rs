//! The embed job on a real (temp) music folder, after the real walk and the
//! real hash stage, with the real bundled model loaded by path. The audio is
//! generated from formulas (the listening tests' signals), never committed.
//!
//! No test counts time or updates: the job is held and released with
//! latches in the file hook.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

use super::support::{wait, Library, TempVolume};
use super::{model_file, signal, wav_bytes};
use crate::audio_model::ledger::{self, Stored};
use crate::audio_model::subset::{self, SUBSET_LEN};
use crate::audio_model::{
    embed_job, request, switch, AudioModel, Embedder, ModelError, EMBEDDING_LEN,
};
use crate::hash::Hasher;
use crate::jobs::{JobId, JobKind, JobQueue, JobRecord, JobStatus};
use crate::scan::walk::Walker;
use crate::settings::{read_feature_enabled, write_feature_enabled, Feature};
use crate::waveform::tests::join_library_for_test;

const RATE: u32 = 16_000;

/// The model, loaded once for the whole run: loading checks 22 MB.
fn model() -> Arc<AudioModel> {
    static MODEL: OnceLock<Arc<AudioModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| Arc::new(AudioModel::load(&model_file()).expect("the model loads")))
        .clone()
}

/// A short track made of a few sines and some noise: `seed` changes it.
fn track(seconds: u32, hz: f64, seed: u32) -> Vec<u8> {
    let samples = signal(
        RATE,
        seconds,
        &[(0.3, hz, 0.0), (0.2, hz * 3.0, 0.5)],
        (0.15, 200.0, 4000.0),
        (0.05, seed),
    );
    wav_bytes(&samples, RATE)
}

/// What a test counts: how many times the model was loaded.
#[derive(Clone, Default)]
struct Loads(Arc<AtomicUsize>);

impl Loads {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

fn embedder(library: &Library, loads: &Loads) -> Embedder<TempVolume> {
    let (volume, loads) = (library.volume.clone(), loads.clone());
    Embedder::new(
        move || volume.clone(),
        move || {
            loads.0.fetch_add(1, Ordering::SeqCst);
            Ok(model())
        },
    )
    .workers(1)
}

fn queue(library: &Library, embedder: Embedder<TempVolume>) -> JobQueue {
    let (walk_volume, hash_volume) = (library.volume.clone(), library.volume.clone());
    JobQueue::builder(library.writer.clone())
        .workers(1)
        .handler(JobKind::Scan, Walker::new(move || walk_volume.clone()))
        .handler(JobKind::Hash, Hasher::new(move || hash_volume.clone()))
        .handler(JobKind::Embed, embedder)
        .start()
        .unwrap()
}

/// Three short tracks, walked and hashed, each a Library track's file.
fn library() -> (Library, Vec<i64>) {
    let library = Library::new();
    library.put("a.wav", &track(12, 220.0, 1));
    library.put("Folder/b.wav", &track(13, 330.0, 2));
    library.put("c.wav", &track(14, 440.0, 3));
    let ids = library.walk_and_hash();
    library.writer.call(|c| join_library_for_test(c)).unwrap();
    let ids = ids.values().copied().collect();
    (library, ids)
}

fn turn_on(library: &Library) {
    library
        .writer
        .call(|c| write_feature_enabled(c, Feature::AudioModel, true))
        .unwrap();
}

fn stored(library: &Library, file: i64) -> Option<Stored> {
    library
        .writer
        .call(move |c| ledger::current(c, file))
        .unwrap()
}

fn failure_of(library: &Library, file: i64) -> Option<String> {
    library
        .writer
        .call(move |c| {
            use rusqlite::OptionalExtension;
            c.query_row(
                "SELECT reason FROM embedding_failure WHERE file_id = ?1",
                [file],
                |r| r.get(0),
            )
            .optional()
        })
        .unwrap()
}

fn count(library: &Library, table: &'static str) -> i64 {
    library
        .writer
        .call(move |c| c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
        .unwrap()
}

fn embed_jobs(library: &Library) -> Vec<JobId> {
    library
        .writer
        .call(|c| {
            c.prepare("SELECT id FROM job WHERE kind = 'embed' ORDER BY id")?
                .query_map([], |r| Ok(JobId(r.get(0)?)))?
                .collect()
        })
        .unwrap()
}

/// Runs one embed job to the end.
fn run(library: &Library, embedder: Embedder<TempVolume>) -> JobRecord {
    let queue = queue(library, embedder);
    let id = queue.enqueue(embed_job()).unwrap();
    let job = wait(&library.writer, id);
    queue.shutdown();
    job
}

/// A hook that counts the files the job takes.
fn counting(library: &Library, loads: &Loads) -> (Embedder<TempVolume>, Arc<AtomicUsize>) {
    let taken = Arc::new(AtomicUsize::new(0));
    let count = taken.clone();
    let embedder = embedder(library, loads).on_file(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    });
    (embedder, taken)
}

/// A hook that lets the first `free` files through and holds the next one:
/// it says so on the first channel and waits for the second.
fn holding_after(
    library: &Library,
    loads: &Loads,
    free: usize,
) -> (Embedder<TempVolume>, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (held_tx, held_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (held_tx, go_rx) = (Mutex::new(held_tx), Mutex::new(go_rx));
    let seen = AtomicUsize::new(0);
    let embedder = embedder(library, loads).on_file(move |_| {
        if seen.fetch_add(1, Ordering::SeqCst) == free {
            held_tx.lock().unwrap().send(()).unwrap();
            let _ = go_rx.lock().unwrap().recv();
        }
    });
    (embedder, held_rx, go_tx)
}

#[test]
fn the_audio_model_is_off_by_default_and_nothing_is_queued_or_loaded_while_off() {
    let (library, _) = library();
    let loads = Loads::default();
    assert!(!library
        .writer
        .call(|c| read_feature_enabled(c, Feature::AudioModel))
        .unwrap());
    // Asking queues nothing, though every track is due.
    request(&library.writer, None, |job| -> Result<JobId, crate::db::DbError> {
        panic!("queued {job:?} while off")
    })
    .unwrap();
    assert_eq!(count(&library, "job"), 0);
    // Even a job that is queued does nothing: it ends at once, no load.
    let job = run(&library, embedder(&library, &loads));
    assert_eq!(job.status, JobStatus::Done);
    assert_eq!(loads.count(), 0);
    assert_eq!(count(&library, "embedding"), 0);
    let status = library
        .writer
        .call(|c| crate::audio_model::control::status_of(c))
        .unwrap();
    assert!(!status.enabled && status.fingerprinted == 0 && !status.running);
}

#[test]
fn a_run_stores_192_floats_and_the_subsets_scores_for_each_track() {
    let (library, ids) = library();
    turn_on(&library);
    let loads = Loads::default();
    let job = run(&library, embedder(&library, &loads));
    assert_eq!(job.status, JobStatus::Done, "{:?}", job.error);
    assert_eq!(loads.count(), 1, "the model is loaded once per run");
    for &id in &ids {
        let got = stored(&library, id).expect("a result per track");
        assert_eq!(got.model_id, subset::model_id());
        assert_eq!(got.fingerprint.len(), EMBEDDING_LEN);
        assert_eq!(got.scores.len(), SUBSET_LEN);
        assert!(got.fingerprint.iter().chain(&got.scores).all(|v| v.is_finite()));
    }
    // What is stored is what the model says of that file.
    let path = library.music.join("a.wav");
    let id = library.ids()["a.wav"];
    let direct = model().listen(&path, &|| true).unwrap();
    let got = stored(&library, id).unwrap();
    assert_eq!(got.scores, subset::pick(&direct.scores));
    let worst = got
        .fingerprint
        .iter()
        .zip(&direct.fingerprint)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-4, "stored fingerprint differs by {worst}");
    // Two different tracks have different fingerprints.
    let other = stored(&library, library.ids()["c.wav"]).unwrap();
    assert_ne!(got.fingerprint, other.fingerprint);
    let status = library
        .writer
        .call(|c| crate::audio_model::control::status_of(c))
        .unwrap();
    assert_eq!((status.fingerprinted, status.total, status.failed), (3, 3, 0));
}

#[test]
fn a_second_run_skips_done_tracks() {
    let (library, _) = library();
    turn_on(&library);
    let loads = Loads::default();
    let (first, taken) = counting(&library, &loads);
    assert_eq!(run(&library, first).status, JobStatus::Done);
    assert_eq!(taken.load(Ordering::SeqCst), 3);
    let (second, taken) = counting(&library, &loads);
    assert_eq!(run(&library, second).status, JobStatus::Done);
    assert_eq!(taken.load(Ordering::SeqCst), 0);
    assert_eq!(loads.count(), 1, "a run with nothing due loads nothing");
}

#[test]
fn a_changed_file_is_redone() {
    let (library, _) = library();
    turn_on(&library);
    let loads = Loads::default();
    assert_eq!(run(&library, embedder(&library, &loads)).status, JobStatus::Done);
    let b = library.ids()["Folder/b.wav"];
    let before = stored(&library, b).unwrap();
    // New audio under the same name: the walk and the hash stage see it.
    library.put("Folder/b.wav", &track(15, 523.0, 9));
    library.walk_and_hash();
    assert!(stored(&library, b).is_none(), "a stale result is not current");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let hooked = embedder(&library, &loads).on_file(move |id| log.lock().unwrap().push(id));
    assert_eq!(run(&library, hooked).status, JobStatus::Done);
    assert_eq!(*seen.lock().unwrap(), vec![b], "only the changed file is redone");
    assert_ne!(stored(&library, b).unwrap().fingerprint, before.fingerprint);
}

#[test]
fn a_corrupt_file_is_recorded_as_failed_and_the_others_still_finish() {
    let (library, ids) = library();
    turn_on(&library);
    // Garbage of the same size and modified time: it passed the walk and
    // the hash, and is damaged by the time the model reads it.
    let bad = library.ids()["Folder/b.wav"];
    let path = library.music.join("Folder").join("b.wav");
    let (len, modified) = {
        let meta = fs::metadata(&path).unwrap();
        (meta.len() as usize, meta.modified().unwrap())
    };
    fs::write(&path, vec![0x5a; len]).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let loads = Loads::default();
    let job = run(&library, embedder(&library, &loads));
    assert_eq!(job.status, JobStatus::Done, "{:?}", job.error);
    assert!(failure_of(&library, bad).is_some(), "the bad file is recorded");
    assert!(stored(&library, bad).is_none());
    for &id in ids.iter().filter(|&&id| id != bad) {
        assert!(stored(&library, id).is_some(), "file {id} was finished");
    }
    let status = library
        .writer
        .call(|c| crate::audio_model::control::status_of(c))
        .unwrap();
    assert_eq!((status.fingerprinted, status.failed, status.total), (2, 1, 3));
    // Not tried again until the file changes.
    let (again, taken) = counting(&library, &loads);
    assert_eq!(run(&library, again).status, JobStatus::Done);
    assert_eq!(taken.load(Ordering::SeqCst), 0);
}

#[test]
fn cancel_stops_between_tracks_and_a_resume_continues() {
    let (library, ids) = library();
    turn_on(&library);
    let loads = Loads::default();
    let (held, reached, release) = holding_after(&library, &loads, 1);
    let queue = queue(&library, held);
    let id = queue.enqueue(embed_job()).unwrap();
    reached.recv().unwrap();
    queue.cancel(id).unwrap();
    release.send(()).unwrap();
    assert_eq!(wait(&library.writer, id).status, JobStatus::Cancelled);
    queue.shutdown();
    assert_eq!(count(&library, "embedding"), 1, "the first track was finished");
    // The next run takes up the rest.
    let (rest, taken) = counting(&library, &loads);
    assert_eq!(run(&library, rest).status, JobStatus::Done);
    assert_eq!(taken.load(Ordering::SeqCst), 2);
    assert!(ids.iter().all(|&id| stored(&library, id).is_some()));
}

#[test]
fn switching_off_cancels_the_run_and_keeps_what_is_stored() {
    let (library, ids) = library();
    let loads = Loads::default();
    let (held, reached, release) = holding_after(&library, &loads, 1);
    let queue = queue(&library, held);
    // Switching on queues the work.
    switch(&library.writer, &queue, true).unwrap();
    let id = embed_jobs(&library)[0];
    reached.recv().unwrap();
    switch(&library.writer, &queue, false).unwrap();
    release.send(()).unwrap();
    assert_eq!(wait(&library.writer, id).status, JobStatus::Cancelled);
    assert_eq!(count(&library, "embedding"), 1, "what was stored is kept");
    let status = library
        .writer
        .call(|c| crate::audio_model::control::status_of(c))
        .unwrap();
    assert!(!status.enabled && status.fingerprinted == 1);
    // Off, asking queues nothing.
    request(&library.writer, None, |job| -> Result<JobId, crate::db::DbError> {
        panic!("queued {job:?} while off")
    })
    .unwrap();
    // Switching on again finishes the rest.
    switch(&library.writer, &queue, true).unwrap();
    let again = *embed_jobs(&library).last().unwrap();
    assert_ne!(again, id);
    assert_eq!(wait(&library.writer, again).status, JobStatus::Done);
    queue.shutdown();
    assert!(ids.iter().all(|&id| stored(&library, id).is_some()));
}

#[test]
fn a_model_that_cannot_be_loaded_ends_the_job_with_a_reason_and_nothing_else() {
    let (library, ids) = library();
    turn_on(&library);
    let volume = library.volume.clone();
    let broken = Embedder::new(
        move || volume.clone(),
        || Err(ModelError::Missing("nowhere/ced-tiny.onnx".into())),
    );
    let job = run(&library, broken);
    assert_eq!(job.status, JobStatus::Failed);
    assert!(job.error.unwrap_or_default().contains("audio model"));
    assert_eq!(count(&library, "embedding") + count(&library, "embedding_failure"), 0);
    // The app carries on: the next run, with the model, finishes.
    let loads = Loads::default();
    assert_eq!(run(&library, embedder(&library, &loads)).status, JobStatus::Done);
    assert!(ids.iter().all(|&id| stored(&library, id).is_some()));
}

#[test]
fn the_embed_job_runs_on_between_one_and_four_threads() {
    let workers = crate::audio_model::job::default_workers();
    assert!((1..=4).contains(&workers));
}
