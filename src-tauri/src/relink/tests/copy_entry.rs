//! A send uses the rekordbox entry of a byte-identical copy (1bE-11, owner
//! decision 2026-10-13; [`crate::send_values`]): a Library track whose
//! linked file has no rekordbox entry, while a byte-identical copy of it in
//! the same track has one that no other Library track is sent as, is sent
//! as that entry. One entry is never sent for two Library tracks.
//! Everything is made up: no file is read.

use super::follow::Drives;
use super::*;
use crate::after_send::{manual_removals, RemovalReason};
use crate::library::LibraryTrackId;
use crate::send_values::{send_values, CannotSend, Outcome, SentAs};

const COPY: &str = "E:/Music/Kappa.mp3";

/// A Library track linked to `E:\Music\New\Kappa.mp3`, which rekordbox
/// has no entry for, and in its track a copy at `E:\Music\Kappa.mp3`,
/// which rekordbox holds (matched by path). The two are byte-identical.
struct Scene {
    lib: Lib,
    music: i64,
    mounted: Mounted,
    linked: i64,
    copy: i64,
    track: LibraryTrackId,
    /// rekordbox's entry, on the copy.
    row: i64,
}

fn scene() -> Scene {
    let (lib, music, mounted) = e_music();
    let linked = lib.file(music, "New/Kappa.mp3", Some(200_000));
    let copy = lib.file(music, "Kappa.mp3", Some(200_000));
    for file in [linked, copy] {
        lib.hashed(file, 1);
        lib.bytes(file, 1);
    }
    lib.merged(&[linked, copy]);
    let row = lib.track_with(&loc(COPY), Some("200"), &[("Name", "Kappa")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), path(copy));
    // Linked to the file rekordbox doesn't hold (an add would link the copy,
    // the file of the trusted match): as after a follow.
    let track = lib.in_library(linked);
    Scene {
        lib,
        music,
        mounted,
        linked,
        copy,
        track,
        row,
    }
}

impl Lib {
    /// Every Library track's answer, as one send asks.
    fn send_all(&self) -> Vec<(LibraryTrackId, Outcome)> {
        self.writer
            .call(|c| {
                let ids: Vec<LibraryTrackId> = c
                    .prepare("SELECT id FROM library_track ORDER BY id")?
                    .query_map([], |r| r.get(0).map(LibraryTrackId))?
                    .collect::<rusqlite::Result<_>>()?;
                send_values(c, &Drives, &ids)
            })
            .unwrap()
            .into_iter()
            .map(|v| (v.library_track, v.outcome))
            .collect()
    }

    /// The rekordbox `TrackID`s a send of every Library track sends as
    /// rekordbox's own entries.
    fn entries_sent(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .send_all()
            .into_iter()
            .filter_map(|(_, outcome)| match outcome {
                Outcome::Ready(values) if values.in_rekordbox => {
                    values.get("TrackID").map(|v| v.value.clone())
                }
                _ => None,
            })
            .collect();
        ids.sort();
        ids
    }

    /// Matches a row to `file` with trust, as relink would have.
    fn trusted_on(&self, row: i64, file: i64) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE rekordbox_track
                     SET file_id = ?2, relink_method = 'filename_duration',
                         relink_confidence = 0.9, relink_probable = 0,
                         recording_id = (SELECT recording_id FROM recording_file
                                         WHERE file_id = ?2)
                     WHERE id = ?1",
                    [row, file],
                )
            })
            .unwrap();
    }

    /// Matches a row to `file` only probably.
    fn probable_on(&self, row: i64, file: i64) {
        self.trusted_on(row, file);
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE rekordbox_track SET relink_probable = 1 WHERE id = ?1",
                    [row],
                )
            })
            .unwrap();
    }

    /// A Library track of a new track of its own, linked to `file` by hand
    /// (grouping never does this: a Library track's file is in its track).
    fn linked_by_hand(&self, file: i64) -> LibraryTrackId {
        self.writer
            .call(move |c| {
                c.execute("INSERT INTO recording DEFAULT VALUES", [])?;
                let other = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                     VALUES (?1, 'linked', ?2, 'ok')",
                    [other, file],
                )?;
                Ok(LibraryTrackId(c.last_insert_rowid()))
            })
            .unwrap()
    }

    fn removals(&self) -> Vec<(RemovalReason, Option<String>)> {
        self.writer
            .call(|c| manual_removals(c, &Drives))
            .unwrap()
            .into_iter()
            .map(|r| (r.reason, r.path))
            .collect()
    }
}

// ---- the rule ----------------------------------------------------------------

#[test]
fn a_track_is_sent_as_the_entry_of_a_byte_identical_copy_with_rekordboxs_own_location() {
    let s = scene();
    assert_eq!(s.lib.sent(s.track), SentAs::Entry(s.row));
    let Outcome::Ready(values) = s.lib.outcome(s.track) else {
        panic!("not sent");
    };
    assert!(values.in_rekordbox);
    assert!(!values.file_missing);
    // Not a second entry beside it: nothing to warn about.
    assert_eq!(values.rekordbox_holds_other_file, None);
    // rekordbox's entry exactly as read, its own Location (the copy's path)
    // included: rekordbox goes on playing the copy.
    assert_eq!(values.get("Location").unwrap().value, loc(COPY));
    assert_eq!(values.get("Name").unwrap().value, "Kappa");
    assert_eq!(s.lib.entries_sent(), ["1"]);
}

