//! 1aG-2: a Library started fresh, sent and read back.
//!
//! The steps, each a real command or job: add the music folder → scan →
//! "Start fresh" (the Library stays empty) → add tracks from All music →
//! make a crate and add some to it → prepare a send
//! (which reads rekordbox's export and matches it) → write it → read the
//! file back with the app's reader.

use std::fs;

use serde_json::{json, Value};
use tracklist_pro_lib::crates::CrateId;
use tracklist_pro_lib::library::LibraryTrack;
use tracklist_pro_lib::rekordbox;
use tracklist_pro_lib::send::{
    LeftOutReason, LosesEntries, Refusal, RefusalReason, SendFailure, TreeKind,
};

use super::fixtures::{
    assert_is_rekordboxs_own_entry, children, collection, entries, songs, track_at, KNOWN,
    NEVER_ON_DISK, TRACKS_IN_ALL_MUSIC, TWINS, UNKNOWN,
};
use super::harness::{Song, World, SONG_SECONDS};
use super::rekordbox_side::{key_of, Rekordbox, ANALYSIS};

/// The run up to an empty Library over a scanned music folder. rekordbox
/// has saved its export, but the app hasn't read it: a send will.
fn started_fresh() -> (World, Rekordbox) {
    let mut world = World::new(&songs());
    let rb = collection(&world);
    world.rekordbox_saves(&rb.export());
    world.add_music_folder_and_scan();
    assert_eq!(world.library(), []);
    (world, rb)
}

/// What [`fresh_library_with_a_crate`] put in the Library.
struct Added {
    /// Two tracks rekordbox has never seen, in the crate.
    new: [LibraryTrack; 2],
    /// A track rekordbox has, in the crate.
    known: LibraryTrack,
    /// The crate `Friday`: `new[0]`, `known`, `new[1]`.
    friday: CrateId,
}

const NEW: [Song; 2] = [UNKNOWN[0], UNKNOWN[1]];
const IN_CRATE: Song = KNOWN[0];
/// A track rekordbox has that's in the Library but in no crate.
const IN_NO_CRATE: Song = KNOWN[3];

fn fresh_library_with_a_crate(world: &World) -> Added {
    // Added to the Library in the order the crate will hold them.
    let first = world.add_from_all_music(&NEW[0]);
    let known = world.add_from_all_music(&IN_CRATE);
    let last = world.add_from_all_music(&NEW[1]);
    world.add_from_all_music(&IN_NO_CRATE);
    let friday = world.create_crate("Friday", &[first.id, known.id, last.id]);
    Added {
        new: [first, last],
        known,
        friday,
    }
}

/// Fails unless `sent` is a new track made from `song`'s file: its tag
/// values and file facts, a TrackID above every one rekordbox has, and
/// nothing that is rekordbox's to decide (analysis, plays, rating).
fn assert_is_a_new_track(world: &World, sent: &rekordbox::Track, song: &Song, rb: &Rekordbox) {
    let path = world.path_of(song);
    assert!(
        sent.track_id.unwrap() > rb.highest_id(),
        "{}: TrackID {:?} isn't above rekordbox's highest, {}",
        song.rel,
        sent.track_id,
        rb.highest_id()
    );
    assert_eq!(sent.name(), song.title);
    assert_eq!(sent.artist(), song.artist);
    assert_eq!(sent.genre(), song.genre);
    assert_eq!(sent.kind(), "WAV File");
    assert_eq!(sent.total_time, Some(SONG_SECONDS));
    assert_eq!(sent.size, Some(fs::metadata(&path).unwrap().len()));
    assert_eq!(sent.sample_rate, Some(8000));
    assert_eq!(sent.location.as_ref().unwrap().match_key(), key_of(&path));
    for theirs in ANALYSIS
        .into_iter()
        .chain(["PlayCount", "Rating", "Colour"])
    {
        assert_eq!(
            sent.attrs.get(theirs),
            None,
            "{}: {theirs} is sent",
            song.rel
        );
    }
    assert!(sent.tempos.is_empty() && sent.cues.is_empty());
}

/// 1cA-5: the columns each screen shows are the user's choice, kept in
/// `ui_setting` and not in the Library, so starting fresh (an empty Library
/// over the scanned folder) and what follows it leave them as they were.
#[test]
fn starting_fresh_keeps_the_column_choices() {
    let mut world = World::new(&songs());
    let choices = [
        ("library", vec!["title", "bpm"]),
        ("allMusic", vec!["title", "artist", "file"]),
        ("crate", vec!["title", "genre"]),
    ];
    // The widths dragged to (1cB-21) are part of the choice and are kept too.
    let stored = |shown: &[&str]| json!({ "shown": shown, "known": shown, "widths": { "title": 300, "bpm": 80 } });
    for (screen, shown) in &choices {
        let _: Value = world.call(
            "set_table_columns",
            json!({ "screen": screen, "columns": stored(shown) }),
        );
    }
    assert_eq!(world.count("SELECT count(*) FROM ui_setting"), 3);

    // The fresh start, and the first things done after it: tracks added, a
    // crate made, a send written.
    let rb = collection(&world);
    world.rekordbox_saves(&rb.export());
    world.add_music_folder_and_scan();
    assert_eq!(world.library(), []);
    fresh_library_with_a_crate(&world);
    world.send();

    assert_eq!(world.count("SELECT count(*) FROM ui_setting"), 3);
    for (screen, shown) in &choices {
        let kept: Option<Value> = world.call("table_columns", json!({ "screen": screen }));
        assert_eq!(kept, Some(stored(shown)), "{screen}");
    }
}

