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

/// The first export's modified time, in seconds since the Unix epoch: the
/// year 2100, so every export these tests save is newer than any send
/// they record (a send is refused when its export is older than the last
/// send). Each save is a minute after the one before.
const EXPORTS_SAVED_FROM: u64 = 4_102_444_800;

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
            saved: std::cell::Cell::new(EXPORTS_SAVED_FROM),
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
        self.export_saved_at(Duration::from_secs(self.saved.get()));
    }

    /// Sets the export's modified time, as a duration since the Unix
    /// epoch: when rekordbox saved it.
    fn export_saved_at(&self, since_epoch: Duration) {
        fs::File::options()
            .write(true)
            .open(&self.export)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + since_epoch)
            .unwrap();
    }

    /// When the last send was recorded, in milliseconds since the epoch.
    fn last_send_ms(&self) -> Option<i64> {
        self.writer
            .call(|c| super::preflight::last_send_ms(c))
            .unwrap()
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
        self.run_with(
            self.sender(),
            self.flow.prepare_job(&self.export.to_string_lossy()),
        );
        self.flow.preflight()
    }

    /// The go, for the preflight waiting. Returns how it failed, if it did.
    fn go(&self, confirmed: bool) -> Option<SendFailure> {
        self.go_answering(confirmed, None)
    }

    /// The go, with the user's answer to "has a send been imported into
    /// rekordbox since this export was saved?" (`None`: no answer).
    fn go_answering(
        &self,
        confirmed: bool,
        imported_since_export: Option<bool>,
    ) -> Option<SendFailure> {
        let token = self.flow.preflight().map(|p| p.token).unwrap_or_default();
        let job = self
            .flow
            .write_job(&token, confirmed, imported_since_export);
        let status = self.run_with(self.sender(), job);
        let failure = self.flow.failure();
        assert_eq!(status == JobStatus::Done, failure.is_none(), "{status:?}");
        failure
    }

    fn go_with(
        &self,
        sender: Sender<Plugged>,
        token: &str,
        confirmed: bool,
    ) -> Option<SendFailure> {
        let status = self.run_with(sender, self.flow.write_job(token, confirmed, None));
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

    /// Every folder and playlist path recorded so far, in the order recorded.
    fn recorded_paths(&self) -> Vec<String> {
        self.writer
            .call(|c| {
                c.prepare("SELECT path || '|' || kind FROM sent_playlist ORDER BY id")?
                    .query_map([], |r| r.get(0))?
                    .collect()
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
    assert!(preflight.file_missing.is_empty());
    assert!(preflight.other_file.is_empty());
    assert!(preflight.loses_entries.is_empty());
    assert_eq!(preflight.refusal, None);
    assert!(preflight.can_send && !preflight.needs_confirm);
    // The export's own time is shown: when rekordbox saved it.
    assert_eq!(preflight.export.path, w.export.to_string_lossy());
    assert_eq!(
        preflight.export.modified_ms,
        (EXPORTS_SAVED_FROM as i64 + 60) * 1000
    );

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
fn a_track_rekordbox_has_whose_file_is_missing_is_sent_and_listed_not_left_out() {
    let w = World::new();
    let (gone, _) = w.library_track("gone.mp3", "Gone", false);
    let (here, _) = w.library_track("here.mp3", "Here", true);
    w.crate_of("Warm up", &[gone, here]);
    w.save_export(&[Rb(40, "gone.mp3", "Gone in rekordbox")]);

    let preflight = w.prepare().unwrap();
    let listed: Vec<_> = preflight
        .file_missing
        .iter()
        .map(|t| (t.library_track, t.title.as_deref()))
        .collect();
    assert_eq!(listed, [(gone, Some("Gone"))]);
    // It's sent, so nothing is left out and the crate keeps its entry.
    assert_eq!(preflight.left_out, Vec::new());
    assert!(preflight.loses_entries.is_empty());
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert!(!preflight.needs_confirm);
    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Gone in rekordbox", "Here"]);
    let _ = here;
}

// --- sent with rekordbox's path, where no file is ------------------------------

/// A path as a rekordbox `Location`.
fn location_of(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let path = path.trim_start_matches("//?/");
    format!("file://localhost/{}", path.replace(' ', "%20"))
}

/// An export holding one track rekordbox has at `location`, lasting 200 s.
fn export_with_one_track_at(location: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="1">
    <TRACK TrackID="40" Name="Known in rekordbox" Artist="Kit" TotalTime="200" PlayCount="7" Location="{location}"/>
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="0"/>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#
    )
}

/// A Library track on `E:\Music\known.mp3`, in a crate, and rekordbox's
/// entry for it at `theirs`: another path, so only the name and the length
/// pair the two (relink's second step, a trusted match).
fn world_with_a_known_track_rekordbox_has_at(theirs: &Path) -> (World, LibraryTrackId) {
    world_with_a_known_track_at_location(&location_of(theirs))
}

/// The same, with rekordbox's `Location` as written.
fn world_with_a_known_track_at_location(location: &str) -> (World, LibraryTrackId) {
    let w = World::new();
    let (known, file) = w.library_track("known.mp3", "Known", true);
    w.insert(
        "UPDATE file SET duration_ms = 200000 WHERE id = ?1",
        (file,),
    );
    w.crate_of("Warm up", &[known]);
    w.save_export_text(&export_with_one_track_at(location));
    (w, known)
}

fn listed_without_a_file(preflight: &Preflight) -> Vec<LibraryTrackId> {
    preflight
        .no_file_at_location
        .iter()
        .map(|t| t.library_track)
        .collect()
}

// Off Windows the item never lists anything: no drive is ever connected
// there, and a rekordbox `Location` is a Windows path.
#[cfg(windows)]
#[test]
fn a_known_track_sent_with_rekordboxs_path_where_no_file_is_is_listed_and_still_sent_unchanged() {
    let sandbox = tempfile::tempdir().unwrap();
    let theirs = fs::canonicalize(sandbox.path())
        .unwrap()
        .join("Old place")
        .join("known.mp3");
    let (w, known) = world_with_a_known_track_rekordbox_has_at(&theirs);

    let preflight = w.prepare().unwrap();
    // rekordbox has it: it's the app's pairing that put the two together.
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (0, 1));
    assert_eq!(listed_without_a_file(&preflight), [known]);
    // A line in the review, not a stop: nothing else about the send moves.
    assert_eq!(preflight.file_missing, []);
    assert_eq!(preflight.left_out, []);
    assert!(preflight.can_send && !preflight.needs_confirm);

    assert_eq!(w.go(false), None);
    let sent = RekordboxXml::read_file(&w.send_file()).unwrap();
    assert_eq!(sent.tracks.len(), 1);
    // rekordbox's own entry at rekordbox's own path, as before.
    assert_eq!(sent.tracks[0].track_id, Some(40));
    assert_eq!(
        sent.tracks[0].location.as_ref().unwrap().match_key(),
        crate::rekordbox::location::decode(&location_of(&theirs))
            .unwrap()
            .match_key()
    );
}

#[test]
fn a_known_track_whose_file_is_at_rekordboxs_path_is_not_listed_even_outside_every_music_folder() {
    let sandbox = tempfile::tempdir().unwrap();
    let folder = fs::canonicalize(sandbox.path()).unwrap().join("Old place");
    fs::create_dir(&folder).unwrap();
    let theirs = folder.join("known.mp3");
    // rekordbox's file is really there, in a folder the app doesn't scan.
    fs::write(&theirs, b"audio").unwrap();
    let (w, _known) = world_with_a_known_track_rekordbox_has_at(&theirs);

    let preflight = w.prepare().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (0, 1));
    assert_eq!(preflight.no_file_at_location, []);
}

#[test]
fn a_known_track_at_a_path_on_a_drive_that_is_not_connected_is_not_listed() {
    // A drive letter nothing is mounted at on this PC.
    let Some(letter) = ('D'..='Z').find(|l| !Path::new(&format!("{l}:\\")).exists()) else {
        return;
    };
    let theirs = PathBuf::from(format!("{letter}:\\Old place\\known.mp3"));
    let (w, _known) = world_with_a_known_track_rekordbox_has_at(&theirs);

    let preflight = w.prepare().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (0, 1));
    // Whether the file is there can't be told while the drive is away.
    assert_eq!(preflight.no_file_at_location, []);
}

