//! The read job, run through the job queue after a real walk over temp
//! folders on a made-up volume.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::db::{Writer, DB_FILE_NAME};
use crate::fingerprint::FirstUp;
use crate::jobs::{self, JobId, JobKind, JobQueue, JobStatus, JobUpdate};
use crate::paths::Volumes;
use crate::read::{read_job, Reader, READ_VERSION};
use crate::scan::folders::{self, MusicFolderId};
use crate::scan::online_only::READ_ONLINE_ONLY_FILES;
use crate::scan::{scan_job, MusicFolderRole, Walker};
use crate::scan_state::{self, Scope, Stage};
use crate::tags::test_audio::{self as audio, id3_text, id3v2, Format};
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_OFFLINE, INVALID_FILE_ATTRIBUTES,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_NORMAL,
};

/// One volume "mounted" at a temp folder, which can be unplugged.
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
            serial: Some(0x1AB2),
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

/// A sandbox: the app data folder (the database) and a "drive" holding
/// the music, side by side in one temp dir.
struct Sandbox {
    root: tempfile::TempDir,
    writer: Writer,
    volume: TempVolume,
}

impl Sandbox {
    fn new() -> Sandbox {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("app-data");
        let drive = root.path().join("drive");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&drive).unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(&data, DB_FILE_NAME)).unwrap();
        let volume = TempVolume::new(&drive);
        Sandbox {
            root,
            writer,
            volume,
        }
    }

    fn drive(&self) -> PathBuf {
        self.root.path().join("drive")
    }

    /// Adds `name` (created if needed) under the drive as a music folder.
    fn folder(&self, name: &str) -> (MusicFolderId, PathBuf) {
        let path = self.drive().join(name);
        fs::create_dir_all(&path).unwrap();
        let folder =
            folders::add(&self.writer, &self.volume, &path, MusicFolderRole::Scan).unwrap();
        (folder.id, path)
    }

    /// A queue with one worker running the walk and `reader`.
    fn queue(&self, reader: Reader<TempVolume>) -> JobQueue {
        let volume = self.volume.clone();
        JobQueue::builder(self.writer.clone())
            .workers(1)
            .handler(JobKind::Scan, Walker::new(move || volume.clone(), |_| {}))
            .handler(JobKind::Read, reader)
            .start()
            .unwrap()
    }

    /// A reader on one thread, so a test of batches, windows and cancels
    /// can say exactly which files were written. [`Sandbox::parallel`]
    /// is the app's shape.
    fn reader(&self) -> Reader<TempVolume> {
        self.parallel().threads(1)
    }

    /// A reader on the app's budget: four threads at most.
    fn parallel(&self) -> Reader<TempVolume> {
        let volume = self.volume.clone();
        Reader::new(move || volume.clone()).threads(4)
    }

    /// Writes the same mixed bag of files under `name`: every format,
    /// tagged MP3s, a broken tag, an empty file, a cut WAV, and one file
    /// that's gone by the time it's read. Returns the folder.
    fn mixed_bag(&self, name: &str) -> PathBuf {
        let (_, dir) = self.folder(name);
        for format in Format::ALL {
            format.write_to(&dir, "tone");
        }
        for n in 0..40 {
            put(
                &dir,
                &format!("tagged {n:02}.mp3"),
                &tagged_mp3(&format!("Track {n}")),
            );
        }
        let tag = id3v2(
            &[
                id3_text(b"TIT2", "Survivor"),
                audio::id3_frame(b"TPE1", &[9, 0xFF, 0xFE, 0x00]),
            ],
            0,
        );
        put(&dir, "broken tag.mp3", &[tag, audio::mp3()].concat());
        put(&dir, "empty.mp3", &[]);
        let wav = audio::wav();
        put(&dir, "cut.wav", &wav[..wav.len() - 1000]);
        put(&dir, "Fake.wav", &tagged_mp3("Actually MP3"));
        put(&dir, "gone.mp3", &audio::mp3());
        dir
    }

    /// Walks every music folder.
    fn walk(&self) {
        let q = self.queue(self.reader());
        let id = q.enqueue(scan_job(None)).unwrap();
        assert_eq!(wait(&self.writer, id), JobStatus::Done);
        q.shutdown();
    }

    /// Runs one read job of `ids` with `reader` to the end. Returns how
    /// many files it looked at, and how it ended.
    fn read_with(
        &self,
        reader: Reader<TempVolume>,
        ids: Option<Vec<MusicFolderId>>,
    ) -> (u64, JobStatus) {
        let looked = Arc::new(AtomicU64::new(0));
        let seen = looked.clone();
        let q = self.queue(reader.on_file(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        }));
        let id = q.enqueue(read_job(ids)).unwrap();
        let status = wait(&self.writer, id);
        q.shutdown();
        (looked.load(Ordering::SeqCst), status)
    }

    /// Runs a read of every music folder. Returns how many files it looked at.
    fn read(&self) -> u64 {
        let (looked, status) = self.read_with(self.reader(), None);
        assert_eq!(status, JobStatus::Done);
        looked
    }

    /// Every file row, by path.
    fn rows(&self) -> BTreeMap<String, Row> {
        self.writer
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT f.rel_path, f.sniffed_format, f.codec, f.bitrate, f.sample_rate,
                            f.duration_ms, f.quality_verdict, f.raw_tags,
                            s.status, s.reason
                     FROM file f LEFT JOIN file_stage s ON s.file_id = f.id AND s.stage = 'read'",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        Row {
                            sniffed_format: r.get(1)?,
                            codec: r.get(2)?,
                            bitrate: r.get(3)?,
                            sample_rate: r.get(4)?,
                            duration_ms: r.get(5)?,
                            verdict: r.get(6)?,
                            raw_tags: r.get(7)?,
                            stage: r.get(8)?,
                            reason: r.get(9)?,
                        },
                    ))
                })?;
                rows.collect()
            })
            .unwrap()
    }

    fn row(&self, rel: &str) -> Row {
        self.rows()
            .remove(rel)
            .unwrap_or_else(|| panic!("no row for {rel}"))
    }
}

