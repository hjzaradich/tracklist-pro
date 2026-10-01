//! Offer tests. Everything is synthetic: a made-up volume, folder, file
//! names and playlists in a migrated database in a temp dir.

use serde_json::json;

use super::*;
use crate::db::Writer;
use crate::rekordbox::store::{replace_snapshot, SnapshotRows};
use crate::rekordbox::RekordboxXml;

/// How a rekordbox entry is matched to a file.
#[derive(Clone, Copy)]
enum Match {
    Trusted(i64),
    Probable(i64),
    /// No file: a Missing track.
    None,
}

/// A database with one volume and one music folder, `Music` (id 1).
struct Lib {
    _dir: tempfile::TempDir,
    writer: Writer,
    next_track_id: std::cell::Cell<i64>,
}

impl Lib {
    fn new() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        writer
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO volume (identity, kind, last_mount_path)
                     VALUES ('serial=1A2B3C4D', 'external', 'E:\\');
                     INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                     VALUES (1, 'Music', 'Music');",
                )
            })
            .unwrap();
        Lib {
            _dir: dir,
            writer,
            next_track_id: std::cell::Cell::new(1),
        }
    }

    fn insert(&self, sql: &'static str, params: impl rusqlite::Params + Send + 'static) -> i64 {
        self.writer
            .call(move |c| {
                c.execute(sql, params)?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// A track with one best file, on disk or not. Returns (track, file).
    fn track(&self, name: &str, present: bool) -> (i64, i64) {
        let track = self.insert(
            "INSERT INTO recording (title) VALUES (?1)",
            [name.to_owned()],
        );
        let file = self.file(track, &format!("{name}.mp3"), "best", present);
        (track, file)
    }

    /// Another file of `track`.
    fn file(&self, track: i64, name: &str, role: &str, present: bool) -> i64 {
        let file = self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
             VALUES (1, ?1, ?1, ?2)",
            (name.to_owned(), present),
        );
        self.insert(
            "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, ?3)",
            (track, file, role.to_owned()),
        );
        file
    }

    /// A file the scan found that grouping hasn't given a track yet.
    fn ungrouped_file(&self, name: &str) -> i64 {
        self.insert(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present)
             VALUES (1, ?1, ?1, 1)",
            [name.to_owned()],
        )
    }

    /// A rekordbox entry at `location`, in `playlists`. Returns its row id.
    fn entry_at(&self, location: String, matched: Match, playlists: &[&[&str]]) -> i64 {
        let track_id = self.next_track_id.get();
        self.next_track_id.set(track_id + 1);
        let attributes = json!({ "TrackID": track_id.to_string(), "Location": location });
        let (file, method, probable) = match matched {
            Match::Trusted(file) => (Some(file), Some("path"), false),
            Match::Probable(file) => (Some(file), Some("filename_only"), true),
            Match::None => (None, None, false),
        };
        self.insert(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, relink_method, relink_probable,
                  playlists)
             VALUES (?1, ?2, '2026-10-01T10:00:00.000Z', ?3, ?4, ?5, ?6)",
            (
                attributes.to_string(),
                format!("key-{track_id}"),
                file,
                method,
                probable,
                json!(playlists).to_string(),
            ),
        )
    }

    /// A rekordbox entry for a file on `E:`.
    fn entry(&self, matched: Match, playlists: &[&[&str]]) -> i64 {
        let n = self.next_track_id.get();
        self.entry_at(
            format!("file://localhost/E:/Old/{n}.mp3"),
            matched,
            playlists,
        )
    }

    fn offer(&self) -> Offer {
        self.writer.call(|c| offer(c, None)).unwrap()
    }

    fn offer_in(&self, playlists: &[&[&str]]) -> Offer {
        let chosen = paths(playlists);
        self.writer.call(move |c| offer(c, Some(&chosen))).unwrap()
    }

    fn add(&self) -> AddSummary {
        self.writer
            .call(|c| Ok(add_offered(c, None)))
            .unwrap()
            .unwrap()
    }

    fn add_in(&self, playlists: &[&[&str]]) -> AddSummary {
        let chosen = paths(playlists);
        self.writer
            .call(move |c| Ok(add_offered(c, Some(&chosen))))
            .unwrap()
            .unwrap()
    }

    fn count(&self, sql: &'static str) -> i64 {
        self.writer
            .call(move |c| c.query_row(sql, [], |r| r.get(0)))
            .unwrap()
    }

    fn library_tracks(&self) -> i64 {
        self.count("SELECT count(*) FROM library_track")
    }

    fn operations(&self) -> i64 {
        self.count("SELECT count(*) FROM operation")
    }

    /// (track, linked file) of every Library track, by track.
    fn library(&self) -> Vec<(i64, i64)> {
        self.writer
            .call(|c| {
                c.prepare(
                    "SELECT recording_id, linked_file_id FROM library_track
                     ORDER BY recording_id",
                )?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
            })
            .unwrap()
    }
}

