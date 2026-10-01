//! The audio a match was made to: a carried match is dropped, and a
//! confirmation downgraded, when the file holds other audio now (1aD-1).

use super::*;

/// The user withdrew a confirmation: its `relink` row is deleted.
fn withdraw(lib: &Lib, location: &str) {
    let key = location::decode(location).unwrap().match_key();
    lib.writer
        .call(move |c| c.execute("DELETE FROM relink WHERE location_key = ?1", [key]))
        .unwrap();
}

#[test]
fn a_match_records_the_audio_its_file_holds() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Alpha.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Alpha.mp3"), Some("200"));
    // Not hashed yet: nothing to record.
    lib.relink(&mounted);
    assert_eq!(lib.evidence(track), None);
    lib.hashed(file, 1);
    assert_eq!(lib.relink(&mounted).changed, 1);
    assert_eq!(lib.evidence(track), Some(1));
    assert_eq!(lib.relink(&mounted).changed, 0);
}

/// A step-4 match: the row's own file gone, and `new` with its audio (1).
fn audio_match() -> (Lib, i64, Mounted, i64, i64) {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    let track = lib.track(&loc("E:/Music/Old.mp3"), Some("200"));
    lib.gone(old);
    let new = lib.file(music, "Moved/New.flac", None);
    lib.hashed(new, 1);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(new));
    assert_eq!(lib.evidence(track), Some(1));
    (lib, music, mounted, new, track)
}

#[test]
fn a_carried_audio_match_is_dropped_when_its_files_audio_changes() {
    let (lib, _music, mounted, new, track) = audio_match();
    // The file was replaced by other audio under the same name.
    lib.edited(new);
    lib.hashed(new, 9);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    assert_eq!(lib.evidence(track), None);
}

#[test]
fn a_tag_only_edit_leaves_a_carried_match_alone() {
    let (lib, _music, mounted, new, track) = audio_match();
    // Changed on disk, not hashed again yet: unknown, not changed.
    lib.edited(new);
    assert_eq!(lib.relink(&mounted).changed, 0);
    assert_eq!(lib.matched(track), same_audio(new));
    // Hashed again: the same audio.
    lib.hashed(new, 1);
    assert_eq!(lib.relink(&mounted).changed, 0);
    assert_eq!(lib.matched(track), same_audio(new));
}

/// A step-5 match to `guess`, hashed as audio 1.
fn name_match() -> (Lib, i64, Mounted, i64, i64) {
    let (lib, music, mounted) = e_music();
    let guess = lib.file(music, "Elsewhere/Beta.mp3", Some(300_000));
    lib.hashed(guess, 1);
    let track = lib.track(&loc("D:/Gone/Beta.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), name_only(guess));
    assert_eq!(lib.evidence(track), Some(1));
    (lib, music, mounted, guess, track)
}

#[test]
fn a_carried_name_guess_is_kept_while_its_file_holds_the_same_audio() {
    let (lib, music, mounted, guess, track) = name_match();
    // A second file with the name turns up: a fresh guess would be
    // ambiguous, but the carried one stands.
    lib.file(music, "Other/Beta.mp3", Some(310_000));
    assert_eq!(lib.relink(&mounted).changed, 0);
    assert_eq!(lib.matched(track), name_only(guess));
    assert!(lib.probable(track));
}

#[test]
fn a_carried_name_guess_is_dropped_when_its_files_audio_changes() {
    let (lib, music, mounted, guess, track) = name_match();
    lib.file(music, "Other/Beta.mp3", Some(310_000));
    lib.edited(guess);
    lib.hashed(guess, 9);
    lib.relink(&mounted);
    // Decided again: two files with the name now, so no match.
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_carried_name_guess_whose_audio_changed_is_guessed_again_with_the_new_audio() {
    let (lib, _music, mounted, guess, track) = name_match();
    lib.edited(guess);
    lib.hashed(guess, 9);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), name_only(guess));
    assert_eq!(lib.evidence(track), Some(9));
}

#[test]
fn a_carried_match_made_before_its_file_was_hashed_gets_its_audio_recorded_later() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Elsewhere/Gamma.mp3", Some(200_000));
    let track = lib.track(&loc("D:/Gone/Gamma Old.mp3"), Some("200"));
    lib.matched_before(track, file, "fingerprint", 0.95);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(file));
    assert_eq!(lib.evidence(track), None);
    // No evidence isn't a change: the first current hash is recorded.
    lib.hashed(file, 3);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(file));
    assert_eq!(lib.evidence(track), Some(3));
    // From then on a change is seen.
    lib.edited(file);
    lib.hashed(file, 4);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

