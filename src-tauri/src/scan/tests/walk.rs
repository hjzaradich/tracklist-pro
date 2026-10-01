#![cfg(test)]
//! 1aA-4: the walk, run as a job on a real (temp) folder tree.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::support::{db, link_dir, TempVolume};
use crate::db::Writer;
use crate::jobs::{self, JobId, JobKind, JobQueue, JobStatus, JobUpdate};
use crate::scan::folders::{add, MusicFolderId};
use crate::scan::walk::{scan_job, ScannedFile, Walker, BATCH_MAX};
use crate::scan::MusicFolderRole;

/// What every scan in these tests is given back.
pub(super) type Batches = Arc<Mutex<Vec<Vec<ScannedFile>>>>;

/// A queue with one worker running the walker over `volume`, sending each
/// batch of new rows to `sent` and every job update to `updates`.
pub(super) fn queue(
    writer: &Writer,
    volume: &TempVolume,
    sent: &Batches,
    updates: &Arc<Mutex<Vec<JobUpdate>>>,
) -> JobQueue {
    let volume = volume.clone();
    let sent = sent.clone();
    let heard = updates.clone();
    JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(move |u: &[JobUpdate]| heard.lock().unwrap().extend_from_slice(u))
        .handler(
            JobKind::Scan,
            Walker::new(
                move || volume.clone(),
                move |files| sent.lock().unwrap().push(files),
            ),
        )
        .start()
        .unwrap()
}

pub(super) fn wait(writer: &Writer, id: JobId) -> JobStatus {
    let start = Instant::now();
    loop {
        let job = writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            return job.status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "the scan never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A scan of every music folder, run to the end. Returns the batches sent.
pub(super) fn scan(writer: &Writer, volume: &TempVolume) -> Vec<Vec<ScannedFile>> {
    let sent = Batches::default();
    let q = queue(writer, volume, &sent, &Arc::default());
    let id = q.enqueue(scan_job(None)).unwrap();
    assert_eq!(wait(writer, id), JobStatus::Done);
    q.shutdown();
    let sent = sent.lock().unwrap().clone();
    sent
}

/// A stored `file` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub id: i64,
    pub folder: i64,
    pub rel_path: String,
    pub rel_path_key: String,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub file_id: Option<String>,
    pub present: bool,
}

pub(super) fn rows(writer: &Writer) -> Vec<Row> {
    writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT id, music_folder_id, rel_path, rel_path_key, size, mtime, file_id,
                        present
                 FROM file ORDER BY rel_path",
            )?;
            let rows = s.query_map([], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    folder: r.get(1)?,
                    rel_path: r.get(2)?,
                    rel_path_key: r.get(3)?,
                    size: r.get(4)?,
                    mtime: r.get(5)?,
                    file_id: r.get(6)?,
                    present: r.get(7)?,
                })
            })?;
            rows.collect()
        })
        .unwrap()
}

pub(super) fn rel_paths(writer: &Writer) -> BTreeSet<String> {
    rows(writer).into_iter().map(|r| r.rel_path).collect()
}

/// `rel` (`/`-separated) under `base`, as a `\\?\` path.
pub(super) fn at(base: &Path, rel: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for part in rel.split('/') {
        path.push(part);
    }
    path
}

