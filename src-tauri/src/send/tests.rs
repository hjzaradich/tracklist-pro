//! The send flow, end to end over a migrated database and a sandbox on
//! disk. Everything is synthetic: a made-up volume, file rows, tags and
//! rekordbox exports. No audio file exists or is opened.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde_json::json;

use super::file::{send_path, SEND_FILE_NAME};
use super::*;
use crate::db::Writer;
use crate::jobs::{self, JobKind, JobQueue, JobStatus, NewJob};
use crate::paths::Volumes;
use crate::rekordbox::RekordboxXml;
use crate::rekordbox_write::{BuildError, SentTrack};
use crate::volume::{identity, IdentitySignals, Volume, VolumeId, VolumeKind};

fn volume_id() -> VolumeId {
    identity(IdentitySignals {
        kind: VolumeKind::External,
        unc_share: None,
        serial: Some(0x1A2B_3C4D),
        filesystem: "NTFS",
        guid: None,
    })
    .unwrap()
}

/// The test volume, mounted at `E:\`.
struct Plugged;

impl Volumes for Plugged {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up in these tests",
            path.display()
        )))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (*id == volume_id()).then(|| PathBuf::from(r"E:\"))
    }
}

/// One `TRACK` of a made-up export: rekordbox's `TrackID`, the file's
/// name under `E:\Music`, and its title.
struct Rb(u32, &'static str, &'static str);