#[test]
fn a_fresh_library_is_empty_and_all_music_shows_every_track_once() {
    let (world, _rb) = started_fresh();
    let all = world.all_music();
    assert_eq!(all.total, Some(TRACKS_IN_ALL_MUSIC));
    assert_eq!(all.rows.len(), TRACKS_IN_ALL_MUSIC as usize);
    assert!(all.rows.iter().all(|t| !t.in_library));
}

#[test]
fn new_tracks_arrive_with_their_tag_values_a_new_track_id_and_their_crate() {
    let (world, rb) = started_fresh();
    fresh_library_with_a_crate(&world);
    assert_eq!(world.library().len(), 4);

    let preflight = world.prepared();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (2, 1));
    assert_eq!(preflight.left_out, []);
    assert_eq!(preflight.other_file, []);
    assert!(!preflight.needs_confirm);
    let sent = world.write_and_read_back(&preflight, false);

    // The two new tracks, and the known one the crate names. The known
    // track in no crate isn't sent (it would only raise a dialog).
    assert_eq!(sent.tracks.len(), 3);
    for song in &NEW {
        let entry = track_at(&sent, &world.path_of(song)).unwrap();
        assert_is_a_new_track(&world, entry, song, &rb);
    }
    let ids: Vec<_> = sent.tracks.iter().map(|t| t.track_id.unwrap()).collect();
    assert!(
        ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2],
        "{ids:?}"
    );
    // rekordbox's highest TrackID belongs to a track that isn't sent: the
    // new ids are above the whole read, not just above what's in the file.
    assert_eq!(
        rb.at(&world.path_of(&NEVER_ON_DISK)).unwrap().id,
        rb.highest_id()
    );
    let known = world.path_of(&IN_CRATE);
    assert_is_rekordboxs_own_entry(track_at(&sent, &known).unwrap(), rb.at(&known).unwrap());
    assert!(track_at(&sent, &world.path_of(&IN_NO_CRATE)).is_none());

    // The crate, under the app's folder, with its entries in order.
    assert_eq!(children(&sent, &[]), ["Crates", "Playlists"]);
    assert_eq!(children(&sent, &["Crates"]), ["Friday"]);
    assert_eq!(children(&sent, &["Playlists"]), [] as [&str; 0]);
    assert_eq!(
        entries(&sent, &["Crates", "Friday"]),
        [NEW[0], IN_CRATE, NEW[1]].map(|s| key_of(&world.path_of(&s)))
    );
}

/// 1cA-9/10: crates in nested folders reach rekordbox as the same tree,
/// after a rename, a move and a delete on the way, in the order the app
/// lists them.
#[test]
fn crates_in_nested_folders_are_sent_as_the_same_tree_after_moves_and_deletes() {
    let (world, _rb) = started_fresh();
    let added = fresh_library_with_a_crate(&world);
    let (a, b) = (added.new[0].id, added.new[1].id);

    let residency = world.create_folder("Friday residency", None);
    let late = world.create_folder("Late", Some(residency));
    world.create_crate_in(residency, "Warmup", &[a, added.known.id]);
    let peak = world.create_crate_in(late, "Peak time", &[b]);
    let scratch = world.create_folder("Scratch", None);
    world.create_crate_in(scratch, "Never sent", &[a]);
    // The crate `Friday` moves in beside Warmup, Peak time up a level, and
    // the folder holding a throwaway crate goes with its crate.
    world.move_crate(added.friday, Some(residency));
    world.move_crate(peak, Some(residency));
    world.delete_crate(scratch);
    world.rename_crate(late, "Later");

    let preflight = world.prepared();
    let sent = world.write_and_read_back(&preflight, false);
    assert_eq!(children(&sent, &["Crates"]), ["Friday residency"]);
    assert_eq!(
        children(&sent, &["Crates", "Friday residency"]),
        ["Later", "Warmup", "Friday", "Peak time"]
    );
    assert_eq!(
        children(&sent, &["Crates", "Friday residency", "Later"]),
        [] as [&str; 0]
    );
    let keys = |songs: &[Song]| -> Vec<String> {
        songs.iter().map(|s| key_of(&world.path_of(s))).collect()
    };
    assert_eq!(
        entries(&sent, &["Crates", "Friday residency", "Warmup"]),
        keys(&[NEW[0], IN_CRATE])
    );
    assert_eq!(
        entries(&sent, &["Crates", "Friday residency", "Friday"]),
        keys(&[NEW[0], IN_CRATE, NEW[1]])
    );
    assert_eq!(
        entries(&sent, &["Crates", "Friday residency", "Peak time"]),
        keys(&[NEW[1]])
    );
    assert_eq!(children(&sent, &["Playlists"]), [] as [&str; 0]);
}