/// Writes `bytes` at `rel` under `base`, making its folders.
pub(super) fn put(base: &Path, rel: &str, bytes: &[u8]) {
    let path = at(base, rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
}

const NFC: &str = "Caf\u{e9}.mp3";
const NFD: &str = "Cafe\u{301}.mp3";

/// A folder name 60 characters long, so four of them pass 260.
fn long_name(n: usize) -> String {
    format!("{n} {}", "Long Folder Name ".repeat(4))[..60].to_owned()
}

fn long_file() -> String {
    let dirs: Vec<String> = (1..=5).map(long_name).collect();
    format!("{}/deep track.mp3", dirs.join("/"))
}

/// A small DJ music folder with the awkward cases of `tools/fixture-gen`
/// (stat-only walk: bytes don't need to be real audio). Returns the files
/// the walk must index, by path from the music folder.
fn awkward_tree(music: &Path) -> BTreeSet<String> {
    let indexed = [
        "Artist/Track.mp3".to_owned(),
        "Artist/Track 2.FLAC".to_owned(),
        "Artist/Track 3.Wav".to_owned(),
        // Trailing dot and trailing space, each beside a look-alike.
        "Q.V.X./Dot.mp3".to_owned(),
        "Q.V.X/No Dot.mp3".to_owned(),
        "Drift Unit /Space.aiff".to_owned(),
        "Drift Unit/No Space.aiff".to_owned(),
        // Names Win32 turns into devices without `\\?\`.
        "Devices/CON.mp3".to_owned(),
        "Devices/AUX.mp3".to_owned(),
        // NFC and NFD twins: two files.
        format!("Twins/{NFC}"),
        format!("Twins/{NFD}"),
        "World/\u{1f3a7} Headphones \u{1f469}\u{200d}\u{1f3a4}.mp3".to_owned(),
        "World/\u{97f3}\u{697d} \u{c74c}\u{c545}.m4a".to_owned(),
        "World/# % + & ' %20.opus".to_owned(),
        "World/empty.mp3".to_owned(),
        long_file(),
        "Top level.mp3".to_owned(),
    ];
    for (n, rel) in indexed.iter().enumerate() {
        // Different sizes, so look-alikes can be told apart.
        let bytes = vec![
            b'a';
            if rel.ends_with("empty.mp3") {
                0
            } else {
                100 + n
            }
        ];
        put(music, rel, &bytes);
    }
    for not_audio in [
        "Artist/cover.jpg",
        "Artist/notes.txt",
        "Artist/Mix.cue",
        "Artist/playlist.m3u8",
        "Artist/no_extension",
        "Artist/mp3",
    ] {
        put(music, not_audio, b"not audio");
    }
    // macOS leftovers (§5.5): never music, whatever the extension.
    for leftover in [
        "Artist/._Track.mp3".to_owned(),
        "Artist/.DS_Store".to_owned(),
        "._Top level.mp3".to_owned(),
        ".DS_Store".to_owned(),
        format!("Twins/._{NFC}"),
    ] {
        put(music, &leftover, b"\x00\x05\x16\x07 Mac OS X");
    }
    // A folder named like an audio file is still a folder.
    put(music, "Folder.mp3/Inside.mp3", b"inside");
    let mut all: BTreeSet<String> = indexed.into_iter().collect();
    all.insert("Folder.mp3/Inside.mp3".to_owned());
    // A junction back into the tree is never followed.
    link_dir(&at(music, "Linked"), &at(music, "Artist"));
    all
}

/// A temp drive with one music folder, `Music`, added.
pub(super) fn drive() -> (tempfile::TempDir, TempVolume, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("Music")).unwrap();
    let volume = TempVolume::new(dir.path(), 0x0BAD_F00D);
    let music = volume.mount.join("Music");
    (dir, volume, music)
}

pub(super) fn add_music(writer: &Writer, volume: &TempVolume, folder: &Path) -> MusicFolderId {
    add(writer, volume, folder, MusicFolderRole::Scan)
        .unwrap()
        .id
}

fn nanos(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).unwrap().as_nanos() as i64
}

#[test]
fn the_walk_indexes_every_audio_file_under_its_exact_on_disk_name() {
    let (_dir, volume, music) = drive();
    let expected = awkward_tree(&music);
    let (_db, writer, _reads) = db();
    let folder = add_music(&writer, &volume, &music);

    scan(&writer, &volume);
    assert_eq!(rel_paths(&writer), expected);

    for row in rows(&writer) {
        assert_eq!(row.folder, folder.0);
        assert!(row.present);
        // The stat is the listing's, for the file at exactly that name.
        let meta = fs::metadata(at(&music, &row.rel_path)).unwrap();
        assert_eq!(row.size, Some(meta.len() as i64), "{}", row.rel_path);
        assert_eq!(
            row.mtime,
            Some(nanos(meta.modified().unwrap())),
            "{}",
            row.rel_path
        );
    }
}

#[test]
fn trailing_dot_and_space_folders_keep_their_files_apart_from_look_alikes() {
    let (_dir, volume, music) = drive();
    awkward_tree(&music);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    let by_path: BTreeMap<String, Row> = rows(&writer)
        .into_iter()
        .map(|r| (r.rel_path.clone(), r))
        .collect();
    for (a, b) in [
        ("Q.V.X./Dot.mp3", "Q.V.X/No Dot.mp3"),
        ("Drift Unit /Space.aiff", "Drift Unit/No Space.aiff"),
    ] {
        let (a, b) = (&by_path[a], &by_path[b]);
        assert_ne!(a.size, b.size, "{} read as its look-alike", a.rel_path);
        assert_ne!(a.file_id, b.file_id);
    }
}

