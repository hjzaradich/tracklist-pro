//! 2C-R1: the database reader, end to end, on synthetic databases only:
//! made-up tables in rekordbox's shape, encrypted by the test-only inverse
//! ([`super::cipher::inverse`]) under made-up keys.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{ffi, Connection};

use super::*;
use crate::db::Writer;
use crate::jobs::{self, JobStatus};
use crate::paths::Volumes;
use crate::rekordbox::source::{self, XmlReader};
use crate::volume::{Volume, VolumeId};

const SCHEMA: &str = "
CREATE TABLE djmdContent (ID VARCHAR(255) PRIMARY KEY, FolderPath, Title, ArtistID,
  ComposerID, AlbumID, GenreID, FileType INTEGER, FileSize INTEGER, Length INTEGER,
  DiscNo INTEGER, TrackNo INTEGER, ReleaseYear INTEGER, BPM INTEGER, StockDate,
  DateCreated, BitRate INTEGER, SampleRate INTEGER, Commnt, DJPlayCount INTEGER,
  Rating INTEGER, RemixerID, KeyID, LabelID, Subtitle, ColorID,
  rb_local_deleted INTEGER DEFAULT 0);
CREATE TABLE djmdArtist (ID, Name);
CREATE TABLE djmdAlbum (ID, Name);
CREATE TABLE djmdGenre (ID, Name);
CREATE TABLE djmdKey (ID, ScaleName);
CREATE TABLE djmdLabel (ID, Name);
CREATE TABLE djmdColor (ID, ColorCode INTEGER, Commnt);
CREATE TABLE djmdPlaylist (ID, Seq INTEGER, Name, Attribute INTEGER, ParentID,
  rb_local_deleted INTEGER DEFAULT 0);
CREATE TABLE djmdSongPlaylist (ID, PlaylistID, ContentID, TrackNo INTEGER,
  rb_local_deleted INTEGER DEFAULT 0);
INSERT INTO djmdArtist VALUES ('1', 'Synthetic Artist'), ('2', 'Made-up Remixer');
INSERT INTO djmdAlbum VALUES ('1', 'Test Album');
INSERT INTO djmdGenre VALUES ('1', 'House');
INSERT INTO djmdKey VALUES ('1', '8A');
INSERT INTO djmdLabel VALUES ('1', 'No Label');
INSERT INTO djmdColor VALUES ('1', 16711935, 'Pink');
INSERT INTO djmdContent (ID, FolderPath, Title, ArtistID, AlbumID, GenreID, FileType,
  FileSize, Length, TrackNo, ReleaseYear, BPM, StockDate, BitRate, SampleRate, Commnt,
  DJPlayCount, Rating, RemixerID, KeyID, LabelID, Subtitle, ColorID) VALUES
  ('101', 'C:/Kit/One & Two.mp3', 'Synthetic One', '1', '1', '1', 1, 5000000, 300, 1,
   2024, 12800, '2025-01-02', 320, 44100, 'a comment', 7, 3, '2', '1', '1',
   'Extended Mix', '1'),
  ('102', 'C:/Kit/two.flac', 'Synthetic Two', '1', NULL, NULL, 5, 9000000, 240, 2,
   NULL, 12450, NULL, 900, 44100, NULL, 0, 0, NULL, NULL, NULL, NULL, NULL);
INSERT INTO djmdContent (ID, FolderPath, Title, rb_local_deleted)
  VALUES ('103', 'C:/Kit/gone.mp3', 'Deleted', 1);
INSERT INTO djmdPlaylist VALUES
  ('10', 0, 'Folder', 1, 'root', 0),
  ('11', 0, 'Peak time', 0, '10', 0),
  ('12', 1, 'Smart', 4, 'root', 0),
  ('13', 2, 'Removed', 0, 'root', 1);
INSERT INTO djmdSongPlaylist VALUES
  ('1', '11', '102', 1, 0), ('2', '11', '101', 2, 0), ('3', '11', '103', 3, 1);
";

/// The synthetic library as a plain SQLite file laid out as SQLCipher
/// lays out its pages: 4096 bytes, the last 80 reserved.
fn plain_library(dir: &Path) -> Vec<u8> {
    plain_library_with(dir, "")
}

