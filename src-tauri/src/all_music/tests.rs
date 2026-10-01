//! All music list tests. Everything is synthetic: a made-up volume, folder
//! and file names in a migrated database in a temp dir.

use std::path::{Path, PathBuf};

use super::*;
use crate::db::Writer;
use crate::volume::VolumeId;
use crate::volume::{identity, IdentitySignals, Volume, VolumeKind};

/// The made-up volume every test file is on.
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

/// The test volume, mounted at `0` or (if `None`) unplugged.
struct Mount(Option<PathBuf>);

impl Volumes for Mount {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up in these tests",
            path.display()
        )))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        self.0.clone().filter(|_| *id == volume_id())
    }
}

fn mounted() -> Mount {
    Mount(Some(PathBuf::from(r"E:\")))
}

/// A database with one volume (last mounted at `E:\`) and one music
/// folder, `Music` (id 1).
struct Lib {
    _dir: tempfile::TempDir,
    writer: Writer,
}

impl Lib {
    fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
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
        Lib { _dir: dir, writer }
    }

    fn insert(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// A track with one best file on disk.
    fn track(&self, title: Option<&str>, artist: Option<&str>, file: &str) -> i64 {
        let track = self.insert(
            "INSERT INTO recording (title, artist) VALUES (?1, ?2)",
            (title.map(str::to_owned), artist.map(str::to_owned)),
        );
        self.file(track, file, "best", true);
        track
    }

    fn file(&self, track: i64, name: &str, role: &str, present: bool) -> i64 {
        let file = self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
             VALUES (1, ?1, ?1, ?2)",
            (name.to_owned(), present),
        );
        self.insert(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, ?3)",
            (track, file, role.to_owned()),
        );
        file
    }

    fn list_with(&self, search: &'static str, volumes: &Mount) -> AllMusicList {
        let (total, tracks) = self.writer.call(move |c| stored(c, search)).unwrap();
        list(total, &tracks, volumes)
    }

    fn list(&self, search: &'static str) -> AllMusicList {
        self.list_with(search, &mounted())
    }

    fn titles(&self, search: &'static str) -> Vec<String> {
        self.list(search)
            .tracks
            .into_iter()
            .map(|t| t.title.unwrap_or_default())
            .collect()
    }

    fn library_tracks(&self) -> i64 {
        self.writer
            .call(|c| c.query_row("SELECT count(*) FROM library_track", [], |r| r.get(0)))
            .unwrap()
    }
}

#[test]
fn with_no_tracks_the_list_is_empty() {
    let lib = Lib::new();
    assert_eq!(
        lib.list(""),
        AllMusicList {
            total: 0,
            tracks: vec![]
        }
    );
}

#[test]
fn each_track_is_listed_with_its_title_artist_and_file() {
    let lib = Lib::new();
    let track = lib.track(Some("Tune"), Some("Someone"), "Sub/tune.mp3");

    let list = lib.list("");
    assert_eq!(list.total, 1);
    assert_eq!(
        list.tracks,
        vec![AllMusicTrack {
            recording_id: track,
            title: Some("Tune".to_owned()),
            artist: Some("Someone".to_owned()),
            file: Some(LinkedFile {
                path: r"E:\Music\Sub\tune.mp3".to_owned(),
                name: "tune.mp3".to_owned(),
                present: true,
                drive_connected: true,
            }),
            in_library: false,
        }]
    );
}

#[test]
fn a_track_with_no_title_is_listed_without_one_and_sorted_by_its_files_name() {
    let lib = Lib::new();
    lib.track(Some("  "), None, "Sub/b2.mp3");
    lib.track(Some("beta"), None, "x.mp3");
    lib.track(Some("Alpha"), None, "y.mp3");

    let list = lib.list("");
    let titles: Vec<_> = list.tracks.iter().map(|t| t.title.as_deref()).collect();
    assert_eq!(titles, vec![Some("Alpha"), None, Some("beta")]);
    assert_eq!(list.tracks[1].file.as_ref().unwrap().name, "b2.mp3");
}

#[test]
fn the_order_is_the_library_lists_title_then_artist_ignoring_case_and_accents() {
    let lib = Lib::new();
    lib.track(Some("Zeta"), None, "1.mp3");
    lib.track(Some("\u{c9}clat"), Some("B"), "2.mp3");
    lib.track(Some("eclat"), Some("a"), "3.mp3");
    lib.track(Some("Echo"), None, "4.mp3");

    assert_eq!(lib.titles(""), vec!["Echo", "eclat", "\u{c9}clat", "Zeta"]);
    // The same key the Library list sorts by.
    let mut by_key = lib.titles("");
    by_key.sort_by_key(|title| library::sort_key(title));
    assert_eq!(by_key, lib.titles(""));
}

#[test]
fn a_search_ignores_case_and_accents_outside_ascii_too() {
    let lib = Lib::new();
    lib.track(Some("\u{c9}dith"), Some("Bj\u{f6}rk"), "a.mp3");
    lib.track(Some("Other"), None, "\u{c0} la carte.mp3");
    lib.track(Some("Plain"), None, "b.mp3");

    for search in ["\u{e9}dith", "EDITH", "\u{c9}DITH", "bjork", "BJ\u{d6}RK"] {
        assert_eq!(lib.titles(search), vec!["\u{c9}dith"], "{search}");
    }
    assert_eq!(lib.titles("a la carte"), vec!["Other"]);
}

#[test]
fn a_file_on_an_unplugged_drive_is_shown_where_the_drive_was_last_mounted() {
    let lib = Lib::new();
    lib.track(Some("Tune"), None, "tune.mp3");

    let list = lib.list_with("", &Mount(None));
    assert_eq!(
        list.tracks[0].file.as_ref().unwrap().path,
        r"E:\Music\tune.mp3"
    );
}

#[test]
fn a_track_shows_its_best_file_not_an_extra_one() {
    let lib = Lib::new();
    let track = lib.insert("INSERT INTO recording (title) VALUES ('Tune')", []);
    lib.file(track, "extra.mp3", "extra", true);
    lib.file(track, "best.flac", "best", true);

    assert_eq!(
        lib.list("").tracks[0].file.as_ref().unwrap().name,
        "best.flac"
    );
}

#[test]
fn a_search_matches_title_artist_or_file_path_ignoring_case() {
    let lib = Lib::new();
    lib.track(Some("Night Drive"), Some("Someone"), "a.mp3");
    lib.track(Some("Morning"), Some("The Night Owls"), "b.mp3");
    lib.track(Some("Noon"), Some("Other"), "Nightly/c.mp3");
    lib.track(Some("Dawn"), Some("Other"), "d.mp3");

    assert_eq!(lib.titles("night"), vec!["Morning", "Night Drive", "Noon"]);
    assert_eq!(lib.list("night").total, 3);
    assert_eq!(lib.titles("  dawn "), vec!["Dawn"]);
    assert_eq!(lib.titles("nothing like it"), Vec::<String>::new());
}

#[test]
fn a_search_takes_percent_and_underscore_literally() {
    let lib = Lib::new();
    lib.track(Some("100% Pure"), None, "a.mp3");
    lib.track(Some("Plain"), None, "b_side.mp3");

    assert_eq!(lib.titles("%"), vec!["100% Pure"]);
    assert_eq!(lib.titles("_"), vec!["Plain"]);
}

#[test]
fn the_list_stops_at_the_limit_and_still_says_how_many_match() {
    let lib = Lib::new();
    lib.writer
        .call(|c| {
            for n in 0..(LIST_LIMIT + 5) {
                c.execute(
                    "INSERT INTO recording (title) VALUES (?1)",
                    [format!("t{n:04}")],
                )?;
                let track = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key)
                     VALUES (1, ?1, ?1)",
                    [format!("{n}.mp3")],
                )?;
                c.execute(
                    "INSERT INTO recording_file (recording_id, file_id, role)
                     VALUES (?1, ?2, 'best')",
                    [track, c.last_insert_rowid()],
                )?;
            }
            Ok(())
        })
        .unwrap();

    let list = lib.list("");
    assert_eq!(list.total as usize, LIST_LIMIT + 5);
    assert_eq!(list.tracks.len(), LIST_LIMIT);
    // The first ones in order, not just any.
    assert_eq!(list.tracks[0].title.as_deref(), Some("t0000"));
    assert_eq!(list.tracks[LIST_LIMIT - 1].title.as_deref(), Some("t0199"));
}

