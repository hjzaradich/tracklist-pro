//! 1aG-1: a Library started from rekordbox, sent and read back.
//!
//! The steps, each a real command or job: add the music folder → scan
//! (walk, read, hashes, fingerprints, grouping) → read rekordbox's export
//! → relink → attach → "Start from rekordbox" (add every offered track) →
//! prepare a send → write it → read the file back with the app's reader.

use serde_json::json;
use tracklist_pro_lib::library::{LibraryTrackId, LibraryTrackKind};
use tracklist_pro_lib::send::{RefusalReason, SendFailure};
use tracklist_pro_lib::start::{AddSummary, Offer};

use super::fixtures::{
    assert_is_rekordboxs_own_entry, children, collection, entries, songs, track_at, KNOWN,
    NEVER_ON_DISK, UNKNOWN,
};
use super::harness::{Song, World};
use super::rekordbox_side::{key_of, Rekordbox};

/// The run up to a Library started from rekordbox's whole collection.
fn started_from_rekordbox() -> (World, Rekordbox) {
    let mut world = World::new(&songs());
    let rb = collection(&world);
    world.rekordbox_saves(&rb.export());
    world.add_music_folder_and_scan();
    world.read_rekordbox();

    let offer: Offer = world.call("rekordbox_offer", json!({ "playlists": null }));
    assert_eq!(
        offer,
        Offer {
            to_add: KNOWN.len() as u32,
            already_in_library: 0,
            // The track whose file was never there waits in Review.
            waiting_in_missing: 1,
            waiting_for_confirmation: 0,
        }
    );
    let added: AddSummary = world.call("add_rekordbox_tracks", json!({ "playlists": null }));
    assert_eq!(added.added, KNOWN.len() as u32);
    (world, rb)
}

/// The [`KNOWN`] songs in the order their tracks joined the Library,
/// which is the order a crate holds tracks added to it together.
fn in_library_order(world: &World) -> Vec<(Song, LibraryTrackId)> {
    let mut tracks: Vec<_> = KNOWN
        .iter()
        .map(|s| (*s, world.library_track(s).id))
        .collect();
    tracks.sort_by_key(|(_, id)| *id);
    tracks
}

/// Crates made with the app's own commands, naming every Library track:
/// `Friday` holds the first three, `Everything` all of them. Returns the
/// songs in the order the crates hold them.
fn crates_naming_every_track(world: &World) -> Vec<Song> {
    let (songs, tracks): (Vec<Song>, Vec<LibraryTrackId>) =
        in_library_order(world).into_iter().unzip();
    world.create_crate("Friday", &tracks[..3]);
    world.create_crate("Everything", &tracks);
    songs
}

fn keys_of(world: &World, songs: &[Song]) -> Vec<String> {
    songs.iter().map(|s| key_of(&world.path_of(s))).collect()
}

/// Review → Missing → "Add folder" is one command: adding the folder.
#[test]
fn adding_the_folder_of_missing_tracks_scans_it_and_they_stop_being_missing() {
    let mut world = World::new(&songs());
    let rb = collection(&world);
    world.rekordbox_saves(&rb.export());
    // Only one of the folders rekordbox's tracks live in is a music folder.
    world.add_folder_and_scan(&world.music.join("House"));
    world.read_rekordbox();
    let offer =
        |world: &World| -> Offer { world.call("rekordbox_offer", json!({ "playlists": null })) };
    let in_house = KNOWN.iter().filter(|s| s.rel.starts_with("House/")).count() as u32;
    let elsewhere = KNOWN.len() as u32 - in_house;
    assert_eq!(
        (offer(&world).to_add, offer(&world).waiting_in_missing),
        (in_house, elsewhere + 1)
    );

    // Nothing but the add: no scan is asked for, the app isn't restarted.
    world.add_folder_and_scan(&world.music.join("Techno"));

    let in_techno = KNOWN
        .iter()
        .filter(|s| s.rel.starts_with("Techno/"))
        .count() as u32;
    assert_eq!(
        (offer(&world).to_add, offer(&world).waiting_in_missing),
        (in_house + in_techno, elsewhere - in_techno + 1)
    );
}

#[test]
fn starting_from_rekordbox_links_each_track_to_rekordboxs_file_once() {
    let (world, _rb) = started_from_rekordbox();

    let library = world.library();
    assert_eq!(library.len(), KNOWN.len());
    for song in &KNOWN {
        let track = world.library_track(song);
        assert_eq!(track.kind, LibraryTrackKind::Linked);
        assert!(!track.source_missing);
        let file = track.file.unwrap();
        assert_eq!(key_of(file.path.as_ref()), key_of(&world.path_of(song)));
    }

    // Asking again adds nothing: the offer is used up.
    let again: AddSummary = world.call("add_rekordbox_tracks", json!({ "playlists": null }));
    assert_eq!(
        (again.added, again.already_in_library),
        (0, KNOWN.len() as u32)
    );
    assert_eq!(world.library().len(), KNOWN.len());
}