/// A queue and the job running on it, for a hook that cancels it.
type RunningJob = (Arc<JobQueue>, JobId);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    sniffed_format: Option<String>,
    codec: Option<String>,
    bitrate: Option<i64>,
    sample_rate: Option<i64>,
    duration_ms: Option<i64>,
    verdict: Option<String>,
    raw_tags: Option<String>,
    /// file_stage status for 'read', if any.
    stage: Option<String>,
    reason: Option<String>,
}

/// How many files the read stage has recorded so far.
fn recorded(writer: &Writer) -> i64 {
    writer
        .call(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM file_stage WHERE stage = 'read'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap()
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
        assert!(
            start.elapsed() < Duration::from_secs(300),
            "the job never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn put(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn tagged_mp3(title: &str) -> Vec<u8> {
    [id3v2(&[id3_text(b"TIT2", title)], 0), audio::mp3()].concat()
}

/// A file's mtime moved on, as rekordbox does when it rewrites tags.
fn touch_later(path: &Path) {
    let later = SystemTime::now() + Duration::from_secs(60);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(later)
        .unwrap();
}

#[test]
fn a_read_fills_every_walked_files_format_codec_properties_tags_and_verdict() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    for format in Format::ALL {
        format.write_to(&dir, "tone");
    }
    put(&dir, "Fake.wav", &tagged_mp3("Actually MP3"));
    put(&dir, "empty.mp3", &[]);
    s.walk();
    assert_eq!(s.read(), 9);

    let rows = s.rows();
    let fake = &rows["Fake.wav"];
    assert_eq!(fake.sniffed_format.as_deref(), Some("mp3"));
    assert_eq!(fake.codec.as_deref(), Some("mp3"));
    assert_eq!(fake.bitrate, Some(128));
    assert_eq!(fake.sample_rate, Some(44_100));
    assert!(fake.duration_ms.unwrap() > 0);
    assert_eq!(fake.verdict, None);
    let tags: serde_json::Value = serde_json::from_str(fake.raw_tags.as_deref().unwrap()).unwrap();
    assert_eq!(tags["id3v2"][0]["value"]["text"], "Actually MP3");

    let empty = &rows["empty.mp3"];
    assert_eq!(empty.verdict.as_deref(), Some("broken"));
    assert_eq!(empty.stage.as_deref(), Some("done"));

    for format in Format::ALL {
        let row = &rows[&format!("tone.{}", format.extension())];
        assert_eq!(row.stage.as_deref(), Some("done"), "{format:?}");
        assert!(
            super::files::untagged(row.raw_tags.as_deref()),
            "{format:?}: {row:?}"
        );
        assert!(row.codec.is_some(), "{format:?}: {row:?}");
        assert_eq!(row.verdict, None, "{format:?}: {row:?}");
    }
}

#[test]
fn a_file_with_a_broken_id3_block_still_gets_indexed_with_its_properties() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    let tag = id3v2(
        &[
            id3_text(b"TIT2", "Survivor"),
            audio::id3_frame(b"TPE1", &[9, 0xFF, 0xFE, 0x00]),
        ],
        0,
    );
    put(&dir, "broken tag.mp3", &[tag, audio::mp3()].concat());
    s.walk();
    s.read();
    let row = s.row("broken tag.mp3");
    assert_eq!(row.stage.as_deref(), Some("done"));
    assert_eq!(row.codec.as_deref(), Some("mp3"));
    assert_eq!(row.sample_rate, Some(44_100));
    assert_eq!(row.verdict, None);
    assert!(row.raw_tags.unwrap().contains("Survivor"));
}

#[test]
fn truncated_zero_byte_and_no_moov_files_get_the_right_verdict() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    let wav = audio::wav();
    put(&dir, "cut.wav", &wav[..wav.len() - 1000]);
    put(&dir, "zero.flac", &[]);
    let m4a = audio::m4a();
    let moov = m4a.windows(4).position(|w| w == b"moov").unwrap() - 4;
    let moov_len = u32::from_be_bytes(m4a[moov..moov + 4].try_into().unwrap()) as usize;
    put(
        &dir,
        "no moov.m4a",
        &[&m4a[..moov], &m4a[moov + moov_len..]].concat(),
    );
    put(&dir, "junk.mp3", &b"not audio at all ".repeat(100));
    put(&dir, "fine.mp3", &audio::mp3());
    s.walk();
    s.read();
    let verdicts: BTreeMap<_, _> = s
        .rows()
        .into_iter()
        .map(|(path, row)| (path, row.verdict))
        .collect();
    let expected: BTreeMap<String, Option<String>> = [
        ("cut.wav", Some("truncated")),
        ("zero.flac", Some("broken")),
        ("no moov.m4a", Some("broken")),
        ("junk.mp3", Some("broken")),
        ("fine.mp3", None),
    ]
    .into_iter()
    .map(|(p, v)| (p.to_owned(), v.map(str::to_owned)))
    .collect();
    assert_eq!(verdicts, expected);
}

