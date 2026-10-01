//! A Library track whose linked file goes missing, through a rekordbox
//! read, relink and send values (1aF-2, owner decision 2026-10-01): does
//! the track still have a trusted rekordbox entry to send unchanged?

use std::path::{Path, PathBuf};

use super::*;
use crate::library::LibraryTrackId;
use crate::paths::Volumes;
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
}

const THEIRS: &str = "file://localhost/E:/Music/Old.mp3";

/// A file rekordbox holds, linked as a Library track, matched by path.
fn linked_and_known() -> (Lib, Mounted, i64, i64, LibraryTrackId) {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    let row = lib.track_with(THEIRS, Some("200"), &[("Name", "Theirs"), ("Rating", "51")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), path(old));
    let track = lib.linked(old);
    assert!(matches!(lib.send_outcome(track), Outcome::Ready(v) if v.in_rekordbox));
    (lib, mounted, old, row, track)
}

#[test]
fn case_a_a_file_that_goes_missing_after_it_was_matched_loses_its_match_at_the_next_relink() {
    let (lib, mounted, old, row, track) = linked_and_known();
    lib.gone(old);
    lib.fresh_read();
    lib.relink(&mounted);
    let outcome = lib.send_outcome(track);
    assert_eq!(lib.matched(row), None);
    assert_eq!(
        outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: old })
    );
}

#[test]
fn case_b_a_file_already_missing_at_the_first_read_is_never_matched() {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    lib.gone(old);
    let row = lib.track_with(THEIRS, Some("200"), &[("Name", "Theirs")]);
    lib.relink(&mounted);
    let track = lib.linked(old);
    let outcome = lib.send_outcome(track);
    assert_eq!(lib.matched(row), None);
    assert_eq!(
        outcome,
        Outcome::CannotSend(CannotSend::FileMissing { file_id: old })
    );
}