#[test]
fn nfc_and_nfd_twins_are_two_rows_that_share_one_match_key() {
    let (_dir, volume, music) = drive();
    awkward_tree(&music);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    let twins: Vec<Row> = rows(&writer)
        .into_iter()
        .filter(|r| r.rel_path.starts_with("Twins/"))
        .collect();
    let spelled: BTreeSet<_> = twins.iter().map(|r| r.rel_path.clone()).collect();
    assert_eq!(
        spelled,
        BTreeSet::from([format!("Twins/{NFC}"), format!("Twins/{NFD}")])
    );
    for twin in &twins {
        assert_eq!(twin.rel_path_key, format!("Twins/{NFC}"));
    }
    assert_ne!(twins[0].size, twins[1].size);
}

#[test]
fn a_file_deeper_than_260_characters_is_indexed() {
    let (_dir, volume, music) = drive();
    awkward_tree(&music);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    let long = long_file();
    assert!(crate::scan::display_path(&at(&music, &long)).len() > 260);
    let row = rows(&writer)
        .into_iter()
        .find(|r| r.rel_path == long)
        .expect("the long path is indexed");
    assert!(row.file_id.is_some());
}

#[test]
fn only_audio_extensions_become_rows_in_any_letter_case() {
    let (_dir, volume, music) = drive();
    awkward_tree(&music);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    let found = rel_paths(&writer);
    for audio in ["Artist/Track 2.FLAC", "Artist/Track 3.Wav"] {
        assert!(found.contains(audio), "{audio}");
    }
    for skipped in [
        "Artist/cover.jpg",
        "Artist/notes.txt",
        "Artist/Mix.cue",
        "Artist/playlist.m3u8",
        "Artist/no_extension",
        "Artist/mp3",
        "Folder.mp3",
    ] {
        assert!(!found.contains(skipped), "{skipped}");
    }
}

#[test]
fn apple_double_and_ds_store_files_never_become_rows() {
    let (_dir, volume, music) = drive();
    let expected = awkward_tree(&music);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    let found = rel_paths(&writer);
    assert_eq!(found, expected);
    for path in &found {
        let name = path.rsplit('/').next().unwrap();
        assert!(
            !name.starts_with("._") && !name.eq_ignore_ascii_case(".DS_Store"),
            "{path}"
        );
    }
    // They're really on disk, beside the files they shadow.
    assert!(at(&music, "Artist/._Track.mp3").is_file());
    assert!(found.contains("Artist/Track.mp3"));
}

#[test]
fn junctions_inside_a_music_folder_are_not_followed() {
    let (_dir, volume, music) = drive();
    awkward_tree(&music);
    // A junction to a folder outside every music folder, too.
    put(&volume.mount, "Elsewhere/Outside.mp3", b"outside");
    link_dir(&at(&music, "Outside link"), &at(&volume.mount, "Elsewhere"));
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    for row in rows(&writer) {
        assert!(
            !row.rel_path.starts_with("Linked/") && !row.rel_path.starts_with("Outside link/"),
            "followed a link: {}",
            row.rel_path
        );
    }
}

#[test]
fn every_file_gets_its_own_file_id_in_the_stored_form() {
    let (_dir, volume, music) = drive();
    awkward_tree(&music);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    let ids: Vec<String> = rows(&writer)
        .into_iter()
        .map(|r| r.file_id.expect("NTFS reports a file id"))
        .collect();
    let distinct: BTreeSet<_> = ids.iter().collect();
    assert_eq!(distinct.len(), ids.len(), "two files share an id: {ids:?}");
    // All on one volume: one serial.
    let serials: BTreeSet<_> = ids.iter().map(|id| &id[..16]).collect();
    assert_eq!(serials.len(), 1, "{ids:?}");
}

