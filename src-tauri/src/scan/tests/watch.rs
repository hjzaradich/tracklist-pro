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
use std::time::{Duration, SystemTime};

use serde_json::json;
use tauri::Manager;

use super::chain::{chained_queue, chained_queue_with, jobs, wait_idle, wait_until, LookedAt};
use super::support::{db, TempVolume};
use super::walk::{add_music, drive, put, rows};
use crate::db::Writer;
use crate::ipc::testing::{app, invoke};
use crate::jobs::{JobQueue, Priority};
use crate::scan::chain::RESCAN_KEY;
use crate::scan::folders::MusicFolderId;
use crate::scan::walk::Walker;
use crate::scan::watch::{indexed_at, set_watch, Enqueue, Status, Watchers};
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

    /// How many scans of `folder` have been queued so far, marked as a
    /// watcher's rescan or not.
    fn scans_of(&self, folder: MusicFolderId) -> usize {
        scans_of(&self.writer, folder).len()
    }

    /// Whether each scan of `folder` so far was marked as a watcher's
    /// rescan (for changes) or not (start, drive back, watch turned on).
    fn rescan_marks_of(&self, folder: MusicFolderId) -> Vec<bool> {
        scans_of(&self.writer, folder)
    }
}

/// Each scan job of `folder` so far: whether it carries the rescan mark.
fn scans_of(writer: &Writer, folder: MusicFolderId) -> Vec<bool> {
    jobs(writer)
        .into_iter()
        .filter(|j| j.0 == "scan")
        .filter_map(|j| serde_json::from_str::<serde_json::Value>(&j.1?).ok())
        .filter(|t| t["music_folder_ids"] == json!([folder.0]))
        .map(|t| t[RESCAN_KEY] == json!(true))
        .collect()
}

