//! 1aB-5 and 1aB-6 as a job: walk a real (temp) folder, then hash it.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use super::fixtures::*;
use crate::db::{ReadPool, Writer};
use crate::hash::{hash_job, Hasher, Summary, AUDIO_HASH_LEN, DEFINITION};
use crate::jobs::{self, JobId, JobKind, JobQueue, JobStatus, JobUpdate, Priority};
use crate::paths::Volumes;
use crate::scan::folders::{add, MusicFolderId};
use crate::scan::{scan_job, MusicFolderRole, Walker};
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

// ---- setup ----------------------------------------------------------------

/// A migrated database in a temp dir.
fn db() -> (tempfile::TempDir, Writer, ReadPool) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    let reads = ReadPool::open(writer.guarded_path()).unwrap();
    (dir, writer, reads)
}

/// A made-up volume "mounted" at a temp folder, which can be unplugged.
#[derive(Clone)]
struct TempVolume {
    id: VolumeId,
    mount: PathBuf,
    online: Arc<AtomicBool>,
}

impl TempVolume {
    fn new(mount: &Path) -> TempVolume {
        let id = identity(IdentitySignals {
            kind: VolumeKind::External,
            unc_share: None,
            serial: Some(0x1AB5),
            filesystem: "NTFS",
            guid: None,
        })
        .unwrap();
        TempVolume {
            id,
            mount: fs::canonicalize(mount).unwrap(),
            online: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl Volumes for TempVolume {
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    fn volume_for(&self, path: &Path) -> io::Result<Volume> {
        if !path.starts_with(&self.mount) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "not on the test volume",
            ));
        }
        Ok(Volume {
            id: self.id.clone(),
            label: "TEST".into(),
            mount_path: self.mount.clone(),
            kind: VolumeKind::External,
        })
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (self.online.load(Ordering::SeqCst) && *id == self.id).then(|| self.mount.clone())
    }
}

/// A temp "drive" with a `Music` folder on it.
fn drive() -> (tempfile::TempDir, TempVolume, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let volume = TempVolume::new(dir.path());
    let music = volume.mount.join("Music");
    fs::create_dir(&music).unwrap();
    (dir, volume, music)
}

