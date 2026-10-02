//! Library track tests. Everything is synthetic: a made-up volume, folders
//! and file names in a migrated database in a temp dir.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::*;
use crate::db::Writer;
use crate::ops::{undo_last_via, UndoOutcome};
use crate::volume::{identity, IdentitySignals, Volume, VolumeKind};

/// No Downloads or temp folder, so a row's reason never depends on the host.
const NO_DIRS: FragileDirs = FragileDirs {
    downloads: None,
    temp: Vec::new(),
};

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

impl Mount {
    fn at(path: &str) -> Mount {
        Mount(Some(PathBuf::from(path)))
    }
}

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

/// A library database with one volume (last mounted at `E:\`) and one
/// music folder, `Music` (id 1).
struct Lib {
    dir: tempfile::TempDir,
    writer: Writer,
    next_track_id: std::cell::Cell<i64>,
}

fn open(dir: &Path) -> Writer {
    Writer::open(&crate::write_guard::test_path(dir, crate::db::DB_FILE_NAME)).unwrap()
}

impl Lib {
    fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = open(dir.path());
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
        Lib {
            dir,
            writer,
            next_track_id: std::cell::Cell::new(1),
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

    /// A track with no files, titled and credited as given.
    fn track(&self, title: Option<&str>, artist: Option<&str>) -> i64 {
        self.insert(
            "INSERT INTO recording (title, artist) VALUES (?1, ?2)",
            (title.map(str::to_owned), artist.map(str::to_owned)),
        )
    }

    /// A file of `track` in the music folder, on disk or not.
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

    /// A rekordbox track matched to `file`, trusted or only probable.
    fn rekordbox_match(&self, file: i64, probable: bool) {
        let track_id = self.next_track_id.get();
        self.next_track_id.set(track_id + 1);
        let attributes = json!({
            "TrackID": track_id.to_string(),
            "Location": format!("file://localhost/E:/Music/{track_id}.mp3"),
        })
        .to_string();
        self.insert(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable)
             VALUES (?1, ?2, '2026-09-30T10:00:00.000Z', ?3, ?4, ?5)",
            (
                attributes,
                format!("E:/Music/{track_id}.mp3"),
                file,
                if probable { "filename_only" } else { "path" },
                probable,
            ),
        );
    }