/// A rekordbox export holding `tracks`, saying it holds `entries`.
fn export_text(tracks: &[Rb], entries: usize) -> String {
    let tracks: Vec<String> = tracks
        .iter()
        .map(|Rb(id, file, name)| {
            format!(
                r#"    <TRACK TrackID="{id}" Name="{name}" Artist="Kit" Rating="204" PlayCount="7" AverageBpm="128.00" Tonality="Am" Location="file://localhost/E:/Music/{file}"/>"#
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="{entries}">
{}
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="0"/>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#,
        tracks.join("\n")
    )
}

/// A sandbox: the app data folder at `<sandbox>/data` (the guard's root,
/// holding the database) and rekordbox's export in `<sandbox>/documents`.
struct World {
    _dir: tempfile::TempDir,
    base: PathBuf,
    guard: WriteGuard,
    writer: Writer,
    flow: SendFlow,
    export: PathBuf,
    /// The export's modified time, moved on by every save.
    saved: std::cell::Cell<u64>,
}

impl World {
    fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let guard = WriteGuard::app_data(&base.join("data")).unwrap();
        let writer = Writer::open(
            &guard
                .check(&crate::db::db_path(guard.app_data_dir()))
                .unwrap(),
        )
        .unwrap();
        let identity = volume_id().as_str().to_owned();
        writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES (?1, 'external', 'E:\\')",
                    [identity],
                )?;
                c.execute_batch(
                    "INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');",
                )
            })
            .unwrap();
        fs::create_dir(base.join("documents")).unwrap();
        let export = base.join("documents").join("rekordbox.xml");
        World {
            flow: SendFlow::new(guard.clone()),
            _dir: dir,
            base,
            guard,
            writer,
            export,
            saved: std::cell::Cell::new(1_700_000_000),
        }
    }

    fn insert(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// A Library track linked to a file at `E:\Music\<name>` with this
    /// ID3 title. Returns the Library track and the file.
    fn library_track(&self, name: &str, title: &str, present: bool) -> (LibraryTrackId, i64) {
        let track = self.insert(
            "INSERT INTO recording (title) VALUES (?1)",
            (title.to_owned(),),
        );
        let file = self.file(track, name, title, present);
        let library = self.insert(
            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (?1, 'linked', ?2, 'ok')",
            (track, file),
        );
        (LibraryTrackId(library), file)
    }

    /// Another file of `track`.
    fn file(&self, track: i64, name: &str, title: &str, present: bool) -> i64 {
        let raw_tags =
            json!({ "id3v2": [{"key": "TIT2", "value": {"type": "text", "text": title}}] })
                .to_string();
        let file = self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present, size,
                               sniffed_format, raw_tags)
             VALUES (1, ?1, ?1, ?2, 1234, 'mp3', ?3)",
            (name.to_owned(), present, raw_tags),
        );
        self.insert(
            "INSERT INTO recording_file (recording_id, file_id, role)
             VALUES (?1, ?2, CASE WHEN EXISTS (SELECT 1 FROM recording_file
                                               WHERE recording_id = ?1)
                             THEN 'extra' ELSE 'best' END)",
            (track, file),
        );
        file
    }

    /// A hand-made crate holding `tracks`, in that order.
    fn crate_of(&self, name: &str, tracks: &[LibraryTrackId]) -> i64 {
        let id = self.insert(
            "INSERT INTO crate (kind, name) VALUES ('static', ?1)",
            (name.to_owned(),),
        );
        for (i, track) in tracks.iter().enumerate() {
            self.insert(
                "INSERT INTO crate_entry (crate_id, library_track_id, added_at)
                 VALUES (?1, ?2, ?3)",
                (id, track.0, format!("2026-01-01T00:00:{i:02}.000Z")),
            );
        }
        id
    }

    /// rekordbox saves its export: `tracks`, at a later time than before.
    fn save_export(&self, tracks: &[Rb]) {
        self.save_export_text(&export_text(tracks, tracks.len()));
    }

    fn save_export_text(&self, text: &str) {
        fs::write(&self.export, text).unwrap();
        self.saved.set(self.saved.get() + 60);
        fs::File::options()
            .write(true)
            .open(&self.export)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(self.saved.get()))
            .unwrap();
    }

    /// Runs one send job to its end with `sender` as the handler.
    fn run_with(&self, sender: Sender<Plugged>, job: NewJob) -> JobStatus {
        let queue = JobQueue::builder(self.writer.clone())
            .workers(1)
            .handler(JobKind::Export, sender)
            .start()
            .unwrap();
        let id = queue.enqueue(job).unwrap();
        let start = Instant::now();
        loop {
            let job = self
                .writer
                .call(move |c| jobs::store::get(c, id))
                .unwrap()
                .unwrap();
            if job.status.is_finished() {
                queue.shutdown();
                return job.status;
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "the job never finished"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn sender(&self) -> Sender<Plugged> {
        Sender::new(self.flow.clone(), || Plugged)
    }

    /// The prepare step: reads the export and builds the preflight.
    fn prepare(&self) -> Option<Preflight> {
        self.run_with(self.sender(), prepare_job(&self.export.to_string_lossy()));
        self.flow.preflight()
    }

    /// The go, for the preflight waiting. Returns how it failed, if it did.
    fn go(&self, confirmed: bool) -> Option<SendFailure> {
        let token = self.flow.preflight().map(|p| p.token).unwrap_or_default();
        self.go_with(self.sender(), &token, confirmed)
    }

    fn go_with(
        &self,
        sender: Sender<Plugged>,
        token: &str,
        confirmed: bool,
    ) -> Option<SendFailure> {
        let status = self.run_with(sender, write_job(token, confirmed));
        let failure = self.flow.failure();
        assert_eq!(status == JobStatus::Done, failure.is_none(), "{status:?}");
        failure
    }

    fn send_file(&self) -> PathBuf {
        send_path(&self.guard)
    }

    /// Every name in the app data folder that isn't the database's.
    fn data_files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.guard.app_data_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with(crate::db::DB_FILE_NAME))
            .collect();
        names.sort();
        names
    }

    /// Everything a send records: every base, and every Library track's
    /// sent marks.
    fn recorded(&self) -> (Vec<String>, Vec<String>) {
        self.writer
            .call(|c| {
                let rows = |sql: &str| -> rusqlite::Result<Vec<String>> {
                    c.prepare(sql)?.query_map([], |r| r.get(0))?.collect()
                };
                Ok((
                    rows(
                        "SELECT library_track_id || '|' || field || '|' || value || '|' || synced_at
                         FROM sync_base ORDER BY library_track_id, field",
                    )?,
                    rows(
                        "SELECT id || '|' || ifnull(last_sent_location, '-') || '|'
                                || ifnull(last_exported_at, '-')
                         FROM library_track ORDER BY id",
                    )?,
                ))
            })
            .unwrap()
    }

    /// The names and titles in the send file's COLLECTION, in file order.
    fn sent_titles(&self) -> Vec<String> {
        let read = RekordboxXml::parse(fs::read(self.send_file()).unwrap().as_slice()).unwrap();
        read.tracks
            .iter()
            .map(|t| t.attrs.get("Name").unwrap_or_default().to_owned())
            .collect()
    }
}

/// A world with one new track and one track rekordbox has, both in a
/// crate, and rekordbox's export saved.
fn world_with_a_send_waiting() -> (World, LibraryTrackId, LibraryTrackId) {
    let w = World::new();
    let (known, _) = w.library_track("known.mp3", "Known", true);
    let (new, _) = w.library_track("new.mp3", "New", true);
    w.crate_of("Warm up", &[known, new]);
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    (w, known, new)
}