#[test]
fn a_rerun_skips_unchanged_files_and_rereads_a_changed_one() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    for n in 0..5 {
        put(&dir, &format!("{n}.mp3"), &tagged_mp3(&format!("Take {n}")));
    }
    s.walk();
    assert_eq!(s.read(), 5);
    assert_eq!(s.read(), 0, "nothing changed, so nothing is read");

    // rekordbox rewrites a tag: new bytes, new size and mtime.
    let changed = put(&dir, "3.mp3", &tagged_mp3("Retitled in rekordbox"));
    touch_later(&changed);
    s.walk();
    assert_eq!(s.read(), 1);
    assert!(s
        .row("3.mp3")
        .raw_tags
        .unwrap()
        .contains("Retitled in rekordbox"));
    assert!(s.row("2.mp3").raw_tags.unwrap().contains("Take 2"));

    // Only the mtime moved: still reread, since the tags may have changed.
    touch_later(&dir.join("1.mp3"));
    s.walk();
    assert_eq!(s.read(), 1);
}

#[test]
fn a_fixed_file_loses_its_broken_verdict_but_1bs_verdicts_are_left_alone() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    let broken = put(&dir, "redownloaded.mp3", &[]);
    let wav = audio::wav();
    let cut = put(&dir, "recopied.wav", &wav[..wav.len() - 1000]);
    let judged = put(&dir, "judged.mp3", &tagged_mp3("Low"));
    s.walk();
    s.read();
    assert_eq!(s.row("redownloaded.mp3").verdict.as_deref(), Some("broken"));
    assert_eq!(s.row("recopied.wav").verdict.as_deref(), Some("truncated"));
    // 1b's quality check judged this one.
    s.writer
        .call(|c| {
            c.execute(
                "UPDATE file SET quality_verdict = 'low_bitrate' WHERE rel_path = 'judged.mp3'",
                [],
            )
        })
        .unwrap();

    fs::write(&broken, tagged_mp3("Whole now")).unwrap();
    fs::write(&cut, &wav).unwrap();
    fs::write(&judged, tagged_mp3("Low, retagged")).unwrap();
    touch_later(&judged);
    s.walk();
    assert_eq!(s.read(), 3);
    assert_eq!(s.row("redownloaded.mp3").verdict, None);
    assert_eq!(s.row("recopied.wav").verdict, None);
    assert_eq!(s.row("judged.mp3").verdict.as_deref(), Some("low_bitrate"));
}

#[test]
fn a_file_found_broken_loses_the_verdict_1b_gave_it() {
    // Broken outranks any quality judgement: there's no audio to judge.
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    let path = put(&dir, "a.mp3", &tagged_mp3("Was fine"));
    s.walk();
    s.read();
    s.writer
        .call(|c| c.execute("UPDATE file SET quality_verdict = 'low_bitrate'", []))
        .unwrap();
    fs::write(&path, []).unwrap();
    s.walk();
    assert_eq!(s.read(), 1);
    assert_eq!(s.row("a.mp3").verdict.as_deref(), Some("broken"));
}

#[test]
fn on_a_slow_drive_a_batch_is_written_once_its_first_file_has_waited_long_enough() {
    // Batches of 1000 would never fill here; only the time window can write
    // one. Each file takes 40 ms, the window is 100 ms, and the job is
    // cancelled before its 8th file: whatever was written, the window did.
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    for n in 0..10 {
        put(&dir, &format!("{n:02}.mp3"), &audio::mp3());
    }
    s.walk();
    let queue_slot: Arc<Mutex<Option<RunningJob>>> = Arc::default();
    let slot = queue_slot.clone();
    let reader = s
        .reader()
        .batches(1000, Duration::from_millis(100))
        .on_file(move |n| {
            std::thread::sleep(Duration::from_millis(40));
            if n == 7 {
                let (q, id) = loop {
                    if let Some(found) = slot.lock().unwrap().clone() {
                        break found;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                };
                q.cancel(id).unwrap();
            }
        });
    let q = Arc::new(s.queue(reader));
    let id = q.enqueue(read_job(None)).unwrap();
    *queue_slot.lock().unwrap() = Some((q.clone(), id));
    assert_eq!(wait(&s.writer, id), JobStatus::Cancelled);
    q.shutdown();
    let written = s.rows().values().filter(|r| r.stage.is_some()).count();
    assert!(
        (3..=7).contains(&written),
        "{written} files written before the cancel"
    );
}

#[test]
fn a_file_gone_since_the_walk_is_skipped_as_unreachable_and_tried_again_next_run() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    let gone = put(&dir, "gone.mp3", &audio::mp3());
    put(&dir, "here.mp3", &audio::mp3());
    s.walk();
    fs::remove_file(&gone).unwrap();
    assert_eq!(s.read(), 2);
    let row = s.row("gone.mp3");
    assert_eq!(row.stage.as_deref(), Some("skipped"));
    assert_eq!(row.reason.as_deref(), Some("unreachable"));
    // Its columns weren't touched: nothing was read.
    assert_eq!(
        (row.sniffed_format, row.codec, row.verdict),
        (None, None, None)
    );
    assert_eq!(s.row("here.mp3").stage.as_deref(), Some("done"));
    // Still unreachable: tried again, still skipped, and the run ends.
    assert_eq!(s.read(), 1);

    // It comes back with the same bytes, so its row's size is unchanged
    // and no walk has run: it's read anyway.
    put(&dir, "gone.mp3", &audio::mp3());
    assert_eq!(s.read(), 1);
    assert_eq!(s.row("gone.mp3").stage.as_deref(), Some("done"));
    assert_eq!(s.row("gone.mp3").codec.as_deref(), Some("mp3"));
}