    fn promote(&self, track: i64) -> Result<Promoted, LibraryError> {
        promote_with(&self.writer, &Mount::at(r"E:\"), &NO_DIRS, track)
    }

    fn list_with(&self, volumes: &Mount) -> Vec<LibraryTrack> {
        let (tracks, located) = self.writer.call(|c| stored_with_locations(c)).unwrap();
        list_with_fragile(&tracks, &located, volumes, &NO_DIRS)
    }

    fn list(&self) -> Vec<LibraryTrack> {
        self.list_with(&Mount::at(r"E:\"))
    }

    fn count(&self, sql: &'static str) -> i64 {
        self.writer
            .call(move |c| c.query_row(sql, [], |r| r.get(0)))
            .unwrap()
    }

    fn library_tracks(&self) -> i64 {
        self.count("SELECT count(*) FROM library_track")
    }

    fn operations(&self) -> i64 {
        self.count("SELECT count(*) FROM operation")
    }

    /// The file `track`'s Library track links to.
    fn linked_file(&self, track: i64) -> Option<i64> {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT linked_file_id FROM library_track WHERE recording_id = ?1",
                    [track],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }
}

mod upkeep;

// Which file a Library track links to.

#[test]
fn a_trusted_rekordbox_match_beats_the_best_file() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let best = lib.file(track, "best.flac", "best", true);
    let rekordbox = lib.file(track, "played.mp3", "undecided", true);
    lib.rekordbox_match(rekordbox, false);

    let choice = lib
        .writer
        .call(move |c| linked_file_for(c, track))
        .unwrap()
        .unwrap();
    assert_eq!(choice.file_id, rekordbox);
    assert_eq!(choice.source, LinkedFileSource::Rekordbox);

    let added = lib.promote(track).unwrap();
    assert!(added.added);
    assert_eq!(lib.linked_file(track), Some(rekordbox));
    assert_ne!(lib.linked_file(track), Some(best));
    assert_eq!(
        added.library_track.file.unwrap().path,
        r"E:\Music\played.mp3"
    );
}

#[test]
fn a_track_whose_only_rekordbox_match_is_probable_cannot_be_added() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let best = lib.file(track, "best.flac", "best", true);
    let probable = lib.file(track, "maybe.mp3", "undecided", true);
    lib.rekordbox_match(probable, true);

    // The probable match is never the file picked...
    let choice = lib
        .writer
        .call(move |c| linked_file_for(c, track))
        .unwrap()
        .unwrap();
    assert_eq!(choice.file_id, best);
    assert_eq!(choice.source, LinkedFileSource::BestFile);

    // ...and linking the best file instead would send the track as new,
    // beside the rekordbox entry that is probably its own: refused.
    assert!(matches!(
        lib.promote(track),
        Err(LibraryError::MatchNotConfirmed)
    ));
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(lib.operations(), 0);
}

#[test]
fn a_probable_match_on_the_tracks_only_file_holds_it_back_too() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let only = lib.file(track, "renamed.mp3", "best", true);
    lib.rekordbox_match(only, true);
    assert!(matches!(
        lib.promote(track),
        Err(LibraryError::MatchNotConfirmed)
    ));
}

#[test]
fn a_track_with_a_trusted_match_is_added_whatever_else_is_only_probable() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let trusted = lib.file(track, "best.flac", "best", true);
    let probable = lib.file(track, "maybe.mp3", "undecided", true);
    lib.rekordbox_match(trusted, false);
    lib.rekordbox_match(probable, true);
    lib.promote(track).unwrap();
    assert_eq!(lib.linked_file(track), Some(trusted));
}

#[test]
fn a_probable_match_to_another_tracks_file_holds_nothing_back() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "mine.mp3", "best", true);
    let other = lib.track(None, None);
    let theirs = lib.file(other, "theirs.mp3", "best", true);
    lib.rekordbox_match(theirs, true);
    lib.promote(track).unwrap();
}

#[test]
fn a_track_rekordbox_does_not_know_links_to_its_best_file() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "extra.mp3", "extra", true);
    let best = lib.file(track, "best.mp3", "best", true);

    lib.promote(track).unwrap();
    assert_eq!(lib.linked_file(track), Some(best));
}

#[test]
fn a_match_to_another_tracks_file_does_not_count_for_this_track() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let best = lib.file(track, "mine.mp3", "best", true);
    let other = lib.track(None, None);
    let others = lib.file(other, "theirs.mp3", "best", true);
    lib.rekordbox_match(others, false);

    lib.promote(track).unwrap();
    assert_eq!(lib.linked_file(track), Some(best));
}

#[test]
fn of_two_trusted_matches_the_file_on_disk_is_linked() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let gone = lib.file(track, "gone.mp3", "best", false);
    let here = lib.file(track, "here.mp3", "undecided", true);
    lib.rekordbox_match(gone, false);
    lib.rekordbox_match(here, false);

    lib.promote(track).unwrap();
    assert_eq!(lib.linked_file(track), Some(here));
}

// Refusals.

#[test]
fn a_track_with_no_trusted_match_and_no_best_file_is_refused() {
    let lib = Lib::new();
    let no_files = lib.track(None, None);
    let undecided_only = lib.track(None, None);
    lib.file(undecided_only, "a.mp3", "undecided", true);
    let probable_only = lib.track(None, None);
    let maybe = lib.file(probable_only, "b.mp3", "undecided", true);
    lib.rekordbox_match(maybe, true);

    for track in [no_files, undecided_only, probable_only] {
        assert!(matches!(lib.promote(track), Err(LibraryError::NoFile)));
    }
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(lib.operations(), 0);
}

