#![cfg(test)]
//! 1aC-6: the per-root watcher. On and off per music folder; a file added
//! under a watched root gets indexed and read; a burst of changes is one
//! rescan; every online folder is rechecked at start and when its drive
//! returns, once per trigger, and an unplugged root stops its watcher and
//! gets it back when the drive returns; nothing is written outside the
//! app data folder.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use serde_json::json;
use tauri::Manager;

use super::chain::{chained_queue, jobs, wait_idle, LookedAt};
use super::support::{db, TempVolume};
use super::walk::{add_music, drive, put, rows};
use crate::db::Writer;
use crate::ipc::testing::{app, invoke};
use crate::jobs::{JobQueue, Priority};
use crate::scan::folders::MusicFolderId;
use crate::scan::watch::{set_watch, Enqueue, Watchers};
use crate::tags::test_audio as audio;

/// Short burst times: a test's burst is over in milliseconds, and the
/// quiet time only has to outlast that on a busy machine.
const QUIET: Duration = Duration::from_secs(2);
const AT_MOST: Duration = Duration::from_secs(30);

/// A watched (or not) music folder with the whole chain behind it.
struct Watched {
    // Dropped in this order: the watcher and the queue let go of the
    // folders before the temp dirs are removed.
    watchers: Watchers,
    queue: Arc<JobQueue>,
    looked: LookedAt,
    writer: Writer,
    volume: TempVolume,
    music: PathBuf,
    folder: MusicFolderId,
    _db: tempfile::TempDir,
    _dir: tempfile::TempDir,
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.watchers.shutdown();
        self.queue.shutdown();
    }
}

/// No folder watched.
fn none() -> Vec<MusicFolderId> {
    Vec::new()
}

impl Watched {
    fn new(watch: bool) -> Watched {
        let (dir, volume, music) = drive();
        let (db_dir, writer, _reads) = db();
        let folder = add_music(&writer, &volume, &music);
        writer.call(move |c| set_watch(c, folder, watch)).unwrap();
        let looked = LookedAt::default();
        let queue = Arc::new(chained_queue(&writer, &volume, &looked));
        let watchers = start(&writer, &volume, &queue);
        Watched {
            watchers,
            queue,
            looked,
            writer,
            volume,
            music,
            folder,
            _db: db_dir,
            _dir: dir,
        }
    }

    fn set_watch(&self, watch: bool) {
        let folder = self.folder;
        self.writer
            .call(move |c| set_watch(c, folder, watch))
            .unwrap();
        self.watchers.refresh();
    }

    /// Waits for the watch thread to have handled everything sent so far
    /// (a refresh queues its rescans before it answers this), then for the
    /// jobs to finish.
    fn settle(&self) -> Vec<MusicFolderId> {
        let watched = self.watchers.watched();
        wait_idle(&self.queue);
        watched
    }