fn paths(playlists: &[&[&str]]) -> Vec<PlaylistPath> {
    playlists
        .iter()
        .map(|p| p.iter().map(|&name| name.to_owned()).collect())
        .collect()
}

fn offered(to_add: u32) -> Offer {
    Offer {
        to_add,
        already_in_library: 0,
        waiting_in_missing: 0,
        waiting_for_confirmation: 0,
    }
}

// What's offered.

#[test]
fn tracks_with_a_trusted_match_are_offered_and_the_action_adds_them_linked() {
    let lib = Lib::new();
    let (a, a_file) = lib.track("a", true);
    let (b, b_file) = lib.track("b", true);
    lib.entry(Match::Trusted(a_file), &[]);
    lib.entry(Match::Trusted(b_file), &[]);

    assert_eq!(lib.offer(), offered(2));
    let summary = lib.add();
    assert_eq!(summary.added, 2);
    assert_eq!(lib.library(), vec![(a, a_file), (b, b_file)]);
    assert_eq!(
        lib.count("SELECT count(*) FROM library_track WHERE kind = 'linked'"),
        2
    );
    // Nothing left to offer.
    assert_eq!(lib.offer().to_add, 0);
}

#[test]
fn the_library_track_links_to_the_file_rekordbox_uses_not_the_best_file() {
    let lib = Lib::new();
    let (track, _best) = lib.track("a", true);
    let played = lib.file(track, "played.mp3", "undecided", true);
    lib.entry(Match::Trusted(played), &[]);

    lib.add();
    assert_eq!(lib.library(), vec![(track, played)]);
}

#[test]
fn probable_matches_and_missing_tracks_are_left_out_and_counted() {
    let lib = Lib::new();
    let (_, trusted) = lib.track("a", true);
    let (_, maybe) = lib.track("b", true);
    let (_, maybe_too) = lib.track("c", true);
    lib.entry(Match::Trusted(trusted), &[]);
    lib.entry(Match::Probable(maybe), &[]);
    lib.entry(Match::Probable(maybe_too), &[]);
    lib.entry(Match::None, &[]);

    assert_eq!(
        lib.offer(),
        Offer {
            to_add: 1,
            already_in_library: 0,
            waiting_in_missing: 1,
            waiting_for_confirmation: 2,
        }
    );
    lib.add();
    // Only the trusted one went in.
    assert_eq!(lib.library_tracks(), 1);
    assert_eq!(lib.library()[0].1, trusted);
}

#[test]
fn a_track_rekordbox_does_not_know_is_never_offered() {
    let lib = Lib::new();
    lib.track("not in rekordbox", true);

    assert_eq!(lib.offer(), offered(0));
    assert_eq!(lib.add().added, 0);
    assert_eq!(lib.library_tracks(), 0);
}

#[test]
fn a_trusted_match_whose_file_has_gone_from_disk_counts_as_missing() {
    let lib = Lib::new();
    let (_, gone) = lib.track("a", false);
    lib.entry(Match::Trusted(gone), &[]);

    assert_eq!(
        lib.offer(),
        Offer {
            waiting_in_missing: 1,
            ..offered(0)
        }
    );
    assert_eq!(lib.add().added, 0);
    assert_eq!(lib.library_tracks(), 0);
}

#[test]
fn streaming_entries_are_neither_offered_nor_counted() {
    let lib = Lib::new();
    lib.entry_at(
        "soundcloud:tracks:1234".to_owned(),
        Match::None,
        &[&["Set"]],
    );

    assert_eq!(lib.offer(), offered(0));
    assert_eq!(lib.offer_in(&[&["Set"]]), offered(0));
}