fn put(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn add_music(writer: &Writer, volume: &TempVolume, folder: &Path) -> MusicFolderId {
    add(writer, volume, folder, MusicFolderRole::Scan)
        .unwrap()
        .id
}

fn wait(writer: &Writer, id: JobId) -> JobStatus {
    let start = Instant::now();
    loop {
        let job = writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            return job.status;
        }
        assert!(start.elapsed() < Duration::from_secs(120), "never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Walks every music folder to the end.
fn walk(writer: &Writer, volume: &TempVolume) {
    let volume = volume.clone();
    let q = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, Walker::new(move || volume.clone(), |_| {}))
        .start()
        .unwrap();
    let id = q.enqueue(scan_job(None)).unwrap();
    assert_eq!(wait(writer, id), JobStatus::Done);
    q.shutdown();
}

fn hasher(volume: &TempVolume) -> Hasher<TempVolume> {
    let volume = volume.clone();
    Hasher::new(move || volume.clone())
}

/// Runs a hash job over `ids` with `hasher` to the end.
fn hash_with(
    writer: &Writer,
    hasher: Hasher<TempVolume>,
    ids: Option<Vec<MusicFolderId>>,
) -> JobStatus {
    let q = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Hash, hasher)
        .start()
        .unwrap();
    let id = q.enqueue(hash_job(ids)).unwrap();
    let status = wait(writer, id);
    q.shutdown();
    status
}

fn hash_all(writer: &Writer, volume: &TempVolume) {
    assert_eq!(hash_with(writer, hasher(volume), None), JobStatus::Done);
}

#[derive(Debug, Clone, PartialEq)]
struct Row {
    id: i64,
    rel_path: String,
    blake3: Option<Vec<u8>>,
    audio_hash: Option<Vec<u8>>,
}

fn rows(writer: &Writer) -> BTreeMap<String, Row> {
    writer
        .call(|c| {
            let mut stmt =
                c.prepare("SELECT id, rel_path, blake3, audio_hash FROM file ORDER BY id")?;
            let rows = stmt.query_map([], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    rel_path: r.get(1)?,
                    blake3: r.get(2)?,
                    audio_hash: r.get(3)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .into_iter()
        .map(|r| (r.rel_path.clone(), r))
        .collect()
}

// ---- tests ----------------------------------------------------------------

#[test]
fn the_hash_job_stores_blake3_and_audio_hash_for_every_file_in_every_format() {
    let (_dir, volume, music) = drive();
    let formats = every_format();
    for (name, bytes) in &formats {
        put(&music, &format!("{name}/track.{}", extension(name)), bytes);
    }
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    hash_all(&writer, &volume);

    let rows = rows(&writer);
    assert_eq!(rows.len(), formats.len());
    for (name, bytes) in &formats {
        let row = &rows[&format!("{name}/track.{}", extension(name))];
        assert_eq!(
            row.blake3.as_deref(),
            Some(&blake3::hash(bytes).as_bytes()[..]),
            "{name}"
        );
        let stored = row
            .audio_hash
            .as_deref()
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(stored.len(), AUDIO_HASH_LEN);
        assert_eq!(stored[0], DEFINITION);
        assert_eq!(stored, &audio(bytes)[..], "{name}");
    }
}

fn extension(name: &str) -> &'static str {
    match name {
        "mp3" => "mp3",
        "adts" => "aac",
        "wav" | "rf64" | "rifx" => "wav",
        "aiff" | "aifc" => "aiff",
        "flac" => "flac",
        "mp4" => "m4a",
        "ogg_vorbis" => "ogg",
        "ogg_opus" => "opus",
        other => panic!("{other}"),
    }
}

#[test]
fn copies_with_different_tags_share_an_audio_hash_on_disk_but_not_a_blake3() {
    let (_dir, volume, music) = drive();
    let frames = mp3_frames();
    let mut names = Vec::new();
    for (i, (_, before)) in leading_tags().into_iter().enumerate() {
        let after = &trailing_tags()[i % trailing_tags().len()].1;
        let name = format!("copy {i}.mp3");
        put(
            &music,
            &name,
            &[before, frames.clone(), after.clone()].concat(),
        );
        names.push(name);
    }
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    hash_all(&writer, &volume);

    let rows = rows(&writer);
    let audio: std::collections::BTreeSet<_> =
        names.iter().map(|n| rows[n].audio_hash.clone()).collect();
    let blake3: std::collections::BTreeSet<_> =
        names.iter().map(|n| rows[n].blake3.clone()).collect();
    assert_eq!(audio.len(), 1, "{audio:?}");
    assert!(audio.iter().all(Option::is_some));
    assert_eq!(blake3.len(), names.len());
}

#[test]
fn a_file_without_recognizable_audio_gets_blake3_and_no_audio_hash() {
    let (_dir, volume, music) = drive();
    put(&music, "not really.mp3", b"a text file with an mp3 name");
    put(&music, "empty.flac", b"");
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    hash_all(&writer, &volume);

    for (name, bytes) in [
        ("not really.mp3", &b"a text file with an mp3 name"[..]),
        ("empty.flac", &b""[..]),
    ] {
        let row = &rows(&writer)[name];
        assert_eq!(
            row.blake3.as_deref(),
            Some(&blake3::hash(bytes).as_bytes()[..])
        );
        assert_eq!(row.audio_hash, None, "{name}: never a whole-file fallback");
    }
}

#[test]
fn a_file_changed_since_the_walk_is_left_for_the_next_walk_and_hash() {
    let (_dir, volume, music) = drive();
    put(&music, "a.wav", &wav_plain());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);

    // Retagged after the walk: the row's size and time are stale.
    let retagged = riff(
        b"RIFF",
        &[
            chunk(b"fmt ", &wav_fmt(), false),
            chunk(b"data", &samples(), false),
            list_info("Retagged"),
        ],
    );
    put(&music, "a.wav", &retagged);
    hash_all(&writer, &volume);
    assert_eq!(rows(&writer)["a.wav"].blake3, None, "hashed a stale row");

    walk(&writer, &volume);
    hash_all(&writer, &volume);
    let row = &rows(&writer)["a.wav"];
    assert_eq!(
        row.blake3.as_deref(),
        Some(&blake3::hash(&retagged).as_bytes()[..])
    );
    assert_eq!(row.audio_hash.as_deref(), Some(&audio(&wav_plain())[..]));
}

#[test]
fn a_second_run_reads_no_file_already_hashed() {
    let (_dir, volume, music) = drive();
    put(&music, "a.flac", &flac_with(&[]));
    put(&music, "b.mp3", &mp3_frames());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    hash_all(&writer, &volume);

    let reads = Arc::new(AtomicU64::new(0));
    let counted = reads.clone();
    let status = hash_with(
        &writer,
        hasher(&volume).on_read(move |_, _| {
            counted.fetch_add(1, Ordering::SeqCst);
        }),
        None,
    );
    assert_eq!(status, JobStatus::Done);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn hash_jobs_run_at_background_priority_on_the_named_folders_only() {
    let job = hash_job(None);
    assert_eq!(job.kind, JobKind::Hash);
    assert_eq!(job.priority, Priority::BACKGROUND);

    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &mp3_frames());
    put(&volume.mount, "Other/b.mp3", &mp3_frames());
    let (_db, writer, _reads) = db();
    let music_id = add_music(&writer, &volume, &music);
    add_music(&writer, &volume, &volume.mount.join("Other"));
    walk(&writer, &volume);
    let status = hash_with(&writer, hasher(&volume), Some(vec![music_id]));
    assert_eq!(status, JobStatus::Done);

    let rows = rows(&writer);
    assert!(rows["a.mp3"].blake3.is_some());
    assert!(rows["b.mp3"].blake3.is_none());
}

/// Runs a hash job over every folder and returns its summary.
fn hash_summary(writer: &Writer, volume: &TempVolume) -> Summary {
    let seen = Arc::new(Mutex::new(None));
    let heard = seen.clone();
    let hasher = hasher(volume).on_summary(move |s| *heard.lock().unwrap() = Some(s));
    assert_eq!(hash_with(writer, hasher, None), JobStatus::Done);
    let summary = seen.lock().unwrap().expect("no summary");
    summary
}

#[test]
fn each_run_counts_what_it_hashed_and_what_it_left_and_why() {
    let (_dir, volume, music) = drive();
    put(&music, "good.mp3", &mp3_frames());
    put(&music, "text.mp3", b"not audio");
    put(&music, "retagged.wav", &wav_plain());
    put(&music, "deleted.flac", &flac_with(&[]));
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    put(
        &music,
        "retagged.wav",
        &[wav_plain(), list_info("x")].concat(),
    );
    fs::remove_file(music.join("deleted.flac")).unwrap();

    let summary = hash_summary(&writer, &volume);
    assert_eq!(
        summary,
        Summary {
            hashed: 1,
            no_audio_hash: 1,
            changed_since_walk: 1,
            unreachable: 1,
            online_only: 0,
            offline: 0,
        }
    );

    // After the next walk the retagged file is hashed; nothing else is due.
    walk(&writer, &volume);
    let summary = hash_summary(&writer, &volume);
    assert_eq!(
        (summary.hashed, summary.changed_since_walk),
        (1, 0),
        "{summary:?}"
    );
}

/// Nanoseconds since the Unix epoch, as the walk stores them.
fn nanos(time: SystemTime) -> i64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

#[test]
fn the_walk_and_the_open_file_agree_on_size_and_mtime() {
    // The walk reads the directory listing; the job reads the open file.
    // For a file nobody is writing they must agree exactly, or nothing
    // would ever be hashed (see job.rs). Checked here on the temp drive
    // (NTFS on Windows), for files written in every way a tagger might.
    let (_dir, _volume, music) = drive();
    put(&music, "new.mp3", &mp3_frames());
    put(&music, "rewritten.mp3", b"short");
    put(
        &music,
        "rewritten.mp3",
        &[mp3_frames(), mp3_frames()].concat(),
    );
    put(&music, "old time.mp3", &mp3_frames());
    fs::File::options()
        .write(true)
        .open(music.join("old time.mp3"))
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_nanos(1_234_567_891_234_567_800))
        .unwrap();
    put(&music, "appended.mp3", &mp3_frames());
    {
        use std::io::Write;
        let mut f = fs::File::options()
            .append(true)
            .open(music.join("appended.mp3"))
            .unwrap();
        f.write_all(&id3v1_tag_bytes()).unwrap();
    }
    for entry in fs::read_dir(&music).unwrap() {
        let entry = entry.unwrap();
        let listed = entry.metadata().unwrap();
        let opened = fs::File::open(entry.path()).unwrap().metadata().unwrap();
        let name = entry.file_name();
        assert_eq!(listed.len(), opened.len(), "{name:?}");
        assert_eq!(
            nanos(listed.modified().unwrap()),
            nanos(opened.modified().unwrap()),
            "{name:?}"
        );
    }
}

fn id3v1_tag_bytes() -> Vec<u8> {
    crate::tags::test_audio::id3v1_tag("Appended")
}

#[test]
fn a_folder_on_an_unplugged_drive_is_skipped_and_stays_unhashed() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &mp3_frames());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    volume.online.store(false, Ordering::SeqCst);
    assert_eq!(hash_summary(&writer, &volume).offline, 1);
    assert_eq!(rows(&writer)["a.mp3"].blake3, None);

    volume.online.store(true, Ordering::SeqCst);
    hash_all(&writer, &volume);
    assert!(rows(&writer)["a.mp3"].blake3.is_some());
}

