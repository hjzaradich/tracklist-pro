//! Step 0: a confirmed relink is re-applied first.

use super::*;

#[test]
fn a_confirmed_relink_is_reapplied_to_a_fresh_read() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Somewhere/Picked.mp3", Some(100_000));
    let location = loc("E:/Gone/Original.mp3");
    lib.confirm(&location, file, "filename_only");
    // A fresh read: the row is unmatched.
    let track = lib.track(&location, Some("200"));
    let summary = lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((file, "filename_only".to_owned(), 1.0))
    );
    assert_eq!(summary.confirmed, 1);
}

#[test]
fn a_confirmed_relink_is_matched_by_the_location_key_in_any_spelling() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Picked.mp3", None);
    lib.confirm("file://localhost/E:/Gone/Cafe%cc%81.mp3", file, "user");
    let track = lib.track("file://localhost/e:/gone/CAF%c3%89.mp3", None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), Some((file, "user".to_owned(), 1.0)));
}

#[test]
fn a_confirmed_relink_wins_over_a_path_match() {
    let (lib, music, mounted) = e_music();
    let at_path = lib.file(music, "Alpha.mp3", Some(200_000));
    let picked = lib.file(music, "Elsewhere/Alpha (Real).mp3", Some(200_000));
    let location = loc("E:/Music/Alpha.mp3");
    lib.confirm(&location, picked, "user");
    let track = lib.track(&location, Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), Some((picked, "user".to_owned(), 1.0)));
    let _ = at_path;
}

#[test]
fn a_confirmed_relink_replaces_an_earlier_automatic_match() {
    let (lib, music, mounted) = e_music();
    let guessed = lib.file(music, "Guess.mp3", Some(200_000));
    let picked = lib.file(music, "Picked.mp3", Some(200_000));
    let location = loc("E:/Gone/Original.mp3");
    let track = lib.track(&location, Some("200"));
    lib.matched_before(track, guessed, "unique_duration", 0.6);
    lib.confirm(&location, picked, "user");
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), Some((picked, "user".to_owned(), 1.0)));
}

#[test]
fn a_confirmed_relink_whose_file_is_gone_falls_back_to_the_steps() {
    let (lib, music, mounted) = e_music();
    let picked = lib.file(music, "Picked.mp3", Some(100_000));
    lib.gone(picked);
    let at_path = lib.file(music, "Beta.mp3", Some(200_000));
    let location = loc("E:/Music/Beta.mp3");
    lib.confirm(&location, picked, "user");
    let track = lib.track(&location, Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(at_path));
}

#[test]
fn a_file_a_confirmed_relink_holds_isnt_given_to_another_track() {
    let (lib, music, mounted) = e_music();
    let picked = lib.file(music, "New/Gamma.mp3", Some(200_300));
    let location = loc("E:/Gone/Anything.mp3");
    lib.confirm(&location, picked, "user");
    let confirmed = lib.track(&location, Some("200"));
    // Same name and duration as the confirmed file.
    let other = lib.track(&loc("E:/Old/Gamma.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.method(confirmed).as_deref(), Some("user"));
    assert_eq!(lib.matched(other), None);
}