#[test]
fn a_matched_file_with_no_track_yet_waits_until_grouping_gives_it_one() {
    let lib = Lib::new();
    let file = lib.ungrouped_file("new.mp3");
    lib.entry(Match::Trusted(file), &[]);
    assert_eq!(lib.offer(), offered(0));

    let track = lib.insert("INSERT INTO recording (title) VALUES ('new')", []);
    lib.insert(
        "INSERT INTO recording_file (recording_id, file_id, role) VALUES (?1, ?2, 'best')",
        (track, file),
    );
    assert_eq!(lib.offer(), offered(1));
}

// Never automatic.

#[test]
fn a_rekordbox_read_and_its_matches_alone_change_no_library_track() {
    let lib = Lib::new();
    let (_, file) = lib.track("a", true);
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>
  <COLLECTION Entries="1">
    <TRACK TrackID="1" Name="Synthetic 1" Location="file://localhost/E:/Music/a.mp3"/>
  </COLLECTION>
  <PLAYLISTS><NODE Type="0" Name="ROOT"></NODE></PLAYLISTS>
</DJ_PLAYLISTS>
"#;
    let rows = SnapshotRows::from_xml(&RekordboxXml::parse(xml.as_bytes()).unwrap());
    lib.writer
        .call(move |c| replace_snapshot(c, &rows, |_, _, _| Ok(())))
        .unwrap();
    // What relink does when it finds the file.
    lib.insert(
        "UPDATE rekordbox_track SET file_id = ?1, relink_method = 'path'",
        [file],
    );

    // The track is offered, and asking for the offer adds nothing.
    assert_eq!(lib.offer(), offered(1));
    assert_eq!(lib.offer(), offered(1));
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(lib.operations(), 0);
}

// Repeats and undo.

#[test]
fn a_repeat_add_adds_nothing_and_logs_nothing() {
    let lib = Lib::new();
    let (_, file) = lib.track("a", true);
    lib.entry(Match::Trusted(file), &[]);

    let first = lib.add();
    assert_eq!(first.added, 1);
    assert!(first.operation_id.is_some());

    let again = lib.add();
    assert_eq!(again.added, 0);
    assert_eq!(again.already_in_library, 1);
    assert_eq!(again.operation_id, None);
    assert_eq!(lib.library_tracks(), 1);
    assert_eq!(lib.operations(), 1);
}

#[test]
fn the_whole_batch_is_one_operation_and_one_undo_removes_it() {
    let lib = Lib::new();
    for name in ["a", "b", "c"] {
        let (_, file) = lib.track(name, true);
        lib.entry(Match::Trusted(file), &[]);
    }

    let summary = lib.add();
    assert_eq!(summary.added, 3);
    assert_eq!(lib.operations(), 1);
    assert_eq!(
        lib.count("SELECT count(*) FROM operation WHERE kind = 'add_rekordbox_tracks'"),
        1
    );

    let operation = summary.operation_id.unwrap();
    let outcome = lib
        .writer
        .call(move |c| Ok(undo_add(c, operation)))
        .unwrap()
        .unwrap();
    assert!(matches!(outcome, UndoOutcome::Undone { .. }), "{outcome:?}");
    assert_eq!(lib.library_tracks(), 0);
    // They're offered again.
    assert_eq!(lib.offer(), offered(3));
}

#[test]
fn the_summarys_undo_never_undoes_a_later_action() {
    let lib = Lib::new();
    let (_, file) = lib.track("a", true);
    lib.entry(Match::Trusted(file), &[]);
    let (other, other_file) = lib.track("b", true);

    let operation = lib.add().operation_id.unwrap();
    // The user adds another track by hand afterwards.
    lib.writer
        .call(move |c| {
            Ok(ops::record(
                c,
                library::PROMOTE_OPERATION,
                &json!({}),
                |rec| library::insert_linked(rec, other, other_file),
            ))
        })
        .unwrap()
        .unwrap();
    assert_eq!(lib.library_tracks(), 2);

    let outcome = lib
        .writer
        .call(move |c| Ok(undo_add(c, operation)))
        .unwrap()
        .unwrap();
    assert_eq!(outcome, UndoOutcome::NothingToUndo);
    assert_eq!(lib.library_tracks(), 2);
}