#[test]
fn a_known_track_at_a_network_path_or_a_macos_path_is_never_looked_up_or_listed() {
    for location in [
        "file://localhost//server/share/Old%20place/known.mp3",
        "file://localhost/Users/dj/Music/Old%20place/known.mp3",
    ] {
        let (w, _known) = world_with_a_known_track_at_location(location);
        let preflight = w.prepare().unwrap();
        // Paired with the Library's file by name and length, as before.
        assert_eq!(
            (preflight.new_tracks, preflight.known_tracks),
            (0, 1),
            "{location}"
        );
        assert_eq!(preflight.no_file_at_location, [], "{location}");
    }
}

/// A track as a send writes it, for [`super::preflight::sent_elsewhere`].
fn sent_at(location: &str, in_rekordbox: bool, file_missing: bool) -> SentTrack {
    SentTrack {
        library_track: LibraryTrackId(1),
        in_rekordbox,
        track_id: 40,
        attributes: vec![("Location".to_owned(), location.to_owned())],
        rekordbox_holds_other_file: None,
        file_missing,
    }
}

#[test]
fn only_a_known_track_sent_at_another_drive_path_than_its_file_is_looked_up_on_disk() {
    use super::preflight::sent_elsewhere;
    const LINKED: Option<&str> = Some(r"E:\Music\known.mp3");
    const ELSEWHERE: &str = "file://localhost/D:/Old%20place/known.mp3";

    // The case the item is for.
    assert_eq!(
        sent_elsewhere(&sent_at(ELSEWHERE, true, false), LINKED).as_deref(),
        Some(r"D:\Old place\known.mp3")
    );
    // Sent although its own file is missing, at another path than that
    // file's (a pairing kept while the file is gone): it's in the
    // missing-file list, and never in this one too.
    assert_eq!(
        sent_elsewhere(&sent_at(ELSEWHERE, true, true), LINKED),
        None
    );
    // A track rekordbox doesn't have is sent where its file is.
    assert_eq!(
        sent_elsewhere(&sent_at(ELSEWHERE, false, false), LINKED),
        None
    );
    // The same path, however rekordbox spells or cases it.
    for same in [
        "file://localhost/E:/Music/known.mp3",
        "file://localhost/e:/music/KNOWN.mp3",
        "file://localhost/E:/Music/%6bnown.mp3",
    ] {
        assert_eq!(
            sent_elsewhere(&sent_at(same, true, false), LINKED),
            None,
            "{same}"
        );
    }
    // A network path, a macOS path, a streaming entry, nonsense.
    for other in [
        "file://localhost//server/share/Old%20place/known.mp3",
        "file://localhost/Users/dj/Music/known.mp3",
        "soundcloud:tracks:1",
        "",
    ] {
        assert_eq!(
            sent_elsewhere(&sent_at(other, true, false), LINKED),
            None,
            "{other}"
        );
    }
    // Without a linked file's path there's nothing to compare with.
    assert_eq!(sent_elsewhere(&sent_at(ELSEWHERE, true, false), None), None);
}

