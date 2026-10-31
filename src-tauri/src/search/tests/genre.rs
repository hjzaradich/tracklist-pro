//! The genre the Overview counts, stored in `track_row` (1cD-R1b; migration
//! 0044; ROADMAP 1.13 "What the numbers mean"). Every input of a track's
//! genre must reach its row once the index has run: the file's tags, the
//! linked file, rekordbox's entry (read, relinked, picked among several) and
//! a merge. Each test below makes a real change to the data and checks the
//! row after the index job against the Overview's own definition
//! (`door_answers_of`, which uses `genres_of`), and that the change marked
//! the track due.

use super::support::{Lib, Mount};
use crate::library::LibraryTrackId;
use crate::overview::door_answers_of;
use crate::search::rebuild;

/// A Library track with its file linked and tagged, and its row written.
struct Track {
    lib: Lib,
    recording: i64,
    library: i64,
    file: i64,
}

fn track(tag: Option<&str>) -> Track {
    let lib = Lib::new();
    let (recording, file) = lib.track(Some("Title"), Some("Artist"), "a.mp3");
    if let Some(genre) = tag {
        lib.tag(file, &[("Name", "Title"), ("Genre", genre)]);
    }
    let library = lib.library(recording, file);
    lib.settle();
    Track {
        lib,
        recording,
        library,
        file,
    }
}

impl Track {
    /// A trusted rekordbox entry for `file` with this genre.
    fn entry(&self, file: i64, genre: &str) -> i64 {
        self.lib.trusted_entry(file, genre)
    }

    /// The genre and its group, as the track's row holds them now.
    fn stored(&self) -> Option<(String, String)> {
        let recording = self.recording;
        self.lib
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT genre, genre_group FROM track_row WHERE recording_id = ?1",
                    [recording],
                    |r| {
                        Ok(r.get::<_, Option<String>>(0)?
                            .zip(r.get::<_, Option<String>>(1)?))
                    },
                )
            })
            .unwrap()
    }

    /// The genre the Overview's definition gives the track now.
    fn defined(&self) -> Option<String> {
        let library = self.library;
        self.lib
            .writer
            .call(move |c| door_answers_of(c, &[LibraryTrackId(library)]))
            .unwrap()
            .remove(&library)
            .and_then(|answer| answer.genre)
    }

    /// The track, or every Library track (a rekordbox read), is waiting for
    /// the index.
    fn due(&self) -> bool {
        let recording = self.recording;
        self.lib
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM search_due
                                    WHERE recording_id IN (?1, -1))",
                    [recording],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// The change has marked the track, and once the index has run its row
    /// holds `genre`, which is also what the definition says.
    fn follows(&self, what: &str, genre: Option<&str>) {
        assert!(self.due(), "{what}: the track is not marked due");
        self.lib.settle();
        assert_eq!(self.defined().as_deref(), genre, "{what}: the definition");
        assert_eq!(
            self.stored().map(|(genre, _)| genre).as_deref(),
            genre,
            "{what}: the row"
        );
    }
}

#[test]
fn a_library_tracks_row_holds_its_genre_as_spelled_and_the_same_in_lower_case() {
    let t = track(Some("  Deep House  "));
    assert_eq!(t.stored(), Some(("Deep House".into(), "deep house".into())));
    assert_eq!(t.defined().as_deref(), Some("Deep House"));
}

#[test]
fn a_track_with_no_genre_holds_none_and_so_does_one_outside_the_library() {
    assert_eq!(track(None).stored(), None);
    assert_eq!(track(Some("   ")).stored(), None);

    // A tagged track that is not in the Library: the Overview's genre is the
    // Library's, so its row holds none (the table's own sort key is kept).
    let lib = Lib::new();
    let (recording, file) = lib.track(Some("Title"), Some("Artist"), "out.mp3");
    lib.tag(file, &[("Name", "Title"), ("Genre", "House")]);
    lib.settle();
    let held: (Option<String>, Option<String>, Option<String>) = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT genre, genre_group, genre_key FROM track_row WHERE recording_id = ?1",
                [recording],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .unwrap();
    assert_eq!(held, (None, None, Some("house".into())));
}

#[test]
fn a_file_tag_written_changed_or_taken_away_reaches_the_row() {
    let t = track(Some("House"));
    t.lib.tag(t.file, &[("Name", "Title"), ("Genre", "Techno")]);
    t.follows("the tag changed", Some("Techno"));
    t.lib.tag(t.file, &[("Name", "Title")]);
    t.follows("the genre frame left", None);
    t.lib.tag(t.file, &[("Name", "Title"), ("Genre", "Disco")]);
    t.follows("a genre written", Some("Disco"));
    t.lib
        .insert("UPDATE file SET raw_tags = NULL WHERE id = ?1", (t.file,));
    t.follows("the tags forgotten", None);
}

