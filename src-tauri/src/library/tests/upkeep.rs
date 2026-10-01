//! Keeping the Library in step with its files (1aE-5), and removing a
//! Library track (1aE-6). Synthetic data, like the rest of the tests.

use super::*;

impl Lib {
    fn set_present(&self, file: i64, present: bool) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET present = ?2 WHERE id = ?1",
                    (file, present),
                )
            })
            .unwrap();
    }

    /// `source_status` as stored, so a test sees what the scan kept, not
    /// what a read worked out.
    fn status(&self, track: i64) -> String {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT source_status FROM library_track WHERE recording_id = ?1",
                    [track],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    fn library_track_id(&self, track: i64) -> LibraryTrackId {
        LibraryTrackId(
            self.writer
                .call(move |c| {
                    c.query_row(
                        "SELECT id FROM library_track WHERE recording_id = ?1",
                        [track],
                        |r| r.get(0),
                    )
                })
                .unwrap(),
        )
    }

    fn remove(&self, track: i64) -> Result<(), LibraryError> {
        remove(&self.writer, self.library_track_id(track))
    }

    fn removed(&self) -> Vec<RemovedTrack> {
        self.writer.call(|c| removed_tracks(c)).unwrap()
    }

    /// A Library track for a new track with one present file.
    fn added(&self, name: &str) -> i64 {
        let track = self.track(Some(name), None);
        self.file(track, name, "best", true);
        self.promote(track).unwrap();
        track
    }

    /// A crate entry, a sync base and an open conflict hanging off
    /// `track`'s Library track.
    fn dependents(&self, track: i64) {
        let id = self.library_track_id(track).0;
        self.writer
            .call(move |c| {
                c.execute_batch(&format!(
                    "INSERT INTO crate (kind, name) VALUES ('static', 'Synthetic crate');
                     INSERT INTO crate_entry (crate_id, library_track_id) VALUES (1, {id});
                     INSERT INTO sync_base (library_track_id, field, value)
                         VALUES ({id}, 'Rating', '0');
                     INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
                         VALUES ({id}, 'Rating', '1', '2', '3');"
                ))
            })
            .unwrap();
    }

    /// Every row hanging off Library tracks, as text, so a test can see
    /// that undo put them all back exactly.
    fn dependent_rows(&self) -> Vec<String> {
        self.writer
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT 'entry|' || id || '|' || crate_id || '|' || library_track_id || '|' || kind || '|' || added_at
                       FROM crate_entry
                     UNION ALL
                     SELECT 'base|' || id || '|' || library_track_id || '|' || field || '|' || value || '|' || synced_at
                       FROM sync_base
                     UNION ALL
                     SELECT 'conflict|' || id || '|' || library_track_id || '|' || field || '|' || status || '|' || detected_at
                       FROM conflict
                     ORDER BY 1",
                )?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                rows.collect()
            })
            .unwrap()
    }
}

// The file-missing flag follows the file (1aE-5).

#[test]
fn a_file_that_disappears_flags_its_track_and_clears_when_it_returns() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    let file = lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();
    assert_eq!(lib.status(track), "ok");

    // What a walk does to a file it didn't see, then to one it sees again.
    lib.set_present(file, false);
    assert_eq!(lib.status(track), "missing");
    assert!(lib.list()[0].source_missing);

    lib.set_present(file, true);
    assert_eq!(lib.status(track), "ok");
    assert!(!lib.list()[0].source_missing);
}

#[test]
fn the_end_of_a_walk_that_did_not_see_the_file_flags_the_track() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();
    // The walk's own end-of-walk step marks the files it didn't see. This
    // file was last seen before the walk started.
    lib.writer
        .call(|c| {
            c.execute(
                "UPDATE file SET last_seen_at = '2000-01-01T00:00:00.000Z'",
                [],
            )?;
            crate::scan::walk::finish(
                c,
                crate::scan::folders::MusicFolderId(1),
                &Default::default(),
                "2026-01-01T00:00:00.000Z",
            )
        })
        .unwrap();
    assert_eq!(lib.status(track), "missing");
}

#[test]
fn an_unplugged_drive_is_not_missing_but_is_not_connected() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "a.mp3", "best", true);
    lib.promote(track).unwrap();

    let listed = lib.list_with(&Mount(None));
    assert!(!listed[0].source_missing);
    assert!(!listed[0].file.as_ref().unwrap().drive_connected);
    assert_eq!(lib.status(track), "ok");
    // Plugged in, nothing is said.
    let listed = lib.list();
    assert!(!listed[0].source_missing);
    assert!(listed[0].file.as_ref().unwrap().drive_connected);
}

#[test]
fn pointing_a_track_at_another_file_takes_that_files_state() {
    let lib = Lib::new();
    let track = lib.track(None, None);
    lib.file(track, "a.mp3", "best", true);
    let second = lib.file(track, "b.mp3", "undecided", false);
    lib.promote(track).unwrap();
    lib.writer
        .call(move |c| c.execute("UPDATE library_track SET linked_file_id = ?1", [second]))
        .unwrap();
    assert_eq!(lib.status(track), "missing");
}

// Removing from the Library (1aE-6).

