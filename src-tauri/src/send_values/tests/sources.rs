//! Where each tag field of a track comes from, for the Details panel
//! (1cB-17; [`tag_sources`]). Everything is made up.

use super::*;

impl Lib {
    fn sources(&self, track: i64) -> TagSources {
        self.sources_of(track, None)
    }

    /// The sources, with the file the table shows for the track.
    fn sources_of(&self, track: i64, shown_file: Option<i64>) -> TagSources {
        self.writer
            .call(move |c| tag_sources(c, &plugged(), track, shown_file))
            .unwrap()
    }

    fn hide(&self, file: i64) {
        self.insert(
            "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
            (file,),
        );
    }
}

/// The field's entry in the answer.
fn field(sources: &TagSources, field: TagField) -> &FieldSources {
    sources
        .fields
        .iter()
        .find(|f| f.field == field)
        .unwrap_or_else(|| panic!("{field:?} is listed"))
}

/// A value as (text, file name or `None` for rekordbox's).
fn plain(value: &SourcedValue) -> (&str, Option<&str>) {
    (
        value.value.as_str(),
        value.file.as_ref().map(|f| f.name.as_str()),
    )
}

fn others(sources: &FieldSources) -> Vec<(&str, Option<&str>)> {
    sources.others.iter().map(plain).collect()
}

#[test]
fn a_track_rekordbox_knows_shows_rekordbox_s_value_and_lists_nothing_when_the_file_agrees() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(
        track,
        "a.mp3",
        "best",
        id3(&[("TIT2", "Same Title"), ("TPE1", "Same Artist")]),
    );
    lib.rekordbox(
        file,
        &[("Name", "Same Title"), ("Artist", "Same Artist")],
        false,
    );
    lib.library(track, Some(file));

    let sources = lib.sources(track);
    assert!(sources.in_rekordbox);
    let title = field(&sources, TagField::Title);
    let shown = title.shown.as_ref().unwrap();
    assert_eq!(shown.origin, ValueOrigin::RekordboxEntry);
    assert_eq!(plain(shown), ("Same Title", None));
    assert!(title.others.is_empty());
    assert!(field(&sources, TagField::Artist).others.is_empty());
}

#[test]
fn a_track_rekordbox_knows_lists_each_differing_file_tag_naming_its_file() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(track, "linked.mp3", "best", id3(&[("TIT2", "File Title")]));
    // An extra copy on disk with a third title, one that agrees with
    // rekordbox, and one that isn't on disk.
    lib.file(track, "copy.mp3", "extra", id3(&[("TIT2", "Copy Title")]));
    lib.file(
        track,
        "agrees.mp3",
        "extra",
        id3(&[("TIT2", "rekordbox Title")]),
    );
    lib.file(
        track,
        "gone.mp3",
        "extra",
        Spec {
            present: Some(false),
            ..id3(&[("TIT2", "Gone Title")])
        },
    );
    lib.rekordbox(linked, &[("Name", "rekordbox Title")], false);
    lib.library(track, Some(linked));

    let sources = lib.sources(track);
    let title = field(&sources, TagField::Title);
    assert_eq!(
        plain(title.shown.as_ref().unwrap()),
        ("rekordbox Title", None)
    );
    // The linked file first, then the others on disk; the one that agrees
    // and the one that is gone are not listed.
    assert_eq!(
        others(title),
        [
            ("File Title", Some("linked.mp3")),
            ("Copy Title", Some("copy.mp3"))
        ]
    );
    assert!(title
        .others
        .iter()
        .all(|o| o.origin == ValueOrigin::File && o.file.is_some()));
}

#[test]
fn a_field_rekordbox_leaves_blank_has_no_value_and_the_file_tag_is_listed_beside_it() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(
        track,
        "a.mp3",
        "best",
        id3(&[("TIT2", "T"), ("TCON", "House")]),
    );
    lib.rekordbox(file, &[("Name", "T"), ("Genre", "")], false);
    lib.library(track, Some(file));

    let sources = lib.sources(track);
    let genre = field(&sources, TagField::Genre);
    assert_eq!(genre.shown, None, "a file's tag doesn't stand in");
    assert_eq!(others(genre), [("House", Some("a.mp3"))]);
}