#[cfg(windows)]
#[test]
fn a_file_coming_back_between_the_review_and_the_go_does_not_refuse_the_go_or_change_what_is_written(
) {
    let sandbox = tempfile::tempdir().unwrap();
    let folder = fs::canonicalize(sandbox.path()).unwrap().join("Old place");
    let theirs = folder.join("known.mp3");
    let (w, known) = world_with_a_known_track_rekordbox_has_at(&theirs);

    // Reviewed with the track listed, and written as reviewed.
    let reviewed = w.prepare().unwrap();
    assert_eq!(listed_without_a_file(&reviewed), [known]);
    assert_eq!(w.go(false), None);
    let as_reviewed = fs::read(w.send_file()).unwrap();

    // Again, from a new export of the same content: listed in the review,
    // then the file is put back before the go.
    w.save_export_text(&export_with_one_track_at(&location_of(&theirs)));
    let reviewed = w.prepare().unwrap();
    assert_eq!(listed_without_a_file(&reviewed), [known]);
    fs::create_dir(&folder).unwrap();
    fs::write(&theirs, b"audio").unwrap();

    // The list was about the disk, not about the send: the go isn't
    // refused, and the file is the one reviewed.
    assert_eq!(w.go(false), None);
    assert_eq!(fs::read(w.send_file()).unwrap(), as_reviewed);
    // The next review looks again.
    w.save_export_text(&export_with_one_track_at(&location_of(&theirs)));
    assert_eq!(w.prepare().unwrap().no_file_at_location, []);
}

#[test]
fn a_track_sent_although_its_own_file_is_missing_is_listed_once_under_that_and_not_here() {
    let w = World::new();
    let (gone, _) = w.library_track("gone.mp3", "Gone", false);
    let (new, _) = w.library_track("new.mp3", "New", true);
    w.crate_of("Warm up", &[gone, new]);
    w.save_export(&[Rb(40, "gone.mp3", "Gone in rekordbox")]);

    let preflight = w.prepare().unwrap();
    assert_eq!(preflight.file_missing.len(), 1);
    // Neither the track whose own file is missing nor the new one.
    assert_eq!(preflight.no_file_at_location, []);
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
    assert_eq!(w.recorded_paths(), Vec::<String>::new());
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

// --- an export older than the last send ------------------------------------------

const NO_ANSWER: Option<bool> = None;
/// "No send has been imported into rekordbox since this export was saved."
const NOTHING_IMPORTED_SINCE: Option<bool> = Some(false);
/// "A send has been imported since."
const IMPORTED_SINCE: Option<bool> = Some(true);

/// A world whose one send has gone through, from an export saved an hour
/// before it: the export is now older than the last send. Returns the
/// file that send wrote and what it recorded.
fn world_whose_export_is_older_than_its_last_send() -> (World, Vec<u8>, (Vec<String>, Vec<String>))
{
    let (w, ..) = world_with_a_send_waiting();
    let hour_ago = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        - Duration::from_secs(3600);
    w.export_saved_at(hour_ago);
    // No send was ever recorded: an export of any age raises no question.
    assert_eq!(w.last_send_ms(), None);
    let preflight = w.prepare().unwrap();
    assert!(!preflight.export_older_than_last_send);
    assert_eq!(w.go_answering(false, NO_ANSWER), None);
    let file = fs::read(w.send_file()).unwrap();
    let recorded = w.recorded();
    (w, file, recorded)
}

#[test]
fn an_export_older_than_the_last_send_is_not_refused_but_raises_the_question() {
    let (w, ..) = world_whose_export_is_older_than_its_last_send();
    let preflight = w.prepare().unwrap();
    assert!(preflight.export_older_than_last_send);
    // It's a question for the user, not a refusal: the review is whole.
    assert_eq!(preflight.refusal, None);
    assert!(preflight.can_send);
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
}

#[test]
fn with_no_answer_a_send_from_an_older_export_is_not_written() {
    let (w, file, recorded) = world_whose_export_is_older_than_its_last_send();
    w.prepare().unwrap();
    assert_eq!(
        w.go_answering(false, NO_ANSWER),
        Some(SendFailure::ExportOlderThanLastSend)
    );
    // The explicit confirm is another question: it doesn't answer this one.
    assert_eq!(
        w.go_answering(true, NO_ANSWER),
        Some(SendFailure::ExportOlderThanLastSend)
    );
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.recorded(), recorded);
}

#[test]
fn answering_that_a_send_was_imported_since_refuses_the_send() {
    let (w, file, recorded) = world_whose_export_is_older_than_its_last_send();
    w.prepare().unwrap();
    assert_eq!(
        w.go_answering(true, IMPORTED_SINCE),
        Some(SendFailure::ExportOlderThanLastSend)
    );
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.recorded(), recorded);
}

