//! "Use this file instead" (1cB-22). Everything is synthetic: made-up files
//! with made-up hashes in a migrated database in a temp dir.

use rusqlite::OptionalExtension;

use super::choose::Refused;
use super::*;
use crate::best_file::Reason;
use crate::search::tests::support::{Lib, Mount};
use crate::track_table::{TableQuery, TableSource};

/// A file's hashes: its audio, and its whole bytes.
fn hashes(lib: &Lib, file: i64, audio: u8, bytes: u8) {
    lib.insert(
        "UPDATE file SET audio_hash = ?1, blake3 = ?2 WHERE id = ?3",
        (vec![audio; 16], vec![bytes; 32], file),
    );
}

/// A track whose best file is `alpha.flac` and whose extra copy is
/// `alpha (copy).mp3`, with audio of their own. The ranking has run: it
/// ties on everything, so the lower file id is the best.
fn two_files() -> (Lib, i64, i64, i64) {
    let lib = Lib::new();
    let (track, a) = lib.track(Some("Alpha"), Some("Ann"), "alpha.flac");
    let b = lib.file(track, "alpha (copy).mp3", "extra");
    hashes(&lib, a, 1, 1);
    hashes(&lib, b, 2, 2);
    rank(&lib);
    (lib, track, a, b)
}

/// Works out the roles of every track that is due.
fn rank(lib: &Lib) {
    lib.writer
        .call(|c| crate::file_roles::settle_batch(c, 500))
        .unwrap();
}

fn roles(lib: &Lib, track: i64) -> Vec<(i64, String)> {
    lib.writer
        .call(move |c| {
            c.prepare(
                "SELECT file_id, role FROM recording_file
                 WHERE recording_id = ?1 ORDER BY file_id",
            )?
            .query_map([track], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
        })
        .unwrap()
}

fn best_of(lib: &Lib, track: i64) -> i64 {
    roles(lib, track)
        .into_iter()
        .find(|(_, role)| role == "best")
        .expect("a track has one best file")
        .0
}

fn choose(lib: &Lib, file: i64) -> Result<Chosen, Refused> {
    lib.writer
        .call(move |c| Ok(use_this_file(c, file)))
        .unwrap()
}

fn choices(lib: &Lib) -> Vec<(i64, Vec<u8>, Vec<u8>)> {
    lib.writer
        .call(|c| {
            c.prepare("SELECT id, content_key, blake3 FROM file_choice ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        })
        .unwrap()
}

fn undo(lib: &Lib) {
    let undone = lib
        .writer
        .call(|c| Ok(ops::undo_for_user(c, None)))
        .unwrap()
        .unwrap();
    assert!(
        matches!(undone, ops::UndoOutcome::Undone { .. }),
        "{undone:?}"
    );
}

fn listed(lib: &Lib) -> Vec<HiddenRow> {
    lib.settle();
    let q = HiddenQuery {
        search: None,
        sort: None,
        offset: 0,
        limit: 100,
        with_total: true,
    };
    lib.writer.call(move |c| page(c, &Mount, &q)).unwrap().rows
}

/// The name of the file the All music table shows for the only track.
fn shown_file(lib: &Lib) -> String {
    lib.settle();
    let q = TableQuery {
        source: TableSource::AllMusic,
        search: None,
        filters: Vec::new(),
        sort: None,
        offset: 0,
        limit: 100,
        with_total: true,
    };
    lib.writer
        .call(move |c| {
            let found = crate::track_table::read(c, &Mount, &q)?.expect("All music exists");
            let page = crate::track_table::page(
                found,
                0,
                &Mount,
                &crate::fragile::FragileDirs {
                    downloads: None,
                    temp: Vec::new(),
                },
            );
            Ok(page.rows[0].file.as_ref().expect("a file").name.clone())
        })
        .unwrap()
}

#[test]
fn the_chosen_file_is_the_one_its_track_shows() {
    let (lib, track, a, b) = two_files();
    assert_eq!(best_of(&lib, track), a);
    assert_eq!(shown_file(&lib), "alpha.flac");
    // Before the choice the extra copy offers the button.
    let before = listed(&lib);
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].way_back, Some(WayBack::UseThisFile));
    assert!(!before[0].best_file_chosen);

    let chosen = choose(&lib, b).unwrap();
    assert!(chosen.operation_id.is_some());

    assert_eq!(best_of(&lib, track), b);
    assert_eq!(shown_file(&lib), "alpha (copy).mp3");
    // The file that was shown is an extra copy now, with the way back, and
    // its reason says the user chose the other one.
    let after = listed(&lib);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].file_id, a);
    assert_eq!(after[0].best_file.as_deref(), Some("alpha (copy).mp3"));
    assert!(after[0].best_file_chosen);
    assert_eq!(after[0].way_back, Some(WayBack::UseThisFile));
}