#[test]
fn rescanning_is_idempotent_no_duplicate_rows_and_nothing_new_to_send() {
    let (_dir, volume, music) = drive();
    let expected = awkward_tree(&music);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);

    let first_sent = scan(&writer, &volume);
    let first = rows(&writer);
    let sent_ids: BTreeSet<i64> = first_sent.iter().flatten().map(|f| f.id).collect();
    assert_eq!(sent_ids.len(), expected.len());

    let second_sent = scan(&writer, &volume);
    assert!(
        second_sent.is_empty(),
        "a rescan sent rows again: {second_sent:?}"
    );
    assert_eq!(rows(&writer), first, "a rescan changed the rows");
}

#[test]
fn a_rescan_refreshes_a_changed_file_and_adds_a_new_one() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"first");
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);
    let before = rows(&writer);

    // rekordbox rewrites tags in place: same file, new size and mtime.
    let file = fs::File::options()
        .write(true)
        .open(at(&music, "a.mp3"))
        .unwrap();
    file.set_len(12).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    drop(file);
    put(&music, "b.mp3", b"new");

    let sent = scan(&writer, &volume);
    let after = rows(&writer);
    assert_eq!(after.len(), 2);
    let a = &after[0];
    assert_eq!(a.id, before[0].id, "same row");
    assert_eq!(a.size, Some(12));
    assert_ne!(a.mtime, before[0].mtime);
    assert_eq!(a.file_id, before[0].file_id, "same file, same id");
    let new: Vec<&str> = sent.iter().flatten().map(|f| f.rel_path.as_str()).collect();
    assert_eq!(new, ["b.mp3"]);
}

/// `n` small audio files spread over `n / 100` folders.
pub(super) fn many(music: &Path, n: usize) -> BTreeSet<String> {
    (0..n)
        .map(|i| {
            let rel = format!("Genre {:02}/Artist {:03}/Track {i:05}.mp3", i % 7, i / 100);
            put(music, &rel, &i.to_le_bytes());
            rel
        })
        .collect()
}

#[test]
fn new_rows_reach_the_frontend_once_each_in_batches() {
    let (_dir, volume, music) = drive();
    let expected = many(&music, 2 * BATCH_MAX + 500);
    let (_db, writer, _reads) = db();
    let folder = add_music(&writer, &volume, &music);

    let sent = scan(&writer, &volume);
    assert!(sent.len() >= 3, "{} batches", sent.len());
    assert!(sent.iter().all(|b| !b.is_empty() && b.len() <= BATCH_MAX));
    let files: Vec<&ScannedFile> = sent.iter().flatten().collect();
    let paths: BTreeSet<String> = files.iter().map(|f| f.rel_path.clone()).collect();
    assert_eq!(paths.len(), files.len(), "a row was sent twice");
    assert_eq!(paths, expected);

    // What was sent is what was stored.
    let stored: BTreeMap<i64, Row> = rows(&writer).into_iter().map(|r| (r.id, r)).collect();
    for f in files {
        let row = &stored[&f.id];
        assert_eq!(f.music_folder_id, folder);
        assert_eq!(f.rel_path, row.rel_path);
        assert_eq!(Some(f.size), row.size);
        assert_eq!(f.modified_ms, row.mtime.unwrap().div_euclid(1_000_000));
    }
}