#[test]
fn sending_a_fresh_library_again_duplicates_nothing() {
    let (mut world, mut rb) = started_fresh();
    fresh_library_with_a_crate(&world);

    let first = world.send();
    let first_bytes = world.sent_bytes();
    // Not imported yet: rekordbox exports the same collection again, and
    // the second send is the same file.
    world.rekordbox_saves(&rb.export());
    world.send();
    assert_eq!(world.sent_bytes(), first_bytes, "the second send's file");

    // rekordbox imports the file: two tracks added, one dialog.
    let before = rb.tracks.len();
    assert_eq!(rb.import(&first), 1);
    assert_eq!(rb.tracks.len(), before + 2);
    world.rekordbox_saves(&rb.export());

    // Now rekordbox has all three: each goes out as rekordbox's own entry
    // (the TrackID rekordbox gave it, no analysis), and importing that
    // adds nothing.
    let preflight = world.prepared();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (0, 3));
    let again = world.write_and_read_back(&preflight, false);
    assert_eq!(again.tracks.len(), 3);
    for song in NEW.iter().chain([&IN_CRATE]) {
        let path = world.path_of(song);
        assert_is_rekordboxs_own_entry(track_at(&again, &path).unwrap(), rb.at(&path).unwrap());
    }
    let imported = rb.clone();
    assert_eq!(rb.import(&again), 3);
    assert_eq!(rb.tracks, imported.tracks);
    assert_eq!(rb.playlists, imported.playlists);
    assert_eq!(
        world.after_send_lists(),
        json!({ "playlistsChecked": true, "stalePlaylists": [], "manualRemovals": [] })
    );
}

/// The sequence that put duplicates in rekordbox: send new tracks, import
/// them, then send again without exporting. The old export doesn't know
/// rekordbox has them now, so they'd go out as new once more. The
/// checklist asks; the true answer here is "yes, a send was imported".
#[test]
fn after_an_import_a_send_from_the_export_from_before_it_is_refused_on_the_users_yes() {
    let (mut world, mut rb) = started_fresh();
    fresh_library_with_a_crate(&world);
    let first = world.send();
    let first_bytes = world.sent_bytes();
    rb.import(&first);

    // "Read export" again, with no new export: the question is raised, and
    // the review still counts the imported tracks as new.
    let state = world.prepare_send();
    assert_eq!(state.failure, None);
    let preflight = state.preflight.unwrap();
    assert!(preflight.export_older_than_last_send);
    assert_eq!(preflight.refusal, None);
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (2, 1));

    // Unanswered, then answered "a send was imported since": not written.
    for answer in [None, Some(true)] {
        let state = world.write_send_answering(&preflight.token, true, answer);
        assert_eq!(state.failure, Some(SendFailure::ExportOlderThanLastSend));
        assert_eq!(state.sent, None);
        assert_eq!(world.sent_bytes(), first_bytes, "nothing is written");
    }

    // rekordbox exports: no question, the send goes, and the tracks it now
    // has are known, not new.
    world.rekordbox_saves(&rb.export());
    let preflight = world.prepared();
    assert!(!preflight.export_older_than_last_send);
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (0, 3));
    world.write_and_read_back(&preflight, false);
}

/// The other case the app can't tell from that one (DOGFOOD K16): the
/// first send was never imported. The user says so, and the send goes
/// from the same export, exactly as it would from a new one.
#[test]
fn a_send_that_was_never_imported_can_be_followed_by_another_from_the_same_export_on_the_users_no()
{
    let (mut world, rb) = started_fresh();
    let added = fresh_library_with_a_crate(&world);
    world.send();
    // Nothing was imported. The Library moves on: one more track.
    let extra = world.add_from_all_music(&UNKNOWN[2]);
    world.create_crate("Saturday", &[added.new[0].id, extra.id]);

    let preflight = world.prepared();
    assert!(preflight.export_older_than_last_send);
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (3, 1));
    let state = world.write_send_answering(&preflight.token, false, Some(false));
    assert_eq!(state.failure, None);
    assert!(state.sent.is_some());
    let on_the_answer = world.sent_bytes();
    let sent = world.read_sent_file();
    assert_eq!(sent.tracks.len(), 4);
    assert!(track_at(&sent, &world.path_of(&UNKNOWN[2])).is_some());

    // rekordbox exports the same collection again: no question this time,
    // and the file is the same, byte for byte.
    world.rekordbox_saves(&rb.export());
    let fresh = world.prepared();
    assert!(!fresh.export_older_than_last_send);
    world.write_and_read_back(&fresh, false);
    assert_eq!(world.sent_bytes(), on_the_answer);
}