#[test]
fn several_rekordbox_entries_for_one_track_give_one_library_track() {
    let lib = Lib::new();
    let (track, best) = lib.track("a", true);
    let duplicate = lib.file(track, "a copy.mp3", "extra", true);
    lib.entry(Match::Trusted(best), &[]);
    lib.entry(Match::Trusted(duplicate), &[]);
    lib.entry(Match::Trusted(duplicate), &[]);

    assert_eq!(lib.offer(), offered(1));
    assert_eq!(lib.add().added, 1);
    assert_eq!(lib.library_tracks(), 1);
    assert_eq!(lib.library()[0].0, track);
}

#[test]
fn an_entry_for_another_file_of_a_track_already_in_the_library_is_not_offered() {
    let lib = Lib::new();
    let (track, best) = lib.track("a", true);
    lib.entry(Match::Trusted(best), &[]);
    lib.add();

    // A later read holds a second entry, matched to a duplicate file.
    let duplicate = lib.file(track, "a copy.mp3", "extra", true);
    lib.entry(Match::Trusted(duplicate), &[]);

    assert_eq!(
        lib.offer(),
        Offer {
            already_in_library: 1,
            ..offered(0)
        }
    );
    let again = lib.add();
    assert_eq!((again.added, again.operation_id), (0, None));
    // The Library track still links to the file it was added with.
    assert_eq!(lib.library(), vec![(track, best)]);
    assert_eq!(lib.operations(), 1);
}

#[test]
fn a_missing_track_enters_the_offer_once_relink_finds_its_file() {
    let lib = Lib::new();
    let (track, file) = lib.track("a", true);
    let entry = lib.entry(Match::None, &[]);
    assert_eq!(
        lib.offer(),
        Offer {
            waiting_in_missing: 1,
            ..offered(0)
        }
    );
    assert_eq!(lib.add().added, 0);

    lib.insert(
        "UPDATE rekordbox_track SET file_id = ?1, relink_method = 'fingerprint' WHERE id = ?2",
        (file, entry),
    );
    assert_eq!(lib.offer(), offered(1));
    assert_eq!(lib.add().added, 1);
    assert_eq!(lib.library(), vec![(track, file)]);
}

#[test]
fn a_probable_match_enters_the_offer_once_it_is_confirmed() {
    let lib = Lib::new();
    let (_, file) = lib.track("a", true);
    let entry = lib.entry(Match::Probable(file), &[]);
    assert_eq!(lib.offer().to_add, 0);

    lib.insert(
        "UPDATE rekordbox_track SET relink_probable = 0, relink_method = 'user' WHERE id = ?1",
        [entry],
    );
    assert_eq!(lib.offer(), offered(1));
}

// The summary.

#[test]
fn the_summary_counts_added_already_there_missing_and_unconfirmed() {
    let lib = Lib::new();
    let (there, there_file) = lib.track("there", true);
    lib.entry(Match::Trusted(there_file), &[]);
    lib.add();

    for name in ["new 1", "new 2"] {
        let (_, file) = lib.track(name, true);
        lib.entry(Match::Trusted(file), &[]);
    }
    let (_, maybe) = lib.track("maybe", true);
    lib.entry(Match::Probable(maybe), &[]);
    for _ in 0..3 {
        lib.entry(Match::None, &[]);
    }

    let summary = lib.add();
    assert_eq!(
        (
            summary.added,
            summary.already_in_library,
            summary.waiting_in_missing,
            summary.waiting_for_confirmation
        ),
        (2, 1, 3, 1)
    );
    assert_eq!(lib.library_tracks(), 3);
    assert_eq!(lib.library()[0].0, there);
}

// Chosen playlists.