// --- the preflight -----------------------------------------------------------

#[test]
fn the_preflight_counts_new_tracks_and_the_dialogs_to_expect() {
    let w = World::new();
    let (in_crate, _) = w.library_track("known.mp3", "Known", true);
    // rekordbox has this one too, but no crate names it: it isn't sent, so
    // it raises no dialog.
    w.library_track("known-loose.mp3", "Loose", true);
    w.library_track("new-a.mp3", "New A", true);
    w.library_track("new-b.mp3", "New B", true);
    w.crate_of("Warm up", &[in_crate]);
    w.save_export(&[
        Rb(40, "known.mp3", "Known in rekordbox"),
        Rb(41, "known-loose.mp3", "Loose in rekordbox"),
    ]);

    let preflight = w.prepare().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (2, 1));
    assert!(preflight.left_out.is_empty());
    assert!(preflight.other_file.is_empty());
    assert!(preflight.loses_entries.is_empty());
    assert_eq!(preflight.refusal, None);
    assert!(preflight.can_send && !preflight.needs_confirm);
    // The export's own time is shown: when rekordbox saved it.
    assert_eq!(preflight.export.path, w.export.to_string_lossy());
    assert_eq!(preflight.export.modified_ms, 1_700_000_060_000);

    assert_eq!(w.go(false), None);
    // The track rekordbox has goes back with rekordbox's values.
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New A", "New B"]);
    assert_eq!(
        w.flow.sent().map(|s| (s.new_tracks, s.known_tracks)),
        Some((2, 1))
    );
}

#[test]
fn the_preflight_lists_every_track_left_out_with_its_reason() {
    let w = World::new();
    let (gone, _) = w.library_track("gone.mp3", "Gone", false);
    let (bell, _) = w.library_track("bell.mp3", "bell\u{7}title", true);
    let (fine, _) = w.library_track("fine.mp3", "Fine", true);
    let no_file = LibraryTrackId({
        let track = w.insert("INSERT INTO recording (title) VALUES ('No file')", ());
        w.insert(
            "INSERT INTO library_track (recording_id, kind, source_status)
             VALUES (?1, 'linked', 'missing')",
            (track,),
        )
    });
    w.save_export(&[]);

    let preflight = w.prepare().unwrap();
    let left: Vec<_> = preflight
        .left_out
        .iter()
        .map(|l| (l.track.library_track, l.reason, l.attribute.as_deref()))
        .collect();
    assert_eq!(
        left,
        [
            (gone, LeftOutReason::FileMissing, None),
            (bell, LeftOutReason::UnsendableCharacter, Some("Name")),
            (no_file, LeftOutReason::NoFile, None),
        ]
    );
    // Each is named by its title, and its file where it has one.
    assert_eq!(preflight.left_out[0].track.title.as_deref(), Some("Gone"));
    assert_eq!(
        preflight.left_out[0].track.file_name.as_deref(),
        Some("gone.mp3")
    );
    assert_eq!(preflight.left_out[2].track.file_name, None);
    // The others still go.
    assert_eq!(preflight.new_tracks, 1);
    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Fine"]);
    let _ = fine;
}

#[test]
fn a_track_rekordbox_holds_as_another_file_is_listed_and_can_still_be_sent() {
    let w = World::new();
    let (track, _) = w.library_track("linked.mp3", "Linked", true);
    // rekordbox has a different file of the same track.
    w.file(1, "other.mp3", "Other", true);
    w.save_export(&[Rb(77, "other.mp3", "Other in rekordbox")]);

    let preflight = w.prepare().unwrap();
    assert_eq!(preflight.other_file.len(), 1);
    let other = &preflight.other_file[0];
    assert_eq!(other.track.library_track, track);
    assert_eq!(other.library_file.as_deref(), Some(r"E:\Music\linked.mp3"));
    assert_eq!(other.rekordbox_file.as_deref(), Some(r"E:\Music\other.mp3"));
    // It's a new entry for rekordbox, and needs no confirm.
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 0));
    assert!(!preflight.needs_confirm);
    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Linked"]);
}

