//! A Library track follows its file (1bE-10): the second step of the relink
//! job ([`crate::library::follow`]), through grouping, relink, the follow,
//! the best-file ranking, the send's own answer and the operation log.
//! Everything is made up: no file is read.

use std::path::{Path, PathBuf};

use super::*;
use crate::fragile::FragileDirs;
use crate::library::follow::{follow_files, Followed, Summary as Follows};
use crate::library::LibraryTrackId;
use crate::ops::{UndoOutcome, UndoRefusal};
use crate::paths::Volumes;
use crate::send_values::{send_values_one, sent_as, CannotSend, Outcome, SentAs};
use crate::volume::{Volume, VolumeId};

/// The test volumes, plugged in: serial 1 at `E:\`, serial 2 at `F:\`.
pub(super) struct Drives;

impl Volumes for Drives {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!("{}", path.display())))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        if id.as_str() == serial(1) {
            Some(PathBuf::from(r"E:\"))
        } else if id.as_str() == serial(2) {
            Some(PathBuf::from(r"F:\"))
        } else {
            None
        }
    }
}

/// No drive is plugged in.
struct NoDrives;

impl Volumes for NoDrives {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!("{}", path.display())))
    }

    fn mount_path(&self, _id: &VolumeId) -> Option<PathBuf> {
        None
    }
}

/// No Downloads or temp folder: nothing here is about fragile places.
const NO_DIRS: FragileDirs = FragileDirs {
    downloads: None,
    temp: Vec::new(),
};

impl Lib {
    /// Adds the track of `file` to the Library the way the app does
    /// ("Add to Library"), as a recorded operation.
    pub(super) fn add(&self, file: i64) -> LibraryTrackId {
        let recording = self.recording_of(file).expect("the file has a track");
        crate::library::promote_with(&self.writer, &Drives, &NO_DIRS, recording)
            .unwrap()
            .library_track
            .id
    }

    /// A Library track linked to `file`, written straight into the table
    /// (no operation): on the file's own track, or on a new track holding
    /// only it.
    pub(super) fn in_library(&self, file: i64) -> LibraryTrackId {
        let recording = match self.recording_of(file) {
            Some(recording) => recording,
            None => self.group(&[file]),
        };
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                     VALUES (?1, 'linked', ?2, 'ok')",
                    (recording, file),
                )?;
                Ok(LibraryTrackId(c.last_insert_rowid()))
            })
            .unwrap()
    }

    /// The follow step, with both test drives plugged in.
    pub(super) fn follow(&self) -> Follows {
        self.writer
            .call(|c| Ok(follow_files(c, &Drives)))
            .unwrap()
            .unwrap()
    }

    /// The relink job's two steps, in its order.
    pub(super) fn job(&self, mounted: &Mounted) -> Follows {
        self.relink(mounted);
        self.follow()
    }

    /// A grouping pass, as the grouping job runs it.
    pub(super) fn regroup(&self) {
        self.writer
            .call(|c| crate::grouping::regroup(c).map(|_| ()))
            .unwrap();
    }

    /// The best-file ranking of every track that is due.
    pub(super) fn roles(&self) {
        self.writer
            .call(|c| {
                while crate::file_roles::settle_batch(c, 100)?.more {}
                Ok(())
            })
            .unwrap();
    }

    pub(super) fn linked_file(&self, track: LibraryTrackId) -> i64 {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT linked_file_id FROM library_track WHERE id = ?1",
                    [track.0],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// Whether the Library list shows the track's file as missing.
    fn shown_missing(&self, track: LibraryTrackId) -> bool {
        let status: String = self
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT source_status FROM library_track WHERE id = ?1",
                    [track.0],
                    |r| r.get(0),
                )
            })
            .unwrap();
        status == "missing"
    }

    /// What a send would write the track from.
    pub(super) fn sent(&self, track: LibraryTrackId) -> SentAs {
        self.writer
            .call(move |c| sent_as(c, &Drives, &[(track, None)]))
            .unwrap()[0]
    }

    fn outcome(&self, track: LibraryTrackId) -> Outcome {
        self.writer
            .call(move |c| send_values_one(c, &Drives, track))
            .unwrap()
            .outcome
    }

    /// The track's "better file", as Review reads it: the file and the step.
    fn better_file(&self, track: LibraryTrackId) -> Option<(i64, String)> {
        use rusqlite::OptionalExtension;
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT bf.file_id, bf.step FROM better_file bf
                     JOIN library_track lt ON lt.recording_id = bf.recording_id
                     WHERE lt.id = ?1",
                    [track.0],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
            })
            .unwrap()
    }

    /// Every operation in the log: its kind and whether it's undone.
    fn operations(&self) -> Vec<(String, bool)> {
        self.writer
            .call(|c| {
                c.prepare("SELECT kind, undone_at IS NOT NULL FROM operation ORDER BY id")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .unwrap()
    }

    fn library_tracks(&self) -> i64 {
        self.writer
            .call(|c| c.query_row("SELECT count(*) FROM library_track", [], |r| r.get(0)))
            .unwrap()
    }

    fn undo(&self) -> UndoOutcome {
        crate::ops::undo_last_via(&self.writer).unwrap()
    }

    fn make_probable(&self, row: i64, file: i64) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE rekordbox_track
                     SET file_id = ?2, relink_method = 'filename_only',
                         relink_confidence = 0.5, relink_probable = 1
                     WHERE id = ?1",
                    [row, file],
                )
            })
            .unwrap();
    }
}

const OLD: &str = "E:/Music/Old.mp3";

