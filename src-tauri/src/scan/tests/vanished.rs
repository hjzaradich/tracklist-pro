#![cfg(test)]
//! 1aB-9: files gone from a folder the walk could list are marked missing
//! (`present = 0`), never deleted; a folder whose drive is unplugged is
//! offline, and its files keep `present = 1`; a drive that goes away or is
//! swapped mid-walk stops that folder without marking anything.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::support::{db, serial, TempVolume};
use super::unreadable::Unlistable;
use super::walk::{add_music, at, drive, many, put, rows, scan, scan_with, wait};
use crate::db::Writer;
use crate::jobs::{JobKind, JobQueue, JobStatus};
use crate::paths::{RelPath, Volumes};
use crate::scan::folders::{stored, stored_one, MusicFolderId};
use crate::scan::walk::{finish, scan_job, Unread, Walker, BATCH_MAX};
use crate::volume::{Volume, VolumeId, VolumeKind};

/// Each row's path and whether it's present.
fn presence(writer: &Writer) -> BTreeMap<String, bool> {
    rows(writer)
        .into_iter()
        .map(|r| (r.rel_path, r.present))
        .collect()
}

fn ids(writer: &Writer) -> BTreeMap<String, i64> {
    rows(writer)
        .into_iter()
        .map(|r| (r.rel_path, r.id))
        .collect()
}

fn walked_at(writer: &Writer, id: MusicFolderId) -> Option<String> {
    writer
        .call(move |c| stored_one(c, id))
        .unwrap()
        .unwrap()
        .walked_at
}

#[test]
fn a_file_gone_from_a_connected_folder_is_marked_missing_and_present_again_when_it_is_back() {
    let (_dir, volume, music) = drive();
    put(&music, "House/a.mp3", b"a");
    put(&music, "House/b.mp3", b"b");
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);
    let before = ids(&writer);

    let parked = at(&music, "b.mp3.parked");
    fs::rename(at(&music, "House/b.mp3"), &parked).unwrap();
    scan(&writer, &volume);
    assert_eq!(
        presence(&writer),
        BTreeMap::from([("House/a.mp3".into(), true), ("House/b.mp3".into(), false)])
    );
    // Never deleted: the same rows.
    assert_eq!(ids(&writer), before);

    fs::rename(&parked, at(&music, "House/b.mp3")).unwrap();
    scan(&writer, &volume);
    assert_eq!(
        presence(&writer),
        BTreeMap::from([("House/a.mp3".into(), true), ("House/b.mp3".into(), true)])
    );
    assert_eq!(ids(&writer), before);
}

#[test]
fn a_whole_subfolder_deleted_marks_each_of_its_files_missing() {
    let (_dir, volume, music) = drive();
    put(&music, "Keep/a.mp3", b"a");
    put(&music, "Gone/b.mp3", b"b");
    put(&music, "Gone/Deeper/c.mp3", b"c");
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    fs::remove_dir_all(at(&music, "Gone")).unwrap();
    scan(&writer, &volume);
    assert_eq!(
        presence(&writer),
        BTreeMap::from([
            ("Gone/Deeper/c.mp3".into(), false),
            ("Gone/b.mp3".into(), false),
            ("Keep/a.mp3".into(), true),
        ])
    );
}

#[test]
fn files_under_a_subfolder_the_walk_could_not_list_are_not_marked_missing() {
    let (_dir, volume, music) = drive();
    put(&music, "Open/a.mp3", b"a");
    put(&music, "Locked/b.mp3", b"b");
    put(&music, "Locked/Deeper/c.mp3", b"c");
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    // A file really gone from the open folder is still noticed.
    fs::remove_file(at(&music, "Open/a.mp3")).unwrap();
    let _locked = Unlistable::new(&at(&music, "Locked"));
    scan(&writer, &volume);
    assert_eq!(
        presence(&writer),
        BTreeMap::from([
            ("Locked/Deeper/c.mp3".into(), true),
            ("Locked/b.mp3".into(), true),
            ("Open/a.mp3".into(), false),
        ])
    );
}

