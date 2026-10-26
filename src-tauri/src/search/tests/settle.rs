//! Keeping the index current: what makes a track due, how its row is
//! written, and that the index is derived data.

use super::support::{Lib, Mount};
use crate::search::{file_name_text, rebuild, settle_page};

#[test]
fn a_files_name_adds_its_stem_without_folder_or_extension() {
    for (path, text) in [
        ("tune.mp3", "tune"),
        ("Sub/Deeper/tune.FLAC", "tune"),
        ("Sub/harbour lights (live).aiff", "harbour lights (live)"),
        ("Mr. Smith - Track.wav", "Mr. Smith - Track"),
        // Not an extension: no letter, too long, or nothing before it.
        ("Track v1.5", "Track v1.5"),
        ("Track.verylong", "Track.verylong"),
        (".mp3", ".mp3"),
        ("no extension", "no extension"),
        ("a.b.c.m4a", "a.b.c"),
    ] {
        assert_eq!(file_name_text(path), text, "{path}");
    }
}

#[test]
fn tracks_are_settled_a_page_at_a_time_until_none_is_due() {
    let lib = Lib::new();
    for n in 0..5 {
        lib.track(Some(&format!("Title {n}")), None, &format!("{n}.mp3"));
    }
    let mut pages = Vec::new();
    loop {
        let n = lib.writer.call(|c| settle_page(c, &Mount, 2)).unwrap();
        pages.push(n);
        if n == 0 {
            break;
        }
    }
    assert_eq!(pages, vec![2, 2, 1, 0]);
    assert!(!lib.pending());
    assert_eq!(lib.finds("title").len(), 5);
}

#[test]
fn a_track_is_never_left_without_a_row_and_not_due_when_a_settle_is_cut_short() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Old title"), None, "a.mp3");
    assert_eq!(lib.finds("old"), vec![track]);
    lib.tag(file, &[("Name", "New title")]);
    assert!(lib.pending());

    // The settle fails at its last step, after the row was replaced.
    lib.exec(
        "CREATE TRIGGER cut_short BEFORE DELETE ON search_due
         BEGIN SELECT RAISE(ABORT, 'cut short'); END",
    );
    let cut = lib.writer.call(|c| settle_page(c, &Mount, 10));
    assert!(cut.is_err());
    // Nothing of it was kept: the track is still due and still has its old
    // row, not a new one that isn't due or no row at all.
    assert!(lib.pending());
    assert_eq!(lib.found_as_is("old").unwrap().tracks, vec![track]);
    assert_eq!(lib.found_as_is("new").unwrap().tracks, Vec::<i64>::new());

    lib.exec("DROP TRIGGER cut_short");
    assert_eq!(lib.finds("new"), vec![track]);
    assert_eq!(lib.finds("old"), Vec::<i64>::new());
}

#[test]
fn a_file_going_missing_or_coming_back_makes_its_track_due() {
    let lib = Lib::new();
    let (_, file) = lib.track(Some("Here"), None, "a.mp3");
    lib.settle();
    assert!(!lib.pending());
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (file,));
    assert!(lib.pending());
    lib.settle();
    lib.insert("UPDATE file SET present = 1 WHERE id = ?1", (file,));
    assert!(lib.pending());
}

#[test]
fn a_track_joining_the_library_is_indexed_as_the_lists_then_show_it() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Tag title"), None, "a.mp3");
    lib.rekordbox_entry(file, &[("Name", "Their title")]);
    assert_eq!(lib.finds("their"), Vec::<i64>::new());
    // Add to Library: the track now shows rekordbox's title.
    lib.library(track, file);
    assert!(lib.pending());
    assert_eq!(lib.finds("their"), vec![track]);
    assert_eq!(lib.finds("tag"), Vec::<i64>::new());
}