#[test]
fn cancelling_mid_batch_keeps_whole_batches_and_the_rest_is_read_next_time() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    for n in 0..10 {
        put(&dir, &format!("{n:02}.mp3"), &audio::mp3());
    }
    s.walk();

    // Batches of 3; cancel before the 8th file is read, once the second
    // batch (files 3 to 5) is committed: file 6 is then in an unwritten
    // batch. The reader hands results to the writer, which commits them
    // in its own time, so the hook waits for that commit before it
    // cancels; otherwise where the cancel lands would depend on speed.
    let queue_slot: Arc<Mutex<Option<RunningJob>>> = Arc::default();
    let slot = queue_slot.clone();
    let writer = s.writer.clone();
    let reader = s
        .reader()
        .batches(3, Duration::from_secs(3600))
        .on_file(move |n| {
            if n == 7 {
                let start = Instant::now();
                while recorded(&writer) < 6 {
                    assert!(
                        start.elapsed() < Duration::from_secs(30),
                        "batch 2 never committed"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                // The job may start before the test has stored its id.
                let (q, id) = loop {
                    if let Some(found) = slot.lock().unwrap().clone() {
                        break found;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                };
                q.cancel(id).unwrap();
            }
        });
    let q = Arc::new(s.queue(reader));
    let id = q.enqueue(read_job(None)).unwrap();
    *queue_slot.lock().unwrap() = Some((q.clone(), id));
    assert_eq!(wait(&s.writer, id), JobStatus::Cancelled);
    q.shutdown();

    let rows = s.rows();
    let read: Vec<_> = rows
        .iter()
        .filter(|(_, r)| r.stage.is_some())
        .map(|(p, _)| p.as_str())
        .collect();
    assert_eq!(
        read,
        ["00.mp3", "01.mp3", "02.mp3", "03.mp3", "04.mp3", "05.mp3"]
    );
    // A file's columns and its stage row were written together.
    for (path, row) in &rows {
        assert_eq!(row.stage.is_some(), row.codec.is_some(), "{path}");
    }
    assert_eq!(s.read(), 4);
    assert!(s
        .rows()
        .values()
        .all(|r| r.stage.as_deref() == Some("done")));
}

#[test]
fn a_parallel_read_writes_exactly_what_a_serial_read_writes() {
    // The same files in two sandboxes: one read on one thread, one on
    // four. Every row must come out the same, gone, locked and online-only
    // files included.
    let serial = Sandbox::new();
    let parallel = Sandbox::new();
    let mut readers = Vec::new();
    for (s, reader) in [(&serial, serial.reader()), (&parallel, parallel.parallel())] {
        let dir = s.mixed_bag("Music");
        // Six the walk sees online-only, so borrowed threads meet some too;
        // one the walk sees online-only that's local again by read time
        // (still skipped: the walk's mark decides); and one that went
        // online-only after the walk (the check before the open decides).
        let clouds: Vec<_> = (0..6)
            .map(|n| put(&dir, &format!("cloud {n}.mp3"), &tagged_mp3("In the cloud")))
            .collect();
        let back = put(&dir, "back.mp3", &tagged_mp3("Back on disk"));
        let freed = put(&dir, "freed.mp3", &tagged_mp3("Freed up"));
        for path in clouds.iter().chain([&back]) {
            set_online_only(path, true);
        }
        s.walk();
        set_online_only(&back, false);
        set_online_only(&freed, true);
        fs::remove_file(dir.join("gone.mp3")).unwrap();
        let locks = [
            lock_against_reading(&dir.join("tagged 07.mp3")),
            lock_against_reading(&freed),
        ];
        let mut placeholders = clouds;
        placeholders.push(freed);
        readers.push((s, reader, locks, placeholders));
    }
    let mut results = Vec::new();
    for (s, reader, locks, placeholders) in readers {
        let (looked, status) = s.read_with(reader, None);
        assert_eq!(status, JobStatus::Done);
        drop(locks);
        for path in placeholders {
            set_online_only(&path, false);
        }
        results.push((looked, s.rows()));
    }
    let (serial_looked, serial_rows) = &results[0];
    let (parallel_looked, parallel_rows) = &results[1];
    assert_eq!(parallel_looked, serial_looked);
    assert_eq!(*serial_looked as usize, serial_rows.len());
    assert_eq!(parallel_rows.len(), serial_rows.len());
    for (path, row) in serial_rows {
        assert_eq!(parallel_rows.get(path), Some(row), "{path}");
    }
    // The bag had every case in it.
    let reason = |path: &str| serial_rows[path].reason.clone();
    assert_eq!(reason("gone.mp3").as_deref(), Some("unreachable"));
    assert_eq!(reason("tagged 07.mp3").as_deref(), Some("unreachable"));
    for n in 0..6 {
        assert_eq!(
            reason(&format!("cloud {n}.mp3")).as_deref(),
            Some("online_only")
        );
    }
    assert_eq!(reason("back.mp3").as_deref(), Some("online_only"));
    assert_eq!(reason("freed.mp3").as_deref(), Some("online_only"));
    assert_eq!(serial_rows["empty.mp3"].verdict.as_deref(), Some("broken"));
    assert_eq!(serial_rows["cut.wav"].verdict.as_deref(), Some("truncated"));
    assert!(serial_rows["broken tag.mp3"]
        .raw_tags
        .as_deref()
        .unwrap()
        .contains("Survivor"));
    assert_eq!(
        serial_rows["Fake.wav"].sniffed_format.as_deref(),
        Some("mp3")
    );
}

#[test]
fn a_parallel_read_borrows_free_threads_from_the_shared_budget_and_gives_them_back() {
    let s = Sandbox::new();
    s.mixed_bag("Music");
    s.walk();
    let files = s.rows().len() as u64;
    let first = FirstUp::default();

    /// Runs `reader` on the queue, noting each thread that reads, with its
    /// Windows priority, and the most threads the line had busy while it
    /// ran. Each reader waits (bounded) until `expected` distinct threads
    /// have shown up, so the count doesn't hang on the scheduler. Returns
    /// (files looked at, priority by thread, most busy).
    fn read_noting(
        s: &Sandbox,
        reader: Reader<TempVolume>,
        first: &FirstUp,
        expected: usize,
    ) -> (
        u64,
        std::collections::HashMap<std::thread::ThreadId, i32>,
        u64,
    ) {
        let looked = Arc::new(AtomicU64::new(0));
        let threads = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let most_busy = Arc::new(AtomicU64::new(0));
        let (count, seen, peak, line) = (
            looked.clone(),
            threads.clone(),
            most_busy.clone(),
            first.clone(),
        );
        let q = s.queue(reader.on_file(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            // SAFETY: the pseudo-handle for this thread is always valid.
            let priority = unsafe { GetThreadPriority(GetCurrentThread()) };
            seen.lock()
                .unwrap()
                .insert(std::thread::current().id(), priority);
            peak.fetch_max(line.busy() as u64, Ordering::SeqCst);
            let start = Instant::now();
            while seen.lock().unwrap().len() < expected && start.elapsed() < Duration::from_secs(5)
            {
                std::thread::sleep(Duration::from_millis(1));
            }
        }));
        let id = q.enqueue(read_job(None)).unwrap();
        assert_eq!(wait(&s.writer, id), JobStatus::Done);
        q.shutdown();
        let threads = threads.lock().unwrap().clone();
        (
            looked.load(Ordering::SeqCst),
            threads,
            most_busy.load(Ordering::SeqCst),
        )
    }

    // Two of the four are busy fingerprinting: the read gets its own
    // thread plus the two that are free, and no more. Its own keeps normal
    // priority; the borrowed ones run below normal, as fingerprinting does.
    let busy = first.try_threads(2, 4).unwrap();
    assert_eq!(busy.n, 2);
    let (looked, threads, most_busy) =
        read_noting(&s, s.parallel().sharing(first.clone()), &first, 3);
    assert_eq!(looked, files);
    assert_eq!(most_busy, 4, "the two free threads were borrowed");
    let mut priorities: Vec<i32> = threads.values().copied().collect();
    priorities.sort();
    assert_eq!(
        priorities,
        [
            THREAD_PRIORITY_BELOW_NORMAL,
            THREAD_PRIORITY_BELOW_NORMAL,
            THREAD_PRIORITY_NORMAL
        ],
        "its own thread and the two borrowed: {threads:?}"
    );
    assert_eq!(first.busy(), 2, "the borrowed threads went back");
    drop(busy);
    assert_eq!(first.busy(), 0);

    // Every thread busy: the read doesn't wait, it reads on its own thread.
    let busy = first.try_threads(4, 4).unwrap();
    assert_eq!(busy.n, 4);
    s.writer
        .call(|c| c.execute("DELETE FROM file_stage WHERE stage = 'read'", []))
        .unwrap();
    let (looked, threads, most_busy) =
        read_noting(&s, s.parallel().sharing(first.clone()), &first, 1);
    assert_eq!(looked, files);
    assert_eq!(
        threads.values().copied().collect::<Vec<_>>(),
        [THREAD_PRIORITY_NORMAL]
    );
    assert_eq!(most_busy, 4);
    assert_eq!(first.busy(), 4);
    drop(busy);
    assert_eq!(first.busy(), 0);
}

#[test]
fn a_read_of_two_files_borrows_at_most_one_thread() {
    // No more helpers than there are files for: a two-file rescan mustn't
    // take three threads from a fingerprint run.
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    put(&dir, "a.mp3", &audio::mp3());
    put(&dir, "b.mp3", &audio::mp3());
    s.walk();
    let first = FirstUp::default();
    let most_busy = Arc::new(AtomicU64::new(0));
    let (peak, line) = (most_busy.clone(), first.clone());
    let q = s.queue(s.parallel().sharing(first.clone()).on_file(move |_| {
        peak.fetch_max(line.busy() as u64, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(5));
    }));
    let id = q.enqueue(read_job(None)).unwrap();
    assert_eq!(wait(&s.writer, id), JobStatus::Done);
    q.shutdown();
    assert_eq!(most_busy.load(Ordering::SeqCst), 1);
    assert_eq!(first.busy(), 0);
}

#[test]
fn a_panic_on_the_job_thread_fails_the_job_and_gives_the_borrowed_threads_back() {
    // The readers are told to stop however the job thread ends, so the
    // job fails instead of hanging with the budget's threads held.
    let s = Sandbox::new();
    s.mixed_bag("Music");
    s.walk();
    let first = FirstUp::default();
    let reader = s
        .parallel()
        .sharing(first.clone())
        .on_page(|_| panic!("injected: the job thread died"));
    let q = s.queue(reader);
    let id = q.enqueue(read_job(None)).unwrap();
    assert_eq!(wait(&s.writer, id), JobStatus::Failed);
    let job = s
        .writer
        .call(move |c| jobs::store::get(c, id))
        .unwrap()
        .unwrap();
    assert!(
        job.error.as_deref().unwrap_or("").contains("panicked"),
        "{:?}",
        job.error
    );
    q.shutdown();
    assert_eq!(first.busy(), 0, "the borrowed threads went back");
    assert!(
        s.rows().values().all(|r| r.stage.is_none()),
        "nothing was written"
    );
}

#[test]
fn a_file_whose_row_changes_while_it_is_paged_is_still_due_afterwards() {
    // The stage records the size and mtime the page carried, not the
    // row's at write time: a walk that updates the row mid-read leaves the
    // file due, so its new content is read next time.
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    for n in 0..10 {
        put(&dir, &format!("{n:02}.mp3"), &audio::mp3());
    }
    s.walk();
    let writer = s.writer.clone();
    let bumped = Arc::new(AtomicBool::new(false));
    let once = bumped.clone();
    let reader = s.parallel().on_file(move |_| {
        if !once.swap(true, Ordering::SeqCst) {
            // The page is fetched before any file is handed out, so this
            // lands after it: the row now says the file changed.
            writer
                .call(|c| {
                    c.execute(
                        "UPDATE file SET size = size + 1 WHERE rel_path = '05.mp3'",
                        [],
                    )
                })
                .unwrap();
        }
    });
    let q = s.queue(reader);
    let id = q.enqueue(read_job(None)).unwrap();
    assert_eq!(wait(&s.writer, id), JobStatus::Done);
    q.shutdown();
    let due = s
        .writer
        .call(|c| scan_state::due(c, Stage::Read, READ_VERSION, &Scope::All, 0, 100))
        .unwrap();
    let paths: Vec<&str> = due.iter().map(|d| d.rel_path.as_str()).collect();
    assert_eq!(paths, ["05.mp3"]);
    assert_eq!(s.read(), 1);
    assert_eq!(s.read(), 0);
}

#[test]
fn cancelling_a_parallel_read_keeps_whole_batches_only_and_the_rest_is_read_next_time() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    for n in 0..30 {
        put(&dir, &format!("{n:02}.mp3"), &audio::mp3());
    }
    s.walk();

    // Batches of 4 on four threads; cancel once 10 files have been taken.
    let queue_slot: Arc<Mutex<Option<RunningJob>>> = Arc::default();
    let slot = queue_slot.clone();
    let reader = s
        .parallel()
        .batches(4, Duration::from_secs(3600))
        .on_file(move |n| {
            std::thread::sleep(Duration::from_millis(5));
            if n == 10 {
                let (q, id) = loop {
                    if let Some(found) = slot.lock().unwrap().clone() {
                        break found;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                };
                q.cancel(id).unwrap();
            }
        });
    let q = Arc::new(s.queue(reader));
    let id = q.enqueue(read_job(None)).unwrap();
    *queue_slot.lock().unwrap() = Some((q.clone(), id));
    assert_eq!(wait(&s.writer, id), JobStatus::Cancelled);
    q.shutdown();

    let rows = s.rows();
    let written = rows.values().filter(|r| r.stage.is_some()).count();
    assert_eq!(written % 4, 0, "{written} written: only whole batches");
    assert!(written < 30, "the cancel landed before the end");
    // A file's columns and its stage row were written together.
    for (path, row) in &rows {
        assert_eq!(row.stage.is_some(), row.codec.is_some(), "{path}");
    }
    // The rest is read next time, and nothing twice.
    let (looked, status) = s.read_with(s.parallel(), None);
    assert_eq!(status, JobStatus::Done);
    assert_eq!(looked as usize, 30 - written);
    assert!(s
        .rows()
        .values()
        .all(|r| r.stage.as_deref() == Some("done")));
}

#[test]
fn a_parallel_reads_progress_only_rises_and_ends_at_one() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    for n in 0..300 {
        put(&dir, &format!("{n:03}.mp3"), &audio::mp3());
    }
    s.walk();
    let updates = Arc::<Mutex<Vec<JobUpdate>>>::default();
    let heard = updates.clone();
    let volume = s.volume.clone();
    let q = JobQueue::builder(s.writer.clone())
        .workers(1)
        .on_updates(move |u: &[JobUpdate]| heard.lock().unwrap().extend_from_slice(u))
        .handler(
            JobKind::Read,
            s.parallel()
                .on_file(|_| std::thread::sleep(Duration::from_millis(2))),
        )
        .handler(JobKind::Scan, Walker::new(move || volume.clone(), |_| {}))
        .start()
        .unwrap();
    let id = q.enqueue(read_job(None)).unwrap();
    assert_eq!(wait(&s.writer, id), JobStatus::Done);
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
    // Updates within 50 ms coalesce, so how many arrive depends on the
    // machine; that some progress short of done was reported, nothing
    // went back, and it ended at one holds everywhere.
    assert!(progress.iter().any(|p| *p < 1.0), "{progress:?}");
    assert!(progress.windows(2).all(|w| w[1] >= w[0]), "{progress:?}");
    assert_eq!(progress.last(), Some(&1.0));
}

