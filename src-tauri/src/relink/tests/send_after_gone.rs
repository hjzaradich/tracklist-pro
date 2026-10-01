//! A Library track whose linked file goes missing, through a rekordbox
//! read, relink, send values, the writer and record_send (1aF-2, owner
//! decision 2026-10-01): relink never matches a row to a missing file, so
//! the track is found through its own Location.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
use crate::rekordbox::RekordboxXml;
use crate::rekordbox_write::{build, record_send, Node, Outgoing, SendInput, TrackInput};
use crate::send_values::{send_values_one, CannotSend, Outcome};
use crate::volume::{Volume, VolumeId};

/// The test volume (serial 1), plugged in at `E:\`.
struct Plugged;

impl Volumes for Plugged {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!("{}", path.display())))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (id.as_str() == serial(1)).then(|| PathBuf::from(r"E:\"))
    }
}

impl Lib {
    /// A Library track linked to `file`, in a track of its own.
    fn linked(&self, file: i64) -> LibraryTrackId {
        self.writer
            .call(move |c| {
                c.execute("INSERT INTO recording DEFAULT VALUES", [])?;
                let recording = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO recording_file (recording_id, file_id, role)
                     VALUES (?1, ?2, 'best')",
                    (recording, file),
                )?;
                c.execute(
                    "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                     VALUES (?1, 'linked', ?2, 'ok')",
                    (recording, file),
                )?;
                Ok(LibraryTrackId(c.last_insert_rowid()))
            })
            .unwrap()
    }

    fn send_outcome(&self, track: LibraryTrackId) -> Outcome {
        self.writer
            .call(move |c| send_values_one(c, &Plugged, track))
            .unwrap()
            .outcome
    }

    /// The highest TrackID of the last read, as a send is numbered from.
    fn highest(&self) -> u64 {
        let highest: i64 = self
            .writer
            .call(|c| {
                c.query_row("SELECT max(track_id) FROM rekordbox_track", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        u64::try_from(highest).unwrap()
    }

    fn scalar<T: rusqlite::types::FromSql + Send + 'static>(
        &self,
        sql: &'static str,
        id: i64,
    ) -> T {
        self.writer
            .call(move |c| c.query_row(sql, [id], |r| r.get(0)))
            .unwrap()
    }
}

/// rekordbox's spelling: a raw `#`, which the writer would escape if it
/// re-encoded the Location.
const THEIRS: &str = "file://localhost/E:/Music/Old%20#1.mp3";

/// A Library track linked to the file `E:\Music\Old #1.mp3`, which rekordbox
/// holds: returns the library, what's mounted, the file, rekordbox's row and
/// the Library track. Nothing has been relinked yet.
fn linked_to_a_file_rekordbox_holds() -> (Lib, Mounted, i64, i64, LibraryTrackId) {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old #1.mp3", Some(200_000));
    lib.hashed(old, 1);
    let row = lib.track_with(THEIRS, Some("200"), &[("Name", "Theirs"), ("Rating", "51")]);
    let track = lib.linked(old);
    (lib, mounted, old, row, track)
}

/// Sends `track` in one crate and returns what was written.
fn send_in_a_crate(lib: &Lib, track: LibraryTrackId) -> (Outgoing, RekordboxXml) {
    let values = lib
        .writer
        .call(move |c| send_values_one(c, &Plugged, track))
        .unwrap();
    let input = SendInput {
        tracks: vec![TrackInput::from(values)],
        crates: vec![Node::Playlist {
            name: "Warm up".into(),
            entries: vec![track],
        }],
        playlists: Vec::new(),
        highest_rekordbox_track_id: lib.highest(),
    };
    let out = build(&input).unwrap();
    let read = RekordboxXml::parse(out.xml()).unwrap();
    (out, read)
}

/// The track is written as rekordbox's own entry, byte-exact, and is in its
/// crate; then it's recorded like any known track.
fn assert_written_whole_and_recorded(lib: &Lib, row: i64, track: LibraryTrackId) {
    let (out, read) = send_in_a_crate(lib, track);
    assert_eq!(out.left_out, []);
    assert_eq!(out.sent.len(), 1);
    let sent = &out.sent[0];
    assert!(sent.in_rekordbox && sent.file_missing);
    // Every attribute of rekordbox's row, as it wrote it, and Location
    // exactly (not re-encoded); no analysis.
    let json: String = lib.scalar("SELECT attributes FROM rekordbox_track WHERE id = ?1", row);
    let theirs: BTreeMap<String, String> = serde_json::from_str(&json).unwrap();
    let written: BTreeMap<String, String> = sent.attributes.iter().cloned().collect();
    assert_eq!(written, theirs);
    assert_eq!(sent.location(), THEIRS);
    let text = std::str::from_utf8(out.xml()).unwrap();
    assert!(text.contains(&format!("Location=\"{THEIRS}\"")));
    // Still in its crate.
    assert_eq!(read.playlists.playlists()[0].1.entries.len(), 1);

    // Recorded like any known track.
    lib.writer
        .call(move |c| record_send(c, &out.sent, out.paths()))
        .unwrap();
    let last: Option<String> = lib.scalar(
        "SELECT last_sent_location FROM library_track WHERE id = ?1",
        track.0,
    );
    assert_eq!(last.as_deref(), Some(THEIRS));
    // If the track is removed from the Library later, the manual-removals
    // list finds rekordbox's row by that Location.
    crate::library::remove(&lib.writer, track).unwrap();
    let by_hand = lib
        .writer
        .call(|c| crate::library::remove_in_rekordbox(c))
        .unwrap();
    assert_eq!(by_hand.len(), 1);
    let key = location::decode(by_hand[0].last_sent_location.as_deref().unwrap())
        .unwrap()
        .match_key();
    let row_key: String = lib.scalar(
        "SELECT location_key FROM rekordbox_track WHERE id = ?1",
        row,
    );
    assert_eq!(key, row_key);
}

#[test]
fn case_a_a_file_that_goes_missing_after_it_was_matched_is_still_sent_whole_and_in_its_crate() {
    let (lib, mounted, old, row, track) = linked_to_a_file_rekordbox_holds();
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), path(old));
    assert!(
        matches!(lib.send_outcome(track), Outcome::Ready(v) if v.in_rekordbox && !v.file_missing)
    );

    // The file goes missing and rekordbox is read again: relink drops the
    // match to a gone file, as it must.
    lib.gone(old);
    lib.fresh_read();
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);
    assert!(matches!(lib.send_outcome(track), Outcome::Ready(v) if v.file_missing));
    assert_written_whole_and_recorded(&lib, row, track);
}