#[test]
fn a_rekordbox_read_gives_rekordboxs_genre_and_a_blank_one_there_gives_none() {
    let t = track(Some("Tag House"));
    let entry = t.entry(t.file, "Rekordbox Disco");
    t.follows("an entry read", Some("Rekordbox Disco"));
    // A fresh read replaces every entry.
    t.lib
        .insert("DELETE FROM rekordbox_track WHERE id = ?1", (entry,));
    t.follows("the entry gone", Some("Tag House"));
    t.entry(t.file, "");
    t.follows("rekordbox knows it with a blank genre", None);
}

#[test]
fn a_relink_that_trusts_unmatches_or_moves_an_entry_reaches_the_row() {
    let t = track(Some("Tag House"));
    let entry = t.entry(t.file, "Rekordbox Disco");
    t.lib.settle();
    t.lib.insert(
        "UPDATE rekordbox_track SET relink_probable = 1 WHERE id = ?1",
        (entry,),
    );
    t.follows("the match became only probable", Some("Tag House"));
    t.lib.insert(
        "UPDATE rekordbox_track SET relink_probable = 0 WHERE id = ?1",
        (entry,),
    );
    t.follows("the match is trusted again", Some("Rekordbox Disco"));
    t.lib.insert(
        "UPDATE rekordbox_track SET file_id = NULL, recording_id = NULL,
                                    relink_method = NULL
         WHERE id = ?1",
        (entry,),
    );
    t.follows("the entry matches no file", Some("Tag House"));
}

#[test]
fn which_of_two_entries_a_track_takes_follows_the_files_role() {
    let t = track(Some("Tag House"));
    let second = t.lib.file(t.recording, "b.mp3", "extra");
    t.lib.insert(
        "UPDATE recording_file SET role = 'best' WHERE file_id = ?1",
        (t.file,),
    );
    t.entry(t.file, "On the best file");
    t.entry(second, "On the other file");
    t.follows("two entries", Some("On the best file"));
    t.lib.insert(
        "UPDATE recording_file SET role = 'extra' WHERE file_id = ?1",
        (t.file,),
    );
    t.lib.insert(
        "UPDATE recording_file SET role = 'best' WHERE file_id = ?1",
        (second,),
    );
    t.follows("the other file became the best", Some("On the other file"));
}

#[test]
fn a_new_link_reads_the_new_files_tag() {
    let t = track(Some("House"));
    let better = t.lib.file(t.recording, "better.mp3", "extra");
    t.lib.tag(better, &[("Name", "Title"), ("Genre", "Garage")]);
    t.lib.settle();
    t.lib.insert(
        "UPDATE library_track SET linked_file_id = ?2 WHERE id = ?1",
        (t.library, better),
    );
    t.follows("linked to the other file", Some("Garage"));
}

#[test]
fn a_track_that_leaves_the_library_loses_its_genre_and_one_that_joins_gains_it() {
    let t = track(Some("House"));
    t.lib
        .insert("DELETE FROM library_track WHERE id = ?1", (t.library,));
    assert!(t.due());
    t.lib.settle();
    assert_eq!(t.stored(), None, "it left the Library");
    t.lib.library(t.recording, t.file);
    assert!(t.due());
    t.lib.settle();
    assert_eq!(t.stored(), Some(("House".into(), "house".into())));
}

#[test]
fn a_merge_that_makes_an_entry_unpickable_reaches_the_row_and_taking_it_back_restores_it() {
    // The entry sits on a file that is not the linked one: it counts only
    // while no merge into the track says otherwise.
    let t = track(Some("Tag House"));
    let other = t.lib.file(t.recording, "b.mp3", "extra");
    t.entry(other, "Merged in");
    t.follows("a trusted entry on another file", Some("Merged in"));
    let merged = t.lib.insert("INSERT INTO recording DEFAULT VALUES", ());
    let merge = t.lib.insert(
        "INSERT INTO track_merge (recording_id, into_id) VALUES (?1, ?2)",
        (merged, t.recording),
    );
    t.follows("a merge into the track", Some("Tag House"));
    t.lib
        .insert("DELETE FROM track_merge WHERE id = ?1", (merge,));
    t.follows("the merge taken back", Some("Merged in"));
}

#[test]
fn a_rebuild_comes_out_with_the_same_genres() {
    let t = track(Some("House"));
    t.entry(t.file, "Rekordbox Disco");
    t.lib.settle();
    let before = t.stored();
    assert_eq!(
        before,
        Some(("Rekordbox Disco".into(), "rekordbox disco".into()))
    );
    t.lib.writer.call(|c| rebuild(c, &Mount, 2)).unwrap();
    assert_eq!(t.stored(), before, "the rows are derived data");
}