#[test]
fn a_read_of_one_music_folder_reads_only_its_files() {
    let s = Sandbox::new();
    let (a, a_dir) = s.folder("A");
    let (_, b_dir) = s.folder("B");
    put(&a_dir, "a.mp3", &audio::mp3());
    put(&b_dir, "b.mp3", &audio::mp3());
    s.walk();
    let (looked, status) = s.read_with(s.reader(), Some(vec![a]));
    assert_eq!((looked, status), (1, JobStatus::Done));
    assert_eq!(s.row("a.mp3").stage.as_deref(), Some("done"));
    assert_eq!(s.row("b.mp3").stage, None);
}

#[test]
fn files_on_an_unplugged_drive_are_left_for_when_it_is_back() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    put(&dir, "a.mp3", &audio::mp3());
    s.walk();
    s.volume.online.store(false, Ordering::SeqCst);
    assert_eq!(s.read(), 1);
    assert_eq!(s.row("a.mp3").stage, None, "nothing recorded while offline");
    s.volume.online.store(true, Ordering::SeqCst);
    assert_eq!(s.read(), 1);
    assert_eq!(s.row("a.mp3").stage.as_deref(), Some("done"));
}

#[test]
fn names_windows_would_mangle_are_read_as_their_exact_file() {
    // A trailing dot or space is dropped by a plain Win32 path; the
    // `\\?\` path keeps it, so each name opens its own file (§5.6).
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    let verbatim = crate::paths::verbatim_absolute(&dir).unwrap();
    fs::create_dir(verbatim.join("Q.X.Z.")).unwrap();
    fs::create_dir(verbatim.join("Q.X.Z")).unwrap();
    fs::write(
        verbatim.join("Q.X.Z.").join("a.mp3"),
        tagged_mp3("With dot"),
    )
    .unwrap();
    // The sibling is a FLAC under the same name, so the sniff's open and the
    // tag reader's open each show which file they got.
    let flac = [b"fLaC".to_vec(), audio::flac()[4..].to_vec()].concat();
    fs::write(verbatim.join("Q.X.Z").join("a.mp3"), flac).unwrap();
    s.walk();
    s.read();
    let dotted = s.row("Q.X.Z./a.mp3");
    assert_eq!(dotted.sniffed_format.as_deref(), Some("mp3"));
    assert_eq!(
        dotted.sample_rate,
        Some(44_100),
        "properties from the tag reader"
    );
    assert!(dotted.raw_tags.unwrap().contains("With dot"));
    let plain = s.row("Q.X.Z/a.mp3");
    assert_eq!(plain.sniffed_format.as_deref(), Some("flac"));
    assert_eq!(plain.codec.as_deref(), Some("flac"));
    assert_eq!(plain.sample_rate, Some(i64::from(audio::SAMPLE_RATE)));
    assert!(!plain.raw_tags.unwrap().contains("With dot"));
}