/// A Library track with its linked file and, if rekordbox knows it, its row.
struct Scene {
    lib: Lib,
    music: i64,
    mounted: Mounted,
    /// `E:\Music\Old.mp3`: audio 1, 200 s.
    old: i64,
    track: LibraryTrackId,
}

/// A Library track linked to `E:\Music\Old.mp3`, a track rekordbox doesn't
/// know: there are no rekordbox rows at all.
fn unknown() -> Scene {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    lib.group(&[old]);
    let track = lib.add(old);
    Scene {
        lib,
        music,
        mounted,
        old,
        track,
    }
}

/// A Library track linked to `E:\Music\Old.mp3`, which rekordbox holds at
/// that path (matched by relink before the track was added). Returns the
/// scene and rekordbox's row.
fn known() -> (Scene, i64) {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    lib.group(&[old]);
    let row = lib.track_with(&loc(OLD), Some("200"), &[("Name", "Kappa")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), path(old));
    let track = lib.add(old);
    assert_eq!(lib.sent(track), SentAs::Entry(row));
    (
        Scene {
            lib,
            music,
            mounted,
            old,
            track,
        },
        row,
    )
}

impl Scene {
    /// The walk found the old file gone and a file with the same audio at
    /// `rel` (in the same music folder); the hash stage and the grouping
    /// job have run. Returns the new file.
    fn moved_to(&self, rel: &str) -> i64 {
        self.lib.gone(self.old);
        self.copy_at(rel)
    }

    /// A file with the old file's audio at `rel`, hashed and grouped.
    fn copy_at(&self, rel: &str) -> i64 {
        let new = self.lib.file(self.music, rel, Some(200_000));
        self.lib.hashed(new, 1);
        self.lib.regroup();
        new
    }

    fn followed(&self, to: i64) -> Follows {
        Follows {
            followed: vec![Followed {
                library_track: self.track,
                from: self.old,
                to,
            }],
            held: 0,
        }
    }
}

const HELD: Follows = Follows {
    followed: Vec::new(),
    held: 1,
};

// ---- the rule ----------------------------------------------------------------

#[test]
fn grouping_puts_a_file_with_the_gone_linked_files_audio_into_the_library_tracks_own_track() {
    // What the follow leans on: the file it moves to is already one of the
    // track's files, so no file changes track.
    let scene = unknown();
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(
        scene.lib.recording_of(new),
        scene.lib.recording_of(scene.old)
    );
    assert!(scene.lib.recording_of(new).is_some());
}

#[test]
fn a_moved_file_with_the_same_audio_is_followed_and_still_sent_as_rekordboxs_own_entry() {
    let (scene, row) = known();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert!(lib.shown_missing(scene.track));

    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    assert_eq!(lib.matched(row), same_audio(new));
    assert_eq!(lib.linked_file(scene.track), new);
    // The Library list no longer shows the file as missing.
    assert!(!lib.shown_missing(scene.track));
    // Still rekordbox's own entry, with rekordbox's old Location: a send
    // never changes a known track's Location (the check before a send
    // lists it).
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));
    let Outcome::Ready(values) = lib.outcome(scene.track) else {
        panic!("not sent");
    };
    assert!(values.in_rekordbox && !values.file_missing);
    assert_eq!(
        values.get("Location").map(|v| v.value.as_str()),
        Some(loc(OLD).as_str())
    );
}

#[test]
fn a_track_rekordbox_does_not_know_follows_too_and_is_sent_as_a_new_track_at_the_new_place() {
    // No rekordbox data at all: relink has nothing to do, the follow does.
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(
        lib.outcome(scene.track),
        Outcome::CannotSend(CannotSend::FileMissing { file_id: scene.old })
    );

    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    assert_eq!(lib.linked_file(scene.track), new);
    assert_eq!(lib.sent(scene.track), SentAs::New);
    let Outcome::Ready(values) = lib.outcome(scene.track) else {
        panic!("not sent");
    };
    assert!(!values.in_rekordbox);
    assert_eq!(
        values.get("Location").map(|v| v.value.as_str()),
        Some(r"E:\Music\Moved\Renamed.mp3")
    );
}