#[test]
fn answering_that_nothing_was_imported_since_sends_exactly_what_a_fresh_export_of_the_same_content_would(
) {
    let (w, first, _) = world_whose_export_is_older_than_its_last_send();
    // The Library has moved on since the first send, so this send differs.
    let (later, _) = w.library_track("later.mp3", "Later", true);
    w.crate_of("Later", &[later]);

    let asked = w.prepare().unwrap();
    assert!(asked.export_older_than_last_send);
    assert_eq!(w.go_answering(false, NOTHING_IMPORTED_SINCE), None);
    let on_the_answer = fs::read(w.send_file()).unwrap();
    assert_ne!(on_the_answer, first);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New", "Later"]);

    // The same content, exported again: no question, and the same file.
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    let fresh = w.prepare().unwrap();
    assert!(!fresh.export_older_than_last_send);
    assert_eq!(
        (fresh.new_tracks, fresh.known_tracks),
        (asked.new_tracks, asked.known_tracks)
    );
    assert_eq!(w.go_answering(false, NO_ANSWER), None);
    assert_eq!(fs::read(w.send_file()).unwrap(), on_the_answer);
}

#[test]
fn the_answer_is_kept_only_in_that_send_jobs_row_and_the_next_send_from_that_export_asks_again() {
    let (w, ..) = world_whose_export_is_older_than_its_last_send();
    w.prepare().unwrap();
    assert_eq!(w.go_answering(false, NOTHING_IMPORTED_SINCE), None);
    let file = fs::read(w.send_file()).unwrap();
    let recorded = w.recorded();

    // The "no" is in the row of the job that carried it, and nowhere else
    // in the database.
    let answers_in_jobs = || -> Vec<(String, Option<bool>)> {
        w.writer
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT status, json_extract(target, '$.importedSinceExport') FROM job
                     WHERE json_extract(target, '$.send') = 'write' ORDER BY id",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .unwrap()
    };
    assert_eq!(
        answers_in_jobs(),
        [("done".to_owned(), None), ("done".to_owned(), Some(false))]
    );
    let elsewhere: i64 = w
        .writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM setting WHERE value LIKE '%importedSince%'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(elsewhere, 0);

    // The same export once more: it's older than the send just made, the
    // question is there again, and that finished job's "no" answers
    // nothing, though its row is still there.
    let preflight = w.prepare().unwrap();
    assert!(preflight.export_older_than_last_send);
    assert_eq!(
        w.go_answering(false, NO_ANSWER),
        Some(SendFailure::ExportOlderThanLastSend)
    );
    assert_eq!(
        answers_in_jobs()[..2],
        [("done".to_owned(), None), ("done".to_owned(), Some(false))]
    );
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.recorded(), recorded);
}

#[test]
fn a_no_does_not_stand_in_for_the_explicit_confirm() {
    let (w, file, _) = world_whose_export_is_older_than_its_last_send();
    // A crate that will arrive with a track left out (its file is missing
    // and rekordbox doesn't have it): this send needs the confirm.
    let (gone, _) = w.library_track("gone.mp3", "Gone", false);
    w.crate_of("Later", &[gone]);
    let recorded = w.recorded();

    let preflight = w.prepare().unwrap();
    assert!(preflight.export_older_than_last_send);
    assert!(preflight.needs_confirm && preflight.can_send);
    // Two questions, two answers: the "no" answers only its own.
    assert_eq!(
        w.go_answering(false, NOTHING_IMPORTED_SINCE),
        Some(SendFailure::NotConfirmed)
    );
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.recorded(), recorded);
    // And the confirm answers only its own.
    assert_eq!(
        w.go_answering(true, NO_ANSWER),
        Some(SendFailure::ExportOlderThanLastSend)
    );
    assert_eq!(fs::read(w.send_file()).unwrap(), file);

    assert_eq!(w.go_answering(true, NOTHING_IMPORTED_SINCE), None);
    assert_ne!(fs::read(w.send_file()).unwrap(), file);
}

#[test]
fn an_export_saved_after_the_last_send_raises_no_question_and_needs_no_answer() {
    let (w, ..) = world_with_a_send_waiting();
    w.prepare().unwrap();
    assert_eq!(w.go(false), None);

    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    let preflight = w.prepare().unwrap();
    assert!(!preflight.export_older_than_last_send);
    assert_eq!(preflight.refusal, None);
    assert_eq!(w.go_answering(false, NO_ANSWER), None);
}

#[test]
fn an_answer_to_a_question_that_was_not_asked_changes_nothing() {
    let (w, ..) = world_with_a_send_waiting();
    let preflight = w.prepare().unwrap();
    assert!(!preflight.export_older_than_last_send);
    assert_eq!(w.go_answering(false, IMPORTED_SINCE), None);
    assert!(w.send_file().is_file());
}

#[test]
fn a_no_does_not_let_through_a_send_that_is_no_longer_the_one_reviewed() {
    let (w, file, _) = world_whose_export_is_older_than_its_last_send();
    let reviewed = w.prepare().unwrap();
    assert!(reviewed.export_older_than_last_send);
    // The Library changes after the review.
    let (later, _) = w.library_track("later.mp3", "Later", true);
    w.crate_of("Later", &[later]);
    let recorded = w.recorded();

    assert_eq!(
        w.go_answering(false, NOTHING_IMPORTED_SINCE),
        Some(SendFailure::LibraryChanged)
    );
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.recorded(), recorded);
}