#[test]
fn every_crate_that_loses_entries_is_named_with_its_count() {
    let w = World::new();
    let (gone_a, _) = w.library_track("gone-a.mp3", "Gone A", false);
    let (gone_b, _) = w.library_track("gone-b.mp3", "Gone B", false);
    let (here, _) = w.library_track("here.mp3", "Here", true);
    let folder = w.insert(
        "INSERT INTO crate (kind, name) VALUES ('folder', 'Friday')",
        (),
    );
    let nested = w.crate_of("Peak", &[gone_a, here, gone_b]);
    w.insert(
        "UPDATE crate SET parent_id = ?1 WHERE id = ?2",
        (folder, nested),
    );
    w.crate_of("All gone", &[gone_a]);
    w.crate_of("Whole", &[here]);
    w.save_export(&[]);

    let preflight = w.prepare().unwrap();
    assert_eq!(
        preflight.loses_entries,
        [
            LosesEntries {
                kind: TreeKind::Crate,
                path: vec!["Friday".into(), "Peak".into()],
                lost: 2,
                entries: 3,
            },
            LosesEntries {
                kind: TreeKind::Crate,
                path: vec!["All gone".into()],
                lost: 1,
                entries: 1,
            },
        ]
    );
}

#[test]
fn two_crates_rekordbox_may_take_for_one_refuse_the_send_and_the_preflight_says_which() {
    let (w, ..) = world_with_a_send_waiting();
    w.crate_of("warm UP", &[]);

    let preflight = w.prepare().unwrap();
    assert_eq!(
        preflight.refusal,
        Some(Refusal {
            reason: RefusalReason::SameName,
            path: vec!["Crates".into(), "warm UP".into()],
        })
    );
    assert!(!preflight.can_send);
    assert_eq!(w.go(true), Some(SendFailure::NotSendable));
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded().0, Vec::<String>::new());
}

#[test]
fn every_reason_the_writer_refuses_a_send_for_has_its_own_explanation() {
    let path = vec!["Crates".to_owned(), "x".to_owned()];
    let explained = |e: BuildError| super::preflight::refusal(&e);
    assert_eq!(
        explained(BuildError::EmptyName { path: path.clone() }),
        Refusal {
            reason: RefusalReason::NoName,
            path: path.clone()
        }
    );
    assert_eq!(
        explained(BuildError::RepeatedName { path: path.clone() }).reason,
        RefusalReason::SameName
    );
    assert_eq!(
        explained(BuildError::UncarriableName { path: path.clone() }).reason,
        RefusalReason::UnsendableName
    );
    assert_eq!(
        explained(BuildError::TooDeep { path }).reason,
        RefusalReason::TooDeep
    );
    assert_eq!(
        explained(BuildError::ReadBack("x".into())),
        Refusal {
            reason: RefusalReason::Internal,
            path: Vec::new()
        }
    );
}

#[test]
fn a_send_cannot_be_started_when_there_is_nothing_to_send() {
    let w = World::new();
    // The only Library track is one rekordbox has, in no crate.
    w.library_track("known.mp3", "Known", true);
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);

    let preflight = w.prepare().unwrap();
    assert!(preflight.nothing_to_send);
    assert!(!preflight.can_send);
    assert_eq!(w.go(true), Some(SendFailure::NotSendable));
    assert_eq!(w.data_files(), Vec::<String>::new());
}

#[test]
fn an_incomplete_export_refuses_the_send_and_nothing_is_written_or_recorded() {
    let (w, ..) = world_with_a_send_waiting();
    // COLLECTION says three tracks and holds one.
    w.save_export_text(&export_text(
        &[Rb(40, "known.mp3", "Known in rekordbox")],
        3,
    ));

    let preflight = w.prepare().unwrap();
    assert_eq!(
        preflight.refusal,
        Some(Refusal {
            reason: RefusalReason::IncompleteExport,
            path: Vec::new(),
        })
    );
    assert!(!preflight.can_send);
    let before = w.recorded();
    assert_eq!(w.go(true), Some(SendFailure::NotSendable));
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded(), before);
    assert_eq!(before.0, Vec::<String>::new());
}

// --- the confirm ---------------------------------------------------------------

#[test]
fn no_confirm_is_needed_when_no_crate_loses_entries_and_every_track_was_read() {
    let (w, ..) = world_with_a_send_waiting();
    let preflight = w.prepare().unwrap();
    assert!(!preflight.needs_confirm);
    assert_eq!(w.go(false), None);
    assert!(w.send_file().is_file());
}

