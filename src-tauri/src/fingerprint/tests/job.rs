//! The fingerprint job on a real (temp) music folder, after the real walk.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use super::audio;
use super::support::{wait, Library};
use crate::fingerprint::stored::CURRENT_PREFIX;
use crate::fingerprint::{
    compare, fingerprint_job, raise, start, Outcome, Unfingerprintable, VERSION,
};
use crate::jobs::{JobId, JobStatus, JobUpdate, Priority};
use crate::tags::test_audio;

/// Runs one fingerprint job over every due file with `library`'s default
/// fingerprinter, to the end.
fn fingerprint_all(library: &Library) -> JobStatus {
    let queue = library.queue(library.fingerprinter());
    let id = queue.enqueue(fingerprint_job(None)).unwrap();
    let status = wait(&library.writer, id).status;
    queue.shutdown();
    status
}

/// Every job update the queue sent. Poison-tolerant, so one failed
/// assertion doesn't take the listener down with it.
#[derive(Clone, Default)]
struct Heard(Arc<Mutex<Vec<JobUpdate>>>);

impl Heard {
    fn sink(&self) -> impl Fn(&[JobUpdate]) + Send + 'static {
        let list = self.0.clone();
        move |u| {
            list.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(u)
        }
    }

    fn all(&self) -> Vec<JobUpdate> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Waits for `job`'s update saying it's done. The database says so
    /// first; the update follows in the next batch, and shutting the queue
    /// down doesn't wait for it.
    fn until_done(&self, job: JobId) -> Vec<JobUpdate> {
        let started = std::time::Instant::now();
        loop {
            let mine: Vec<_> = self.all().into_iter().filter(|u| u.id == job).collect();
            if mine.iter().any(|u| u.status == JobStatus::Done) {
                return mine;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "no done update for job {job}: {mine:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn song(seed: u64, seconds: f64) -> Vec<u8> {
    audio::flac(&audio::pcm(seed, seconds, 22_050, 1))
}

#[test]
fn every_present_file_gets_a_fingerprint_stored_with_its_version() {
    let library = Library::new();
    library.put("Artist/One.flac", &song(1, 12.0));
    library.put(
        "Artist/Two.wav",
        &audio::wav(&audio::pcm(2, 12.0, 44_100, 2)),
    );
    library.put("Three.aiff", &audio::aiff(&audio::pcm(3, 12.0, 22_050, 1)));
    let ids = library.walk();
    assert_eq!(fingerprint_all(&library), JobStatus::Done);
    for (path, id) in &ids {
        let blob = library
            .blob(*id)
            .unwrap_or_else(|| panic!("{path} has none"));
        assert_eq!(&blob[..5], CURRENT_PREFIX.as_slice(), "{path}");
        let fp = library.fingerprint(*id).unwrap();
        assert_eq!(fp.version(), VERSION);
        assert!(fp.seconds() > 8.0, "{path}: {}", fp.seconds());
        assert_eq!(library.outcome(*id), Some(Outcome::Done));
    }
}

#[test]
fn the_same_song_in_two_formats_in_the_library_compares_as_the_same() {
    let library = Library::new();
    library.put("a.flac", &song(4, 40.0));
    library.put("b.wav", &audio::wav(&audio::pcm(4, 40.0, 44_100, 2)));
    library.put("c.flac", &song(5, 40.0));
    let ids = library.walk();
    assert_eq!(fingerprint_all(&library), JobStatus::Done);
    let fp = |name: &str| library.fingerprint(ids[name]).unwrap();
    assert!(compare(&fp("a.flac"), &fp("b.wav")).unwrap().is_duplicate());
    assert!(!compare(&fp("a.flac"), &fp("c.flac"))
        .unwrap()
        .is_duplicate());
}

#[test]
fn the_format_comes_from_the_bytes_so_a_wav_holding_flac_is_fingerprinted() {
    let library = Library::new();
    library.put("Mislabeled.wav", &song(6, 12.0));
    let ids = library.walk();
    fingerprint_all(&library);
    assert!(library.fingerprint(ids["Mislabeled.wav"]).is_some());
}

#[test]
fn an_undecodable_file_keeps_a_null_fingerprint_and_a_reason_and_the_rest_still_get_theirs() {
    let library = Library::new();
    library.put("Opus.opus", &test_audio::ogg_opus());
    library.put("Empty.mp3", b"");
    library.put("Garbage.flac", &vec![0x5A; 50_000]);
    library.put("Short.wav", &audio::wav(&audio::pcm(7, 1.0, 22_050, 1)));
    library.put("Good.flac", &song(8, 12.0));
    let ids = library.walk();
    assert_eq!(fingerprint_all(&library), JobStatus::Done);

    let failed = |name: &str| {
        assert_eq!(library.blob(ids[name]), None, "{name}");
        match library.outcome(ids[name]) {
            Some(Outcome::Failed(why)) => why,
            other => panic!("{name}: {other:?}"),
        }
    };
    assert_eq!(failed("Opus.opus"), Unfingerprintable::UnsupportedCodec);
    assert_eq!(failed("Short.wav"), Unfingerprintable::TooShort);
    failed("Empty.mp3");
    failed("Garbage.flac");
    assert!(library.fingerprint(ids["Good.flac"]).is_some());
}

#[test]
fn a_failed_file_is_not_tried_again_until_its_size_or_modified_time_changes() {
    let library = Library::new();
    let path = library.put("Opus.opus", &test_audio::ogg_opus());
    let ids = library.walk();
    let tried = Arc::new(Mutex::new(Vec::new()));
    let seen = tried.clone();
    let run = || {
        let seen = seen.clone();
        let queue = library.queue(library.fingerprinter().on_file(move |id| {
            seen.lock().unwrap().push(id);
        }));
        let id = queue.enqueue(fingerprint_job(None)).unwrap();
        wait(&library.writer, id);
        queue.shutdown();
    };
    run();
    run();
    assert_eq!(*tried.lock().unwrap(), [ids["Opus.opus"]], "tried once");

    // Replaced by a real song: the walk sees the new size, and it's due.
    fs::write(&path, song(9, 12.0)).unwrap();
    library.walk();
    run();
    assert_eq!(tried.lock().unwrap().len(), 2);
    assert!(library.fingerprint(ids["Opus.opus"]).is_some());
}

#[test]
fn a_fingerprinted_file_is_done_again_only_once_its_size_or_modified_time_changes() {
    let library = Library::new();
    let path = library.put("Song.flac", &song(10, 12.0));
    let ids = library.walk();
    let id = ids["Song.flac"];
    fingerprint_all(&library);
    let first = library.fingerprint(id).unwrap();

    let tried = Arc::new(Mutex::new(0));
    let count = tried.clone();
    let queue = library.queue(library.fingerprinter().on_file(move |_| {
        *count.lock().unwrap() += 1;
    }));
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    wait(&library.writer, job);
    assert_eq!(*tried.lock().unwrap(), 0, "unchanged: not done again");

    // Retagged the way rekordbox does it: new mtime, same audio.
    let later = SystemTime::now() + Duration::from_secs(60);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    library.walk();
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    wait(&library.writer, job);
    queue.shutdown();
    assert_eq!(*tried.lock().unwrap(), 1, "the new mtime makes it due");
    assert_eq!(library.fingerprint(id).unwrap(), first);
}

#[test]
fn a_fingerprint_from_an_older_version_is_replaced() {
    let library = Library::new();
    library.put("Song.flac", &song(11, 12.0));
    let ids = library.walk();
    let id = ids["Song.flac"];
    // Left by a build whose VERSION was 0.
    library
        .writer
        .call(move |c| {
            c.execute(
                "UPDATE file SET fingerprint = X'544C465000' WHERE id = ?1",
                [id],
            )
        })
        .unwrap();
    fingerprint_all(&library);
    assert_eq!(library.fingerprint(id).unwrap().version(), VERSION);
}

#[test]
fn raised_files_are_fingerprinted_before_the_rest() {
    let library = Library::new();
    for n in 0..8 {
        library.put(&format!("{n}.flac"), &song(20 + n, 4.0));
    }
    let ids = library.walk();
    let id = |n: u64| ids[&format!("{n}.flac")];
    let order = Arc::new(Mutex::new(Vec::new()));
    let (started, first_file) = mpsc::channel();
    let (go, wait_for_go) = mpsc::channel::<()>();
    let (seen, wait_for_go) = (order.clone(), Mutex::new(wait_for_go));
    let started = Mutex::new(started);
    let queue = library.queue(library.fingerprinter().on_file(move |file| {
        let mut seen = seen.lock().unwrap();
        seen.push(file);
        if seen.len() == 1 {
            // Hold the first file until the UI has raised the others.
            started.lock().unwrap().send(()).unwrap();
            wait_for_go.lock().unwrap().recv().unwrap();
        }
    }));
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    first_file.recv().unwrap();
    // The UI scrolled: 6 and 7 are on screen, then 4.
    assert_eq!(
        raise(&queue, &library.first, vec![id(6), id(7)]).unwrap(),
        None
    );
    assert_eq!(raise(&queue, &library.first, vec![id(4)]).unwrap(), None);
    go.send(()).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    queue.shutdown();

    let order = order.lock().unwrap().clone();
    let expected: Vec<i64> = [0, 4, 6, 7, 1, 2, 3, 5].into_iter().map(id).collect();
    assert_eq!(order, expected);
    assert!(ids.values().all(|&f| library.fingerprint(f).is_some()));
}

#[test]
fn raising_files_when_no_fingerprint_job_runs_starts_one_for_just_those_at_user_priority() {
    let library = Library::new();
    for n in 0..4 {
        library.put(&format!("{n}.flac"), &song(30 + n, 4.0));
    }
    let ids = library.walk();
    let heard = Heard::default();
    let queue = library.queue_with_updates(library.fingerprinter(), heard.sink());
    let wanted = vec![ids["2.flac"]];
    let job = raise(&queue, &library.first, wanted)
        .unwrap()
        .expect("nothing was running, so a job starts");
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    let updates = heard.until_done(job);
    queue.shutdown();
    assert!(
        updates.iter().all(|u| u.priority == Priority::USER.0),
        "{updates:?}"
    );
    assert!(library.fingerprint(ids["2.flac"]).is_some());
    for other in ["0.flac", "1.flac", "3.flac"] {
        assert!(library.fingerprint(ids[other]).is_none(), "{other}");
    }
}

#[test]
fn asking_to_fingerprint_everything_twice_reuses_the_job_already_queued() {
    let library = Library::new();
    let queue = library.queue(library.fingerprinter());
    queue.shutdown(); // keep jobs queued
    let a = start(&queue, fingerprint_job(None)).unwrap();
    let b = start(&queue, fingerprint_job(None)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn cancelling_stops_mid_file_and_stores_nothing_for_it() {
    let library = Library::new();
    library.put("Long.flac", &song(40, 240.0));
    let ids = library.walk();
    let (at_packet, packet) = mpsc::channel();
    let (go, wait_for_go) = mpsc::channel::<()>();
    let (at_packet, wait_for_go) = (Mutex::new(at_packet), Mutex::new(wait_for_go));
    let packets = Arc::new(Mutex::new(0u64));
    let count = packets.clone();
    let queue = library.queue(library.fingerprinter().on_packet(move |n| {
        *count.lock().unwrap() = n;
        if n == 50 {
            at_packet.lock().unwrap().send(()).unwrap();
            wait_for_go.lock().unwrap().recv().unwrap();
        }
    }));
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    packet.recv().unwrap();
    queue.cancel(job).unwrap();
    go.send(()).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Cancelled);
    queue.shutdown();
    // 240 s of 4096-frame FLAC blocks is about 1,300 packets.
    assert!(
        *packets.lock().unwrap() <= 51,
        "it stopped at the next packet"
    );
    assert_eq!(library.blob(ids["Long.flac"]), None);
    assert_eq!(library.outcome(ids["Long.flac"]), None, "not a failure");
}

#[test]
fn a_decoder_crash_counts_against_that_file_alone() {
    let library = Library::new();
    library.put("a.flac", &song(50, 5.0));
    library.put("b.flac", &song(51, 5.0));
    let ids = library.walk();
    let crash_on = ids["a.flac"];
    let current = Arc::new(Mutex::new(0));
    let (set, get) = (current.clone(), current);
    let queue = library.queue(
        library
            .fingerprinter()
            .on_file(move |id| *set.lock().unwrap() = id)
            .on_packet(move |n| {
                if n == 3 && *get.lock().unwrap() == crash_on {
                    panic!("a decoder bug");
                }
            }),
    );
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    queue.shutdown();
    assert_eq!(
        library.outcome(crash_on),
        Some(Outcome::Failed(Unfingerprintable::DecoderCrashed))
    );
    assert!(library.fingerprint(ids["b.flac"]).is_some());
}

#[test]
fn a_file_on_an_unplugged_drive_stays_due_for_when_it_is_back() {
    let library = Library::new();
    library.put("Song.flac", &song(52, 5.0));
    let ids = library.walk();
    library.volume.set_online(false);
    assert_eq!(fingerprint_all(&library), JobStatus::Done);
    assert_eq!(library.outcome(ids["Song.flac"]), None);
    library.volume.set_online(true);
    fingerprint_all(&library);
    assert!(library.fingerprint(ids["Song.flac"]).is_some());
}

#[test]
fn a_file_locked_by_another_program_is_not_marked_failed_and_is_done_once_free() {
    use std::os::windows::fs::OpenOptionsExt;
    let library = Library::new();
    let path = library.put("Locked.flac", &song(55, 5.0));
    let ids = library.walk();
    // Another program holding it open with no sharing, as some taggers do.
    let lock = fs::File::options()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert_eq!(fingerprint_all(&library), JobStatus::Done);
    assert_eq!(library.blob(ids["Locked.flac"]), None);
    assert_eq!(
        library.outcome(ids["Locked.flac"]),
        Some(Outcome::Skipped(crate::scan_state::UNREACHABLE.into())),
        "a skip, never a failure"
    );
    drop(lock);
    fingerprint_all(&library);
    assert!(library.fingerprint(ids["Locked.flac"]).is_some());
}

#[test]
fn an_online_only_onedrive_file_is_skipped_unless_the_user_opted_in() {
    let library = Library::new();
    library.put("Cloud.flac", &song(58, 5.0));
    library.put("Local.flac", &song(59, 5.0));
    let ids = library.walk();
    let cloud = ids["Cloud.flac"];
    // What the walk records for a OneDrive placeholder (1aB-8); a test
    // can't make a real one.
    library
        .writer
        .call(move |c| c.execute("UPDATE file SET online_only = 1 WHERE id = ?1", [cloud]))
        .unwrap();
    fingerprint_all(&library);
    assert_eq!(
        library.blob(cloud),
        None,
        "never opened, so never downloaded"
    );
    assert_eq!(
        library.outcome(cloud),
        Some(Outcome::Skipped(crate::scan_state::ONLINE_ONLY.into()))
    );
    assert!(library.fingerprint(ids["Local.flac"]).is_some());

    // Opted in: skips are due again, and now it's read.
    library
        .writer
        .call(|c| {
            c.execute(
                "INSERT INTO setting (key, value) VALUES (?1, json('true'))",
                [crate::scan::online_only::READ_ONLINE_ONLY_FILES],
            )
        })
        .unwrap();
    fingerprint_all(&library);
    assert!(library.fingerprint(cloud).is_some());
}

#[test]
fn a_file_changed_since_the_walk_is_left_for_the_next_walk() {
    let library = Library::new();
    let path = library.put("Song.flac", &song(53, 5.0));
    let ids = library.walk();
    fs::write(&path, song(54, 6.0)).unwrap();
    fingerprint_all(&library);
    assert_eq!(library.blob(ids["Song.flac"]), None);
    assert_eq!(library.outcome(ids["Song.flac"]), None);
    library.walk();
    fingerprint_all(&library);
    assert!(library.fingerprint(ids["Song.flac"]).is_some());
}

#[test]
fn progress_only_rises_and_ends_at_one() {
    let library = Library::new();
    for n in 0..5 {
        library.put(&format!("{n}.flac"), &song(60 + n, 6.0));
    }
    library.walk();
    let heard = Heard::default();
    let queue = library.queue_with_updates(library.fingerprinter(), heard.sink());
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    wait(&library.writer, job);
    let updates = heard.until_done(job);
    queue.shutdown();
    let progress: Vec<f64> = updates.iter().filter_map(|u| u.progress).collect();
    // Updates within 50 ms of each other coalesce (jobs::dispatch), so how
    // many arrive depends on the machine's speed: a fast one may send two.
    // What holds everywhere: something short of done was reported, nothing
    // went back, and it ended at exactly one.
    assert!(progress.iter().any(|p| *p < 1.0), "{progress:?}");
    assert!(progress.windows(2).all(|w| w[1] >= w[0]), "{progress:?}");
    assert_eq!(progress.last(), Some(&1.0));
}

#[test]
fn several_threads_share_the_work_and_never_do_a_file_twice() {
    let library = Library::new();
    for n in 0..12 {
        library.put(&format!("{n}.flac"), &song(70 + n, 4.0));
    }
    let ids = library.walk();
    let taken = Arc::new(Mutex::new(Vec::new()));
    let seen = taken.clone();
    let queue = library.queue(
        library
            .fingerprinter()
            .threads(4)
            .on_file(move |id| seen.lock().unwrap().push(id)),
    );
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    queue.shutdown();
    let mut taken = taken.lock().unwrap().clone();
    taken.sort();
    let mut all: Vec<i64> = ids.values().copied().collect();
    all.sort();
    assert_eq!(taken, all);
}

#[test]
fn a_row_the_walk_changes_mid_decode_gets_no_fingerprint_stored() {
    let library = Library::new();
    library.put("Song.flac", &song(56, 8.0));
    let ids = library.walk();
    let id = ids["Song.flac"];
    let writer = library.writer.clone();
    let queue = library.queue(library.fingerprinter().on_packet(move |n| {
        if n == 5 {
            // What a rewalk records when it sees the file changed (the
            // file itself is untouched, so only the stored row differs).
            writer
                .call(move |c| c.execute("UPDATE file SET mtime = mtime + 1 WHERE id = ?1", [id]))
                .unwrap();
        }
    }));
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    queue.shutdown();
    assert_eq!(library.blob(id), None, "the write is refused");
    assert_eq!(library.outcome(id), None);
    // The next walk puts the row right, and then it's done.
    library.walk();
    fingerprint_all(&library);
    assert!(library.fingerprint(id).is_some());
}

#[test]
fn a_file_rewritten_while_it_decodes_is_left_for_the_next_walk() {
    let library = Library::new();
    let path = library.put("Song.flac", &song(57, 8.0));
    let ids = library.walk();
    let id = ids["Song.flac"];
    let touched = path.clone();
    let queue = library.queue(library.fingerprinter().on_packet(move |n| {
        if n == 5 {
            // Another program saves the file while it's being read.
            fs::File::options()
                .write(true)
                .open(&touched)
                .unwrap()
                .set_modified(SystemTime::now() + Duration::from_secs(120))
                .unwrap();
        }
    }));
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    queue.shutdown();
    assert_eq!(library.blob(id), None);
    assert_eq!(library.outcome(id), None, "not a failure either");
    library.walk();
    fingerprint_all(&library);
    assert!(library.fingerprint(id).is_some());
}

#[test]
fn raised_files_a_cancelled_job_did_not_finish_go_to_a_new_job() {
    let library = Library::new();
    for n in 0..4 {
        library.put(&format!("{n}.flac"), &song(90 + n, 30.0));
    }
    let ids = library.walk();
    let id = |n: u64| ids[&format!("{n}.flac")];
    let (at, reached) = mpsc::channel();
    let (go, wait_for_go) = mpsc::channel::<()>();
    let (at, wait_for_go) = (Mutex::new(at), Mutex::new(wait_for_go));
    let (raised_one, raised_two) = (id(2), id(3));
    let stops = Arc::new(Mutex::new(0));
    let seen = stops.clone();
    let queue = library.queue(library.fingerprinter().on_file(move |file| {
        // Hold the first file, then the first raised one, once each.
        let mut stops = seen.lock().unwrap();
        if *stops < 2 && (*stops == 0 || file == raised_one) {
            *stops += 1;
            at.lock().unwrap().send(file).unwrap();
            wait_for_go.lock().unwrap().recv().unwrap();
        }
    }));
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    assert_eq!(reached.recv().unwrap(), id(0));
    raise(&queue, &library.first, vec![raised_one, raised_two]).unwrap();
    go.send(()).unwrap();
    // The job takes the first raised file, and is cancelled while on it.
    assert_eq!(reached.recv().unwrap(), raised_one);
    queue.cancel(job).unwrap();
    go.send(()).unwrap();
    assert_eq!(wait(&library.writer, job).status, JobStatus::Cancelled);

    // Both raised files went to a new job, which does just those.
    let started = std::time::Instant::now();
    while library.fingerprint(raised_one).is_none() || library.fingerprint(raised_two).is_none() {
        assert!(started.elapsed() < Duration::from_secs(120), "never done");
        std::thread::sleep(Duration::from_millis(20));
    }
    queue.shutdown();
    assert!(
        library.fingerprint(id(1)).is_none(),
        "not raised, so not redone"
    );
}

#[test]
fn all_fingerprint_jobs_together_stay_within_one_thread_budget() {
    let library = Library::new();
    for n in 0..6 {
        library.put(&format!("{n}.flac"), &song(100 + n, 6.0));
    }
    let ids = library.walk();
    let most = Arc::new(Mutex::new(0));
    let (first, seen) = (library.first.clone(), most.clone());
    let queue = library.queue_with(
        library.fingerprinter().threads(2).on_packet(move |_| {
            let mut most = seen.lock().unwrap();
            *most = (*most).max(first.busy());
        }),
        3,
        |_| {},
    );
    // Three jobs at once on a three-worker queue, each wanting two threads.
    let jobs: Vec<_> = [None, Some(vec![ids["4.flac"]]), Some(vec![ids["5.flac"]])]
        .into_iter()
        .map(|target| queue.enqueue(fingerprint_job(target)).unwrap())
        .collect();
    for job in jobs {
        assert_eq!(wait(&library.writer, job).status, JobStatus::Done);
    }
    queue.shutdown();
    assert!(
        *most.lock().unwrap() <= 2,
        "{} threads at once",
        most.lock().unwrap()
    );
    assert!(ids.values().all(|&f| library.fingerprint(f).is_some()));
}

#[test]
fn fingerprint_threads_run_below_normal_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    let library = Library::new();
    library.put("Song.flac", &song(110, 4.0));
    library.walk();
    let priority = Arc::new(Mutex::new(None));
    let seen = priority.clone();
    let queue = library.queue(library.fingerprinter().on_file(move |_| {
        // SAFETY: a pseudo-handle to this thread; nothing to close.
        *seen.lock().unwrap() = Some(unsafe { GetThreadPriority(GetCurrentThread()) });
    }));
    let job = queue.enqueue(fingerprint_job(None)).unwrap();
    wait(&library.writer, job);
    queue.shutdown();
    assert_eq!(
        *priority.lock().unwrap(),
        Some(THREAD_PRIORITY_BELOW_NORMAL)
    );
}

/// Every file and folder under `dir`, with its bytes and modified time.
fn snapshot(dir: &Path, skip: &Path, out: &mut BTreeMap<PathBuf, (Vec<u8>, SystemTime)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path == skip {
            continue;
        }
        let meta = fs::symlink_metadata(&path).unwrap();
        let bytes = if meta.is_dir() {
            snapshot(&path, skip, out);
            Vec::new()
        } else {
            fs::read(&path).unwrap()
        };
        out.insert(path, (bytes, meta.modified().unwrap()));
    }
}

#[test]
fn fingerprinting_writes_nothing_outside_the_app_data_folder() {
    let sandbox = tempfile::tempdir().unwrap();
    let sandbox = fs::canonicalize(sandbox.path()).unwrap();
    let data = sandbox.join("data");
    let library = Library::with_data_in(Some(&data));
    library.put("Artist/Song.flac", &song(80, 8.0));
    library.put(
        "Artist/Song.wav",
        &audio::wav(&audio::pcm(80, 8.0, 22_050, 2)),
    );
    library.put("Bad.opus", &test_audio::ogg_opus());
    library.walk();

    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let watch = |out: &mut BTreeMap<_, _>| {
        snapshot(&library.base, &library.data, out);
        snapshot(&sandbox, &data, out);
    };
    for entry in fs::read_dir(library.music.join("Artist")).unwrap() {
        fs::File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    library.walk();
    let mut before = BTreeMap::new();
    watch(&mut before);

    assert_eq!(fingerprint_all(&library), JobStatus::Done);

    let mut after = BTreeMap::new();
    watch(&mut after);
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .collect();
    assert!(
        changed.is_empty(),
        "changed outside the data folder: {changed:?}"
    );
}

// ---- 1aC-10: a tag-only rewrite doesn't decode again ------------------------

mod carried {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::hash::hash_job;
    use crate::tags::test_audio::{id3_text, id3v2};

    fn mp3(seed: u64) -> Vec<u8> {
        audio::mp3(&audio::pcm(seed, 10.0, 22_050, 1), 128)
    }

    /// The same audio with an ID3v2 tag in front, as a tagger writes it: a
    /// new size, and the audio_hash unchanged.
    fn retagged(bytes: &[u8], title: &str) -> Vec<u8> {
        [id3v2(&[id3_text(b"TIT2", title)], 256), bytes.to_vec()].concat()
    }

    fn hash_all(library: &Library) {
        let queue = library.queue(library.fingerprinter());
        let id = queue.enqueue(hash_job(None)).unwrap();
        assert_eq!(wait(&library.writer, id).status, JobStatus::Done);
        queue.shutdown();
    }

    /// Runs a fingerprint job over everything due; returns how many packets
    /// it decoded, which is 0 if no file was opened to decode.
    fn fingerprint_counting(library: &Library) -> u64 {
        let packets = Arc::new(AtomicU64::new(0));
        let count = packets.clone();
        let queue = library.queue(library.fingerprinter().on_packet(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        }));
        let id = queue.enqueue(fingerprint_job(None)).unwrap();
        assert_eq!(wait(&library.writer, id).status, JobStatus::Done);
        queue.shutdown();
        packets.load(Ordering::SeqCst)
    }

    /// A library with `Song.mp3` walked, hashed and fingerprinted.
    fn settled() -> (Library, PathBuf, i64, Vec<u8>) {
        let library = Library::new();
        let original = mp3(21);
        let path = library.put("Song.mp3", &original);
        let id = library.walk()["Song.mp3"];
        hash_all(&library);
        assert!(fingerprint_counting(&library) > 0, "decoded the first time");
        (library, path, id, original)
    }

    fn recorded(library: &Library, id: i64) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        library
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT audio_hash, fingerprint_audio_hash FROM file WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .unwrap()
    }

    fn due(library: &Library) -> usize {
        library
            .writer
            .call(|c| crate::fingerprint::ledger::due_ids(c, None))
            .unwrap()
            .len()
    }

    #[test]
    fn a_fingerprint_records_the_audio_hash_it_came_from() {
        let (library, _, id, _) = settled();
        let (audio_hash, from) = recorded(&library, id);
        assert!(audio_hash.is_some());
        assert_eq!(from, audio_hash);
    }

    #[test]
    fn a_tag_only_rewrite_skips_the_decode() {
        let (library, path, id, original) = settled();
        let before = library.blob(id).unwrap();
        let old_size = fs::metadata(&path).unwrap().len();

        // New size and mtime, same audio.
        fs::write(&path, retagged(&original, "Night Drive")).unwrap();
        library.walk();
        hash_all(&library);
        assert_eq!(due(&library), 1, "the new stat makes it due");
        assert_ne!(fs::metadata(&path).unwrap().len(), old_size);

        assert_eq!(fingerprint_counting(&library), 0, "nothing was decoded");
        assert_eq!(library.blob(id).unwrap(), before, "the fingerprint stands");
        assert_eq!(library.outcome(id), Some(Outcome::Done));
        assert_eq!(due(&library), 0, "and it's done at the new size and mtime");
        let (audio_hash, from) = recorded(&library, id);
        assert_eq!(from, audio_hash, "still tied to the audio it came from");
    }

    #[test]
    fn changed_audio_is_fingerprinted_again() {
        let (library, path, id, _) = settled();
        let before = library.blob(id).unwrap();
        fs::write(&path, mp3(22)).unwrap();
        library.walk();
        hash_all(&library);
        assert!(fingerprint_counting(&library) > 0, "decoded");
        assert_ne!(library.blob(id).unwrap(), before);
    }

    #[test]
    fn a_stale_hash_stage_means_a_normal_fingerprint() {
        let (library, path, id, original) = settled();
        fs::write(&path, retagged(&original, "Night Drive")).unwrap();
        library.walk();
        // The hash stage hasn't run for the new size and mtime.
        assert!(
            fingerprint_counting(&library) > 0,
            "decoded, not waited for"
        );
        assert_eq!(library.outcome(id), Some(Outcome::Done));
        let (_, from) = recorded(&library, id);
        assert_eq!(
            from, None,
            "made while the hash was stale, so no audio_hash is vouched for"
        );
    }

    #[test]
    fn a_fingerprint_version_bump_means_a_normal_fingerprint() {
        let (library, path, id, original) = settled();
        fs::write(&path, retagged(&original, "Night Drive")).unwrap();
        library.walk();
        hash_all(&library);
        // The fingerprint row is from another version.
        library
            .writer
            .call(move |c| {
                c.execute(
                    "UPDATE file_stage SET version = version + 1
                     WHERE file_id = ?1 AND stage = 'fingerprint'",
                    [id],
                )
            })
            .unwrap();
        assert!(fingerprint_counting(&library) > 0, "decoded");
    }

    #[test]
    fn a_null_recorded_hash_means_a_normal_fingerprint() {
        let (library, path, id, original) = settled();
        library
            .writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET fingerprint_audio_hash = NULL WHERE id = ?1",
                    [id],
                )
            })
            .unwrap();
        fs::write(&path, retagged(&original, "Night Drive")).unwrap();
        library.walk();
        hash_all(&library);
        assert!(fingerprint_counting(&library) > 0, "decoded");
        let (audio_hash, from) = recorded(&library, id);
        assert_eq!(from, audio_hash, "and it's recorded again");
    }

    #[test]
    fn a_file_with_no_audio_hash_means_a_normal_fingerprint() {
        let (library, path, id, original) = settled();
        fs::write(&path, retagged(&original, "Night Drive")).unwrap();
        library.walk();
        hash_all(&library);
        // The hash stage is current, but has no audio_hash to offer.
        library
            .writer
            .call(move |c| c.execute("UPDATE file SET audio_hash = NULL WHERE id = ?1", [id]))
            .unwrap();
        assert!(fingerprint_counting(&library) > 0, "decoded");
    }

    #[test]
    fn a_fingerprint_that_failed_is_never_carried_forward() {
        let library = Library::new();
        let path = library.put("Bad.opus", &test_audio::ogg_opus());
        let id = library.walk()["Bad.opus"];
        hash_all(&library);
        fingerprint_counting(&library);
        assert!(matches!(library.outcome(id), Some(Outcome::Failed(_))));
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&[0; 64]);
        fs::write(&path, bytes).unwrap();
        library.walk();
        hash_all(&library);
        fingerprint_counting(&library);
        assert!(matches!(library.outcome(id), Some(Outcome::Failed(_))));
        assert_eq!(library.blob(id), None, "still no fingerprint");
    }

    #[test]
    fn a_hash_stage_that_skipped_the_file_means_a_normal_fingerprint() {
        let (library, path, id, _) = settled();
        let before = library.blob(id).unwrap();
        fs::write(&path, mp3(22)).unwrap();
        library.walk();
        // The hash stage couldn't reach it at its new stat: its row is
        // current but skipped, and audio_hash is still the old audio's.
        library
            .writer
            .call(move |c| {
                let (size, mtime): (Option<i64>, Option<i64>) =
                    c.query_row("SELECT size, mtime FROM file WHERE id = ?1", [id], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?;
                crate::scan_state::record(
                    c,
                    crate::scan_state::Stage::Hash,
                    i64::from(crate::hash::DEFINITION),
                    &[crate::scan_state::Recorded {
                        file: id,
                        size,
                        mtime,
                        outcome: crate::scan_state::Outcome::unreachable(),
                    }],
                )
            })
            .unwrap();
        assert!(fingerprint_counting(&library) > 0, "decoded");
        assert_ne!(library.blob(id).unwrap(), before);
    }

    #[test]
    fn a_hash_stage_of_another_definition_means_a_normal_fingerprint() {
        let (library, path, id, original) = settled();
        fs::write(&path, retagged(&original, "Night Drive")).unwrap();
        library.walk();
        hash_all(&library);
        library
            .writer
            .call(move |c| {
                c.execute(
                    "UPDATE file_stage SET version = version + 1
                     WHERE file_id = ?1 AND stage = 'hash'",
                    [id],
                )
            })
            .unwrap();
        assert!(fingerprint_counting(&library) > 0, "decoded");
    }

    #[test]
    fn a_skipped_fingerprint_row_is_never_carried_forward() {
        let (library, path, id, original) = settled();
        fs::write(&path, retagged(&original, "Night Drive")).unwrap();
        library.walk();
        hash_all(&library);
        // Skipped last time (e.g. unreachable after a version bump): the
        // blob it kept isn't vouched for.
        library
            .writer
            .call(move |c| {
                c.execute(
                    "UPDATE file_stage SET status = 'skipped', reason = 'unreachable'
                     WHERE file_id = ?1 AND stage = 'fingerprint'",
                    [id],
                )
            })
            .unwrap();
        assert!(fingerprint_counting(&library) > 0, "decoded");
    }

    #[test]
    fn a_row_moved_since_it_was_read_is_not_carried_forward() {
        let (library, path, id, original) = settled();
        fs::write(&path, retagged(&original, "Night Drive")).unwrap();
        library.walk();
        hash_all(&library);
        let carried = library
            .writer
            .call(move |c| {
                let due = crate::fingerprint::ledger::due(c, id)?.unwrap();
                // A walk's touch-only carry moves the row and the hash row
                // between the read and the carry.
                c.execute("UPDATE file SET mtime = mtime + 1 WHERE id = ?1", [id])?;
                c.execute(
                    "UPDATE file_stage SET mtime = mtime + 1 WHERE file_id = ?1 AND stage = 'hash'",
                    [id],
                )?;
                crate::fingerprint::ledger::carry_forward(c, &due)
            })
            .unwrap();
        assert!(!carried);
    }

    #[test]
    fn a_failed_fingerprint_records_no_audio_hash() {
        let library = Library::new();
        let _ = library.put("Bad.opus", &test_audio::ogg_opus());
        let id = library.walk()["Bad.opus"];
        hash_all(&library);
        fingerprint_counting(&library);
        assert!(matches!(library.outcome(id), Some(Outcome::Failed(_))));
        let (_, from) = recorded(&library, id);
        assert_eq!(from, None);
    }
}
