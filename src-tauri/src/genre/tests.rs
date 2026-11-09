//! The genre taxonomy (3A-G): the resolver and the rule it names, the edits
//! and their Undo, the seed, and Review's unmapped group. Genre names are
//! invented; no real tracks or artists.

use rusqlite::Connection;

use super::edit::{self, GenreError};
use super::unmapped;
use super::{fold, Alias, AliasKind, Catalog, Node, Resolution, Rule};
use crate::db::Writer;
use crate::ops::{undo_last_via, UndoOutcome};

fn node(id: i64, parent_id: Option<i64>, name: &str) -> Node {
    Node {
        id,
        parent_id,
        name: name.to_owned(),
    }
}

fn alias(genre_id: i64, alias: &str, kind: AliasKind) -> Alias {
    Alias {
        genre_id,
        alias: alias.to_owned(),
        kind,
    }
}

/// 1 Electronic > (2 House > 4 Deep), (3 Drum & Bass); 5 Latin > 6 Salsa;
/// and 7 Ambient under Electronic too.
fn catalog() -> Catalog {
    Catalog::new(
        &[
            node(1, None, "Electronic"),
            node(2, Some(1), "House"),
            node(3, Some(1), "Drum & Bass"),
            node(4, Some(2), "Deep"),
            node(5, None, "Latin"),
            node(6, Some(5), "Salsa"),
            node(7, Some(1), "Ambient"),
        ],
        &[
            alias(3, "DnB", AliasKind::Exact),
            alias(6, "Música Tropical", AliasKind::Folded),
        ],
    )
}

fn got(c: &Catalog, spelling: &str) -> Option<(i64, Rule)> {
    c.resolve(spelling).map(|r| (r.node, r.rule))
}

#[test]
fn a_spelling_resolves_by_the_first_rule_that_applies_and_names_it() {
    let c = catalog();
    assert_eq!(got(&c, "House"), Some((2, Rule::Exact)));
    assert_eq!(got(&c, "  House "), Some((2, Rule::Exact)), "trimmed");
    assert_eq!(got(&c, "house"), Some((2, Rule::Folded)));
    assert_eq!(got(&c, "HOUSE"), Some((2, Rule::Folded)));
    assert_eq!(got(&c, "drum & bass"), Some((3, Rule::Folded)));
    // Accents don't matter to a folded match, in either direction.
    assert_eq!(got(&c, "Hóuse"), Some((2, Rule::Folded)));
    assert_eq!(got(&c, "música tropical"), Some((6, Rule::Alias)));
    assert_eq!(got(&c, "Musica Tropical"), Some((6, Rule::Alias)));
}

#[test]
fn parent_sub_needs_the_sub_to_be_below_the_parent() {
    let c = catalog();
    assert_eq!(got(&c, "House (Deep)"), Some((4, Rule::ParentSub)));
    assert_eq!(got(&c, "house ( deep )"), Some((4, Rule::ParentSub)));
    // Any ancestor will do, not only the direct parent.
    assert_eq!(got(&c, "Electronic (Deep)"), Some((4, Rule::ParentSub)));
    assert_eq!(got(&c, "Latin (Deep)"), None, "Deep is not under Latin");
    assert_eq!(got(&c, "House (Salsa)"), None);
    assert_eq!(got(&c, "House (Unknown)"), None, "no guessing the parent");
    assert_eq!(got(&c, "Unknown (Deep)"), None);
    assert_eq!(got(&c, "(Deep)"), None);
    assert_eq!(got(&c, "House ()"), None);
}

#[test]
fn an_exact_alias_matches_only_the_same_text_and_a_miss_is_unmapped() {
    let c = catalog();
    assert_eq!(got(&c, "DnB"), Some((3, Rule::Alias)));
    assert_eq!(got(&c, "dnb"), None);
    assert_eq!(got(&c, ""), None);
    assert_eq!(got(&c, "   "), None);
    assert_eq!(got(&c, "Polka"), None);
    // An empty tree maps nothing.
    assert_eq!(Catalog::default().resolve("House"), None);
}

#[test]
fn a_loop_in_a_bad_database_does_not_hang_the_resolver() {
    let c = Catalog::new(
        &[
            node(1, Some(2), "A"),
            node(2, Some(1), "B"),
            node(3, None, "C"),
        ],
        &[],
    );
    assert_eq!(got(&c, "C (A)"), None);
}

// The edits, on a real database.

fn writer() -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    (dir, writer)
}

