//! The app's own values of `Name`, `Artist`, `Album`, `Remixer`, `Label` and
//! `Year` (Run 6A, L1; ROADMAP 1.10 and 2.2): the lists, the search, the
//! table's sort keys and the Details panel show and use what the next send
//! would carry, as they already do for `Genre`. Every name is invented.

use super::support::{Lib, Mount};
use crate::conflicts::app_values;
use crate::library::LibraryTrackId;
use crate::ops::{self, UndoOutcome};
use crate::send_values::{TagField, ValueOrigin};

/// A Library track tagged "Tag title" by "Zed artist", its row worked out.
struct Track {
    lib: Lib,
    recording: i64,
    file: i64,
    library: LibraryTrackId,
}

fn track() -> Track {
    let lib = Lib::new();
    let (recording, file) = lib.track(Some("Tag title"), Some("Zed artist"), "a.mp3");
    lib.tag(
        file,
        &[
            ("Name", "Tag title"),
            ("Artist", "Zed artist"),
            ("Album", "Zed album"),
            ("Remixer", "Zed remixer"),
            ("Label", "Zed label"),
            ("Comments", "Remark one"),
            ("Composer", "Writer one"),
        ],
    );
    let library = LibraryTrackId(lib.library(recording, file));
    lib.settle();
    Track {
        lib,
        recording,
        file,
        library,
    }
}

impl Track {
    fn set(&self, field: &'static str, value: &'static str) {
        let library = self.library;
        self.lib
            .writer
            .call(move |c| Ok(app_values::set(c, &Mount, library, field, value)))
            .unwrap()
            .unwrap();
    }

