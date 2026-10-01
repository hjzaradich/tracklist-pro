//! Step 5: the file name alone, always probable (1aD-2).

use super::*;

#[test]
fn the_only_file_with_the_locations_name_is_matched_as_probable() {
    let (lib, music, mounted) = e_music();
    // Its duration doesn't fit, so it's no step-2 match.
    let file = lib.file(music, "Elsewhere/Mu.mp3", Some(300_000));
    let recording = lib.group(&[file]);
    let track = lib.track(&loc("D:/Gone/mu.MP3"), Some("200"));
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), name_only(file));
    assert!(lib.probable(track), "never trusted until confirmed");
    assert_eq!(lib.recording(track), Some(recording));
    assert_eq!((summary.probable, summary.missing), (1, 0));
}

#[test]
fn a_name_match_to_a_file_not_read_yet_is_probable_until_its_duration_fits() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Elsewhere/Nu.mp3", None);
    let track = lib.track(&loc("D:/Gone/Nu.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), name_only(file));
    assert!(lib.probable(track));
    // Stage 2 reads it: name and duration agree, which step 2 accepts.
    lib.read(file, 200_000);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((file, "filename_duration".to_owned(), 0.9))
    );
    assert!(!lib.probable(track));
}

#[test]
fn a_name_guess_gives_way_when_the_locations_own_file_turns_up() {
    let (lib, music, mounted) = e_music();
    let guess = lib.file(music, "Elsewhere/Xi.mp3", Some(300_000));
    let track = lib.track(&loc("E:/Music/Xi.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), name_only(guess));
    let own = lib.file(music, "Xi.mp3", Some(200_000));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(own));
    assert!(!lib.probable(track));
}

#[test]
fn two_different_files_with_the_name_are_ambiguous() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "One/Omicron.mp3", Some(300_000));
    lib.file(music, "Two/Omicron.mp3", Some(310_000));
    let track = lib.track(&loc("D:/Gone/Omicron.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn exact_copies_with_the_name_match_the_lowest_file_id() {
    let (lib, music, mounted) = e_music();
    let first = lib.file(music, "One/Pi.mp3", Some(300_000));
    let second = lib.file(music, "Two/Pi.mp3", Some(300_000));
    lib.audio_hash(first, 7);
    lib.audio_hash(second, 7);
    let track = lib.track(&loc("D:/Gone/Pi.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), name_only(first));
}

#[test]
fn a_same_named_file_another_row_holds_blocks_the_name() {
    let (lib, music, mounted) = e_music();
    let held = lib.file(music, "One/Rho.mp3", Some(300_000));
    lib.file(music, "Two/Rho.mp3", Some(310_000));
    let holder = lib.track(&loc("E:/Music/One/Rho.mp3"), Some("300"));
    let track = lib.track(&loc("D:/Gone/Rho.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(holder), path(held));
    // The free one isn't the only file with the name.
    assert_eq!(lib.matched(track), None);
}

#[test]
fn the_only_file_with_the_name_isnt_taken_when_another_row_holds_it() {
    let (lib, music, mounted) = e_music();
    let held = lib.file(music, "One/Sigma.mp3", Some(300_000));
    let holder = lib.track(&loc("E:/Music/One/Sigma.mp3"), Some("300"));
    let track = lib.track(&loc("D:/Gone/Sigma.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(holder), path(held));
    assert_eq!(lib.matched(track), None);
}

#[test]
fn two_rows_with_the_same_file_name_dont_share_the_one_file() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Elsewhere/Tau.mp3", Some(300_000));
    let first = lib.track(&loc("D:/Gone/Tau.mp3"), Some("200"));
    let second = lib.track(&loc("D:/Also Gone/Tau.mp3"), Some("100"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(first), None);
    assert_eq!(lib.matched(second), None);
}

#[test]
fn a_name_guess_isnt_evidence_of_where_its_neighbours_went() {
    let (lib, music, mounted) = e_music();
    let guess = lib.file(music, "Elsewhere/Upsilon.mp3", Some(300_000));
    // In the folder the guess is in: the only file of its duration.
    lib.file(music, "Elsewhere/Renamed.mp3", Some(100_200));
    let guessed = lib.track(&loc("D:/Gone/Upsilon.mp3"), Some("200"));
    let neighbour = lib.track(&loc("D:/Gone/Phi.mp3"), Some("100"));
    // Twice: the second run sees the guess the first one stored.
    for _ in 0..2 {
        lib.relink(&mounted);
        assert_eq!(lib.matched(guessed), name_only(guess));
        assert_eq!(lib.matched(neighbour), None, "guesses never chain");
    }
}