/// Every job's kind and status, oldest first.
fn kinds_and_statuses(writer: &Writer) -> Vec<(String, String)> {
    jobs(writer).into_iter().map(|j| (j.0, j.3)).collect()
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
fn a_folder_added_with_its_scan_already_queued_is_not_scanned_again_but_one_found_new_is() {
    let w = Watched::new(false);
    w.settle();
    assert_eq!(w.scans_of(w.folder), 1);

    // Added by a caller that queues the scan itself and says so.
    let told = w.music.parent().unwrap().join("Told");
    std::fs::create_dir_all(&told).unwrap();
    let told = add_music(&w.writer, &w.volume, &told);
    w.watchers.added(told);
    w.settle();
    assert_eq!(w.scans_of(told), 0);

    // Found by a plain refresh: new here, so it's scanned.
    let found = w.music.parent().unwrap().join("Found");
    std::fs::create_dir_all(&found).unwrap();
    let found = add_music(&w.writer, &w.volume, &found);
    w.watchers.refresh();
    w.settle();
    assert_eq!(w.scans_of(found), 1);
    assert_eq!(w.scans_of(told), 0);
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

#[test]
fn a_burst_during_a_running_walk_folds_into_one_more_walk_after_it() {
    // A watched folder with a file, so its catch-up walk has an entry to
    // stop at.
    let (dir, volume, music) = drive();
    put(&music, "first.mp3", &audio::mp3());
    let (db_dir, writer, _reads) = db();
    let folder = add_music(&writer, &volume, &music);
    writer.call(move |c| set_watch(c, folder, true)).unwrap();
    let looked = LookedAt::default();
    // The walk, held at its first entry until released; once.
    let (release, held) = std::sync::mpsc::channel::<()>();
    let held = std::sync::Mutex::new(Some(held));
    let v = volume.clone();
    let queue = Arc::new(chained_queue_with(&writer, &volume, &looked, 2, |b| {
        b.handler(
            crate::jobs::JobKind::Scan,
            crate::scan::chain::after_walk(Walker::new(move || v.clone(), |_| {}).on_entry(
                move |_| {
                    let held = held.lock().unwrap().take();
                    if let Some(held) = held {
                        let _ = held.recv_timeout(Duration::from_secs(30));
                    }
                },
            )),
        )
    }));
    let watchers = start(&writer, &volume, &queue);
    let _ = watchers.watched();
    wait_until("the catch-up walk is running", || {
        kinds_and_statuses(&writer) == [("scan".into(), "running".into())]
    });

    // A change lands while the walk runs: its burst comes due, but no
    // second walk of the same root starts.
    put(&music, "second.mp3", &audio::mp3());
    std::thread::sleep(QUIET + Duration::from_millis(500));
    assert_eq!(
        kinds_and_statuses(&writer),
        [("scan".into(), "running".into())],
        "a second walk was queued beside the running one"
    );

    // When it ends, it runs once more, and the change is picked up. The
    // catch-up walk had no rescan mark, so neither does its rerun: it
    // retries everything the catch-up would have.
    release.send(()).unwrap();
    wait_until("both files are indexed", || rows(&writer).len() == 2);
    wait_idle(&queue);
    assert_eq!(scans_of(&writer, folder), [false, false]);
    watchers.shutdown();
    queue.shutdown();
    drop((db_dir, dir));
}

#[test]
fn changes_the_walk_would_never_index_do_not_rescan_but_a_finished_download_does() {
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    assert_eq!(w.scans_of(w.folder), 1);

    put(&w.music, "notes.txt", b"notes");
    put(&w.music, "._a.mp3", b"\x00\x05\x16\x07");
    put(&w.music, ".DS_Store", b"\x00");
    put(&w.music, "a.mp3.crdownload", &audio::mp3());
    put(&w.music, "cover.jpg", b"jpg");
    std::thread::sleep(QUIET + Duration::from_millis(500));
    wait_idle(&w.queue);
    assert_eq!(
        w.scans_of(w.folder),
        1,
        "nothing the walk could index changed"
    );

    // The download finishes: renamed to an audio name.
    fs::rename(w.music.join("a.mp3.crdownload"), w.music.join("a.mp3")).unwrap();
    wait_until("the finished download is indexed", || {
        rows(&w.writer).iter().any(|r| r.rel_path == "a.mp3")
    });
    wait_idle(&w.queue);
    assert_eq!(w.rescan_marks_of(w.folder), [false, true]);
    w.watchers.shutdown();
}

#[test]
fn a_locked_file_does_not_make_every_rescan_run_the_stages_again() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", &audio::mp3());
    put(&music, "locked.mp3", &audio::mp3());
    // Held open with no sharing: every stage finds it unreachable.
    let _lock = fs::File::options()
        .read(true)
        .share_mode(0)
        .open(music.join("locked.mp3"))
        .unwrap();
    let (_db, writer, _reads) = db();
    let folder = add_music(&writer, &volume, &music);
    writer.call(move |c| set_watch(c, folder, true)).unwrap();
    let looked = LookedAt::default();
    let queue = Arc::new(chained_queue(&writer, &volume, &looked));
    let watchers = start(&writer, &volume, &queue);
    let _ = watchers.watched();
    wait_idle(&queue);
    // The catch-up: every stage ran, and skipped the locked file.
    let kinds = |writer: &Writer| -> Vec<String> {
        kinds_and_statuses(writer)
            .into_iter()
            .map(|k| k.0)
            .collect()
    };
    assert_eq!(
        kinds(&writer),
        ["scan", "read", "hash", "group", "fingerprint"]
    );
    let skipped: Vec<(String, String)> = writer
        .call(|c| {
            let mut s = c.prepare(
                "SELECT s.stage, s.reason FROM file_stage s JOIN file f ON f.id = s.file_id
                 WHERE f.rel_path = 'locked.mp3' ORDER BY s.stage",
            )?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap();
    assert_eq!(
        skipped,
        [
            ("fingerprint".into(), "unreachable".into()),
            ("hash".into(), "unreachable".into()),
            ("read".into(), "unreachable".into()),
        ]
    );

    // A change with nothing new for the stages (a file removed): the
    // watcher's rescan runs, and no stage runs for the locked file alone.
    fs::remove_file(music.join("a.mp3")).unwrap();
    wait_until("the removed file is marked missing", || {
        rows(&writer)
            .iter()
            .any(|r| r.rel_path == "a.mp3" && !r.present)
    });
    wait_idle(&queue);
    assert_eq!(
        kinds(&writer),
        ["scan", "read", "hash", "group", "fingerprint", "scan"]
    );

    // A scan that isn't the watcher's own (asked for here; at app start
    // and on a drive's return the same) retries it.
    queue
        .enqueue(crate::scan::scan_job(Some(vec![folder])))
        .unwrap();
    wait_idle(&queue);
    assert_eq!(
        kinds(&writer),
        [
            "scan",
            "read",
            "hash",
            "group",
            "fingerprint",
            "scan",
            "scan",
            "read",
            "hash",
            "group",
            "fingerprint"
        ]
    );
    watchers.shutdown();
    queue.shutdown();
}

/// Sends the eject window one of Windows' handle messages for
/// `registration`, as Windows does before and after an eject.
fn send_handle_message(watchers: &Watchers, event: u32, registration: isize) {
    use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageW, DBT_DEVTYP_HANDLE, DEV_BROADCAST_HANDLE, WM_DEVICECHANGE,
    };
    let window = watchers.eject_window().expect("the eject window exists");
    let body = DEV_BROADCAST_HANDLE {
        dbch_size: std::mem::size_of::<DEV_BROADCAST_HANDLE>() as u32,
        dbch_devicetype: DBT_DEVTYP_HANDLE,
        dbch_hdevnotify: registration as *mut _,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            window.window(),
            WM_DEVICECHANGE,
            event as WPARAM,
            &body as *const DEV_BROADCAST_HANDLE as LPARAM,
        )
    };
}

#[test]
fn an_eject_stops_the_watcher_and_closes_its_handle_and_a_refused_eject_starts_it_again() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DBT_DEVICEQUERYREMOVE, DBT_DEVICEQUERYREMOVEFAILED,
    };
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    let status = w.watchers.status();
    assert_eq!(status.handles_open, 1, "a query-only handle on the root");
    assert_eq!(status.registrations.len(), 1);
    let registration = status.registrations[0].1;

    // Windows asks whether the drive may go: by the time it has its
    // answer, the watcher is stopped and the handle closed.
    send_handle_message(&w.watchers, DBT_DEVICEQUERYREMOVE, registration);
    let status = w.watchers.status();
    assert_eq!(status.watched, none());
    assert_eq!(status.suspended, [w.folder]);
    assert_eq!(status.handles_open, 0);
    assert_eq!(w.scans_of(w.folder), 1);

    // Something else refused the eject: the watcher starts again, catches
    // up, and hears changes again.
    send_handle_message(&w.watchers, DBT_DEVICEQUERYREMOVEFAILED, registration);
    let status = w.watchers.status();
    assert_eq!(status.watched, [w.folder]);
    assert_eq!(status.suspended, none());
    assert_eq!(status.handles_open, 1);
    assert_eq!(w.rescan_marks_of(w.folder), [false, true]);
    put(&w.music, "after.mp3", &audio::mp3());
    wait_until("a file added after the refused eject is indexed", || {
        rows(&w.writer).len() == 1
    });
    wait_idle(&w.queue);

    // An eject that goes through: the drive goes offline, and the root
    // is let go of entirely.
    let registration = w.watchers.status().registrations[0].1;
    send_handle_message(&w.watchers, DBT_DEVICEQUERYREMOVE, registration);
    assert_eq!(w.watchers.status().handles_open, 0);
    w.volume.set_online(false);
    w.watchers.refresh();
    let status = w.watchers.status();
    assert_eq!(status, Status::default());
    w.watchers.shutdown();
}