/// What's recorded about each entry under a folder.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    File {
        bytes: Vec<u8>,
        modified: SystemTime,
        readonly: bool,
    },
    Folder {
        modified: SystemTime,
    },
}

fn snapshot(dir: &Path, skip: &Path, out: &mut BTreeMap<PathBuf, Entry>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path == skip {
            continue;
        }
        let meta = fs::symlink_metadata(&path).unwrap();
        let entry = if meta.is_dir() {
            snapshot(&path, skip, out);
            Entry::Folder {
                modified: meta.modified().unwrap(),
            }
        } else {
            Entry::File {
                bytes: fs::read(&path).unwrap(),
                modified: meta.modified().unwrap(),
                readonly: meta.permissions().readonly(),
            }
        };
        out.insert(path, entry);
    }
}

#[test]
fn the_read_job_writes_nothing_outside_the_app_data_folder() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    for format in Format::ALL {
        format.write_to(&dir, "tone");
    }
    fs::create_dir(dir.join("sub")).unwrap();
    put(&dir.join("sub"), "tagged.mp3", &tagged_mp3("Leave me be"));
    put(&dir, "empty.mp3", &[]);
    let readonly = put(&dir, "readonly.flac", &audio::flac());
    let mut perms = fs::metadata(&readonly).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&readonly, perms).unwrap();
    s.walk();

    let app_data = s.root.path().join("app-data");
    let mut before = BTreeMap::new();
    snapshot(s.root.path(), &app_data, &mut before);
    assert!(before.len() > 10);
    // Let a changed modified time show.
    std::thread::sleep(Duration::from_millis(50));
    assert!(s.read() >= 9);
    let mut after = BTreeMap::new();
    snapshot(s.root.path(), &app_data, &mut after);
    assert_eq!(before, after);

    let mut perms = fs::metadata(&readonly).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&readonly, perms).unwrap();
}

