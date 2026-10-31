//! The send review's warning about files changed after the export (1cB-4;
//! ROADMAP 1.10, "Second channel"): counted from the modified time the
//! scan recorded and the one on disk at prepare, a warning only, and never
//! a source of rekordbox's values. Times are set by hand; every name is
//! invented.

use super::*;
use crate::send::preflight::changed_after_export;

/// One millisecond, in the nanoseconds `file.mtime` holds.
const MS: i64 = 1_000_000;

impl World {
    /// When rekordbox saved the export last, as [`ExportRead::modified_ms`]
    /// gives it.
    fn export_ms(&self) -> i64 {
        i64::try_from(self.saved.get()).unwrap() * 1000
    }

    /// The scan recorded this modified time for `file`, in nanoseconds
    /// since the Unix epoch.
    fn file_modified(&self, file: i64, mtime_ns: i64) {
        self.insert("UPDATE file SET mtime = ?1 WHERE id = ?2", (mtime_ns, file));
    }

    fn linked_file(&self, track: LibraryTrackId) -> i64 {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT linked_file_id FROM library_track WHERE id = ?1",
                    [track.0],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    fn count_of(&self, table: &'static str) -> i64 {
        self.writer
            .call(move |c| c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
            .unwrap()
    }
}

fn listed(preflight: &Preflight) -> Vec<LibraryTrackId> {
    preflight
        .changed_after_export
        .iter()
        .map(|t| t.library_track)
        .collect()
}

#[test]
fn a_track_rekordbox_has_whose_file_changed_after_the_export_is_counted_and_listed() {
    let (w, known, _) = world_with_a_send_waiting();
    // One millisecond after the export was saved.
    w.file_modified(w.linked_file(known), (w.export_ms() + 1) * MS);

    let preflight = w.prepare().unwrap();
    assert_eq!(listed(&preflight), [known]);
    assert_eq!(
        preflight.changed_after_export[0].title.as_deref(),
        Some("Known")
    );
}

#[test]
fn a_file_changed_before_the_export_or_unchanged_since_is_not_counted() {
    let w = World::new();
    let (before, before_file) = w.library_track("before.mp3", "Before", true);
    let (same_ms, same_ms_file) = w.library_track("same.mp3", "Same", true);
    let (never_seen, _) = w.library_track("unstated.mp3", "Unstated", true);
    let (after, after_file) = w.library_track("after.mp3", "After", true);
    w.crate_of("Warm up", &[before, same_ms, never_seen, after]);
    w.save_export(&[
        Rb(40, "before.mp3", "Before"),
        Rb(41, "same.mp3", "Same"),
        Rb(42, "unstated.mp3", "Unstated"),
        Rb(43, "after.mp3", "After"),
    ]);
    let export = w.export_ms();
    // A minute before the export; within the export's own millisecond;
    // no modified time recorded at all; and the first nanosecond after it.
    w.file_modified(before_file, (export - 60_000) * MS);
    w.file_modified(same_ms_file, export * MS + MS - 1);
    w.file_modified(after_file, (export + 1) * MS);

    let preflight = w.prepare().unwrap();
    assert_eq!(preflight.known_tracks, 4);
    assert_eq!(listed(&preflight), [after]);
}

#[test]
fn a_file_no_library_track_links_a_track_the_send_leaves_out_and_a_new_track_are_not_counted() {
    let w = World::new();
    let (sent, _) = w.library_track("sent.mp3", "Sent", true);
    // rekordbox has it, but no crate names it: it isn't sent.
    let (_, loose_file) = w.library_track("loose.mp3", "Loose", true);
    // rekordbox doesn't have it: its values come from its own file.
    let (new, new_file) = w.library_track("new.mp3", "New", true);
    // In All music and in rekordbox, but in no Library track.
    let recording = w.insert("INSERT INTO recording (title) VALUES ('Stray')", ());
    let stray_file = w.file(recording, "stray.mp3", "Stray", true);
    w.crate_of("Warm up", &[sent, new]);
    w.save_export(&[
        Rb(40, "sent.mp3", "Sent"),
        Rb(41, "loose.mp3", "Loose"),
        Rb(42, "stray.mp3", "Stray"),
    ]);
    let after = (w.export_ms() + 1) * MS;
    for file in [loose_file, new_file, stray_file] {
        w.file_modified(file, after);
    }

    let preflight = w.prepare().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert!(listed(&preflight).is_empty());
}

#[test]
fn the_file_rekordboxs_entry_is_matched_to_is_watched_and_the_linked_file_only_without_one() {
    let w = World::new();
    let (known, known_file) = w.library_track("known.mp3", "Known", true);
    let (other, other_file) = w.library_track("other.mp3", "Other", true);
    w.crate_of("Warm up", &[known, other]);
    w.save_export(&[Rb(40, "known.mp3", "Known")]);
    let export = w.export_ms();
    w.prepare().unwrap();
    let before = (export - 60_000) * MS;
    let after = (export + 1) * MS;
    // `other` goes out as rekordbox's entry 40 (as a copy's entry does),
    // whose file is `known.mp3`.
    let sent_as_40 = |track: LibraryTrackId, in_rekordbox: bool| SentTrack {
        library_track: track,
        in_rekordbox,
        track_id: 40,
        attributes: Vec::new(),
        rekordbox_holds_other_file: None,
        file_missing: false,
    };
    let watched = |sent: SentTrack| {
        w.writer
            .call(move |c| changed_after_export(c, export, &[sent]))
            .unwrap()
    };

    // The entry's file changed, the linked file didn't: counted.
    w.file_modified(known_file, after);
    w.file_modified(other_file, before);
    assert_eq!(
        watched(sent_as_40(other, true)),
        [(other, known_file, true)]
    );
    // The linked file changed, the entry's file didn't: rekordbox writes
    // into the entry's file, so a change elsewhere is no edit of its.
    w.file_modified(known_file, before);
    w.file_modified(other_file, after);
    assert_eq!(
        watched(sent_as_40(other, true)),
        [(other, known_file, false)]
    );
    // A probable match isn't trusted: the linked file is watched instead.
    w.insert("UPDATE rekordbox_track SET relink_probable = 1", ());
    assert_eq!(
        watched(sent_as_40(other, true)),
        [(other, other_file, true)]
    );
    // A new track numbered 40 in the file isn't rekordbox's entry 40, and
    // isn't watched at all.
    assert!(watched(sent_as_40(other, false)).is_empty());
}

#[test]
fn a_refused_send_warns_about_nothing() {
    let (w, known, _) = world_with_a_send_waiting();
    // COLLECTION says three tracks and holds one.
    w.save_export_text(&export_text(
        &[Rb(40, "known.mp3", "Known in rekordbox")],
        3,
    ));
    w.file_modified(w.linked_file(known), (w.export_ms() + 1) * MS);

    let preflight = w.prepare().unwrap();
    assert!(preflight.refusal.is_some());
    assert!(listed(&preflight).is_empty());
}

#[test]
fn a_scan_between_the_review_and_the_go_changes_nothing_about_the_go() {
    let (w, known, _) = world_with_a_send_waiting();
    let preflight = w.prepare().unwrap();
    assert!(listed(&preflight).is_empty());
    // A scan lands after the review.
    w.file_modified(w.linked_file(known), (w.export_ms() + 1) * MS);
    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New"]);
}

#[test]
fn the_send_still_goes_with_the_warning_and_needs_no_confirm() {
    let (w, known, _) = world_with_a_send_waiting();
    w.file_modified(w.linked_file(known), (w.export_ms() + 1) * MS);

    let preflight = w.prepare().unwrap();
    assert_eq!(listed(&preflight), [known]);
    assert!(preflight.can_send && !preflight.needs_confirm);
    assert_eq!(preflight.refusal, None);

    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New"]);
}

#[test]
fn a_file_tag_is_never_taken_as_rekordboxs_value_and_the_warning_changes_nothing_sent() {
    let w = World::new();
    let (known, file) = w.library_track("known.mp3", "Known", true);
    w.crate_of("Warm up", &[known]);
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    // rekordbox wrote an edit into the file after the export: the scan
    // read a new title and a new modified time.
    let tags = json!({ "id3v2": [{"key": "TIT2", "value": {"type": "text", "text": "Retitled in the file"}}] })
        .to_string();
    w.insert("UPDATE file SET raw_tags = ?1 WHERE id = ?2", (tags, file));
    w.file_modified(file, (w.export_ms() + 30_000) * MS);

    let preflight = w.prepare().unwrap();
    assert_eq!(listed(&preflight), [known]);
    assert_eq!(w.go(false), None);
    // rekordbox's value went out, not the file's; the merge opened no
    // conflict and took nothing in as the app's own value.
    assert_eq!(w.sent_titles(), ["Known in rekordbox"]);
    assert_eq!((w.count_of("conflict"), w.count_of("app_value")), (0, 0));
    let warned = fs::read(w.send_file()).unwrap();

    // The same export saved again, after the file changed: no warning, and
    // the very same bytes go out.
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    let preflight = w.prepare().unwrap();
    assert!(listed(&preflight).is_empty());
    assert_eq!(w.go(false), None);
    assert_eq!(fs::read(w.send_file()).unwrap(), warned);
    assert_eq!((w.count_of("conflict"), w.count_of("app_value")), (0, 0));
}

// --- on disk -----------------------------------------------------------------

// Off Windows no drive is ever connected, and only the scan's answer counts.
#[cfg(windows)]
mod on_disk {
    use super::*;
    use crate::send::preflight::{changed_on_disk, Watched, LIST_FROM};

    /// The test volume, mounted at a folder of the sandbox, so its files are
    /// real files on a local drive.
    #[derive(Clone)]
    struct MountedAt(PathBuf);

    impl Volumes for MountedAt {
        fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
            Err(std::io::Error::other(format!(
                "{} isn't looked up in these tests",
                path.display()
            )))
        }

        fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
            (*id == volume_id()).then(|| self.0.clone())
        }
    }