#[test]
fn a_music_folder_the_walk_could_not_list_at_all_marks_nothing_missing() {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"a");
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);

    let _locked = Unlistable::new(&music);
    scan(&writer, &volume);
    assert_eq!(presence(&writer), BTreeMap::from([("a.mp3".into(), true)]));
}

#[test]
fn unplugging_a_drive_makes_its_folder_offline_and_keeps_its_files_present_and_replugging_brings_it_back(
) {
    let (_dir, volume, music) = drive();
    put(&music, "a.mp3", b"a");
    put(&music, "b.mp3", b"b");
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);
    let rows_before = rows(&writer);
    let folder = || writer.call(|c| stored(c)).unwrap().remove(0);
    assert!(folder().to_music_folder(&volume).online);

    volume.set_online(false);
    // Greyed out in the app: offline, shown where it was last seen.
    let offline = folder().to_music_folder(&volume);
    assert!(!offline.online);
    assert_eq!(offline.path, crate::scan::display_path(&music));
    // A scan while it's unplugged changes nothing: offline, not missing.
    scan(&writer, &volume);
    assert_eq!(rows(&writer), rows_before);

    volume.set_online(true);
    assert!(folder().to_music_folder(&volume).online);
    scan(&writer, &volume);
    assert_eq!(
        presence(&writer),
        BTreeMap::from([("a.mp3".into(), true), ("b.mp3".into(), true)])
    );
}

#[test]
fn a_drive_unplugged_mid_walk_stops_that_folder_and_marks_nothing_missing() {
    let (_dir, volume, music) = drive();
    many(&music, BATCH_MAX * 2);
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);
    scan(&writer, &volume);
    let rows_before = rows(&writer);
    let first_walk = walked_at(&writer, id);

    // New files the second walk would find, then the drive goes away
    // after the first batch's worth of entries.
    many(&music, BATCH_MAX * 3);
    let unplug = volume.clone();
    let walker = Walker::new(
        {
            let volume = volume.clone();
            move || volume.clone()
        },
        |_| {},
    )
    .batch_window(Duration::from_secs(3600))
    .on_entry(move |n| {
        if n == (BATCH_MAX + BATCH_MAX / 2) as u64 {
            unplug.set_online(false);
        }
    });
    assert_eq!(scan_with(&writer, walker, None), JobStatus::Done);

    // Every row it knew is still present, and the last full walk stands.
    let after = rows(&writer);
    for row in &rows_before {
        let now = after.iter().find(|r| r.id == row.id).unwrap();
        assert!(now.present, "{} was marked missing", row.rel_path);
    }
    assert_eq!(walked_at(&writer, id), first_walk);
    // Nothing found after the drive went away was written.
    assert!(after.len() < BATCH_MAX * 3, "{} rows", after.len());
}

/// A volume whose drive the test can swap for another on the same mount
/// point, the way a second USB drive takes the first one's letter.
#[derive(Clone)]
struct Swappable {
    mount: PathBuf,
    first: VolumeId,
    second: VolumeId,
    swapped: Arc<AtomicBool>,
}

impl Swappable {
    fn plugged_in(&self) -> VolumeId {
        if self.swapped.load(Ordering::SeqCst) {
            self.second.clone()
        } else {
            self.first.clone()
        }
    }
}

impl Volumes for Swappable {
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    fn volume_for(&self, _path: &Path) -> io::Result<Volume> {
        Ok(Volume {
            id: self.plugged_in(),
            label: String::new(),
            mount_path: self.mount.clone(),
            kind: VolumeKind::External,
        })
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (*id == self.plugged_in()).then(|| self.mount.clone())
    }
}