// Start fresh: the Library is empty until the user adds a track here.

#[test]
fn starting_fresh_the_library_is_empty_and_adding_one_track_from_the_list_puts_it_there() {
    let lib = Lib::new();
    let track = lib.track(Some("Tune"), None, "tune.mp3");
    lib.track(Some("Other"), None, "other.mp3");
    assert_eq!(lib.library_tracks(), 0);
    assert!(lib.list("").tracks.iter().all(|t| !t.in_library));

    let added = library::promote(&lib.writer, &mounted(), track).unwrap();
    assert!(added.added);
    assert_eq!(lib.library_tracks(), 1);
    let in_library: Vec<_> = lib
        .list("")
        .tracks
        .into_iter()
        .map(|t| (t.title.unwrap(), t.in_library))
        .collect();
    assert_eq!(
        in_library,
        vec![("Other".to_owned(), false), ("Tune".to_owned(), true)]
    );
}

#[test]
fn a_track_taken_out_of_the_library_is_listed_as_not_in_it_and_can_be_added_back() {
    let lib = Lib::new();
    let track = lib.track(Some("Tune"), None, "tune.mp3");
    library::promote(&lib.writer, &mounted(), track).unwrap();
    // Taken out again (here, by undoing the add).
    crate::ops::undo_last_via(&lib.writer).unwrap();
    assert!(!lib.list("").tracks[0].in_library);

    let again = library::promote(&lib.writer, &mounted(), track).unwrap();
    assert!(again.added);
    assert!(lib.list("").tracks[0].in_library);
}