    fn undo(&self) {
        let outcome = self
            .lib
            .writer
            .call(|c| Ok(ops::undo_for_user(c, None)))
            .unwrap()
            .unwrap();
        assert!(matches!(outcome, UndoOutcome::Undone { .. }), "{outcome:?}");
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

    /// What the track table shows for the track: title, artist.
    fn row(&self) -> (Option<String>, Option<String>) {
        let recording = self.recording;
        let row = self
            .lib
            .writer
            .call(move |c| crate::track_table::rows_of(c, &Mount, &[recording]))
            .unwrap()
            .pop()
            .unwrap();
        (row.title, row.artist)
    }

    /// The sort keys the table orders by: title, artist.
    fn keys(&self) -> (String, Option<String>) {
        let recording = self.recording;
        self.lib
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT title_key, artist_key FROM track_row WHERE recording_id = ?1",
                    [recording],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .unwrap()
    }

    /// What the Library list shows: title, artist.
    fn library_list(&self) -> (Option<String>, Option<String>) {
        let library = self.library;
        let tracks = self
            .lib
            .writer
            .call(move |c| crate::library::stored_these(c, &Mount, &[library]))
            .unwrap();
        (tracks[0].title.clone(), tracks[0].artist.clone())
    }

    /// The Details panel's value of `field` and where it comes from.
    fn details(&self, field: TagField) -> Option<(String, ValueOrigin)> {
        let recording = self.recording;
        let sources = self
            .lib
            .writer
            .call(move |c| crate::send_values::tag_sources(c, &Mount, recording, None))
            .unwrap();
        sources
            .fields
            .into_iter()
            .find(|f| f.field == field)
            .and_then(|f| f.shown)
            .map(|v| (v.value, v.origin))
    }
}

fn some(text: &str) -> Option<String> {
    Some(text.to_owned())
}

#[test]
fn an_own_name_shows_in_the_list_and_the_table_and_is_found_by_search() {
    let t = track();
    assert_eq!(t.lib.finds("Mine"), Vec::<i64>::new());

    t.set("Name", "Mine alone");

    assert!(t.due(), "setting an own value marks the track due");
    assert_eq!(t.lib.finds("Mine"), vec![t.recording]);
    assert_eq!(
        t.lib.finds("tag title"),
        Vec::<i64>::new(),
        "the file's title is not what the lists show any more"
    );
    assert_eq!(t.row().0, some("Mine alone"));
    assert_eq!(t.keys().0, "mine alone", "the table sorts by it");
    assert_eq!(t.library_list().0, some("Mine alone"));
    assert_eq!(
        t.details(TagField::Title),
        Some(("Mine alone".into(), ValueOrigin::Yours))
    );
}

#[test]
fn an_own_artist_album_remixer_and_label_show_and_are_found_by_search() {
    let t = track();
    t.set("Artist", "Mine artist");
    t.set("Album", "Mine album");
    t.set("Remixer", "Mine remixer");
    t.set("Label", "Mine label");

    for word in ["artist", "album", "remixer", "label"] {
        assert_eq!(
            t.lib.finds(&format!("mine {word}")),
            vec![t.recording],
            "{word}"
        );
        assert_eq!(
            t.lib.finds(&format!("zed {word}")),
            Vec::<i64>::new(),
            "{word}"
        );
    }
    assert_eq!(t.row().1, some("Mine artist"));
    assert_eq!(t.keys().1, some("mine artist"));
    assert_eq!(t.library_list().1, some("Mine artist"));
    for (field, value) in [
        (TagField::Artist, "Mine artist"),
        (TagField::Album, "Mine album"),
        (TagField::Remixer, "Mine remixer"),
        (TagField::Label, "Mine label"),
    ] {
        assert_eq!(
            t.details(field),
            Some((value.into(), ValueOrigin::Yours)),
            "{field:?}"
        );
    }
}

#[test]
fn an_own_year_is_the_year_the_details_panel_shows_first() {
    let t = track();
    t.lib.rekordbox_entry(t.file, &[("Year", "1999")]);
    assert_eq!(t.details(TagField::Year).unwrap().0, "1999");

    t.set("Year", "2003");

    assert_eq!(
        t.details(TagField::Year),
        Some(("2003".into(), ValueOrigin::Yours))
    );
}

#[test]
fn undo_brings_the_previous_value_back_after_the_index_refresh() {
    let t = track();
    t.lib.rekordbox_entry(t.file, &[("Name", "Their title")]);
    t.lib.settle();
    assert_eq!(t.row().0, some("Their title"));
    t.set("Name", "Mine alone");
    t.lib.settle();
    assert_eq!(t.row().0, some("Mine alone"));

    t.undo();

    assert!(t.due(), "the Undo marks the track due too");
    t.lib.settle();
    assert_eq!(t.row().0, some("Their title"));
    assert_eq!(t.keys().0, "their title");
    assert_eq!(t.library_list().0, some("Their title"));
    assert_eq!(t.lib.finds("mine"), Vec::<i64>::new());
    assert_eq!(t.lib.finds("their title"), vec![t.recording]);
    assert_eq!(
        t.details(TagField::Title).unwrap().1,
        ValueOrigin::RekordboxEntry
    );
}

#[test]
fn undo_with_no_earlier_value_shows_the_files_tag_again() {
    let t = track();
    t.set("Artist", "Mine artist");
    t.lib.settle();

    t.undo();
    t.lib.settle();

    assert_eq!(t.row().1, some("Zed artist"));
    assert_eq!(t.lib.finds("zed artist"), vec![t.recording]);
}

#[test]
fn a_track_without_own_values_shows_what_it_showed_and_is_not_marked_due() {
    let t = track();
    let (second, second_file) = t.lib.track(Some("Second"), Some("Other"), "b.mp3");
    t.lib.library(second, second_file);
    t.lib.settle();
    let table = |t: &Track| {
        t.lib
            .writer
            .call(move |c| {
                let row = crate::track_table::rows_of(c, &Mount, &[second])?.remove(0);
                let keys: (String, Option<String>) = c.query_row(
                    "SELECT title_key, artist_key FROM track_row WHERE recording_id = ?1",
                    [second],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                Ok((row.title, row.artist, keys))
            })
            .unwrap()
    };
    let before = table(&t);
    assert_eq!(before.0, some("Second"));

    t.set("Label", "Mine label");
    t.set("Name", "Mine alone");

    let marked: Vec<i64> = t
        .lib
        .writer
        .call(|c| {
            c.prepare("SELECT recording_id FROM search_due")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    assert_eq!(
        marked,
        vec![t.recording],
        "only the track whose value changed"
    );
    t.lib.settle();
    assert_eq!(table(&t), before);
}

#[test]
fn a_value_from_rekordbox_still_wins_over_the_file_tag_when_there_is_no_own_value() {
    let t = track();
    t.lib.rekordbox_entry(
        t.file,
        &[
            ("Name", "Their title"),
            ("Artist", "Their artist"),
            ("Album", "Their album"),
        ],
    );
    t.lib.settle();

    assert_eq!(t.row(), (some("Their title"), some("Their artist")));
    assert_eq!(t.lib.finds("their album"), vec![t.recording]);
    assert_eq!(t.lib.finds("tag album"), Vec::<i64>::new());

    // An own value beats rekordbox's, and only for the field it is for.
    t.set("Album", "Mine album");
    assert_eq!(t.lib.finds("mine album"), vec![t.recording]);
    assert_eq!(t.row(), (some("Their title"), some("Their artist")));
}

#[test]
fn a_blank_own_artist_shows_no_artist_and_a_blank_own_name_shows_the_files_name() {
    let t = track();
    t.set("Artist", " ");
    t.set("Name", " ");
    t.lib.settle();

    assert_eq!(t.row().1, None);
    assert_eq!(t.keys().1, None, "blank sorts last");
    let (title, _) = t.row();
    assert_eq!(
        title,
        some("a.mp3"),
        "the file's name stands in for no title"
    );
    assert_eq!(t.lib.finds("tag title"), Vec::<i64>::new());
}

#[test]
fn an_own_name_counts_as_a_title_for_the_overviews_missing_tags() {
    let lib = Lib::new();
    let (recording, file) = lib.track(None, Some("Someone"), "untitled.mp3");
    lib.tag(file, &[("Artist", "Someone"), ("Genre", "House")]);
    let library = LibraryTrackId(lib.library(recording, file));
    let missing = |lib: &Lib, ask_all: bool| {
        lib.writer
            .call(move |c| crate::overview::missing_tag_ids(c, &Mount, ask_all, None))
            .unwrap()
    };
    assert!(missing(&lib, false).contains(&library.0));

    lib.writer
        .call(move |c| Ok(app_values::set(c, &Mount, library, "Name", "Mine alone")))
        .unwrap()
        .unwrap();

    // The short way and the way that asks about every track agree.
    assert!(!missing(&lib, false).contains(&library.0));
    assert_eq!(missing(&lib, false), missing(&lib, true));
    let shown = lib
        .writer
        .call(move |c| crate::send_values::shown_tags(c, &Mount, &[library]))
        .unwrap();
    assert_eq!(shown[&library].title, "Mine alone");
}

// ---- Comments and Composer (6B F2) ---------------------------------------------------

#[test]
fn an_own_comments_shows_in_the_details_and_is_found_by_search() {
    let t = track();
    assert_eq!(t.lib.finds("remark one"), vec![t.recording]);

    t.set("Comments", "Mine comment");

    assert!(t.due(), "setting an own Comments marks the track due");
    assert_eq!(t.lib.finds("mine comment"), vec![t.recording]);
    assert_eq!(
        t.lib.finds("remark one"),
        Vec::<i64>::new(),
        "the file's comment is not what the search holds any more"
    );
    assert_eq!(
        t.details(TagField::Comment),
        Some(("Mine comment".into(), ValueOrigin::Yours))
    );
}

#[test]
fn a_blank_own_comments_shows_no_comment_and_finds_nothing() {
    let t = track();
    t.set("Comments", " ");
    t.lib.settle();

    assert_eq!(t.lib.finds("remark one"), Vec::<i64>::new());
    assert_eq!(t.details(TagField::Comment), None);
}

#[test]
fn a_track_without_own_comments_is_found_by_its_comment_and_is_not_marked_due() {
    let t = track();
    let (second, second_file) = t.lib.track(Some("Second"), Some("Other"), "b.mp3");
    t.lib.tag(second_file, &[("Comments", "Second comment")]);
    t.lib.library(second, second_file);
    t.lib.settle();

    t.set("Comments", "Mine comment");

    let marked: Vec<i64> = t
        .lib
        .writer
        .call(|c| {
            c.prepare("SELECT recording_id FROM search_due")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    assert_eq!(
        marked,
        vec![t.recording],
        "only the track whose value changed"
    );
    t.lib.settle();
    assert_eq!(t.lib.finds("second comment"), vec![second]);
    assert_eq!(t.lib.finds("mine comment"), vec![t.recording]);
}

#[test]
fn an_own_composer_is_the_composer_the_details_panel_shows_and_it_is_not_searched() {
    let t = track();
    assert_eq!(
        t.details(TagField::Composer),
        Some(("Writer one".into(), ValueOrigin::File))
    );

    t.set("Composer", "Mine composer");

    assert_eq!(
        t.details(TagField::Composer),
        Some(("Mine composer".into(), ValueOrigin::Yours))
    );
    for word in ["writer one", "mine composer"] {
        assert_eq!(t.lib.finds(word), Vec::<i64>::new(), "{word}");
    }
}