#[test]
fn it_follows_into_another_music_folder_on_another_drive() {
    for rekordbox_knows in [true, false] {
        let (scene, row) = match rekordbox_knows {
            true => {
                let (scene, row) = known();
                (scene, Some(row))
            }
            false => (unknown(), None),
        };
        let lib = &scene.lib;
        let f = lib.volume(&serial(2), Some(r"F:\"));
        let other = lib.folder(f, "Sets");
        let mounted = Mounted::new([(serial(1), r"E:\"), (serial(2), r"F:\")]);
        lib.gone(scene.old);
        let new = lib.file(other, "Kappa.mp3", Some(200_000));
        lib.hashed(new, 1);
        lib.regroup();

        assert_eq!(lib.job(&mounted), scene.followed(new));
        assert_eq!(lib.linked_file(scene.track), new);
        match row {
            Some(row) => assert_eq!(lib.sent(scene.track), SentAs::Entry(row)),
            None => assert_eq!(lib.sent(scene.track), SentAs::New),
        }
    }
}

#[test]
fn another_encode_never_follows_even_of_the_same_length_and_title() {
    let (scene, row) = known();
    let lib = &scene.lib;
    lib.gone(scene.old);
    // Another encode of it: other audio, the same length to the
    // millisecond, titled as rekordbox names the entry, merged with the old
    // file by the grouping job.
    let encode = lib.file(scene.music, "Moved/Kappa.flac", Some(200_000));
    lib.hashed(encode, 2);
    lib.title(encode, "Kappa");
    lib.joins(encode, scene.old);

    assert_eq!(lib.job(&scene.mounted), Follows::default());
    // Relink trusts the mate for the entry; the link still doesn't move.
    assert_eq!(lib.matched(row), same_track(encode));
    assert_eq!(lib.linked_file(scene.track), scene.old);
    // The Library list shows the file as missing, and Review has its item:
    // the copy that is on disk.
    assert!(lib.shown_missing(scene.track));
    lib.roles();
    assert_eq!(
        lib.better_file(scene.track),
        Some((encode, "onDisk".to_owned()))
    );
    // And the track is still sent as rekordbox's own entry.
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));
    // Again, after the ranking: still nothing moves.
    assert_eq!(lib.job(&scene.mounted), Follows::default());
    assert_eq!(lib.linked_file(scene.track), scene.old);
}

#[test]
fn a_drive_that_is_not_plugged_in_is_never_taken_for_gone() {
    // The linked file is on a drive that was unplugged: no walk listed its
    // folder, so it is still present. A copy of the same audio sits in its
    // track, on disk. Nothing moves, with the drive away or back.
    let scene = unknown();
    let lib = &scene.lib;
    let f = lib.volume(&serial(2), Some(r"F:\"));
    let other = lib.folder(f, "Sets");
    let copy = lib.file(other, "Kappa.mp3", Some(200_000));
    lib.hashed(copy, 1);
    lib.regroup();
    assert_eq!(lib.recording_of(copy), lib.recording_of(scene.old));

    let unplugged = lib
        .writer
        .call(|c| Ok(follow_files(c, &NoDrives)))
        .unwrap()
        .unwrap();
    assert_eq!(unplugged, Follows::default());
    assert_eq!(lib.follow(), Follows::default());
    assert_eq!(lib.linked_file(scene.track), scene.old);
    assert!(!lib.shown_missing(scene.track));
}

/// Only the first test drive is plugged in.
struct OnlyE;

impl Volumes for OnlyE {
    fn volume_for(&self, path: &Path) -> std::io::Result<Volume> {
        Err(std::io::Error::other(format!("{}", path.display())))
    }

    fn mount_path(&self, id: &VolumeId) -> Option<PathBuf> {
        (id.as_str() == serial(1)).then(|| PathBuf::from(r"E:\"))
    }
}

#[test]
fn a_copy_on_a_drive_that_is_not_plugged_in_counts_as_on_disk_and_is_followed_to() {
    // Unplugged isn't missing, here as everywhere: the copy was indexed
    // while its drive was in, and is still present. The linked file is gone
    // (its own drive is in, and was walked).
    let scene = unknown();
    let lib = &scene.lib;
    let f = lib.volume(&serial(2), Some(r"F:\"));
    let other = lib.folder(f, "Sets");
    let copy = lib.file(other, "Kappa.mp3", Some(200_000));
    lib.hashed(copy, 1);
    lib.regroup();
    lib.gone(scene.old);
    let follows = lib
        .writer
        .call(|c| Ok(follow_files(c, &OnlyE)))
        .unwrap()
        .unwrap();
    assert_eq!(follows, scene.followed(copy));
    assert!(!lib.shown_missing(scene.track));
}

#[test]
fn a_file_is_followed_to_only_while_its_hash_is_current() {
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    // It changed on disk since it was hashed: it may hold anything now.
    lib.edited(new);
    assert_eq!(lib.job(&scene.mounted), Follows::default());
    assert_eq!(lib.linked_file(scene.track), scene.old);
    // Hashed again, the same audio.
    lib.hashed(new, 1);
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
}

#[test]
fn a_gone_file_whose_hash_was_not_current_when_it_went_is_no_evidence() {
    let scene = unknown();
    let lib = &scene.lib;
    // The linked file changed, and went before the hash stage ran again:
    // its hash is of older content.
    lib.edited(scene.old);
    scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), Follows::default());
    assert_eq!(lib.linked_file(scene.track), scene.old);
}

#[test]
fn a_file_with_the_same_audio_that_is_not_on_disk_is_not_followed_to() {
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    lib.gone(new);
    assert_eq!(lib.job(&scene.mounted), Follows::default());
    assert_eq!(lib.linked_file(scene.track), scene.old);
}

#[test]
fn a_file_grouping_keeps_in_another_track_is_not_followed() {
    let scene = unknown();
    let lib = &scene.lib;
    lib.gone(scene.old);
    let new = lib.file(scene.music, "Moved/Renamed.mp3", Some(200_000));
    lib.hashed(new, 1);
    // In a track of its own (a held set, or the user split it off).
    lib.group(&[new]);
    assert_eq!(lib.follow(), Follows::default());
    assert_eq!(lib.linked_file(scene.track), scene.old);
}

#[test]
fn two_library_tracks_holding_the_same_audio_nobody_follows() {
    // Two Library tracks that are one recording (kept apart until the user
    // merges them, 1bE-9), both with their file gone, and one file of that
    // audio on disk. Whichever track grouping put it in, neither follows.
    for joins_first in [true, false] {
        let (lib, music, mounted) = e_music();
        let first = lib.file(music, "First/Kappa.mp3", Some(200_000));
        let second = lib.file(music, "Second/Kappa.mp3", Some(200_000));
        for file in [first, second] {
            lib.hashed(file, 1);
            lib.group(&[file]);
        }
        let tracks = [lib.add(first), lib.add(second)];
        lib.gone(first);
        lib.gone(second);
        let new = lib.file(music, "Moved/Renamed.mp3", Some(200_000));
        lib.hashed(new, 1);
        lib.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO recording_file (recording_id, file_id)
                     SELECT recording_id, ?1 FROM recording_file WHERE file_id = ?2",
                    [new, if joins_first { first } else { second }],
                )
            })
            .unwrap();
        assert_eq!(lib.job(&mounted), Follows::default());
        assert_eq!(lib.linked_file(tracks[0]), first);
        assert_eq!(lib.linked_file(tracks[1]), second);
        // The same while only one of the two files is gone.
        lib.back(second);
        assert_eq!(lib.job(&mounted), Follows::default());
        assert_eq!(lib.linked_file(tracks[0]), first);
    }
}

#[test]
fn what_grouping_does_with_one_file_two_library_tracks_could_claim_leaves_nobody_following() {
    // The same, through the grouping job itself.
    let (lib, music, mounted) = e_music();
    let first = lib.file(music, "First/Kappa.mp3", Some(200_000));
    let second = lib.file(music, "Second/Kappa.mp3", Some(200_000));
    for file in [first, second] {
        lib.hashed(file, 1);
        lib.group(&[file]);
    }
    let tracks = [lib.add(first), lib.add(second)];
    lib.gone(first);
    lib.gone(second);
    let new = lib.file(music, "Moved/Renamed.mp3", Some(200_000));
    lib.hashed(new, 1);
    lib.regroup();
    assert_eq!(lib.job(&mounted), Follows::default());
    assert_eq!(lib.linked_file(tracks[0]), first);
    assert_eq!(lib.linked_file(tracks[1]), second);
}

#[test]
fn a_file_a_library_track_is_linked_to_is_never_followed_to() {
    // Can't come from grouping (a Library track's file is in its own
    // track); written by hand, the follow still refuses.
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    let (new_file, old) = (new, scene.old);
    lib.writer
        .call(move |c| {
            c.execute("INSERT INTO recording DEFAULT VALUES", [])?;
            let other = c.last_insert_rowid();
            c.execute(
                "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                 VALUES (?1, 'linked', ?2, 'ok')",
                [other, new_file],
            )
        })
        .unwrap();
    assert_eq!(lib.follow(), Follows::default());
    assert_eq!(lib.linked_file(scene.track), old);
    let twice: i64 = lib
        .writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM (SELECT 1 FROM library_track
                                       GROUP BY linked_file_id HAVING count(*) > 1)",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(twice, 0);
}

#[test]
fn of_several_files_with_the_same_audio_the_one_rekordboxs_entry_is_on_is_followed() {
    let (scene, row) = known();
    let lib = &scene.lib;
    lib.gone(scene.old);
    let first = scene.copy_at("Copies/First.mp3");
    let second = scene.copy_at("Copies/Second.mp3");
    // The ranking calls the second one the track's best file, and relink
    // matches the entry to it.
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE recording_file SET role = 'extra' WHERE file_id <> ?1",
                [second],
            )?;
            c.execute(
                "UPDATE recording_file SET role = 'best' WHERE file_id = ?1",
                [second],
            )
        })
        .unwrap();
    assert_eq!(lib.job(&scene.mounted), scene.followed(second));
    assert_eq!(lib.matched(row), same_audio(second));
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));
    let _ = first;
}