#[test]
fn case_b_a_file_already_missing_at_the_first_read_is_sent_whole_and_in_its_crate() {
    let (lib, mounted, old, row, track) = linked_to_a_file_rekordbox_holds();
    lib.gone(old);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), None);
    assert_written_whole_and_recorded(&lib, row, track);
}

#[test]
fn a_row_relink_matched_to_another_present_file_is_not_used_so_the_track_is_left_out() {
    let (lib, mounted, old, row, track) = linked_to_a_file_rekordbox_holds();
    lib.gone(old);
    // A same-named file of the same length elsewhere: relink gives
    // rekordbox's row to it (step 2), so the row belongs to that file.
    let elsewhere = lib.folder(1, "Elsewhere");
    let copy = lib.file(elsewhere, "Old #1.mp3", Some(200_000));
    lib.relink(&mounted);
    assert_eq!(lib.matched(row).map(|m| m.0), Some(copy));
    assert_eq!(
        lib.send_outcome(track),
        Outcome::CannotSend(CannotSend::FileMissing { file_id: old })
    );
}

#[test]
fn a_missing_file_with_no_rekordbox_row_at_its_location_is_left_out() {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old #1.mp3", Some(200_000));
    lib.gone(old);
    let track = lib.linked(old);
    lib.track_with(&loc("E:/Music/Other.mp3"), Some("200"), &[]);
    lib.relink(&mounted);
    assert_eq!(
        lib.send_outcome(track),
        Outcome::CannotSend(CannotSend::FileMissing { file_id: old })
    );
}
