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
    _dir: tempfile::TempDir,
    writer: Writer,
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
            .call(|c| lists(c))
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
            json!([{ "id": id, "name": "Warm up", "trackCount": 1 }])
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
            json!([{ "id": id, "name": "Warm up 2", "trackCount": 0 }])
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