/// A known track is sent only when a crate names it (rule 4: it would only
/// raise a dialog). So a Library started from rekordbox has nothing to
/// send until a crate is made.
#[test]
fn with_no_crate_a_library_started_from_rekordbox_has_nothing_to_send() {
    let (world, _rb) = started_from_rekordbox();

    let state = world.prepare_send();
    assert_eq!(state.failure, None);
    let preflight = state.preflight.unwrap();
    assert!(preflight.nothing_to_send);
    assert!(!preflight.can_send);
    assert_eq!((preflight.new_tracks, preflight.known_tracks), (0, 0));
    assert_eq!(preflight.left_out, []);

    let state = world.write_send(&preflight.token, false);
    assert_eq!(state.failure, Some(SendFailure::NotSendable));
    assert_eq!(state.sent, None);
    assert!(!world.sent_file().exists(), "nothing is written");
}

#[test]
fn every_known_track_goes_out_as_rekordboxs_own_entry_and_an_import_changes_nothing_in_rekordbox() {
    let (world, mut rb) = started_from_rekordbox();
    let in_crates = crates_naming_every_track(&world);

    let preflight = world.prepared();
    assert_eq!(
        (preflight.new_tracks, preflight.known_tracks),
        (0, KNOWN.len() as u32)
    );
    assert_eq!(preflight.left_out, []);
    assert_eq!(preflight.file_missing, []);
    assert_eq!(preflight.other_file, []);
    assert!(!preflight.needs_confirm);
    let sent = world.write_and_read_back(&preflight, false);

    // COLLECTION: each known track once, as rekordbox's own entry: its
    // TrackID, its values (the title is rekordbox's, not the file tag's),
    // and no BPM, key, grid or cues.
    assert_eq!(sent.tracks.len(), KNOWN.len());
    assert!(sent.is_complete());
    for song in &KNOWN {
        let path = world.path_of(song);
        let entry = track_at(&sent, &path).unwrap_or_else(|| panic!("{} isn't sent", song.rel));
        assert_is_rekordboxs_own_entry(entry, rb.at(&path).unwrap());
        assert_ne!(entry.name(), song.title);
    }
    // The track whose file was never on disk isn't a Library track.
    assert!(track_at(&sent, &world.path_of(&NEVER_ON_DISK)).is_none());

    // PLAYLISTS: the app's two folders, the crates under `Crates`,
    // entries in order.
    assert_eq!(children(&sent, &[]), ["Crates", "Playlists"]);
    assert_eq!(children(&sent, &["Crates"]), ["Friday", "Everything"]);
    assert_eq!(children(&sent, &["Playlists"]), [] as [&str; 0]);
    assert_eq!(
        entries(&sent, &["Crates", "Friday"]),
        keys_of(&world, &in_crates[..3])
    );
    assert_eq!(
        entries(&sent, &["Crates", "Everything"]),
        keys_of(&world, &in_crates)
    );

    // What the file does in rekordbox (by §5.2's rules): one dialog per
    // track, no new entry, and every track exactly as it was: BPM, key,
    // grid, cues, play count and all. The user's own playlists stay.
    let before = rb.clone();
    assert_eq!(rb.import(&sent), KNOWN.len());
    assert_eq!(rb.tracks, before.tracks);
    assert_eq!(rb.playlists[..before.playlists.len()], before.playlists[..]);
    let ids: Vec<u64> = in_crates
        .iter()
        .map(|s| rb.at(&world.path_of(s)).unwrap().id)
        .collect();
    assert_eq!(rb.playlist(&["Crates", "Friday"]), Some(&ids[..3].to_vec()));
    assert_eq!(rb.playlist(&["Crates", "Everything"]), Some(&ids));
}

#[test]
fn sending_again_with_nothing_changed_writes_the_same_file_and_leaves_nothing_to_clean_up() {
    let (mut world, mut rb) = started_from_rekordbox();
    crates_naming_every_track(&world);

    let first = world.send();
    let first_bytes = world.sent_bytes();
    // The first send is never imported: rekordbox exports the same
    // collection again, and the second send is the same file.
    world.rekordbox_saves(&rb.export());
    world.send();
    assert_eq!(world.sent_bytes(), first_bytes, "the second send's file");

    // rekordbox imports the file and saves a new export; the next send
    // reads that one, and still duplicates nothing.
    let tracks_in_rekordbox = rb.tracks.len();
    rb.import(&first);
    world.rekordbox_saves(&rb.export());
    let preflight = world.prepared();
    assert_eq!(
        (preflight.new_tracks, preflight.known_tracks),
        (0, KNOWN.len() as u32)
    );
    let third = world.write_and_read_back(&preflight, false);
    assert_eq!(world.sent_bytes(), first_bytes, "the send after an import");
    rb.import(&third);
    assert_eq!(rb.tracks.len(), tracks_in_rekordbox);

    // Nothing was renamed or removed: both lists are empty, and they were
    // checked against a read.
    assert_eq!(
        world.after_send_lists(),
        json!({ "playlistsChecked": true, "stalePlaylists": [], "manualRemovals": [] })
    );
}