#[test]
fn after_a_crate_is_renamed_and_a_track_removed_the_lists_name_the_old_crate_and_the_track() {
    let (mut world, mut rb) = started_fresh();
    let added = fresh_library_with_a_crate(&world);
    let removed = NEW[1];

    // The first send is imported, and rekordbox saves a new export.
    rb.import(&world.send());
    world.rekordbox_saves(&rb.export());

    world.rename_crate(added.friday, "Saturday");
    world.remove_from_library(added.new[1].id);
    let sent = world.send();

    // The send carries the crate under its new name, without the track.
    assert_eq!(children(&sent, &["Crates"]), ["Saturday"]);
    assert_eq!(
        entries(&sent, &["Crates", "Saturday"]),
        [NEW[0], IN_CRATE].map(|s| key_of(&world.path_of(&s)))
    );
    assert!(track_at(&sent, &world.path_of(&removed)).is_none());
    assert_eq!(added.known.id, world.library_track(&IN_CRATE).id);

    // An import can't delete either from rekordbox, so the lists name them.
    let lists = world.after_send_lists();
    assert_eq!(lists["playlistsChecked"], true);
    let stale = lists["stalePlaylists"].as_array().unwrap();
    assert_eq!(stale.len(), 1, "{stale:?}");
    assert_eq!(stale[0]["path"], json!(["Crates", "Friday"]));
    assert_eq!(stale[0]["kind"], "playlist");
    let removals = lists["manualRemovals"].as_array().unwrap();
    assert_eq!(removals.len(), 1, "{removals:?}");
    // Named by where it was sent: nothing gives a track a title of its own
    // in Phase 1a (the lists show the file's name instead).
    assert_eq!(
        key_of(removals[0]["path"].as_str().unwrap().as_ref()),
        key_of(&world.path_of(&removed))
    );

    // The user deletes both in rekordbox; the next read clears the lists.
    rb.import(&sent);
    rb.delete_playlist(&["Crates", "Friday"]);
    let key = key_of(&world.path_of(&removed));
    rb.tracks.retain(|t| t.location_key() != key);
    world.rekordbox_saves(&rb.export());
    world.read_rekordbox();
    assert_eq!(
        world.after_send_lists(),
        json!({ "playlistsChecked": true, "stalePlaylists": [], "manualRemovals": [] })
    );
}

#[test]
fn a_new_track_whose_file_is_missing_is_left_out_with_the_reason_and_the_send_needs_a_confirm() {
    let (mut world, _rb) = started_fresh();
    fresh_library_with_a_crate(&world);
    let gone = NEW[1];

    world.user_deletes(&gone);
    world.scan();
    assert!(world.library_track(&gone).source_missing);

    // rekordbox has no entry for it, so there's nothing to send in its
    // place: it's left out, named, and its crate loses the entry.
    let preflight = world.prepared();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 1));
    assert_eq!(preflight.left_out.len(), 1);
    assert_eq!(preflight.left_out[0].reason, LeftOutReason::FileMissing);
    assert_eq!(
        preflight.left_out[0].track.file_name.as_deref(),
        Some(gone.file_name())
    );
    assert_eq!(preflight.file_missing, []);
    assert_eq!(
        preflight.loses_entries,
        [LosesEntries {
            kind: TreeKind::Crate,
            path: vec!["Friday".into()],
            lost: 1,
            entries: 3,
        }]
    );
    assert!(preflight.can_send && preflight.needs_confirm);

    // Without the confirm nothing is written.
    let state = world.write_send(&preflight.token, false);
    assert_eq!(state.failure, Some(SendFailure::NotConfirmed));
    assert!(!world.sent_file().exists());

    let sent = world.write_and_read_back(&preflight, true);
    assert_eq!(sent.tracks.len(), 2);
    assert!(track_at(&sent, &world.path_of(&gone)).is_none());
    assert_eq!(
        entries(&sent, &["Crates", "Friday"]),
        [NEW[0], IN_CRATE].map(|s| key_of(&world.path_of(&s)))
    );
}

/// The twin "Add to Library" links, and the other one: All music shows the
/// twins as one track, and whichever file the add links, rekordbox gets
/// the other one in each test.
fn add_a_twin(world: &World) -> (LibraryTrack, Song, Song) {
    let row = world
        .all_music()
        .rows
        .into_iter()
        .find(|t| {
            let name = t.file.as_ref().map(|f| f.name.as_str());
            TWINS.iter().any(|twin| name == Some(twin.file_name()))
        })
        .unwrap();
    let shown = row.file.unwrap().name;
    let (linked, other) = if shown == TWINS[0].file_name() {
        (TWINS[0], TWINS[1])
    } else {
        (TWINS[1], TWINS[0])
    };
    (world.add_from_all_music(&linked), linked, other)
}