#[test]
fn a_no_does_not_lift_a_refusal_of_the_send() {
    let (w, file, recorded) = world_whose_export_is_older_than_its_last_send();
    // The same old export, now also incomplete: three tracks said, one held.
    let saved = fs::metadata(&w.export).unwrap().modified().unwrap();
    fs::write(
        &w.export,
        export_text(&[Rb(40, "known.mp3", "Known in rekordbox")], 3),
    )
    .unwrap();
    w.export_saved_at(saved.duration_since(SystemTime::UNIX_EPOCH).unwrap());

    let preflight = w.prepare().unwrap();
    assert!(preflight.export_older_than_last_send);
    assert_eq!(
        preflight.refusal.as_ref().map(|r| r.reason),
        Some(RefusalReason::IncompleteExport)
    );
    assert!(!preflight.can_send);
    assert_eq!(
        w.go_answering(true, NOTHING_IMPORTED_SINCE),
        Some(SendFailure::NotSendable)
    );
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.recorded(), recorded);
}

#[test]
fn only_a_plain_no_in_the_send_job_is_a_no() {
    let (w, ..) = world_whose_export_is_older_than_its_last_send();
    let token = w.prepare().unwrap().token;
    for (answer, stored) in [
        (NO_ANSWER, serde_json::Value::Null),
        (IMPORTED_SINCE, json!(true)),
        (NOTHING_IMPORTED_SINCE, json!(false)),
    ] {
        let job = w.flow.write_job(&token, false, answer);
        assert_eq!(job.target.unwrap()["importedSinceExport"], stored);
    }
}

/// 2026-10-01T10:00:00.123Z, written out by hand in both forms.
const SENT_AT: &str = "2026-10-01T10:00:00.123Z";
const SENT_AT_MS: u64 = 1_790_848_800_123;

#[test]
fn the_export_is_compared_with_the_last_send_to_the_millisecond() {
    let (w, ..) = world_with_a_send_waiting();
    // A send recorded at a time written here by hand, on a Library track
    // and nowhere else.
    w.writer
        .call(|c| {
            c.execute(
                "UPDATE library_track SET last_exported_at = ?1 WHERE id = 1",
                [SENT_AT],
            )
        })
        .unwrap();
    assert_eq!(w.last_send_ms(), Some(SENT_AT_MS as i64));
    let older = |w: &World| w.prepare().unwrap().export_older_than_last_send;

    w.export_saved_at(Duration::from_millis(SENT_AT_MS - 1));
    assert!(older(&w));

    w.export_saved_at(Duration::from_millis(SENT_AT_MS));
    assert!(!older(&w));

    w.export_saved_at(Duration::from_millis(SENT_AT_MS + 1));
    assert!(!older(&w));
}

#[test]
fn each_place_a_send_is_recorded_counts_on_its_own_and_the_latest_wins() {
    // Rows of each kind, with hand-written times; the other two kinds
    // hold an earlier time or none.
    let w = World::new();
    let (track, _) = w.library_track("a.mp3", "A", true);
    let set = |sql: &'static str| {
        w.writer.call(move |c| c.execute_batch(sql)).unwrap();
    };
    assert_eq!(w.last_send_ms(), None);

    set("UPDATE library_track SET last_exported_at = '2026-10-01T10:00:00.123Z'");
    assert_eq!(w.last_send_ms(), Some(1_790_848_800_123));

    // A sent crate, a second later.
    set(r#"INSERT INTO sent_playlist (path, path_key, kind, sent_at)
           VALUES ('["Crates","A"]', '["crates","a"]', 'playlist', '2026-10-01T10:00:01.005Z')"#);
    assert_eq!(w.last_send_ms(), Some(1_790_848_801_005));

    // A removed track's record, later still; then the track's own time
    // goes with the Library track.
    crate::library::remove(&w.writer, track).unwrap();
    set("UPDATE library_removal SET last_exported_at = '2026-10-01T10:00:02.999Z'");
    assert_eq!(w.last_send_ms(), Some(1_790_848_802_999));
    set("DELETE FROM sent_playlist");
    assert_eq!(w.last_send_ms(), Some(1_790_848_802_999));
}

#[test]
fn a_send_is_still_remembered_by_its_tracks_once_a_read_has_dropped_its_crates() {
    let (w, file, _) = world_whose_export_is_older_than_its_last_send();

    // What a later read does when rekordbox no longer has the sent crate:
    // its record goes. The tracks' own record of the send stays.
    w.writer
        .call(|c| c.execute_batch("DELETE FROM sent_playlist"))
        .unwrap();
    assert!(w.last_send_ms().is_some());

    // The export from before that send is still older than it.
    let preflight = w.prepare().unwrap();
    assert!(preflight.export_older_than_last_send);
    assert_eq!(
        w.go_answering(true, NO_ANSWER),
        Some(SendFailure::ExportOlderThanLastSend)
    );
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
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
fn a_go_whose_file_would_differ_from_the_reviewed_one_is_refused_and_writes_nothing() {
    // The read and the export are unchanged, and so is everything the
    // preflight says: only a value in the file differs.
    let (w, ..) = world_with_a_send_waiting();
    let reviewed = w.prepare().unwrap();
    let retitled =
        json!({ "id3v2": [{"key": "TIT2", "value": {"type": "text", "text": "Renamed"}}] })
            .to_string();
    w.insert(
        "UPDATE file SET raw_tags = ?1 WHERE rel_path = 'new.mp3'",
        (retitled,),
    );
    // Nothing the preflight shows has changed.
    let again = w
        .writer
        .call(|c| review(c, &Plugged))
        .unwrap()
        .unwrap()
        .preflight;
    assert_eq!(
        Preflight {
            token: String::new(),
            ..again.clone()
        },
        Preflight {
            token: String::new(),
            ..reviewed.clone()
        }
    );
    assert_ne!(again.token, reviewed.token);

    assert_eq!(w.go(true), Some(SendFailure::LibraryChanged));
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded().0, Vec::<String>::new());
    assert_eq!(w.flow.preflight(), None);
}

