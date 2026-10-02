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
        let mount = Mount(volumes.0.clone());
        let (total, tracks) = self
            .writer
            .call(move |c| stored(c, &mount, search))
            .unwrap();
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
            match_not_confirmed: false,
        }]
    );
}

#[test]
fn a_track_whose_only_rekordbox_match_is_probable_is_marked_until_it_is_trusted() {
    let lib = Lib::new();
    lib.track(Some("Plain"), None, "plain.mp3");
    let held = lib.track(Some("Renamed"), None, "renamed.mp3");
    let file: i64 = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT file_id FROM recording_file WHERE recording_id = ?1",
                [held],
                |r| r.get(0),
            )
        })
        .unwrap();
    let entry = lib.insert(
        "INSERT INTO rekordbox_track
             (attributes, location_key, read_at, file_id, relink_method, relink_probable)
         VALUES ('{\"TrackID\":\"7\",\"Location\":\"file://localhost/E:/Music/old.mp3\"}',
                 'E:/Music/old.mp3', '2026-09-30T10:00:00.000Z', ?1, 'filename_only', 1)",
        (file,),
    );
    let marked = |lib: &Lib| -> Vec<(Option<String>, bool)> {
        lib.list("")
            .tracks
            .into_iter()
            .map(|t| (t.title, t.match_not_confirmed))
            .collect()
    };
    assert_eq!(
        marked(&lib),
        [
            (Some("Plain".to_owned()), false),
            (Some("Renamed".to_owned()), true)
        ]
    );

    // Once the match is trusted, the track is added like any other.
    lib.insert(
        "UPDATE rekordbox_track SET relink_probable = 0, relink_method = 'user' WHERE id = ?1",
        (entry,),
    );
    assert!(marked(&lib).iter().all(|(_, held)| !held));
}

#[test]
fn a_track_with_no_title_is_listed_under_its_files_name_and_sorted_by_it() {
    let lib = Lib::new();
    lib.track(Some("  "), None, "Sub/b2.mp3");
    lib.track(Some("beta"), None, "x.mp3");
    lib.track(Some("Alpha"), None, "y.mp3");

    let list = lib.list("");
    let titles: Vec<_> = list.tracks.iter().map(|t| t.title.as_deref()).collect();
    assert_eq!(titles, vec![Some("Alpha"), Some("b2.mp3"), Some("beta")]);
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

// ---- the title and artist shown (1aG-9) --------------------------------------

impl Lib {
    /// Gives `file` ID3v2 title and artist tags.
    fn tag(&self, file: i64, title: &str, artist: Option<&str>) {
        let mut items = vec![serde_json::json!(
            {"key": "TIT2", "value": {"type": "text", "text": title}}
        )];
        if let Some(artist) = artist {
            items.push(serde_json::json!(
                {"key": "TPE1", "value": {"type": "text", "text": artist}}
            ));
        }
        let raw = serde_json::json!({ "id3v2": items }).to_string();
        self.insert("UPDATE file SET raw_tags = ?1 WHERE id = ?2", (raw, file));
    }

    /// A Library track for `track`, linked to its best file, which
    /// rekordbox already has under `name` and `artist`.
    fn library_known(&self, track: i64, file: i64, name: &str, artist: &str) {
        self.insert(
            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (?1, 'linked', ?2, 'ok')",
            (track, file),
        );
        let attributes = serde_json::json!({
            "TrackID": "7", "Name": name, "Artist": artist,
            "Location": "file://localhost/E:/Music/known.mp3",
        })
        .to_string();
        self.insert(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable)
             VALUES (?1, 'E:/MUSIC/KNOWN.MP3', '2026-09-30T10:00:00.000Z', ?2, 'path', 0)",
            (attributes, file),
        );
    }
}

#[test]
fn all_music_rows_show_the_title_a_send_would_write_a_new_tracks_from_its_tags() {
    let lib = Lib::new();
    let track = lib.track(None, None, "x.mp3");
    let file = lib
        .writer
        .call(|c| c.query_row("SELECT id FROM file", [], |r| r.get(0)))
        .unwrap();
    lib.tag(file, "Tagged title", Some("Tagged artist"));

    let shown = lib.list("").tracks;
    assert_eq!(shown[0].recording_id, track);
    assert_eq!(shown[0].title.as_deref(), Some("Tagged title"));
    assert_eq!(shown[0].artist.as_deref(), Some("Tagged artist"));
}

