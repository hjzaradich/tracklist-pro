#![cfg(test)]
//! 1aC-1: the unchanged check. A file the walk sees again is compared with
//! its row before the upsert: a touch (only the mtime moved) whose content
//! is the same makes no stage due again; anything else does.
//!
//! These run the real walk and the real hash job on files in a temp folder,
//! so the partial hash the hash stage stores is the one the walk compares.

use std::collections::BTreeMap;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_OFFLINE,
};

use super::support::{db, TempVolume};
use super::walk::{add_music, at, drive, put, rows, wait};
use crate::db::Writer;
use crate::hash::{hash_job, Hasher, DEFINITION};
use crate::jobs::{JobKind, JobQueue, JobStatus};
use crate::scan::online_only::write_opt_in;
use crate::scan::unchanged::{compare, verdict, wants_read, Change, Stored, Verdict};
use crate::scan::walk::{scan_job, Found};
use crate::scan::Walker;
use crate::scan_state::{self, Outcome, Recorded, Scope, Stage};
use crate::tags::test_audio::{id3_text, id3v2};

// ---- files ----------------------------------------------------------------

/// `n` MPEG-1 Layer III frames of 417 bytes, each with its own bytes.
fn frames(n: usize) -> Vec<u8> {
    (0..n)
        .flat_map(|k| {
            let mut frame: Vec<u8> = (0..417)
                .map(|i| (i as u8).wrapping_mul(31).wrapping_add(k as u8))
                .collect();
            frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
            frame
        })
        .collect()
}

/// Frames in a comment as long as `artwork_len` (artwork, to a tagger):
/// the file is more than two 64 KiB edges of audio, so its middle isn't
/// sampled. `mark` is the byte at `MARK_AT` in the artwork.
const ARTWORK: usize = 200_000;
const MARK_AT: usize = 100_000;
const FRAMES: usize = 400;

fn track(mark: char) -> Vec<u8> {
    let mut text = "x".repeat(ARTWORK);
    text.replace_range(MARK_AT..=MARK_AT, &mark.to_string());
    [id3v2(&[id3_text(b"COMM", &text)], 0), frames(FRAMES)].concat()
}

/// Where the artwork mark, the first audio byte and the last audio byte are.
fn artwork_mark() -> usize {
    // The ID3 header, the frame header and its encoding byte, then the text.
    10 + 10 + 1 + MARK_AT
}
fn audio_start() -> usize {
    id3v2(&[id3_text(b"COMM", &"x".repeat(ARTWORK))], 0).len()
}
fn audio_end() -> usize {
    audio_start() + FRAMES * 417
}

fn edit(path: &Path, at: usize) {
    let mut bytes = fs::read(path).unwrap();
    bytes[at] ^= 0x55;
    fs::write(path, bytes).unwrap();
}

/// Sets a file's modified time to a distinct moment in 2017: `n` seconds
/// after 1.5 billion.
fn set_mtime(path: &Path, n: u64) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(1_500_000_000 + n))
        .unwrap();
}