/// What a rekordbox read does, landing from another job: every rekordbox
/// row replaced, unmatched, and the read recorded as `read_at` of `path`.
fn another_read(writer: &Writer, text: String, path: Option<String>, read_at: &'static str) {
    try_another_read(writer, text, path, read_at).unwrap();
}

fn try_another_read(
    writer: &Writer,
    text: String,
    path: Option<String>,
    read_at: &'static str,
) -> Result<(), crate::db::DbError> {
    use crate::rekordbox::store::{replace_snapshot, SnapshotRows};
    let rows = SnapshotRows::from_xml(&RekordboxXml::parse(text.as_bytes()).unwrap());
    writer
        .call(move |c| {
            replace_snapshot(c, &rows, |tx, _, _| {
                tx.execute(
                    "UPDATE setting SET value = json_set(value, '$.readAt', ?1)
                     WHERE key = 'rekordbox_xml_last_read'",
                    [read_at],
                )?;
                if let Some(path) = &path {
                    tx.execute(
                        "UPDATE setting SET value = json_set(value, '$.path', ?1)
                         WHERE key = 'rekordbox_xml_last_read'",
                        [path],
                    )?;
                }
                Ok(())
            })
        })
        .map(drop)
}

#[test]
fn a_read_landing_during_prepare_never_makes_known_tracks_count_as_new() {
    // Another read replaces the snapshot right after prepare's own read:
    // its rows are unmatched. The preflight still matches them first.
    let (w, ..) = world_with_a_send_waiting();
    let (writer, text) = (w.writer.clone(), fs::read_to_string(&w.export).unwrap());
    let sender = w.sender().after_read(move || {
        another_read(&writer, text.clone(), None, "2031-01-01T00:00:00.000Z");
    });
    let status = w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Done);

    let preflight = w.flow.preflight().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    // The preflight is of the read whose rows it used, and sends as that.
    assert_eq!(preflight.export.read_at, "2031-01-01T00:00:00.000Z");
    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New"]);
}

#[test]
fn no_read_can_land_between_matching_the_tracks_and_building_the_preflight() {
    // A read tried at that very point: it can't get the database, because
    // the matching and the preflight are one writer job. Were they two, it
    // would land, leave every row unmatched, and the known track would
    // count as new.
    let (w, ..) = world_with_a_send_waiting();
    let (writer, text) = (w.writer.clone(), fs::read_to_string(&w.export).unwrap());
    let tried = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = tried.clone();
    let sender = w.sender().after_relink(move || {
        let landed = try_another_read(&writer, text.clone(), None, "2031-01-01T00:00:00.000Z");
        seen.lock().unwrap().push(landed.is_ok());
    });
    let status = w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Done);

    // The read was tried once, and refused.
    assert_eq!(*tried.lock().unwrap(), [false]);
    let preflight = w.flow.preflight().unwrap();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert_ne!(preflight.export.read_at, "2031-01-01T00:00:00.000Z");
}

#[test]
fn a_send_job_queued_by_another_run_of_the_app_does_nothing_whenever_it_runs() {
    // Whatever order startup does things in, a worker that meets a job
    // an earlier run left must not act on it.
    let (w, ..) = world_with_a_send_waiting();
    let earlier = SendFlow::new(w.guard.clone());
    let before = w.recorded();

    // Its prepare: the export isn't read, no preflight appears.
    let status = w.run_with(w.sender(), earlier.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Cancelled);
    assert_eq!(w.flow.preflight(), None);
    let read: i64 = w
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM rekordbox_track", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(read, 0);

    // Its write, even with this run's own preflight token and the confirm:
    // nothing is written, and the preflight is still there for a real go.
    let token = w.prepare().unwrap().token;
    let revision = w.flow.state(None).revision;
    let status = w.run_with(w.sender(), earlier.write_job(&token, true, None));
    assert_eq!(status, JobStatus::Cancelled);
    assert_eq!(w.data_files(), Vec::<String>::new());
    assert_eq!(w.recorded(), before);
    assert_eq!(w.flow.state(None).revision, revision);
    assert_eq!(w.flow.failure(), None);
    assert_eq!(w.go(false), None);

    // A job that names no run at all is treated the same.
    let nameless = NewJob::new(JobKind::Export)
        .target(json!({ "send": "write", "token": token, "confirmed": true }));
    assert_eq!(w.run_with(w.sender(), nameless), JobStatus::Cancelled);
}

#[test]
fn a_read_of_another_export_landing_during_prepare_leaves_no_preflight() {
    let (w, ..) = world_with_a_send_waiting();
    let (writer, text) = (w.writer.clone(), export_text(&[], 0));
    let sender = w.sender().after_read(move || {
        another_read(
            &writer,
            text.clone(),
            Some(r"C:\Elsewhere\other.xml".to_owned()),
            "2031-01-01T00:00:00.000Z",
        );
    });
    let status = w.run_with(sender, w.flow.prepare_job(&w.export.to_string_lossy()));
    assert_eq!(status, JobStatus::Failed);
    assert_eq!(w.flow.preflight(), None);
    assert_eq!(w.flow.failure(), Some(SendFailure::ExportChanged));
    assert_eq!(w.flow.state(None).step, Some(SendStep::Prepare));
}