#[test]
fn the_ranking_names_the_choice_as_its_reason() {
    let (lib, track, _, b) = two_files();
    choose(&lib, b).unwrap();
    let ranking = lib
        .writer
        .call(move |c| {
            let loaded = crate::file_roles::facts::load(c, &[track])?;
            let facts: Vec<_> = loaded[&track].iter().map(|l| l.facts.clone()).collect();
            Ok(crate::best_file::rank(&facts))
        })
        .unwrap();
    assert_eq!(ranking.best(), Some(b));
    assert_eq!(ranking.order[0].reason, Reason::Chosen);
    assert_eq!(ranking.order[1].reason, Reason::Chosen);
}

#[test]
fn a_choice_survives_a_ranking_run_a_move_a_retag_and_a_rescan() {
    let (lib, track, a, b) = two_files();
    choose(&lib, b).unwrap();

    // The next ranking run, with every track due.
    lib.exec("INSERT OR IGNORE INTO best_file_due (recording_id) SELECT id FROM recording");
    rank(&lib);
    assert_eq!(best_of(&lib, track), b);

    // A move to another music folder.
    lib.exec(
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Other', 'Other');
         UPDATE file SET music_folder_id = 2 WHERE id = 2",
    );
    rank(&lib);
    assert_eq!(best_of(&lib, track), b);

    // A retag: other tags, other bytes, the same audio.
    lib.insert(
        "UPDATE file SET raw_tags = '{}', blake3 = ?1 WHERE id = ?2",
        (vec![9u8; 32], b),
    );
    rank(&lib);
    assert_eq!(best_of(&lib, track), b);

    // The music folder removed and added again: new files, the same content.
    lib.exec("DELETE FROM recording_file; DELETE FROM file");
    let a2 = lib.file(track, "alpha.flac", "undecided");
    let b2 = lib.file(track, "alpha (copy).mp3", "undecided");
    assert!(a2 > a && b2 > b, "the files are new rows");
    hashes(&lib, a2, 1, 1);
    hashes(&lib, b2, 2, 9);
    rank(&lib);
    assert_eq!(best_of(&lib, track), b2);
    assert_eq!(choices(&lib).len(), 1);
}

#[test]
fn undo_takes_the_choice_back_exactly_and_the_ranking_chooses_again() {
    let (lib, track, a, b) = two_files();
    let roles_before = roles(&lib, track);

    let chosen = choose(&lib, b).unwrap();
    assert_eq!(best_of(&lib, track), b);
    assert_eq!(choices(&lib).len(), 1);
    // One operation, of its own kind.
    let kinds: Vec<String> = lib
        .writer
        .call(|c| {
            c.prepare("SELECT kind FROM operation ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .unwrap();
    assert_eq!(kinds, vec![USE_THIS_FILE_OPERATION.to_owned()]);
    assert!(chosen.operation_id.is_some());

    undo(&lib);

    assert_eq!(choices(&lib), Vec::new());
    assert_eq!(roles(&lib, track), roles_before);
    assert_eq!(best_of(&lib, track), a);
    assert_eq!(shown_file(&lib), "alpha.flac");
}

#[test]
fn choosing_another_file_replaces_the_earlier_choice_and_each_undo_brings_one_back() {
    let (lib, track, a, b) = two_files();
    let c = lib.file(track, "alpha (other).ogg", "extra");
    hashes(&lib, c, 3, 3);
    rank(&lib);

    choose(&lib, b).unwrap();
    let first = choices(&lib);
    assert_eq!(first.len(), 1);
    choose(&lib, c).unwrap();
    assert_eq!(best_of(&lib, track), c);
    // One choice a track: the second took the first away.
    let second = choices(&lib);
    assert_eq!(second.len(), 1);
    assert_ne!(first[0].1, second[0].1);

    undo(&lib);
    assert_eq!(choices(&lib), first);
    assert_eq!(best_of(&lib, track), b);
    undo(&lib);
    assert_eq!(choices(&lib), Vec::new());
    assert_eq!(best_of(&lib, track), a);
}

#[test]
fn a_library_tracks_extra_file_has_the_reason_and_no_button_and_the_backend_refuses() {
    let (lib, track, a, b) = two_files();
    lib.library(track, a);

    let rows = listed(&lib);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].why, Why::ExtraFile);
    assert_eq!(rows[0].way_back, None);
    assert!(rows[0].track_in_library);
    assert_eq!(rows[0].best_file.as_deref(), Some("alpha.flac"));

    assert!(matches!(choose(&lib, b), Err(Refused::InLibrary)));
    assert_eq!(choices(&lib), Vec::new());
    assert_eq!(best_of(&lib, track), a);
}

#[test]
fn a_chosen_file_that_is_gone_or_encoded_again_leaves_the_decision_to_the_ranking() {
    let (lib, track, a, b) = two_files();
    choose(&lib, b).unwrap();
    assert_eq!(best_of(&lib, track), b);

    // Gone from disk: the ranking decides again; the choice is kept.
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (b,));
    rank(&lib);
    assert_eq!(best_of(&lib, track), a);
    assert_eq!(choices(&lib).len(), 1);
    // Back on disk: the choice holds again.
    lib.insert("UPDATE file SET present = 1 WHERE id = ?1", (b,));
    rank(&lib);
    assert_eq!(best_of(&lib, track), b);

    // Encoded again: other audio, so the choice covers nothing.
    lib.insert(
        "UPDATE file SET audio_hash = ?1, blake3 = ?2 WHERE id = ?3",
        (vec![7u8; 16], vec![7u8; 32], b),
    );
    rank(&lib);
    assert_eq!(best_of(&lib, track), a);
}