/// The same, with `extra` SQL run on it first.
fn plain_library_with(dir: &Path, extra: &str) -> Vec<u8> {
    let path = dir.join("plain-fixture.db");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "page_size", 4096).unwrap();
    let mut reserve: std::ffi::c_int = 80;
    // SAFETY: a live connection, its main database, and an int argument,
    // as SQLITE_FCNTL_RESERVE_BYTES takes.
    let rc = unsafe {
        ffi::sqlite3_file_control(
            conn.handle(),
            c"main".as_ptr(),
            ffi::SQLITE_FCNTL_RESERVE_BYTES,
            (&mut reserve as *mut std::ffi::c_int).cast(),
        )
    };
    assert_eq!(rc, ffi::SQLITE_OK);
    conn.execute_batch(SCHEMA).unwrap();
    conn.execute_batch(extra).unwrap();
    drop(conn);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes[20], 80, "the fixture reserves SQLCipher's bytes");
    bytes
}

/// A made-up raw key (64 hex digits from a counter).
fn key_text(seed: u8) -> String {
    let hex: String = (0..32u8)
        .map(|i| format!("{:02x}", i.wrapping_mul(13) ^ seed))
        .collect();
    format!("x'{hex}'")
}

struct Fixture {
    writer: Writer,
    guard: WriteGuard,
    session: Arc<Session>,
    /// The synthetic `master.db`, outside app data.
    db: PathBuf,
    app_data: PathBuf,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture::with_rows("")
    }

    /// A fixture whose database also holds the rows `extra` inserts.
    fn with_rows(extra: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let app_data = base.join("data");
        let guard = WriteGuard::app_data(&app_data).unwrap();
        let writer = Writer::open(
            &guard
                .check(&app_data.join(crate::db::DB_FILE_NAME))
                .unwrap(),
        )
        .unwrap();
        let library = base.join("library"); // ci-reads: not a frontend read
        fs::create_dir(&library).unwrap();
        let key = DbKey::from_text(key_text(1)).unwrap();
        let encrypted =
            cipher::inverse::encrypt(&key, b"synthetic salt!!", &plain_library_with(&base, extra));
        let db = library.join("master.db");
        fs::write(&db, encrypted).unwrap();
        Fixture {
            writer,
            guard,
            session: Arc::default(),
            db,
            app_data,
            _dir: dir,
        }
    }

    fn turn_on(&self) {
        self.writer
            .call(|c| crate::settings::write_feature_enabled(c, Feature::RekordboxDatabase, true))
            .unwrap();
        self.session.set_key(DbKey::from_text(key_text(1)));
    }

    fn reader(&self) -> XmlReader {
        let guard = self.guard.clone();
        XmlReader::default().with_database(Arc::new(DatabaseReader {
            guard: Box::new(move || Some(guard.clone())),
            session: self.session.clone(),
        }))
    }

    /// Runs `job` with the reader and waits for it.
    fn run(&self, job: NewJob) -> (JobStatus, Option<String>) {
        let queue = JobQueue::builder(self.writer.clone())
            .workers(1)
            .handler(JobKind::ReadRekordbox, self.reader())
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
                return (job.status, job.error);
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "the read never finished"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn stored(&self) -> source::XmlSource {
        self.writer.call(|c| stored_source(c)).unwrap()
    }

    fn snapshot(&self) -> Vec<(i64, String)> {
        self.writer
            .call(|c| {
                c.prepare("SELECT track_id, location_key FROM rekordbox_track ORDER BY track_id")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .unwrap()
    }

    fn staging_is_gone(&self) -> bool {
        !self
            .app_data
            .join(crate::write_guard::STAGING_DIR)
            .join(STAGING)
            .exists()
    }

    fn read_pool(&self) -> ReadPool {
        ReadPool::open(self.writer.guarded_path()).unwrap()
    }
}

/// An XML export holding one track, as the XML read gets it.
fn export(dir: &Path) -> PathBuf {
    let path = dir.join("export.xml");
    fs::write(
        &path,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0"><PRODUCT Name="rekordbox" Version="7" Company="x"/>
<COLLECTION Entries="1"><TRACK TrackID="7" Name="From XML" Location="file://localhost/C:/Kit/xml.mp3"/></COLLECTION>
<PLAYLISTS><NODE Type="0" Name="ROOT" Count="0"/></PLAYLISTS></DJ_PLAYLISTS>"#,
    )
    .unwrap();
    path
}

#[test]
fn a_read_stores_the_library_like_an_xml_read_and_marks_it_as_the_database() {
    let fx = Fixture::new();
    fx.turn_on();
    let before = fs::read(&fx.db).unwrap();
    // A -wal with no valid header holds nothing; a -shm is never opened.
    fs::write(
        PathBuf::from(format!("{}-wal", fx.db.display())),
        b"no frames",
    )
    .unwrap();
    fs::write(PathBuf::from(format!("{}-shm", fx.db.display())), b"index").unwrap();

    let (status, error) = fx.run(read_job(&fx.db));
    assert_eq!(status, JobStatus::Done, "{error:?}");
    let ids: Vec<i64> = fx.snapshot().into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, [101, 102], "the deleted track is left out");
    let stored = fx.stored();
    let last = stored.last_read.unwrap();
    assert_eq!(last.source, ReadSource::Database);
    assert_eq!(last.path, fx.db.to_string_lossy());
    assert_eq!(last.summary.tracks, 2);
    assert_eq!(stored.path, None, "the chosen XML export is unchanged");
    assert!(fx.staging_is_gone());
    assert_eq!(
        fs::read(&fx.db).unwrap(),
        before,
        "rekordbox's file is only read"
    );
    assert_eq!(fx.session.lock().failure, None);
}

#[test]
fn the_library_reads_as_an_export_with_the_same_values_and_playlists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.db");
    fs::write(&path, plain_library(dir.path())).unwrap();
    let conn = Connection::open(&path).unwrap();
    let xml = library::read(&conn).unwrap();
    assert!(xml.is_complete());
    let one = &xml.tracks[0];
    for (name, value) in [
        ("Name", "Synthetic One"),
        ("Artist", "Synthetic Artist"),
        ("Album", "Test Album"),
        ("Genre", "House"),
        ("Kind", "MP3 File"),
        ("AverageBpm", "128.00"),
        ("DateAdded", "2025-01-02"),
        ("Rating", "153"),
        ("Remixer", "Made-up Remixer"),
        ("Tonality", "8A"),
        ("Label", "No Label"),
        ("Mix", "Extended Mix"),
        ("Colour", "0xFF00FF"),
        ("Location", "file://localhost/C:/Kit/One%20%26%20Two.mp3"),
    ] {
        assert_eq!(one.attrs.get(name), Some(value), "{name}");
    }
    assert_eq!(one.average_bpm, Some(128.0));
    assert_eq!(xml.tracks[1].attrs.get("AverageBpm"), Some("124.50"));
    assert_eq!(xml.tracks[1].album(), "");
    let folders = &xml.playlists.children;
    assert_eq!(
        folders.len(),
        1,
        "the intelligent and the removed playlists are left out"
    );
    let crate::rekordbox::Node::Folder(folder) = &folders[0] else {
        panic!("not a folder: {folders:?}");
    };
    assert_eq!(folder.name, "Folder");
    let crate::rekordbox::Node::Playlist(list) = &folder.children[0] else {
        panic!("not a playlist");
    };
    let keys: Vec<&str> = list.entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, ["102", "101"]);
}