#[test]
fn of_several_files_with_the_same_audio_an_unknown_track_follows_to_its_best_file_else_the_lowest_id(
) {
    for best_is_second in [true, false] {
        let scene = unknown();
        let lib = &scene.lib;
        lib.gone(scene.old);
        let first = scene.copy_at("Copies/First.mp3");
        let second = scene.copy_at("Copies/Second.mp3");
        if best_is_second {
            lib.writer
                .call(move |c| {
                    c.execute(
                        "UPDATE recording_file SET role = 'extra' WHERE file_id <> ?1",
                        [second],
                    )?;
                    c.execute(
                        "UPDATE recording_file SET role = 'best' WHERE file_id = ?1",
                        [second],
                    )
                })
                .unwrap();
        }
        let to = if best_is_second { second } else { first };
        assert_eq!(lib.job(&scene.mounted), scene.followed(to));
    }
}

// ---- the rekordbox entry -----------------------------------------------------

#[test]
fn the_automatic_follow_never_changes_which_rekordbox_entry_a_track_is_sent_as() {
    // 1. The entry isn't matched to the new file (yet): a read of
    //    rekordbox replaced the rows and relink hasn't run. The track is
    //    sent as rekordbox's own entry, found at the old Location; on the
    //    new file it would be a new track. Nothing moves until relink has
    //    matched the entry to that file.
    let (scene, row) = known();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    lib.fresh_read();
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));
    assert_eq!(lib.follow(), HELD);
    assert_eq!(lib.linked_file(scene.track), scene.old);
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));

    // 2. rekordbox itself holds a second entry for the moved file (the
    //    user imported it again at the new place). Following would put the
    //    track on that entry, without the old one's cues: the user's choice
    //    in Review, never the app's.
    let (scene, row) = known();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    let second = lib.track_with(
        &loc("E:/Music/Moved/Renamed.mp3"),
        Some("200"),
        &[("Name", "Kappa")],
    );
    assert_eq!(lib.job(&scene.mounted), HELD);
    assert_eq!(lib.matched(second), path(new));
    assert_eq!(lib.matched(row), None);
    assert_eq!(lib.linked_file(scene.track), scene.old);
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));
    // It is a "Clear best file" item meanwhile.
    lib.roles();
    assert_eq!(
        lib.better_file(scene.track),
        Some((new, "onDisk".to_owned()))
    );

    // 3. Relink gave the entry to another file: a look-alike with the old
    //    file's name and length, elsewhere. The same audio is on disk too,
    //    under another name. The track stays on its entry.
    let (scene, row) = known();
    let lib = &scene.lib;
    scene.moved_to("Moved/Renamed.mp3");
    let lookalike = lib.file(scene.music, "Elsewhere/Old.mp3", Some(200_000));
    lib.hashed(lookalike, 9);
    lib.regroup();
    assert_eq!(lib.job(&scene.mounted), HELD);
    assert_eq!(lib.matched(row).map(|m| m.0), Some(lookalike));
    assert_eq!(lib.linked_file(scene.track), scene.old);
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));
}