#[test]
fn a_track_rekordbox_holds_under_a_byte_identical_file_is_sent_as_that_entry() {
    // The twins are byte-identical (1bE-11): the Library track is sent as
    // rekordbox's entry for the other file, with rekordbox's own Location,
    // so an import adds no second entry and keeps the cues.
    let (mut world, mut rb) = started_fresh();
    let (track, _linked, other) = add_a_twin(&world);
    world.create_crate("Friday", &[track.id]);
    let theirs = rb.has(&world.path_of(&other), &other, 12, "126.00", "5A");
    world.rekordbox_saves(&rb.export());

    let preflight = world.prepared();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (0, 1));
    assert_eq!(preflight.other_file.len(), 0);
    assert!(preflight.can_send && !preflight.needs_confirm);

    let sent = world.write_and_read_back(&preflight, false);
    assert_eq!(sent.tracks.len(), 1);
    let entry = rb.at(&world.path_of(&other)).unwrap().clone();
    assert_eq!(entry.id, theirs);
    assert_is_rekordboxs_own_entry(&sent.tracks[0], &entry);
    assert_eq!(
        sent.tracks[0].location.as_ref().unwrap().match_key(),
        key_of(&world.path_of(&other))
    );
    let before = rb.tracks.len();
    assert_eq!(rb.import(&sent), 1);
    assert_eq!(rb.tracks.len(), before);
}

#[test]
fn a_track_rekordbox_holds_under_a_file_with_other_tags_is_flagged_and_arrives_as_a_second_entry() {
    let (mut world, mut rb) = started_fresh();
    let (_track, linked, other) = add_a_twin(&world);
    // rekordbox's file holds the same audio with another tag (§5.1): not a
    // byte-identical copy.
    let retagged = Song {
        genre: "Deep House",
        ..other
    };
    world.user_adds(&retagged);
    world.scan();
    let theirs = rb.has(&world.path_of(&other), &retagged, 12, "126.00", "5A");
    world.rekordbox_saves(&rb.export());

    let preflight = world.prepared();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (1, 0));
    assert_eq!(preflight.other_file.len(), 1);
    let flagged = &preflight.other_file[0];
    let key = |path: &Option<String>| key_of(path.as_deref().unwrap().as_ref());
    assert_eq!(key(&flagged.library_file), key_of(&world.path_of(&linked)));
    assert_eq!(key(&flagged.rekordbox_file), key_of(&world.path_of(&other)));
    // A warning, not a stop.
    assert!(preflight.can_send && !preflight.needs_confirm);

    // The Library's file goes out as a new track; rekordbox's entry for
    // the other file isn't sent, and an import leaves it as it was.
    let sent = world.write_and_read_back(&preflight, false);
    assert_eq!(sent.tracks.len(), 1);
    assert_is_a_new_track(&world, &sent.tracks[0], &linked, &rb);
    let before = rb.clone();
    assert_eq!(rb.import(&sent), 0);
    assert_eq!(rb.tracks.len(), before.tracks.len() + 1);
    assert_eq!(rb.tracks[..before.tracks.len()], before.tracks[..]);
    assert_eq!(rb.at(&world.path_of(&other)).unwrap().id, theirs);
}

// --- 1cC-3: the crate tree, sent end to end ---------------------------------

/// The `Location` keys of `songs`, as the file read back spells them.
fn keys_of(world: &World, songs: &[Song]) -> Vec<String> {
    songs.iter().map(|s| key_of(&world.path_of(s))).collect()
}

/// How often a track at `song`'s path is in the file's COLLECTION.
fn times_in_collection(world: &World, sent: &rekordbox::RekordboxXml, song: &Song) -> usize {
    let key = key_of(&world.path_of(song));
    sent.tracks
        .iter()
        .filter(|t| t.location.as_ref().is_ok_and(|l| l.match_key() == key))
        .count()
}

/// Every playlist of the file, as `Crates/Folder/Name`, sorted.
fn playlist_paths(sent: &rekordbox::RekordboxXml) -> Vec<String> {
    let mut all: Vec<String> = sent
        .playlists
        .playlists()
        .iter()
        .map(|(at, playlist)| format!("{}/{}", at.join("/"), playlist.name))
        .collect();
    all.sort();
    all
}

/// "Add to crate" for one track, a moment after the one before, so that the
/// crate's order is the order of the adds and not the Library's.
fn add_to_crate_later(world: &World, id: CrateId, track: &LibraryTrack) {
    std::thread::sleep(std::time::Duration::from_millis(5));
    let added: Value = world.call(
        "add_tracks_to_crate",
        json!({ "id": id, "tracks": [track.id] }),
    );
    assert_eq!(added["changed"], 1, "{added}");
}