#[test]
fn a_track_whose_rekordbox_file_is_not_on_disk_is_refused_without_falling_back() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "best.flac", "best", true);
    let gone = lib.file(track, "gone.mp3", "undecided", false);
    lib.rekordbox_match(gone, false);

    match lib.promote(track) {
        Err(LibraryError::FileMissing { path }) => assert_eq!(path, r"E:\Music\gone.mp3"),
        other => panic!("{other:?}"),
    }
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(lib.operations(), 0);
}

#[test]
fn a_track_whose_best_file_is_not_on_disk_is_refused() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "gone.mp3", "best", false);
    lib.file(track, "here.mp3", "extra", true);

    assert!(matches!(
        lib.promote(track),
        Err(LibraryError::FileMissing { .. })
    ));
    assert_eq!(lib.library_tracks(), 0);
}

#[test]
fn a_track_that_does_not_exist_is_refused() {
    let lib = Lib::new();
    assert!(matches!(lib.promote(99), Err(LibraryError::TrackNotFound)));
    assert_eq!(lib.library_tracks(), 0);
}

#[test]
fn a_refused_add_names_the_file_even_when_its_full_path_cannot_be_made() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "Sub/gone.mp3", "best", false);
    // The volume's identity no longer reads back, so there's no full path.
    lib.writer
        .call(|c| c.execute("UPDATE volume SET identity = 'dev=?'", []))
        .unwrap();

    match lib.promote(track) {
        Err(LibraryError::FileMissing { path }) => assert_eq!(path, r"Sub\gone.mp3"),
        other => panic!("{other:?}"),
    }
}

// An unplugged drive isn't a missing file (1aB-9).

#[test]
fn a_track_whose_file_is_on_an_unplugged_drive_can_be_added() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let file = lib.file(track, "a.mp3", "best", true);

    let added = promote(&lib.writer, &Mount(None), track).unwrap();
    assert!(added.added);
    assert_eq!(lib.linked_file(track), Some(file));
    let linked = added.library_track.file.unwrap();
    assert!(linked.present);
    assert_eq!(linked.path, r"E:\Music\a.mp3");
}

// One Library track per track.

#[test]
fn adding_a_track_twice_keeps_its_one_library_track_and_logs_nothing_more() {
    let lib = Lib::new();
    let track = lib.track(Some("Synthetic Tune"), None);
    let first_file = lib.file(track, "a.mp3", "best", true);
    let first = lib.promote(track).unwrap();
    assert!(first.added);
    assert_eq!(lib.operations(), 1);

    // rekordbox now uses another of the track's files: the existing link
    // still isn't re-pointed.
    let other = lib.file(track, "b.mp3", "undecided", true);
    lib.rekordbox_match(other, false);
    let again = lib.promote(track).unwrap();
    assert!(!again.added);
    assert_eq!(again.library_track, first.library_track);
    assert_eq!(lib.library_tracks(), 1);
    assert_eq!(lib.operations(), 1);
    assert_eq!(lib.linked_file(track), Some(first_file));
}

#[test]
fn the_table_refuses_a_second_library_track_for_one_track() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let file = lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();

    let second = lib.writer.call(move |c| {
        c.execute(
            "INSERT INTO library_track (recording_id, linked_file_id) VALUES (?1, ?2)",
            [track, file],
        )
    });
    assert!(second.is_err(), "{second:?}");
    assert_eq!(lib.library_tracks(), 1);
}

// Undo.

#[test]
fn undo_after_a_repeat_add_removes_the_last_track_really_added() {
    let lib = Lib::new();
    let first = lib.track(None, None);
    lib.file(first, "a.mp3", "best", true);
    let second = lib.track(None, None);
    lib.file(second, "b.mp3", "best", true);
    lib.promote(first).unwrap();
    lib.promote(second).unwrap();
    // Changes nothing, so there's nothing of it to undo.
    assert!(!lib.promote(first).unwrap().added);

    assert!(matches!(
        undo_last_via(&lib.writer).unwrap(),
        UndoOutcome::Undone { .. }
    ));
    let left: Vec<_> = lib.list().iter().map(|t| t.recording_id).collect();
    assert_eq!(left, [first]);
}