#[test]
fn cancelling_mid_file_stops_at_once_and_keeps_only_the_files_already_hashed() {
    const FILES: usize = 5;
    const STOP_AT: u64 = 2 * 4096;
    let (_dir, volume, music) = drive();
    let big = [wav_plain(), vec![0; 60_000]].concat();
    for i in 0..FILES {
        put(&music, &format!("track {i}.wav"), &big);
    }
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    let ids: Vec<i64> = {
        let mut ids: Vec<i64> = rows(&writer).values().map(|r| r.id).collect();
        ids.sort();
        ids
    };
    let third = ids[2];

    let (reached, at_stop) = mpsc::channel::<()>();
    let (resume, cancelled) = mpsc::channel::<()>();
    let (reached, cancelled) = (Mutex::new(reached), Mutex::new(cancelled));
    let furthest = Arc::new(AtomicU64::new(0));
    let seen = furthest.clone();
    let hasher = hasher(&volume)
        .buffer(4096)
        // Only the cancel writes: the test decides what's stored.
        .batch_window(Duration::from_secs(3600))
        .on_read(move |file, offset| {
            if file != third {
                return;
            }
            seen.fetch_max(offset, Ordering::SeqCst);
            if offset == STOP_AT {
                reached.lock().unwrap().send(()).unwrap();
                let _ = cancelled
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(30));
            }
        });
    let q = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Hash, hasher)
        .start()
        .unwrap();
    let id = q.enqueue(hash_job(None)).unwrap();

    at_stop.recv_timeout(Duration::from_secs(60)).unwrap();
    q.cancel(id).unwrap();
    let asked = Instant::now();
    resume.send(()).unwrap();
    assert_eq!(wait(&writer, id), JobStatus::Cancelled);
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "{:?}",
        asked.elapsed()
    );
    q.shutdown();

    // Not one more read of the file it was in.
    assert_eq!(furthest.load(Ordering::SeqCst), STOP_AT);
    let hashed: Vec<bool> = {
        let rows = rows(&writer);
        let mut by_id: Vec<_> = rows.values().collect();
        by_id.sort_by_key(|r| r.id);
        by_id.iter().map(|r| r.blake3.is_some()).collect()
    };
    assert_eq!(hashed, [true, true, false, false, false]);

    // The next run finishes the rest.
    hash_all(&writer, &volume);
    assert!(rows(&writer).values().all(|r| r.blake3.is_some()));
}

