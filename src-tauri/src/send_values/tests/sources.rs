//! Where each tag field of a track comes from, for the Details panel
//! (1cB-17; [`tag_sources`]). Everything is made up.

use super::*;

impl Lib {
    fn sources(&self, track: i64) -> TagSources {
        self.writer
            .call(move |c| tag_sources(c, &plugged(), track))
            .unwrap()
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