#[test]
fn a_follow_never_takes_the_file_another_tracks_entry_is_matched_to() {
    // Two Library tracks with their files gone. rekordbox knows the second
    // one, and relink matched its entry to a file of the FIRST one's audio
    // that happens to have the second one's name and length. That entry
    // stands in for the second track only while no Library track is linked
    // to the file it's matched to: so the first track doesn't follow there.
    let scene = unknown();
    let lib = &scene.lib;
    let bee = lib.file(scene.music, "Bee.mp3", Some(240_000));
    lib.hashed(bee, 2);
    lib.group(&[bee]);
    let row = lib.track_with(&loc("E:/Music/Bee.mp3"), Some("240"), &[("Name", "Bee")]);
    let second = lib.add(bee);
    lib.gone(bee);
    lib.gone(scene.old);
    let new = lib.file(scene.music, "Moved/Bee.mp3", Some(240_000));
    lib.hashed(new, 1);
    lib.regroup();
    assert_eq!(lib.recording_of(new), lib.recording_of(scene.old));

    assert_eq!(lib.job(&scene.mounted), HELD);
    assert_eq!(lib.matched(row).map(|m| m.0), Some(new));
    assert_eq!(lib.linked_file(scene.track), scene.old);
    assert_eq!(lib.sent(second), SentAs::Entry(row));
    assert_eq!(lib.sent(scene.track), SentAs::Nothing);
}

#[test]
fn a_track_rekordbox_does_not_know_never_follows_onto_a_file_rekordbox_holds_an_entry_for() {
    // The track is sent as no entry today. On a file an entry is matched
    // to, it would be sent as that entry (a trusted match), or as a new
    // track beside the entry that is probably its own (a probable one):
    // either way the follow would decide which rekordbox entry the track
    // is. That is the user's choice in Review.
    for probable in [true, false] {
        let scene = unknown();
        let lib = &scene.lib;
        let new = scene.moved_to("Moved/Renamed.mp3");
        let row = lib.track_with(&loc("E:/Gone/Other name.mp3"), Some("200"), &[]);
        if probable {
            lib.make_probable(row, new);
        } else {
            lib.matched_before(row, new, "user", 1.0);
        }
        assert_eq!(lib.sent(scene.track), SentAs::Nothing);
        assert_eq!(lib.follow(), HELD, "probable: {probable}");
        assert_eq!(lib.linked_file(scene.track), scene.old);
        assert_eq!(lib.sent(scene.track), SentAs::Nothing);
    }
}

#[test]
fn a_track_does_not_follow_while_a_file_of_its_track_is_the_probable_match_of_an_entry() {
    // The file it would follow to has no entry, but another file of the
    // track is probably an entry's own: linked to the new file the track
    // would go to rekordbox as new beside it. An add is refused there too
    // (1.8).
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    let encode = lib.file(scene.music, "Moved/Kappa.flac", Some(200_000));
    lib.hashed(encode, 2);
    lib.joins(encode, new);
    let row = lib.track_with(&loc("E:/Gone/Other name.mp3"), Some("200"), &[]);
    lib.make_probable(row, encode);
    assert_eq!(lib.follow(), HELD);
    assert_eq!(lib.linked_file(scene.track), scene.old);
}

#[test]
fn no_follow_while_any_other_rekordbox_entry_is_matched_to_the_file_whatever_the_match() {
    // The track's own entry is on the moved file, and so is a second entry.
    // Which of the two a send would pick for a file both are trusted on
    // (the most played, then the lowest TrackID) must not decide: the track
    // doesn't follow, and is sent as the same entry before and after.
    for second_is_played_more in [true, false] {
        // 1. The track's entry is a confirmed relink to the moved file
        //    (step 0), and rekordbox holds its own entry at the new path
        //    (step 1 doesn't look at who holds the file).
        let (scene, row) = known();
        let lib = &scene.lib;
        let new = scene.moved_to("Moved/Renamed.mp3");
        lib.confirm_now(&loc(OLD), new, Method::Fingerprint);
        let plays = if second_is_played_more { "90" } else { "0" };
        let second = lib.track_with(
            &loc("E:/Music/Moved/Renamed.mp3"),
            Some("200"),
            &[("Name", "Kappa"), ("PlayCount", plays)],
        );
        lib.relink(&scene.mounted);
        assert_eq!(lib.matched(row).map(|m| m.0), Some(new));
        assert!(!lib.probable(row));
        assert_eq!(lib.matched(second), path(new));
        let before = lib.sent(scene.track);
        assert_eq!(before, SentAs::Entry(row));
        assert_eq!(
            lib.follow(),
            HELD,
            "second played more: {second_is_played_more}"
        );
        assert_eq!(lib.linked_file(scene.track), scene.old);
        assert_eq!(lib.sent(scene.track), before);
    }

    // 2. The second entry is only a probable match to the file.
    let (scene, row) = known();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    lib.relink(&scene.mounted);
    assert_eq!(lib.matched(row), same_audio(new));
    let second = lib.track_with(&loc("E:/Gone/Other name.mp3"), Some("200"), &[]);
    lib.make_probable(second, new);
    let before = lib.sent(scene.track);
    assert_eq!(before, SentAs::Entry(row));
    assert_eq!(lib.follow(), HELD);
    assert_eq!(lib.linked_file(scene.track), scene.old);
    assert_eq!(lib.sent(scene.track), before);

    // With only its own entry on the file, the same track follows.
    let (scene, row) = known();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    lib.confirm_now(&loc(OLD), new, Method::Fingerprint);
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));
}

