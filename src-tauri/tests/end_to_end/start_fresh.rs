//! 1aG-2: a Library started fresh, sent and read back.
//!
//! The steps, each a real command or job: add the music folder → scan →
//! "Start fresh" (the Library stays empty) → add tracks from All music →
//! make a crate and add some to it → prepare a send
//! (which reads rekordbox's export and matches it) → write it → read the
//! file back with the app's reader.

use std::fs;

use serde_json::json;
use tracklist_pro_lib::crates::CrateId;
use tracklist_pro_lib::library::LibraryTrack;
use tracklist_pro_lib::rekordbox;
use tracklist_pro_lib::send::{LeftOutReason, LosesEntries, SendFailure, TreeKind};

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

#[test]
fn a_fresh_library_is_empty_and_all_music_shows_every_track_once() {
    let (world, _rb) = started_fresh();
    let all = world.all_music();
    assert_eq!(all.total, TRACKS_IN_ALL_MUSIC);
    assert_eq!(all.tracks.len(), TRACKS_IN_ALL_MUSIC as usize);
    assert!(all.tracks.iter().all(|t| !t.in_library));
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

#[test]
fn sending_a_fresh_library_again_duplicates_nothing() {
    let (mut world, mut rb) = started_fresh();
    fresh_library_with_a_crate(&world);

    let first = world.send();
    let first_bytes = world.sent_bytes();
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

#[test]
fn a_track_rekordbox_holds_under_another_file_is_flagged_and_arrives_as_a_second_entry() {
    let (mut world, mut rb) = started_fresh();

    // All music shows the twins as one track. Whichever file "Add to
    // Library" links, rekordbox has the other one.
    let row = world
        .all_music()
        .tracks
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
    world.add_from_all_music(&linked);
    let theirs = rb.has(&world.path_of(&other), &other, 12, "126.00", "5A");
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