    /// How many scans of `folder` have been queued so far.
    fn scans_of(&self, folder: MusicFolderId) -> usize {
        let target = format!(r#"{{"music_folder_ids":[{}]}}"#, folder.0);
        jobs(&self.writer)
            .into_iter()
            .filter(|j| j.0 == "scan" && j.1.as_deref() == Some(&target))
            .count()
    }
}

fn start(writer: &Writer, volume: &TempVolume, queue: &Arc<JobQueue>) -> Watchers {
    let queue = queue.clone();
    let enqueue: Enqueue = Arc::new(move |job| queue.enqueue(job));
    let volume = volume.clone();
    Watchers::start_with(
        QUIET,
        AT_MOST,
        writer.clone(),
        move || volume.clone(),
        enqueue,
    )
    .unwrap()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "timed out waiting until {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn turning_a_folders_watch_on_starts_its_watcher_and_off_stops_it() {
    let w = Watched::new(false);
    assert_eq!(w.settle(), none());
    assert_eq!(
        w.scans_of(w.folder),
        1,
        "rechecked at start, watched or not"
    );

    w.set_watch(true);
    assert_eq!(w.settle(), [w.folder]);
    assert_eq!(w.scans_of(w.folder), 2, "a watcher starting catches up");

    w.set_watch(false);
    assert_eq!(w.settle(), none());
    assert_eq!(w.scans_of(w.folder), 2);
    w.watchers.shutdown();
}

#[test]
fn a_watched_folder_is_scanned_once_at_start_not_once_per_reason() {
    // At start a watched folder is both "just online" and "just watched":
    // one scan, not two.
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    assert_eq!(w.scans_of(w.folder), 1);
    // A refresh with nothing new scans nothing.
    w.watchers.refresh();
    w.settle();
    assert_eq!(w.scans_of(w.folder), 1);
    w.watchers.shutdown();
}

#[test]
fn a_file_added_in_a_watched_folder_gets_indexed_and_read() {
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    assert_eq!(rows(&w.writer).len(), 0);

    put(&w.music, "New/track.mp3", &audio::mp3());
    wait_until("the new file is read", || {
        w.looked.read.load(std::sync::atomic::Ordering::SeqCst) == 1
    });
    wait_idle(&w.queue);
    let indexed = rows(&w.writer);
    assert_eq!(indexed.len(), 1);
    assert_eq!(indexed[0].rel_path, "New/track.mp3");
    // Read, hashed and fingerprinted by the chain, at background priority.
    assert_eq!(w.looked.hashed.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        w.looked
            .fingerprinted
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    let rescans: Vec<_> = jobs(&w.writer)
        .into_iter()
        .filter(|j| j.0 == "scan")
        .collect();
    assert_eq!(
        rescans.len(),
        2,
        "the catch-up scan and the burst's: {rescans:?}"
    );
    assert!(rescans.iter().all(|j| j.2 == Priority::BACKGROUND.0));
    w.watchers.shutdown();
}

#[test]
fn a_burst_of_changes_becomes_one_rescan() {
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    assert_eq!(w.scans_of(w.folder), 1);

    for i in 0..40 {
        put(&w.music, &format!("Burst/{i:02}.mp3"), &audio::mp3());
    }
    wait_until("every file of the burst is indexed", || {
        rows(&w.writer).len() == 40
    });
    wait_idle(&w.queue);
    assert_eq!(w.scans_of(w.folder), 2, "one rescan for the whole burst");
    w.watchers.shutdown();
}

#[test]
fn an_unplugged_root_stops_its_watcher_and_gets_it_back_with_a_catch_up_when_the_drive_returns() {
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    assert_eq!(w.scans_of(w.folder), 1);

    w.volume.set_online(false);
    w.watchers.refresh();
    assert_eq!(w.settle(), none());
    assert_eq!(w.scans_of(w.folder), 1, "an offline root isn't scanned");

    w.volume.set_online(true);
    w.watchers.refresh();
    assert_eq!(w.settle(), [w.folder]);
    // Back online and watched again: one scan, not one per reason.
    assert_eq!(w.scans_of(w.folder), 2, "back: one catch-up scan");
    // And it's really watching again.
    put(&w.music, "back.mp3", &audio::mp3());
    wait_until("the file added after the return is indexed", || {
        rows(&w.writer).len() == 1
    });
    wait_idle(&w.queue);
    w.watchers.shutdown();
}

#[test]
fn an_unwatched_folder_is_rechecked_at_start_and_when_its_drive_returns_but_not_on_changes() {
    let w = Watched::new(true);
    // A second music folder on the same drive, not watched. New to the
    // watch thread, it counts as just online at the next refresh.
    let other_path = w.volume.mount.join("Other");
    fs::create_dir(&other_path).unwrap();
    let other = add_music(&w.writer, &w.volume, &other_path);
    w.watchers.refresh();
    assert_eq!(w.settle(), [w.folder]);
    assert_eq!(w.scans_of(other), 1, "rechecked when first seen online");

    w.volume.set_online(false);
    w.watchers.refresh();
    w.settle();
    assert_eq!(w.scans_of(other), 1, "offline: not scanned");
    w.volume.set_online(true);
    w.watchers.refresh();
    assert_eq!(w.settle(), [w.folder]);
    assert_eq!(w.scans_of(other), 2, "rechecked when its drive is back");
    assert_eq!(w.scans_of(w.folder), 2);

    put(&other_path, "quiet.mp3", &audio::mp3());
    // Give a watcher that shouldn't exist time to react, then look.
    std::thread::sleep(QUIET + Duration::from_millis(500));
    wait_idle(&w.queue);
    assert_eq!(
        w.scans_of(other),
        2,
        "a change in an unwatched folder does nothing"
    );
    assert!(
        rows(&w.writer).is_empty(),
        "nothing indexed the unwatched folder's new file"
    );
    w.watchers.shutdown();
}

/// Every file under `dir`: bytes and modified time; every folder: its
/// modified time.
fn snapshot(dir: &Path, out: &mut BTreeMap<PathBuf, (Option<Vec<u8>>, SystemTime)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            snapshot(&path, out);
            out.insert(path, (None, meta.modified().unwrap()));
        } else {
            out.insert(
                path.clone(),
                (Some(fs::read(&path).unwrap()), meta.modified().unwrap()),
            );
        }
    }
}