    impl World {
        /// Where the test volume is mounted for [`World::prepare_on`], with its
        /// `Music` folder made: a folder of the sandbox, written as Windows
        /// writes a drive path.
        fn volume_root(&self) -> PathBuf {
            let root = PathBuf::from(
                self.base
                    .join("volume")
                    .to_string_lossy()
                    .trim_start_matches(r"\\?\"),
            );
            fs::create_dir_all(root.join("Music")).unwrap();
            root
        }

        /// The prepare step, with the test volume mounted at `root`.
        fn prepare_on(&self, root: &Path) -> Option<Preflight> {
            let root = root.to_path_buf();
            let sender = Sender::new(self.flow.clone(), move || MountedAt(root.clone()));
            // A send of 100,000 tracks takes minutes in a debug build.
            self.run_within(
                sender,
                self.flow.prepare_job(&self.export.to_string_lossy()),
                Duration::from_secs(480),
            );
            self.flow.preflight()
        }
    }

    /// An export of the files named, each `TrackID` with its file under
    /// `<root>\Music`.
    fn export_on(root: &Path, tracks: &[(u32, &str)]) -> String {
        let tracks: String = tracks
            .iter()
            .map(|(id, name)| {
                let location = location_of(&root.join("Music").join(name));
                format!(
                    "    <TRACK TrackID=\"{id}\" Name=\"{name}\" Artist=\"Kit\" Location=\"{location}\"/>\n"
                )
            })
            .collect();
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
    <DJ_PLAYLISTS Version="1.0.0">
      <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
      <COLLECTION Entries="{}">
    {tracks}  </COLLECTION>
      <PLAYLISTS>
        <NODE Type="0" Name="ROOT" Count="0"/>
      </PLAYLISTS>
    </DJ_PLAYLISTS>
    "#,
            tracks.lines().count()
        )
    }

