//! "Use" on a genre guess (ROADMAP 3.8, owner decisions 2026-11-08; 5B B1):
//! the track's genre set as the app's own value of `Genre`, one Undo, and
//! the genre the lists, the index, the canonical genre and Review's
//! Unmapped genres show. Every name and value is invented. The send carrying
//! it is proved in `send::tests::merge` (the "Use" section).

use super::*;
use crate::conflicts::app_values;
use crate::ops::{self, UndoOutcome};
use crate::search::tests::support::{Lib, Mount};

/// A Library track whose file is tagged `tag`, its row worked out.
struct Track {
    lib: Lib,
    recording: i64,
    library: LibraryTrackId,
    file: i64,
}

fn track(tag: Option<&str>) -> Track {
    let lib = Lib::new();
    let (recording, file) = lib.track(Some("Title"), Some("Artist"), "a.mp3");
    if let Some(genre) = tag {
        lib.tag(file, &[("Name", "Title"), ("Genre", genre)]);
    }
    let library = LibraryTrackId(lib.library(recording, file));
    lib.settle();
    Track {
        lib,
        recording,
        library,
        file,
    }
}

impl Track {
    /// The user presses "Use" on a guess named `name`.
    fn use_genre(&self, name: &str) -> Result<Option<i64>, SetGenreError> {
        use_genre(&self.lib, self.library, name)
    }

    fn undo(&self) -> UndoOutcome {
        self.lib
            .writer
            .call(|c| Ok(ops::undo_for_user(c, None)))
            .unwrap()
            .unwrap()
    }