#[test]
fn progress_only_rises_and_ends_at_one() {
    let (_dir, volume, music) = drive();
    many(&music, 1500);
    awkward_tree(&music);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);

    let updates = Arc::<Mutex<Vec<JobUpdate>>>::default();
    // Updates within 50 ms of each other coalesce (jobs::dispatch), and a
    // fast walk can finish inside one window, leaving only the final
    // update. So the walk is held at its 500th entry until the listener has
    // heard a progress update short of done: one is then in the list
    // whatever the machine's speed.
    let heard = updates.clone();
    let walk_volume = volume.clone();
    let walker = Walker::new(move || walk_volume.clone(), |_| {}).on_entry(move |entries| {
        if entries == 500 {
            let start = Instant::now();
            while !heard
                .lock()
                .unwrap()
                .iter()
                .any(|u| u.kind == JobKind::Scan && u.progress.is_some_and(|p| p < 1.0))
            {
                assert!(
                    start.elapsed() < Duration::from_secs(30),
                    "no progress update short of done"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    });
    let sink = updates.clone();
    let q = JobQueue::builder(writer.clone())
        .workers(1)
        .on_updates(move |u: &[JobUpdate]| sink.lock().unwrap().extend_from_slice(u))
        .handler(JobKind::Scan, walker)
        .start()
        .unwrap();
    let id = q.enqueue(scan_job(None)).unwrap();
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
    assert!(progress.iter().any(|p| *p < 1.0), "{progress:?}");
    assert!(
        progress.windows(2).all(|w| w[1] >= w[0]),
        "went back: {progress:?}"
    );
    assert_eq!(progress.last(), Some(&1.0));
}

#[test]
fn cancelling_mid_walk_stops_promptly_and_keeps_only_whole_batches() {
    let (_dir, volume, music) = drive();
    let all = many(&music, 3 * BATCH_MAX);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);

    // The first batch's arrival holds the walk until the test has asked
    // to cancel, so the cancel lands mid-walk every time.
    let (arrived, first) = mpsc::channel::<usize>();
    let (resume, cancelled) = mpsc::channel::<()>();
    let (arrived, cancelled) = (Mutex::new(arrived), Mutex::new(cancelled));
    let sent = Batches::default();
    let heard = sent.clone();
    let walker_volume = volume.clone();
    let q = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(
            JobKind::Scan,
            Walker::new(
                move || walker_volume.clone(),
                move |files: Vec<ScannedFile>| {
                    let n = files.len();
                    heard.lock().unwrap().push(files);
                    let _ = arrived.lock().unwrap().send(n);
                    let _ = cancelled
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(30));
                },
            ),
        )
        .start()
        .unwrap();
    let id = q.enqueue(scan_job(None)).unwrap();

    let first_batch = first.recv_timeout(Duration::from_secs(60)).unwrap();
    q.cancel(id).unwrap();
    let asked = Instant::now();
    resume.send(()).unwrap();
    assert_eq!(wait(&writer, id), JobStatus::Cancelled);
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "took {:?} to stop",
        asked.elapsed()
    );
    q.shutdown();

    // Only the batch that was written before the cancel is in the index,
    // whole: every row it announced, and no row it didn't.
    let stored = rel_paths(&writer);
    let announced: BTreeSet<String> = sent
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .map(|f| f.rel_path.clone())
        .collect();
    assert_eq!(announced.len(), first_batch);
    assert_eq!(stored, announced);
    assert!(stored.len() < all.len());

    // A new scan picks up where the index is and finishes it.
    scan(&writer, &volume);
    assert_eq!(rel_paths(&writer), all);
}

#[test]
fn a_scan_of_named_folders_walks_only_those() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"a");
    put(&volume.mount, "Other/b.mp3", b"b");
    let (_db, writer, _reads) = db();
    let music_id = add_music(&writer, &volume, &music);
    add_music(&writer, &volume, &at(&volume.mount, "Other"));

    let sent = Batches::default();
    let q = queue(&writer, &volume, &sent, &Arc::default());
    let id = q.enqueue(scan_job(Some(vec![music_id]))).unwrap();
    assert_eq!(wait(&writer, id), JobStatus::Done);
    assert_eq!(rel_paths(&writer), BTreeSet::from(["a.mp3".to_owned()]));

    // A target that names no folders is refused, not taken as "all".
    let bad = q
        .enqueue(jobs::NewJob::new(JobKind::Scan).target(serde_json::json!({ "folders": 1 })))
        .unwrap();
    assert_eq!(wait(&writer, bad), JobStatus::Failed);
    q.shutdown();
}

#[test]
fn a_music_folder_on_an_unplugged_drive_is_skipped_and_keeps_its_rows() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"a");
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);
    let before = rows(&writer);

    volume.set_online(false);
    let sent = scan(&writer, &volume);
    assert!(sent.is_empty());
    assert_eq!(rows(&writer), before);
}

/// Everything under `dir`: each entry's kind, bytes and modified time. Links
/// are recorded, not followed.
pub(super) fn snapshot(dir: &Path, out: &mut BTreeMap<PathBuf, (String, Vec<u8>, SystemTime)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        let modified = meta.modified().unwrap();
        if meta.file_type().is_symlink() || fs::read_link(&path).is_ok() {
            out.insert(path, ("link".into(), Vec::new(), modified));
        } else if meta.is_dir() {
            snapshot(&path, out);
            out.insert(path, ("folder".into(), Vec::new(), modified));
        } else {
            let bytes = fs::read(&path).unwrap();
            out.insert(path, ("file".into(), bytes, modified));
        }
    }
}