#[test]
fn the_same_audio_with_other_tags_is_not_a_byte_identical_copy() {
    // rekordbox wrote a tag into its own file (§5.1): the audio is the
    // same, the bytes aren't. The track goes to rekordbox as a new one, and
    // the send warns of the other file.
    let s = scene();
    s.lib.bytes(s.copy, 2);
    assert_eq!(s.lib.sent(s.track), SentAs::New);
    let Outcome::Ready(values) = s.lib.outcome(s.track) else {
        panic!("not sent");
    };
    assert_eq!(
        values.rekordbox_holds_other_file.map(|o| o.file_id),
        Some(s.copy)
    );
}

#[test]
fn bytes_count_only_while_both_hashes_are_current() {
    for edited in ["copy", "linked", "unhashed"] {
        let s = scene();
        match edited {
            "copy" => s.lib.edited(s.copy),
            "linked" => s.lib.edited(s.linked),
            _ => {
                s.lib
                    .writer
                    .call(move |c| {
                        c.execute("UPDATE file SET blake3 = NULL", [])?;
                        c.execute("DELETE FROM file_stage", [])
                    })
                    .unwrap();
            }
        }
        assert_eq!(s.lib.sent(s.track), SentAs::New, "{edited}");
    }
}

#[test]
fn a_byte_identical_copy_in_another_track_is_not_used() {
    let s = scene();
    s.lib
        .writer
        .call(move |c| {
            c.execute("INSERT INTO recording DEFAULT VALUES", [])?;
            let other = c.last_insert_rowid();
            c.execute(
                "UPDATE recording_file SET recording_id = ?1 WHERE file_id = ?2",
                [other, s.copy],
            )
        })
        .unwrap();
    assert_eq!(s.lib.sent(s.track), SentAs::New);
}

#[test]
fn two_copies_with_an_entry_each_neither_is_used() {
    let s = scene();
    let second = s.lib.file(s.music, "Other/Kappa.mp3", Some(200_000));
    s.lib.hashed(second, 1);
    s.lib.bytes(second, 1);
    s.lib.joins(second, s.copy);
    let other = s
        .lib
        .track_with(&loc("E:/Music/Other/Kappa.mp3"), Some("200"), &[]);
    s.lib.relink(&s.mounted);
    assert_eq!(s.lib.matched(other), path(second));
    assert_eq!(s.lib.sent(s.track), SentAs::New);
    assert_eq!(s.lib.entries_sent(), Vec::<String>::new());
}

#[test]
fn a_copy_with_two_entries_matched_or_only_a_probable_one_is_not_used() {
    // Two entries matched to the copy (whatever the second match is).
    for probable in [false, true] {
        let s = scene();
        let second = s
            .lib
            .track_with(&loc("E:/Music/Elsewhere/Kappa.mp3"), Some("200"), &[]);
        if probable {
            s.lib.probable_on(second, s.copy);
        } else {
            s.lib.trusted_on(second, s.copy);
        }
        assert_eq!(s.lib.sent(s.track), SentAs::New, "probable: {probable}");
    }
    // The copy's only entry is a probable match: not known to be this track.
    let s = scene();
    s.lib.probable_on(s.row, s.copy);
    assert_eq!(s.lib.sent(s.track), SentAs::New);
}

#[test]
fn a_gone_linked_file_is_sent_as_its_copys_entry_if_its_hash_was_current_when_it_went() {
    let s = scene();
    s.lib.gone(s.linked);
    assert_eq!(s.lib.sent(s.track), SentAs::Entry(s.row));
    let Outcome::Ready(values) = s.lib.outcome(s.track) else {
        panic!("not sent");
    };
    assert!(values.file_missing);
    assert_eq!(values.get("Location").unwrap().value, loc(COPY));

    // Changed before it went: its bytes then aren't known.
    let s = scene();
    s.lib.edited(s.linked);
    s.lib.gone(s.linked);
    assert_eq!(
        s.lib.outcome(s.track),
        Outcome::CannotSend(CannotSend::FileMissing { file_id: s.linked })
    );
}

// ---- one entry, one Library track ---------------------------------------------

#[test]
fn an_entry_another_library_track_is_linked_to_is_not_taken() {
    let s = scene();
    let other = s.lib.linked_by_hand(s.copy);
    assert_eq!(s.lib.sent(other), SentAs::Entry(s.row));
    assert_eq!(s.lib.sent(s.track), SentAs::New);
    assert_eq!(s.lib.entries_sent(), ["1"]);
}