#[test]
fn progress_only_rises_and_ends_at_one() {
    let (_dir, volume, music) = drive();
    for i in 0..20 {
        put(
            &music,
            &format!("{i}.wav"),
            &[wav_plain(), vec![0; 50_000 * i]].concat(),
        );
    }
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);

    let updates = Arc::new(Mutex::new(Vec::<JobUpdate>::new()));
    let heard = updates.clone();
    let q = JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(move |u: &[JobUpdate]| heard.lock().unwrap().extend_from_slice(u))
        .handler(JobKind::Hash, hasher(&volume).buffer(4096))
        .start()
        .unwrap();
    let id = q.enqueue(hash_job(None)).unwrap();
    assert_eq!(wait(&writer, id), JobStatus::Done);
    // The status is stored before its update reaches the listener; wait for
    // the update that says it's done.
    let start = Instant::now();
    while !updates
        .lock()
        .unwrap()
        .iter()
        .any(|u| u.id == id && u.status == JobStatus::Done)
    {
        assert!(start.elapsed() < Duration::from_secs(30), "no done update");
        std::thread::sleep(Duration::from_millis(5));
    }
    q.shutdown();

    let progress: Vec<f64> = updates
        .lock()
        .unwrap()
        .iter()
        .filter(|u| u.id == id)
        .filter_map(|u| u.progress)
        .collect();
    // Updates within 50 ms are coalesced, so how many arrive varies; that
    // progress moves mid-file is checked on the stored value below.
    assert!(progress.windows(2).all(|w| w[1] >= w[0]), "{progress:?}");
    assert_eq!(progress.last(), Some(&1.0));
}