#[test]
fn the_frontend_gets_the_list_over_ipc() {
    use crate::ipc::testing::{app, invoke};
    use serde_json::json;
    use tauri::Manager;

    let (_data, app) = app();
    let identity = volume_id().as_str().to_owned();
    app.state::<Writer>()
        .call(move |c| {
            c.execute(
                "INSERT INTO volume (identity, kind, last_mount_path)
                 VALUES (?1, 'external', 'E:\\')",
                [identity],
            )?;
            c.execute_batch(
                "INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                 VALUES (1, 'Music', 'Music');
                 INSERT INTO file (music_folder_id, rel_path, rel_path_key)
                 VALUES (1, 'a.mp3', 'a.mp3');
                 INSERT INTO recording (title, artist) VALUES ('Tune', 'Someone');
                 INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 1, 'best');",
            )
        })
        .unwrap();

    let list = invoke(&app, "all_music_tracks", json!({ "search": null })).unwrap();
    assert_eq!(list["total"], 1);
    assert_eq!(list["tracks"][0]["recordingId"], 1);
    assert_eq!(list["tracks"][0]["title"], "Tune");
    assert_eq!(list["tracks"][0]["inLibrary"], false);
    assert_eq!(list["tracks"][0]["file"]["name"], "a.mp3");
    let none = invoke(&app, "all_music_tracks", json!({ "search": "zzz" })).unwrap();
    assert_eq!(none, json!({ "total": 0, "tracks": [] }));
}

#[test]
fn a_track_with_no_files_is_not_in_all_music() {
    let lib = Lib::new();
    lib.insert(
        "INSERT INTO recording (title, artist) VALUES ('Kept', 'Someone')",
        [],
    );
    lib.track(Some("Tune"), None, "tune.mp3");

    assert_eq!(lib.titles(""), vec!["Tune"]);
    assert_eq!(lib.list("").total, 1);
    assert_eq!(lib.titles("kept"), Vec::<String>::new());
    assert_eq!(lib.list("someone").total, 0);
}

#[test]
fn a_track_removed_from_the_library_can_be_added_back_which_clears_its_removal_record() {
    let lib = Lib::new();
    let track = lib.track(Some("Tune"), None, "tune.mp3");
    let added = library::promote(&lib.writer, &mounted(), track).unwrap();
    library::remove(&lib.writer, added.library_track.id).unwrap();
    assert!(!lib.list("").tracks[0].in_library);
    let removals = |lib: &Lib| -> i64 {
        lib.writer
            .call(|c| c.query_row("SELECT count(*) FROM library_removal", [], |r| r.get(0)))
            .unwrap()
    };
    assert_eq!(removals(&lib), 1);

    let again = library::promote(&lib.writer, &mounted(), track).unwrap();
    assert!(again.added);
    assert!(lib.list("").tracks[0].in_library);
    assert_eq!(removals(&lib), 0);
}

#[test]
fn a_search_finds_a_track_that_sorts_beyond_the_limit() {
    let lib = Lib::new();
    lib.writer
        .call(|c| {
            for n in 0..(LIST_LIMIT + 5) {
                c.execute(
                    "INSERT INTO recording (title) VALUES (?1)",
                    [format!("a{n:04}")],
                )?;
                let track = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key)
                     VALUES (1, ?1, ?1)",
                    [format!("{n}.mp3")],
                )?;
                c.execute(
                    "INSERT INTO recording_file (recording_id, file_id, role)
                     VALUES (?1, ?2, 'best')",
                    [track, c.last_insert_rowid()],
                )?;
            }
            Ok(())
        })
        .unwrap();
    // Sorts after every other track, so a blank search doesn't show it.
    lib.track(Some("Zzz Needle"), None, "last.mp3");
    assert!(!lib.titles("").contains(&"Zzz Needle".to_owned()));

    let found = lib.list("needle");
    assert_eq!(found.total, 1);
    assert_eq!(found.tracks[0].title.as_deref(), Some("Zzz Needle"));
    // A search wider than the limit is cut after matching, not before.
    let wide = lib.list("mp3");
    assert_eq!(wide.total as usize, LIST_LIMIT + 6);
    assert_eq!(wide.tracks.len(), LIST_LIMIT);
}
