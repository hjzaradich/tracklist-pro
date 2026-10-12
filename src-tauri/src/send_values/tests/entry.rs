//! Whose entry the row at a gone linked file's Location is (1bE-10, owner
//! decision 2026-10-12), and [`sent_as`], which answers what a send decides
//! without building any values.

use super::*;

/// A file on disk in a track of its own that isn't in the Library.
fn outside_the_library(lib: &Lib, name: &str) -> (i64, i64) {
    let track = lib.track();
    let file = lib.file(track, name, "best", Spec::default());
    (track, file)
}

#[test]
fn an_entry_relink_matched_to_a_file_no_library_track_is_linked_to_is_still_this_tracks() {
    let lib = Lib::new();
    let (library, _) = gone(&lib);
    // Relink gave the row at the gone file's Location to another file (a
    // copy of the track, or a look-alike elsewhere).
    let (_, other) = outside_the_library(&lib, "other.mp3");
    lib.rekordbox_at(
        "gone.mp3",
        Some(other),
        false,
        READ,
        &[("Name", "Theirs"), ("Rating", "51")],
    );
    let values = lib.ready(library);
    assert!(values.in_rekordbox && values.file_missing);
    assert_eq!(value(&values, "Name"), Some("Theirs"));
    assert_eq!(value(&values, "Rating"), Some("51"));
    // rekordbox's own Location, never the other file's.
    assert_eq!(
        value(&values, "Location"),
        Some("file://localhost/E:/Music/gone.mp3")
    );
}

#[test]
fn an_entry_relink_matched_only_probably_to_such_a_file_is_still_this_tracks() {
    let lib = Lib::new();
    let (library, _) = gone(&lib);
    let (_, other) = outside_the_library(&lib, "other.mp3");
    lib.rekordbox_at("gone.mp3", Some(other), true, READ, &[("Name", "Theirs")]);
    let values = lib.ready(library);
    assert!(values.in_rekordbox && values.file_missing);
    assert_eq!(value(&values, "Name"), Some("Theirs"));
}

#[test]
fn the_entry_is_the_other_tracks_from_the_moment_that_track_is_in_the_library_and_comes_back_after()
{
    let lib = Lib::new();
    let (library, file) = gone(&lib);
    let (other_track, other) = outside_the_library(&lib, "other.mp3");
    let row = lib.rekordbox_at("gone.mp3", Some(other), false, READ, &[("Name", "Theirs")]);
    assert!(lib.ready(library).file_missing);

    // The track that owns the matched file is added to the Library: the
    // entry is its own now, and the gone-file track is left out, as before
    // 1bE-10. One entry is never sent for two tracks.
    let added = lib.library(other_track, Some(other));
    let both = lib.values(&[library, added]);
    assert_eq!(
        both[0].outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
    let Outcome::Ready(theirs) = &both[1].outcome else {
        panic!("{:?}", both[1].outcome);
    };
    assert!(theirs.in_rekordbox && !theirs.file_missing);
    // Asked one at a time it is the same: every Library track counts, not
    // only those of the batch.
    assert_eq!(
        lib.one(library).outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: file })
    );
    assert_eq!(
        lib.sent_as(&[(library, None), (added, None)]),
        [SentAs::Nothing, SentAs::Entry(row)]
    );

    // Removed again: the entry stands in for the gone file again.
    lib.insert("DELETE FROM library_track WHERE id = ?1", (added.0,));
    assert!(lib.ready(library).file_missing);
    assert_eq!(lib.sent_as(&[(library, None)]), [SentAs::Entry(row)]);
}

impl Lib {
    fn sent_as(&self, tracks: &[(LibraryTrackId, Option<i64>)]) -> Vec<SentAs> {
        let tracks = tracks.to_vec();
        self.writer
            .call(move |c| sent_as(c, &plugged(), &tracks))
            .unwrap()
    }
}

#[test]
fn sent_as_says_what_a_send_decides_for_a_track_as_it_is_linked() {
    let lib = Lib::new();
    // rekordbox knows this one at its file.
    let (known, known_file) = lib.linked("known.mp3", Spec::default());
    let row = lib.rekordbox_at("known.mp3", Some(known_file), false, READ, &[]);
    // This one it doesn't know.
    let (new, _) = lib.linked("new.mp3", Spec::default());
    // This one's file is gone, with no entry at its Location.
    let (missing, _) = gone(&lib);
    // And one whose only match is probable is a new track.
    let (probable, probable_file) = lib.linked("probable.mp3", Spec::default());
    lib.rekordbox_at("elsewhere.mp3", Some(probable_file), true, READ, &[]);

    assert_eq!(
        lib.sent_as(&[
            (known, None),
            (new, None),
            (missing, None),
            (probable, None),
            (LibraryTrackId(999), None),
        ]),
        [
            SentAs::Entry(row),
            SentAs::New,
            SentAs::Nothing,
            SentAs::New,
            SentAs::Nothing,
        ]
    );
    // It agrees with the send itself.
    assert!(lib.ready(known).in_rekordbox);
    assert!(!lib.ready(new).in_rekordbox);
}

#[test]
fn sent_as_answers_for_a_track_as_if_it_were_linked_to_another_file_and_writes_nothing() {
    let lib = Lib::new();
    let (library, old) = gone(&lib);
    let at_old = lib.rekordbox_at("gone.mp3", None, false, READ, &[("Name", "Theirs")]);
    // Two more files of the same track: one rekordbox holds an entry for,
    // one it doesn't.
    let track: i64 = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT recording_id FROM recording_file WHERE file_id = ?1",
                [old],
                |r| r.get(0),
            )
        })
        .unwrap();
    let held = lib.file(track, "held.mp3", "extra", Spec::default());
    let at_held = lib.rekordbox_at("held.mp3", Some(held), false, READ, &[]);
    let plain = lib.file(track, "plain.mp3", "extra", Spec::default());

    assert_eq!(lib.sent_as(&[(library, None)]), [SentAs::Entry(at_old)]);
    assert_eq!(
        lib.sent_as(&[(library, Some(held))]),
        [SentAs::Entry(at_held)]
    );
    assert_eq!(lib.sent_as(&[(library, Some(plain))]), [SentAs::New]);
    // Only asked: the link is what it was.
    let linked: i64 = lib
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT linked_file_id FROM library_track WHERE id = ?1",
                [library.0],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(linked, old);
}