#[test]
fn a_different_drive_swapped_in_on_the_same_mount_mid_walk_is_not_indexed_as_the_first() {
    let (_dir, volume, music) = drive();
    many(&music, BATCH_MAX * 3);
    let swappable = Swappable {
        mount: volume.mount.clone(),
        first: volume.id.clone(),
        second: serial(0x5EC0_4D00),
        swapped: Arc::default(),
    };
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);

    let swap = swappable.swapped.clone();
    let walker = Walker::new(
        {
            let swappable = swappable.clone();
            move || swappable.clone()
        },
        |_| {},
    )
    .batch_window(Duration::from_secs(3600))
    .on_entry(move |n| {
        if n == (BATCH_MAX + BATCH_MAX / 2) as u64 {
            swap.store(true, Ordering::SeqCst);
        }
    });
    assert_eq!(scan_with(&writer, walker, None), JobStatus::Done);

    // Only the first batch, found before the swap, is in the first
    // drive's folder; nothing was marked missing or counted as walked.
    assert_eq!(rows(&writer).len(), BATCH_MAX);
    assert!(rows(&writer).iter().all(|r| r.present));
    assert_eq!(walked_at(&writer, id), None);
}

/// A flat music folder holding `names`, walked once in full.
struct Walked {
    _drive: tempfile::TempDir,
    _data: tempfile::TempDir,
    volume: TempVolume,
    music: PathBuf,
    writer: Writer,
}

fn walked_once(names: &[&str]) -> Walked {
    let (drive_dir, volume, music) = drive();
    for name in names {
        put(&music, name, name.as_bytes());
    }
    let (data, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    scan(&writer, &volume);
    Walked {
        _drive: drive_dir,
        _data: data,
        volume,
        music,
        writer,
    }
}

#[test]
fn a_drive_unplugged_after_the_last_batch_but_before_the_walk_ends_marks_nothing_missing() {
    let Walked {
        volume,
        music,
        writer,
        // Bound, so the temp dirs live to the end of the test.
        _drive,
        _data,
    } = walked_once(&["a.mp3", "b.mp3", "c.mp3"]);
    fs::remove_file(at(&music, "b.mp3")).unwrap();

    // Two entries left (a, c); the drive goes just as the last is looked at.
    let unplug = volume.clone();
    let walker = Walker::new(
        {
            let volume = volume.clone();
            move || volume.clone()
        },
        |_| {},
    )
    .on_entry(move |n| {
        if n == 1 {
            unplug.set_online(false);
        }
    });
    assert_eq!(scan_with(&writer, walker, None), JobStatus::Done);
    assert!(presence(&writer)["b.mp3"], "b.mp3 was marked missing");
}

#[test]
fn a_cancelled_walk_marks_nothing_missing() {
    let Walked {
        volume,
        music,
        writer,
        // Bound, so the temp dirs live to the end of the test.
        _drive,
        _data,
    } = walked_once(&["a.mp3", "b.mp3", "c.mp3", "d.mp3"]);
    fs::remove_file(at(&music, "b.mp3")).unwrap();

    let (reached, at_stop) = std::sync::mpsc::channel::<()>();
    let (resume, cancelled) = std::sync::mpsc::channel::<()>();
    let (reached, cancelled) = (Mutex::new(reached), Mutex::new(cancelled));
    let walker = Walker::new(
        {
            let volume = volume.clone();
            move || volume.clone()
        },
        |_| {},
    )
    .on_entry(move |n| {
        if n == 1 {
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
    q.cancel(id).unwrap();
    resume.send(()).unwrap();
    assert_eq!(wait(&writer, id), JobStatus::Cancelled);
    q.shutdown();
    assert!(presence(&writer)["b.mp3"], "b.mp3 was marked missing");
}

/// A drive whose place in the drive list hasn't caught up with a swap yet:
/// its mount point still answers for the first drive, but the drive itself
/// now says it's another.
#[derive(Clone)]
struct LaggingList {
    inner: Swappable,
}

impl Volumes for LaggingList {
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        self.inner.real_path(path)
    }

    fn volume_for(&self, path: &Path) -> io::Result<Volume> {
        self.inner.volume_for(path)
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (*id == self.inner.first).then(|| self.inner.mount.clone())
    }
}

#[test]
fn a_swap_the_drive_list_has_not_caught_up_with_still_stops_the_walk() {
    let (_dir, volume, music) = drive();
    many(&music, BATCH_MAX * 3);
    let lagging = LaggingList {
        inner: Swappable {
            mount: volume.mount.clone(),
            first: volume.id.clone(),
            second: serial(0x5EC0_4D01),
            swapped: Arc::default(),
        },
    };
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);

    let swap = lagging.inner.swapped.clone();
    let walker = Walker::new(
        {
            let lagging = lagging.clone();
            move || lagging.clone()
        },
        |_| {},
    )
    .batch_window(Duration::from_secs(3600))
    .on_entry(move |n| {
        if n == (BATCH_MAX + BATCH_MAX / 2) as u64 {
            swap.store(true, Ordering::SeqCst);
        }
    });
    assert_eq!(scan_with(&writer, walker, None), JobStatus::Done);
    assert_eq!(rows(&writer).len(), BATCH_MAX);
    assert_eq!(walked_at(&writer, id), None);
}

/// A drive whose serial stays the same while its GUID changes mid-walk:
/// a sector clone swapped in on the same letter, which the drive list
/// hasn't caught up with.
#[derive(Clone)]
struct GuidSwap {
    volume: TempVolume,
    swapped: Arc<AtomicBool>,
}

impl Volumes for GuidSwap {
    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        self.volume.real_path(path)
    }

    fn volume_for(&self, path: &Path) -> io::Result<Volume> {
        self.volume.volume_for(path)
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        self.volume.mount_path(id)
    }

    fn sighting_for(&self, path: &Path) -> io::Result<crate::volume::Sighting> {
        let guid = if self.swapped.load(Ordering::SeqCst) {
            "{bbbbbbbb-0000-0000-0000-000000000002}"
        } else {
            "{aaaaaaaa-0000-0000-0000-000000000001}"
        };
        Ok(crate::volume::Sighting {
            volume: self.volume.volume_for(path)?,
            guid: Some(guid.to_owned()),
        })
    }
}

#[test]
fn a_clone_with_the_same_serial_swapped_in_mid_walk_stops_the_walk() {
    let (_dir, volume, music) = drive();
    many(&music, BATCH_MAX * 3);
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);
    let swapping = GuidSwap {
        volume,
        swapped: Arc::default(),
    };

    let swap = swapping.swapped.clone();
    let walker = Walker::new(
        {
            let swapping = swapping.clone();
            move || swapping.clone()
        },
        |_| {},
    )
    .batch_window(Duration::from_secs(3600))
    .on_entry(move |n| {
        if n == (BATCH_MAX + BATCH_MAX / 2) as u64 {
            swap.store(true, Ordering::SeqCst);
        }
    });
    assert_eq!(scan_with(&writer, walker, None), JobStatus::Done);
    assert_eq!(rows(&writer).len(), BATCH_MAX);
    assert_eq!(walked_at(&writer, id), None);
}