/// A crate with nothing in it, made inside `parent`.
fn create_empty_crate_in(world: &World, parent: CrateId, name: &str) -> CrateId {
    world.call("create_crate", json!({ "name": name, "parent": parent }))
}

/// Folders nested `levels` deep (`Level 1` at the top, each inside the one
/// before), made one "New folder" at a time. Returns their names and the
/// innermost one.
fn folders_nested(world: &World, levels: usize) -> (Vec<String>, CrateId) {
    let mut names = Vec::new();
    let mut parent = None;
    for level in 1..=levels {
        let name = format!("Level {level}");
        parent = Some(world.create_folder(&name, parent));
        names.push(name);
    }
    (names, parent.unwrap())
}

/// The path of a node of the depth tests' tree, from `Crates` down: the
/// nested folders, then `last`.
fn path_to(names: &[String], last: &str) -> Vec<String> {
    std::iter::once("Crates".to_owned())
        .chain(names.iter().cloned())
        .chain(std::iter::once(last.to_owned()))
        .collect()
}

/// 1cC-3: folders four deep, crates at every level, siblings in the order
/// they were made (not the alphabet's) and each crate's tracks in the order
/// they were added: the file holds the same tree, at the exact paths.
#[test]
fn a_nested_crate_tree_is_sent_at_the_exact_paths_with_its_orders() {
    let (world, _rb) = started_fresh();
    // The Library's order is not the files' alphabetical order.
    let n9 = world.add_from_all_music(&UNKNOWN[2]);
    let n7 = world.add_from_all_music(&UNKNOWN[0]);
    let n8 = world.add_from_all_music(&UNKNOWN[1]);
    let k1 = world.add_from_all_music(&KNOWN[1]);
    let k0 = world.add_from_all_music(&KNOWN[0]);

    // Siblings are made in the reverse of the alphabet, folders and
    // crates mixed.
    let zulu = world.create_folder("Zulu", None);
    world.create_crate("Top crate", &[n9.id, k0.id]);
    let alpha = world.create_folder("Alpha", None);

    let yankee = world.create_folder("Yankee", Some(zulu));
    world.create_crate_in(zulu, "Xray", &[n8.id]);
    world.create_folder("Mike", Some(zulu));

    // n9 joined the Library first, but joins this crate last.
    let whiskey = world.create_crate_in(yankee, "Whiskey", &[n7.id]);
    add_to_crate_later(&world, whiskey, &n9);
    let victor = world.create_folder("Victor", Some(yankee));
    world.create_crate_in(victor, "Uniform", &[n7.id, k1.id, k0.id]);
    world.create_crate_in(victor, "Tango", &[n8.id]);
    let oscar = world.create_folder("Oscar", Some(victor));
    world.create_crate_in(oscar, "November", &[k0.id]);

    world.create_crate_in(alpha, "Sierra", &[k0.id]);
    let romeo = world.create_folder("Romeo", Some(alpha));
    let quebec = world.create_folder("Quebec", Some(romeo));
    world.create_crate_in(quebec, "Papa", &[n9.id, k1.id]);

    let preflight = world.prepared();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (3, 2));
    assert!(preflight.left_out.is_empty() && preflight.loses_entries.is_empty());
    let sent = world.write_and_read_back(&preflight, false);

    // Folders and crates, siblings in the order they were made.
    assert_eq!(children(&sent, &["Crates"]), ["Zulu", "Top crate", "Alpha"]);
    assert_eq!(
        children(&sent, &["Crates", "Zulu"]),
        ["Yankee", "Xray", "Mike"]
    );
    assert_eq!(
        children(&sent, &["Crates", "Zulu", "Yankee"]),
        ["Whiskey", "Victor"]
    );
    assert_eq!(
        children(&sent, &["Crates", "Zulu", "Yankee", "Victor"]),
        ["Uniform", "Tango", "Oscar"]
    );
    assert_eq!(
        children(&sent, &["Crates", "Zulu", "Yankee", "Victor", "Oscar"]),
        ["November"]
    );
    assert_eq!(children(&sent, &["Crates", "Alpha"]), ["Sierra", "Romeo"]);
    assert_eq!(children(&sent, &["Crates", "Alpha", "Romeo"]), ["Quebec"]);
    assert_eq!(
        children(&sent, &["Crates", "Alpha", "Romeo", "Quebec"]),
        ["Papa"]
    );
    assert_eq!(
        children(&sent, &["Crates", "Zulu", "Mike"]),
        [] as [&str; 0]
    );
    assert_eq!(children(&sent, &["Playlists"]), [] as [&str; 0]);
    assert_eq!(
        playlist_paths(&sent),
        [
            "Crates/Alpha/Romeo/Quebec/Papa",
            "Crates/Alpha/Sierra",
            "Crates/Top crate",
            "Crates/Zulu/Xray",
            "Crates/Zulu/Yankee/Victor/Oscar/November",
            "Crates/Zulu/Yankee/Victor/Tango",
            "Crates/Zulu/Yankee/Victor/Uniform",
            "Crates/Zulu/Yankee/Whiskey",
        ]
    );

    // Tracks in the order they were added to each crate.
    let keys = |songs: &[Song]| keys_of(&world, songs);
    let (s7, s8, s9) = (UNKNOWN[0], UNKNOWN[1], UNKNOWN[2]);
    let (c0, c1) = (KNOWN[0], KNOWN[1]);
    let at = |path: &[&str]| entries(&sent, path);
    assert_eq!(at(&["Crates", "Top crate"]), keys(&[s9, c0]));
    assert_eq!(at(&["Crates", "Zulu", "Xray"]), keys(&[s8]));
    assert_eq!(
        at(&["Crates", "Zulu", "Yankee", "Whiskey"]),
        keys(&[s7, s9]),
        "the track added last is last, whatever the Library's order"
    );
    assert_eq!(
        at(&["Crates", "Zulu", "Yankee", "Victor", "Uniform"]),
        keys(&[s7, c1, c0])
    );
    assert_eq!(
        at(&["Crates", "Zulu", "Yankee", "Victor", "Tango"]),
        keys(&[s8])
    );
    assert_eq!(
        at(&["Crates", "Zulu", "Yankee", "Victor", "Oscar", "November"]),
        keys(&[c0])
    );
    assert_eq!(at(&["Crates", "Alpha", "Sierra"]), keys(&[c0]));
    assert_eq!(
        at(&["Crates", "Alpha", "Romeo", "Quebec", "Papa"]),
        keys(&[s9, c1])
    );

    // Every track once, though most are in several crates.
    assert_eq!(sent.tracks.len(), 5);
    for song in [s7, s8, s9, c0, c1] {
        assert_eq!(times_in_collection(&world, &sent, &song), 1, "{}", song.rel);
    }
}