#[test]
fn a_known_track_whose_file_went_missing_is_sent_as_rekordboxs_own_entry_at_its_own_location() {
    let (mut world, rb) = started_from_rekordbox();
    let in_crates = crates_naming_every_track(&world);
    let gone = KNOWN[1];
    let path = world.path_of(&gone);

    world.user_deletes(&gone);
    world.scan();
    assert!(world.library_track(&gone).source_missing);

    let preflight = world.prepared();
    assert_eq!(
        (preflight.new_tracks, preflight.known_tracks),
        (0, KNOWN.len() as u32)
    );
    assert_eq!(preflight.left_out, []);
    assert_eq!(preflight.loses_entries, []);
    assert!(!preflight.needs_confirm);
    let missing: Vec<_> = preflight
        .file_missing
        .iter()
        .map(|t| t.file_name.as_deref())
        .collect();
    assert_eq!(missing, [Some(gone.file_name())]);
    let sent = world.write_and_read_back(&preflight, false);

    // rekordbox's entry goes back untouched, its Location byte for byte
    // (the other tracks' are re-encoded), so the crates naming it are
    // whole.
    let known = rb.at(&path).unwrap();
    let entry = track_at(&sent, &path).unwrap();
    assert_is_rekordboxs_own_entry(entry, known);
    assert_eq!(entry.location_raw(), known.get("Location"));
    assert_eq!(
        entries(&sent, &["Crates", "Everything"]),
        keys_of(&world, &in_crates)
    );
}

#[test]
fn an_incomplete_export_refuses_the_send_and_the_last_file_stays_as_it_was() {
    let (mut world, rb) = started_from_rekordbox();
    crates_naming_every_track(&world);
    world.send();
    let last = world.sent_bytes();

    // The export says it holds one more track than it does.
    world.rekordbox_saves(&rb.export_declaring(rb.tracks.len() + 1));
    let state = world.prepare_send();
    let preflight = state.preflight.unwrap();
    assert_eq!(
        preflight.refusal.as_ref().map(|r| r.reason),
        Some(RefusalReason::IncompleteExport)
    );
    assert!(!preflight.can_send);
    let state = world.write_send(&preflight.token, true);
    assert_eq!(state.failure, Some(SendFailure::NotSendable));
    assert_eq!(state.sent, None);
    assert_eq!(world.sent_bytes(), last);

    // A whole export sends again.
    world.rekordbox_saves(&rb.export());
    world.send();
    assert_eq!(world.sent_bytes(), last);
}

/// The sequence that put a second entry in rekordbox: a file moved since
/// rekordbox imported it is matched by its name alone (a probable match),
/// so "Start from rekordbox" leaves it out; added by hand it was linked as
/// a track rekordbox doesn't know, and sent as new beside rekordbox's own
/// entry. Now it can't be added until the match can be confirmed.
#[test]
fn a_track_whose_rekordbox_match_is_only_probable_cannot_be_added_so_it_is_never_sent_as_new() {
    let mut world = World::new(&songs());
    let mut rb = collection(&world);
    let moved = UNKNOWN[0];
    // rekordbox has the file where it used to be, and no length for it:
    // only the name ties its entry to the file in the music folder.
    let old_place = world.music.join("Old place").join(moved.file_name());
    rb.has(&old_place, &moved, 9, "125.00", "2A");
    rb.tracks.last_mut().unwrap().unset("TotalTime");
    world.rekordbox_saves(&rb.export());
    world.add_music_folder_and_scan();
    world.read_rekordbox();

    let offer: Offer = world.call("rekordbox_offer", json!({ "playlists": null }));
    assert_eq!(offer.waiting_for_confirmation, 1);
    assert_eq!(offer.to_add, KNOWN.len() as u32);
    let added: AddSummary = world.call("add_rekordbox_tracks", json!({ "playlists": null }));
    assert_eq!(added.added, KNOWN.len() as u32);

    // All music marks the row: its Add control is greyed out there.
    let all = world.all_music();
    let held_back: Vec<_> = all
        .tracks
        .iter()
        .filter(|t| t.match_not_confirmed)
        .map(|t| t.file.as_ref().unwrap().name.clone())
        .collect();
    assert_eq!(held_back, [moved.file_name()]);
    let row = all.tracks.iter().find(|t| t.match_not_confirmed).unwrap();

    // And the add itself refuses, whoever asks.
    let refused = world
        .try_call::<serde_json::Value>("promote_track", json!({ "recordingId": row.recording_id }))
        .unwrap_err();
    assert_eq!(refused["kind"], "libraryMatchNotConfirmed");
    assert_eq!(world.library().len(), KNOWN.len());

    // So a send carries rekordbox's own tracks and nothing new.
    crates_naming_every_track(&world);
    let preflight = world.prepared();
    assert_eq!(
        (preflight.new_tracks, preflight.known_tracks),
        (0, KNOWN.len() as u32)
    );
    let sent = world.write_and_read_back(&preflight, false);
    assert!(track_at(&sent, &world.path_of(&moved)).is_none());
    let before = rb.tracks.len();
    rb.import(&sent);
    assert_eq!(rb.tracks.len(), before, "no second entry");
}