#[test]
fn the_walk_writes_nothing_outside_the_app_data_folder() {
    let (_dir, volume, music) = drive();
    awkward_tree(&music);
    many(&music, 300);
    // Every file's modified time in the past, so any write would move it.
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let mut before = BTreeMap::new();
    snapshot(&volume.mount, &mut before);
    for (path, (kind, _, _)) in &before {
        if kind == "file" {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
    }
    let mut before = BTreeMap::new();
    snapshot(&volume.mount, &mut before);

    let (data, writer, _reads) = db();
    let mut data_before = BTreeMap::new();
    snapshot(data.path(), &mut data_before);
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);
    scan(&writer, &volume);
    assert!(!rows(&writer).is_empty());

    let mut after = BTreeMap::new();
    snapshot(&volume.mount, &mut after);
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .collect();
    assert!(changed.is_empty(), "the walk changed: {changed:?}");

    // The writes it did make went to the database, in the data folder.
    let mut data_after = BTreeMap::new();
    snapshot(data.path(), &mut data_after);
    assert!(data_after.keys().all(|p| p.starts_with(data.path())));
    assert_ne!(data_before, data_after, "the database was written");
}

#[test]
fn timing_smoke_a_few_thousand_files() {
    const FILES: usize = 5000;
    let (_dir, volume, music) = drive();
    many(&music, FILES);
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);

    let start = Instant::now();
    let sent = scan(&writer, &volume);
    let walk = start.elapsed();
    let first = sent.first().map(|b| b.len()).unwrap_or(0);
    assert_eq!(sent.iter().map(Vec::len).sum::<usize>(), FILES);

    // What the file id costs on its own, per file.
    let paths: Vec<PathBuf> = rows(&writer)
        .iter()
        .map(|r| at(&music, &r.rel_path))
        .collect();
    let start = Instant::now();
    for path in &paths {
        crate::scan::file_id::file_id(path).unwrap();
    }
    let ids = start.elapsed();

    let rescan_start = Instant::now();
    scan(&writer, &volume);
    let rescan = rescan_start.elapsed();

    println!(
        "walk: {FILES} files in {walk:?} = {:.0} files/s (first batch {first} files); \
         rescan {rescan:?} = {:.0} files/s; file id alone {:.1} µs/file",
        FILES as f64 / walk.as_secs_f64(),
        FILES as f64 / rescan.as_secs_f64(),
        ids.as_secs_f64() * 1e6 / FILES as f64,
    );
    // A smoke check, not a benchmark (the 100k target is 1aC-7): even a
    // loaded CI runner walks a few thousand files in well under a minute.
    assert!(walk < Duration::from_secs(60), "{walk:?}");
}

/// Runs a scan of `ids` (all folders for `None`) to the end on its own
/// queue with `walker`. Returns its final status.
pub(super) fn scan_with<V: crate::paths::Volumes + 'static>(
    writer: &Writer,
    walker: Walker<V, impl Fn(Vec<ScannedFile>) + Send + Sync + 'static>,
    ids: Option<Vec<MusicFolderId>>,
) -> JobStatus {
    let q = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, walker)
        .start()
        .unwrap();
    let id = q.enqueue(scan_job(ids)).unwrap();
    let status = wait(writer, id);
    q.shutdown();
    status
}