// ---- over time ---------------------------------------------------------------

#[test]
fn running_it_again_changes_nothing() {
    let (scene, _) = known();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    let log = lib.operations();
    for _ in 0..3 {
        assert_eq!(lib.job(&scene.mounted), Follows::default());
    }
    assert_eq!(lib.operations(), log);
    assert_eq!(lib.linked_file(scene.track), new);
}

#[test]
fn the_old_file_coming_back_moves_nothing_back() {
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    let log = lib.operations();
    // The old file is on disk again (a copy was put back).
    lib.back(scene.old);
    lib.regroup();
    for _ in 0..2 {
        assert_eq!(lib.job(&scene.mounted), Follows::default());
    }
    assert_eq!(lib.linked_file(scene.track), new);
    assert_eq!(lib.operations(), log);
    assert_eq!(lib.sent(scene.track), SentAs::New);
}

#[test]
fn when_the_file_is_moved_back_a_track_rekordbox_holds_no_entry_for_follows_it_back() {
    // Not a loop: each move is a change on disk. The new file is gone and
    // the old one is there again.
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    lib.gone(new);
    lib.back(scene.old);
    lib.regroup();
    assert_eq!(
        lib.job(&scene.mounted),
        Follows {
            followed: vec![Followed {
                library_track: scene.track,
                from: new,
                to: scene.old,
            }],
            held: 0,
        }
    );
    assert_eq!(lib.sent(scene.track), SentAs::New);
    assert_eq!(lib.job(&scene.mounted), Follows::default());
}

#[test]
fn known_limit_when_the_file_is_moved_back_to_the_path_of_rekordboxs_entry_the_track_waits_in_review(
) {
    // The file moved, the track followed, and the file is moved back: the
    // new place is gone and the old one is there again. Relink matches
    // rekordbox's entry to the file at its own path (step 1). The track,
    // linked to the file that is gone now, is sent as no entry at that
    // moment (no entry is at the gone file's own path), and the only file
    // it could follow to has an entry matched to it. A follow never puts a
    // track on an entry it isn't sent as: nothing moves, the track is a
    // "Clear best file" item, and until the user answers it is left out of
    // the send. Pinned so it can't change unnoticed.
    let (scene, row) = known();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));
    lib.gone(new);
    lib.back(scene.old);
    lib.regroup();
    for _ in 0..2 {
        assert_eq!(lib.job(&scene.mounted), HELD);
    }
    assert_eq!(lib.matched(row), path(scene.old));
    assert_eq!(lib.linked_file(scene.track), new);
    assert_eq!(lib.sent(scene.track), SentAs::Nothing);
    assert!(lib.shown_missing(scene.track));
    lib.roles();
    assert_eq!(
        lib.better_file(scene.track),
        Some((scene.old, "onDisk".to_owned()))
    );
}

#[test]
fn known_limit_when_the_old_file_comes_back_beside_the_new_one_rekordboxs_entry_returns_to_its_path(
) {
    // The one sequence in which a followed track stops being sent as its
    // rekordbox entry, pinned so it can't change unnoticed. The follow
    // itself changed nothing (the entry was on the new file). Then the old
    // file is on disk again while the new one still is: relink's first step
    // matches the entry to the file at its own Location, as it must, and
    // the link doesn't move back (the linked file is on disk). The track
    // is then a new track for rekordbox, and the send says so
    // (`rekordbox_holds_other_file`: "a second rekordbox entry results").
    let (scene, row) = known();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    assert_eq!(lib.sent(scene.track), SentAs::Entry(row));

    lib.back(scene.old);
    lib.regroup();
    assert_eq!(lib.job(&scene.mounted), Follows::default());
    assert_eq!(lib.matched(row), path(scene.old));
    assert_eq!(lib.linked_file(scene.track), new);
    assert_eq!(lib.sent(scene.track), SentAs::New);
    let Outcome::Ready(values) = lib.outcome(scene.track) else {
        panic!("not sent");
    };
    assert_eq!(
        values.rekordbox_holds_other_file.map(|s| s.file_id),
        Some(scene.old)
    );
}

#[test]
fn nothing_but_the_follow_moves_a_link_not_grouping_the_ranking_relink_or_a_send() {
    // Everything else that runs after a scan, over a track that has both a
    // file of the same audio and another encode on disk: the link stays
    // where it is until the follow step itself runs.
    let (scene, row) = known();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    let encode = lib.file(scene.music, "Moved/Kappa.flac", Some(200_000));
    lib.hashed(encode, 2);
    lib.title(encode, "Kappa");
    lib.joins(encode, new);
    for _ in 0..2 {
        lib.regroup();
        lib.roles();
        lib.relink(&scene.mounted);
        lib.writer
            .call(|c| crate::attach::attach(c).map(|_| ()))
            .unwrap();
        let _ = lib.outcome(scene.track);
        assert_eq!(lib.linked_file(scene.track), scene.old);
    }
    assert_eq!(lib.matched(row), same_audio(new));
    assert_eq!(lib.follow(), scene.followed(new));
}

