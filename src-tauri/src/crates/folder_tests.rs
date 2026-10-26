//! Crate folder tests (1cA-9/10): the tree, the sibling-name rule inside a
//! folder, deleting a folder with what is in it, moving, and Undo of each.
//! Everything is synthetic, like the crate tests this builds on.

use super::*;

impl Lib {
    fn folder(&self, parent: Option<CrateId>, name: &str) -> Done<CrateId> {
        let name = name.to_owned();
        self.writer
            .call(move |c| Ok(create_folder_on(c, parent, &name)))
            .unwrap()
            .unwrap()
    }

    fn folder_ok(&self, parent: Option<CrateId>, name: &str) -> CrateId {
        self.folder(parent, name).unwrap()
    }

    fn crate_in(&self, parent: Option<CrateId>, name: &str) -> Done<CrateId> {
        let name = name.to_owned();
        self.writer
            .call(move |c| Ok(create_in(c, parent, &name)))
            .unwrap()
            .unwrap()
    }

    fn crate_in_ok(&self, parent: Option<CrateId>, name: &str) -> CrateId {
        self.crate_in(parent, name).unwrap()
    }

    fn move_to(&self, id: CrateId, folder: Option<CrateId>) -> Done<Option<i64>> {
        self.writer
            .call(move |c| Ok(move_on(c, id, folder)))
            .unwrap()
            .unwrap()
    }

    fn contents(&self, id: CrateId) -> Option<FolderContents> {
        self.writer.call(move |c| folder_contents(c, id)).unwrap()
    }

    /// The tree as the sidebar draws it: "Folder/Crate" paths in list
    /// order, folders marked with a trailing slash.
    fn paths(&self) -> Vec<String> {
        let all = self.writer.call(|c| list(c)).unwrap();
        fn walk(all: &[Crate], parent: Option<CrateId>, prefix: &str, out: &mut Vec<String>) {
            for c in all.iter().filter(|c| c.parent_id == parent) {
                let path = format!("{prefix}{}", c.name);
                if c.kind == CrateKind::Folder {
                    out.push(format!("{path}/"));
                    walk(all, Some(c.id), &format!("{path}/"), out);
                } else {
                    out.push(path);
                }
            }
        }
        let mut out = Vec::new();
        walk(&all, None, "", &mut out);
        out
    }