#[test]
fn a_crate_losing_entries_needs_the_confirm_and_goes_once_it_is_given() {
    let (w, ..) = world_with_a_send_waiting();
    let (gone, _) = w.library_track("gone.mp3", "Gone", false);
    w.crate_of("Later", &[gone]);

    let preflight = w.prepare().unwrap();
    assert_eq!(preflight.loses_entries.len(), 1);
    assert!(preflight.needs_confirm);
    // Without the confirm: refused, nothing written, the preflight kept.
    assert_eq!(w.go(false), Some(SendFailure::NotConfirmed));
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded().0, Vec::<String>::new());
    assert_eq!(w.flow.preflight(), Some(preflight));
    // With it: sent.
    assert_eq!(w.go(true), None);
    assert_eq!(w.data_files(), [SEND_FILE_NAME]);
}

#[test]
fn tracks_the_read_could_not_store_need_the_confirm_too() {
    let (w, ..) = world_with_a_send_waiting();
    // Two tracks with one TrackID: the second can't be stored.
    w.save_export(&[
        Rb(40, "known.mp3", "Known in rekordbox"),
        Rb(40, "twin.mp3", "Twin"),
    ]);

    let preflight = w.prepare().unwrap();
    assert_eq!(preflight.export.not_stored, 1);
    assert!(preflight.loses_entries.is_empty());
    assert!(preflight.needs_confirm);
    assert_eq!(w.go(false), Some(SendFailure::NotConfirmed));
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.go(true), None);
    assert_eq!(w.data_files(), [SEND_FILE_NAME]);
}

// --- a send never runs on an old read --------------------------------------------

#[test]
fn preparing_a_send_reads_the_export_again_every_time() {
    let (w, ..) = world_with_a_send_waiting();
    let first = w.prepare().unwrap();
    assert_eq!((first.new_tracks, first.known_tracks), (1, 1));

    // rekordbox imports the new track and saves its export again.
    w.save_export(&[
        Rb(40, "known.mp3", "Known in rekordbox"),
        Rb(41, "new.mp3", "New in rekordbox"),
    ]);
    let second = w.prepare().unwrap();
    assert_eq!((second.new_tracks, second.known_tracks), (0, 2));
    assert_eq!(second.export.modified_ms, first.export.modified_ms + 60_000);
    assert_ne!(second.token, first.token);
    // The first preflight is gone: its token sends nothing.
    assert_eq!(
        w.go_with(w.sender(), &first.token, true),
        Some(SendFailure::NoPreflight)
    );
    assert_eq!(w.data_files(), Vec::<String>::new());
}

#[test]
fn a_go_without_a_preflight_writes_nothing() {
    let (w, ..) = world_with_a_send_waiting();
    assert_eq!(
        w.go_with(w.sender(), "no such preflight", true),
        Some(SendFailure::NoPreflight)
    );
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded().0, Vec::<String>::new());
}

#[test]
fn a_go_after_rekordbox_saved_its_export_again_is_refused_and_writes_nothing() {
    let (w, ..) = world_with_a_send_waiting();
    w.prepare().unwrap();
    // Same contents, saved again: the read is old now.
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);

    assert_eq!(w.go(true), Some(SendFailure::ExportChanged));
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded().0, Vec::<String>::new());
    // The old preflight can't be tried again.
    assert_eq!(w.flow.preflight(), None);
}

#[test]
fn a_go_after_the_export_was_removed_is_refused_and_writes_nothing() {
    let (w, ..) = world_with_a_send_waiting();
    w.prepare().unwrap();
    fs::remove_file(&w.export).unwrap();

    assert_eq!(w.go(true), Some(SendFailure::ExportChanged));
    assert_eq!(w.data_files(), Vec::<String>::new());
}

#[test]
fn a_go_after_rekordbox_was_read_again_elsewhere_is_refused_and_writes_nothing() {
    let (w, ..) = world_with_a_send_waiting();
    let preflight = w.prepare().unwrap();
    // Another read of the same file (the rekordbox panel's "Read again")
    // records its own time.
    w.insert(
        "UPDATE setting SET value = json_set(value, '$.readAt', '2031-01-01T00:00:00.000Z')
         WHERE key = 'rekordbox_xml_last_read'",
        (),
    );

    assert_eq!(
        w.go_with(w.sender(), &preflight.token, true),
        Some(SendFailure::ExportChanged)
    );
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded().0, Vec::<String>::new());
}

#[test]
fn a_go_after_the_library_changed_is_refused_and_writes_nothing() {
    let (w, ..) = world_with_a_send_waiting();
    w.prepare().unwrap();
    // A track added after the review would go out unreviewed.
    w.library_track("late.mp3", "Late", true);

    assert_eq!(w.go(true), Some(SendFailure::LibraryChanged));
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded().0, Vec::<String>::new());
    assert_eq!(w.flow.preflight(), None);
}