// --- Confirmations ---

/// A confirmed relink to `file`, hashed as audio 1 when confirmed.
fn confirmed(method: Method) -> (Lib, Mounted, String, i64, i64) {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Elsewhere/Picked.mp3", Some(90_000));
    lib.hashed(file, 1);
    let location = loc("D:/Gone/Delta.mp3");
    let track = lib.track(&location, Some("200"));
    lib.confirm_now(&location, file, method);
    assert_eq!(lib.confirmed_audio(&location), Some(1));
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((file, method.as_str().to_owned(), 1.0))
    );
    assert!(!lib.probable(track));
    (lib, mounted, location, file, track)
}

#[test]
fn a_confirmation_whose_files_audio_changed_is_kept_but_only_probable() {
    let (lib, mounted, location, file, track) = confirmed(Method::User);
    lib.edited(file);
    lib.hashed(file, 9);
    let summary = lib.relink(&mounted);
    // Still this row's file, but no rekordbox data is attached until the
    // user confirms again.
    assert_eq!(lib.matched(track), Some((file, "user".to_owned(), 0.5)));
    assert!(lib.probable(track));
    assert_eq!((summary.confirmed, summary.probable), (0, 1));
    // The confirmation itself is untouched.
    assert_eq!(lib.confirmed_audio(&location), Some(1));
    assert_eq!(lib.relink(&mounted).changed, 0);
}

#[test]
fn confirming_again_records_the_new_audio_and_is_trusted() {
    let (lib, mounted, location, file, track) = confirmed(Method::User);
    lib.edited(file);
    lib.hashed(file, 9);
    lib.relink(&mounted);
    assert!(lib.probable(track));
    lib.confirm_now(&location, file, Method::User);
    assert_eq!(lib.confirmed_audio(&location), Some(9));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), Some((file, "user".to_owned(), 1.0)));
    assert!(!lib.probable(track));
}

#[test]
fn a_tag_only_edit_leaves_a_confirmation_trusted() {
    let (lib, mounted, _location, file, track) = confirmed(Method::User);
    // Changed on disk, not hashed again yet.
    lib.edited(file);
    assert_eq!(lib.relink(&mounted).changed, 0);
    assert!(!lib.probable(track));
    // Hashed again: the audio is the same.
    lib.hashed(file, 1);
    assert_eq!(lib.relink(&mounted).changed, 0);
    assert_eq!(lib.matched(track), Some((file, "user".to_owned(), 1.0)));
    assert!(!lib.probable(track));
}

#[test]
fn a_confirmation_with_no_audio_recorded_is_never_downgraded_and_records_it_once_known() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Elsewhere/Picked.mp3", Some(90_000));
    let location = loc("D:/Gone/Epsilon.mp3");
    let track = lib.track(&location, Some("200"));
    // Confirmed before the file was hashed (or before the app recorded
    // audio with confirmations).
    lib.confirm(&location, file, "user");
    lib.relink(&mounted);
    assert_eq!(lib.confirmed_audio(&location), None);
    assert!(!lib.probable(track));
    lib.hashed(file, 5);
    lib.relink(&mounted);
    assert_eq!(lib.confirmed_audio(&location), Some(5));
    assert_eq!(lib.matched(track), Some((file, "user".to_owned(), 1.0)));
    assert!(!lib.probable(track));
    // Recorded once, never overwritten by a run: a later change is seen.
    lib.edited(file);
    lib.hashed(file, 6);
    lib.relink(&mounted);
    assert_eq!(lib.confirmed_audio(&location), Some(5));
    assert!(lib.probable(track));
}