#[test]
fn a_playlist_pick_offers_and_adds_only_the_tracks_in_those_playlists() {
    let lib = Lib::new();
    let (warmup, warmup_file) = lib.track("warmup", true);
    let (both, both_file) = lib.track("both", true);
    let (_, peak_file) = lib.track("peak", true);
    let (_, loose_file) = lib.track("loose", true);
    lib.entry(Match::Trusted(warmup_file), &[&["Sets", "Warmup"]]);
    lib.entry(Match::Trusted(both_file), &[&["Sets", "Warmup"], &["Peak"]]);
    lib.entry(Match::Trusted(peak_file), &[&["Peak"]]);
    lib.entry(Match::Trusted(loose_file), &[]);
    // Left out, and counted only when their playlist is chosen.
    let (_, maybe) = lib.track("maybe", true);
    lib.entry(Match::Probable(maybe), &[&["Sets", "Warmup"]]);
    lib.entry(Match::None, &[&["Peak"]]);

    assert_eq!(lib.offer().to_add, 4);
    assert_eq!(
        lib.offer_in(&[&["Sets", "Warmup"]]),
        Offer {
            to_add: 2,
            already_in_library: 0,
            waiting_in_missing: 0,
            waiting_for_confirmation: 1,
        }
    );

    let summary = lib.add_in(&[&["Sets", "Warmup"]]);
    assert_eq!(summary.added, 2);
    assert_eq!(
        lib.library(),
        vec![(warmup, warmup_file), (both, both_file)]
    );
    // The rest is still offered, and no playlist was imported or remembered.
    assert_eq!(lib.offer().to_add, 2);
    assert_eq!(lib.count("SELECT count(*) FROM crate"), 0);
    assert_eq!(lib.count("SELECT count(*) FROM setting"), 0);
}

#[test]
fn a_playlist_is_chosen_by_its_whole_path_not_its_name() {
    let lib = Lib::new();
    let (_, a) = lib.track("a", true);
    let (_, b) = lib.track("b", true);
    lib.entry(Match::Trusted(a), &[&["2025", "Warmup"]]);
    lib.entry(Match::Trusted(b), &[&["2026", "Warmup"]]);

    assert_eq!(lib.offer_in(&[&["2026", "Warmup"]]), offered(1));
    assert_eq!(lib.offer_in(&[&["Warmup"]]), offered(0));
    assert_eq!(lib.offer_in(&[]), offered(0));
}

#[test]
fn the_playlists_to_pick_are_listed_by_folder_and_name_with_their_track_counts() {
    let lib = Lib::new();
    let (_, a) = lib.track("a", true);
    lib.entry(Match::Trusted(a), &[&["Sets", "warmup"], &["Peak"]]);
    lib.entry(Match::None, &[&["Sets", "Closing"], &["Peak"]]);
    lib.entry(Match::None, &[]);
    lib.entry_at("soundcloud:tracks:9".to_owned(), Match::None, &[&["Peak"]]);

    let list = lib.writer.call(|c| playlists(c)).unwrap();
    let shown: Vec<(Vec<&str>, u32)> = list
        .iter()
        .map(|p| (p.path.iter().map(String::as_str).collect(), p.tracks))
        .collect();
    assert_eq!(
        shown,
        vec![
            (vec!["Peak"], 2),
            (vec!["Sets", "Closing"], 1),
            (vec!["Sets", "warmup"], 1),
        ]
    );
}

// Over IPC, as the frontend calls them.

#[test]
fn the_frontend_can_ask_for_the_offer_add_it_and_undo_it() {
    use crate::ipc::testing::{app, invoke};
    use tauri::Manager;

    let (_data, app) = app();
    app.state::<Writer>()
        .call(|c| {
            c.execute_batch(
                r#"INSERT INTO volume (identity, kind) VALUES ('serial=1A2B3C4D', 'external');
                   INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
                   VALUES (1, 'Music', 'Music');
                   INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
                   INSERT INTO recording (title) VALUES ('a');
                   INSERT INTO recording_file (recording_id, file_id, role) VALUES (1, 1, 'best');
                   INSERT INTO rekordbox_track
                       (attributes, location_key, read_at, file_id, relink_method, playlists)
                   VALUES ('{"TrackID": "1", "Location": "file://localhost/E:/Music/a.mp3"}',
                           'E:/Music/a.mp3', '2026-10-01T10:00:00.000Z', 1, 'path', '[["Peak"]]');"#,
            )
        })
        .unwrap();

    let offer = invoke(&app, "rekordbox_offer", json!({ "playlists": null })).unwrap();
    assert_eq!(
        offer,
        json!({
            "toAdd": 1, "alreadyInLibrary": 0, "waitingInMissing": 0,
            "waitingForConfirmation": 0,
        })
    );
    let narrowed = invoke(&app, "rekordbox_offer", json!({ "playlists": [["Other"]] })).unwrap();
    assert_eq!(narrowed["toAdd"], 0);
    let list = invoke(&app, "rekordbox_playlists", json!({})).unwrap();
    assert_eq!(list, json!([{ "path": ["Peak"], "tracks": 1 }]));

    let summary = invoke(
        &app,
        "add_rekordbox_tracks",
        json!({ "playlists": [["Peak"]] }),
    )
    .unwrap();
    assert_eq!(summary["added"], 1);
    let tracks = invoke(&app, "library_tracks", json!({})).unwrap();
    assert_eq!(tracks.as_array().unwrap().len(), 1);

    let undone = invoke(
        &app,
        "undo_add_rekordbox_tracks",
        json!({ "operationId": summary["operationId"] }),
    )
    .unwrap();
    assert_eq!(undone["status"], "undone");
    let tracks = invoke(&app, "library_tracks", json!({})).unwrap();
    assert_eq!(tracks, json!([]));
}