#[test]
fn a_preflight_is_good_for_one_send() {
    let (w, ..) = world_with_a_send_waiting();
    let preflight = w.prepare().unwrap();
    assert_eq!(w.go(false), None);
    let sent = fs::read(w.send_file()).unwrap();
    let recorded = w.recorded();

    assert_eq!(
        w.go_with(w.sender(), &preflight.token, true),
        Some(SendFailure::NoPreflight)
    );
    assert_eq!(fs::read(w.send_file()).unwrap(), sent);
    assert_eq!(w.recorded(), recorded);
}

#[test]
fn a_failed_read_leaves_no_preflight_to_send() {
    let (w, ..) = world_with_a_send_waiting();
    assert!(w.prepare().is_some());
    // The export is cut off.
    w.save_export_text("<?xml version=\"1.0\"?><DJ_PLAYLISTS><COLLECTION Entries=\"1\">");

    assert_eq!(w.prepare(), None);
    assert_eq!(w.flow.failure(), Some(SendFailure::ReadFailed));
}

// --- nothing is written before the go ---------------------------------------------

#[test]
fn preparing_a_send_writes_no_file_and_records_nothing() {
    let (w, ..) = world_with_a_send_waiting();
    let documents = fs::read(&w.export).unwrap();
    let before = w.recorded();

    let preflight = w.prepare().unwrap();
    assert!(preflight.can_send);
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert!(!w.send_file().exists());
    assert_eq!(w.recorded(), before);
    assert_eq!(before.0, Vec::<String>::new());
    assert!(
        before.1.iter().all(|mark| mark.ends_with("|-|-")),
        "{before:?}"
    );
    // And rekordbox's export is only read.
    assert_eq!(fs::read(&w.export).unwrap(), documents);
}

// --- the file ------------------------------------------------------------------------

/// Every name directly in the sandbox and in rekordbox's folder.
fn outside_listing(w: &World) -> Vec<String> {
    let mut names = Vec::new();
    for dir in [w.base.clone(), w.base.join("documents")] {
        for entry in fs::read_dir(&dir).unwrap() {
            names.push(entry.unwrap().path().to_string_lossy().into_owned());
        }
    }
    names.sort();
    names
}