#[test]
fn progress_moves_while_a_big_file_is_read_not_only_between_files() {
    const HALF: u64 = 1 << 20;
    let (_dir, volume, music) = drive();
    let file = riff(
        b"RIFF",
        &[
            chunk(b"fmt ", &wav_fmt(), false),
            chunk(b"data", &vec![7; 2 * HALF as usize], false),
        ],
    );
    put(&music, "long mix.wav", &file);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);

    // Halfway through the file, what the job's stored progress says.
    let midway = Arc::new(Mutex::new(None::<f64>));
    let seen = midway.clone();
    let reader = writer.clone();
    let hasher = hasher(&volume).buffer(4096).on_read(move |_, offset| {
        if offset == HALF {
            let progress = reader
                .call(|c| {
                    c.query_row("SELECT progress FROM job WHERE kind = 'hash'", [], |r| {
                        r.get::<_, Option<f64>>(0)
                    })
                })
                .unwrap();
            *seen.lock().unwrap() = progress;
        }
    });
    assert_eq!(hash_with(&writer, hasher, None), JobStatus::Done);
    let midway = midway.lock().unwrap().expect("no progress stored mid-file");
    assert!((0.4..0.6).contains(&midway), "{midway}");
}

/// Everything under `dir`: each entry's kind, bytes and modified time.
fn snapshot(dir: &Path, out: &mut BTreeMap<PathBuf, (String, Vec<u8>, SystemTime)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        let modified = meta.modified().unwrap();
        if meta.is_dir() {
            snapshot(&path, out);
            out.insert(path, ("folder".into(), Vec::new(), modified));
        } else {
            let bytes = fs::read(&path).unwrap();
            out.insert(path, ("file".into(), bytes, modified));
        }
    }
}