// A track can be left with no files (e.g. one removed from the Library
// after it was sent to rekordbox, kept for the send flow).

#[test]
fn a_track_with_no_files_is_never_offered() {
    let lib = Lib::new();
    let fileless = lib.insert("INSERT INTO recording (title) VALUES ('kept')", []);
    // rekordbox still lists it, with no file here, and stale match columns
    // naming the track don't count either.
    let entry = lib.entry(Match::None, &[]);
    lib.insert(
        "UPDATE rekordbox_track SET recording_id = ?1 WHERE id = ?2",
        (fileless, entry),
    );

    assert_eq!(lib.offer().to_add, 0);
    assert_eq!(lib.add().added, 0);
    assert_eq!(lib.library_tracks(), 0);
}

// Tracks the user removed from the Library in the app.

impl Lib {
    /// Removes `track`'s Library track, as "Remove from Library" does.
    fn remove_from_library(&self, track: i64) {
        let id = self
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT id FROM library_track WHERE recording_id = ?1",
                    [track],
                    |r| r.get(0),
                )
            })
            .unwrap();
        library::remove(&self.writer, library::LibraryTrackId(id)).unwrap();
    }

    fn removal_records(&self) -> i64 {
        self.count("SELECT count(*) FROM library_removal")
    }
}

#[test]
fn a_track_the_user_removed_is_not_offered_again() {
    let lib = Lib::new();
    let (removed, removed_file) = lib.track("removed", true);
    let (kept, kept_file) = lib.track("kept", true);
    lib.entry(Match::Trusted(removed_file), &[&["Peak"]]);
    lib.entry(Match::Trusted(kept_file), &[&["Peak"]]);
    lib.add();
    lib.remove_from_library(removed);
    assert_eq!(lib.library(), vec![(kept, kept_file)]);

    // Not offered, not counted as in the Library, whole collection or pick.
    let left_out = Offer {
        already_in_library: 1,
        ..offered(0)
    };
    assert_eq!(lib.offer(), left_out);
    assert_eq!(lib.offer_in(&[&["Peak"]]), left_out);
    let again = lib.add();
    assert_eq!((again.added, again.operation_id), (0, None));
    assert_eq!(lib.add_in(&[&["Peak"]]).added, 0);
    assert_eq!(lib.library(), vec![(kept, kept_file)]);
    assert_eq!(lib.removal_records(), 1);
}

#[test]
fn a_removed_track_added_back_by_hand_is_an_ordinary_library_track_again() {
    let lib = Lib::new();
    let (track, file) = lib.track("a", true);
    lib.entry(Match::Trusted(file), &[]);
    lib.add();
    lib.remove_from_library(track);
    assert_eq!(lib.offer(), offered(0));

    // "Add to Library" in All music.
    let added = library::promote(&lib.writer, &crate::scan::system_volumes(), track).unwrap();
    assert!(added.added);
    assert_eq!(lib.library(), vec![(track, file)]);
    assert_eq!(lib.removal_records(), 0);
    assert_eq!(
        lib.offer(),
        Offer {
            already_in_library: 1,
            ..offered(0)
        }
    );
}

#[test]
fn undoing_a_removal_puts_the_track_back_and_the_offer_counts_it_as_in_the_library() {
    let lib = Lib::new();
    let (track, file) = lib.track("a", true);
    lib.entry(Match::Trusted(file), &[]);
    lib.add();
    lib.remove_from_library(track);

    crate::ops::undo_last_via(&lib.writer).unwrap();
    assert_eq!(lib.library(), vec![(track, file)]);
    assert_eq!(lib.offer().already_in_library, 1);
    assert_eq!(lib.offer().to_add, 0);
}
