//! The world a run happens in: a generated music folder, the folder
//! rekordbox saves its export to, and the app, started the way `run`
//! starts it but with its data folder in the sandbox.
//!
//! A [`World`] is driven only through what a user can do: the app's
//! commands, called over IPC as the frontend calls them, and the jobs
//! those commands queue. It waits for jobs on the job queue's own update
//! events, never on a clock.
//!
//! Everything outside the app data folder is recorded when the world is
//! made (path, size, content hash, modified time). The user's and
//! rekordbox's own changes ([`World::rekordbox_saves`],
//! [`World::user_deletes`]) are the only ones allowed; when the world
//! ends, anything else that was added, removed or changed fails the test.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tauri::test::{mock_builder, MockRuntime};
use tauri::Manager;
use tauri_specta::Event;
use tracklist_pro_lib::all_music::AllMusicList;
use tracklist_pro_lib::crates::CrateId;
use tracklist_pro_lib::jobs::{self, ActivitySnapshot, JobId, JobRecord, JobStatus};
use tracklist_pro_lib::library::{LibraryTrack, LibraryTrackId, Promoted};
use tracklist_pro_lib::rekordbox::RekordboxXml;
use tracklist_pro_lib::send::{Preflight, SendFailure, SendState};
use tracklist_pro_lib::{db, scan, IDENTIFIER};

/// A generated audio file: where it goes in the music folder, its tags,
/// and which audio it holds. Two songs with the same `audio` are
/// duplicates: the same audio under two names.
#[derive(Debug, Clone, Copy)]
pub struct Song {
    /// Its path inside the music folder, `/`-separated.
    pub rel: &'static str,
    pub title: &'static str,
    pub artist: &'static str,
    pub genre: &'static str,
    pub audio: u32,
}

impl Song {
    /// The file's name, without its folders.
    pub fn file_name(&self) -> &'static str {
        self.rel.rsplit('/').next().unwrap()
    }
}

/// How long every generated file plays, in whole seconds.
pub const SONG_SECONDS: u32 = 3;
const SAMPLE_RATE: u32 = 8000;

/// A mono 16-bit WAV: [`SONG_SECONDS`] of quiet noise seeded by the
/// song's `audio`, then its tags as a RIFF INFO list.
fn wav(song: &Song) -> Vec<u8> {
    let samples = (SAMPLE_RATE * SONG_SECONDS) as usize;
    let mut data = Vec::with_capacity(samples * 2);
    let mut state = song.audio.wrapping_mul(2_654_435_761).wrapping_add(1);
    for _ in 0..samples {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let v = ((state >> 16) % 4000) as i16 - 2000;
        data.extend_from_slice(&v.to_le_bytes());
    }

    let mut info = b"INFO".to_vec();
    for (id, text) in [
        (b"INAM", song.title),
        (b"IART", song.artist),
        (b"IGNR", song.genre),
    ] {
        info.extend_from_slice(id);
        info.extend_from_slice(&(text.len() as u32 + 1).to_le_bytes());
        info.extend_from_slice(text.as_bytes());
        info.push(0);
        if info.len() % 2 == 1 {
            info.push(0);
        }
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((4 + 24 + 8 + data.len() + 8 + info.len()) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&data);
    bytes.extend_from_slice(b"LIST");
    bytes.extend_from_slice(&(info.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&info);
    bytes
}

/// What's recorded about everything outside the app data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    File {
        size: u64,
        /// blake3 of its bytes.
        hash: String,
        modified: SystemTime,
    },
    Folder {
        modified: SystemTime,
    },
}

/// `dir` itself and everything under it.
fn record(dir: &Path, out: &mut BTreeMap<PathBuf, Entry>) {
    let modified = fs::metadata(dir).unwrap().modified().unwrap();
    out.insert(dir.to_owned(), Entry::Folder { modified });
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            record(&path, out);
        } else {
            let bytes = fs::read(&path).unwrap();
            out.insert(
                path,
                Entry::File {
                    size: meta.len(),
                    hash: blake3::hash(&bytes).to_hex().to_string(),
                    modified: meta.modified().unwrap(),
                },
            );
        }
    }
}