#[test]
fn the_hash_job_writes_nothing_outside_the_app_data_folder() {
    let (_dir, volume, music) = drive();
    for (name, bytes) in every_format() {
        put(&music, &format!("{name}.{}", extension(name)), &bytes);
    }
    put(&music, "broken.mp3", b"not audio");
    // Every file's modified time in the past, so any write would move it.
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    for entry in fs::read_dir(&music).unwrap() {
        fs::File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let (data, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    let mut before = BTreeMap::new();
    snapshot(&volume.mount, &mut before);
    let mut data_before = BTreeMap::new();
    snapshot(data.path(), &mut data_before);

    hash_all(&writer, &volume);
    assert!(rows(&writer).values().all(|r| r.blake3.is_some()));

    let mut after = BTreeMap::new();
    snapshot(&volume.mount, &mut after);
    assert_eq!(before, after, "the hash job changed the music folder");
    let mut data_after = BTreeMap::new();
    snapshot(data.path(), &mut data_after);
    assert!(data_after.keys().all(|p| p.starts_with(data.path())));
    assert_ne!(data_before, data_after, "the database was written");
}

#[test]
fn throughput_of_the_hash_job_on_disk() {
    const FILES: usize = 24;
    const MB_EACH: usize = 4;
    let (_dir, volume, music) = drive();
    let samples: Vec<u8> = (0..MB_EACH << 20)
        .map(|i| (i * 7 + (i >> 11)) as u8)
        .collect();
    for i in 0..FILES {
        let mut data = samples.clone();
        data[0] = i as u8;
        let file = riff(
            b"RIFF",
            &[
                chunk(b"fmt ", &wav_fmt(), false),
                chunk(b"data", &data, false),
            ],
        );
        put(&music, &format!("{i:02}.wav"), &file);
    }
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);

    let start = Instant::now();
    hash_all(&writer, &volume);
    let elapsed = start.elapsed();
    let mb = (FILES * MB_EACH) as f64;
    let rate = mb / elapsed.as_secs_f64();
    println!(
        "hash job: {FILES} files, {mb} MiB in {elapsed:?} = {rate:.0} MiB/s \
         (files just written, so mostly from the OS cache)"
    );
    assert!(rows(&writer).values().all(|r| r.audio_hash.is_some()));
    assert!(rate > 10.0, "{rate:.0} MiB/s");
}

#[test]
fn a_file_rewritten_in_place_while_it_is_read_is_not_recorded() {
    let (_dir, volume, music) = drive();
    let original = [wav_plain(), vec![0; 60_000]].concat();
    put(&music, "a.wav", &original);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);

    // Halfway through, a tagger rewrites the file in place: same size,
    // other bytes, new modified time.
    let path = music.join("a.wav");
    let rewritten = [wav_plain(), vec![1; 60_000]].concat();
    assert_eq!(rewritten.len(), original.len());
    let seen = Arc::new(Mutex::new(None));
    let heard = seen.clone();
    let hasher = hasher(&volume)
        .buffer(4096)
        .on_read(move |_, offset| {
            if offset == 8 * 4096 {
                fs::write(&path, &rewritten).unwrap();
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000))
                    .unwrap();
            }
        })
        .on_summary(move |s| *heard.lock().unwrap() = Some(s));
    assert_eq!(hash_with(&writer, hasher, None), JobStatus::Done);

    assert_eq!(
        rows(&writer)["a.wav"].blake3,
        None,
        "a torn read was stored"
    );
    let summary = seen.lock().unwrap().expect("no summary");
    assert_eq!(summary.changed_since_walk, 1, "{summary:?}");
    assert_eq!(summary.hashed, 0, "{summary:?}");
}

// ---- file_stage (stage `hash`) ----------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct StageRow {
    version: i64,
    size: Option<i64>,
    mtime: Option<i64>,
    status: String,
    reason: Option<String>,
}

/// Each file's `hash` row in file_stage, by path, and the file row's stat.
fn stage_rows(writer: &Writer) -> BTreeMap<String, (StageRow, Option<i64>, Option<i64>)> {
    writer
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT f.rel_path, s.version, s.size, s.mtime, s.status, s.reason,
                        f.size, f.mtime
                 FROM file_stage s JOIN file f ON f.id = s.file_id
                 WHERE s.stage = 'hash'",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (
                        StageRow {
                            version: r.get(1)?,
                            size: r.get(2)?,
                            mtime: r.get(3)?,
                            status: r.get(4)?,
                            reason: r.get(5)?,
                        },
                        r.get(6)?,
                        r.get(7)?,
                    ),
                ))
            })?;
            rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()
        })
        .unwrap()
}