#[test]
fn finishing_a_walk_skips_every_folder_whose_listing_broke_off() {
    let (_dir, volume, music) = drive();
    let (_db, writer, _reads) = db();
    let id = add_music(&writer, &volume, &music);
    // Rows seen by an earlier walk, none seen by this one.
    let found = |rel: &str| crate::scan::walk::Found {
        rel: RelPath::parse(rel).unwrap(),
        size: 1,
        mtime_ns: 0,
        file_id: None,
        online_only: false,
    };
    writer
        .call(move |c| {
            let rows = [
                found("Broke/a.mp3"),
                found("Broke/Deeper/b.mp3"),
                found("Brokeless/c.mp3"),
                found("d.mp3"),
            ];
            crate::scan::walk::upsert(c, id, &rows, "2000-01-01T00:00:00.000Z")?;
            c.execute(
                "UPDATE file SET last_seen_at = '2000-01-01T00:00:00.000Z'",
                [],
            )
        })
        .unwrap();

    let unread = Unread {
        folders: 1,
        files: 0,
        unlisted: vec![RelPath::parse("Broke").unwrap()],
    };
    let missing = writer
        .call(move |c| finish(c, id, &unread, "2026-01-01T00:00:00.000Z"))
        .unwrap();
    assert_eq!(missing, 2);
    assert_eq!(
        presence(&writer),
        BTreeMap::from([
            ("Broke/Deeper/b.mp3".into(), true),
            ("Broke/a.mp3".into(), true),
            ("Brokeless/c.mp3".into(), false),
            ("d.mp3".into(), false),
        ])
    );
}