#[test]
fn temp_files_renamed_away_or_removed_next_to_the_music_do_not_rescan() {
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    // A track added under the watcher: its burst is the second scan, and
    // the baseline from here on.
    put(&w.music, "track.mp3", &audio::mp3());
    wait_until("the track is indexed", || rows(&w.writer).len() == 1);
    wait_idle(&w.queue);
    assert_eq!(w.scans_of(w.folder), 2);

    // Three autosaves of a project file next to the music (write a temp
    // file, rename it over the target), a temp file made and removed, and
    // a download that ends up with a non-audio name.
    for _ in 0..3 {
        put(&w.music, "set.tmp", b"project");
        fs::rename(w.music.join("set.tmp"), w.music.join("set.als")).unwrap();
    }
    put(&w.music, "scratch.tmp", b"x");
    fs::remove_file(w.music.join("scratch.tmp")).unwrap();
    put(&w.music, "notes.pdf.crdownload", b"pdf");
    fs::rename(
        w.music.join("notes.pdf.crdownload"),
        w.music.join("notes.pdf"),
    )
    .unwrap();
    std::thread::sleep(QUIET + Duration::from_millis(500));
    wait_idle(&w.queue);
    assert_eq!(w.scans_of(w.folder), 2, "nothing the index knew changed");

    // A file the index knows, renamed away: that's a change.
    fs::rename(w.music.join("track.mp3"), w.music.join("track.mp3.bak")).unwrap();
    wait_until("the renamed-away track is marked missing", || {
        rows(&w.writer)
            .iter()
            .any(|r| r.rel_path == "track.mp3" && !r.present)
    });
    wait_idle(&w.queue);
    assert_eq!(w.scans_of(w.folder), 3);
    w.watchers.shutdown();
}

#[test]
fn a_burst_of_gone_paths_asks_the_index_once_per_path_and_not_at_all_once_a_rescan_is_due() {
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    let lookups = || w.watchers.status().index_lookups;
    assert_eq!(lookups(), 0);

    // The same temp file written and removed twenty times, and five other
    // temp files removed once each: six distinct gone paths, six lookups.
    for _ in 0..20 {
        put(&w.music, "same.tmp", b"x");
        fs::remove_file(w.music.join("same.tmp")).unwrap();
    }
    for n in 0..5 {
        put(&w.music, &format!("other {n}.tmp"), b"x");
        fs::remove_file(w.music.join(format!("other {n}.tmp"))).unwrap();
    }
    std::thread::sleep(QUIET + Duration::from_millis(500));
    wait_idle(&w.queue);
    assert_eq!(w.scans_of(w.folder), 1, "none of them was indexed");
    assert_eq!(lookups(), 6);

    // With a rescan already due for the root, gone paths aren't looked up.
    put(&w.music, "new.mp3", &audio::mp3());
    for n in 0..5 {
        put(&w.music, &format!("late {n}.tmp"), b"x");
        fs::remove_file(w.music.join(format!("late {n}.tmp"))).unwrap();
    }
    wait_until("the new track is indexed", || rows(&w.writer).len() == 1);
    wait_idle(&w.queue);
    assert_eq!(w.scans_of(w.folder), 2);
    assert_eq!(lookups(), 6);
    w.watchers.shutdown();
}