#[test]
fn undoing_an_add_takes_the_track_out_of_the_library_again() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();
    assert_eq!(lib.list().len(), 1);

    match undo_last_via(&lib.writer).unwrap() {
        UndoOutcome::Undone { operation } => assert_eq!(operation.kind, PROMOTE_OPERATION),
        other => panic!("{other:?}"),
    }
    assert_eq!(lib.library_tracks(), 0);
    assert!(lib.list().is_empty());
    // The track and its file are still in All music.
    assert_eq!(lib.count("SELECT count(*) FROM recording"), 1);
    assert_eq!(lib.count("SELECT count(*) FROM file"), 1);

    // And it can be added again.
    assert!(lib.promote(track).unwrap().added);
}

#[test]
fn an_add_records_the_track_it_added_without_ui_text() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();

    let (kind, details): (String, String) = lib
        .writer
        .call(|c| {
            c.query_row("SELECT kind, details FROM operation", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
        })
        .unwrap();
    assert_eq!(kind, "promote");
    let details: serde_json::Value = serde_json::from_str(&details).unwrap();
    assert_eq!(details, json!({ "recordingId": track }));
}

// The file is never written.

#[cfg(windows)]
#[test]
fn adding_listing_and_undoing_never_write_or_touch_the_linked_file() {
    let lib = Lib::new();
    // The volume is mounted at a real folder, holding the real linked file.
    let mount = std::fs::canonicalize(lib.dir.path()).unwrap();
    std::fs::create_dir(mount.join("Music")).unwrap();
    let on_disk = mount.join("Music").join("a.mp3");
    std::fs::write(&on_disk, b"synthetic audio bytes").unwrap();
    let before = std::fs::metadata(&on_disk).unwrap();
    let volumes = Mount(Some(mount));

    let track = lib.track(None, None);
    lib.file(track, "a.mp3", "best", true);
    let added = promote(&lib.writer, &volumes, track).unwrap();
    // The Library track really names that file.
    assert_eq!(
        added.library_track.file.unwrap().path,
        display_path(&on_disk)
    );
    assert_eq!(lib.list_with(&volumes).len(), 1);
    undo_last_via(&lib.writer).unwrap();
    promote(&lib.writer, &volumes, track).unwrap();

    let after = std::fs::metadata(&on_disk).unwrap();
    assert_eq!(std::fs::read(&on_disk).unwrap(), b"synthetic audio bytes");
    assert_eq!(after.len(), before.len());
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    // Nothing else appeared next to it either.
    let names: Vec<_> = std::fs::read_dir(on_disk.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["a.mp3"]);
}

#[test]
fn the_library_module_opens_no_files() {
    // The module works from the database alone: its source never names the
    // file system. (The write guard's source scan covers writes app-wide;
    // this also keeps reads out.)
    let source = include_str!("mod.rs");
    for forbidden in ["std::fs", "fs::", "File::", "OpenOptions"] {
        assert!(
            !source.contains(forbidden),
            "library/mod.rs uses {forbidden}"
        );
    }
}

// The list.

#[test]
fn an_empty_library_lists_no_tracks() {
    let lib = Lib::new();
    lib.track(Some("Not Added"), None);
    assert!(lib.list().is_empty());
}