#[test]
fn an_entry_a_track_with_a_gone_file_is_sent_as_from_its_own_location_is_not_taken() {
    // rekordbox's entry is at the path of another Library track's file,
    // which is gone; relink matched it to the copy (as step 2 would). That
    // track is sent as the entry (1bE-10), so the copy's track is not.
    let s = scene();
    let gone_path = "E:/Music/Gone/Kappa.mp3";
    let entry = s.lib.track_with(&loc(gone_path), Some("200"), &[]);
    // The copy's own entry goes: only `entry` is on the copy now.
    s.lib
        .writer
        .call(move |c| c.execute("DELETE FROM rekordbox_track WHERE id = ?1", [s.row]))
        .unwrap();
    s.lib.trusted_on(entry, s.copy);
    assert_eq!(s.lib.sent(s.track), SentAs::Entry(entry));

    let gone = s.lib.file(s.music, "Gone/Kappa.mp3", Some(200_000));
    s.lib.hashed(gone, 9);
    let other = s.lib.in_library(gone);
    s.lib.gone(gone);
    assert_eq!(s.lib.sent(other), SentAs::Entry(entry));
    assert_eq!(s.lib.sent(s.track), SentAs::New);
    assert_eq!(s.lib.entries_sent(), ["2"]);
}

#[test]
fn two_library_tracks_each_with_a_byte_identical_copy_of_one_file_one_entry_is_sent_once() {
    // Two Library tracks that are one recording (kept apart until the user
    // merges them), every file byte-identical, and rekordbox's file in one
    // of the two tracks: that track is sent as the entry, the other one as
    // new; whichever track the file is in.
    for in_first in [true, false] {
        let (lib, music, mounted) = e_music();
        let first = lib.file(music, "First/Kappa.mp3", Some(200_000));
        let second = lib.file(music, "Second/Kappa.mp3", Some(200_000));
        let held = lib.file(music, "Kappa.mp3", Some(200_000));
        for file in [first, second, held] {
            lib.hashed(file, 1);
            lib.bytes(file, 1);
        }
        let (a, b) = if in_first {
            (lib.merged(&[first, held]), lib.merged(&[second]))
        } else {
            (lib.merged(&[first]), lib.merged(&[second, held]))
        };
        assert_ne!(a, b);
        let row = lib.track_with(&loc(COPY), Some("200"), &[]);
        lib.relink(&mounted);
        assert_eq!(lib.matched(row), path(held));
        let tracks = [lib.in_library(first), lib.in_library(second)];
        let (with, without) = if in_first {
            (tracks[0], tracks[1])
        } else {
            (tracks[1], tracks[0])
        };
        assert_eq!(lib.sent(with), SentAs::Entry(row));
        assert_eq!(lib.sent(without), SentAs::New);
        assert_eq!(lib.entries_sent(), ["1"]);
    }
}

// ---- a merge and the list of tracks to remove -----------------------------------

#[test]
fn after_a_merge_the_staying_track_is_sent_as_the_leaving_tracks_entry_never_also_listed() {
    // Two Library tracks that are one recording: the leaving one is sent
    // as rekordbox's entry, the staying one (byte-identical file) as new.
    // A merge (1bE-9) records the leaving track's entry to remove in
    // rekordbox and moves its files into the staying track; built here as
    // the merge leaves the tables. The staying track is then sent as that
    // entry, so it isn't listed. With other bytes, it is listed.
    for same_bytes in [true, false] {
        let (lib, music, mounted) = e_music();
        let staying_file = lib.file(music, "New/Kappa.mp3", Some(200_000));
        let leaving_file = lib.file(music, "Kappa.mp3", Some(200_000));
        lib.hashed(staying_file, 1);
        lib.hashed(leaving_file, 1);
        lib.bytes(staying_file, 1);
        lib.bytes(leaving_file, if same_bytes { 1 } else { 2 });
        let staying_track = lib.merged(&[staying_file]);
        let leaving_track = lib.merged(&[leaving_file]);
        let row = lib.track_with(&loc(COPY), Some("200"), &[]);
        lib.relink(&mounted);
        let staying = lib.in_library(staying_file);
        let leaving = lib.in_library(leaving_file);
        assert_eq!(lib.sent(leaving), SentAs::Entry(row));
        assert_eq!(lib.sent(staying), SentAs::New);

        lib.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO replaced_entry (recording_id, location, location_key)
                     SELECT ?1, json_extract(attributes, '$.Location'), location_key
                     FROM rekordbox_track WHERE id = ?2",
                    [leaving_track, row],
                )?;
                c.execute("DELETE FROM library_track WHERE id = ?1", [leaving.0])?;
                c.execute(
                    "UPDATE recording_file SET recording_id = ?1, role = 'undecided'
                     WHERE recording_id = ?2",
                    [staying_track, leaving_track],
                )
            })
            .unwrap();
        lib.relink(&mounted);

        if same_bytes {
            assert_eq!(lib.sent(staying), SentAs::Entry(row));
            assert_eq!(lib.removals(), []);
        } else {
            assert_eq!(lib.sent(staying), SentAs::New);
            assert_eq!(
                lib.removals(),
                [(
                    RemovalReason::Replaced,
                    Some(r"E:\Music\Kappa.mp3".to_owned())
                )]
            );
        }
    }
}