#[test]
fn a_refresh_during_an_eject_does_not_reopen_the_root_until_the_grace_is_out() {
    use windows_sys::Win32::UI::WindowsAndMessaging::DBT_DEVICEQUERYREMOVE;
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    let registration = w.watchers.status().registrations[0].1;
    send_handle_message(&w.watchers, DBT_DEVICEQUERYREMOVE, registration);
    assert_eq!(w.watchers.status().handles_open, 0);

    // Anything that refreshes meanwhile (another drive, a toggle, a
    // folder removed) leaves the root alone: reopening it would make
    // Windows refuse the eject as "in use".
    w.watchers.refresh();
    let status = w.watchers.status();
    assert_eq!(status.watched, none());
    assert_eq!(status.suspended, [w.folder]);
    assert_eq!(status.handles_open, 0);
    assert_eq!(w.scans_of(w.folder), 1);

    // No refusal ever comes and the drive is still here: once the grace
    // is out, a refresh watches it again.
    w.watchers.set_eject_grace(Duration::ZERO);
    w.watchers.refresh();
    let status = w.watchers.status();
    assert_eq!(status.watched, [w.folder]);
    assert_eq!(status.suspended, none());
    assert_eq!(status.handles_open, 1);
    w.settle();
    w.watchers.shutdown();
}

#[test]
fn a_removed_folder_that_still_has_indexed_files_rescans_and_they_are_marked_missing() {
    let w = Watched::new(true);
    assert_eq!(w.settle(), [w.folder]);
    put(&w.music, "Album/a.mp3", &audio::mp3());
    put(&w.music, "Album/b.mp3", &audio::mp3());
    wait_until("the album is indexed", || rows(&w.writer).len() == 2);
    wait_idle(&w.queue);
    assert_eq!(w.scans_of(w.folder), 2);

    fs::remove_dir_all(w.music.join("Album")).unwrap();
    wait_until("the album's files are marked missing", || {
        rows(&w.writer).iter().all(|r| !r.present)
    });
    wait_idle(&w.queue);
    assert_eq!(w.scans_of(w.folder), 3);
    w.watchers.shutdown();
}

#[test]
fn a_gone_path_is_looked_up_by_its_exact_spelling_or_the_range_of_paths_under_it() {
    let (_db, writer, reads) = db();
    let (_dir, volume, music) = drive();
    let folder = add_music(&writer, &volume, &music);
    for rel in [
        "dir/x.mp3",
        "dir0/y.mp3",
        "dir-b/z.mp3",
        "d%r/w.mp3",
        "d_r/v.mp3",
        "dirt.mp3",
        "Deep/er/still.mp3",
    ] {
        put(&music, rel, &audio::mp3());
    }
    let q = Arc::new(chained_queue(&writer, &volume, &LookedAt::default()));
    q.enqueue(crate::scan::scan_job(Some(vec![folder])))
        .unwrap();
    wait_idle(&q);
    assert_eq!(rows(&writer).len(), 7);
    let indexed = |rel: &str| reads.read(|c| indexed_at(c, folder, rel)).unwrap();

    // A folder: the range of paths under it, and nothing beside it.
    assert!(indexed("dir"));
    assert!(indexed("dir0"));
    assert!(indexed("dir-b"));
    assert!(indexed("Deep"));
    assert!(indexed("Deep/er"));
    assert!(!indexed("di"), "a prefix that isn't a folder");
    assert!(!indexed("dirt"), "a file's stem isn't a folder");
    assert!(!indexed("dir/x"), "a file's stem isn't a folder either");
    // A file: its exact spelling.
    assert!(indexed("dir/x.mp3"));
    assert!(indexed("dirt.mp3"));
    assert!(!indexed("dir/y.mp3"));
    // Nothing is a wildcard.
    assert!(indexed("d%r"));
    assert!(indexed("d_r"));
    assert!(!indexed("d%"));
    assert!(!indexed("d_"));
    assert!(!indexed("dxr"));
    assert!(!indexed("%"));
    assert!(!indexed("_"));
    // The on-disk spelling, which is what an event names.
    assert!(!indexed("DIR"));
    q.shutdown();
}