#[test]
fn the_list_gives_each_tracks_title_artist_file_path_presence_and_kind() {
    let lib = Lib::new();
    let track = lib.track(Some("Synthetic Tune"), Some("Made Up Artist"));
    lib.file(track, "Sub/tune.mp3", "best", true);
    let added = lib.promote(track).unwrap();

    let listed = lib.list();
    assert_eq!(listed, std::slice::from_ref(&added.library_track));
    let row = &listed[0];
    assert_eq!(row.recording_id, track);
    assert_eq!(row.kind, LibraryTrackKind::Linked);
    assert_eq!(row.title.as_deref(), Some("Synthetic Tune"));
    assert_eq!(row.artist.as_deref(), Some("Made Up Artist"));
    assert_eq!(
        row.file,
        Some(LinkedFile {
            path: r"E:\Music\Sub\tune.mp3".to_owned(),
            name: "tune.mp3".to_owned(),
            present: true,
            drive_connected: true,
        })
    );
    assert!(row.added_at.ends_with('Z'), "{}", row.added_at);
}

#[test]
fn a_linked_file_that_has_gone_from_disk_is_listed_as_not_present() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let file = lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();
    lib.writer
        .call(move |c| c.execute("UPDATE file SET present = 0 WHERE id = ?1", [file]))
        .unwrap();

    let listed = lib.list();
    assert_eq!(listed.len(), 1);
    let file = listed[0].file.as_ref().unwrap();
    assert!(!file.present);
    assert_eq!(file.path, r"E:\Music\a.mp3");
}