/// 1cC-3: what a tree holds that is hollow or shared. Nothing in a crate,
/// nothing in a folder, a crate whose every track is left out, and a track
/// in two crates.
#[test]
fn empty_crates_empty_folders_and_shared_tracks_are_sent_as_they_are() {
    let (mut world, _rb) = started_fresh();
    let a = world.add_from_all_music(&UNKNOWN[0]);
    let b = world.add_from_all_music(&UNKNOWN[1]);
    let lost = world.add_from_all_music(&UNKNOWN[2]);
    let k = world.add_from_all_music(&KNOWN[0]);

    let gig = world.create_folder("Gig", None);
    create_empty_crate_in(&world, gig, "Empty crate");
    world.create_folder("Empty folder", Some(gig));
    let hollow = world.create_folder("Hollow", Some(gig));
    world.create_folder("Hollower", Some(hollow));
    world.create_crate_in(gig, "Left out", &[lost.id]);
    world.create_crate_in(gig, "One", &[a.id, k.id]);
    world.create_crate_in(gig, "Two", &[a.id, b.id]);
    world.create_folder("Unused", None);

    // The one track of "Left out" is gone from disk, and rekordbox has no
    // entry to send in its place.
    world.user_deletes(&UNKNOWN[2]);
    world.scan();
    assert!(world.library_track(&UNKNOWN[2]).source_missing);

    let preflight = world.prepared();
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (2, 1));
    assert_eq!(preflight.left_out.len(), 1);
    assert_eq!(preflight.left_out[0].reason, LeftOutReason::FileMissing);
    assert_eq!(
        preflight.loses_entries,
        [LosesEntries {
            kind: TreeKind::Crate,
            path: vec!["Gig".into(), "Left out".into()],
            lost: 1,
            entries: 1,
        }]
    );
    assert!(preflight.can_send && preflight.needs_confirm);
    let sent = world.write_and_read_back(&preflight, true);

    // The empty folders are folders, and every crate is written, the one
    // that lost its only track too.
    assert_eq!(children(&sent, &["Crates"]), ["Gig", "Unused"]);
    assert_eq!(children(&sent, &["Crates", "Unused"]), [] as [&str; 0]);
    assert_eq!(
        children(&sent, &["Crates", "Gig"]),
        [
            "Empty crate",
            "Empty folder",
            "Hollow",
            "Left out",
            "One",
            "Two"
        ]
    );
    assert_eq!(
        children(&sent, &["Crates", "Gig", "Empty folder"]),
        [] as [&str; 0]
    );
    assert_eq!(children(&sent, &["Crates", "Gig", "Hollow"]), ["Hollower"]);
    assert_eq!(
        children(&sent, &["Crates", "Gig", "Hollow", "Hollower"]),
        [] as [&str; 0]
    );
    assert_eq!(
        playlist_paths(&sent),
        [
            "Crates/Gig/Empty crate",
            "Crates/Gig/Left out",
            "Crates/Gig/One",
            "Crates/Gig/Two",
        ]
    );
    assert_eq!(
        entries(&sent, &["Crates", "Gig", "Empty crate"]),
        [] as [String; 0]
    );
    assert_eq!(
        entries(&sent, &["Crates", "Gig", "Left out"]),
        [] as [String; 0]
    );

    // The track in two crates is in both, and in COLLECTION once; the
    // track that was left out is not there at all.
    let keys = |songs: &[Song]| keys_of(&world, songs);
    assert_eq!(
        entries(&sent, &["Crates", "Gig", "One"]),
        keys(&[UNKNOWN[0], KNOWN[0]])
    );
    assert_eq!(
        entries(&sent, &["Crates", "Gig", "Two"]),
        keys(&[UNKNOWN[0], UNKNOWN[1]])
    );
    assert_eq!(sent.tracks.len(), 3);
    for song in [UNKNOWN[0], UNKNOWN[1], KNOWN[0]] {
        assert_eq!(times_in_collection(&world, &sent, &song), 1, "{}", song.rel);
    }
    assert_eq!(times_in_collection(&world, &sent, &UNKNOWN[2]), 0);
}