#[test]
fn a_library_track_rekordbox_does_not_know_shows_its_linked_file_first() {
    let lib = Lib::new();
    let track = lib.track();
    // The best file is not the linked one.
    lib.file(track, "best.mp3", "best", id3(&[("TIT2", "Best Title")]));
    let linked = lib.file(
        track,
        "linked.mp3",
        "extra",
        id3(&[("TIT2", "Linked Title")]),
    );
    lib.library(track, Some(linked));

    let sources = lib.sources(track);
    assert!(!sources.in_rekordbox);
    let title = field(&sources, TagField::Title);
    assert_eq!(
        plain(title.shown.as_ref().unwrap()),
        ("Linked Title", Some("linked.mp3"))
    );
    assert_eq!(others(title), [("Best Title", Some("best.mp3"))]);
}

#[test]
fn a_track_outside_the_library_whose_files_disagree_lists_each_file_s_value() {
    let lib = Lib::new();
    let track = lib.track();
    lib.file(
        track,
        "second.mp3",
        "extra",
        id3(&[("TIT2", "Other Title")]),
    );
    lib.file(track, "best.mp3", "best", id3(&[("TIT2", "Best Title")]));
    lib.file(track, "third.mp3", "extra", id3(&[("TIT2", "Best Title")]));

    let sources = lib.sources(track);
    assert!(!sources.in_rekordbox);
    assert_eq!(sources.confirmed_entry, None);
    let title = field(&sources, TagField::Title);
    // The best file is the one shown; the file that agrees isn't listed.
    assert_eq!(
        plain(title.shown.as_ref().unwrap()),
        ("Best Title", Some("best.mp3"))
    );
    assert_eq!(others(title), [("Other Title", Some("second.mp3"))]);
}

#[test]
fn a_track_with_one_file_shows_that_file_s_tags_and_only_the_fields_it_has() {
    let lib = Lib::new();
    let track = lib.track();
    lib.file(
        track,
        "only.mp3",
        "best",
        id3(&[("TIT2", "Title"), ("TPE1", "Artist"), ("TALB", "  ")]),
    );

    let sources = lib.sources(track);
    let listed: Vec<TagField> = sources.fields.iter().map(|f| f.field).collect();
    // A blank album and the fields the file lacks are not listed.
    assert_eq!(listed, [TagField::Title, TagField::Artist]);
    for f in &sources.fields {
        assert!(f.others.is_empty());
        let shown = f.shown.as_ref().unwrap();
        assert_eq!(shown.origin, ValueOrigin::File);
        assert_eq!(shown.file.as_ref().unwrap().name, "only.mp3");
    }
}

#[test]
fn a_track_that_does_not_exist_lists_nothing() {
    let lib = Lib::new();
    assert_eq!(lib.sources(9_999), TagSources::default());
}

#[test]
fn rekordbox_s_year_of_zero_is_no_year_and_the_file_s_year_is_listed() {
    let lib = Lib::new();
    let track = lib.track();
    let file = lib.file(
        track,
        "a.mp3",
        "best",
        id3(&[("TIT2", "T"), ("TDRC", "2019")]),
    );
    lib.rekordbox(file, &[("Name", "T"), ("Year", "0")], false);
    lib.library(track, Some(file));

    let sources = lib.sources(track);
    let year = field(&sources, TagField::Year);
    assert_eq!(year.shown, None, "0 is how rekordbox says it has no year");
    assert_eq!(others(year), [("2019", Some("a.mp3"))]);

    // A real year of rekordbox's is a value, as before.
    let track = lib.track();
    let file = lib.file(track, "b.mp3", "best", id3(&[("TDRC", "2019")]));
    lib.rekordbox(file, &[("Year", "2020")], false);
    lib.library(track, Some(file));
    let sources = lib.sources(track);
    let year = field(&sources, TagField::Year);
    assert_eq!(plain(year.shown.as_ref().unwrap()), ("2020", None));
}