#[test]
fn timing_smoke_the_read_job_keeps_up_a_steady_pace() {
    // Not a benchmark: a floor far below what a laptop does, to catch a
    // regression that makes reading files crawl. The rate is printed.
    const FILES: usize = 2000;
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    let formats = [Format::Mp3, Format::Flac, Format::Wav, Format::M4a];
    for n in 0..FILES {
        let format = formats[n % formats.len()];
        let bytes = if format == Format::Mp3 {
            tagged_mp3(&format!("Track {n}"))
        } else {
            format.bytes()
        };
        put(&dir, &format!("{n:05}.{}", format.extension()), &bytes);
    }
    s.walk();
    let start = Instant::now();
    assert_eq!(s.read(), FILES as u64);
    let rate = FILES as f64 / start.elapsed().as_secs_f64();
    println!("read job: {rate:.0} files/s over {FILES} small files");
    assert!(rate > 50.0, "only {rate:.0} files/s");
}

/// Makes `path` look online-only (a OneDrive placeholder) to the walk and
/// to the gate, the way lane 4's tests do: a real placeholder needs a sync
/// provider.
fn set_online_only(path: &Path, online_only: bool) {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    assert_ne!(attributes, INVALID_FILE_ATTRIBUTES, "{}", path.display());
    let attributes = if online_only {
        attributes | FILE_ATTRIBUTE_OFFLINE
    } else {
        attributes & !FILE_ATTRIBUTE_OFFLINE
    };
    assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), attributes) }, 0);
}