#[test]
fn a_rekordbox_read_makes_every_library_track_due_and_no_other() {
    let lib = Lib::new();
    let (known, known_file) = lib.track(Some("Tag one"), None, "one.mp3");
    lib.library(known, known_file);
    lib.rekordbox_entry(known_file, &[("Name", "Their one")]);
    let (plain, _) = lib.track(Some("Plain two"), None, "two.mp3");
    lib.settle();
    assert_eq!(lib.finds("their"), vec![known]);

    // A read replaces the snapshot, with another title for the track.
    lib.exec("DELETE FROM rekordbox_track");
    let due: Vec<i64> = lib
        .writer
        .call(|c| {
            c.prepare("SELECT recording_id FROM search_due ORDER BY 1")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    assert_eq!(due, vec![-1], "one mark, until the job expands it");
    lib.rekordbox_entry(known_file, &[("Name", "Their newer one")]);
    lib.writer.call(|c| settle_page(c, &Mount, 100)).unwrap();
    assert_eq!(lib.found_as_is("newer").unwrap().tracks, vec![known]);
    assert_eq!(lib.found_as_is("plain").unwrap().tracks, vec![plain]);
    assert!(!lib.pending());
}

#[test]
fn a_relink_that_moves_rekordbox_s_entry_to_another_file_updates_the_library_track() {
    let lib = Lib::new();
    let (track, file) = lib.track(Some("Tag title"), None, "a.mp3");
    lib.library(track, file);
    let entry = lib.rekordbox_entry(file, &[("Name", "Their title")]);
    assert_eq!(lib.finds("their"), vec![track]);
    // The match is dropped (the file is not rekordbox's any more).
    lib.exec("UPDATE rekordbox_track SET file_id = NULL, relink_method = NULL");
    let _ = entry;
    assert!(lib.pending());
    assert_eq!(lib.finds("their"), Vec::<i64>::new());
    assert_eq!(lib.finds("tag"), vec![track]);
}

#[test]
fn the_index_built_a_change_at_a_time_is_the_index_built_from_nothing() {
    let lib = Lib::new();
    let (one, one_file) = lib.track(Some("Alpha One"), Some("Artist A"), "Sub/one.mp3");
    lib.settle();
    let (two, two_file) = lib.track(None, None, "two (live).flac");
    lib.settle();
    let (three, three_file) = lib.track(Some("Caf\u{e9} Three"), None, "three.mp3");
    lib.tag(
        three_file,
        &[
            ("Name", "Caf\u{e9} Three"),
            ("Album", "Album Z"),
            ("Remixer", "Remix Y"),
            ("Label", "Label X"),
            ("Genre", "Deep House"),
            ("Comments", "late night"),
        ],
    );
    lib.library(three, three_file);
    lib.rekordbox_entry(three_file, &[("Name", "Their Three"), ("Genre", "Techno")]);
    lib.settle();
    lib.tag(one_file, &[("Name", "Alpha Uno"), ("Artist", "Artist A")]);
    lib.settle();
    let extra = lib.file(two, "two-copy.mp3", "extra");
    lib.insert(
        "UPDATE file SET hidden_reason = 'user' WHERE id = ?1",
        (extra,),
    );
    lib.settle();
    lib.insert(
        "UPDATE file SET rel_path = 'renamed.flac', rel_path_key = 'renamed.flac' WHERE id = ?1",
        (two_file,),
    );
    lib.settle();
    let (gone, _) = lib.track(Some("Short lived"), None, "gone.mp3");
    lib.settle();
    lib.insert(
        "DELETE FROM recording_file WHERE recording_id = ?1",
        (gone,),
    );
    lib.insert("DELETE FROM recording WHERE id = ?1", (gone,));
    let _ = (one, three);

    let incremental = lib.index_contents();
    assert!(incremental.len() > 10);
    // Wrong rows, as any drift would leave them.
    lib.exec("DELETE FROM search_track WHERE rowid = 1");
    lib.exec("INSERT INTO search_track (rowid, title) VALUES (99, 'stray')");

    lib.writer.call(|c| rebuild(c, &Mount, 3)).unwrap();
    assert_eq!(lib.index_contents(), incremental);
    assert!(!lib.pending());
    assert_eq!(lib.finds("stray"), Vec::<i64>::new());
}

#[test]
fn a_database_that_has_tracks_when_the_index_arrives_gets_a_row_for_each() {
    // Everything is due after an upgrade (the migration marks it): here the
    // index is emptied behind the triggers' back, as it is before the job's
    // first run (and `track_row` is a new table, so empty), and the job's
    // step alone brings it back.
    let lib = Lib::new();
    let (a, _) = lib.track(Some("First"), None, "a.mp3");
    let (b, _) = lib.track(Some("Second"), None, "b.mp3");
    lib.settle();
    lib.exec("INSERT INTO search_track (search_track) VALUES ('delete-all')");
    lib.exec("DELETE FROM track_row");
    lib.exec("INSERT INTO search_due (recording_id) SELECT id FROM recording");
    assert_eq!(lib.finds("first"), vec![a]);
    assert_eq!(lib.finds("second"), vec![b]);
}