#[test]
fn removing_a_track_takes_it_out_of_the_library_and_the_send_set_and_records_it() {
    let lib = Lib::new();
    let kept = lib.added("kept.mp3");
    let gone = lib.added("gone.mp3");

    lib.remove(gone).unwrap();

    // The send set is the Library tracks: the removed one isn't in it.
    let listed: Vec<i64> = lib.list().iter().map(|t| t.recording_id).collect();
    assert_eq!(listed, [kept]);
    let removed = lib.removed();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].recording_id, gone);
    // The track itself stays.
    assert_eq!(
        lib.count("SELECT count(*) FROM recording WHERE title = 'gone.mp3'"),
        1
    );
}

#[test]
fn removing_then_undoing_restores_the_same_library_track_and_clears_the_record() {
    let lib = Lib::new();
    let track = lib.added("a.mp3");
    lib.dependents(track);
    let id = lib.library_track_id(track);
    let before = lib.list();
    let rows_before = lib.dependent_rows();
    assert_eq!(rows_before.len(), 3);

    lib.remove(track).unwrap();
    assert_eq!(lib.library_tracks(), 0);
    assert!(lib.dependent_rows().is_empty());
    assert_eq!(lib.removed().len(), 1);

    assert!(matches!(
        undo_last_via(&lib.writer).unwrap(),
        UndoOutcome::Undone { .. }
    ));
    assert_eq!(lib.library_track_id(track), id);
    assert_eq!(lib.list(), before);
    assert!(lib.removed().is_empty());
    assert_eq!(lib.dependent_rows(), rows_before);
}

#[test]
fn a_removal_is_one_operation_with_no_ui_text() {
    let lib = Lib::new();
    let track = lib.added("a.mp3");
    let ops_before = lib.operations();
    lib.remove(track).unwrap();
    assert_eq!(lib.operations(), ops_before + 1);
    let (kind, details): (String, String) = lib
        .writer
        .call(|c| {
            c.query_row(
                "SELECT kind, details FROM operation ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap();
    assert_eq!(kind, REMOVE_OPERATION);
    assert!(details.contains("libraryTrackId"));
}

#[test]
fn removing_a_track_that_is_not_in_the_library_changes_nothing() {
    let lib = Lib::new();
    lib.added("a.mp3");
    let ops_before = lib.operations();
    assert!(matches!(
        remove(&lib.writer, LibraryTrackId(999)),
        Err(LibraryError::TrackNotFound)
    ));
    assert_eq!(lib.operations(), ops_before);
    assert_eq!(lib.library_tracks(), 1);
}

#[test]
fn adding_a_removed_track_back_by_hand_clears_its_record_and_undo_brings_it_back() {
    let lib = Lib::new();
    let track = lib.added("a.mp3");
    lib.remove(track).unwrap();
    assert_eq!(lib.removed().len(), 1);

    let added = lib.promote(track).unwrap();
    assert!(added.added);
    assert!(lib.removed().is_empty());
    assert_eq!(lib.library_tracks(), 1);

    // Undoing the add takes the track out again, with its record.
    undo_last_via(&lib.writer).unwrap();
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(lib.removed().len(), 1);
}

#[test]
fn only_removed_tracks_that_were_sent_are_listed_to_remove_in_rekordbox() {
    let lib = Lib::new();
    let sent = lib.added("sent.mp3");
    let unsent = lib.added("unsent.mp3");
    lib.writer
        .call(|c| {
            c.execute(
                "UPDATE library_track SET last_sent_location = 'file://localhost/E:/Music/sent.mp3',
                     last_exported_at = '2026-09-30T10:00:00.000Z'
                 WHERE recording_id = (SELECT id FROM recording WHERE title = 'sent.mp3')",
                [],
            )
        })
        .unwrap();
    lib.remove(sent).unwrap();
    lib.remove(unsent).unwrap();

    let by_hand = lib.writer.call(|c| remove_in_rekordbox(c)).unwrap();
    assert_eq!(by_hand.len(), 1);
    assert_eq!(by_hand[0].recording_id, sent);
    assert_eq!(
        by_hand[0].last_sent_location.as_deref(),
        Some("file://localhost/E:/Music/sent.mp3")
    );
    assert_eq!(lib.removed().len(), 2);
}

#[cfg(windows)]
#[test]
fn removing_a_library_track_leaves_the_file_on_disk_untouched() {
    let lib = Lib::new();
    let mount = std::fs::canonicalize(lib.dir.path()).unwrap();
    std::fs::create_dir(mount.join("Music")).unwrap();
    let on_disk = mount.join("Music").join("a.mp3");
    std::fs::write(&on_disk, b"synthetic audio bytes").unwrap();
    let before = std::fs::metadata(&on_disk).unwrap();

    let track = lib.added("a.mp3");
    lib.remove(track).unwrap();
    undo_last_via(&lib.writer).unwrap();
    lib.remove(track).unwrap();

    let after = std::fs::metadata(&on_disk).unwrap();
    assert_eq!(std::fs::read(&on_disk).unwrap(), b"synthetic audio bytes");
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    // The file's own row is still there too.
    assert_eq!(lib.count("SELECT count(*) FROM file"), 1);
}