    /// Sets the modified time of the file at `path` on disk, in milliseconds
    /// since the Unix epoch.
    fn touch_at(path: &Path, ms: i64) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(
                SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap()),
            )
            .unwrap();
    }

    /// rekordbox edited one file after the export and no scan has run since;
    /// `others` more files the scan saw, unchanged, share its folder.
    fn check_on_disk(others: usize) {
        let w = World::new();
        let root = w.volume_root();
        let music = root.join("Music");
        let mut names = vec!["edited.mp3".to_owned(), "untouched.mp3".to_owned()];
        names.extend((0..others).map(|i| format!("other-{i}.mp3")));
        let mut tracks = Vec::new();
        let mut files = Vec::new();
        for name in &names {
            let (track, file) = w.library_track(name, name, true);
            fs::write(music.join(name), b"").unwrap();
            tracks.push(track);
            files.push(file);
        }
        let (gone, gone_file) = w.library_track("gone.mp3", "Gone", true);
        tracks.push(gone);
        names.push("gone.mp3".to_owned());
        w.crate_of("Warm up", &tracks);
        let entries: Vec<(u32, &str)> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (40 + u32::try_from(i).unwrap(), name.as_str()))
            .collect();
        w.save_export_text(&export_on(&root, &entries));
        let export = w.export_ms();
        // The last scan saw every file on disk before the export.
        for (name, &file) in names.iter().zip(&files) {
            w.file_modified(file, (export - 60_000) * MS);
            touch_at(&music.join(name), export - 60_000);
        }
        // rekordbox then wrote an edit into one.
        touch_at(&music.join("edited.mp3"), export + 1_000);
        // A file the disk can't show: the scan's answer stands.
        w.file_modified(gone_file, (export + 1) * MS);

        let preflight = w.prepare_on(&root).unwrap();
        assert_eq!(preflight.known_tracks as usize, names.len());
        assert_eq!(listed(&preflight), [tracks[0], gone]);
    }

    #[test]
    fn a_file_changed_on_disk_after_the_export_is_counted_though_no_scan_has_seen_it() {
        // Few files in the folder: each is looked up on its own.
        check_on_disk(0);
    }

    #[test]
    fn a_file_changed_on_disk_is_counted_from_its_folders_listing_too() {
        // Enough to list the folder once instead.
        check_on_disk(LIST_FROM);
    }

    /// A send of `count` tracks rekordbox has, each on a real file, every tenth
    /// file modified on disk after the export with no scan since: times the
    /// whole prepare step, and the look at the disk alone.
    #[test]
    #[ignore = "makes 100,000 files; run with -- --ignored --nocapture"]
    fn preparing_a_send_of_100000_tracks_looks_at_every_file_once() {
        let count: i64 = 100_000;
        let w = World::new();
        let root = w.volume_root();
        let music = root.join("Music");
        let started = Instant::now();
        for i in 1..=count {
            fs::write(music.join(format!("f{i}.mp3")), b"").unwrap();
        }
        println!("made {count} files in {:?}", started.elapsed());
        w.writer
            .call(move |c| {
                c.execute_batch(&format!(
                    "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
                     INSERT INTO recording (id) SELECT i FROM n;
                     WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
                     INSERT INTO file (id, music_folder_id, rel_path, rel_path_key, present, size,
                                       sniffed_format, mtime)
                     SELECT i, 1, 'f' || i || '.mp3', 'f' || i || '.mp3', 1, 1234, 'mp3', 0 FROM n;
                     INSERT INTO recording_file (recording_id, file_id, role)
                     SELECT id, id, 'best' FROM file ORDER BY id;
                     INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                     SELECT id, 'linked', id, 'ok' FROM file ORDER BY id;
                     INSERT INTO crate (kind, name) VALUES ('static', 'All');
                     INSERT INTO crate_entry (crate_id, library_track_id, added_at)
                     SELECT 1, id, '2026-01-01T00:00:00.000Z' FROM library_track ORDER BY id;"
                ))
            })
            .unwrap();
        let names: Vec<String> = (1..=count).map(|i| format!("f{i}.mp3")).collect();
        let tracks: Vec<(u32, &str)> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (u32::try_from(i + 1).unwrap(), name.as_str()))
            .collect();
        w.save_export_text(&export_on(&root, &tracks));
        let export = w.export_ms();
        for i in (10..=count).step_by(10) {
            touch_at(&music.join(format!("f{i}.mp3")), export + 1_000);
        }

        let started = Instant::now();
        let preflight = w.prepare_on(&root).unwrap();
        println!(
            "prepared a send of {count} tracks in {:?}",
            started.elapsed()
        );
        assert_eq!(i64::from(preflight.known_tracks), count);
        assert_eq!(i64::try_from(listed(&preflight).len()).unwrap(), count / 10);

        let watched: Vec<Watched> = names
            .iter()
            .enumerate()
            .map(|(i, name)| Watched {
                track: LibraryTrackId(i64::try_from(i).unwrap() + 1),
                path: Some(crate::scan::display_path(&music.join(name))),
                recorded: false,
            })
            .collect();
        let started = Instant::now();
        let changed = changed_on_disk(&watched, export);
        println!("looked at {count} files on disk in {:?}", started.elapsed());
        assert_eq!(i64::try_from(changed.len()).unwrap(), count / 10);
    }

    /// How long the writer is held: a thread asks it to do nothing, over and
    /// over, so each answer takes as long as whatever the writer was doing
    /// (1cD-S1). Stands in for a user's save waiting on a send.
    struct WriterWatch {
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        thread: std::thread::JoinHandle<(Duration, Duration)>,
    }

    impl WriterWatch {
        fn start(writer: &Writer) -> WriterWatch {
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (writer, stopped) = (writer.clone(), stop.clone());
            let thread = std::thread::spawn(move || {
                let (mut longest, mut in_all) = (Duration::ZERO, Duration::ZERO);
                while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    let at = Instant::now();
                    writer.call(|_| Ok(())).unwrap();
                    let waited = at.elapsed();
                    longest = longest.max(waited);
                    if waited > Duration::from_millis(50) {
                        in_all += waited;
                    }
                }
                (longest, in_all)
            });
            WriterWatch { stop, thread }
        }

        fn report(self, step: &str) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let (longest, in_all) = self.thread.join().unwrap();
            println!("  writer during {step}: held {longest:?} at most, {in_all:?} in all");
        }
    }

    /// 1cC-M1: a whole send of 100,000 tracks rekordbox has, each on a real
    /// (empty) file, in one crate: the prepare step, then the go (which
    /// reads the export again, merges, builds the XML and reads it back).
    /// Prints; asserts nothing about time. Run alone, in a release build:
    /// `cargo test --release --lib preparing_and_sending_100000 -- --ignored --nocapture`.
    #[test]
    #[ignore = "makes 100,000 files; run with -- --ignored --nocapture"]
    fn preparing_and_sending_100000_tracks_cost() {
        let mib = crate::ops::history_tests::peak_working_set_mib;
        let count: i64 = 100_000;
        let w = World::new();
        let root = w.volume_root();
        let music = root.join("Music");
        let started = Instant::now();
        for i in 1..=count {
            fs::write(music.join(format!("f{i}.mp3")), b"").unwrap();
        }
        println!("made {count} files in {:?}", started.elapsed());
        w.writer
            .call(move |c| {
                c.execute_batch(&format!(
                    "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
                     INSERT INTO recording (id) SELECT i FROM n;
                     WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count})
                     INSERT INTO file (id, music_folder_id, rel_path, rel_path_key, present, size,
                                       sniffed_format, mtime)
                     SELECT i, 1, 'f' || i || '.mp3', 'f' || i || '.mp3', 1, 1234, 'mp3', 0 FROM n;
                     INSERT INTO recording_file (recording_id, file_id, role)
                     SELECT id, id, 'best' FROM file ORDER BY id;
                     INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                     SELECT id, 'linked', id, 'ok' FROM file ORDER BY id;
                     INSERT INTO crate (kind, name) VALUES ('static', 'All');
                     INSERT INTO crate_entry (crate_id, library_track_id, added_at)
                     SELECT 1, id, '2026-01-01T00:00:00.000Z' FROM library_track ORDER BY id;"
                ))
            })
            .unwrap();
        let names: Vec<String> = (1..=count).map(|i| format!("f{i}.mp3")).collect();
        let tracks: Vec<(u32, &str)> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (u32::try_from(i + 1).unwrap(), name.as_str()))
            .collect();
        w.save_export_text(&export_on(&root, &tracks));
        println!("peak memory after building the library: {} MiB", mib());

        let watch = WriterWatch::start(&w.writer);
        let started = Instant::now();
        let preflight = w.prepare_on(&root).unwrap();
        println!(
            "prepare of a send of {count} tracks: {:?}; peak {} MiB",
            started.elapsed(),
            mib()
        );
        watch.report("prepare");
        assert_eq!(i64::from(preflight.known_tracks), count);

        let on = root.clone();
        let sender = Sender::new(w.flow.clone(), move || MountedAt(on.clone()));
        let watch = WriterWatch::start(&w.writer);
        let started = Instant::now();
        let status = w.run_within(
            sender,
            w.flow
                .write_job(&preflight.token, preflight.needs_confirm, None),
            Duration::from_secs(580),
        );
        println!(
            "go of a send of {count} tracks: {:?} ({status:?}); peak {} MiB",
            started.elapsed(),
            mib()
        );
        watch.report("go");
        println!("failure: {:?}", w.flow.failure());
        println!(
            "the send file: {} MiB",
            fs::metadata(w.send_file()).map(|m| m.len()).unwrap_or(0) / (1024 * 1024)
        );

        // Where the review's time goes: its steps one by one (the prepare
        // runs them after the read and the relink; the go runs them again).
        let on = root.clone();
        w.writer
            .call(move |c| {
                let volumes = MountedAt(on);
                let at = Instant::now();
                let ids = crate::library::stored_ids(c)?;
                println!("  library::stored_ids: {:?}", at.elapsed());
                let at = Instant::now();
                let merged = crate::conflicts::merge_before_send(c, &volumes, &ids)?;
                println!("  conflicts::merge_before_send: {:?}", at.elapsed());
                let at = Instant::now();
                let crates = crate::send::crate_tree(c)?;
                println!("  crate_tree: {:?}", at.elapsed());
                let at = Instant::now();
                let input = crate::rekordbox_write::gather_merged(c, merged, crates, Vec::new())?;
                println!("  rekordbox_write::gather_merged: {:?}", at.elapsed());
                let at = Instant::now();
                let built = crate::rekordbox_write::build(&input);
                println!(
                    "  rekordbox_write::build: {:?} ({} sent)",
                    at.elapsed(),
                    built.map_or(0, |b| b.sent.len())
                );
                let at = Instant::now();
                let reviewed = crate::send::preflight::review(c, &volumes)?;
                println!(
                    "  the whole review again: {:?} ({})",
                    at.elapsed(),
                    reviewed.is_some()
                );
                Ok(())
            })
            .unwrap();
    }
}