#[test]
fn the_go_writes_one_fixed_file_inside_the_app_data_folder_and_records_the_send() {
    let (w, known, new) = world_with_a_send_waiting();
    let outside = outside_listing(&w);
    let export = fs::read(&w.export).unwrap();
    w.prepare().unwrap();
    assert_eq!(w.go(false), None);

    // The one file, directly in the app data folder, and nothing else new.
    let dest = w.send_file();
    assert_eq!(dest, w.guard.app_data_dir().join("tracklist-pro.xml"));
    assert!(dest.starts_with(w.guard.app_data_dir()));
    assert!(w.guard.check(&dest).is_ok());
    assert_eq!(w.data_files(), ["tracklist-pro.xml"]);
    // Nothing outside the app data folder changed.
    assert_eq!(outside_listing(&w), outside);
    assert_eq!(fs::read(&w.export).unwrap(), export);

    // rekordbox's analysis never goes back to it.
    let text = fs::read_to_string(&dest).unwrap();
    assert!(!text.contains("AverageBpm") && !text.contains("Tonality"));
    // Rule 8: the send is recorded for both tracks.
    let (bases, marks) = w.recorded();
    for track in [known, new] {
        assert!(
            bases
                .iter()
                .any(|b| b.starts_with(&format!("{}|Location|", track.0))),
            "{bases:?}"
        );
    }
    assert!(marks.iter().all(|m| !m.ends_with("|-")), "{marks:?}");
    // A send isn't in the operation log.
    let operations: i64 = w
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(operations, 0);
    // The state the checklist shows names the same file, without the
    // `\\?\` prefix.
    let state = w.flow.state(None);
    assert_eq!(state.file_path, crate::scan::display_path(&dest));
    assert!(!state.file_path.starts_with(r"\\?\"));
    assert!(state.sent.is_some() && state.preflight.is_none());
}

#[test]
fn every_send_replaces_the_same_file() {
    let (w, ..) = world_with_a_send_waiting();
    w.prepare().unwrap();
    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New"]);

    w.library_track("second.mp3", "Second", true);
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    w.prepare().unwrap();
    assert_eq!(w.go(false), None);
    assert_eq!(w.data_files(), [SEND_FILE_NAME]);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New", "Second"]);
}

/// A first send that went through, then a second one prepared after the
/// Library grew: the file and the record of the first, to compare with.
fn world_with_a_second_send_waiting() -> (World, Vec<u8>, (Vec<String>, Vec<String>)) {
    let (w, ..) = world_with_a_send_waiting();
    w.prepare().unwrap();
    assert_eq!(w.go(false), None);
    let file = fs::read(w.send_file()).unwrap();
    w.library_track("second.mp3", "Second", true);
    let recorded = w.recorded();
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    w.prepare().unwrap();
    (w, file, recorded)
}

#[test]
fn a_failed_write_leaves_the_last_sends_file_and_every_base_untouched() {
    let (w, file, recorded) = world_with_a_second_send_waiting();
    // Every name the write could give its new file is taken by a folder,
    // which clearing leftovers never deletes.
    let mut blockers = vec![format!("{SEND_FILE_NAME}.part")];
    blockers.extend((1..16).map(|n| format!("{SEND_FILE_NAME}.{n}.part")));
    for name in &blockers {
        fs::create_dir(w.guard.app_data_dir().join(name)).unwrap();
    }

    assert_eq!(w.go(false), Some(SendFailure::CantWrite));
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.recorded(), recorded);
    assert_eq!(w.flow.sent(), None);
    // With the way clear, the same preflight goes through.
    for name in &blockers {
        fs::remove_dir(w.guard.app_data_dir().join(name)).unwrap();
    }
    assert_eq!(w.go(false), None);
    assert_ne!(fs::read(w.send_file()).unwrap(), file);
}

fn failing_record(_: &mut rusqlite::Connection, _: &[SentTrack]) -> rusqlite::Result<()> {
    Err(rusqlite::Error::InvalidQuery)
}

#[test]
fn a_failed_record_puts_the_last_sends_file_back_and_leaves_every_base_untouched() {
    let (w, file, recorded) = world_with_a_second_send_waiting();
    let token = w.flow.preflight().unwrap().token;

    let failure = w.go_with(w.sender().recording_with(failing_record), &token, false);
    assert_eq!(failure, Some(SendFailure::CantRecord));
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.data_files(), [SEND_FILE_NAME]);
    assert_eq!(w.recorded(), recorded);
    assert_eq!(w.flow.sent(), None);
    // The same preflight can be tried again, and goes through.
    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New", "Second"]);
    assert_ne!(w.recorded(), recorded);
}

#[test]
fn record_sends_own_rollback_also_puts_the_last_sends_file_back() {
    // The real record, failing partway: a sent track that's no longer in
    // the Library makes it roll back.
    let (w, file, recorded) = world_with_a_second_send_waiting();
    let token = w.flow.preflight().unwrap().token;
    let partway = |c: &mut rusqlite::Connection, sent: &[SentTrack]| {
        let mut sent = sent.to_vec();
        sent.last_mut().unwrap().library_track = LibraryTrackId(9_999);
        crate::rekordbox_write::record_send(c, &sent)
    };

    let failure = w.go_with(w.sender().recording_with(partway), &token, false);
    assert_eq!(failure, Some(SendFailure::CantRecord));
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.recorded(), recorded);
}

#[test]
fn a_failed_record_on_the_first_send_leaves_the_new_file_and_records_nothing() {
    // There's no earlier file to put back: the new one stays (the next
    // send replaces it) and the send is reported as failed.
    let (w, ..) = world_with_a_send_waiting();
    let token = w.prepare().unwrap().token;

    let failure = w.go_with(w.sender().recording_with(failing_record), &token, false);
    assert_eq!(failure, Some(SendFailure::CantRecord));
    assert_eq!(w.data_files(), [SEND_FILE_NAME]);
    assert_eq!(w.recorded().0, Vec::<String>::new());
    assert_eq!(w.flow.sent(), None);
}