#[test]
fn cancelling_partway_through_a_batch_keeps_exactly_the_batches_already_sent() {
    const FILES: usize = 3 * BATCH_MAX;
    // Mid-way through the second batch.
    const STOP_AT: u64 = (BATCH_MAX + BATCH_MAX / 2) as u64;
    let (_dir, volume, music) = drive();
    // One flat folder: every entry is a file, so entry n is file n.
    for i in 0..FILES {
        put(&music, &format!("Track {i:05}.mp3"), &i.to_le_bytes());
    }
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);

    let (reached, at_stop) = mpsc::channel::<()>();
    let (resume, cancelled) = mpsc::channel::<()>();
    let (reached, cancelled) = (Mutex::new(reached), Mutex::new(cancelled));
    let furthest = Arc::new(AtomicU64::new(0));
    let seen = furthest.clone();
    let sent = Batches::default();
    let heard = sent.clone();
    let walker_volume = volume.clone();
    let walker = Walker::new(
        move || walker_volume.clone(),
        move |files| heard.lock().unwrap().push(files),
    )
    // Only full batches: the test decides where the cancel lands.
    .batch_window(Duration::from_secs(3600))
    .on_entry(move |n| {
        seen.fetch_max(n, Ordering::SeqCst);
        if n == STOP_AT {
            reached.lock().unwrap().send(()).unwrap();
            let _ = cancelled
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(30));
        }
    });
    let q = JobQueue::builder(writer.clone())
        .workers(1)
        .handler(JobKind::Scan, walker)
        .start()
        .unwrap();
    let id = q.enqueue(scan_job(None)).unwrap();

    at_stop.recv_timeout(Duration::from_secs(60)).unwrap();
    let announced: BTreeSet<String> = sent
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .map(|f| f.rel_path.clone())
        .collect();
    q.cancel(id).unwrap();
    resume.send(()).unwrap();
    assert_eq!(wait(&writer, id), JobStatus::Cancelled);
    q.shutdown();

    // The first batch went out whole; the 500 files found since are neither
    // stored nor sent.
    assert_eq!(announced.len(), BATCH_MAX);
    assert_eq!(rel_paths(&writer), announced);
    let sent_after: usize = sent.lock().unwrap().iter().map(Vec::len).sum();
    assert_eq!(sent_after, BATCH_MAX, "a batch went out after the cancel");
    // And it stopped at once: no entry after the one where it was cancelled.
    assert_eq!(furthest.load(Ordering::SeqCst), STOP_AT);
}

#[test]
fn a_rescan_of_one_folder_leaves_a_same_named_file_in_another_folder_alone() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"music");
    put(&volume.mount, "Other/a.mp3", b"other folder");
    let (_db, writer, _reads) = db();
    let music_id = add_music(&writer, &volume, &music);
    let other_id = add_music(&writer, &volume, &at(&volume.mount, "Other"));
    scan(&writer, &volume);
    let row_in = |folder: MusicFolderId| {
        rows(&writer)
            .into_iter()
            .find(|r| r.folder == folder.0)
            .unwrap()
    };
    let other_before = row_in(other_id);

    put(&music, "a.mp3", b"music, retagged and longer");
    let walker_volume = volume.clone();
    let walker = Walker::new(move || walker_volume.clone(), |_| {});
    assert_eq!(
        scan_with(&writer, walker, Some(vec![music_id])),
        JobStatus::Done
    );

    assert_eq!(row_in(music_id).size, Some(26));
    assert_eq!(row_in(other_id), other_before);
}

#[test]
fn files_found_early_go_out_while_the_walk_is_still_going() {
    let (_dir, volume, music) = drive();
    for i in 0..3 {
        put(&music, &format!("A/Track {i}.mp3"), b"audio");
    }
    // Then a long stretch with no audio at all.
    for i in 0..300 {
        put(&music, &format!("Z/{i:03}/notes.txt"), b"not audio");
    }
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);

    let entries = Arc::new(AtomicU64::new(0));
    let counted = entries.clone();
    let at_first_batch = Arc::new(Mutex::new(None::<(usize, u64)>));
    let first = at_first_batch.clone();
    let now = entries.clone();
    let walker_volume = volume.clone();
    let walker = Walker::new(
        move || walker_volume.clone(),
        move |files| {
            let mut first = first.lock().unwrap();
            if first.is_none() {
                *first = Some((files.len(), now.load(Ordering::SeqCst)));
            }
        },
    )
    .on_entry(move |n| {
        counted.store(n + 1, Ordering::SeqCst);
        // Slow enough that the no-audio stretch takes over a second.
        std::thread::sleep(Duration::from_millis(2));
    });
    assert_eq!(scan_with(&writer, walker, None), JobStatus::Done);

    // The walk lists A (3 files), then Z's 300 folders (600 ms, no audio,
    // so nothing is pushed), then each of them. The 3 files go out when
    // the first of those folders is taken up, not when the walk ends.
    let (size, entries_then) = at_first_batch.lock().unwrap().unwrap();
    let total = entries.load(Ordering::SeqCst);
    assert_eq!(size, 3);
    assert!(
        entries_then + 250 < total,
        "the first files waited for the walk to end ({entries_then} of {total} entries)"
    );
}