#[test]
fn files_holding_the_same_audio_are_told_apart_by_their_bytes() {
    let (lib, track, a, b) = two_files();
    // The same audio with other tags: other bytes.
    hashes(&lib, b, 1, 2);
    rank(&lib);
    assert_eq!(best_of(&lib, track), a);
    assert_eq!(listed(&lib)[0].way_back, Some(WayBack::UseThisFile));

    choose(&lib, b).unwrap();
    assert_eq!(best_of(&lib, track), b);
    lib.exec("INSERT OR IGNORE INTO best_file_due (recording_id) SELECT id FROM recording");
    rank(&lib);
    assert_eq!(best_of(&lib, track), b);

    // The very same bytes can't be told apart: no button, and a refusal.
    let (lib, track, a, b) = two_files();
    hashes(&lib, b, 1, 1);
    rank(&lib);
    assert_eq!(listed(&lib)[0].way_back, None);
    assert!(matches!(choose(&lib, b), Err(Refused::SameAsShown)));
    assert_eq!(best_of(&lib, track), a);
}

#[test]
fn a_file_that_cannot_be_chosen_is_refused() {
    let (lib, track, a, b) = two_files();
    // The track's shown file, an unknown file.
    assert!(matches!(choose(&lib, a), Err(Refused::Gone)));
    assert!(matches!(choose(&lib, 9999), Err(Refused::Gone)));
    // Hidden by the user: its way back is "Show again".
    lib.insert("UPDATE file SET hidden_reason = 'user' WHERE id = ?1", (b,));
    assert!(matches!(choose(&lib, b), Err(Refused::Gone)));
    lib.insert("UPDATE file SET hidden_reason = NULL WHERE id = ?1", (b,));
    // Not on disk.
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (b,));
    assert!(matches!(choose(&lib, b), Err(Refused::Gone)));
    lib.insert("UPDATE file SET present = 1 WHERE id = ?1", (b,));
    // Nothing to remember it by.
    lib.insert(
        "UPDATE file SET audio_hash = NULL, blake3 = NULL WHERE id = ?1",
        (b,),
    );
    assert!(matches!(choose(&lib, b), Err(Refused::NoAudioKnown)));
    assert_eq!(listed(&lib)[0].way_back, None);

    assert_eq!(choices(&lib), Vec::new());
    assert_eq!(best_of(&lib, track), a);
}

fn better_file(lib: &Lib, track: i64) -> Option<(i64, String)> {
    lib.writer
        .call(move |c| {
            c.query_row(
                "SELECT file_id, step FROM better_file WHERE recording_id = ?1",
                [track],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })
        .unwrap()
}

#[test]
fn a_choice_has_no_say_for_a_track_in_the_library_and_holds_again_when_it_leaves() {
    let (lib, track, a, b) = two_files();
    choose(&lib, b).unwrap();
    assert_eq!(best_of(&lib, track), b);

    // The track joins the Library linked to the other file: the choice rests,
    // and the ranking is as if there were none.
    let library_track = lib.library(track, a);
    rank(&lib);
    assert_eq!(best_of(&lib, track), a);
    assert_eq!(better_file(&lib, track), None);
    assert_eq!(choices(&lib).len(), 1);

    // The linked file goes: Review's "copy found" offer is made as before,
    // though the copy is the file that was chosen.
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (a,));
    rank(&lib);
    assert_eq!(better_file(&lib, track), Some((b, "onDisk".to_owned())));
    lib.insert("UPDATE file SET present = 1 WHERE id = ?1", (a,));
    rank(&lib);

    // The track leaves the Library: the choice applies again.
    lib.insert("DELETE FROM library_track WHERE id = ?1", (library_track,));
    rank(&lib);
    assert_eq!(best_of(&lib, track), b);
}