#[test]
fn a_send_clears_a_crashed_sends_leftovers_and_only_those() {
    let (w, ..) = world_with_a_send_waiting();
    let data = w.guard.app_data_dir().to_owned();
    let leftovers = [
        format!("{SEND_FILE_NAME}.part"),
        format!("{SEND_FILE_NAME}.3.part"),
        format!("{SEND_FILE_NAME}.15.part"),
    ];
    let not_ours = [
        format!("{SEND_FILE_NAME}.16.part"),
        format!("{SEND_FILE_NAME}.part.bak"),
        "notes.xml.part".to_owned(),
        "other.part".to_owned(),
    ];
    for name in leftovers.iter().chain(&not_ours) {
        fs::write(data.join(name), name).unwrap();
    }
    // Preparing clears nothing: it writes nothing at all.
    w.prepare().unwrap();
    assert_eq!(w.data_files().len(), leftovers.len() + not_ours.len());

    assert_eq!(w.go(false), None);
    let mut expected: Vec<String> = not_ours.to_vec();
    expected.push(SEND_FILE_NAME.to_owned());
    expected.sort();
    assert_eq!(w.data_files(), expected);
    for name in &not_ours {
        assert_eq!(fs::read(data.join(name)).unwrap(), name.as_bytes());
    }
}

// --- the crate tree ----------------------------------------------------------------

#[test]
fn the_crate_tree_is_sent_as_folders_and_hand_made_crates_in_their_stored_order() {
    let w = World::new();
    let (a, _) = w.library_track("a.mp3", "A", true);
    let (b, _) = w.library_track("b.mp3", "B", true);
    let folder = w.insert(
        "INSERT INTO crate (kind, name, position) VALUES ('folder', 'Friday', 1)",
        (),
    );
    let inner = w.crate_of("Peak", &[b, a]);
    w.insert(
        "UPDATE crate SET parent_id = ?1 WHERE id = ?2",
        (folder, inner),
    );
    let first = w.crate_of("Opening", &[a]);
    w.insert("UPDATE crate SET position = 0 WHERE id = ?1", (first,));
    // A smart crate's tracks come from rules nothing evaluates yet.
    w.insert(
        "INSERT INTO crate (kind, name, rules, position) VALUES ('smart', 'Clever', '{}', 2)",
        (),
    );
    w.save_export(&[]);
    w.prepare().unwrap();
    assert_eq!(w.go(false), None);

    let read = RekordboxXml::parse(fs::read(w.send_file()).unwrap().as_slice()).unwrap();
    let playlists: Vec<(Vec<String>, String, Vec<String>)> = read
        .playlists
        .playlists()
        .into_iter()
        .map(|(folders, playlist)| {
            (
                folders.iter().map(|f| (*f).to_owned()).collect(),
                playlist.name.clone(),
                playlist.entries.iter().map(|e| e.key.clone()).collect(),
            )
        })
        .collect();
    assert_eq!(
        playlists,
        [
            (
                vec!["Crates".to_owned()],
                "Opening".to_owned(),
                vec!["1".to_owned()]
            ),
            (
                vec!["Crates".to_owned(), "Friday".to_owned()],
                "Peak".to_owned(),
                vec!["2".to_owned(), "1".to_owned()]
            ),
        ]
    );
}

// --- over IPC -------------------------------------------------------------------------

mod ipc {
    use serde_json::{json, Value};
    use tauri::Manager;

    use crate::ipc::testing::{app, invoke};
    use crate::write_guard::WriteGuard;

    #[test]
    fn the_checklist_is_told_the_send_files_path_inside_the_app_data_folder() {
        let (_data, app) = app();
        let state = invoke(&app, "send_state", json!({})).unwrap();
        let guard = app.state::<WriteGuard>();
        let expected = crate::scan::display_path(&guard.app_data_dir().join("tracklist-pro.xml"));
        assert_eq!(state["filePath"], json!(expected));
        assert_eq!(state["preflight"], Value::Null);
        assert_eq!(state["exportPath"], Value::Null);
        assert_eq!(state["revision"], json!(0));
    }

    #[test]
    fn a_send_cannot_be_prepared_before_an_export_is_chosen() {
        let (_data, app) = app();
        let refused = invoke(&app, "prepare_send", json!({ "path": null })).unwrap_err();
        assert_eq!(refused["kind"], json!("noRekordboxXml"));
    }

    #[test]
    fn the_apps_own_job_queue_runs_the_go_and_refuses_one_with_no_preflight() {
        let (_data, app) = app();
        let job = invoke(
            &app,
            "write_send",
            json!({ "token": "nothing reviewed", "confirmed": true }),
        )
        .unwrap();
        assert!(job.is_number());
        // The step ends (the revision moves) with the refusal recorded.
        let start = std::time::Instant::now();
        let state = loop {
            let state = invoke(&app, "send_state", json!({})).unwrap();
            if state["revision"] != json!(0) {
                break state;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(60));
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(state["failure"], json!("noPreflight"));
        let guard = app.state::<WriteGuard>();
        assert!(!guard.app_data_dir().join("tracklist-pro.xml").exists());
    }
}