/// A Library started from rekordbox, where almost no track is ever sent
/// (no crate names it): a removed track is still rekordbox's to remove.
#[test]
fn a_removed_track_that_was_never_sent_is_listed_while_rekordbox_still_holds_it() {
    let (mut world, mut rb) = started_from_rekordbox();
    let removed = KNOWN[4];
    let path = world.path_of(&removed);
    assert_eq!(world.after_send_lists()["manualRemovals"], json!([]));

    world.remove_from_library(world.library_track(&removed).id);

    let lists = world.after_send_lists();
    let removals = lists["manualRemovals"].as_array().unwrap();
    assert_eq!(removals.len(), 1, "{removals:?}");
    assert_eq!(
        key_of(removals[0]["path"].as_str().unwrap().as_ref()),
        key_of(&path)
    );

    // The user removes it in rekordbox; the next read clears the list.
    let key = key_of(&path);
    rb.tracks.retain(|t| t.location_key() != key);
    world.rekordbox_saves(&rb.export());
    world.read_rekordbox();
    assert_eq!(world.after_send_lists()["manualRemovals"], json!([]));
}

/// A track rekordbox has never had isn't rekordbox's to remove.
#[test]
fn a_removed_track_rekordbox_never_had_is_not_listed() {
    let (world, _rb) = started_from_rekordbox();
    let added = world.add_from_all_music(&UNKNOWN[1]);
    world.remove_from_library(added.id);
    assert_eq!(world.after_send_lists()["manualRemovals"], json!([]));
}

/// Cycle 1's surprise (C1-9): rekordbox has a track at a path where its
/// file no longer is; the app pairs the entry with the file's new place by
/// name and length, and the send (rightly) leaves rekordbox's path alone.
/// rekordbox then shows "file not found", and the review said nothing.
#[test]
fn a_known_track_rekordbox_still_cannot_find_is_named_in_the_review_and_sent_as_before() {
    let mut world = World::new(&songs());
    let mut rb = collection(&world);
    let moved = UNKNOWN[0];
    let old_place = world.music.join("Old place").join(moved.file_name());
    rb.has(&old_place, &moved, 9, "125.00", "2A");
    world.rekordbox_saves(&rb.export());
    world.add_music_folder_and_scan();
    world.read_rekordbox();
    // Paired for good (name and length), so it's offered like the rest.
    let added: AddSummary = world.call("add_rekordbox_tracks", json!({ "playlists": null }));
    assert_eq!(added.added, KNOWN.len() as u32 + 1);
    let mut tracks: Vec<LibraryTrackId> = world.library().iter().map(|t| t.id).collect();
    tracks.sort();
    world.create_crate("Everything", &tracks);

    let preflight = world.prepared();
    assert_eq!(
        (preflight.new_tracks, preflight.known_tracks),
        (0, KNOWN.len() as u32 + 1)
    );
    let named: Vec<_> = preflight
        .no_file_at_location
        .iter()
        .map(|t| t.file_name.as_deref())
        .collect();
    assert_eq!(named, [Some(moved.file_name())]);
    assert_eq!(preflight.file_missing, []);
    assert!(preflight.can_send && !preflight.needs_confirm);

    // What is sent is what was always sent: rekordbox's own entry, at
    // rekordbox's own path.
    let sent = world.write_and_read_back(&preflight, false);
    let entry = track_at(&sent, &old_place).expect("sent at rekordbox's path");
    assert_is_rekordboxs_own_entry(entry, rb.at(&old_place).unwrap());
    assert!(track_at(&sent, &world.path_of(&moved)).is_none());
}