#[test]
fn a_track_with_neither_tags_nor_title_shows_its_file_name_and_no_artist() {
    let lib = Lib::new();
    lib.track(Some("  "), None, "Sub/b2.mp3");

    let shown = lib.list("").tracks;
    assert_eq!(shown[0].title.as_deref(), Some("b2.mp3"));
    assert_eq!(shown[0].artist, None);
}

#[test]
fn all_music_rows_show_the_title_a_send_would_write_a_known_tracks_from_rekordbox_and_a_set_title_wins(
) {
    let lib = Lib::new();
    let known = lib.track(None, None, "known.mp3");
    let file = lib
        .writer
        .call(|c| c.query_row("SELECT id FROM file", [], |r| r.get(0)))
        .unwrap();
    lib.tag(file, "Tag title", Some("Tag artist"));
    lib.library_known(known, file, "Their title", "Their artist");
    lib.track(Some("Own title"), Some("Own artist"), "other.mp3");

    let shown: Vec<_> = lib
        .list("")
        .tracks
        .into_iter()
        .map(|t| (t.title.unwrap(), t.artist.unwrap(), t.in_library))
        .collect();
    assert_eq!(
        shown,
        // In the order of the stored title and the file name ("known.mp3",
        // "Own title"), each row showing what a send would write.
        vec![
            ("Their title".to_owned(), "Their artist".to_owned(), true),
            ("Own title".to_owned(), "Own artist".to_owned(), false),
        ]
    );
}

#[test]
fn all_music_search_and_order_are_unchanged_by_shown_titles() {
    let lib = Lib::new();
    // No stored titles: the shown ones come from a tag and a rekordbox
    // entry, but the order and the search still go by the stored title and
    // the file name, as before titles were shown.
    let tagged = lib.track(None, None, "m.mp3");
    let known = lib.track(None, None, "zebra.mp3");
    let plain = lib.track(None, None, "alpha.mp3");
    let stored = lib.track(Some("Stored"), None, "q.mp3");
    let files: Vec<i64> = lib
        .writer
        .call(|c| {
            c.prepare("SELECT id FROM file ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    lib.tag(files[0], "Aardvark", None);
    lib.library_known(known, files[1], "Aaa Their title", "");

    // The order: file names alpha, m, zebra and the stored title "Stored"
    // (m < q < zebra by the sort key), each row showing its shown title.
    let shown: Vec<_> = lib
        .list("")
        .tracks
        .iter()
        .map(|t| (t.recording_id, t.title.clone().unwrap()))
        .collect();
    assert_eq!(
        shown,
        vec![
            (plain, "alpha.mp3".to_owned()),
            (tagged, "Aardvark".to_owned()),
            (stored, "Stored".to_owned()),
            (known, "Aaa Their title".to_owned()),
        ]
    );
    // The search: a title that is only shown isn't searched; the file name
    // and the stored title still are.
    assert_eq!(lib.list("aardvark").total, 0);
    assert_eq!(lib.list("their title").total, 0);
    assert_eq!(lib.titles("zebra"), vec!["Aaa Their title"]);
    assert_eq!(lib.titles("stored"), vec!["Stored"]);
    assert_eq!(lib.titles("m.mp3"), vec!["Aardvark"]);
}

#[test]
fn listing_all_music_leaves_the_database_unchanged() {
    let lib = Lib::new();
    let track = lib.track(None, None, "known.mp3");
    let file = lib
        .writer
        .call(|c| c.query_row("SELECT id FROM file", [], |r| r.get(0)))
        .unwrap();
    lib.tag(file, "Tag title", Some("Tag artist"));
    lib.library_known(track, file, "Their title", "Their artist");
    lib.track(None, None, "plain.mp3");
    let snapshot = || {
        lib.writer
            .call(|c| {
                let tables: Vec<String> = c
                    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                let mut counts = Vec::new();
                for table in tables {
                    let n: i64 =
                        c.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| {
                            r.get(0)
                        })?;
                    counts.push((table, n));
                }
                let recordings: Vec<(i64, Option<String>, Option<String>)> = c
                    .prepare("SELECT id, title, artist FROM recording ORDER BY id")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                Ok((counts, recordings))
            })
            .unwrap()
    };
    let before = snapshot();
    assert_eq!(lib.list("").tracks.len(), 2);
    assert_eq!(snapshot(), before);
}