/// Holds `path` open with no sharing: while the handle lives, any open of
/// its data fails, so a file the job opened would come back unreachable.
/// Its attributes can still be asked for.
fn lock_against_reading(path: &Path) -> fs::File {
    fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

fn opt_in(s: &Sandbox, on: bool) {
    s.writer
        .call(move |c| {
            c.execute(
                "INSERT INTO setting (key, value) VALUES (?1, json(?2))
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                (READ_ONLINE_ONLY_FILES, if on { "true" } else { "false" }),
            )
        })
        .unwrap();
}

#[test]
fn an_online_only_file_is_skipped_without_being_opened_until_the_user_opts_in() {
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    put(&dir, "local.mp3", &audio::mp3());
    let cloud = put(&dir, "cloud.mp3", &tagged_mp3("In the cloud"));
    set_online_only(&cloud, true);
    s.walk();

    let lock = lock_against_reading(&cloud);
    assert_eq!(s.read(), 2);
    let row = s.row("cloud.mp3");
    // Online-only, not unreachable: the job never tried to open it.
    assert_eq!(row.stage.as_deref(), Some("skipped"));
    assert_eq!(row.reason.as_deref(), Some("online_only"));
    assert_eq!((row.sniffed_format, row.codec), (None, None));
    assert_eq!(s.row("local.mp3").stage.as_deref(), Some("done"));
    // A skip is decided again on every run, and the run still ends.
    assert_eq!(s.read(), 1);
    drop(lock);

    opt_in(&s, true);
    assert_eq!(s.read(), 1);
    let row = s.row("cloud.mp3");
    assert_eq!(row.stage.as_deref(), Some("done"));
    assert!(row.raw_tags.unwrap().contains("In the cloud"));
    set_online_only(&cloud, false);
}

#[test]
fn a_file_that_went_online_only_since_the_walk_is_not_opened_either() {
    // The walk saw it local; OneDrive freed it up before the read. The
    // check right before opening catches it.
    let s = Sandbox::new();
    let (_, dir) = s.folder("Music");
    let freed = put(&dir, "freed.mp3", &audio::mp3());
    s.walk();
    set_online_only(&freed, true);
    let lock = lock_against_reading(&freed);
    assert_eq!(s.read(), 1);
    let row = s.row("freed.mp3");
    assert_eq!(row.stage.as_deref(), Some("skipped"));
    assert_eq!(row.reason.as_deref(), Some("online_only"));
    assert_eq!(row.codec, None);
    drop(lock);
    set_online_only(&freed, false);
}