/// Runs a hash job and counts the buffer reads it made.
fn hash_counting_reads(writer: &Writer, volume: &TempVolume) -> u64 {
    let reads = Arc::new(AtomicU64::new(0));
    let counted = reads.clone();
    let hasher = hasher(volume).on_read(move |_, _| {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(hash_with(writer, hasher, None), JobStatus::Done);
    reads.load(Ordering::SeqCst)
}

#[test]
fn a_hashed_file_is_recorded_done_at_the_definition_version_with_its_stat() {
    let (_dir, volume, music) = drive();
    put(&music, "a.flac", &flac_with(&[]));
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    hash_all(&writer, &volume);

    let (row, size, mtime) = stage_rows(&writer)["a.flac"].clone();
    assert_eq!(row.version, i64::from(DEFINITION));
    assert_eq!((row.status.as_str(), row.reason), ("done", None));
    assert_eq!((row.size, row.mtime), (size, mtime));
}

#[test]
fn a_file_without_audio_is_recorded_failed_with_its_reason_and_not_read_again() {
    let (_dir, volume, music) = drive();
    put(&music, "text.mp3", b"a text file with an mp3 name");
    put(&music, "empty.flac", b"");
    put(&music, "cut.wav", &wav_plain()[..60]);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    hash_all(&writer, &volume);

    let stages = stage_rows(&writer);
    for (name, reason) in [
        ("text.mp3", "unknown_format"),
        ("empty.flac", "empty"),
        ("cut.wav", "truncated"),
    ] {
        let row = &stages[name].0;
        assert_eq!(row.status, "failed", "{name}");
        assert_eq!(row.reason.as_deref(), Some(reason), "{name}");
        // It has its blake3 all the same.
        assert!(rows(&writer)[name].blake3.is_some(), "{name}");
    }
    assert_eq!(
        hash_counting_reads(&writer, &volume),
        0,
        "a failed file was read again"
    );
}

#[test]
fn a_file_retagged_since_it_was_hashed_is_hashed_again_and_keeps_its_audio_hash() {
    let (_dir, volume, music) = drive();
    put(&music, "a.wav", &wav_plain());
    put(&music, "b.wav", &wav_plain());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    hash_all(&writer, &volume);
    let before = rows(&writer);

    // a.wav retagged (a new size); b.wav only touched, as rekordbox bumps
    // modified times (§5.1).
    let retagged = riff(
        b"RIFF",
        &[
            chunk(b"fmt ", &wav_fmt(), false),
            chunk(b"data", &samples(), false),
            list_info("Retagged"),
        ],
    );
    put(&music, "a.wav", &retagged);
    fs::File::options()
        .write(true)
        .open(music.join("b.wav"))
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000))
        .unwrap();
    walk(&writer, &volume);
    assert!(
        hash_counting_reads(&writer, &volume) > 0,
        "nothing was hashed again"
    );

    let after = rows(&writer);
    assert_eq!(
        after["a.wav"].blake3.as_deref(),
        Some(&blake3::hash(&retagged).as_bytes()[..])
    );
    assert_ne!(after["a.wav"].blake3, before["a.wav"].blake3);
    assert_eq!(after["a.wav"].audio_hash, before["a.wav"].audio_hash);
    assert_eq!(after["b.wav"], before["b.wav"]);
    let stages = stage_rows(&writer);
    for name in ["a.wav", "b.wav"] {
        let (row, size, mtime) = &stages[name];
        assert_eq!(
            (row.size, row.mtime),
            (*size, *mtime),
            "{name}: recorded the new stat"
        );
    }
    assert_eq!(hash_counting_reads(&writer, &volume), 0);
}

