//! Step 3: the only file of that duration in the candidate set.

use super::*;
use crate::relink::MAX_CANDIDATES;

fn by_duration(file: i64) -> Option<Match> {
    Some((file, "unique_duration".to_owned(), 0.6))
}

#[test]
fn a_file_renamed_where_it_was_matches_by_unique_duration() {
    let (lib, music, mounted) = e_music();
    let renamed = lib.file(music, "Album/01 Renamed.mp3", Some(200_300));
    let a = lib.file(music, "Album/02 Other.mp3", Some(310_000));
    let b = lib.file(music, "Album/03 Another.mp3", Some(150_000));
    let track = lib.track(&loc("E:/Music/Album/01 Original.mp3"), Some("200"));
    let other = lib.track(&loc("E:/Music/Album/02 Other.mp3"), Some("310"));
    let another = lib.track(&loc("E:/Music/Album/03 Another.mp3"), Some("150"));
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), by_duration(renamed));
    assert_eq!(lib.matched(other), path(a));
    assert_eq!(lib.matched(another), path(b));
    assert_eq!(summary.unique_duration, 1);
}

#[test]
fn a_file_moved_with_its_neighbours_and_renamed_matches_by_unique_duration() {
    let (lib, music, mounted) = e_music();
    // The folder moved from D: (gone) into the music folder; one track
    // kept its name, one was renamed.
    let kept = lib.file(music, "Moved/Kept.mp3", Some(250_000));
    let renamed = lib.file(music, "Moved/New Name.mp3", Some(200_300));
    let neighbour = lib.track(&loc("D:/Old/Kept.mp3"), Some("250"));
    let track = lib.track(&loc("D:/Old/Old Name.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(neighbour),
        Some((kept, "filename_duration".to_owned(), 0.9))
    );
    assert_eq!(lib.matched(track), by_duration(renamed));
}

#[test]
fn a_file_with_a_unique_duration_outside_the_candidate_set_isnt_matched() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Album/Kept.mp3", Some(250_000));
    // The only file of that duration, but in an unrelated folder.
    lib.file(music, "Unrelated/Anything.mp3", Some(200_300));
    let neighbour = lib.track(&loc("E:/Music/Album/Kept.mp3"), Some("250"));
    let track = lib.track(&loc("E:/Music/Album/Gone.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.method(neighbour).as_deref(), Some("path"));
    assert_eq!(lib.matched(track), None);
}

#[test]
fn two_files_that_fit_in_the_candidate_set_leave_the_track_unmatched() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Album/Clean.mp3", Some(200_100));
    lib.file(music, "Album/Dirty.mp3", Some(200_900));
    let track = lib.track(&loc("E:/Music/Album/Gone.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_look_alike_another_track_has_blocks_the_match_instead_of_being_skipped() {
    let (lib, music, mounted) = e_music();
    let clean = lib.file(music, "Album/Clean.mp3", Some(200_100));
    lib.file(music, "Album/Unknown Cut.mp3", Some(200_900));
    let has_clean = lib.track(&loc("E:/Music/Album/Clean.mp3"), Some("200"));
    let track = lib.track(&loc("E:/Music/Album/Dirty.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(has_clean), path(clean));
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_fitting_file_another_track_has_is_never_taken() {
    let (lib, music, mounted) = e_music();
    let clean = lib.file(music, "Album/Clean.mp3", Some(200_100));
    let has_clean = lib.track(&loc("E:/Music/Album/Clean.mp3"), Some("200"));
    let track = lib.track(&loc("E:/Music/Album/Dirty.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(has_clean), path(clean));
    assert_eq!(lib.matched(track), None);
}

#[test]
fn an_unread_candidate_blocks_the_match_until_it_is_read() {
    let (lib, music, mounted) = e_music();
    let renamed = lib.file(music, "Album/Renamed.mp3", Some(200_300));
    let unread = lib.file(music, "Album/Not Read Yet.mp3", None);
    let track = lib.track(&loc("E:/Music/Album/Original.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // Read with a different duration: now the fit is provably unique.
    lib.read(unread, 330_000);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), by_duration(renamed));
}

#[test]
fn an_unread_candidate_that_turns_out_to_fit_too_leaves_the_track_unmatched() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Album/Renamed.mp3", Some(200_300));
    let unread = lib.file(music, "Album/Not Read Yet.mp3", None);
    let track = lib.track(&loc("E:/Music/Album/Original.mp3"), Some("200"));
    lib.relink(&mounted);
    lib.read(unread, 200_700);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn two_tracks_that_fit_the_one_file_leave_both_unmatched() {
    let (lib, music, mounted) = e_music();
    // Fits 200 (199.5 to 201.5 s) and 201 (200.5 to 202.5 s).
    lib.file(music, "Album/Renamed.mp3", Some(200_600));
    let a = lib.track(&loc("E:/Music/Album/First.mp3"), Some("200"));
    let b = lib.track(&loc("E:/Music/Album/Second.mp3"), Some("201"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(a), None);
    assert_eq!(lib.matched(b), None);
}

#[test]
fn neighbours_matched_by_duration_alone_dont_widen_the_candidate_set() {
    let (lib, music, mounted) = e_music();
    let guessed = lib.file(music, "Somewhere/Guess.mp3", Some(300_000));
    lib.file(music, "Somewhere/Renamed.mp3", Some(200_300));
    let neighbour = lib.track(&loc("D:/Old/Guess Source.mp3"), Some("300"));
    // An earlier run matched the neighbour by duration alone.
    lib.matched_before(neighbour, guessed, "unique_duration", 0.6);
    let track = lib.track(&loc("D:/Old/Original.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_neighbour_the_user_confirmed_widens_the_candidate_set() {
    let (lib, music, mounted) = e_music();
    let guessed = lib.file(music, "Somewhere/Guess.mp3", Some(300_000));
    let renamed = lib.file(music, "Somewhere/Renamed.mp3", Some(200_300));
    lib.track(&loc("D:/Old/Guess Source.mp3"), Some("300"));
    lib.confirm(&loc("D:/Old/Guess Source.mp3"), guessed, "filename_only");
    let track = lib.track(&loc("D:/Old/Original.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), by_duration(renamed));
}

#[test]
fn a_candidate_set_over_the_limit_makes_no_match() {
    for (files, matches) in [(MAX_CANDIDATES, true), (MAX_CANDIDATES + 1, false)] {
        let (lib, music, mounted) = e_music();
        // Every other file lasts ten minutes and more, far from the track.
        let fits = lib.file(music, "Big/Fits.mp3", Some(200_300));
        for n in 1..files {
            lib.file(
                music,
                &format!("Big/Other {n}.mp3"),
                Some(600_000 + n as i64 * 3_000),
            );
        }
        let track = lib.track(&loc("E:/Music/Big/Original.mp3"), Some("200"));
        lib.relink(&mounted);
        let want = if matches { by_duration(fits) } else { None };
        assert_eq!(lib.matched(track), want, "{files} candidates");
    }
}

#[test]
fn files_in_a_subfolder_arent_candidates() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Album/Kept.mp3", Some(250_000));
    lib.file(music, "Album/Bonus/Renamed.mp3", Some(200_300));
    lib.track(&loc("E:/Music/Album/Kept.mp3"), Some("250"));
    let track = lib.track(&loc("E:/Music/Album/Original.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_renamed_file_on_a_drive_that_isnt_plugged_in_matches_by_unique_duration() {
    let lib = Lib::new();
    let vol = lib.volume(&serial(1), Some(r"E:\"));
    let music = lib.folder(vol, "Music");
    let renamed = lib.file(music, "Album/Renamed.mp3", Some(200_300));
    let track = lib.track(&loc("E:/Music/Album/Original.mp3"), Some("200"));
    lib.relink(&Mounted::default());
    assert_eq!(lib.matched(track), by_duration(renamed));
}

#[test]
fn a_unique_duration_match_never_looks_at_the_size() {
    let (lib, music, mounted) = e_music();
    let renamed = lib.file(music, "Album/Renamed.mp3", Some(200_300));
    lib.size(renamed, 5_555_555);
    let decoy = lib.file(music, "Album/Same Size.mp3", Some(400_000));
    lib.size(decoy, 4321);
    let track = lib.track_with(
        &loc("E:/Music/Album/Original.mp3"),
        Some("200"),
        &[("Size", "4321")],
    );
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), by_duration(renamed));
}