fn add(w: &Writer, parent: Option<i64>, name: &'static str) -> Result<i64, &'static str> {
    refused(
        w.call(move |c| Ok(edit::add_node(c, parent, name)))
            .unwrap()
            .unwrap(),
    )
}

fn refused<T>(done: edit::Done<T>) -> Result<T, &'static str> {
    done.map_err(|e| match e {
        GenreError::NotFound => "not found",
        GenreError::NameEmpty => "empty",
        GenreError::NameTaken { .. } => "taken",
        GenreError::MoveIntoItself => "into itself",
        GenreError::AliasTaken { .. } => "alias taken",
        GenreError::NameIsAlias { .. } => "name is alias",
        GenreError::TextTooLong => "too long",
        GenreError::Ops(_) => "ops",
    })
}

fn tree(w: &Writer) -> Vec<(i64, Option<i64>, String, i64)> {
    w.call(|c| {
        c.prepare("SELECT id, parent_id, name, position FROM genre ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect()
    })
    .unwrap()
}

fn undo(w: &Writer) {
    assert!(matches!(
        undo_last_via(w).unwrap(),
        UndoOutcome::Undone { .. }
    ));
}

#[test]
fn names_are_unique_across_the_tree_ignoring_case_and_accents() {
    let (_dir, w) = writer();
    let electronic = add(&w, None, "Electronic").unwrap();
    assert_eq!(add(&w, Some(electronic), "electrónic"), Err("taken"));
    assert_eq!(add(&w, None, "  "), Err("empty"));
    assert_eq!(add(&w, Some(999), "Rock"), Err("not found"));
    let house = add(&w, Some(electronic), "House").unwrap();
    let rock = add(&w, None, "Rock").unwrap();
    // Siblings take the next position.
    assert_eq!(
        tree(&w),
        [
            (electronic, None, "Electronic".into(), 0),
            (house, Some(electronic), "House".into(), 0),
            (rock, None, "Rock".into(), 1),
        ]
    );
    // Hostile text is only ever a value.
    let odd = "x'); DROP TABLE genre; --";
    let id = w
        .call(move |c| Ok(edit::add_node(c, None, odd)))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(tree(&w).iter().find(|n| n.0 == id).unwrap().2, odd);
}

#[test]
fn rename_and_move_are_undone_one_at_a_time() {
    let (_dir, w) = writer();
    let electronic = add(&w, None, "Electronic").unwrap();
    let rock = add(&w, None, "Rock").unwrap();
    let house = add(&w, Some(electronic), "House").unwrap();
    let start = tree(&w);

    let renamed = w
        .call(move |c| Ok(edit::rename_node(c, house, "Tech House")))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(renamed.is_some());
    let unchanged = w
        .call(move |c| Ok(edit::rename_node(c, house, "Tech House")))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(unchanged, None, "the same name records nothing");
    let taken = w
        .call(move |c| Ok(edit::rename_node(c, house, "rock")))
        .unwrap()
        .unwrap();
    assert_eq!(refused(taken), Err("taken"));
    let after_rename = tree(&w);

    let moved = w
        .call(move |c| Ok(edit::move_node(c, house, Some(rock))))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(moved.is_some());
    assert_eq!(tree(&w)[2].1, Some(rock));
    // Into itself and into its own descendant are refused.
    let into_self = w
        .call(move |c| Ok(edit::move_node(c, rock, Some(rock))))
        .unwrap()
        .unwrap();
    assert_eq!(refused(into_self), Err("into itself"));
    let into_child = w
        .call(move |c| Ok(edit::move_node(c, rock, Some(house))))
        .unwrap()
        .unwrap();
    assert_eq!(refused(into_child), Err("into itself"));

    undo(&w);
    assert_eq!(tree(&w), after_rename);
    undo(&w);
    assert_eq!(tree(&w), start);
}

#[test]
fn aliases_are_added_refused_removed_and_undone() {
    let (_dir, w) = writer();
    let house = add(&w, None, "House").unwrap();
    let rock = add(&w, None, "Rock").unwrap();
    let alias_of = |w: &Writer, genre, text: &'static str| {
        refused(
            w.call(move |c| Ok(edit::add_alias(c, genre, text, AliasKind::Folded)))
                .unwrap()
                .unwrap(),
        )
    };
    let id = alias_of(&w, house, "Four On The Floor").unwrap();
    // A spelling that already leads somewhere is refused, whatever its case.
    assert_eq!(alias_of(&w, rock, "four on the FLOOR"), Err("alias taken"));
    assert_eq!(alias_of(&w, rock, "house"), Err("alias taken"));
    assert_eq!(alias_of(&w, rock, " "), Err("empty"));
    assert_eq!(alias_of(&w, 999, "Anything"), Err("not found"));

    let catalog = |w: &Writer| w.call(|c| Catalog::load(c)).unwrap();
    assert_eq!(
        catalog(&w).resolve("four on the floor"),
        Some(Resolution {
            node: house,
            rule: Rule::Alias
        })
    );
    w.call(move |c| Ok(edit::remove_alias(c, id)))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(catalog(&w).resolve("four on the floor"), None);
    undo(&w);
    assert_eq!(
        catalog(&w).resolve("four on the floor").map(|r| r.node),
        Some(house)
    );
    undo(&w);
    assert_eq!(catalog(&w).resolve("four on the floor"), None);
}