/// No job may go this long without a single update. Only a hung job
/// reaches it; nothing is measured against it.
const PATIENCE: Duration = Duration::from_secs(10 * 60);

/// The sandbox and the running app.
pub struct World {
    app: tauri::App<MockRuntime>,
    webview: tauri::WebviewWindow<MockRuntime>,
    /// One message per batch of job updates the app sends its webview.
    updates: mpsc::Receiver<()>,
    base: PathBuf,
    /// The music folder the user points the app at.
    pub music: PathBuf,
    /// Where rekordbox saves its export.
    documents: PathBuf,
    /// The app data folder.
    pub app_data: PathBuf,
    /// Everything under the music folder and beside the export, as the
    /// user and rekordbox last left it.
    outside: BTreeMap<PathBuf, Entry>,
    /// When rekordbox last saved its export, in seconds since the epoch.
    saved: u64,
    // Last, so the app has let go of its files before the folder goes.
    _dir: tempfile::TempDir,
}

impl World {
    /// A music folder holding `songs`, and the app started on an empty
    /// data folder. Nothing is scanned or read yet.
    pub fn new(songs: &[Song]) -> World {
        let dir = tempfile::Builder::new()
            .prefix("tlp-end-to-end-")
            .tempdir()
            .unwrap();
        // The folder as it really is on disk (no 8.3 short names), written
        // the way a user would paste it.
        let base = fs::canonicalize(dir.path()).unwrap();
        let base = PathBuf::from(base.to_string_lossy().trim_start_matches(r"\\?\"));
        let music = base.join("music");
        let documents = base.join("documents");
        let app_data = base.join("appdata").join(IDENTIFIER);
        fs::create_dir(&music).unwrap();
        fs::create_dir(&documents).unwrap();
        fs::create_dir(base.join("appdata")).unwrap();
        for song in songs {
            let path = music.join(song.rel.replace('/', "\\"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, wav(song)).unwrap();
        }

        let (app, webview, updates) = open_app(app_data.clone());

        let mut world = World {
            app,
            webview,
            updates,
            base,
            music,
            documents,
            app_data,
            outside: BTreeMap::new(),
            saved: 0,
            _dir: dir,
        };
        world.outside = world.outside_now();
        world
    }

    /// The app is killed, and started again on the same data folder.
    /// `left_behind` runs in between, on the database as the dead run
    /// left it: the place to put what a kill leaves there.
    pub fn kill_and_start_again(
        &mut self,
        left_behind: impl FnOnce(&rusqlite::Connection) + Send + 'static,
    ) {
        self.stop_app();
        self.writer()
            .call(move |c| {
                left_behind(c);
                Ok(())
            })
            .unwrap();
        let (app, webview, updates) = open_app(self.app_data.clone());
        self.webview = webview;
        self.updates = updates;
        self.app = app;
    }

    /// What closing the app does: no watcher or job outlives the run.
    fn stop_app(&self) {
        if let Some(watchers) = self.app.try_state::<scan::Watchers>() {
            watchers.shutdown();
        }
        if let Some(queue) = self.app.try_state::<jobs::JobQueue>() {
            queue.shutdown();
        }
    }

    /// Every job's kind and status, as stored, oldest first.
    pub fn jobs(&self) -> Vec<(JobId, String, String)> {
        self.writer()
            .call(|c| {
                let mut stmt = c.prepare("SELECT id, kind, status FROM job ORDER BY id")?;
                let rows = stmt.query_map([], |r| Ok((JobId(r.get(0)?), r.get(1)?, r.get(2)?)))?;
                rows.collect()
            })
            .unwrap()
    }

    /// A job's row, as stored.
    pub fn job(&self, id: JobId) -> JobRecord {
        self.writer()
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap()
    }

    // --- the disk outside the app data folder ---

    fn outside_now(&self) -> BTreeMap<PathBuf, Entry> {
        let mut out = BTreeMap::new();
        record(&self.music, &mut out);
        record(&self.documents, &mut out);
        out
    }

    /// Everything that differs from how the user and rekordbox last left
    /// the music folder and the export's folder: each entry added, removed
    /// or changed (a file's bytes, size or modified time, a folder's
    /// modified time), and anything in the sandbox that isn't under the
    /// app data folder.
    pub fn changes_outside_the_app_data_folder(&self) -> Vec<String> {
        let now = self.outside_now();
        let mut changes = Vec::new();
        for (path, entry) in &self.outside {
            match now.get(path) {
                None => changes.push(format!("{} was removed", path.display())),
                Some(found) if found != entry => changes.push(format!(
                    "{} was changed: {entry:?} became {found:?}",
                    path.display()
                )),
                Some(_) => {}
            }
        }
        for path in now.keys() {
            if !self.outside.contains_key(path) {
                changes.push(format!("{} was added", path.display()));
            }
        }
        // Whatever the app wrote is under its data folder: the sandbox
        // holds nothing else.
        for (folder, expected) in [
            (self.base.clone(), vec!["appdata", "documents", "music"]),
            (self.base.join("appdata"), vec![IDENTIFIER]),
        ] {
            for entry in fs::read_dir(&folder).unwrap() {
                let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                if !expected.contains(&name.as_str()) {
                    changes.push(format!("{} was added", folder.join(name).display()));
                }
            }
        }
        changes
    }

    /// Fails unless nothing outside the app data folder has changed
    /// ([`World::changes_outside_the_app_data_folder`]).
    pub fn assert_nothing_outside_the_app_data_folder_changed(&self) {
        let changes = self.changes_outside_the_app_data_folder();
        assert!(changes.is_empty(), "{changes:#?}");
    }

    /// rekordbox saves its export (File → Export Collection), later than
    /// any save before and than any send so far. Returns the export's path.
    pub fn rekordbox_saves(&mut self, text: &str) -> String {
        self.assert_nothing_outside_the_app_data_folder_changed();
        let path = self.export();
        fs::write(&path, text).unwrap();
        // A minute after the save before, and a minute after the last
        // send the app recorded: rekordbox exports after the import. The
        // first export is from 2025, before anything the app does.
        let last_send: Option<i64> = self
            .writer()
            .call(|c| {
                c.query_row(
                    "SELECT max(CAST(strftime('%s', at) AS INTEGER))
                     FROM (SELECT last_exported_at AS at FROM library_track
                           UNION ALL SELECT last_exported_at FROM library_removal
                           UNION ALL SELECT sent_at FROM sent_playlist)",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        let after_the_send = last_send.map_or(0, |s| u64::try_from(s).unwrap());
        self.saved = self.saved.max(after_the_send).max(1_750_000_000) + 60;
        let saved = SystemTime::UNIX_EPOCH + Duration::from_secs(self.saved);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(saved)
            .unwrap();
        self.outside = self.outside_now();
        path.to_string_lossy().into_owned()
    }

    /// The user puts a new file in the music folder, outside the app.
    pub fn user_adds(&mut self, song: &Song) {
        self.assert_nothing_outside_the_app_data_folder_changed();
        let path = self.path_of(song);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, wav(song)).unwrap();
        self.outside = self.outside_now();
    }

    /// The user deletes a file from the music folder, outside the app.
    pub fn user_deletes(&mut self, song: &Song) {
        self.assert_nothing_outside_the_app_data_folder_changed();
        fs::remove_file(self.path_of(song)).unwrap();
        self.outside = self.outside_now();
    }

    /// Where rekordbox saves its export.
    pub fn export(&self) -> PathBuf {
        self.documents.join("rekordbox.xml")
    }

    /// A song's full path, as Windows writes it.
    pub fn path_of(&self, song: &Song) -> PathBuf {
        self.music.join(song.rel.replace('/', "\\"))
    }

    // --- the app, as the frontend reaches it ---

    /// Calls a command over IPC and returns its answer or its error.
    pub fn try_call<T: DeserializeOwned>(&self, cmd: &str, args: Value) -> Result<T, Value> {
        let request = tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        };
        tauri::test::get_ipc_response(&self.webview, request).map(|body| {
            body.deserialize()
                .unwrap_or_else(|e| panic!("{cmd} answered something unexpected: {e}"))
        })
    }

    /// Calls a command that must succeed.
    pub fn call<T: DeserializeOwned>(&self, cmd: &str, args: Value) -> T {
        self.try_call(cmd, args)
            .unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
    }

    /// Waits until no job is queued or running. A job that queues another
    /// does so before it's recorded as finished, so an empty Activity
    /// means the whole chain is through.
    pub fn settle(&self) {
        loop {
            let activity: ActivitySnapshot = self.call("activity", json!({}));
            if activity.jobs.is_empty() {
                return;
            }
            self.updates
                .recv_timeout(PATIENCE)
                .expect("a job never finished");
        }
    }

    /// Calls a command that queues a job, waits for it and everything it
    /// queues in turn, and returns the job as it ended.
    pub fn run_job(&self, cmd: &str, args: Value) -> JobRecord {
        let id: JobId = self.call(cmd, args);
        self.settle();
        let writer = self.writer();
        writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap()
    }

    /// [`World::run_job`], for a job that must succeed.
    pub fn job_done(&self, cmd: &str, args: Value) {
        let job = self.run_job(cmd, args);
        assert_eq!(job.status, JobStatus::Done, "{cmd}: {:?}", job.error);
    }

    fn writer(&self) -> db::Writer {
        self.app.state::<db::Writer>().inner().clone()
    }

    // --- what the user does ---

    /// "Add folder": the music folder is added, and the scan that adding
    /// it queues runs through every stage the scan chains (read, hashes,
    /// fingerprints, grouping, relink, attach). Nothing else is asked for.
    pub fn add_music_folder_and_scan(&self) {
        self.add_folder_and_scan(&self.music);
    }

    /// [`World::add_music_folder_and_scan`], for a folder inside the
    /// generated one.
    pub fn add_folder_and_scan(&self, folder: &Path) {
        let before = self.jobs().len();
        let _: Value = self.call(
            "add_music_folder",
            json!({ "path": folder.to_string_lossy(), "role": null }),
        );
        self.settle();
        let scans: Vec<_> = self.jobs()[before..]
            .iter()
            .filter(|(_, kind, _)| kind == "scan")
            .map(|(_, _, status)| status.clone())
            .collect();
        assert_eq!(scans, ["done"], "adding a folder scans it once");
    }

    /// Scans the music folders again.
    pub fn scan(&self) {
        self.job_done("scan_music_folders", json!({ "ids": null }));
    }

    /// Reads rekordbox's export, and lets the matching and attaching it
    /// queues finish.
    pub fn read_rekordbox(&self) {
        self.job_done(
            "read_rekordbox_xml",
            json!({ "path": self.export().to_string_lossy() }),
        );
    }

    pub fn library(&self) -> Vec<LibraryTrack> {
        self.call("library_tracks", json!({}))
    }

    /// The Library track linked to this song's file.
    pub fn library_track(&self, song: &Song) -> LibraryTrack {
        self.library()
            .into_iter()
            .find(|t| t.file.as_ref().is_some_and(|f| f.name == song.file_name()))
            .unwrap_or_else(|| panic!("{} isn't in the Library", song.rel))
    }

    pub fn all_music(&self) -> AllMusicList {
        self.call("all_music_tracks", json!({ "search": null }))
    }

    /// "Add to Library" on the All music row showing this song's file.
    pub fn add_from_all_music(&self, song: &Song) -> LibraryTrack {
        let row = self
            .all_music()
            .tracks
            .into_iter()
            .find(|t| t.file.as_ref().is_some_and(|f| f.name == song.file_name()))
            .unwrap_or_else(|| panic!("{} isn't in All music", song.rel));
        let promoted: Promoted =
            self.call("promote_track", json!({ "recordingId": row.recording_id }));
        assert!(promoted.added, "{} was already in the Library", song.rel);
        promoted.library_track
    }

    /// "Remove from Library".
    pub fn remove_from_library(&self, id: LibraryTrackId) {
        let _: Value = self.call("remove_library_track", json!({ "id": id }));
    }

    /// "New crate", then its tracks added in one go. Give the tracks in
    /// the order they joined the Library: tracks added together come in
    /// that order, whatever order they're given in.
    pub fn create_crate(&self, name: &str, tracks: &[LibraryTrackId]) -> CrateId {
        let id: CrateId = self.call("create_crate", json!({ "name": name }));
        let added: Value = self.call("add_tracks_to_crate", json!({ "id": id, "tracks": tracks }));
        assert_eq!(added["changed"], tracks.len(), "{added}");
        id
    }

    pub fn rename_crate(&self, id: CrateId, name: &str) {
        let _: Value = self.call("rename_crate", json!({ "id": id, "name": name }));
    }

    pub fn send_state(&self) -> SendState {
        self.call("send_state", json!({}))
    }

    /// The send's first step: reads the export again and builds the
    /// preflight. Returns what the checklist then shows.
    pub fn prepare_send(&self) -> SendState {
        self.run_job(
            "prepare_send",
            json!({ "path": self.export().to_string_lossy() }),
        );
        self.send_state()
    }

    /// The preflight of a send that's ready to go.
    pub fn prepared(&self) -> Preflight {
        let state = self.prepare_send();
        assert_eq!(state.failure, None);
        state.preflight.expect("the prepare step left a preflight")
    }

    /// The user's go. Returns what the checklist then shows.
    pub fn write_send(&self, token: &str, confirmed: bool) -> SendState {
        self.write_send_answering(token, confirmed, None)
    }

    /// The user's go, with their answer to the checklist's question about
    /// an export older than the last send: has a send been imported into
    /// rekordbox since it was saved? (`None`: not asked, or not answered.)
    pub fn write_send_answering(
        &self,
        token: &str,
        confirmed: bool,
        imported_since_export: Option<bool>,
    ) -> SendState {
        self.run_job(
            "write_send",
            json!({
                "token": token,
                "confirmed": confirmed,
                "importedSinceExport": imported_since_export,
            }),
        );
        self.send_state()
    }

    /// Writes the send `preflight` describes, which must go through, and
    /// reads the file back with the app's own reader.
    pub fn write_and_read_back(&self, preflight: &Preflight, confirmed: bool) -> RekordboxXml {
        let state = self.write_send(&preflight.token, confirmed);
        assert_eq!(state.failure, None::<SendFailure>);
        let sent = state.sent.expect("the send is recorded");
        assert_eq!(
            (sent.new_tracks, sent.known_tracks),
            (preflight.new_tracks, preflight.known_tracks)
        );
        self.read_sent_file()
    }

    /// A whole send that needs no confirm: prepare, go, read back.
    pub fn send(&self) -> RekordboxXml {
        let preflight = self.prepared();
        assert!(preflight.can_send, "{preflight:?}");
        assert!(!preflight.needs_confirm, "{preflight:?}");
        self.write_and_read_back(&preflight, false)
    }

    /// The one file a send writes, where the checklist says it is.
    pub fn sent_file(&self) -> PathBuf {
        let path = PathBuf::from(self.send_state().file_path);
        assert!(
            path.starts_with(&self.app_data),
            "the send file {} is outside the app data folder",
            path.display()
        );
        path
    }

    pub fn sent_bytes(&self) -> Vec<u8> {
        fs::read(self.sent_file()).unwrap()
    }

    pub fn read_sent_file(&self) -> RekordboxXml {
        RekordboxXml::read_file(&self.sent_file()).unwrap()
    }

    /// The two after-send lists, as the screen gets them.
    pub fn after_send_lists(&self) -> Value {
        self.call("after_send_lists", json!({}))
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.stop_app();
        if !std::thread::panicking() {
            self.assert_nothing_outside_the_app_data_folder_changed();
        }
    }
}

/// The app started on `data_dir`, its main window, and a channel that
/// hears each batch of job updates the app sends that window.
fn open_app(
    data_dir: PathBuf,
) -> (
    tauri::App<MockRuntime>,
    tauri::WebviewWindow<MockRuntime>,
    mpsc::Receiver<()>,
) {
    let app = start_app(data_dir);
    // Startup opened the main window from the config.
    let webview = app.get_webview_window("main").unwrap();
    let (heard, updates) = mpsc::channel();
    let heard = Mutex::new(heard);
    jobs::JobUpdates::listen_any(app.handle(), move |_| {
        let _ = heard.lock().unwrap().send(());
    });
    (app, webview, updates)
}

/// Starts the app on Tauri's mock runtime with its data folder at
/// `data_dir`: the app's own startup (`setup` in `lib.rs`, through its
/// test entry point) with the compiled-in `tauri.conf.json`, so the
/// plugins, the state, the job queue and the guarded main window are the
/// ones `run` makes. Only the runtime and the data folder differ.
#[allow(deprecated)]
fn start_app(data_dir: PathBuf) -> tauri::App<MockRuntime> {
    let mut app = tracklist_pro_lib::setup_for_tests(mock_builder(), data_dir)
        .build(tauri::generate_context!(test = true))
        .unwrap();
    // The startup hook runs when the event loop starts; on the mock
    // runtime one iteration runs it and returns.
    app.run_iteration(|_, _| {});
    app
}

/// The disk record is only worth something if it notices. Each change is
/// made by the test itself, behind the app's back, and then put right so
/// the world can end.
mod the_disk_record {
    use super::*;

    const SONGS: [Song; 2] = [
        Song {
            rel: "One.wav",
            title: "Generated One",
            artist: "Generated Artist",
            genre: "House",
            audio: 1,
        },
        Song {
            rel: "Deep/Two.wav",
            title: "Generated Two",
            artist: "Generated Artist",
            genre: "House",
            audio: 2,
        },
    ];

    fn mentions(changes: &[String], path: &Path, what: &str) -> bool {
        let path = path.display().to_string();
        changes
            .iter()
            .any(|c| c.starts_with(&path) && c.contains(what))
    }

    #[test]
    fn notices_a_file_added_removed_or_rewritten_in_the_music_folder() {
        let mut world = World::new(&SONGS);
        assert_eq!(world.changes_outside_the_app_data_folder(), [] as [&str; 0]);

        let added = world.music.join("Deep").join("extra.txt");
        fs::write(&added, "x").unwrap();
        let changes = world.changes_outside_the_app_data_folder();
        assert!(mentions(&changes, &added, "was added"), "{changes:?}");
        fs::remove_file(&added).unwrap();

        // Same size, same modified time, other bytes: only the content
        // hash can tell.
        let one = world.path_of(&SONGS[0]);
        let original = fs::read(&one).unwrap();
        let modified = fs::metadata(&one).unwrap().modified().unwrap();
        let mut rewritten = original.clone();
        *rewritten.last_mut().unwrap() ^= 1;
        fs::write(&one, &rewritten).unwrap();
        fs::File::options()
            .write(true)
            .open(&one)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let changes = world.changes_outside_the_app_data_folder();
        assert!(mentions(&changes, &one, "was changed"), "{changes:?}");

        fs::remove_file(&one).unwrap();
        let changes = world.changes_outside_the_app_data_folder();
        assert!(mentions(&changes, &one, "was removed"), "{changes:?}");

        // From here on the folder is the user's own doing.
        world.outside = world.outside_now();
    }

    #[test]
    fn notices_a_file_touched_without_its_bytes_changing_and_a_stray_file_in_the_sandbox() {
        let mut world = World::new(&SONGS);

        let two = world.path_of(&SONGS[1]);
        let modified = fs::metadata(&two).unwrap().modified().unwrap();
        let handle = fs::File::options().write(true).open(&two).unwrap();
        handle
            .set_modified(modified + Duration::from_secs(60))
            .unwrap();
        let changes = world.changes_outside_the_app_data_folder();
        assert!(mentions(&changes, &two, "was changed"), "{changes:?}");
        handle.set_modified(modified).unwrap();
        drop(handle);

        let stray = world.base.join("stray.xml");
        fs::write(&stray, "x").unwrap();
        let changes = world.changes_outside_the_app_data_folder();
        assert!(mentions(&changes, &stray, "was added"), "{changes:?}");
        fs::remove_file(&stray).unwrap();

        world.outside = world.outside_now();
    }
}