    fn last_kind(&self) -> String {
        self.writer
            .call(|c| {
                c.query_row(
                    "SELECT kind FROM operation ORDER BY id DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }
}

fn why<T: std::fmt::Debug>(refused: Done<T>) -> &'static str {
    match refused {
        Err(CratesError::NotFound) => "not found",
        Err(CratesError::NameEmpty) => "empty",
        Err(CratesError::NameTaken { .. }) => "taken",
        Err(CratesError::NameUnsendable) => "unsendable",
        Err(CratesError::NotAFolder) => "not a folder",
        Err(CratesError::MoveIntoItself) => "into itself",
        Err(CratesError::TooDeep) => "too deep",
        other => panic!("not refused: {other:?}"),
    }
}

// ---- the tree ---------------------------------------------------------------------

#[test]
fn folders_and_crates_nest_and_are_listed_in_each_folders_own_order() {
    let lib = Lib::new();
    let friday = lib.folder_ok(None, "Friday residency");
    lib.crate_in_ok(Some(friday), "Warmup");
    let late = lib.folder_ok(Some(friday), "Late");
    lib.crate_in_ok(Some(late), "Peak time");
    lib.crate_in_ok(Some(friday), "Closing");
    lib.made("Loose");
    assert_eq!(
        lib.paths(),
        [
            "Friday residency/",
            "Friday residency/Warmup",
            "Friday residency/Late/",
            "Friday residency/Late/Peak time",
            "Friday residency/Closing",
            "Loose"
        ]
    );
    let all = lib.writer.call(|c| list(c)).unwrap();
    assert_eq!(all[0].kind, CrateKind::Folder);
    assert_eq!(all[0].parent_id, None);
    assert!(all
        .iter()
        .filter(|c| c.name == "Warmup")
        .all(|c| c.parent_id == Some(friday)));
}

#[test]
fn a_folder_holds_crates_and_folders_never_tracks() {
    let lib = Lib::new();
    let a = lib.track("A");
    let folder = lib.folder_ok(None, "Sets");
    assert!(lib.add(folder, &[a]).is_err());
    assert!(lib
        .writer
        .call(move |c| track_ids(c, folder))
        .unwrap()
        .is_none());
    // A crate is not a place to put anything.
    let crate_ = lib.made("Warm up");
    assert_eq!(why(lib.crate_in(Some(crate_), "Inside")), "not a folder");
    assert_eq!(why(lib.folder(Some(crate_), "Inside")), "not a folder");
    assert_eq!(why(lib.crate_in(Some(CrateId(999)), "Inside")), "not found");
}

#[test]
fn the_same_name_in_two_folders_is_fine_but_not_twice_in_one() {
    let lib = Lib::new();
    let (a, b) = (lib.folder_ok(None, "A"), lib.folder_ok(None, "B"));
    lib.crate_in_ok(Some(a), "Warmup");
    lib.crate_in_ok(Some(b), "Warmup");
    lib.made("Warmup");
    // Crates and folders share one set of names.
    for name in ["warmup", "WARMUP  ", "Warmup"] {
        assert_eq!(why(lib.crate_in(Some(a), name)), "taken", "{name:?}");
        assert_eq!(why(lib.folder(Some(a), name)), "taken", "{name:?}");
        assert_eq!(why(lib.folder(None, name)), "taken", "{name:?}");
    }
    lib.crate_in_ok(Some(b), "Caf\u{e9}");
    assert_eq!(why(lib.folder(Some(b), "Cafe\u{301}")), "taken");
}

#[test]
fn a_folder_name_is_trimmed_and_refused_when_empty_or_unsendable() {
    let lib = Lib::new();
    let id = lib.folder_ok(None, "  Sets \t");
    assert_eq!(lib.paths(), ["Sets/"]);
    for name in ["", "  ", "\t"] {
        assert_eq!(why(lib.folder(None, name)), "empty");
        assert_eq!(why(lib.rename(id, name)), "empty");
    }
    assert_eq!(why(lib.folder(None, "a\u{1}b")), "unsendable");
    assert_eq!(why(lib.rename(id, "a\u{1}b")), "unsendable");
    assert_eq!(lib.paths(), ["Sets/"]);
}

#[test]
fn renaming_compares_with_the_others_in_the_same_folder_only() {
    let lib = Lib::new();
    let (a, b) = (lib.folder_ok(None, "A"), lib.folder_ok(None, "B"));
    let one = lib.crate_in_ok(Some(a), "One");
    lib.crate_in_ok(Some(a), "Two");
    let elsewhere = lib.crate_in_ok(Some(b), "Two");
    assert_eq!(why(lib.rename(one, "two ")), "taken");
    // The same name in another folder is fine, and so is another letter case
    // for its own name.
    lib.rename(one, "ONE").unwrap();
    lib.rename(elsewhere, "One").unwrap();
    assert_eq!(why(lib.rename(b, "a")), "taken");
    lib.rename(a, "Friday").unwrap();
    assert_eq!(
        lib.paths(),
        ["Friday/", "Friday/ONE", "Friday/Two", "B/", "B/One"]
    );
}

#[test]
fn a_folder_rename_is_undone_exactly_and_named_as_a_folder_rename() {
    let lib = Lib::new();
    let id = lib.folder_ok(None, "Sets");
    lib.crate_in_ok(Some(id), "Warmup");
    let before = lib.state();
    assert!(lib.rename(id, "Friday").unwrap().is_some());
    assert_eq!(lib.last_kind(), RENAME_FOLDER_OPERATION);
    lib.undo();
    assert_eq!(lib.state(), before);
}

#[test]
fn making_a_folder_or_a_crate_in_a_folder_is_undone_exactly() {
    let lib = Lib::new();
    let sets = lib.folder_ok(None, "Sets");
    let before = lib.state();
    lib.folder_ok(Some(sets), "Late");
    assert_eq!(lib.last_kind(), CREATE_FOLDER_OPERATION);
    lib.undo();
    assert_eq!(lib.state(), before);
    lib.crate_in_ok(Some(sets), "Warmup");
    assert_eq!(lib.last_kind(), CREATE_OPERATION);
    lib.undo();
    assert_eq!(lib.state(), before);
}

#[test]
fn a_refusal_records_nothing_and_changes_nothing() {
    let lib = Lib::new();
    let a = lib.folder_ok(None, "A");
    let inner = lib.folder_ok(Some(a), "Inner");
    let crate_ = lib.crate_in_ok(Some(inner), "Warmup");
    let (state, operations) = (lib.state(), lib.operations());
    let _ = lib.folder(Some(a), "inner");
    let _ = lib.folder(Some(crate_), "x");
    let _ = lib.move_to(a, Some(a));
    let _ = lib.move_to(a, Some(inner));
    let _ = lib.move_to(a, Some(crate_));
    let _ = lib.move_to(crate_, Some(CrateId(999)));
    let _ = lib.move_to(CrateId(999), None);
    let _ = lib.delete(CrateId(999));
    assert_eq!((lib.state(), lib.operations()), (state, operations));
}

// ---- deleting a folder ------------------------------------------------------------

/// A tree with a few crates that hold tracks, siblings in a deliberate
/// order, and ids and positions that are not simply increasing.
struct Tree {
    folder: CrateId,
    tracks: [LibraryTrackId; 3],
}

fn tree(lib: &Lib) -> Tree {
    let tracks = [lib.track("A"), lib.track("B"), lib.track("C")];
    lib.made("Before");
    let folder = lib.folder_ok(None, "Friday residency");
    lib.made("After");
    let warm = lib.crate_in_ok(Some(folder), "Warmup");
    let late = lib.folder_ok(Some(folder), "Late");
    let peak = lib.crate_in_ok(Some(late), "Peak time");
    let deep = lib.folder_ok(Some(late), "Deeper");
    let deepest = lib.crate_in_ok(Some(deep), "Deepest");
    lib.folder_ok(Some(late), "Empty folder");
    let closing = lib.crate_in_ok(Some(folder), "Closing");
    lib.add(warm, &[tracks[0], tracks[1]]).unwrap();
    lib.add(peak, &[tracks[1], tracks[2]]).unwrap();
    lib.add(deepest, &[tracks[2]]).unwrap();
    lib.add(closing, &[tracks[0]]).unwrap();
    lib.added_at(warm, &[tracks[1], tracks[0]]);
    lib.added_at(peak, &[tracks[2], tracks[1]]);
    // Positions that don't match the ids: reorder the siblings.
    lib.run("UPDATE crate SET position = 7 WHERE id = ?1", (warm.0,));
    lib.run("UPDATE crate SET position = 3 WHERE id = ?1", (closing.0,));
    lib.run("UPDATE crate SET position = 5 WHERE id = ?1", (late.0,));
    Tree { folder, tracks }
}

#[test]
fn a_folder_says_what_deleting_it_would_delete() {
    let lib = Lib::new();
    let t = tree(&lib);
    assert_eq!(
        lib.contents(t.folder),
        Some(FolderContents {
            crates: 4,
            folders: 3,
            tracks: 3
        })
    );
    let empty = lib.folder_ok(None, "Nothing");
    assert_eq!(
        lib.contents(empty),
        Some(FolderContents {
            crates: 0,
            folders: 0,
            tracks: 0
        })
    );
    // A crate or nothing at all is not a folder.
    let plain = lib.made("Plain");
    assert_eq!(lib.contents(plain), None);
    assert_eq!(lib.contents(CrateId(999)), None);
}

#[test]
fn deleting_a_folder_deletes_all_it_holds_in_one_operation_and_leaves_the_tracks() {
    let lib = Lib::new();
    let t = tree(&lib);
    let operations = lib.operations();
    lib.delete(t.folder).unwrap();
    assert_eq!(lib.operations(), operations + 1);
    assert_eq!(lib.last_kind(), DELETE_FOLDER_OPERATION);
    assert_eq!(lib.paths(), ["Before", "After"]);
    let (entries, tracks): (i64, i64) = lib
        .writer
        .call(|c| {
            Ok((
                c.query_row("SELECT count(*) FROM crate_entry", [], |r| r.get(0))?,
                c.query_row("SELECT count(*) FROM library_track", [], |r| r.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!((entries, tracks), (0, 3));
}

#[test]
fn undoing_a_folder_delete_restores_every_row_position_and_entry_order_exactly() {
    let lib = Lib::new();
    let t = tree(&lib);
    let (before, paths) = (lib.state(), lib.paths());
    let orders: Vec<_> = lib
        .writer
        .call(|c| {
            Ok(dump(
                c,
                "SELECT crate_id, library_track_id, added_at FROM crate_entry \
                 ORDER BY crate_id, added_at, library_track_id",
            ))
        })
        .unwrap();
    lib.delete(t.folder).unwrap();
    lib.undo();
    assert_eq!(lib.state(), before);
    assert_eq!(lib.paths(), paths);
    // The order of siblings is the stored positions, not the ids.
    assert_eq!(
        paths,
        [
            "Before",
            "Friday residency/",
            "Friday residency/Closing",
            "Friday residency/Late/",
            "Friday residency/Late/Peak time",
            "Friday residency/Late/Deeper/",
            "Friday residency/Late/Deeper/Deepest",
            "Friday residency/Late/Empty folder/",
            "Friday residency/Warmup",
            "After"
        ]
    );
    let after: Vec<_> = lib
        .writer
        .call(|c| {
            Ok(dump(
                c,
                "SELECT crate_id, library_track_id, added_at FROM crate_entry \
                 ORDER BY crate_id, added_at, library_track_id",
            ))
        })
        .unwrap();
    assert_eq!(after, orders);
    let _ = t.tracks;
}

#[test]
fn deleting_a_crate_in_a_folder_and_an_empty_folder_both_undo() {
    let lib = Lib::new();
    let t = tree(&lib);
    let before = lib.state();
    let warm = lib
        .writer
        .call(|c| {
            c.query_row("SELECT id FROM crate WHERE name = 'Warmup'", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    lib.delete(CrateId(warm)).unwrap();
    assert_eq!(lib.last_kind(), DELETE_OPERATION);
    lib.undo();
    assert_eq!(lib.state(), before);
    let empty = lib
        .writer
        .call(|c| {
            c.query_row(
                "SELECT id FROM crate WHERE name = 'Empty folder'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    lib.delete(CrateId(empty)).unwrap();
    lib.undo();
    assert_eq!(lib.state(), before);
    let _ = t;
}

#[test]
fn undoing_a_folder_delete_is_refused_when_a_name_it_brings_back_is_taken() {
    let lib = Lib::new();
    let sets = lib.folder_ok(None, "Sets");
    lib.crate_in_ok(Some(sets), "Warmup");
    lib.delete(sets).unwrap();
    // Made outside the log (the log's own history can't do this).
    lib.run(
        "INSERT INTO crate (kind, name, position) VALUES ('folder', 'SETS', 0)",
        (),
    );
    let before = lib.state();
    assert!(matches!(
        undo_last_via(&lib.writer).unwrap(),
        UndoOutcome::Refused { .. }
    ));
    assert_eq!(lib.state(), before);
}

// ---- moving -----------------------------------------------------------------------

#[test]
fn a_crate_moves_into_a_folder_last_among_its_siblings_and_to_the_top() {
    let lib = Lib::new();
    let sets = lib.folder_ok(None, "Sets");
    lib.crate_in_ok(Some(sets), "One");
    let loose = lib.made("Loose");
    let top = lib.made("Top");
    assert!(lib.move_to(loose, Some(sets)).unwrap().is_some());
    assert_eq!(lib.last_kind(), MOVE_OPERATION);
    assert_eq!(lib.paths(), ["Sets/", "Sets/One", "Sets/Loose", "Top"]);
    assert!(lib.move_to(loose, None).unwrap().is_some());
    assert_eq!(lib.paths(), ["Sets/", "Sets/One", "Top", "Loose"]);
    let _ = top;
}

#[test]
fn a_folder_moves_with_everything_in_it() {
    let lib = Lib::new();
    let t = tree(&lib);
    let target = lib.folder_ok(None, "Archive");
    let before = lib.paths();
    lib.move_to(t.folder, Some(target)).unwrap();
    let moved = lib.paths();
    assert_eq!(moved.len(), before.len());
    assert!(moved.contains(&"Archive/Friday residency/Late/Deeper/Deepest".to_owned()));
    assert_eq!(lib.contents(t.folder).unwrap().tracks, 3);
}

#[test]
fn undoing_a_move_puts_the_crate_back_exactly() {
    let lib = Lib::new();
    let t = tree(&lib);
    let target = lib.folder_ok(None, "Archive");
    let before = lib.state();
    lib.move_to(t.folder, Some(target)).unwrap();
    assert_ne!(lib.state(), before);
    lib.undo();
    assert_eq!(lib.state(), before);
    // A crate moved out to the top and back goes back to its own place.
    let warm: i64 = lib
        .writer
        .call(|c| {
            c.query_row("SELECT id FROM crate WHERE name = 'Warmup'", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    lib.move_to(CrateId(warm), None).unwrap();
    lib.undo();
    assert_eq!(lib.state(), before);
}

#[test]
fn a_move_to_where_it_is_records_nothing() {
    let lib = Lib::new();
    let sets = lib.folder_ok(None, "Sets");
    let one = lib.crate_in_ok(Some(sets), "One");
    lib.crate_in_ok(Some(sets), "Two");
    let top = lib.made("Top");
    let (state, operations) = (lib.state(), lib.operations());
    assert_eq!(lib.move_to(one, Some(sets)).unwrap(), None);
    assert_eq!(lib.move_to(top, None).unwrap(), None);
    assert_eq!((lib.state(), lib.operations()), (state, operations));
}

#[test]
fn a_folder_cannot_go_into_itself_or_its_own_subfolder() {
    let lib = Lib::new();
    let a = lib.folder_ok(None, "A");
    let b = lib.folder_ok(Some(a), "B");
    let c = lib.folder_ok(Some(b), "C");
    assert_eq!(why(lib.move_to(a, Some(a))), "into itself");
    assert_eq!(why(lib.move_to(a, Some(b))), "into itself");
    assert_eq!(why(lib.move_to(a, Some(c))), "into itself");
    assert_eq!(why(lib.move_to(b, Some(c))), "into itself");
    // Going up or sideways is fine.
    let other = lib.folder_ok(None, "Other");
    lib.move_to(c, Some(other)).unwrap();
    lib.move_to(b, None).unwrap();
    assert_eq!(lib.paths(), ["A/", "Other/", "Other/C/", "B/"]);
}

#[test]
fn nothing_goes_into_a_crate_or_a_folder_that_is_not_there() {
    let lib = Lib::new();
    let sets = lib.folder_ok(None, "Sets");
    let one = lib.made("One");
    let two = lib.made("Two");
    assert_eq!(why(lib.move_to(sets, Some(one))), "not a folder");
    assert_eq!(why(lib.move_to(two, Some(one))), "not a folder");
    assert_eq!(why(lib.move_to(two, Some(CrateId(999)))), "not found");
    assert_eq!(why(lib.move_to(CrateId(999), Some(sets))), "not found");
}

#[test]
fn a_move_is_refused_when_the_folder_has_that_name_by_the_senders_rule() {
    let lib = Lib::new();
    let sets = lib.folder_ok(None, "Sets");
    lib.crate_in_ok(Some(sets), "Warmup");
    let loose = lib.made("WARMUP ");
    let other = lib.folder_ok(None, "Other");
    let folder = lib.folder_ok(Some(other), "warmup");
    assert_eq!(why(lib.move_to(loose, Some(sets))), "taken");
    assert_eq!(why(lib.move_to(folder, Some(sets))), "taken");
    // Elsewhere the same name is fine.
    lib.move_to(loose, Some(other)).map(|_| ()).unwrap_err();
    let spare = lib.folder_ok(None, "Spare");
    lib.move_to(loose, Some(spare)).unwrap();
}

#[test]
fn a_move_that_would_nest_deeper_than_a_send_can_write_is_refused() {
    let lib = Lib::new();
    // As many folders as a send writes, one inside the other, made straight
    // in the table. (The writer counts the `Crates` folder as the first level.)
    let mut parent: Option<i64> = None;
    let mut deepest = 0;
    for level in 0..MAX_DEPTH - 1 {
        deepest = lib.run(
            "INSERT INTO crate (parent_id, kind, name) VALUES (?1, 'folder', ?2)",
            (parent, format!("L{level}")),
        );
        parent = Some(deepest);
    }
    let deepest = CrateId(deepest);
    assert_eq!(why(lib.folder(Some(deepest), "One too many")), "too deep");
    // A crate fits in the deepest folder; a folder with a folder in it
    // doesn't fit anywhere near.
    lib.crate_in_ok(Some(deepest), "A crate");
    let pair = lib.folder_ok(None, "Pair");
    lib.folder_ok(Some(pair), "Inner");
    assert_eq!(why(lib.move_to(pair, Some(deepest))), "too deep");
    let single = lib.folder_ok(None, "Single");
    assert_eq!(why(lib.move_to(single, Some(deepest))), "too deep");
    // The tree a send writes is still fine.
    let tree = lib.writer.call(|c| crate_tree(c)).unwrap();
    assert!(build(&SendInput {
        crates: tree,
        ..SendInput::default()
    })
    .is_ok());
}

#[test]
fn a_move_undoes_only_while_the_names_fit() {
    let lib = Lib::new();
    let sets = lib.folder_ok(None, "Sets");
    let loose = lib.made("Warmup");
    lib.move_to(loose, Some(sets)).unwrap();
    // Someone else (outside the log) takes the name at the top.
    lib.run(
        "INSERT INTO crate (kind, name, position) VALUES ('static', 'WARMUP', 9)",
        (),
    );
    let before = lib.state();
    assert!(matches!(
        undo_last_via(&lib.writer).unwrap(),
        UndoOutcome::Refused { .. }
    ));
    assert_eq!(lib.state(), before);
}

// ---- what a send writes ------------------------------------------------------------

#[test]
fn the_tree_a_send_writes_follows_folders_moves_and_deletes() {
    let lib = Lib::new();
    let a = lib.track("A");
    let sets = lib.folder_ok(None, "Sets");
    let warm = lib.crate_in_ok(Some(sets), "Warmup");
    lib.add(warm, &[a]).unwrap();
    let late = lib.folder_ok(Some(sets), "Late");
    let peak = lib.crate_in_ok(Some(late), "Peak");
    lib.move_to(peak, Some(sets)).unwrap();
    lib.delete(late).unwrap();
    let tree = lib.writer.call(|c| crate_tree(c)).unwrap();
    assert_eq!(
        tree,
        [Node::Folder {
            name: "Sets".to_owned(),
            children: vec![
                Node::Playlist {
                    name: "Warmup".to_owned(),
                    entries: vec![a]
                },
                Node::Playlist {
                    name: "Peak".to_owned(),
                    entries: vec![]
                },
            ]
        }]
    );
    assert!(build(&SendInput {
        crates: tree,
        ..SendInput::default()
    })
    .is_ok());
}