/// 1cC-3: 63 folders one inside the other (with the `Crates` folder, the 64
/// levels a send can write) is sent, crate at the bottom and all; the app
/// itself won't make a 64th.
#[test]
fn a_tree_63_folders_deep_is_sent_and_a_64th_folder_is_refused_by_the_app() {
    let (world, _rb) = started_fresh();
    let track = world.add_from_all_music(&UNKNOWN[0]);
    let (names, deepest) = folders_nested(&world, 63);
    world.create_crate_in(deepest, "Bottom", &[track.id]);
    world.create_crate("Top", &[track.id]);

    // One more folder, however it is made, would be a send that is refused.
    let refused = world
        .try_call::<Value>(
            "create_crate_folder",
            json!({ "name": "Level 64", "parent": deepest }),
        )
        .unwrap_err();
    assert_eq!(refused["kind"], "crateTooDeep");
    let elsewhere = world.create_folder("Elsewhere", None);
    let refused = world
        .try_call::<Value>("move_crate", json!({ "id": elsewhere, "folder": deepest }))
        .unwrap_err();
    assert_eq!(refused["kind"], "crateTooDeep");

    let sent = world.send();
    assert_eq!(
        children(&sent, &["Crates"]),
        ["Level 1", "Top", "Elsewhere"]
    );
    let path = path_to(&names, "Bottom");
    assert_eq!(path.len(), 65);
    // Every level holds the next folder and nothing else, down to the crate.
    for level in 1..=63 {
        let at: Vec<&str> = path[..=level].iter().map(String::as_str).collect();
        let next = &path[level + 1];
        assert_eq!(children(&sent, &at), [next.as_str()], "inside {at:?}");
    }
    let bottom: Vec<&str> = path.iter().map(String::as_str).collect();
    assert_eq!(entries(&sent, &bottom), keys_of(&world, &[UNKNOWN[0]]));
    assert_eq!(times_in_collection(&world, &sent, &UNKNOWN[0]), 1);
}

/// 1cC-3: a tree one folder deeper than a send can write (which the app
/// itself never makes, so it is put in the database by hand, as a damaged
/// or hand-edited one would be) is refused whole, with the reason and the
/// folder named; nothing is written or recorded.
#[test]
fn a_tree_deeper_than_63_folders_is_refused_through_the_real_send_and_nothing_is_written() {
    let (mut world, _rb) = started_fresh();
    let track = world.add_from_all_music(&UNKNOWN[0]);
    let (names, deepest) = folders_nested(&world, 63);
    world.create_crate_in(deepest, "Bottom", &[track.id]);
    world.create_crate("Top", &[track.id]);

    world.kill_and_start_again(move |c| {
        c.execute(
            "INSERT INTO crate (kind, name, parent_id) VALUES ('folder', 'Level 64', ?1)",
            [deepest.0],
        )
        .unwrap();
    });
    world.settle();

    let state = world.prepare_send();
    assert_eq!(state.failure, None);
    let preflight = state.preflight.expect("the prepare step left a preflight");
    assert_eq!(
        preflight.refusal,
        Some(Refusal {
            reason: RefusalReason::TooDeep,
            path: path_to(&names, "Level 64"),
        })
    );
    assert!(!preflight.can_send);

    let state = world.write_send(&preflight.token, true);
    assert_eq!(state.failure, Some(SendFailure::NotSendable));
    assert!(!world.sent_file().exists());
    assert_eq!(world.count("SELECT count(*) FROM sent_playlist"), 0);
    assert_eq!(world.count("SELECT count(*) FROM sent_entry"), 0);
}