#[test]
fn only_the_follow_ever_changes_a_library_tracks_linked_file_in_the_apps_code() {
    // A source scan: an existing link is written in one place. Anything
    // else that names the column in a write is an insert of a new Library
    // track (`library::insert_linked`).
    let mut setters = Vec::new();
    let mut updaters = Vec::new();
    for (path, code) in crate::ops::history_tests::app_sources() {
        let name = path
            .to_string_lossy()
            .replace('\\', "/")
            .rsplit("/src/")
            .next()
            .unwrap()
            .to_owned();
        if code.contains("\"linked_file_id\"") {
            setters.push(name.clone());
        }
        let flat = code
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let writes = flat.match_indices("update library_track").any(|(at, _)| {
            let statement = flat[at..].split('"').next().unwrap_or_default();
            let set = statement.split(" where ").next().unwrap_or_default();
            set.contains("linked_file_id")
        });
        if writes {
            updaters.push(name);
        }
    }
    setters.sort();
    assert_eq!(setters, ["library/follow.rs", "library/mod.rs"]);
    assert_eq!(updaters, Vec::<String>::new());
    // library/mod.rs names it once, in the insert of a new Library track.
    let library =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/library/mod.rs"))
            .unwrap();
    let code = library.split("#[cfg(test)]").next().unwrap();
    assert_eq!(code.matches("\"linked_file_id\"").count(), 1);
    let insert = code
        .split("pub fn insert_linked(")
        .nth(1)
        .and_then(|rest| {
            rest.split(
                "
}
",
            )
            .next()
        })
        .expect("library::insert_linked");
    assert!(insert.contains("(\"linked_file_id\", Value::Integer(file_id))"));
    assert!(insert.contains("rec.insert("));
    // And no migration's trigger writes it.
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    for entry in std::fs::read_dir(migrations).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "sql") {
            let sql = std::fs::read_to_string(&path).unwrap().to_lowercase();
            let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                !flat.contains("set linked_file_id"),
                "{} writes linked_file_id",
                path.display()
            );
        }
    }
}

// ---- the log and Undo --------------------------------------------------------