#[test]
fn a_database_read_keeps_each_playlists_order_and_an_app_playlist_copied_from_it_too() {
    let fx = Fixture::new();
    fx.turn_on();
    let (status, error) = fx.run(read_job(&fx.db));
    assert_eq!(status, JobStatus::Done, "{error:?}");
    // In the database's order (TrackNo), not by id; the removed entry is out.
    let stored: Vec<(String, i64, Option<i64>)> = fx
        .writer
        .call(|c| {
            c.prepare("SELECT path, position, track_id FROM rekordbox_playlist_entry ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        })
        .unwrap();
    let path = r#"["Folder","Peak time"]"#.to_owned();
    assert_eq!(stored, [(path.clone(), 0, Some(102)), (path, 1, Some(101))]);

    // Both tracks are Library tracks the relink matched: the copy keeps
    // that order.
    fx.writer
        .call(|c| {
            c.execute_batch(
                "INSERT INTO volume (id, identity, kind, last_mount_path)
                     VALUES (1, 'serial=1A2B3C4D', 'external', 'C:\\');
                 INSERT INTO music_folder (id, volume_id, rel_path, rel_path_key)
                     VALUES (1, 1, 'Kit', 'Kit');
                 INSERT INTO file (id, music_folder_id, rel_path, rel_path_key, present)
                     VALUES (1, 1, 'one.mp3', 'one.mp3', 1), (2, 1, 'two.flac', 'two.flac', 1);
                 INSERT INTO recording (id, title) VALUES (1, 'One'), (2, 'Two');
                 INSERT INTO recording_file (recording_id, file_id, role)
                     VALUES (1, 1, 'best'), (2, 2, 'best');
                 INSERT INTO library_track (id, recording_id, source_status)
                     VALUES (1, 1, 'missing'), (2, 2, 'missing');
                 UPDATE rekordbox_track SET file_id = 1, relink_method = 'path'
                     WHERE track_id = 101;
                 UPDATE rekordbox_track SET file_id = 2, relink_method = 'path'
                     WHERE track_id = 102;",
            )
        })
        .unwrap();
    let copied = fx
        .writer
        .call(|c| {
            let path = ["Folder".to_owned(), "Peak time".to_owned()];
            Ok(crate::playlists::import::copy_on(c, &path, "Peak time"))
        })
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!((copied.added, copied.not_in_library), (2, 0));
    let order: Vec<i64> = fx
        .writer
        .call(|c| {
            c.prepare("SELECT library_track_id FROM playlist_entry ORDER BY position")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    assert_eq!(order, [2, 1]);
}

#[test]
fn a_wrong_key_a_damaged_file_and_a_missing_one_each_fail_clearly_and_change_nothing() {
    let fx = Fixture::new();
    fx.turn_on();
    let xml = export(fx.db.parent().unwrap());
    let (status, _) = fx.run(source::read_job(&xml));
    assert_eq!(status, JobStatus::Done);
    let snapshot = fx.snapshot();
    let read = fx.stored().last_read;

    let mut cases = vec![];
    fx.session.set_key(DbKey::from_text(key_text(9)));
    cases.push((
        fx.run(read_job(&fx.db)),
        fx.session.lock().failure,
        DbFailure::WrongKey,
    ));
    fx.session.set_key(DbKey::from_text(key_text(1)));
    let mut bytes = fs::read(&fx.db).unwrap();
    let last = bytes.len() - 200;
    bytes[last] ^= 1;
    let damaged = fx.db.with_file_name("damaged.db");
    fs::write(&damaged, bytes).unwrap();
    cases.push((
        fx.run(read_job(&damaged)),
        fx.session.lock().failure,
        DbFailure::Damaged,
    ));
    let missing = fx.db.with_file_name("none.db");
    cases.push((
        fx.run(read_job(&missing)),
        fx.session.lock().failure,
        DbFailure::NotFound,
    ));
    for ((status, error), recorded, reason) in cases {
        assert_eq!(status, JobStatus::Failed);
        assert!(error.unwrap().starts_with(reason.says()));
        assert_eq!(recorded, Some(reason));
        assert_eq!(fx.snapshot(), snapshot);
        assert_eq!(fx.stored().last_read, read);
        assert_eq!(fx.stored().path, Some(xml.to_string_lossy().into_owned()));
        assert_eq!(
            fx.stored().last_failure,
            None,
            "the XML source's failure is untouched"
        );
        assert!(fx.staging_is_gone());
    }
    assert_eq!(fx.session.lock().failures, 3);
}

#[test]
fn turned_off_a_database_job_runs_nothing_and_the_xml_read_is_unchanged() {
    let fx = Fixture::new();
    // The key alone doesn't turn the feature on.
    fx.session.set_key(DbKey::from_text(key_text(1)));
    let (status, error) = fx.run(read_job(&fx.db));
    assert_eq!(status, JobStatus::Failed);
    assert!(error.unwrap().starts_with(DbFailure::TurnedOff.says()));
    assert!(
        !fx.app_data.join(crate::write_guard::STAGING_DIR).exists(),
        "nothing was staged"
    );
    assert_eq!(fx.stored().last_read, None);
    assert!(fx.snapshot().is_empty());

    // The read job with the database reader joined reads XML as before.
    let xml = export(fx.db.parent().unwrap());
    let (status, _) = fx.run(source::read_job(&xml));
    assert_eq!(status, JobStatus::Done);
    let stored = fx.stored();
    assert_eq!(stored.last_read.unwrap().source, ReadSource::Xml);
    assert_eq!(stored.path, Some(xml.to_string_lossy().into_owned()));
    assert_eq!(fx.snapshot(), [(7, fx.snapshot()[0].1.clone())]);
}

#[test]
fn turned_off_the_commands_show_nothing_keep_no_key_and_start_no_read() {
    let fx = Fixture::new();
    let reads = fx.read_pool();
    let session = Session::default();
    let folder = tempfile::tempdir().unwrap();
    fs::create_dir_all(folder.path().join("Pioneer").join("rekordbox")).unwrap(); // ci-reads: not a frontend read
    assert_eq!(
        status(&reads, &session, Some(folder.path())).unwrap(),
        RekordboxDatabase::default()
    );
    assert!(!set_key(&reads, &session, key_text(1)).unwrap());
    assert!(session.key().is_none());
    let queue = JobQueue::builder(fx.writer.clone())
        .workers(1)
        .start()
        .unwrap();
    assert_eq!(
        start_read(&reads, &queue, &session, &fx.db).unwrap(),
        DatabaseReadStart::Refused {
            reason: DbFailure::TurnedOff
        }
    );
    let queued: i64 = fx
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM job", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(queued, 0);

    // On: a key is needed first, then a file that's there.
    fx.writer
        .call(|c| crate::settings::write_feature_enabled(c, Feature::RekordboxDatabase, true))
        .unwrap();
    let refused = |path: &Path| start_read(&reads, &queue, &session, path).unwrap();
    assert_eq!(
        refused(&fx.db),
        DatabaseReadStart::Refused {
            reason: DbFailure::NoKey
        }
    );
    assert!(set_key(&reads, &session, key_text(1)).unwrap());
    let missing = fx.db.with_file_name("none.db");
    assert_eq!(
        refused(&missing),
        DatabaseReadStart::Refused {
            reason: DbFailure::NotFound
        }
    );
    let shown = status(&reads, &session, Some(folder.path())).unwrap();
    assert!(shown.key_set);
    assert_eq!(
        shown.usual_folder,
        Some(
            folder
                .path()
                .join("Pioneer")
                .join("rekordbox") // ci-reads: not a frontend read
                .to_string_lossy()
                .into_owned()
        )
    );
    assert!(
        !set_key(&reads, &session, String::new()).unwrap(),
        "an empty key forgets it"
    );
    queue.shutdown();
}

struct NoVolumes;

impl Volumes for NoVolumes {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up",
            path.display()
        )))
    }

    fn mount_path(&self, _id: &VolumeId) -> Option<PathBuf> {
        None
    }
}

#[test]
fn a_database_read_opens_the_send_review_as_an_xml_read_does() {
    let fx = Fixture::new();
    fx.turn_on();
    let review = || {
        fx.writer
            .call(|c| crate::send::review(c, &NoVolumes).map(|r| r.is_some()))
            .unwrap()
    };
    assert!(!review(), "never read: no review");
    let (status, _) = fx.run(read_job(&fx.db));
    assert_eq!(status, JobStatus::Done);
    assert!(review());
}

#[test]
fn the_key_is_never_shown_and_a_raw_key_is_read_as_one() {
    let key = DbKey::from_text("made-up words".into()).unwrap();
    assert_eq!(format!("{key:?}"), "DbKey(redacted)");
    assert_eq!(key.raw(), None);
    assert!(DbKey::from_text(String::new()).is_none());
    let raw = DbKey::from_text(key_text(0)).unwrap();
    assert_eq!(raw.raw().unwrap()[1], 13);
    assert!(!format!("{:?}", Some(&raw)).contains("x'"));
    assert_eq!(DbKey::from_text("x'12'".into()).unwrap().raw(), None);
}

#[test]
fn a_user_with_only_the_database_is_told_a_send_needs_an_export_and_a_chosen_export_still_sends() {
    let fx = Fixture::new();
    fx.turn_on();
    let (status, _) = fx.run(read_job(&fx.db));
    assert_eq!(status, JobStatus::Done);
    let refused = crate::send::export_to_send(&fx.stored(), None).unwrap_err();
    assert_eq!(refused.kind(), crate::ipc::ErrorKind::SendNeedsExport);
    let chosen = export(fx._dir.path());
    let chosen = chosen.to_string_lossy().into_owned();
    assert_eq!(
        crate::send::export_to_send(&fx.stored(), Some(chosen.clone())).unwrap(),
        chosen
    );
}

#[test]
fn a_newer_database_read_refuses_a_send_even_with_an_older_export_path_stored() {
    let fx = Fixture::new();
    fx.turn_on();
    let xml = export(fx.db.parent().unwrap());
    assert_eq!(fx.run(source::read_job(&xml)).0, JobStatus::Done);
    assert_eq!(fx.run(read_job(&fx.db)).0, JobStatus::Done);
    let from_database = fx.snapshot();
    let stored = fx.stored();
    assert_eq!(stored.path.as_deref(), Some(&*xml.to_string_lossy()));

    let refused = crate::send::export_to_send(&stored, None).unwrap_err();
    assert_eq!(refused.kind(), crate::ipc::ErrorKind::SendNeedsExport);
    // The pure choice changes nothing; the refused prepare_send leaving the
    // snapshot and queue alone is proved in send/tests.rs.
    assert_eq!(fx.snapshot(), from_database);
    let last = fx.stored().last_read.unwrap();
    assert_eq!(last.source, source::ReadSource::Database);
}

#[test]
fn a_newer_export_read_after_a_database_read_sends_from_that_export_as_before() {
    let fx = Fixture::new();
    fx.turn_on();
    assert_eq!(fx.run(read_job(&fx.db)).0, JobStatus::Done);
    let xml = export(fx.db.parent().unwrap());
    let xml = xml.to_string_lossy().into_owned();
    // A path chosen in the checklist is an export, read as one.
    assert_eq!(
        crate::send::export_to_send(&fx.stored(), Some(xml.clone())).unwrap(),
        xml
    );
    assert_eq!(fx.run(source::read_job(Path::new(&xml))).0, JobStatus::Done);
    assert_eq!(
        fx.stored().last_read.unwrap().source,
        source::ReadSource::Xml
    );
    assert_eq!(
        crate::send::export_to_send(&fx.stored(), None).unwrap(),
        xml
    );
}

#[test]
fn one_bad_row_is_left_out_and_counted_and_the_rest_is_still_read() {
    // A track with no id and no file, one with text where numbers go and an
    // unknown file type, an entry for a track that isn't there, and a
    // playlist that is its own parent.
    let fx = Fixture::with_rows(
        "
INSERT INTO djmdContent (ID, FolderPath, Title, FileType, BPM, Length)
  VALUES ('104', 'C:/Kit/odd.mp3', 'Odd Values', 99, 'fast', 'long'),
         (NULL, NULL, 'No Id, No Path', 1, 12000, 200);
INSERT INTO djmdPlaylist VALUES ('14', 3, 'Loop', 0, '14', 0);
INSERT INTO djmdSongPlaylist VALUES ('4', '11', '999', 4, 0), ('5', NULL, '101', 5, 0);
",
    );
    fx.turn_on();
    let (status, error) = fx.run(read_job(&fx.db));
    assert_eq!(status, JobStatus::Done, "{error:?}");
    let ids: Vec<i64> = fx.snapshot().into_iter().map(|(id, _)| id).collect();
    assert_eq!(
        ids,
        [101, 102, 104],
        "the good rows and the odd-valued one are read; the row with no id is not"
    );
    let last = fx.stored().last_read.unwrap();
    assert_eq!(last.source, ReadSource::Database);
    assert_eq!(last.summary.tracks, 3);
    assert_eq!(last.summary.not_stored, 1, "the bad row is counted");
    assert!(fx.staging_is_gone());
}
