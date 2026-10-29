//! Crate command tests, over a migrated database in a temp dir. Everything
//! is synthetic: made-up tracks with no files.

use std::path::{Path, PathBuf};

use rusqlite::types::Value;

use super::*;
use crate::after_send::{lists, save_tree, RekordboxTree, TreeNode};
use crate::ops::{undo_last_via, UndoOutcome};
use crate::rekordbox_write::record_send;
use crate::send::crate_tree;
use crate::volume::{Volume, VolumeId};

#[path = "folder_tests.rs"]
mod folders;

/// Library tracks have no files here, so no drive is ever looked up.
struct NoVolumes;

impl Volumes for NoVolumes {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up in these tests",
            path.display()
        )))
    }

    fn mount_path(&self, _id: &VolumeId) -> Option<PathBuf> {
        None
    }
}

struct Lib {
    writer: Writer,
    // Last: fields drop in order, so the connections close before the folder goes.
    _dir: tempfile::TempDir,
}

/// Every row of a query, as text, for comparing whole states.
fn dump(conn: &Connection, sql: &str) -> Vec<Vec<String>> {
    let mut stmt = conn.prepare(sql).unwrap();
    let columns = stmt.column_count();
    stmt.query_map([], |r| {
        (0..columns)
            .map(|i| r.get::<_, Value>(i).map(|v| format!("{v:?}")))
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// Everything crates are made of, row by row.
type State = (Vec<Vec<String>>, Vec<Vec<String>>);

impl Lib {
    fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        Lib { _dir: dir, writer }
    }

    fn run(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// A Library track with no file, titled `title`.
    fn track(&self, title: &'static str) -> LibraryTrackId {
        let recording = self.run(
            "INSERT INTO recording (title, artist) VALUES (?1, 'Kit')",
            (title,),
        );
        LibraryTrackId(self.run(
            "INSERT INTO library_track (recording_id, source_status) VALUES (?1, 'missing')",
            (recording,),
        ))
    }

    /// A track that isn't in the Library, titled `title`: its id.
    fn outside(&self, title: &'static str) -> i64 {
        self.run(
            "INSERT INTO recording (title, artist) VALUES (?1, 'Kit')",
            (title,),
        )
    }

    /// A track outside the Library with one best file (`<title>.mp3`) in a
    /// music folder, on disk or not: its id.
    fn outside_with_file(&self, title: &'static str, present: bool) -> i64 {
        let recording = self.outside(title);
        self.run(
            "INSERT INTO volume (identity, kind, last_mount_path)
             SELECT 'test-volume', 'external', 'E:\\' WHERE NOT EXISTS (SELECT 1 FROM volume)",
            (),
        );
        self.run(
            "INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
             SELECT (SELECT min(id) FROM volume), 'Music', 'Music'
             WHERE NOT EXISTS (SELECT 1 FROM music_folder)",
            (),
        );
        let file = self.run(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
             VALUES ((SELECT min(id) FROM music_folder), ?1, ?1, ?2)",
            (format!("{title}.mp3"), present),
        );
        self.run(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
            (recording, file),
        );
        recording
    }

    /// The best file of a track.
    fn best_file(&self, recording: i64) -> i64 {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT file_id FROM recording_file WHERE recording_id = ?1 AND role = 'best'",
                    [recording],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// Whether a track is in the Library now.
    fn in_library(&self, recording: i64) -> bool {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM library_track WHERE recording_id = ?1)",
                    [recording],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// The track (`TableRow.recordingId`) a Library track is.
    fn recording_of(&self, track: LibraryTrackId) -> i64 {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT recording_id FROM library_track WHERE id = ?1",
                    [track.0],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    fn add_recordings(&self, id: CrateId, recordings: &[i64]) -> Done<AddedToCrate> {
        let recordings = recordings.to_vec();
        self.writer
            .call(move |c| Ok(add_recordings_on(c, id, &recordings)))
            .unwrap()
            .unwrap()
    }

    fn crates_of(&self, recording: i64) -> Vec<CrateId> {
        self.writer.call(move |c| crates_of(c, recording)).unwrap()
    }

    fn create(&self, name: &str) -> Done<CrateId> {
        let name = name.to_owned();
        self.writer
            .call(move |c| Ok(create_on(c, &name)))
            .unwrap()
            .unwrap()
    }

    fn made(&self, name: &str) -> CrateId {
        self.create(name).unwrap()
    }

    fn rename(&self, id: CrateId, name: &str) -> Done<Option<i64>> {
        let name = name.to_owned();
        self.writer
            .call(move |c| Ok(rename_on(c, id, &name)))
            .unwrap()
            .unwrap()
    }

    fn delete(&self, id: CrateId) -> Done<()> {
        self.writer
            .call(move |c| Ok(delete_on(c, id)))
            .unwrap()
            .unwrap()
    }

    fn add(&self, id: CrateId, tracks: &[LibraryTrackId]) -> Done<Changed> {
        let tracks = tracks.to_vec();
        self.writer
            .call(move |c| Ok(add_on(c, id, &tracks)))
            .unwrap()
            .unwrap()
    }

    fn take_out(&self, id: CrateId, tracks: &[LibraryTrackId]) -> Done<Changed> {
        let tracks = tracks.to_vec();
        self.writer
            .call(move |c| Ok(remove_on(c, id, &tracks)))
            .unwrap()
            .unwrap()
    }

    fn undo(&self) {
        match undo_last_via(&self.writer).unwrap() {
            UndoOutcome::Undone { .. } => {}
            other => panic!("expected the undo to work, got {other:?}"),
        }
    }

    fn crates(&self) -> Vec<(String, u32)> {
        self.writer
            .call(|c| list(c))
            .unwrap()
            .into_iter()
            .map(|c| (c.name, c.track_count))
            .collect()
    }

    fn ids(&self, id: CrateId) -> Vec<LibraryTrackId> {
        self.writer
            .call(move |c| track_ids(c, id))
            .unwrap()
            .unwrap()
    }

    fn operations(&self) -> i64 {
        self.writer
            .call(|c| c.query_row("SELECT count(*) FROM operation", [], |r| r.get(0)))
            .unwrap()
    }

    /// Every column of every crate and entry row.
    fn state(&self) -> State {
        self.writer
            .call(|c| {
                Ok((
                    dump(c, "SELECT * FROM crate ORDER BY id"),
                    dump(c, "SELECT * FROM crate_entry ORDER BY id"),
                ))
            })
            .unwrap()
    }

    /// Everything an add can change: the crates, their entries, the Library,
    /// and how many operations are logged.
    fn everything(&self) -> (State, Vec<Vec<String>>, i64) {
        let library = self
            .writer
            .call(|c| Ok(dump(c, "SELECT * FROM library_track ORDER BY id")))
            .unwrap();
        (self.state(), library, self.operations())
    }

    /// The files under the folder the database lives in, so a test can tell
    /// that nothing was written beside it (the database's own journal files
    /// come and go, and are left out).
    fn files_on_disk(&self) -> Vec<String> {
        fn walk(dir: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push(path.to_string_lossy().into_owned());
                }
            }
        }
        let mut out = Vec::new();
        walk(self._dir.path(), &mut out);
        out.retain(|f| !f.ends_with("-wal") && !f.ends_with("-shm"));
        out.sort();
        out
    }

    /// Gives a crate's entries these add times, in the order of `tracks`, so
    /// the order they were added in doesn't depend on the clock.
    fn added_at(&self, id: CrateId, tracks: &[LibraryTrackId]) {
        for (i, track) in tracks.iter().enumerate() {
            self.run(
                "UPDATE crate_entry SET added_at = ?3 WHERE crate_id = ?1 AND library_track_id = ?2",
                (id.0, track.0, format!("2026-10-01T10:00:0{i}.000Z")),
            );
        }
    }
}

/// (changed, skipped) of an add or a removal.
fn counts(changed: Changed) -> (u32, u32) {
    (changed.changed, changed.skipped)
}

fn names(refused: Done<CrateId>) -> String {
    match refused {
        Err(CratesError::NameTaken { existing }) => format!("taken by {existing}"),
        Err(CratesError::NameEmpty) => "empty".to_owned(),
        Err(CratesError::NameUnsendable) => "unsendable".to_owned(),
        other => format!("{other:?}"),
    }
}

// ---- making, renaming, deleting ---------------------------------------------

#[test]
fn a_new_crate_is_a_static_top_level_crate_with_no_tracks_listed_after_the_others() {
    let lib = Lib::new();
    lib.made("Warm up");
    lib.made("Peak time");
    assert_eq!(
        lib.crates(),
        [("Warm up".to_owned(), 0), ("Peak time".to_owned(), 0)]
    );
    assert_eq!(
        lib.writer
            .call(|c| c.query_row(
                "SELECT count(*) FROM crate WHERE kind = 'static' AND parent_id IS NULL",
                [],
                |r| r.get::<_, i64>(0)
            ))
            .unwrap(),
        2
    );
}

#[test]
fn a_name_is_trimmed_before_it_is_stored() {
    let lib = Lib::new();
    lib.made("  Warm up \t");
    assert_eq!(lib.crates(), [("Warm up".to_owned(), 0)]);
    let id = lib.made("Peak");
    lib.rename(id, "   Peak time  ").unwrap();
    assert_eq!(lib.crates()[1].0, "Peak time");
}

#[test]
fn an_empty_or_blank_name_is_refused_on_create_and_on_rename() {
    let lib = Lib::new();
    let id = lib.made("Warm up");
    for name in ["", "   ", "\t \n"] {
        assert_eq!(names(lib.create(name)), "empty", "{name:?}");
        assert!(
            matches!(lib.rename(id, name), Err(CratesError::NameEmpty)),
            "{name:?}"
        );
    }
    assert_eq!(lib.crates(), [("Warm up".to_owned(), 0)]);
}

#[test]
fn a_name_differing_only_by_case_or_trailing_space_is_refused_on_create() {
    let lib = Lib::new();
    lib.made("Warm up");
    for name in ["warm up", "WARM UP", "Warm up   ", "Warm up\t", " warm UP "] {
        assert_eq!(names(lib.create(name)), "taken by Warm up", "{name:?}");
    }
    assert_eq!(lib.crates(), [("Warm up".to_owned(), 0)]);
}

#[test]
fn a_name_differing_only_by_unicode_form_is_refused_on_create() {
    let lib = Lib::new();
    lib.made("Caf\u{e9}");
    // "e" and a combining acute accent.
    assert_eq!(names(lib.create("Cafe\u{301}")), "taken by Caf\u{e9}");
    lib.made("Cafe\u{301}s");
    assert_eq!(names(lib.create("Caf\u{e9}S")), "taken by Cafe\u{301}s");
}

#[test]
fn a_name_differing_only_by_case_or_trailing_space_is_refused_on_rename() {
    let lib = Lib::new();
    lib.made("Warm up");
    let id = lib.made("Peak time");
    for name in ["warm up", "WARM UP", "Warm up  "] {
        assert!(
            matches!(lib.rename(id, name), Err(CratesError::NameTaken { existing }) if existing == "Warm up"),
            "{name:?}"
        );
    }
    assert_eq!(
        lib.crates(),
        [("Warm up".to_owned(), 0), ("Peak time".to_owned(), 0)]
    );
}

#[test]
fn a_crate_can_be_renamed_to_its_own_name_in_another_letter_case() {
    let lib = Lib::new();
    let id = lib.made("warm up");
    lib.rename(id, "Warm up").unwrap();
    assert_eq!(lib.crates(), [("Warm up".to_owned(), 0)]);
}

#[test]
fn a_refused_name_records_nothing_and_changes_nothing() {
    let lib = Lib::new();
    let id = lib.made("Warm up");
    let (state, operations) = (lib.state(), lib.operations());
    let _ = lib.create("warm up");
    let _ = lib.create("");
    let _ = lib.rename(id, "");
    let _ = lib.rename(CrateId(999), "Anything");
    let _ = lib.delete(CrateId(999));
    assert_eq!((lib.state(), lib.operations()), (state, operations));
}

#[test]
fn a_name_with_a_character_xml_cannot_carry_is_refused() {
    let lib = Lib::new();
    let id = lib.made("Fine");
    for name in ["a\u{1}b", "bell\u{7}", "nul\u{0}", "end\u{ffff}"] {
        assert_eq!(names(lib.create(name)), "unsendable", "{name:?}");
        assert!(
            matches!(lib.rename(id, name), Err(CratesError::NameUnsendable)),
            "{name:?}"
        );
    }
    assert_eq!(lib.crates(), [("Fine".to_owned(), 0)]);
}

#[test]
fn a_send_is_never_refused_because_of_a_crate_the_app_let_the_user_make() {
    let lib = Lib::new();
    let track = lib.track("A");
    let accepted = [
        "Warm up",
        "Peak & Close <b>",
        r#"Say "hi" 'there'"#,
        "Tab\there",
        "日本語のクレート",
        "Party 🚀",
        "Caf\u{e9}",
        "Cafe\u{301}s",
        "Crates",
        "Playlists",
        "100%",
        "a/b\\c",
    ];
    for name in accepted {
        let id = lib.made(name);
        lib.add(id, &[track]).unwrap();
    }
    // Names the same as one of these are refused.
    for name in ["warm up", "Warm up ", "CAFE\u{301}S"] {
        assert!(lib.create(name).is_err(), "{name:?}");
    }
    let tree = lib.writer.call(|c| crate_tree(c)).unwrap();
    assert_eq!(tree.len(), accepted.len());
    let built = build(&SendInput {
        crates: tree,
        ..SendInput::default()
    });
    assert!(built.is_ok(), "{:?}", built.err());
}

// ---- adding by track id (1cB-13) and a track's crates (1cB-14) ---------------

/// (added, already there, also added to the Library) of an add by track id.
fn added(done: AddedToCrate) -> (u32, u32, u32) {
    (done.added, done.already_there, done.also_added_to_library)
}

/// (no file, file missing, match not confirmed, low bitrate) of an add.
fn left_out(done: AddedToCrate) -> (u32, u32, u32, u32) {
    let l = done.left_out;
    (
        l.no_file,
        l.file_missing,
        l.match_not_confirmed,
        l.low_bitrate,
    )
}

#[test]
fn adding_selected_tracks_is_one_operation_and_one_undo_takes_them_all_out() {
    let lib = Lib::new();
    let (a, b, c) = (lib.track("A"), lib.track("B"), lib.track("C"));
    let id = lib.made("Warm up");
    let before = lib.operations();
    let done = lib
        .add_recordings(id, &[a, b, c].map(|t| lib.recording_of(t)))
        .unwrap();
    assert_eq!(added(done), (3, 0, 0));
    assert_eq!(lib.operations(), before + 1);
    assert_eq!(done.operation_id, Some(before + 1));
    lib.undo();
    assert_eq!(lib.ids(id), Vec::<LibraryTrackId>::new());
}

#[test]
fn a_track_already_in_the_crate_is_not_added_twice_and_the_count_says_so() {
    let lib = Lib::new();
    let (a, b) = (lib.track("A"), lib.track("B"));
    let id = lib.made("Warm up");
    lib.add(id, &[a]).unwrap();
    let both = [a, b].map(|t| lib.recording_of(t));
    assert_eq!(added(lib.add_recordings(id, &both).unwrap()), (1, 1, 0));
    assert_eq!(lib.ids(id).len(), 2);
    // Listed twice counts once; all there already records nothing.
    let operations = lib.operations();
    let again = lib
        .add_recordings(id, &[both[0], both[0], both[1]])
        .unwrap();
    assert_eq!((added(again), again.operation_id), ((0, 2, 0), None));
    assert_eq!(lib.operations(), operations);
}

#[test]
fn an_outside_track_joins_the_library_and_the_crate_in_one_operation_and_one_undo_restores_both() {
    let lib = Lib::new();
    let stray = lib.outside_with_file("Stray", true);
    let file = lib.best_file(stray);
    let id = lib.made("Warm up");
    let before = lib.everything();
    assert!(!lib.in_library(stray));

    let done = lib.add_recordings(id, &[stray]).unwrap();

    assert_eq!(added(done), (1, 0, 1));
    assert_eq!(left_out(done), (0, 0, 0, 0));
    assert!(lib.in_library(stray));
    assert_eq!(lib.crates_of(stray), [id]);
    // Linked to its best file, as "Add to Library" does it; one operation.
    let linked: (String, i64) = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT kind, linked_file_id FROM library_track WHERE recording_id = ?1",
                [stray],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap();
    assert_eq!(linked, ("linked".to_owned(), file));
    assert_eq!(lib.operations(), before.2 + 1);
    assert_eq!(done.operation_id, Some(before.2 + 1));

    // One Undo takes the entry and the Library track away together.
    lib.undo();
    assert!(!lib.in_library(stray));
    assert_eq!(lib.everything(), before);
}

#[test]
fn a_mixed_selection_adds_library_and_outside_tracks_in_one_operation() {
    let lib = Lib::new();
    let (a, b) = (lib.track("A"), lib.track("B"));
    let (one, two) = (
        lib.outside_with_file("One", true),
        lib.outside_with_file("Two", true),
    );
    let id = lib.made("Warm up");
    lib.add(id, &[b]).unwrap();
    let before = lib.everything();

    let given = [
        lib.recording_of(a),
        one,
        lib.recording_of(b),
        two,
        // Listed twice counts once.
        one,
    ];
    let done = lib.add_recordings(id, &given).unwrap();

    // A, One and Two joined; B was there already; two came with the Library.
    assert_eq!(added(done), (3, 1, 2));
    assert_eq!(lib.ids(id).len(), 4);
    assert!(lib.in_library(one) && lib.in_library(two));
    assert_eq!(lib.operations(), before.2 + 1);
    lib.undo();
    assert_eq!(lib.everything(), before);
}

#[test]
fn tracks_add_to_library_refuses_are_left_out_of_both_and_counted_with_the_reason() {
    let lib = Lib::new();
    let no_file = lib.outside("No file");
    let missing = lib.outside_with_file("Missing", false);
    let unsure = lib.outside_with_file("Unsure", true);
    let file = lib.best_file(unsure);
    lib.run(
        "INSERT INTO rekordbox_track
             (attributes, location_key, read_at, file_id, relink_method, relink_probable)
         VALUES ('{}', 'E:/Music/unsure.mp3', '2026-09-30T10:00:00.000Z', ?1, 'filename_only', 1)",
        (file,),
    );
    let thin = lib.outside_with_file("Thin", true);
    let thin_file = lib.best_file(thin);
    lib.run(
        "UPDATE file SET codec = 'mp3', bitrate = 128 WHERE id = ?1",
        (thin_file,),
    );
    let good = lib.outside_with_file("Good", true);
    let id = lib.made("Warm up");

    let done = lib
        .add_recordings(id, &[no_file, missing, unsure, thin, good])
        .unwrap();

    assert_eq!(added(done), (1, 0, 1));
    assert_eq!(left_out(done), (1, 1, 1, 1));
    for refused in [no_file, missing, unsure, thin] {
        assert!(!lib.in_library(refused));
        assert_eq!(lib.crates_of(refused), Vec::<CrateId>::new());
    }
    assert!(lib.in_library(good));

    // Only refused tracks: nothing is recorded, and Undo is not offered.
    let before = lib.everything();
    let none = lib.add_recordings(id, &[no_file, missing]).unwrap();
    assert_eq!((added(none), none.operation_id), ((0, 0, 0), None));
    assert_eq!(left_out(none), (1, 1, 0, 0));
    assert_eq!(lib.everything(), before);
}

#[test]
fn adding_an_outside_track_to_a_crate_writes_and_copies_no_file() {
    let lib = Lib::new();
    let stray = lib.outside_with_file("Stray", true);
    let id = lib.made("Warm up");
    let before = lib.files_on_disk();
    let rows = |lib: &Lib| -> i64 {
        lib.writer
            .call(|c| c.query_row("SELECT count(*) FROM file", [], |r| r.get(0)))
            .unwrap()
    };
    let records = rows(&lib);

    lib.add_recordings(id, &[stray]).unwrap();
    assert_eq!(lib.files_on_disk(), before);
    assert_eq!(rows(&lib), records);
    lib.undo();
    assert_eq!(lib.files_on_disk(), before);
    assert_eq!(rows(&lib), records);
}

#[test]
fn a_new_crate_with_tracks_is_one_operation_and_one_undo_takes_the_crate_the_tracks_and_the_library_tracks_away() {
    let lib = Lib::new();
    let a = lib.track("A");
    let stray = lib.outside_with_file("Stray", true);
    let before = lib.everything();
    let recordings = vec![lib.recording_of(a), stray];

    let made = lib
        .writer
        .call(move |c| {
            Ok(add_recordings_to_new_crate_on(
                c,
                "  Closing ",
                &recordings,
            ))
        })
        .unwrap()
        .unwrap();
        .unwrap();

    assert_eq!(made.name, "Closing");
    assert_eq!(added(made.added), (2, 0, 1));
    assert_eq!(lib.ids(made.crate_id).len(), 2);
    assert!(lib.in_library(stray));
    assert_eq!(lib.operations(), before.2 + 1);
    assert_eq!(made.added.operation_id, Some(before.2 + 1));

    lib.undo();
    assert_eq!(lib.everything(), before);
    assert_eq!(lib.crates(), Vec::<(String, u32)>::new());
}

#[test]
fn a_failed_add_to_a_new_crate_leaves_no_crate_and_changes_nothing() {
    let lib = Lib::new();
    let a = lib.track("A");
    let stray = lib.outside_with_file("Stray", true);
    lib.writer
        .call(|c| Ok(create_folder_on(c, None, "Sets")))
        .unwrap()
        .unwrap()
        .unwrap();
    let before = lib.everything();
    let add = |name: &'static str, recordings: Vec<i64>| {
        lib.writer
            .call(move |c| Ok(add_recordings_to_new_crate_on(c, name, &recordings)))
            .unwrap()
            .unwrap()
    };
    let known = vec![lib.recording_of(a), stray];

    // A name a folder at the top has, an empty name, and a track that doesn't exist.
    assert!(matches!(
        add("sets", known.clone()),
        Err(CratesError::NameTaken { .. })
    ));
    assert!(matches!(
        add("  ", known.clone()),
        Err(CratesError::NameEmpty)
    ));
    assert!(matches!(
        add("Closing", vec![known[0], 99_999]),
        Err(CratesError::TrackNotFound)
    ));

    assert_eq!(lib.everything(), before);
    assert!(!lib.in_library(stray));
}

#[test]
fn adding_by_track_id_to_a_folder_or_a_missing_crate_or_with_a_missing_track_is_refused() {
    let lib = Lib::new();
    let a = lib.track("A");
    let recording = lib.recording_of(a);
    let id = lib.made("Warm up");
    let folder = lib
        .writer
        .call(|c| Ok(create_folder_on(c, None, "Friday")))
        .unwrap()
        .unwrap()
        .unwrap();
    let before = lib.state();
    assert!(matches!(
        lib.add_recordings(folder, &[recording]),
        Err(CratesError::NotFound)
    ));
    assert!(matches!(
        lib.add_recordings(CrateId(999), &[recording]),
        Err(CratesError::NotFound)
    ));
    assert!(matches!(
        lib.add_recordings(id, &[recording, 99_999]),
        Err(CratesError::TrackNotFound)
    ));
    assert_eq!(lib.state(), before);
}

#[test]
fn a_track_knows_its_crates_but_not_folders_or_other_tracks_crates() {
    let lib = Lib::new();
    let (a, b) = (lib.track("A"), lib.track("B"));
    let (warm, peak, other) = (lib.made("Warm up"), lib.made("Peak"), lib.made("Other"));
    lib.add(warm, &[a, b]).unwrap();
    lib.add(peak, &[a]).unwrap();
    lib.add(other, &[b]).unwrap();
    assert_eq!(lib.crates_of(lib.recording_of(a)), [warm, peak]);
    assert_eq!(lib.crates_of(lib.recording_of(b)), [warm, other]);
    assert_eq!(lib.crates_of(lib.outside("Stray")), Vec::<CrateId>::new());
    // Taking it out of one crate is one operation, and one Undo puts it back.
    lib.take_out(warm, &[a]).unwrap();
    assert_eq!(lib.crates_of(lib.recording_of(a)), [peak]);
    lib.undo();
    assert_eq!(lib.crates_of(lib.recording_of(a)), [warm, peak]);
}

#[test]
fn deleting_a_crate_takes_its_entries_but_leaves_the_library_tracks() {
    let lib = Lib::new();
    let (a, b) = (lib.track("A"), lib.track("B"));
    let keep = lib.made("Keep");
    let gone = lib.made("Gone");
    lib.add(keep, &[a]).unwrap();
    lib.add(gone, &[a, b]).unwrap();
    lib.delete(gone).unwrap();
    assert_eq!(lib.crates(), [("Keep".to_owned(), 1)]);
    assert_eq!(lib.ids(keep), [a]);
    let tracks: i64 = lib
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM library_track", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(tracks, 2);
}

// ---- adding and removing tracks ----------------------------------------------

#[test]
fn adding_a_track_twice_leaves_one_entry() {
    let lib = Lib::new();
    let a = lib.track("A");
    let id = lib.made("Warm up");
    assert_eq!(counts(lib.add(id, &[a]).unwrap()), (1, 0));
    assert_eq!(counts(lib.add(id, &[a]).unwrap()), (0, 1));
    assert_eq!(lib.ids(id), [a]);
    // A track listed twice in one call counts once.
    let b = lib.track("B");
    assert_eq!(counts(lib.add(id, &[b, b]).unwrap()), (1, 1));
    assert_eq!(lib.crates(), [("Warm up".to_owned(), 2)]);
}

#[test]
fn adding_only_tracks_already_there_records_no_operation() {
    let lib = Lib::new();
    let a = lib.track("A");
    let id = lib.made("Warm up");
    lib.add(id, &[a]).unwrap();
    let operations = lib.operations();
    lib.add(id, &[a]).unwrap();
    assert_eq!(lib.operations(), operations);
    // So undo still takes back the add that did something.
    lib.undo();
    assert_eq!(lib.ids(id), Vec::<LibraryTrackId>::new());
}

#[test]
fn a_change_answers_with_the_operation_it_recorded() {
    let lib = Lib::new();
    let (a, b) = (lib.track("A"), lib.track("B"));
    let id = lib.made("Warm up");
    let latest = |lib: &Lib| -> i64 {
        lib.writer
            .call(|c| c.query_row("SELECT max(id) FROM operation", [], |r| r.get(0)))
            .unwrap()
    };
    assert_eq!(
        lib.add(id, &[a, b]).unwrap().operation_id,
        Some(latest(&lib))
    );
    assert_eq!(lib.rename(id, "Peak time").unwrap(), Some(latest(&lib)));
    assert_eq!(
        lib.take_out(id, &[a]).unwrap().operation_id,
        Some(latest(&lib))
    );
}

#[test]
fn a_command_that_changes_nothing_records_no_operation_and_leaves_the_log_alone() {
    let lib = Lib::new();
    let (a, b) = (lib.track("A"), lib.track("B"));
    let id = lib.made("Warm up");
    lib.add(id, &[a]).unwrap();
    let (operations, state) = (lib.operations(), lib.state());

    // Renamed to the name it has.
    assert_eq!(lib.rename(id, "Warm up").unwrap(), None);
    assert_eq!(lib.rename(id, "  Warm up  ").unwrap(), None);
    // An add where every track is already there.
    assert_eq!(lib.add(id, &[a]).unwrap().operation_id, None);
    // A removal of tracks that aren't in the crate (already gone).
    assert_eq!(lib.take_out(id, &[b]).unwrap().operation_id, None);

    assert_eq!((lib.operations(), lib.state()), (operations, state));
    // So undo still takes back the add that did something, not an earlier one.
    lib.undo();
    assert_eq!(lib.ids(id), Vec::<LibraryTrackId>::new());
    assert_eq!(lib.crates(), [("Warm up".to_owned(), 0)]);
}

#[test]
fn one_or_many_tracks_can_be_added_in_one_call() {
    let lib = Lib::new();
    let (a, b, c) = (lib.track("A"), lib.track("B"), lib.track("C"));
    let id = lib.made("Warm up");
    assert_eq!(lib.add(id, &[a, b, c]).unwrap().changed, 3);
    assert_eq!(lib.crates(), [("Warm up".to_owned(), 3)]);
}

#[test]
fn adding_an_unknown_track_or_to_an_unknown_crate_is_refused_and_adds_none() {
    let lib = Lib::new();
    let a = lib.track("A");
    let id = lib.made("Warm up");
    let before = lib.state();
    assert!(matches!(
        lib.add(id, &[a, LibraryTrackId(999)]),
        Err(CratesError::TrackNotFound)
    ));
    assert!(matches!(
        lib.add(CrateId(999), &[a]),
        Err(CratesError::NotFound)
    ));
    assert_eq!(lib.state(), before);
}

#[test]
fn removing_tracks_takes_them_out_of_the_crate_only_and_skips_those_not_in_it() {
    let lib = Lib::new();
    let (a, b, c) = (lib.track("A"), lib.track("B"), lib.track("C"));
    let id = lib.made("Warm up");
    lib.add(id, &[a, b]).unwrap();
    let changed = lib.take_out(id, &[b, c]).unwrap();
    assert_eq!(counts(changed), (1, 1));
    assert_eq!(lib.ids(id), [a]);
    let tracks: i64 = lib
        .writer
        .call(|c| c.query_row("SELECT count(*) FROM library_track", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(tracks, 3);
}

#[test]
fn a_crates_tracks_come_in_the_order_they_were_added_and_the_order_a_send_writes() {
    let lib = Lib::new();
    let (a, b, c) = (lib.track("A"), lib.track("B"), lib.track("C"));
    let id = lib.made("Warm up");
    lib.add(id, &[a, b, c]).unwrap();
    lib.added_at(id, &[c, a, b]);
    assert_eq!(lib.ids(id), [c, a, b]);
    let tree = lib.writer.call(|c| crate_tree(c)).unwrap();
    assert_eq!(
        tree,
        [Node::Playlist {
            name: "Warm up".to_owned(),
            entries: vec![c, a, b]
        }]
    );
    // And as rows, in that order.
    let (stored, located) = lib
        .writer
        .call(|conn| library::stored_with_locations(conn, &NoVolumes))
        .unwrap();
    let rows = ordered_tracks(
        &lib.ids(id),
        &stored,
        &located,
        &NoVolumes,
        &FragileDirs {
            downloads: None,
            temp: Vec::new(),
        },
    );
    let titles: Vec<_> = rows.iter().map(|t| t.title.clone().unwrap()).collect();
    assert_eq!(titles, ["C", "A", "B"]);
}

// ---- undo ----------------------------------------------------------------------

#[test]
fn undoing_a_create_removes_the_crate() {
    let lib = Lib::new();
    lib.made("Warm up");
    let before = lib.state();
    lib.made("Peak time");
    lib.undo();
    assert_eq!(lib.state(), before);
}

#[test]
fn undoing_a_rename_puts_the_old_name_back() {
    let lib = Lib::new();
    let id = lib.made("Warm up");
    let before = lib.state();
    lib.rename(id, "Warm up (new)").unwrap();
    assert_ne!(lib.state(), before);
    lib.undo();
    assert_eq!(lib.state(), before);
}

#[test]
fn undoing_an_add_takes_out_exactly_the_tracks_it_added() {
    let lib = Lib::new();
    let (a, b, c) = (lib.track("A"), lib.track("B"), lib.track("C"));
    let id = lib.made("Warm up");
    lib.add(id, &[a]).unwrap();
    let before = lib.state();
    lib.add(id, &[a, b, c]).unwrap();
    lib.undo();
    assert_eq!(lib.state(), before);
    assert_eq!(lib.ids(id), [a]);
}

#[test]
fn undoing_a_removal_restores_the_entries_and_their_order_exactly() {
    let lib = Lib::new();
    let (a, b, c, d) = (
        lib.track("A"),
        lib.track("B"),
        lib.track("C"),
        lib.track("D"),
    );
    let id = lib.made("Warm up");
    lib.add(id, &[a, b, c, d]).unwrap();
    lib.added_at(id, &[d, b, a, c]);
    let (before, order) = (lib.state(), lib.ids(id));
    assert_eq!(order, [d, b, a, c]);
    lib.take_out(id, &[b, c]).unwrap();
    assert_eq!(lib.ids(id), [d, a]);
    lib.undo();
    assert_eq!(lib.state(), before);
    assert_eq!(lib.ids(id), order);
}

#[test]
fn deleting_a_crate_with_entries_undoes_whole() {
    let lib = Lib::new();
    let (a, b, c) = (lib.track("A"), lib.track("B"), lib.track("C"));
    let first = lib.made("First");
    let id = lib.made("Warm up");
    lib.made("Last");
    lib.add(first, &[c]).unwrap();
    lib.add(id, &[a, b, c]).unwrap();
    lib.added_at(id, &[c, a, b]);
    let (before, order) = (lib.state(), lib.ids(id));
    lib.delete(id).unwrap();
    assert_eq!(
        lib.crates(),
        [("First".to_owned(), 1), ("Last".to_owned(), 0)]
    );
    lib.undo();
    assert_eq!(lib.state(), before);
    assert_eq!(lib.ids(id), order);
    assert_eq!(
        lib.crates(),
        [
            ("First".to_owned(), 1),
            ("Warm up".to_owned(), 3),
            ("Last".to_owned(), 0)
        ]
    );
}

#[test]
fn deleting_an_empty_crate_undoes_too() {
    let lib = Lib::new();
    let id = lib.made("Empty");
    let before = lib.state();
    lib.delete(id).unwrap();
    lib.undo();
    assert_eq!(lib.state(), before);
}

#[test]
fn undo_is_refused_and_changes_nothing_once_the_crate_has_changed_since() {
    let lib = Lib::new();
    let id = lib.made("Warm up");
    lib.rename(id, "Warm up (new)").unwrap();
    // Changed outside the log, after the rename.
    lib.run(
        "UPDATE crate SET name = 'Someone else' WHERE id = ?1",
        (id.0,),
    );
    let before = lib.state();
    assert!(matches!(
        undo_last_via(&lib.writer).unwrap(),
        UndoOutcome::Refused { .. }
    ));
    assert_eq!(lib.state(), before);
}

// ---- the Library and a send ----------------------------------------------------

#[test]
fn removing_a_library_track_that_sits_in_a_crate_then_undoing_puts_the_entry_back() {
    let lib = Lib::new();
    let (a, b) = (lib.track("A"), lib.track("B"));
    let id = lib.made("Warm up");
    lib.add(id, &[a, b]).unwrap();
    lib.added_at(id, &[b, a]);
    let before = lib.state();

    library::remove(&lib.writer, a).unwrap();
    assert_eq!(lib.ids(id), [b]);
    assert_eq!(lib.crates(), [("Warm up".to_owned(), 1)]);

    lib.undo();
    assert_eq!(lib.state(), before);
    assert_eq!(lib.ids(id), [b, a]);
    assert_eq!(lib.crates(), [("Warm up".to_owned(), 2)]);
}

#[test]
fn a_crate_made_filled_and_renamed_is_sent_under_its_new_name_and_the_old_one_is_stale() {
    let lib = Lib::new();
    let a = lib.track("A");
    let id = lib.made("Warm up");
    lib.add(id, &[a]).unwrap();

    // A send with the crate as it is, recorded; rekordbox then shows it.
    let tree = lib.writer.call(|c| crate_tree(c)).unwrap();
    let sent = build(&SendInput {
        crates: tree,
        ..SendInput::default()
    })
    .unwrap();
    let paths = sent.paths().to_vec();
    lib.writer
        .call(move |c| record_send(c, &[], &paths))
        .unwrap();
    lib.writer
        .call(|c| {
            save_tree(
                c,
                &RekordboxTree {
                    crates: vec![TreeNode::playlist("Warm up")],
                    playlists: vec![],
                },
            )
        })
        .unwrap();
    let stale = |lib: &Lib| -> Vec<String> {
        lib.writer
            .call(|c| lists(c, &NoVolumes))
            .unwrap()
            .stale_playlists
            .iter()
            .map(|s| s.path.join(" / "))
            .collect()
    };
    assert_eq!(stale(&lib), Vec::<String>::new());

    lib.rename(id, "Warm up (new)").unwrap();

    // The next send carries the new name with the same track...
    let tree = lib.writer.call(|c| crate_tree(c)).unwrap();
    assert_eq!(
        tree,
        [Node::Playlist {
            name: "Warm up (new)".to_owned(),
            entries: vec![a]
        }]
    );
    // ...and the old name, which rekordbox still has, is listed to delete.
    assert_eq!(stale(&lib), ["Crates / Warm up"]);
}

// ---- over IPC --------------------------------------------------------------------

mod ipc {
    use serde_json::json;
    use tauri::Manager;

    use crate::db::Writer;
    use crate::ipc::testing::{app, invoke};

    #[test]
    fn the_frontend_makes_fills_lists_renames_and_deletes_a_crate_and_undoes_it() {
        let (_data, app) = app();
        let writer = app.state::<Writer>();
        let library_track = writer
            .call(|c| {
                c.execute("INSERT INTO recording (title, artist) VALUES ('Tune', 'Kit')", [])?;
                let recording = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO library_track (recording_id, source_status) VALUES (?1, 'missing')",
                    [recording],
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap();

        let id = invoke(&app, "create_crate", json!({ "name": " Warm up " })).unwrap();
        let added = invoke(
            &app,
            "add_tracks_to_crate",
            json!({ "id": id, "tracks": [library_track] }),
        )
        .unwrap();
        assert_eq!(added["changed"], json!(1));
        assert_eq!(added["skipped"], json!(0));
        assert!(added["operationId"].is_number());

        let listed = invoke(&app, "list_crates", json!({})).unwrap();
        assert_eq!(
            listed,
            json!([{ "id": id, "parentId": null, "kind": "static", "name": "Warm up", "trackCount": 1 }])
        );
        let tracks = invoke(&app, "crate_tracks", json!({ "id": id })).unwrap();
        assert_eq!(tracks[0]["title"], json!("Tune"));
        assert_eq!(tracks[0]["id"], json!(library_track));

        // Renaming to the name it has records nothing, and says so.
        let same = invoke(&app, "rename_crate", json!({ "id": id, "name": "Warm up" })).unwrap();
        assert_eq!(same, json!(null));
        let renamed = invoke(
            &app,
            "rename_crate",
            json!({ "id": id, "name": "Warm up 2" }),
        )
        .unwrap();
        assert!(renamed.is_number());
        invoke(
            &app,
            "remove_tracks_from_crate",
            json!({ "id": id, "tracks": [library_track] }),
        )
        .unwrap();
        invoke(&app, "delete_crate", json!({ "id": id })).unwrap();
        assert_eq!(invoke(&app, "list_crates", json!({})).unwrap(), json!([]));

        let undone = invoke(&app, "undo_last_operation", json!({})).unwrap();
        assert_eq!(undone["status"], json!("undone"));
        assert_eq!(
            invoke(&app, "list_crates", json!({})).unwrap(),
            json!([{ "id": id, "parentId": null, "kind": "static", "name": "Warm up 2", "trackCount": 0 }])
        );
    }

    #[test]
    fn the_frontend_makes_folders_moves_and_deletes_them_and_hears_the_refusals() {
        let (_data, app) = app();
        let folder = invoke(
            &app,
            "create_crate_folder",
            json!({ "name": " Sets ", "parent": null }),
        )
        .unwrap();
        let inner = invoke(
            &app,
            "create_crate_folder",
            json!({ "name": "Late", "parent": folder }),
        )
        .unwrap();
        let crate_ = invoke(
            &app,
            "create_crate",
            json!({ "name": "Peak", "parent": inner }),
        )
        .unwrap();
        let loose = invoke(
            &app,
            "create_crate",
            json!({ "name": "Loose", "parent": null }),
        )
        .unwrap();

        let listed = invoke(&app, "list_crates", json!({})).unwrap();
        let shape: Vec<_> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["name"].clone(), c["kind"].clone(), c["parentId"].clone()))
            .collect();
        assert_eq!(
            shape,
            [
                (json!("Sets"), json!("folder"), json!(null)),
                (json!("Loose"), json!("static"), json!(null)),
                (json!("Late"), json!("folder"), folder.clone()),
                (json!("Peak"), json!("static"), inner.clone()),
            ]
        );

        let held = invoke(&app, "crate_folder_contents", json!({ "id": folder })).unwrap();
        assert_eq!(held, json!({ "crates": 1, "folders": 1, "tracks": 0 }));
        let not_a_folder =
            invoke(&app, "crate_folder_contents", json!({ "id": loose })).unwrap_err();
        assert_eq!(not_a_folder["kind"], json!("crateNotFound"));

        // Refusals reach the frontend as kinds.
        let folder_taken = invoke(
            &app,
            "create_crate_folder",
            json!({ "name": "SETS", "parent": null }),
        )
        .unwrap_err();
        assert_eq!(
            folder_taken,
            json!({ "kind": "folderNameTaken", "params": { "name": "Sets" } })
        );
        let folder_empty = invoke(
            &app,
            "create_crate_folder",
            json!({ "name": " ", "parent": null }),
        )
        .unwrap_err();
        assert_eq!(folder_empty["kind"], json!("folderNameEmpty"));
        let folder_gone =
            invoke(&app, "move_crate", json!({ "id": folder, "folder": 999 })).unwrap_err();
        assert_eq!(folder_gone["kind"], json!("folderNotFound"));
        let cycle =
            invoke(&app, "move_crate", json!({ "id": folder, "folder": inner })).unwrap_err();
        assert_eq!(cycle["kind"], json!("crateMoveIntoItself"));
        let into_crate =
            invoke(&app, "move_crate", json!({ "id": loose, "folder": crate_ })).unwrap_err();
        assert_eq!(into_crate["kind"], json!("crateNotAFolder"));
        let taken = invoke(
            &app,
            "create_crate",
            json!({ "name": "PEAK", "parent": inner }),
        )
        .unwrap_err();
        assert_eq!(
            taken,
            json!({ "kind": "crateNameTaken", "params": { "name": "Peak" } })
        );

        // A move answers its operation, and nothing when it changes nothing.
        let moved = invoke(&app, "move_crate", json!({ "id": loose, "folder": folder })).unwrap();
        assert!(moved.is_number());
        let again = invoke(&app, "move_crate", json!({ "id": loose, "folder": folder })).unwrap();
        assert_eq!(again, json!(null));
        let undone = invoke(&app, "undo_last_operation", json!({})).unwrap();
        assert_eq!(undone["status"], json!("undone"));
        assert_eq!(undone["operation"]["kind"], json!("move_crate"));

        // Deleting the folder takes what is in it; one undo brings it all back.
        invoke(&app, "delete_crate", json!({ "id": folder })).unwrap();
        assert_eq!(
            invoke(&app, "list_crates", json!({}))
                .unwrap()
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        let undone = invoke(&app, "undo_last_operation", json!({})).unwrap();
        assert_eq!(undone["operation"]["kind"], json!("delete_crate_folder"));
        assert_eq!(
            invoke(&app, "list_crates", json!({}))
                .unwrap()
                .as_array()
                .map(Vec::len),
            Some(4)
        );
    }

    #[test]
    fn a_refused_name_reaches_the_frontend_as_a_kind_with_the_existing_name() {
        let (_data, app) = app();
        invoke(&app, "create_crate", json!({ "name": "Warm up" })).unwrap();
        let taken = invoke(&app, "create_crate", json!({ "name": "WARM UP " })).unwrap_err();
        assert_eq!(
            taken,
            json!({ "kind": "crateNameTaken", "params": { "name": "Warm up" } })
        );
        let empty = invoke(&app, "create_crate", json!({ "name": "  " })).unwrap_err();
        assert_eq!(empty["kind"], json!("crateNameEmpty"));
        let missing = invoke(&app, "crate_tracks", json!({ "id": 99 })).unwrap_err();
        assert_eq!(missing["kind"], json!("crateNotFound"));
        assert_eq!(
            invoke(&app, "list_crates", json!({}))
                .unwrap()
                .as_array()
                .map(Vec::len),
            Some(1)
        );
    }
}

#[test]
fn a_crates_tracks_show_what_a_send_would_write_in_the_order_of_the_crate() {
    let lib = Lib::new();
    lib.run(
        "INSERT INTO volume (identity, kind, last_mount_path) VALUES (?1, 'external', 'E:\\')",
        (crate::volume::identity(crate::volume::IdentitySignals {
            kind: crate::volume::VolumeKind::External,
            unc_share: None,
            serial: Some(0x1A2B_3C4D),
            filesystem: "NTFS",
            guid: None,
        })
        .unwrap()
        .as_str()
        .to_owned(),),
    );
    lib.run(
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Music', 'Music')",
        (),
    );
    // Library tracks with no title of their own, each with a file.
    let untitled = |name: &'static str, tags: Option<(&'static str, &'static str)>| {
        let recording = lib.run("INSERT INTO recording DEFAULT VALUES", ());
        let raw_tags = tags.map(|(title, artist)| {
            serde_json::json!({"id3v2": [
                {"key": "TIT2", "value": {"type": "text", "text": title}},
                {"key": "TPE1", "value": {"type": "text", "text": artist}},
            ]})
            .to_string()
        });
        let file = lib.run(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, raw_tags)
             VALUES (1, ?1, ?1, ?2)",
            (name, raw_tags),
        );
        lib.run(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
            (recording, file),
        );
        let track = lib.run(
            "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
             VALUES (?1, 'linked', ?2, 'ok')",
            (recording, file),
        );
        (LibraryTrackId(track), file)
    };
    let (known, known_file) = untitled("known.mp3", Some(("Tag of a known track", "Tag artist")));
    let (new, _) = untitled("new.mp3", Some(("Tag of a new track", "New artist")));
    let (bare, _) = untitled("Sub/bare.mp3", None);
    lib.run(
        "INSERT INTO rekordbox_track
             (attributes, location_key, read_at, file_id, relink_method, relink_probable)
         VALUES (?1, 'E:/MUSIC/KNOWN.MP3', '2026-09-30T10:00:00.000Z', ?2, 'path', 0)",
        (
            r#"{"TrackID":"7","Name":"Their title","Artist":"Their artist","Location":"file://localhost/E:/Music/known.mp3"}"#,
            known_file,
        ),
    );
    let id = lib.made("Warm up");
    lib.add(id, &[known, new, bare]).unwrap();
    lib.added_at(id, &[bare, known, new]);

    let (stored, located) = lib
        .writer
        .call(|conn| library::stored_with_locations(conn, &Plugged))
        .unwrap();
    let rows = ordered_tracks(
        &lib.ids(id),
        &stored,
        &located,
        &Plugged,
        &FragileDirs {
            downloads: None,
            temp: Vec::new(),
        },
    );
    let shown: Vec<_> = rows
        .iter()
        .map(|t| (t.title.as_deref(), t.artist.as_deref()))
        .collect();
    assert_eq!(
        shown,
        [
            // Neither a title nor a tag: the file's name, no artist.
            (Some("bare.mp3"), None),
            // rekordbox knows it: its name and artist, not the file's tags.
            (Some("Their title"), Some("Their artist")),
            (Some("Tag of a new track"), Some("New artist")),
        ]
    );
}

/// The volume the files of the crate tests are on, mounted at `E:\`.
struct Plugged;

impl Volumes for Plugged {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!(
            "{} isn't looked up in these tests",
            path.display()
        )))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        let mine = crate::volume::identity(crate::volume::IdentitySignals {
            kind: crate::volume::VolumeKind::External,
            unc_share: None,
            serial: Some(0x1A2B_3C4D),
            filesystem: "NTFS",
            guid: None,
        })
        .unwrap();
        (*id == mine).then(|| PathBuf::from(r"E:\"))
    }
}