#[test]
fn a_follow_is_written_to_the_log_and_undo_names_the_users_own_last_action() {
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    assert_eq!(
        lib.operations(),
        [
            ("promote".to_owned(), false),
            ("follow_file".to_owned(), false)
        ]
    );
    let details: String = lib
        .writer
        .call(|c| {
            c.query_row(
                "SELECT details FROM operation WHERE kind = 'follow_file'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    let details: serde_json::Value = serde_json::from_str(&details).unwrap();
    assert_eq!(
        details,
        serde_json::json!({ "libraryTrackId": scene.track.0, "from": scene.old, "to": new })
    );
    // The follow is the newest operation; Undo still offers the add.
    let next = lib
        .writer
        .call(|c| Ok(crate::ops::next_undo(c)))
        .unwrap()
        .unwrap();
    assert_eq!(next.operation.map(|o| o.kind).as_deref(), Some("promote"));
    assert_eq!(next.refusal, None);
}

#[test]
fn undoing_the_add_after_a_follow_takes_the_follow_back_with_it() {
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    let UndoOutcome::Undone { operation } = lib.undo() else {
        panic!("not undone");
    };
    assert_eq!(operation.kind, "promote");
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(
        lib.operations(),
        [
            ("promote".to_owned(), true),
            ("follow_file".to_owned(), true)
        ]
    );
    // Nothing is left to undo, and nothing to follow.
    assert_eq!(lib.undo(), UndoOutcome::NothingToUndo);
    assert_eq!(lib.job(&scene.mounted), Follows::default());
}

#[test]
fn undoing_the_add_still_works_after_the_follow_and_the_removal_of_the_old_music_folder() {
    // The main flow of this feature: the files move to a new music folder,
    // the tracks follow, and the user removes the old folder from the app,
    // which deletes the old file rows. Undoing the add then deletes the
    // track; it is never linked back to a file row that is gone.
    let scene = unknown();
    let lib = &scene.lib;
    let new_place = lib.folder(1, "New place");
    lib.gone(scene.old);
    let new = lib.file(new_place, "Kappa.mp3", Some(200_000));
    lib.hashed(new, 1);
    lib.regroup();
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));

    let old_folder = crate::scan::MusicFolderId(scene.music);
    lib.writer
        .call(move |c| crate::scan::folders::remove(c, old_folder))
        .unwrap()
        .unwrap();
    let old = scene.old;
    let rows: i64 = lib
        .writer
        .call(move |c| {
            c.query_row("SELECT count(*) FROM file WHERE id = ?1", [old], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_eq!(rows, 0, "the old file row went with its folder");

    let next = lib
        .writer
        .call(|c| Ok(crate::ops::next_undo(c)))
        .unwrap()
        .unwrap();
    assert_eq!(next.refusal, None);
    let UndoOutcome::Undone { operation } = lib.undo() else {
        panic!("not undone");
    };
    assert_eq!(operation.kind, "promote");
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(
        lib.operations(),
        [
            ("promote".to_owned(), true),
            ("follow_file".to_owned(), true)
        ]
    );
    // The file that is there is untouched, and the track can be added again
    // (once grouping and the ranking have run, as they do after a folder is
    // removed).
    lib.regroup();
    lib.roles();
    let again = lib.add(new);
    assert_eq!(lib.linked_file(again), new);
}

#[test]
fn undoing_the_removal_of_a_followed_track_is_refused_once_the_file_it_was_on_is_gone_too() {
    // What an undo has to write back must still exist: here the removed
    // track's own linked file, whose row went with its music folder. The
    // reason is the one this always had.
    let scene = unknown();
    let lib = &scene.lib;
    let new_place = lib.folder(1, "New place");
    lib.gone(scene.old);
    let new = lib.file(new_place, "Kappa.mp3", Some(200_000));
    lib.hashed(new, 1);
    lib.regroup();
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    crate::library::remove(&lib.writer, scene.track).unwrap();
    let folder = crate::scan::MusicFolderId(new_place);
    lib.writer
        .call(move |c| crate::scan::folders::remove(c, folder))
        .unwrap()
        .unwrap();
    let outcome = lib.undo();
    let UndoOutcome::Refused {
        operation, reason, ..
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!(operation.kind, "remove_from_library");
    assert_eq!(reason, UndoRefusal::SourceGone);
    assert_eq!(lib.library_tracks(), 0);
}

#[test]
fn undoing_an_unrelated_action_leaves_the_follow_alone() {
    let scene = unknown();
    let lib = &scene.lib;
    // Another track, added before the move and so before the follow.
    let other = lib.file(scene.music, "Other.mp3", Some(180_000));
    lib.hashed(other, 5);
    lib.group(&[other]);
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    lib.add(other);
    // The user's last action is that add.
    let UndoOutcome::Undone { .. } = lib.undo() else {
        panic!("not undone");
    };
    assert_eq!(lib.library_tracks(), 1);
    assert_eq!(lib.linked_file(scene.track), new);
    assert_eq!(
        lib.operations(),
        [
            ("promote".to_owned(), false),
            ("follow_file".to_owned(), false),
            ("promote".to_owned(), true)
        ]
    );
}

#[test]
fn an_action_older_than_the_follow_on_another_track_is_undone_without_touching_the_follow() {
    // Two adds, then the first track's file moves. Undo takes back the
    // second add (the user's last action); the follow on the first track
    // stands. One more Undo takes back the first add and its follow.
    let scene = unknown();
    let lib = &scene.lib;
    let other = lib.file(scene.music, "Other.mp3", Some(180_000));
    lib.hashed(other, 5);
    lib.group(&[other]);
    lib.add(other);
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));

    let UndoOutcome::Undone { .. } = lib.undo() else {
        panic!("not undone");
    };
    assert_eq!(lib.linked_file(scene.track), new);
    assert_eq!(
        lib.operations(),
        [
            ("promote".to_owned(), false),
            ("promote".to_owned(), true),
            ("follow_file".to_owned(), false)
        ]
    );
    let UndoOutcome::Undone { .. } = lib.undo() else {
        panic!("not undone");
    };
    assert_eq!(lib.library_tracks(), 0);
}

#[test]
fn after_undoing_an_add_and_adding_again_the_track_ends_linked_to_the_file_on_disk() {
    // There is no redo: adding again is how an undone add comes back.
    for rekordbox_knows in [true, false] {
        let (scene, row) = match rekordbox_knows {
            true => {
                let (scene, row) = known();
                (scene, Some(row))
            }
            false => (unknown(), None),
        };
        let lib = &scene.lib;
        let new = scene.moved_to("Moved/Renamed.mp3");
        assert_eq!(lib.job(&scene.mounted), scene.followed(new));
        let UndoOutcome::Undone { .. } = lib.undo() else {
            panic!("not undone");
        };
        assert_eq!(lib.library_tracks(), 0);
        // The ranking has run since the move, as it does after every job.
        lib.roles();
        let again = lib.add(new);
        lib.job(&scene.mounted);
        assert_eq!(lib.linked_file(again), new, "known: {rekordbox_knows}");
        match row {
            Some(row) => assert_eq!(lib.sent(again), SentAs::Entry(row)),
            None => assert_eq!(lib.sent(again), SentAs::New),
        }
    }
}

#[test]
fn undoing_an_add_before_the_move_and_adding_again_after_it_links_the_file_on_disk() {
    let scene = unknown();
    let lib = &scene.lib;
    let UndoOutcome::Undone { .. } = lib.undo() else {
        panic!("not undone");
    };
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), Follows::default());
    lib.roles();
    let again = lib.add(new);
    assert_eq!(lib.job(&scene.mounted), Follows::default());
    assert_eq!(lib.linked_file(again), new);
}

#[test]
fn a_follow_that_cannot_be_taken_back_refuses_the_undo_of_the_add_as_changed_since() {
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    // The link changed again, outside the log.
    let (track, old) = (scene.track, scene.old);
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE library_track SET linked_file_id = ?2 WHERE id = ?1",
                [track.0, old],
            )
        })
        .unwrap();
    let outcome = lib.undo();
    let UndoOutcome::Refused {
        operation,
        reason,
        conflicts,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!(operation.kind, "promote");
    assert_eq!(reason, UndoRefusal::ChangedSince);
    let fields: Vec<_> = conflicts.iter().map(|c| c.field.as_deref()).collect();
    assert_eq!(fields, [Some("linked_file_id")]);
    // Nothing half done.
    assert_eq!(lib.library_tracks(), 1);
    assert_eq!(lib.linked_file(scene.track), scene.old);
    assert_eq!(
        lib.operations(),
        [
            ("promote".to_owned(), false),
            ("follow_file".to_owned(), false)
        ]
    );
}

#[test]
fn removing_a_followed_track_and_undoing_the_removal_brings_it_back_on_the_file_it_followed_to() {
    let scene = unknown();
    let lib = &scene.lib;
    let new = scene.moved_to("Moved/Renamed.mp3");
    assert_eq!(lib.job(&scene.mounted), scene.followed(new));
    crate::library::remove(&lib.writer, scene.track).unwrap();
    assert_eq!(lib.library_tracks(), 0);
    let UndoOutcome::Undone { operation } = lib.undo() else {
        panic!("not undone");
    };
    assert_eq!(operation.kind, "remove_from_library");
    assert_eq!(lib.linked_file(scene.track), new);
    assert_eq!(lib.job(&scene.mounted), Follows::default());
}
