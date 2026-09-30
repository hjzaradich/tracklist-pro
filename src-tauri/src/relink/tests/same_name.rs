//! Step 2: the same file name, and the duration fits.

use super::*;

fn by_name(file: i64) -> Option<Match> {
    Some((file, "filename_duration".to_owned(), 0.9))
}

#[test]
fn a_moved_file_with_the_same_name_and_duration_matches_by_filename_and_duration() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "New Place/Alpha.mp3", Some(200_400));
    let track = lib.track(&loc("E:/Old Place/Alpha.mp3"), Some("200"));
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), by_name(file));
    assert_eq!(summary.filename_duration, 1);
}

#[test]
fn the_same_name_with_a_duration_outside_the_window_doesnt_match() {
    let (lib, music, mounted) = e_music();
    // TotalTime 200 fits 199.500 s up to, not including, 201.500 s.
    lib.file(music, "A/Too Short.mp3", Some(199_499));
    lib.file(music, "A/Too Long.mp3", Some(201_500));
    let short = lib.track(&loc("E:/Gone/Too Short.mp3"), Some("200"));
    let long = lib.track(&loc("E:/Gone/Too Long.mp3"), Some("200"));
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(short), None);
    assert_eq!(lib.matched(long), None);
    assert_eq!(summary.missing, 2);
}

#[test]
fn the_window_edges_match_as_the_truncated_total_time_says() {
    let (lib, music, mounted) = e_music();
    let low = lib.file(music, "B/Low Edge.mp3", Some(199_500));
    let high = lib.file(music, "B/High Edge.mp3", Some(201_499));
    let low_track = lib.track(&loc("E:/Gone/Low Edge.mp3"), Some("200"));
    let high_track = lib.track(&loc("E:/Gone/High Edge.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(low_track), by_name(low));
    assert_eq!(lib.matched(high_track), by_name(high));
}

#[test]
fn the_same_name_matches_in_nfd_and_in_another_letter_case() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "C/Caf\u{e9} Beta.MP3", Some(180_000));
    // A macOS export: NFD, and the name in lowercase.
    let track = lib.track(
        "file://localhost/Users/dj/Music/cafe%cc%81%20beta.mp3",
        Some("180"),
    );
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), by_name(file));
}

#[test]
fn two_same_named_files_that_both_fit_leave_the_track_unmatched() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Copy 1/Gamma.mp3", Some(240_100));
    lib.file(music, "Copy 2/Gamma.mp3", Some(240_900));
    let track = lib.track(&loc("E:/Gone/Gamma.mp3"), Some("240"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_same_named_file_that_doesnt_fit_doesnt_make_it_ambiguous() {
    let (lib, music, mounted) = e_music();
    let fits = lib.file(music, "Right/Delta.mp3", Some(240_100));
    lib.file(music, "Other Cut/Delta.mp3", Some(300_000));
    let track = lib.track(&loc("E:/Gone/Delta.mp3"), Some("240"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), by_name(fits));
}

#[test]
fn a_same_named_file_of_unknown_duration_blocks_the_match() {
    let (lib, music, mounted) = e_music();
    let fits = lib.file(music, "Read/Epsilon.mp3", Some(240_100));
    let unread = lib.file(music, "Unread/Epsilon.mp3", None);
    let track = lib.track(&loc("E:/Gone/Epsilon.mp3"), Some("240"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // Once it's read and doesn't fit, the match is made.
    lib.read(unread, 90_000);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), by_name(fits));
}

#[test]
fn two_tracks_that_fit_the_same_file_leave_both_unmatched() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Now/Zeta.mp3", Some(240_100));
    let a = lib.track(&loc("E:/Was Here/Zeta.mp3"), Some("240"));
    let b = lib.track(&loc("E:/Or Here/Zeta.mp3"), Some("240"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(a), None);
    assert_eq!(lib.matched(b), None);
}

#[test]
fn a_file_another_track_has_by_path_isnt_taken_by_name() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Eta.mp3", Some(240_100));
    let at_path = lib.track(&loc("E:/Music/Eta.mp3"), Some("240"));
    let elsewhere = lib.track(&loc("E:/Old/Eta.mp3"), Some("240"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(at_path), path(file));
    assert_eq!(lib.matched(elsewhere), None);
}

#[test]
fn a_track_without_a_usable_total_time_is_never_matched_by_duration() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "D/Theta.mp3", Some(0));
    lib.file(music, "D/Iota.mp3", Some(500));
    lib.file(music, "D/Kappa.mp3", Some(200_000));
    let zero = lib.track(&loc("E:/Gone/Theta.mp3"), Some("0"));
    let tiny = lib.track(&loc("E:/Gone/Iota.mp3"), Some("0"));
    let none = lib.track(&loc("E:/Gone/Kappa.mp3"), None);
    let junk = lib.track(&loc("E:/Gone/Kappa.mp3"), Some("200.0"));
    lib.relink(&mounted);
    for track in [zero, tiny, none, junk] {
        assert_eq!(lib.matched(track), None, "track {track}");
    }
}

#[test]
fn a_different_name_with_the_same_duration_doesnt_match_by_name() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Elsewhere/Renamed.mp3", Some(240_100));
    let track = lib.track(&loc("E:/Gone/Original.mp3"), Some("240"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_file_on_a_drive_that_isnt_plugged_in_can_match_by_name_and_duration() {
    let lib = Lib::new();
    let vol = lib.volume(&serial(1), Some(r"G:\"));
    let music = lib.folder(vol, "Archive");
    let file = lib.file(music, "Lambda.mp3", Some(240_100));
    let track = lib.track(&loc("E:/Gone/Lambda.mp3"), Some("240"));
    lib.relink(&Mounted::default());
    assert_eq!(lib.matched(track), by_name(file));
}

#[test]
fn a_name_match_never_looks_at_the_size() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "New/Mu.mp3", Some(240_100));
    lib.size(file, 7_777_777);
    let decoy = lib.file(music, "New/Decoy.mp3", Some(100_000));
    lib.size(decoy, 1234);
    // The export's Size matches the decoy, not the file.
    let track = lib.track_with(&loc("E:/Gone/Mu.mp3"), Some("240"), &[("Size", "1234")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), by_name(file));
}