#[test]
fn a_track_outside_the_library_reads_the_tags_of_the_file_the_table_shows() {
    let lib = Lib::new();
    let track = lib.track();
    // The best file is not the one the table shows (rekordbox's trusted
    // match is, `track_table`): the path and the tags must be one file's.
    let best = lib.file(track, "best.flac", "best", id3(&[("TIT2", "Best Title")]));
    let matched = lib.file(
        track,
        "matched.mp3",
        "extra",
        id3(&[("TIT2", "Matched Title")]),
    );
    lib.rekordbox(matched, &[("Name", "Theirs")], false);

    let sources = lib.sources_of(track, Some(matched));
    let title = field(&sources, TagField::Title);
    assert_eq!(
        plain(title.shown.as_ref().unwrap()),
        ("Matched Title", Some("matched.mp3"))
    );
    assert_eq!(others(title), [("Best Title", Some("best.flac"))]);

    // Asked without a file, or with one that isn't the track's: best first.
    for asked in [None, Some(best + 1_000)] {
        let sources = lib.sources_of(track, asked);
        let title = field(&sources, TagField::Title);
        assert_eq!(
            plain(title.shown.as_ref().unwrap()),
            ("Best Title", Some("best.flac"))
        );
    }
}

#[test]
fn a_hidden_or_gone_file_is_not_listed_beside_the_shown_one_for_a_library_track() {
    let lib = Lib::new();
    let track = lib.track();
    let linked = lib.file(
        track,
        "linked.mp3",
        "best",
        id3(&[("TIT2", "Linked Title")]),
    );
    let hidden = lib.file(
        track,
        "hidden.mp3",
        "extra",
        id3(&[("TIT2", "Hidden Title")]),
    );
    lib.file(track, "shown.mp3", "extra", id3(&[("TIT2", "Other Title")]));
    lib.file(
        track,
        "gone.mp3",
        "extra",
        Spec {
            present: Some(false),
            ..id3(&[("TIT2", "Gone Title")])
        },
    );
    lib.hide(hidden);
    lib.library(track, Some(linked));

    let sources = lib.sources(track);
    let title = field(&sources, TagField::Title);
    assert_eq!(
        plain(title.shown.as_ref().unwrap()),
        ("Linked Title", Some("linked.mp3"))
    );
    assert_eq!(others(title), [("Other Title", Some("shown.mp3"))]);
}

#[test]
fn a_hidden_or_gone_file_is_not_listed_beside_the_shown_one_for_a_track_outside_the_library() {
    let lib = Lib::new();
    let track = lib.track();
    lib.file(track, "best.mp3", "best", id3(&[("TIT2", "Best Title")]));
    let hidden = lib.file(
        track,
        "hidden.mp3",
        "extra",
        id3(&[("TIT2", "Hidden Title")]),
    );
    lib.file(track, "other.mp3", "extra", id3(&[("TIT2", "Other Title")]));
    lib.file(
        track,
        "gone.mp3",
        "extra",
        Spec {
            present: Some(false),
            ..id3(&[("TIT2", "Gone Title")])
        },
    );
    lib.hide(hidden);

    let sources = lib.sources(track);
    let title = field(&sources, TagField::Title);
    assert_eq!(others(title), [("Other Title", Some("other.mp3"))]);
}

#[test]
fn the_file_a_track_shows_is_read_even_when_it_is_gone_or_hidden_as_the_lists_do() {
    let lib = Lib::new();
    // Outside the Library, the only file hidden.
    let outside = lib.track();
    let only = lib.file(outside, "only.mp3", "best", id3(&[("TIT2", "Only Title")]));
    lib.hide(only);
    let sources = lib.sources(outside);
    assert_eq!(
        plain(field(&sources, TagField::Title).shown.as_ref().unwrap()),
        ("Only Title", Some("only.mp3"))
    );
    // A Library track whose linked file is gone.
    let track = lib.track();
    let linked = lib.file(
        track,
        "linked.mp3",
        "best",
        Spec {
            present: Some(false),
            ..id3(&[("TIT2", "Linked Title")])
        },
    );
    lib.library(track, Some(linked));
    let sources = lib.sources(track);
    assert_eq!(
        plain(field(&sources, TagField::Title).shown.as_ref().unwrap()),
        ("Linked Title", Some("linked.mp3"))
    );
}