    /// The store's `Genre` row for the track: id, value, replaced.
    fn own(&self) -> Option<(i64, String, Option<String>)> {
        let library = self.library.0;
        self.lib
            .writer
            .call(move |c| {
                c.prepare(
                    "SELECT id, value, replaced FROM app_value
                     WHERE library_track_id = ?1 AND field = 'Genre'",
                )?
                .query_map([library], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .pop()
    }

    fn due(&self) -> bool {
        let recording = self.recording;
        self.lib
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM search_due WHERE recording_id = ?1)",
                    [recording],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// The genre the track table's row (and so the Details panel) shows.
    fn shown(&self) -> Option<String> {
        let recording = self.recording;
        self.lib
            .writer
            .call(move |c| crate::track_table::rows_of(c, &Mount, &[recording]))
            .unwrap()
            .pop()
            .and_then(|row| row.genre)
    }

    /// The genre the index keeps for the Overview and the doors, and its
    /// canonical node.
    fn indexed(&self) -> (Option<String>, Option<i64>) {
        let recording = self.recording;
        self.lib
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT genre, genre_node FROM track_row WHERE recording_id = ?1",
                    [recording],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .unwrap()
    }

    /// The change marked the track due, and once the index has run the
    /// table, the index and the Overview's definition all show `genre`.
    fn follows(&self, what: &str, genre: Option<&str>) {
        assert!(self.due(), "{what}: the track is not marked due");
        self.lib.settle();
        assert_eq!(self.shown().as_deref(), genre, "{what}: the table");
        assert_eq!(self.indexed().0.as_deref(), genre, "{what}: the index");
        let library = self.library;
        let defined = self
            .lib
            .writer
            .call(move |c| crate::overview::genres_of(c, &[library]))
            .unwrap()
            .remove(&library.0);
        assert_eq!(defined.as_deref(), genre, "{what}: the Overview's genre");
    }

    fn operations(&self) -> Vec<(String, String)> {
        self.lib
            .writer
            .call(|c| {
                c.prepare("SELECT kind, details FROM operation ORDER BY id")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .unwrap()
    }
}

fn use_genre(lib: &Lib, library: LibraryTrackId, name: &str) -> Result<Option<i64>, SetGenreError> {
    let name = name.to_owned();
    lib.writer
        .call(move |c| Ok(set_genre_on(c, &Mount, library, &name)))
        .unwrap()
}

// ---- Setting it ---------------------------------------------------------------

#[test]
fn use_sets_the_genre_and_the_table_details_index_and_overview_show_it_over_the_files_tag() {
    let t = track(Some("House"));
    assert_eq!(t.shown().as_deref(), Some("House"));

    t.use_genre("Techno").unwrap();

    t.follows("Use", Some("Techno"));
    // The file is never touched: its tag still says House.
    let file = t.file;
    let tags: String = t
        .lib
        .writer
        .call(move |c| {
            c.query_row("SELECT raw_tags FROM file WHERE id = ?1", [file], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert!(tags.contains("House") && !tags.contains("Techno"), "{tags}");
}

#[test]
fn the_details_panel_shows_the_genre_set_with_use_as_yours_with_the_one_it_replaced_under_it() {
    use crate::send_values::{tag_sources, FieldSources, TagField, ValueOrigin};
    let t = track(Some("House"));
    t.lib.trusted_entry(t.file, "Rekordbox Disco");
    t.use_genre("Techno").unwrap();

    let recording = t.recording;
    let genre: FieldSources = t
        .lib
        .writer
        .call(move |c| tag_sources(c, &Mount, recording, None))
        .unwrap()
        .fields
        .into_iter()
        .find(|f| f.field == TagField::Genre)
        .unwrap();

    let shown = genre.shown.unwrap();
    assert_eq!(
        (shown.value.as_str(), shown.origin, shown.file),
        ("Techno", ValueOrigin::Yours, None)
    );
    let others: Vec<(&str, ValueOrigin)> = genre
        .others
        .iter()
        .map(|o| (o.value.as_str(), o.origin))
        .collect();
    assert_eq!(
        others,
        [
            ("Rekordbox Disco", ValueOrigin::RekordboxEntry),
            ("House", ValueOrigin::File)
        ]
    );
}

#[test]
fn the_details_panel_lists_the_genre_field_in_its_place_when_only_use_gave_one() {
    use crate::send_values::{tag_sources, TagField, ValueOrigin};
    let t = track(None);
    t.use_genre("Ambient").unwrap();
    let recording = t.recording;
    let fields = t
        .lib
        .writer
        .call(move |c| tag_sources(c, &Mount, recording, None))
        .unwrap()
        .fields;
    let listed: Vec<TagField> = fields.iter().map(|f| f.field).collect();
    assert_eq!(listed, [TagField::Title, TagField::Artist, TagField::Genre]);
    let genre = fields[2].shown.as_ref().unwrap();
    assert_eq!(
        (genre.value.as_str(), genre.origin),
        ("Ambient", ValueOrigin::Yours)
    );
    assert!(fields[2].others.is_empty());
}

#[test]
fn use_on_a_track_with_no_genre_gives_it_one() {
    let t = track(None);
    assert_eq!(t.shown(), None);
    t.use_genre("Ambient").unwrap();
    t.follows("Use", Some("Ambient"));
}

#[test]
fn use_wins_over_rekordboxs_genre_and_records_rekordboxs_as_the_value_it_replaced() {
    let t = track(Some("Tag House"));
    t.lib.trusted_entry(t.file, "Rekordbox Disco");
    t.lib.settle();
    assert_eq!(t.shown().as_deref(), Some("Rekordbox Disco"));

    t.use_genre("Techno").unwrap();

    t.follows("Use over rekordbox's genre", Some("Techno"));
    let (_, value, replaced) = t.own().unwrap();
    assert_eq!(
        (value.as_str(), replaced.as_deref()),
        ("Techno", Some("Rekordbox Disco"))
    );
}

#[test]
fn use_on_a_track_rekordbox_does_not_know_is_stored_with_no_replaced_value() {
    let t = track(Some("House"));
    t.use_genre("Techno").unwrap();
    let (_, value, replaced) = t.own().unwrap();
    assert_eq!((value.as_str(), replaced), ("Techno", None));
}

#[test]
fn use_is_one_operation_of_kind_set_genre_that_names_the_genre_for_the_undo_button() {
    let t = track(Some("House"));
    let operation = t.use_genre("Techno").unwrap();
    assert!(operation.is_some());

    let operations = t.operations();
    assert_eq!(operations.len(), 1, "{operations:?}");
    assert_eq!(operations[0].0, SET_GENRE_OPERATION);
    let next = t
        .lib
        .writer
        .call(|c| Ok(ops::next_undo(c)))
        .unwrap()
        .unwrap();
    let info = next.operation.unwrap();
    assert_eq!(Some(info.id), operation);
    assert_eq!(info.kind, "set_genre");
    assert_eq!(info.details.name.as_deref(), Some("Techno"));
}

#[test]
fn use_trims_the_name_and_using_the_same_name_again_records_nothing() {
    let t = track(Some("House"));
    t.use_genre("  Techno ").unwrap();
    assert_eq!(t.own().unwrap().1, "Techno");
    t.lib.settle();

    assert_eq!(t.use_genre("Techno").unwrap(), None);
    assert_eq!(t.operations().len(), 1);
    assert!(!t.due(), "nothing changed, so nothing is due");
}

#[test]
fn use_of_the_genre_rekordbox_shows_leaves_the_field_following_rekordbox() {
    let t = track(Some("Tag House"));
    t.lib.trusted_entry(t.file, "Disco");
    t.use_genre("Techno").unwrap();
    t.use_genre("Disco").unwrap();
    assert_eq!(
        t.own(),
        None,
        "the store's rule: rekordbox's own value clears it"
    );
    t.follows("back to rekordbox's", Some("Disco"));
}

#[test]
fn use_refuses_a_track_that_is_not_in_the_library_and_changes_nothing() {
    let lib = Lib::new();
    let (recording, file) = lib.track(Some("Title"), None, "out.mp3");
    lib.tag(file, &[("Name", "Title"), ("Genre", "House")]);
    lib.settle();

    let refused = use_genre(&lib, LibraryTrackId(recording), "Techno");

    assert!(
        matches!(
            refused,
            Err(SetGenreError::Store(AppValueError::NoTrack(_)))
        ),
        "{refused:?}"
    );
    let written: i64 = lib
        .writer
        .call(|c| {
            c.query_row(
                "SELECT (SELECT count(*) FROM operation) + (SELECT count(*) FROM app_value)
                      + (SELECT count(*) FROM search_due)",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(written, 0);
}

#[test]
fn use_refuses_a_blank_name_and_changes_nothing() {
    let t = track(Some("House"));
    assert!(matches!(t.use_genre("   "), Err(SetGenreError::NameEmpty)));
    assert_eq!(t.own(), None);
    assert!(t.operations().is_empty());
}

#[test]
fn use_refuses_a_name_rekordbox_cant_take_as_its_own_refusal_and_changes_nothing() {
    use crate::ipc::{ErrorKind, IpcError};
    let t = track(Some("House"));

    let refused = t.use_genre("Tech\u{1}no").unwrap_err();

    assert!(
        matches!(
            refused,
            SetGenreError::Store(AppValueError::BadValue { .. })
        ),
        "{refused:?}"
    );
    assert_eq!(
        IpcError::from(refused).kind(),
        ErrorKind::GenreNameNotSendable,
        "a refusal the user can read, not an internal error"
    );
    assert_eq!(t.own(), None);
    assert!(t.operations().is_empty());
    assert!(!t.due());
}

#[test]
fn every_refusal_of_use_reaches_the_screen_as_its_own_kind() {
    use crate::ipc::{ErrorKind, IpcError};
    let kind = |e: SetGenreError| IpcError::from(e).kind();
    assert_eq!(kind(SetGenreError::NameEmpty), ErrorKind::GenreNameEmpty);
    assert_eq!(
        kind(SetGenreError::Store(AppValueError::NoTrack(
            LibraryTrackId(9)
        ))),
        ErrorKind::LibraryTrackNotFound
    );
    assert_eq!(
        kind(SetGenreError::Store(AppValueError::BadValue {
            field: GENRE_FIELD.into(),
            value: "\u{1}".into(),
        })),
        ErrorKind::GenreNameNotSendable
    );
}

// ---- Undo -----------------------------------------------------------------------

#[test]
fn undo_of_use_with_no_earlier_own_genre_leaves_none_and_the_files_genre_shows_again() {
    let t = track(Some("House"));
    t.use_genre("Techno").unwrap();
    t.lib.settle();

    match t.undo() {
        UndoOutcome::Undone { operation } => assert_eq!(operation.kind, SET_GENRE_OPERATION),
        other => panic!("expected the Use undone, got {other:?}"),
    }

    assert_eq!(t.own(), None);
    t.follows("Undo", Some("House"));
}

#[test]
fn undo_of_use_puts_the_earlier_own_genre_back_exactly() {
    let t = track(Some("Tag House"));
    t.lib.trusted_entry(t.file, "Rekordbox Disco");
    // An earlier edit of the same field in the same store (as Review →
    // Conflicts' "Edit" or a merge leaves one).
    let library = t.library;
    t.lib
        .writer
        .call(move |c| Ok(app_values::set(c, &Mount, library, "Genre", "Deep House")))
        .unwrap()
        .unwrap();
    let earlier = t.own().unwrap();
    t.use_genre("Techno").unwrap();
    t.lib.settle();
    assert_eq!(t.shown().as_deref(), Some("Techno"));

    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));

    assert_eq!(t.own(), Some(earlier), "same row, value and replaced value");
    t.follows("Undo", Some("Deep House"));
}

#[test]
fn undo_of_a_use_that_cleared_the_own_genre_puts_it_back() {
    let t = track(Some("Tag House"));
    t.lib.trusted_entry(t.file, "Disco");
    t.use_genre("Techno").unwrap();
    let earlier = t.own().unwrap();
    t.use_genre("Disco").unwrap();
    t.lib.settle();

    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));

    assert_eq!(t.own(), Some(earlier));
    t.follows("Undo", Some("Techno"));
}

// ---- The genre tree -------------------------------------------------------------

#[test]
fn a_name_the_genre_tree_knows_gets_its_node_and_one_it_does_not_lands_in_unmapped() {
    let t = track(Some("House"));
    let node = t
        .lib
        .writer
        .call(|c| Ok(crate::genre::edit::add_node(c, None, "Techno")))
        .unwrap()
        .unwrap()
        .unwrap();
    t.lib.settle();

    t.use_genre("Techno").unwrap();
    t.lib.settle();
    assert_eq!(t.indexed(), (Some("Techno".into()), Some(node)));

    t.use_genre("Zouk Bass").unwrap();
    t.follows("an unknown name", Some("Zouk Bass"));
    assert_eq!(t.indexed().1, None);
    let unmapped = t
        .lib
        .writer
        .call(|c| crate::genre::unmapped::unmapped(c))
        .unwrap();
    let spellings: Vec<(&str, u32)> = unmapped
        .spellings
        .iter()
        .map(|s| (s.spelling.as_str(), s.tracks))
        .collect();
    assert_eq!(spellings, [("Zouk Bass", 1)]);
}

// ---- Keeping the index current ---------------------------------------------------

#[test]
fn any_operation_that_changes_a_tracks_own_genre_marks_it_due_and_so_does_its_undo() {
    let t = track(Some("House"));
    let library = t.library;
    // The store's own operations, not "Use": a resolve and a merge write
    // the same rows the same way.
    t.lib
        .writer
        .call(move |c| Ok(app_values::set(c, &Mount, library, "Genre", "Disco")))
        .unwrap()
        .unwrap();
    t.follows("an edit of the store", Some("Disco"));
    t.lib
        .writer
        .call(move |c| Ok(app_values::clear(c, library, "Genre")))
        .unwrap()
        .unwrap();
    t.follows("the edit cleared", Some("House"));
    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    t.follows("the clear undone", Some("Disco"));
    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    t.follows("the edit undone", Some("House"));
}

#[test]
fn an_operation_on_another_field_of_the_store_marks_nothing_due() {
    let t = track(Some("House"));
    let library = t.library;
    t.lib
        .writer
        .call(move |c| Ok(app_values::set(c, &Mount, library, "Comments", "warm up")))
        .unwrap()
        .unwrap();
    assert!(!t.due());
    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    assert!(!t.due());
}

#[test]
fn an_operation_marks_only_the_tracks_whose_own_genre_it_changed() {
    let t = track(Some("House"));
    let (other_recording, other_file) = t.lib.track(Some("Other"), None, "b.mp3");
    let other = LibraryTrackId(t.lib.library(other_recording, other_file));
    t.lib.settle();

    use_genre(&t.lib, other, "Techno").unwrap();

    assert!(!t.due(), "the first track's genre didn't change");
    let other_due: bool = t
        .lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT EXISTS (SELECT 1 FROM search_due WHERE recording_id = ?1)",
                [other_recording],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert!(other_due);
}