#[test]
fn matching_rekordbox_tracks_again_after_the_review_does_not_refuse_the_go() {
    // Every read queues a background relink, which may run between the
    // review and the go. With nothing changed it changes nothing.
    let (w, ..) = world_with_a_send_waiting();
    w.prepare().unwrap();
    w.writer
        .call(|c| {
            let known = crate::relink::identities(c)?;
            let mounted = crate::relink::Mounted::ask(&known, &Plugged);
            crate::relink::relink(c, &mounted)
        })
        .unwrap();
    assert_eq!(w.go(false), None);
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New"]);
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
    // The send that went through is still shown as sent.
    assert!(w.flow.sent().is_some());
    assert_eq!(w.flow.state(None).step, Some(SendStep::Write));
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
    let (second, _) = w.library_track("second.mp3", "Second", true);
    // A crate the second send writes for the first time.
    w.crate_of("Late", &[second]);
    let recorded = w.recorded();
    w.save_export(&[Rb(40, "known.mp3", "Known in rekordbox")]);
    w.prepare().unwrap();
    (w, file, recorded)
}

#[test]
fn a_failed_write_leaves_the_last_sends_file_and_every_base_untouched() {
    let (w, file, recorded) = world_with_a_second_send_waiting();
    let paths = w.recorded_paths();
    assert_eq!(paths, [r#"["Crates","Warm up"]|playlist"#]);
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
    assert_eq!(w.recorded_paths(), paths);
    assert_eq!(w.flow.sent(), None);
    // With the way clear, the same preflight goes through.
    for name in &blockers {
        fs::remove_dir(w.guard.app_data_dir().join(name)).unwrap();
    }
    assert_eq!(w.go(false), None);
    assert_ne!(fs::read(w.send_file()).unwrap(), file);
}

fn failing_record(
    _: &mut rusqlite::Connection,
    _: &[SentTrack],
    _: &[crate::rekordbox_write::SentPath],
) -> rusqlite::Result<()> {
    Err(rusqlite::Error::InvalidQuery)
}

#[test]
fn a_failed_record_puts_the_last_sends_file_back_and_leaves_every_base_untouched() {
    let (w, file, recorded) = world_with_a_second_send_waiting();
    let paths = w.recorded_paths();
    let token = w.flow.preflight().unwrap().token;

    let failure = w.go_with(w.sender().recording_with(failing_record), &token, false);
    assert_eq!(failure, Some(SendFailure::CantRecord));
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.data_files(), [SEND_FILE_NAME]);
    assert_eq!(w.recorded(), recorded);
    assert_eq!(w.recorded_paths(), paths);
    assert_eq!(w.flow.sent(), None);
    // The same preflight can be tried again, and goes through.
    assert_eq!(w.go(false), None);
    assert_eq!(
        w.recorded_paths().len(),
        paths.len() + 1,
        "the new crate's path"
    );
    assert_eq!(w.sent_titles(), ["Known in rekordbox", "New", "Second"]);
    assert_ne!(w.recorded(), recorded);
}

#[test]
fn record_sends_own_rollback_also_puts_the_last_sends_file_back() {
    // The real record, failing partway: a sent track that's no longer in
    // the Library makes it roll back.
    let (w, file, recorded) = world_with_a_second_send_waiting();
    let paths = w.recorded_paths();
    let token = w.flow.preflight().unwrap().token;
    let partway = |c: &mut rusqlite::Connection,
                   sent: &[SentTrack],
                   paths: &[crate::rekordbox_write::SentPath]| {
        let mut sent = sent.to_vec();
        sent.last_mut().unwrap().library_track = LibraryTrackId(9_999);
        crate::rekordbox_write::record_send(c, &sent, paths)
    };

    let failure = w.go_with(w.sender().recording_with(partway), &token, false);
    assert_eq!(failure, Some(SendFailure::CantRecord));
    assert_eq!(fs::read(w.send_file()).unwrap(), file);
    assert_eq!(w.recorded(), recorded);
    // The paths were written before the track that failed: they roll back too.
    assert_eq!(w.recorded_paths(), paths);
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

    // The send recorded every folder and playlist it wrote (1aF-2), in the
    // same transaction as the bases: what the stale-playlist list works from.
    let recorded: Vec<(String, String)> = w
        .writer
        .call(|c| {
            c.prepare("SELECT path, kind FROM sent_playlist ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })
        .unwrap();
    assert_eq!(
        recorded,
        [
            (r#"["Crates","Opening"]"#.to_owned(), "playlist".to_owned()),
            (r#"["Crates","Friday"]"#.to_owned(), "folder".to_owned()),
            (
                r#"["Crates","Friday","Peak"]"#.to_owned(),
                "playlist".to_owned()
            ),
        ]
    );
}

// --- send jobs never outlive a run of the app ----------------------------------------

#[test]
fn unfinished_send_jobs_are_dropped_and_no_other_job_is() {
    let w = World::new();
    let job = |kind: &'static str, status: &'static str, target: Option<serde_json::Value>| {
        w.insert(
            "INSERT INTO job (kind, target, status, started_at, finished_at)
             VALUES (?1, ?2, ?3, CASE WHEN ?3 <> 'queued' THEN 'earlier' END,
                     CASE WHEN ?3 = 'done' THEN 'earlier' END)",
            (kind, target.map(|t| t.to_string()), status),
        )
    };
    let prepare = w.flow.prepare_job("C:\\x.xml").target;
    let write = w.flow.write_job("token", true, None).target;
    let dropped = [
        job("export", "queued", prepare.clone()),
        job("export", "queued", write.clone()),
        job("export", "running", prepare.clone()),
        job("export", "running", write.clone()),
    ];
    let kept = [
        (job("export", "done", write), "done"),
        (job("export", "queued", None), "queued"),
        (
            job("export", "queued", Some(json!({ "other": 1 }))),
            "queued",
        ),
        (job("scan", "queued", None), "queued"),
        (job("read_rekordbox", "queued", prepare), "queued"),
    ];

    let count = w.writer.call(|c| drop_unfinished_jobs(c)).unwrap();
    assert_eq!(count, dropped.len());
    let status = |id: i64| {
        w.writer
            .call(move |c| jobs::store::get(c, jobs::JobId(id)))
            .unwrap()
            .unwrap()
    };
    for id in dropped {
        let job = status(id);
        assert_eq!(job.status, JobStatus::Cancelled, "{id}");
        assert!(job.finished_at.is_some());
    }
    for (id, expected) in kept {
        assert_eq!(status(id).status.as_str(), expected, "{id}");
    }
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
    #[allow(deprecated)]
    fn send_jobs_left_by_an_earlier_run_never_run_at_the_next_start() {
        // An earlier run's database: a prepare and a write, still queued.
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let export = dir.path().join("rekordbox.xml");
        std::fs::write(&export, super::export_text(&[], 0)).unwrap();
        let (prepare, write) = {
            let guard = WriteGuard::app_data(&data).unwrap();
            let writer = crate::db::Writer::open(
                &guard
                    .check(&crate::db::db_path(guard.app_data_dir()))
                    .unwrap(),
            )
            .unwrap();
            let queue = crate::jobs::JobQueue::builder(writer.clone())
                .workers(1)
                .start()
                .unwrap();
            // No worker takes them: the queue is stopped first.
            queue.shutdown();
            let earlier = crate::send::SendFlow::new(guard.clone());
            (
                queue
                    .enqueue(earlier.prepare_job(&export.to_string_lossy()))
                    .unwrap(),
                queue
                    .enqueue(earlier.write_job("token", true, None))
                    .unwrap(),
            )
        };

        let mut app = crate::setup(tauri::test::mock_builder(), crate::DataDir::At(data))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.run_iteration(|_, _| {});

        // Dropped before the queue started, so neither can have run.
        let writer = app.state::<crate::db::Writer>();
        for id in [prepare, write] {
            let job = writer
                .call(move |c| crate::jobs::store::get(c, id))
                .unwrap()
                .unwrap();
            assert_eq!(job.status, crate::jobs::JobStatus::Cancelled);
            assert_eq!(job.attempts, 0);
        }
        // The export wasn't read, and the checklist shows no failure.
        let state = invoke(&app, "send_state", json!({})).unwrap();
        assert_eq!(state["revision"], json!(0));
        assert_eq!(state["failure"], Value::Null);
        assert_eq!(state["exportPath"], Value::Null);
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

#[test]
fn the_preflights_track_lists_name_each_track_by_what_a_send_writes() {
    let w = World::new();
    // No recording here has a title of its own.
    let untitled = |name: &str, tag: &str, present: bool| {
        let recording = w.insert("INSERT INTO recording DEFAULT VALUES", ());
        let file = w.file(recording, name, tag, present);
        let library = w.insert(
            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (?1, 'linked', ?2, 'ok')",
            (recording, file),
        );
        (recording, LibraryTrackId(library))
    };
    // Left out: its file is gone and rekordbox doesn't have it: its stored tag.
    let (_, gone) = untitled("gone.mp3", "Remembered title", false);
    // Left out for a character XML can't carry: shown as it would be sent.
    let (_, bell) = untitled("bell.mp3", "bell\u{7}title", true);
    // rekordbox has it but its file is gone: rekordbox's own name and artist.
    let (_, known_gone) = untitled("known-gone.mp3", "Tag of a known track", false);
    // rekordbox holds another file of it: this file's tag.
    let (sibling, linked) = untitled("linked.mp3", "Tag of the linked file", true);
    w.file(sibling, "other.mp3", "Tag of the other file", true);
    // rekordbox's own entry is only sent for a track a crate names.
    w.crate_of("Warm up", &[known_gone]);
    w.save_export(&[
        Rb(40, "known-gone.mp3", "Their title"),
        Rb(41, "other.mp3", "Their other title"),
    ]);

    let preflight = w.prepare().unwrap();
    let left_out: Vec<_> = preflight
        .left_out
        .iter()
        .map(|l| {
            (
                l.track.library_track,
                l.track.title.as_deref(),
                l.track.artist.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        left_out,
        [
            (gone, Some("Remembered title"), None),
            (bell, Some("bell\u{7}title"), None),
        ]
    );
    let file_missing: Vec<_> = preflight
        .file_missing
        .iter()
        .map(|t| (t.library_track, t.title.as_deref(), t.artist.as_deref()))
        .collect();
    // The export's own Artist for the track.
    assert_eq!(
        file_missing,
        [(known_gone, Some("Their title"), Some("Kit"))]
    );
    let other_file: Vec<_> = preflight
        .other_file
        .iter()
        .map(|o| (o.track.library_track, o.track.title.as_deref()))
        .collect();
    assert_eq!(other_file, [(linked, Some("Tag of the linked file"))]);
}