fn mtime_of(path: &Path) -> i64 {
    let t = fs::metadata(path).unwrap().modified().unwrap();
    t.duration_since(UNIX_EPOCH).unwrap().as_nanos() as i64
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Makes the file look online only in its folder's listing, or not.
fn set_offline(path: &Path, offline: bool) {
    let w = wide(path);
    let attributes = unsafe { GetFileAttributesW(w.as_ptr()) };
    let attributes = if offline {
        attributes | FILE_ATTRIBUTE_OFFLINE
    } else {
        attributes & !FILE_ATTRIBUTE_OFFLINE
    };
    assert_ne!(unsafe { SetFileAttributesW(w.as_ptr(), attributes) }, 0);
}

// ---- a library to scan ------------------------------------------------------

struct Lib {
    _dir: tempfile::TempDir,
    volume: TempVolume,
    music: PathBuf,
    _db: tempfile::TempDir,
    writer: Writer,
    /// Every file the walk opened for the unchanged check, in every run.
    reads: Arc<Mutex<Vec<PathBuf>>>,
    /// What to do before each directory entry the walk looks at, if set.
    entry_hook: EntryHook,
}

type EntryHook = Arc<Mutex<Option<Box<dyn Fn(u64) + Send + Sync>>>>;

fn lib() -> Lib {
    let (dir, volume, music) = drive();
    let (dbdir, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    Lib {
        _dir: dir,
        volume,
        music,
        _db: dbdir,
        writer,
        reads: Arc::default(),
        entry_hook: Arc::default(),
    }
}

impl Lib {
    fn put(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        put(&self.music, rel, bytes);
        at(&self.music, rel)
    }

    fn run(&self, job: crate::jobs::NewJob) {
        let (volume, reads) = (self.volume.clone(), self.reads.clone());
        let (hash_volume, entry) = (self.volume.clone(), self.entry_hook.clone());
        let q = JobQueue::builder(self.writer.clone())
            .workers(1)
            .handler(
                JobKind::Scan,
                Walker::new(move || volume.clone(), |_| {})
                    .on_entry(move |n| {
                        if let Some(hook) = &*entry.lock().unwrap() {
                            hook(n);
                        }
                    })
                    .on_read(move |p| reads.lock().unwrap().push(p.to_path_buf())),
            )
            .handler(JobKind::Hash, Hasher::new(move || hash_volume.clone()))
            .start()
            .unwrap();
        let id = q.enqueue(job).unwrap();
        assert_eq!(wait(&self.writer, id), JobStatus::Done);
        q.shutdown();
    }

    fn walk(&self) {
        self.run(scan_job(None));
    }

    fn hash(&self) {
        self.run(hash_job(None));
    }

    /// The files the walk has opened since the last call.
    fn take_reads(&self) -> Vec<PathBuf> {
        std::mem::take(&mut self.reads.lock().unwrap())
    }

    /// Records the `read` and `fingerprint` stages as done for every file,
    /// at the size and mtime each file has now, as those stages would.
    fn finish_read_and_fingerprint(&self) {
        self.writer
            .call(|c| {
                for stage in [Stage::Read, Stage::Fingerprint] {
                    let due = scan_state::due(c, stage, 1, &Scope::All, 0, 1000)?;
                    let done: Vec<Recorded> =
                        due.iter().map(|d| Recorded::of(d, Outcome::Done)).collect();
                    scan_state::record(c, stage, 1, &done)?;
                }
                Ok(())
            })
            .unwrap();
    }

    /// How many files are due for (read, hash, fingerprint).
    fn due(&self) -> (u64, u64, u64) {
        self.writer
            .call(|c| {
                let count = |stage, version| scan_state::count_due(c, stage, version, &Scope::All);
                Ok((
                    count(Stage::Read, 1)?,
                    count(Stage::Hash, i64::from(DEFINITION))?,
                    count(Stage::Fingerprint, 1)?,
                ))
            })
            .unwrap()
    }

    fn partial_hashes(&self) -> Vec<Option<Vec<u8>>> {
        self.writer
            .call(|c| {
                let mut s = c.prepare("SELECT partial_hash FROM file ORDER BY rel_path")?;
                let rows = s.query_map([], |r| r.get(0))?;
                rows.collect()
            })
            .unwrap()
    }

    /// A library with `a.mp3` scanned, hashed and finished by every stage.
    fn settled(&self) -> PathBuf {
        let a = self.put("a.mp3", &track('y'));
        self.walk();
        self.hash();
        self.finish_read_and_fingerprint();
        assert_eq!(self.due(), (0, 0, 0), "everything is done to start with");
        assert!(
            self.partial_hashes()[0].is_some(),
            "the hash stage stored it"
        );
        assert!(self.take_reads().is_empty());
        a
    }
}

// ---- the check --------------------------------------------------------------

#[test]
fn an_mtime_only_touch_does_not_make_any_stage_due_again() {
    let lib = lib();
    let a = lib.settled();
    let stored = lib.partial_hashes();

    // rekordbox opens the file and closes it: same bytes, a new mtime.
    set_mtime(&a, 1);
    lib.walk();

    assert_eq!(
        lib.take_reads(),
        std::slice::from_ref(&a),
        "the touched file was compared"
    );
    let row = &rows(&lib.writer)[0];
    assert_eq!(row.mtime, Some(mtime_of(&a)), "the row has the new mtime");
    assert_eq!(lib.due(), (0, 0, 0), "no stage is due again");
    assert_eq!(lib.partial_hashes(), stored, "the stored hash still stands");
    // And the hash job finds nothing to do.
    lib.hash();
    assert_eq!(lib.due(), (0, 0, 0));
}

#[test]
fn a_touch_followed_by_a_touch_is_still_unchanged() {
    let lib = lib();
    let a = lib.settled();
    for n in 1..=3 {
        set_mtime(&a, n);
        lib.walk();
        assert_eq!(lib.due(), (0, 0, 0), "touch {n}");
    }
}

#[test]
fn a_walk_of_files_nobody_touched_reads_nothing() {
    let lib = lib();
    lib.settled();
    lib.walk();
    lib.walk();
    assert!(lib.take_reads().is_empty());
    assert_eq!(lib.due(), (0, 0, 0));
}

#[test]
fn a_same_size_tag_edit_inside_big_artwork_is_caught() {
    let lib = lib();
    let a = lib.settled();

    edit(&a, artwork_mark());
    set_mtime(&a, 1);
    lib.walk();

    assert_eq!(fs::read(&a).unwrap().len(), track('y').len(), "same size");
    assert_eq!(lib.due(), (1, 1, 1), "every stage is due again");
    assert_eq!(
        lib.partial_hashes(),
        [None],
        "the old content's hash is gone"
    );
    // The hash stage stores the new one, so the next touch is recognized.
    lib.hash();
    lib.finish_read_and_fingerprint();
    set_mtime(&a, 2);
    lib.walk();
    assert_eq!(lib.due(), (0, 0, 0));
}

#[test]
fn a_change_in_the_audio_edges_is_caught() {
    for (name, offset) in [
        ("the first audio byte", audio_start()),
        ("the end of the first 64 KiB", audio_start() + 64 * 1024 - 1),
        ("the start of the last 64 KiB", audio_end() - 64 * 1024),
        ("the last audio byte", audio_end() - 1),
    ] {
        let lib = lib();
        let a = lib.settled();
        edit(&a, offset);
        set_mtime(&a, 1);
        lib.walk();
        assert_eq!(lib.due(), (1, 1, 1), "{name}");
    }
}

#[test]
fn a_same_size_edit_in_the_middle_of_the_audio_is_the_known_blind_spot() {
    // The partial hash reads the audio's edges. An edit of the same length
    // away from both looks like a touch (hash/partial.rs explains why that's
    // the trade-off). It's caught the next time the size or an edge changes.
    let lib = lib();
    let a = lib.settled();
    edit(&a, (audio_start() + audio_end()) / 2);
    set_mtime(&a, 1);
    lib.walk();
    assert_eq!(
        lib.due(),
        (0, 0, 0),
        "if this is caught now, the partial hash reads more: update its docs"
    );
}

#[test]
fn a_size_change_makes_the_stages_due() {
    let lib = lib();
    let a = lib.settled();
    let mut bytes = fs::read(&a).unwrap();
    bytes.extend_from_slice(&frames(1));
    fs::write(&a, bytes).unwrap();
    lib.walk();
    assert_eq!(lib.due(), (1, 1, 1));
    assert!(lib.take_reads().is_empty(), "a resized file isn't compared");
    assert_eq!(lib.partial_hashes(), [None]);
}

/// Replaces `a` with another file of `bytes`, which keeps `a`'s size and
/// mtime: a new file id, since it's a different file.
fn reissue(lib: &Lib, a: &Path, bytes: &[u8], mtime: i64) {
    let other = lib.put("other.mp3", bytes);
    set_mtime(&other, 0);
    fs::remove_file(a).unwrap();
    fs::rename(&other, a).unwrap();
    assert_eq!(mtime_of(a), mtime, "the copy kept the time");
}

/// A library with `a.mp3` at mtime 0, walked, hashed and finished by every
/// stage.
fn settled_at_zero(lib: &Lib) -> PathBuf {
    let a = lib.put("a.mp3", &track('y'));
    set_mtime(&a, 0);
    lib.walk();
    lib.hash();
    lib.finish_read_and_fingerprint();
    assert_eq!(lib.due(), (0, 0, 0));
    lib.take_reads();
    a
}

#[test]
fn a_new_file_id_with_different_content_redoes_every_stage_even_with_the_same_size_and_mtime() {
    let lib = lib();
    let a = settled_at_zero(&lib);
    let before = rows(&lib.writer)[0].clone();

    reissue(&lib, &a, &track('z'), before.mtime.unwrap());
    lib.walk();

    let after = rows(&lib.writer)[0].clone();
    assert_eq!(
        (after.size, after.mtime),
        (before.size, before.mtime),
        "same size and mtime"
    );
    assert_ne!(after.file_id, before.file_id, "another file");
    assert_eq!(lib.take_reads(), [a], "compared, and it differs");
    assert_eq!(lib.due(), (1, 1, 1), "so every stage redoes it");
    assert_eq!(lib.partial_hashes(), [None]);
}

#[test]
fn a_new_file_id_with_the_same_content_keeps_its_stages() {
    // A restore, a sync tool or a share with unstable ids: identical bytes,
    // size and mtime under a new id. It's not worth re-hashing.
    let lib = lib();
    let a = settled_at_zero(&lib);
    let before = rows(&lib.writer)[0].clone();
    let stored = lib.partial_hashes();
    let stages = stage_rows(&lib);

    reissue(&lib, &a, &track('y'), before.mtime.unwrap());
    lib.walk();

    let after = rows(&lib.writer)[0].clone();
    assert_ne!(after.file_id, before.file_id, "the row has the new id");
    assert_eq!(lib.take_reads(), [a], "the content was compared");
    assert_eq!(lib.due(), (0, 0, 0), "no stage is due");
    assert_eq!(stage_rows(&lib), stages, "the stage rows are untouched");
    assert_eq!(lib.partial_hashes(), stored);
}

#[test]
fn a_new_file_id_with_no_stored_partial_hash_is_changed() {
    let lib = lib();
    let a = lib.put("a.mp3", &track('y'));
    set_mtime(&a, 0);
    lib.walk();
    lib.finish_read_and_fingerprint();
    assert_eq!(lib.partial_hashes(), [None], "the hash stage hasn't run");
    let mtime = rows(&lib.writer)[0].mtime.unwrap();

    reissue(&lib, &a, &track('y'), mtime);
    lib.walk();

    assert!(lib.take_reads().is_empty(), "nothing to compare with");
    assert_eq!(lib.due(), (1, 1, 1));
}

#[test]
fn a_new_file_id_the_walk_cannot_read_is_changed() {
    let lib = lib();
    let a = settled_at_zero(&lib);
    let mtime = rows(&lib.writer)[0].mtime.unwrap();
    reissue(&lib, &a, &track('y'), mtime);
    let _lock = lock_against_reading(&a);

    lib.walk();

    assert_eq!(lib.due(), (1, 1, 1), "it couldn't be compared");
}

#[test]
fn a_touched_file_with_no_stored_partial_hash_is_changed() {
    let lib = lib();
    let a = lib.put("a.mp3", &track('y'));
    lib.walk();
    // The hash stage hasn't run, so there's nothing to compare with.
    lib.finish_read_and_fingerprint();
    assert_eq!(lib.partial_hashes(), [None]);
    set_mtime(&a, 1);
    lib.walk();
    assert_eq!(lib.due(), (1, 1, 1));
    assert!(
        lib.take_reads().is_empty(),
        "nothing to compare, so no read"
    );
}

#[test]
fn a_touched_file_of_an_unknown_format_is_changed() {
    let lib = lib();
    let junk: Vec<u8> = (0..50_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let a = lib.put("junk.mp3", &junk);
    lib.walk();
    lib.hash();
    lib.finish_read_and_fingerprint();
    assert_eq!(lib.due(), (0, 0, 0));
    assert_eq!(
        lib.partial_hashes(),
        [None],
        "no shortcut for unknown bytes"
    );
    set_mtime(&a, 1);
    lib.walk();
    assert_eq!(lib.due(), (1, 1, 1));
}

#[test]
fn a_stage_that_was_already_due_stays_due_after_a_touch() {
    let lib = lib();
    let a = lib.settled();
    // A real change; the hash stage redoes it, the others haven't yet.
    edit(&a, audio_start());
    set_mtime(&a, 1);
    lib.walk();
    lib.hash();
    assert_eq!(lib.due(), (1, 0, 1));

    set_mtime(&a, 2);
    lib.walk();
    assert_eq!(
        lib.due(),
        (1, 0, 1),
        "the touch carried the hash stage on, and left the others due"
    );
}

#[test]
fn a_file_that_is_back_after_being_missing_and_only_touched_is_not_redone() {
    let lib = lib();
    let a = lib.settled();
    // Moved out of the music folder and back: the same file, same file id.
    let parked = lib.volume.mount.join("parked.mp3");
    fs::rename(&a, &parked).unwrap();
    lib.walk();
    assert!(!rows(&lib.writer)[0].present);
    fs::rename(&parked, &a).unwrap();
    set_mtime(&a, 5);
    lib.walk();
    assert!(rows(&lib.writer)[0].present);
    assert_eq!(lib.due(), (0, 0, 0), "same file, same bytes, only touched");
}

#[test]
fn a_file_deleted_and_written_again_is_another_file() {
    let lib = lib();
    let a = lib.settled();
    let bytes = fs::read(&a).unwrap();
    fs::remove_file(&a).unwrap();
    lib.walk();
    fs::write(&a, bytes).unwrap();
    lib.walk();
    assert!(rows(&lib.writer)[0].present);
    assert_eq!(lib.due(), (1, 1, 1), "a new file id: nothing carries over");
}

fn stage_rows(lib: &Lib) -> Vec<(String, String, i64, i64)> {
    lib.writer
        .call(|c| {
            let mut s =
                c.prepare("SELECT stage, status, version, mtime FROM file_stage ORDER BY stage")?;
            let r = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            r.collect()
        })
        .unwrap()
}

#[test]
fn a_touch_carries_failed_skipped_and_old_version_rows_as_they_are() {
    // A carried row moves to the new mtime and nothing else: a stage that
    // failed stays failed (it waits until the content changes, and it
    // hasn't), one that was skipped is still tried again, and one from an
    // older version is still redone.
    let lib = lib();
    let a = lib.settled();
    lib.writer
        .call(|c| {
            c.execute(
                "UPDATE file_stage SET status = 'failed', reason = 'x' WHERE stage = 'fingerprint'",
                [],
            )?;
            c.execute(
                "UPDATE file_stage SET status = 'skipped', reason = 'unreachable' WHERE stage = 'read'",
                [],
            )?;
            c.execute(
                "UPDATE file_stage SET version = version + 100 WHERE stage = 'hash'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(lib.due(), (1, 1, 0));
    let before = stage_rows(&lib);

    set_mtime(&a, 1);
    lib.walk();

    let after = stage_rows(&lib);
    for (was, now) in before.iter().zip(&after) {
        assert_eq!(
            (&was.0, &was.1, was.2),
            (&now.0, &now.1, now.2),
            "status and version are kept"
        );
        assert_eq!(now.3, mtime_of(&a), "{} moved to the new mtime", now.0);
    }
    assert_eq!(
        lib.due(),
        (1, 1, 0),
        "skipped and old-version rows are still due; failed still waits"
    );
}

// ---- reading ----------------------------------------------------------------

/// Holds `path` open with no sharing: while it lives, opening the file to
/// read it fails. Asking for its attributes or file id still works.
fn lock_against_reading(path: &Path) -> fs::File {
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap();
    assert!(fs::read(path).is_err());
    lock
}

#[test]
fn the_first_walk_opens_no_files() {
    let lib = lib();
    let paths: Vec<PathBuf> = (0..5)
        .map(|i| lib.put(&format!("Artist/t{i}.mp3"), &track('y')))
        .collect();
    let _locks: Vec<fs::File> = paths.iter().map(|p| lock_against_reading(p)).collect();
    lib.walk();
    lib.walk();
    assert_eq!(rows(&lib.writer).len(), 5, "all indexed");
    // The `on_read` hook is the witness: it sits at the only place the walk
    // reads a file. The locks only show that indexing needs no read.
    assert!(lib.take_reads().is_empty(), "no file was opened to compare");
}

#[test]
fn an_online_only_file_is_never_read() {
    let lib = lib();
    let a = lib.settled();
    set_mtime(&a, 1);
    set_offline(&a, true);
    let _lock = lock_against_reading(&a);

    lib.walk();

    assert!(
        lib.take_reads().is_empty(),
        "an online-only file was opened"
    );
    let row = &rows(&lib.writer)[0];
    assert!(row.present, "still indexed");
    assert_eq!(
        lib.due().1,
        1,
        "a touched online-only file counts as changed"
    );
    assert_eq!(lib.partial_hashes(), [None]);
}

#[test]
fn a_file_that_became_online_only_after_the_listing_is_not_opened() {
    // OneDrive can free a file's space at any moment, so the walk asks again
    // just before opening (ReadGate::may_open). `a.mp3` is listed as local;
    // before `b.mp3` is listed, it becomes a placeholder.
    let lib = lib();
    let a = lib.settled();
    lib.put("b.mp3", &track('q'));
    lib.walk();
    lib.hash();
    lib.finish_read_and_fingerprint();
    set_mtime(&a, 1);
    let dehydrate = a.clone();
    *lib.entry_hook.lock().unwrap() = Some(Box::new(move |n| {
        if n == 1 {
            set_offline(&dehydrate, true);
        }
    }));

    lib.walk();

    assert!(lib.take_reads().is_empty(), "a placeholder was opened");
    assert_eq!(
        lib.due(),
        (1, 1, 1),
        "it counts as changed, and b.mp3 is done"
    );
}

#[test]
fn an_online_only_file_is_compared_once_the_user_opts_in() {
    let lib = lib();
    lib.writer.call(|c| write_opt_in(c, true)).unwrap();
    let a = lib.put("a.mp3", &track('y'));
    set_offline(&a, true);
    lib.walk();
    lib.hash();
    lib.finish_read_and_fingerprint();
    assert_eq!(lib.due(), (0, 0, 0));
    assert!(lib.partial_hashes()[0].is_some());

    set_mtime(&a, 1);
    lib.walk();
    assert_eq!(lib.take_reads(), [a]);
    assert_eq!(lib.due(), (0, 0, 0));
}

// ---- nothing written --------------------------------------------------------

fn snapshot(dir: &Path, out: &mut BTreeMap<PathBuf, (bool, Vec<u8>, Option<SystemTime>)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::metadata(&path).unwrap();
        if meta.is_dir() {
            out.insert(path.clone(), (true, Vec::new(), None));
            snapshot(&path, out);
        } else {
            out.insert(
                path.clone(),
                (false, fs::read(&path).unwrap(), meta.modified().ok()),
            );
        }
    }
}

#[test]
fn checking_files_writes_nothing_outside_the_app_data_folder() {
    let lib = lib();
    let a = lib.settled();
    let b = lib.put("Sub/b.mp3", &track('q'));
    lib.walk();
    lib.hash();
    set_mtime(&a, 1);
    set_mtime(&b, 2);
    let mut before = BTreeMap::new();
    snapshot(&lib.volume.mount, &mut before);

    // Touched files are compared (read), then the stages carry on.
    lib.walk();
    lib.hash();
    lib.walk();

    assert!(!lib.take_reads().is_empty(), "the comparison read files");
    let mut after = BTreeMap::new();
    snapshot(&lib.volume.mount, &mut after);
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .collect();
    assert!(changed.is_empty(), "the music folder changed: {changed:?}");
}

// ---- the comparison itself --------------------------------------------------

fn stored(size: i64, mtime: i64, file_id: Option<&str>, partial: Option<u8>) -> Stored {
    Stored {
        size: Some(size),
        mtime: Some(mtime),
        file_id: file_id.map(str::to_owned),
        partial_hash: partial.map(|b| vec![b; 33]),
    }
}

fn found(size: i64, mtime: i64, file_id: Option<&str>) -> Found {
    Found {
        rel: crate::paths::RelPath::parse("a.mp3").unwrap(),
        size,
        mtime_ns: mtime,
        file_id: file_id.map(str::to_owned),
        online_only: false,
    }
}

#[test]
fn the_comparison_tells_same_touched_resized_and_replaced_apart() {
    let row = stored(100, 5, Some("id-1"), Some(1));
    for (listing, expected) in [
        (found(100, 5, Some("id-1")), Change::Same),
        (found(100, 6, Some("id-1")), Change::Touched),
        (found(101, 5, Some("id-1")), Change::Resized),
        (found(101, 6, Some("id-1")), Change::Resized),
        (found(100, 5, Some("id-2")), Change::Reissued),
        (found(100, 6, Some("id-2")), Change::Replaced),
        (found(101, 5, Some("id-2")), Change::Replaced),
        (found(999, 999, Some("id-2")), Change::Replaced),
    ] {
        assert_eq!(compare(&row, &listing), expected, "{listing:?}");
    }
}

#[test]
fn a_file_id_that_is_unknown_on_either_side_never_says_replaced() {
    // Some FAT and network drives report none; rows from before file ids
    // were stored have none. Size and mtime decide.
    for (was, now) in [(None, Some("id")), (Some("id"), None), (None, None)] {
        let row = stored(100, 5, was, None);
        assert_eq!(compare(&row, &found(100, 5, now)), Change::Same);
        assert_eq!(compare(&row, &found(100, 6, now)), Change::Touched);
        assert_eq!(compare(&row, &found(7, 5, now)), Change::Resized);
    }
}

#[test]
fn a_touch_is_unchanged_only_when_the_partial_hashes_are_equal() {
    let row = stored(100, 5, None, Some(1));
    let same = [1u8; 33];
    let other = [2u8; 33];
    assert_eq!(
        verdict(Change::Touched, &row, Some(&same)),
        Verdict::TouchedOnly
    );
    for now in [Some(&other), None] {
        assert_eq!(
            verdict(Change::Touched, &row, now),
            Verdict::Changed { replaced: false }
        );
    }
    // Nothing stored: never unchanged, even if something was read.
    let none = stored(100, 5, None, None);
    assert_eq!(
        verdict(Change::Touched, &none, Some(&same)),
        Verdict::Changed { replaced: false }
    );
    assert_eq!(verdict(Change::Same, &row, None), Verdict::Unchanged);
    assert_eq!(
        verdict(Change::Resized, &row, Some(&same)),
        Verdict::Changed { replaced: false }
    );
    assert_eq!(
        verdict(Change::Replaced, &row, Some(&same)),
        Verdict::Changed { replaced: true }
    );
    // A new id with the same size and mtime is decided by the hash too.
    assert_eq!(
        verdict(Change::Reissued, &row, Some(&same)),
        Verdict::NewIdOnly
    );
    for now in [Some(&other), None] {
        assert_eq!(
            verdict(Change::Reissued, &row, now),
            Verdict::Changed { replaced: true }
        );
    }
    assert_eq!(
        verdict(Change::Reissued, &none, Some(&same)),
        Verdict::Changed { replaced: true }
    );
}

#[test]
fn only_a_touched_file_with_a_stored_hash_is_read() {
    let with = stored(100, 5, None, Some(1));
    let without = stored(100, 5, None, None);
    for change in [Change::Same, Change::Resized, Change::Replaced] {
        assert!(!wants_read(change, &with), "{change:?}");
    }
    for change in [Change::Touched, Change::Reissued] {
        assert!(wants_read(change, &with), "{change:?}");
        assert!(!wants_read(change, &without), "{change:?}");
    }
}