#[test]
fn a_downgraded_confirmation_that_is_withdrawn_is_decided_again() {
    for method in [Method::FilenameOnly, Method::Fingerprint, Method::User] {
        let (lib, mounted, location, file, track) = confirmed(method);
        lib.edited(file);
        lib.hashed(file, 9);
        lib.relink(&mounted);
        assert!(lib.probable(track));
        withdraw(&lib, &location);
        lib.relink(&mounted);
        // Not carried over as a guess: nothing else names this file.
        assert_eq!(lib.matched(track), None, "{method:?}");
    }
}

#[test]
fn a_downgraded_confirmation_still_keeps_its_file_from_a_guess() {
    let (lib, mounted, _location, file, track) = confirmed(Method::User);
    // Another row would take the file by name.
    let other = lib.track(&loc("D:/Also Gone/Picked.mp3"), Some("200"));
    lib.edited(file);
    lib.hashed(file, 9);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), Some((file, "user".to_owned(), 0.5)));
    assert_eq!(lib.matched(other), None);
}

// --- Reruns and fresh reads ---

/// A library with a match from every step 4 and 5 makes, and a
/// confirmation.
fn later_steps() -> (Lib, Mounted) {
    let (lib, music, mounted) = e_music();
    // Step 4, same audio.
    let moved = lib.file(music, "Moved.mp3", Some(200_000));
    lib.hashed(moved, 1);
    lib.track(&loc("E:/Music/Moved.mp3"), Some("200"));
    let now_here = lib.file(music, "New Place/Renamed.flac", None);
    lib.hashed(now_here, 1);
    // Step 4, fingerprint, one accepted and one probable.
    for (n, seed, title) in [(1, 11, Some("Zeta 1")), (2, 12, None)] {
        let old = lib.file(music, &format!("Old {n}.mp3"), Some(100_000 * n + 200_000));
        lib.hashed(old, 20 + n as u8);
        lib.fingerprinted(old, &song(seed, 30));
        lib.track_with(
            &loc(&format!("E:/Music/Old {n}.mp3")),
            None,
            &[("Name", &format!("Zeta {n}"))],
        );
        let new = lib.file(
            music,
            &format!("Converted {n}/Zeta.flac"),
            Some(100_000 * n + 200_300),
        );
        lib.fingerprinted(new, &reencoded(&song(seed, 30)));
        if let Some(title) = title {
            lib.title(new, title);
        }
        lib.gone(old);
    }
    lib.gone(moved);
    // Step 5.
    lib.file(music, "Elsewhere/Eta.mp3", Some(50_000));
    lib.track(&loc("D:/Gone/Eta.mp3"), Some("200"));
    // Confirmed, and missing.
    let picked = lib.file(music, "Elsewhere/Picked.mp3", Some(90_000));
    lib.hashed(picked, 3);
    lib.track(&loc("D:/Gone/Theta.mp3"), Some("200"));
    lib.confirm_now(&loc("D:/Gone/Theta.mp3"), picked, Method::User);
    lib.track(&loc("D:/Gone/Iota.mp3"), Some("200"));
    (lib, mounted)
}

#[test]
fn relinking_again_with_nothing_changed_changes_nothing_in_the_later_steps() {
    let (lib, mounted) = later_steps();
    let first = lib.relink(&mounted);
    assert_eq!(
        (
            first.fingerprint,
            first.probable,
            first.confirmed,
            first.missing
        ),
        (2, 2, 1, 1)
    );
    let after_first = lib.all_states();
    let second = lib.relink(&mounted);
    assert_eq!(lib.all_states(), after_first);
    assert_eq!(
        second,
        Summary {
            changed: 0,
            ..first
        }
    );
}

#[test]
fn a_fresh_read_gives_the_same_matches_when_nothing_on_disk_changed() {
    let (lib, mounted) = later_steps();
    lib.relink(&mounted);
    let before = lib.all_states();
    assert!(before.iter().filter(|(_, m, _)| m.is_some()).count() >= 5);
    // The read replaces the snapshot: every carried match is gone.
    lib.fresh_read();
    lib.relink(&mounted);
    assert_eq!(lib.all_states(), before);
}