#[test]
fn watching_and_the_chain_it_starts_write_nothing_outside_the_app_data_folder() {
    let (dir, volume, music) = drive();
    put(&music, "House/a.mp3", &audio::mp3());
    put(&music, "House/b.wav", &audio::wav());
    let mut before = BTreeMap::new();
    snapshot(dir.path(), &mut before);

    let (_db, writer, _reads) = db();
    let folder = add_music(&writer, &volume, &music);
    writer.call(move |c| set_watch(c, folder, true)).unwrap();
    let looked = LookedAt::default();
    let queue = Arc::new(chained_queue(&writer, &volume, &looked));
    let watchers = start(&writer, &volume, &queue);
    let _ = watchers.watched();
    wait_idle(&queue);
    // The catch-up scan and its chain read both files…
    assert_eq!(rows(&writer).len(), 2);
    assert_eq!(looked.read.load(std::sync::atomic::Ordering::SeqCst), 2);
    watchers.shutdown();
    // …and changed nothing on the drive.
    let mut after = BTreeMap::new();
    snapshot(dir.path(), &mut after);
    assert_eq!(after, before);
}

#[test]
fn the_frontend_turns_a_folders_watch_on_and_off() {
    let (_data, app) = app();
    let drive = tempfile::tempdir().unwrap();
    let music = fs::canonicalize(drive.path()).unwrap().join("Music");
    fs::create_dir_all(&music).unwrap();
    let path = crate::scan::display_path(&music);
    let added = invoke(
        &app,
        "add_music_folder",
        json!({ "path": path, "role": null }),
    )
    .unwrap();
    assert_eq!(added["watch"], json!(false));
    let id = added["id"].clone();

    let answer = invoke(
        &app,
        "set_music_folder_watch",
        json!({ "id": id, "watch": true }),
    );
    assert_eq!(answer, Ok(json!(null)));
    let listed = invoke(&app, "music_folders", json!({})).unwrap();
    assert_eq!(listed[0]["watch"], json!(true));
    // The app's own watchers picked it up (and queued its catch-up scan).
    let watchers = app.state::<Watchers>();
    assert_eq!(watchers.watched(), [MusicFolderId(id.as_i64().unwrap())]);

    let answer = invoke(
        &app,
        "set_music_folder_watch",
        json!({ "id": id, "watch": false }),
    );
    assert_eq!(answer, Ok(json!(null)));
    assert_eq!(watchers.watched(), none());
    assert_eq!(
        invoke(&app, "music_folders", json!({})).unwrap()[0]["watch"],
        json!(false)
    );

    assert_eq!(
        invoke(
            &app,
            "set_music_folder_watch",
            json!({ "id": 999, "watch": true })
        ),
        Err(json!({ "kind": "musicFolderNotFound", "params": {} }))
    );
    // Removing a watched folder stops its watcher too.
    invoke(
        &app,
        "set_music_folder_watch",
        json!({ "id": id, "watch": true }),
    )
    .unwrap();
    assert_eq!(watchers.watched().len(), 1);
    invoke(&app, "remove_music_folder", json!({ "id": id })).unwrap();
    assert_eq!(watchers.watched(), none());
    // The queue may still be walking the folder; let it finish before the
    // temp folders go.
    wait_idle(&app.state::<JobQueue>());
}