#[test]
fn a_new_audio_hash_definition_hashes_every_file_again() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &mp3_frames());
    put(&music, "text.mp3", b"not audio");
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    hash_all(&writer, &volume);
    assert_eq!(hash_counting_reads(&writer, &volume), 0);

    // As if these rows were written by another definition.
    writer
        .call(|c| {
            c.execute(
                "UPDATE file_stage SET version = 99 WHERE stage = 'hash'",
                [],
            )
        })
        .unwrap();
    assert!(hash_counting_reads(&writer, &volume) >= 2);
    let stages = stage_rows(&writer);
    assert!(stages
        .values()
        .all(|(r, _, _)| r.version == i64::from(DEFINITION)));
}

#[test]
fn an_unreachable_file_is_recorded_as_skipped_and_stays_due() {
    let (_dir, volume, music) = drive();
    put(&music, "gone.mp3", &mp3_frames());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    fs::remove_file(music.join("gone.mp3")).unwrap();
    hash_all(&writer, &volume);

    let row = &stage_rows(&writer)["gone.mp3"].0;
    assert_eq!(row.status, "skipped");
    assert_eq!(row.reason.as_deref(), Some(crate::scan_state::UNREACHABLE));
    let due = writer
        .call(|c| {
            use crate::scan_state::{count_due, Scope, Stage};
            count_due(c, Stage::Hash, i64::from(DEFINITION), &Scope::All)
        })
        .unwrap();
    assert_eq!(due, 1, "an unreachable file must be tried again");
}

#[test]
fn a_file_changed_since_the_walk_or_on_an_unplugged_drive_gets_no_stage_row() {
    let (_dir, volume, music) = drive();
    put(&music, "a.wav", &wav_plain());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    put(&music, "a.wav", &[wav_plain(), list_info("x")].concat());
    hash_all(&writer, &volume);
    volume.online.store(false, Ordering::SeqCst);
    hash_all(&writer, &volume);
    assert!(stage_rows(&writer).is_empty());
}

#[test]
fn an_online_only_file_is_skipped_without_opening_until_the_user_opts_in() {
    let (_dir, volume, music) = drive();
    put(&music, "local.mp3", &mp3_frames());
    put(&music, "cloud.mp3", &mp3_frames());
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    walk(&writer, &volume);
    // As the walk marks a OneDrive placeholder (1aB-8).
    writer
        .call(|c| {
            c.execute(
                "UPDATE file SET online_only = 1 WHERE rel_path = 'cloud.mp3'",
                [],
            )
        })
        .unwrap();
    let cloud = rows(&writer)["cloud.mp3"].id;

    let opened = Arc::new(AtomicBool::new(false));
    let seen = opened.clone();
    let summary = Arc::new(Mutex::new(None));
    let heard = summary.clone();
    let hasher = hasher(&volume)
        .on_read(move |file, _| {
            if file == cloud {
                seen.store(true, Ordering::SeqCst);
            }
        })
        .on_summary(move |s| *heard.lock().unwrap() = Some(s));
    assert_eq!(hash_with(&writer, hasher, None), JobStatus::Done);
    assert!(
        !opened.load(Ordering::SeqCst),
        "an online-only file was read"
    );
    assert_eq!(summary.lock().unwrap().unwrap().online_only, 1);
    assert!(rows(&writer)["local.mp3"].blake3.is_some());
    assert_eq!(rows(&writer)["cloud.mp3"].blake3, None);
    let row = &stage_rows(&writer)["cloud.mp3"].0;
    assert_eq!(row.status, "skipped");
    assert_eq!(row.reason.as_deref(), Some(crate::scan_state::ONLINE_ONLY));

    // Opted in: the next run reads it.
    writer
        .call(|c| {
            c.execute(
                "INSERT INTO setting (key, value) VALUES (?1, json('true'))",
                [crate::scan::online_only::READ_ONLINE_ONLY_FILES],
            )
        })
        .unwrap();
    hash_all(&writer, &volume);
    assert_eq!(
        rows(&writer)["cloud.mp3"].audio_hash,
        rows(&writer)["local.mp3"].audio_hash
    );
    assert_eq!(stage_rows(&writer)["cloud.mp3"].0.status, "done");
}