#[test]
fn a_retag_of_the_chosen_file_does_not_undo_the_choice_even_when_another_copy_shares_its_audio() {
    let (lib, track, a, b) = two_files();
    // The same audio with other tags: other bytes.
    hashes(&lib, b, 1, 2);
    rank(&lib);
    choose(&lib, b).unwrap();
    assert_eq!(best_of(&lib, track), b);

    // Another program retags the chosen copy; the scan hashes it again.
    lib.insert(
        "UPDATE file SET raw_tags = '{}', blake3 = ?1 WHERE id = ?2",
        (vec![9u8; 32], b),
    );
    rank(&lib);
    assert_eq!(best_of(&lib, track), b);
    // The choice follows the bytes, and its Undo still works.
    assert_eq!(choices(&lib)[0].2, vec![9u8; 32]);
    undo(&lib);
    assert_eq!(choices(&lib), Vec::new());
    assert_eq!(best_of(&lib, track), a);
}

#[test]
fn a_file_with_no_audio_hash_is_kept_by_its_whole_file_hash() {
    let (lib, track, a, b) = two_files();
    lib.insert(
        "UPDATE file SET audio_hash = NULL, blake3 = ?1 WHERE id = ?2",
        (vec![5u8; 32], b),
    );
    rank(&lib);
    assert_eq!(listed(&lib)[0].way_back, Some(WayBack::UseThisFile));

    choose(&lib, b).unwrap();
    assert_eq!(best_of(&lib, track), b);
    assert_eq!(choices(&lib)[0].1, vec![5u8; 32]);

    // A ranking run and a move keep it.
    lib.exec("INSERT OR IGNORE INTO best_file_due (recording_id) SELECT id FROM recording");
    rank(&lib);
    assert_eq!(best_of(&lib, track), b);
    lib.exec(
        "INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'Other', 'Other');
         UPDATE file SET music_folder_id = 2 WHERE id = 2",
    );
    rank(&lib);
    assert_eq!(best_of(&lib, track), b);

    // Its bytes are its key: changed bytes are another key, the choice
    // covers nothing, and the ranking decides again.
    lib.insert(
        "UPDATE file SET blake3 = ?1 WHERE id = ?2",
        (vec![6u8; 32], b),
    );
    rank(&lib);
    assert_eq!(best_of(&lib, track), a);
    assert_eq!(choices(&lib).len(), 1);
}

#[test]
fn the_details_panel_reads_the_tags_of_the_file_a_choice_made_the_table_show() {
    let (lib, track, a, b) = two_files();
    lib.tag(a, &[("Name", "Best Title")]);
    lib.tag(b, &[("Name", "Chosen Title")]);
    choose(&lib, b).unwrap();
    lib.settle();

    // The file and path the table shows for the track, as the panel gets
    // them with the selected row.
    let q = TableQuery {
        source: TableSource::AllMusic,
        search: None,
        filters: Vec::new(),
        sort: None,
        offset: 0,
        limit: 100,
        with_total: true,
    };
    let shown = lib
        .writer
        .call(move |c| {
            let found = crate::track_table::read(c, &Mount, &q)?.expect("All music exists");
            let dirs = crate::fragile::FragileDirs {
                downloads: None,
                temp: Vec::new(),
            };
            let page = crate::track_table::page(found, 0, &Mount, &dirs);
            Ok(page.rows[0].file.clone().expect("a file"))
        })
        .unwrap();
    assert_eq!(shown.id, b, "the choice is the file the table shows");

    let sources = lib
        .writer
        .call(move |c| crate::send_values::tag_sources(c, &Mount, track, Some(shown.id)))
        .unwrap();
    let title = sources
        .fields
        .iter()
        .find(|f| f.field == crate::send_values::TagField::Title)
        .expect("a title");
    let first = title.shown.as_ref().expect("a value");
    assert_eq!(first.value, "Chosen Title");
    assert_eq!(first.file.as_ref().map(|f| f.id), Some(b));
    // The file it replaced is listed, naming its file.
    assert_eq!(title.others.len(), 1);
    assert_eq!(title.others[0].file.as_ref().map(|f| f.id), Some(a));
}
