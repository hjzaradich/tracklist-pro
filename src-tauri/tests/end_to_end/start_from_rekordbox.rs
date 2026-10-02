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
    NEVER_ON_DISK,
};
use super::harness::{crates_by_hand, World};
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

/// The Library tracks of [`KNOWN`], in that order.
fn known_tracks(world: &World) -> Vec<LibraryTrackId> {
    KNOWN.iter().map(|s| world.library_track(s).id).collect()
}

/// Crates naming every Library track: `Gigs/Friday` holds the first
/// three, `Everything` all of them.
fn crates_naming_every_track(world: &World) {
    let tracks = known_tracks(world);
    let gigs = crates_by_hand::folder(world, None, "Gigs");
    crates_by_hand::crate_of(world, Some(gigs), "Friday", &tracks[..3]);
    crates_by_hand::crate_of(world, None, "Everything", &tracks);
}

fn keys_of(world: &World, songs: &[super::harness::Song]) -> Vec<String> {
    songs.iter().map(|s| key_of(&world.path_of(s))).collect()
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

/// Today's behavior, pinned (the foreman's ruling on the 1aG-1 question):
/// a known track is sent only when a crate names it, and no command makes
/// a crate before 1cA-10. So a Library started from rekordbox, as the app
/// can build one in Phase 1a, has nothing to send.
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
    crates_naming_every_track(&world);

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

    // PLAYLISTS: the app's two folders, the crates under `Crates` in
    // their folders, entries in order.
    assert_eq!(children(&sent, &[]), ["Crates", "Playlists"]);
    assert_eq!(children(&sent, &["Crates"]), ["Gigs", "Everything"]);
    assert_eq!(children(&sent, &["Crates", "Gigs"]), ["Friday"]);
    assert_eq!(children(&sent, &["Playlists"]), [] as [&str; 0]);
    assert_eq!(
        entries(&sent, &["Crates", "Gigs", "Friday"]),
        keys_of(&world, &KNOWN[..3])
    );
    assert_eq!(
        entries(&sent, &["Crates", "Everything"]),
        keys_of(&world, &KNOWN)
    );

    // What the file does in rekordbox (by §5.2's rules): one dialog per
    // track, no new entry, and every track exactly as it was: BPM, key,
    // grid, cues, play count and all. The user's own playlists stay.
    let before = rb.clone();
    assert_eq!(rb.import(&sent), KNOWN.len());
    assert_eq!(rb.tracks, before.tracks);
    assert_eq!(rb.playlists[..before.playlists.len()], before.playlists[..]);
    let ids: Vec<u64> = KNOWN
        .iter()
        .map(|s| rb.at(&world.path_of(s)).unwrap().id)
        .collect();
    assert_eq!(
        rb.playlist(&["Crates", "Gigs", "Friday"]),
        Some(&ids[..3].to_vec())
    );
    assert_eq!(rb.playlist(&["Crates", "Everything"]), Some(&ids));
}

#[test]
fn sending_again_with_nothing_changed_writes_the_same_file_and_leaves_nothing_to_clean_up() {
    let (mut world, mut rb) = started_from_rekordbox();
    crates_naming_every_track(&world);

    let first = world.send();
    let first_bytes = world.sent_bytes();
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
    crates_naming_every_track(&world);
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
        keys_of(&world, &KNOWN)
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