#[test]
fn the_seed_is_one_undoable_operation_and_leaves_existing_genres_alone() {
    let (_dir, w) = writer();
    add(&w, None, "electronic").unwrap();
    let added = w
        .call(|c| Ok(edit::load_seed(c)))
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(added > 20, "a starter tree: {added}");
    let all = tree(&w);
    assert_eq!(all.len(), 1 + added as usize);
    // Every name is unique and every parent exists.
    let keys: std::collections::HashSet<String> = all.iter().map(|n| fold(&n.2)).collect();
    assert_eq!(keys.len(), all.len());
    let c = w.call(|c| Catalog::load(c)).unwrap();
    assert_eq!(got(&c, "Hip Hop").map(|g| g.1), Some(Rule::Alias));
    assert_eq!(got(&c, "Electronic").map(|g| g.1), Some(Rule::Folded));
    assert!(got(&c, "Electronic (House)").is_some());
    // The seed again adds nothing new.
    let again = w
        .call(|c| Ok(edit::load_seed(c)))
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(again, 0);
    undo(&w);
    assert_eq!(tree(&w).len(), 1, "undoing the seed leaves the first genre");
}

#[test]
fn the_seed_gives_the_audio_models_genre_names_their_nodes() {
    let (_dir, w) = writer();
    w.call(|c| Ok(edit::load_seed(c)))
        .unwrap()
        .unwrap()
        .unwrap();
    let c = w.call(|c| Catalog::load(c)).unwrap();
    let node_of = |name: &str| got(&c, name).map(|g| g.0);
    assert!(node_of("House").is_some());
    assert_eq!(node_of("House music"), node_of("House"));
    assert_eq!(node_of("Electronic dance"), node_of("Electronic"));
    assert_eq!(node_of("Electronic dance music"), node_of("Electronic"));
    assert_eq!(node_of("Rhythm and blues"), node_of("R&B"));
    assert_eq!(node_of("Disco"), node_of("Funk & Disco"));
    assert_eq!(got(&c, "Electronic dance").map(|g| g.1), Some(Rule::Alias));
}

// Review's unmapped group.

fn library_track(c: &Connection, id: i64, genre: Option<&str>, title: &str) {
    c.execute("INSERT INTO recording DEFAULT VALUES", [])
        .unwrap();
    c.execute(
        "INSERT INTO track_row (recording_id, flags, title_key, genre, genre_group)
         VALUES (?1, 1, ?2, ?3, lower(?3))",
        rusqlite::params![id, title, genre],
    )
    .unwrap();
}

#[test]
fn review_lists_tracks_with_no_genre_first_then_the_spellings_no_rule_maps() {
    let (_dir, w) = writer();
    let house = add(&w, None, "House").unwrap();
    w.call(|c| {
        library_track(c, 1, None, "untitled a");
        library_track(c, 2, Some("House"), "b");
        library_track(c, 3, Some("Hause"), "c");
        library_track(c, 4, Some("Hause"), "d");
        library_track(c, 5, Some("Polka"), "e");
        // Not a Library track: not counted.
        library_track(c, 6, Some("Waltz"), "f");
        c.execute("UPDATE track_row SET flags = 0 WHERE recording_id = 6", [])?;
        Ok(())
    })
    .unwrap();

    let q = w.call(|c| unmapped::unmapped(c)).unwrap();
    assert_eq!(q.no_genre, 1);
    assert_eq!(q.no_genre_sample.len(), 1);
    assert_eq!(q.no_genre_sample[0].recording_id, 1);
    let spellings: Vec<_> = q
        .spellings
        .iter()
        .map(|s| (s.spelling.as_str(), s.tracks))
        .collect();
    assert_eq!(spellings, [("Hause", 2), ("Polka", 1)]);

    // An alias takes a spelling out of the queue at once.
    w.call(move |c| Ok(edit::add_alias(c, house, "Hause", AliasKind::Exact)))
        .unwrap()
        .unwrap()
        .unwrap();
    let q = w.call(|c| unmapped::unmapped(c)).unwrap();
    assert_eq!(q.spellings.len(), 1);
    let all = w.call(|c| unmapped::spellings(c)).unwrap();
    let hause = all.iter().find(|s| s.spelling == "Hause").unwrap();
    assert_eq!(
        hause.resolution,
        Some(Resolution {
            node: house,
            rule: Rule::Alias
        })
    );
    let exact = all.iter().find(|s| s.spelling == "House").unwrap();
    assert_eq!(exact.resolution.map(|r| r.rule), Some(Rule::Exact));
}