#[test]
fn a_file_on_an_unplugged_volume_is_listed_where_the_volume_was_last_mounted() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();

    let listed = lib.list_with(&Mount(None));
    let file = listed[0].file.as_ref().unwrap();
    assert_eq!(file.path, r"E:\Music\a.mp3");
    // Offline files stay present (1aB-9).
    assert!(file.present);
    assert!(!file.drive_connected);

    // Plugged in under another letter, it's listed there.
    let listed = lib.list_with(&Mount::at(r"G:\"));
    assert_eq!(listed[0].file.as_ref().unwrap().path, r"G:\Music\a.mp3");
}

#[test]
fn a_track_with_no_title_or_a_blank_one_is_listed_without_a_title() {
    let lib = Lib::new();
    for (title, name) in [(None, "a.mp3"), (Some("  "), "b.mp3")] {
        let track = lib.track(title, Some(""));
        lib.file(track, name, "best", true);
        lib.promote(track).unwrap();
    }
    for row in lib.list() {
        assert_eq!(row.title, None);
        assert_eq!(row.artist, None);
    }
}

#[test]
fn the_list_is_sorted_by_shown_title_ignoring_case_then_artist_then_order_added() {
    let lib = Lib::new();
    let add = |title: Option<&str>, artist: Option<&str>, name: &str| {
        let track = lib.track(title, artist);
        lib.file(track, name, "best", true);
        lib.promote(track).unwrap().library_track.id
    };
    let zulu = add(Some("zulu"), None, "1.mp3");
    let bravo_b = add(Some("Bravo"), Some("b artist"), "2.mp3");
    // No title: sorted by its file's name, "charlie.mp3".
    let charlie = add(None, None, "charlie.mp3");
    let bravo_a = add(Some("bravo"), Some("A Artist"), "3.mp3");
    let alpha_first = add(Some("Alpha"), Some("Same"), "4.mp3");
    let alpha_second = add(Some("alpha"), Some("same"), "5.mp3");

    let order: Vec<_> = lib.list().iter().map(|t| t.id).collect();
    assert_eq!(
        order,
        [alpha_first, alpha_second, bravo_a, bravo_b, charlie, zulu]
    );
}

#[test]
fn accented_titles_sort_with_their_base_letters_and_other_scripts_after_latin() {
    let lib = Lib::new();
    let add = |title: &str, artist: Option<&str>, name: &str| {
        let track = lib.track(Some(title), artist);
        lib.file(track, name, "best", true);
        lib.promote(track).unwrap().library_track.id
    };
    let cyrillic = add("Жара", None, "1.mp3");
    let zulu = add("Zulu", None, "2.mp3");
    let elan = add("Élan", None, "3.mp3");
    let foxtrot = add("foxtrot", None, "4.mp3");
    let delta = add("Delta", None, "5.mp3");
    // Differ only by accent and case: a tie, settled by artist.
    let arger_b = add("ärger", Some("B"), "6.mp3");
    let arger_a = add("Arger", Some("Å"), "7.mp3");

    let order: Vec<_> = lib.list().iter().map(|t| t.id).collect();
    assert_eq!(
        order,
        [arger_a, arger_b, delta, elan, foxtrot, zulu, cyrillic]
    );
}

#[test]
fn library_tracks_are_still_there_after_the_database_is_reopened() {
    let lib = Lib::new();
    let track = lib.track(Some("Synthetic Tune"), Some("Made Up Artist"));
    lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();
    let before = lib.list();

    let Lib { dir, writer, .. } = lib;
    drop(writer);
    let writer = open(dir.path());
    let (tracks, located) = writer.call(|c| stored_with_locations(c)).unwrap();
    let mount = Mount::at(r"E:\");
    assert_eq!(
        list_with_fragile(&tracks, &located, &mount, &NO_DIRS),
        before
    );
    assert_eq!(before.len(), 1);
}

// Over IPC, the way the frontend calls it.

mod ipc {
    use serde_json::json;
    use tauri::Manager;

    use crate::db::Writer;
    use crate::ipc::testing::{app, invoke};

    #[test]
    fn a_fresh_library_lists_no_tracks_over_ipc() {
        let (_data, app) = app();
        assert_eq!(invoke(&app, "library_tracks", json!({})), Ok(json!([])));
    }

    #[test]
    fn a_track_added_over_ipc_is_listed_and_adding_it_again_changes_nothing() {
        let (_data, app) = app();
        app.state::<Writer>()
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                         VALUES ('serial=NTFS-1A2B3C4D', 'external', 'Q:\\');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                         VALUES (1, 'Music', 'Music');
                     INSERT INTO file (music_folder_id, rel_path, rel_path_key)
                         VALUES (1, 'a.mp3', 'a.mp3');
                     INSERT INTO recording (title) VALUES ('Synthetic Tune');
                     INSERT INTO recording_file (recording_id, file_id, role)
                         VALUES (1, 1, 'best');",
                )
            })
            .unwrap();

        let added = invoke(&app, "promote_track", json!({ "recordingId": 1 })).unwrap();
        assert_eq!(added["added"], json!(true));
        let track = &added["libraryTrack"];
        assert_eq!(track["recordingId"], json!(1));
        assert_eq!(track["kind"], json!("linked"));
        assert_eq!(track["title"], json!("Synthetic Tune"));
        assert_eq!(track["artist"], json!(null));
        assert_eq!(track["file"]["name"], json!("a.mp3"));
        assert_eq!(track["file"]["present"], json!(true));

        // The test volume is an external drive, and the real command (not a
        // rebuilt copy of it) says so on the listed row too.
        assert_eq!(track["fragile"], json!("external"));
        let listed = invoke(&app, "library_tracks", json!({})).unwrap();
        assert_eq!(listed, json!([track]));
        assert_eq!(listed[0]["fragile"], json!("external"));

        let again = invoke(&app, "promote_track", json!({ "recordingId": 1 })).unwrap();
        assert_eq!(again["added"], json!(false));
        assert_eq!(&again["libraryTrack"], track);

        // Undo over IPC takes it out again.
        let undone = invoke(&app, "undo_last_operation", json!({})).unwrap();
        assert_eq!(undone["status"], json!("undone"));
        assert_eq!(invoke(&app, "library_tracks", json!({})), Ok(json!([])));
    }

    #[test]
    fn refused_adds_reach_the_frontend_as_error_kinds() {
        let (_data, app) = app();
        assert_eq!(
            invoke(&app, "promote_track", json!({ "recordingId": 7 })),
            Err(json!({ "kind": "libraryTrackNotFound", "params": {} }))
        );
        app.state::<Writer>()
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                         VALUES ('serial=NTFS-1A2B3C4D', 'external', 'Q:\\');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                         VALUES (1, 'Music', 'Music');
                     INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
                         VALUES (1, 'gone.mp3', 'gone.mp3', 0);
                     INSERT INTO recording DEFAULT VALUES;
                     INSERT INTO recording DEFAULT VALUES;
                     INSERT INTO recording_file (recording_id, file_id, role)
                         VALUES (2, 1, 'best');",
                )
            })
            .unwrap();
        assert_eq!(
            invoke(&app, "promote_track", json!({ "recordingId": 1 })),
            Err(json!({ "kind": "libraryNoFile", "params": {} }))
        );
        let missing = invoke(&app, "promote_track", json!({ "recordingId": 2 })).unwrap_err();
        assert_eq!(missing["kind"], json!("libraryFileMissing"));
        let path = missing["params"]["path"].as_str().unwrap();
        assert!(path.ends_with(r"Music\gone.mp3"), "{path}");
    }

    #[test]
    fn an_add_held_back_for_an_unconfirmed_match_reaches_the_frontend_as_its_own_error_kind() {
        let (_data, app) = app();
        app.state::<Writer>()
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                         VALUES ('serial=NTFS-1A2B3C4D', 'external', 'Q:\\');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                         VALUES (1, 'Music', 'Music');
                     INSERT INTO recording DEFAULT VALUES;
                     INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
                         VALUES (1, 'renamed.mp3', 'renamed.mp3', 1);
                     INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 1, 'best');
                     INSERT INTO rekordbox_track
                         (attributes, location_key, read_at, file_id, relink_method, relink_probable)
                     VALUES ('{\"TrackID\":\"7\",\"Location\":\"file://localhost/Q:/Music/old.mp3\"}',
                             'Q:/Music/old.mp3', '2026-09-30T10:00:00.000Z', 1, 'filename_only', 1);",
                )
            })
            .unwrap();
        assert_eq!(
            invoke(&app, "promote_track", json!({ "recordingId": 1 })),
            Err(json!({ "kind": "libraryMatchNotConfirmed", "params": {} }))
        );
    }

    #[test]
    fn bindings_declare_the_library_commands_and_types() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.ts");
        crate::ipc::export_bindings(&path).unwrap();
        let ts = std::fs::read_to_string(&path).unwrap();
        for expected in [
            r#"libraryTracks: () => typedError<LibraryTrack[], IpcError>(__TAURI_INVOKE("library_tracks"))"#,
            r#"promoteTrack: (recordingId: number) => typedError<Promoted, IpcError>(__TAURI_INVOKE("promote_track", { recordingId }))"#,
            "export type LibraryTrack = {",
            "export type LinkedFile = {",
            r#"export type LibraryTrackKind = "#,
            "export type LibraryTrackId = number;",
        ] {
            assert!(ts.contains(expected), "missing `{expected}` in:\n{ts}");
        }
    }
}

#[test]
fn the_list_carries_each_rows_fragile_reason() {
    let lib = Lib::new();
    let track = lib.track(Some("A"), None);
    lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();
    // The test volume is an external drive.
    assert_eq!(lib.list()[0].fragile, Some(FragileReason::External));
}

#[test]
fn a_row_whose_file_is_in_a_fragile_folder_says_so_before_the_drive_kind() {
    let lib = Lib::new();
    let track = lib.track(Some("A"), None);
    lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();
    let (tracks, located) = lib.writer.call(|c| stored_with_locations(c)).unwrap();
    let dirs = FragileDirs {
        downloads: Some(PathBuf::from(r"E:\Music")),
        temp: vec![],
    };
    let listed = list_with_fragile(&tracks, &located, &Mount::at(r"E:\"), &dirs);
    assert_eq!(listed[0].fragile, Some(FragileReason::Downloads));
}

#[test]
fn a_track_just_added_carries_its_fragile_reason() {
    let lib = Lib::new();
    let track = lib.track(Some("A"), None);
    lib.file(track, "a.mp3", "best", true);
    assert_eq!(
        lib.promote(track).unwrap().library_track.fragile,
        Some(FragileReason::External)
    );
}