fn alias_to(w: &Writer, genre: i64, text: &'static str) -> Result<i64, &'static str> {
    refused(
        w.call(move |c| Ok(edit::add_alias(c, genre, text, AliasKind::Folded)))
            .unwrap()
            .unwrap(),
    )
}

/// Empties the due list, runs `act`, and says whether tracks are due after.
fn due_after(w: &Writer, act: impl FnOnce(&Writer)) -> bool {
    w.call(|c| c.execute("DELETE FROM search_due", [])).unwrap();
    act(w);
    w.call(|c| {
        c.query_row(
            "SELECT count(*) FROM search_due WHERE recording_id = -1",
            [],
            |r| r.get::<_, i64>(0),
        )
    })
    .unwrap()
        == 1
}

#[test]
fn edits_that_change_resolution_mark_tracks_due_and_a_position_change_marks_none() {
    let (_dir, w) = writer();
    let a = add(&w, None, "Electronic").unwrap();
    let b = add(&w, None, "Rock").unwrap();
    let alias = alias_to(&w, a, "EDM").unwrap();
    // Insert.
    assert!(due_after(&w, |w| {
        add(w, None, "Jazz").unwrap();
    }));
    assert!(due_after(&w, |w| {
        alias_to(w, b, "Rawk").unwrap();
    }));
    // Update: a rename and a move.
    assert!(due_after(&w, |w| {
        w.call(move |c| Ok(edit::rename_node(c, b, "Hard Rock")))
            .unwrap()
            .unwrap()
            .unwrap();
    }));
    assert!(due_after(&w, |w| {
        w.call(move |c| Ok(edit::move_node(c, b, Some(a))))
            .unwrap()
            .unwrap()
            .unwrap();
    }));
    // Delete: an alias, and a node (through undoing its add).
    assert!(due_after(&w, |w| {
        w.call(move |c| Ok(edit::remove_alias(c, alias)))
            .unwrap()
            .unwrap()
            .unwrap();
    }));
    assert!(due_after(&w, |w| {
        add(w, None, "Folk").unwrap();
        w.call(|c| c.execute("DELETE FROM search_due", [])).unwrap();
        undo(w);
    }));
    // A change of position alone and a display-name-only change resolve nothing differently.
    assert!(!due_after(&w, |w| {
        w.call(move |c| {
            c.execute(
                "UPDATE genre SET position = position + 5 WHERE id = ?1",
                [a],
            )
        })
        .unwrap();
    }));
    assert!(!due_after(&w, |w| {
        w.call(move |c| c.execute("UPDATE genre SET name = 'ELECTRONIC' WHERE id = ?1", [a]))
            .unwrap();
    }));
}

#[test]
fn a_name_and_an_alias_cannot_share_a_spelling_and_text_is_capped() {
    let (_dir, w) = writer();
    let a = add(&w, None, "Electronic").unwrap();
    let b = add(&w, None, "Rock").unwrap();
    alias_to(&w, a, "EDM").unwrap();
    assert_eq!(add(&w, None, "edm"), Err("name is alias"));
    assert_eq!(
        refused(
            w.call(move |c| Ok(edit::rename_node(c, b, "Édm")))
                .unwrap()
                .unwrap()
        ),
        Err("name is alias")
    );
    assert_eq!(alias_to(&w, b, "rock"), Err("alias taken"));
    let long: &'static str = Box::leak("x".repeat(201).into_boxed_str());
    assert_eq!(add(&w, None, long), Err("too long"));
    assert_eq!(alias_to(&w, b, long), Err("too long"));
    let ok: &'static str = Box::leak("x".repeat(200).into_boxed_str());
    assert!(add(&w, None, ok).is_ok());
}
