//! Step 1: the Location still names a present file.

use super::*;

#[test]
fn a_location_that_still_names_a_present_file_matches_by_path() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Album One/01 Alpha.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Album One/01 Alpha.mp3"), Some("200"));
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(file));
    assert_eq!(summary.path, 1);
    assert_eq!(summary.missing, 0);
}

#[test]
fn a_music_folder_at_the_top_of_a_drive_matches_by_path() {
    let lib = Lib::new();
    let vol = lib.volume(&serial(1), Some(r"E:\"));
    let top = lib.folder(vol, "");
    let file = lib.file(top, "Loose/Beta.flac", None);
    let track = lib.track(&loc("E:/Loose/Beta.flac"), None);
    lib.relink(&Mounted::new([(serial(1), r"\\?\E:\")]));
    assert_eq!(lib.matched(track), path(file));
}

#[test]
fn a_path_match_needs_no_duration() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Gamma.mp3", None);
    let track = lib.track(&loc("E:/Music/Gamma.mp3"), None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(file));
}

#[test]
fn an_nfd_location_matches_its_nfc_file_and_an_nfc_location_its_nfd_file() {
    let (lib, music, mounted) = e_music();
    let nfc_file = lib.file(music, "Caf\u{e9} One.mp3", None);
    let nfd_file = lib.file(music, "Cr\u{65}\u{300}me Two.mp3", None);
    // rekordbox escapes the UTF-8 bytes: é NFD is e + %cc%81.
    let nfd_track = lib.track("file://localhost/E:/Music/Cafe%cc%81%20One.mp3", None);
    let nfc_track = lib.track("file://localhost/E:/Music/Cr%c3%a8me%20Two.mp3", None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(nfd_track), path(nfc_file));
    assert_eq!(lib.matched(nfc_track), path(nfd_file));
}

#[test]
fn letter_case_differences_match_by_path_as_on_windows() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Sub/\u{c9}cho Delta.MP3", None);
    // Drive letter, folders and the name all in other cases.
    let track = lib.track(&loc("e:/MUSIC/sub/\u{e9}CHO delta.mp3"), None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(file));
}

#[test]
fn nfc_and_nfd_twin_files_match_the_one_spelled_like_the_location() {
    let (lib, music, mounted) = e_music();
    let nfc = lib.file(music, "Caf\u{e9}.mp3", None);
    let nfd = lib.file(music, "Cafe\u{301}.mp3", None);
    let to_nfd = lib.track("file://localhost/E:/Music/Cafe%cc%81.mp3", None);
    let to_nfc = lib.track("file://localhost/e:/music/CAF%c3%89.mp3", None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(to_nfd), path(nfd));
    assert_eq!(lib.matched(to_nfc), path(nfc));
}

#[test]
fn twin_files_spelled_unlike_the_location_leave_it_unmatched() {
    let lib = Lib::new();
    let vol = lib.volume(&serial(1), Some(r"E:\"));
    // Two music folders whose names are NFC and NFD twins, each holding
    // the file: the Location's key names both, and neither folder is
    // spelled like it (a third spelling, mixed).
    let a = lib.folder(vol, "\u{c9}t\u{e9}");
    let b = lib.folder(vol, "E\u{301}te\u{301}");
    lib.file(a, "x.mp3", None);
    lib.file(b, "x.mp3", None);
    let track = lib.track("file://localhost/E:/%c3%89te%cc%81/x.mp3", None);
    let summary = lib.relink(&Mounted::new([(serial(1), r"E:\")]));
    assert_eq!(lib.matched(track), None);
    assert_eq!(summary.missing, 1);
}

#[test]
fn a_location_whose_file_is_gone_stays_missing() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Epsilon.mp3", Some(200_000));
    lib.gone(file);
    let track = lib.track(&loc("E:/Music/Epsilon.mp3"), Some("200"));
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    assert_eq!(summary.missing, 1);
}

#[test]
fn a_location_on_another_drive_letter_doesnt_match_by_path() {
    let lib = Lib::new();
    let e = serial(1);
    let vol = lib.volume(&e, Some(r"F:\"));
    let music = lib.folder(vol, "Music");
    lib.file(music, "Zeta.mp3", None);
    // The export is from when the drive was E:; it's F: now.
    let track = lib.track(&loc("E:/Music/Zeta.mp3"), None);
    lib.relink(&Mounted::new([(e, r"F:\")]));
    // Not by path; the name alone is only a probable guess (step 5).
    assert_eq!(lib.method(track).as_deref(), Some("filename_only"));
    assert!(lib.probable(track));
}

#[test]
fn a_network_location_matches_its_file_on_the_share() {
    let lib = Lib::new();
    let unc = "unc=\\\\nas\\music".to_owned();
    let vol = lib.volume(&unc, Some(r"\\nas\music\"));
    let top = lib.folder(vol, "");
    let file = lib.file(top, "Crate/Eta.wav", None);
    let track = lib.track("file://localhost//NAS/Music/Crate/Eta.wav", None);
    lib.relink(&Mounted::new([(unc, r"\\?\UNC\nas\music\")]));
    assert_eq!(lib.matched(track), path(file));
}

#[test]
fn a_macos_location_never_matches_by_path() {
    let lib = Lib::new();
    // Even a volume whose "mount" would spell the same text can't: a
    // macOS path is never a Windows path.
    let vol = lib.volume(&serial(1), Some(r"E:\"));
    let music = lib.folder(vol, "Users/dj/Music");
    lib.file(music, "Theta.mp3", None);
    let track = lib.track("file://localhost/Users/dj/Music/Theta.mp3", None);
    lib.relink(&Mounted::new([(serial(1), r"E:\")]));
    // Not by path; the name alone is only a probable guess (step 5).
    assert_eq!(lib.method(track).as_deref(), Some("filename_only"));
    assert!(lib.probable(track));
}

#[test]
fn a_file_on_a_drive_that_isnt_plugged_in_matches_by_where_it_was_last_mounted() {
    let lib = Lib::new();
    let vol = lib.volume(&serial(1), Some(r"E:\"));
    let music = lib.folder(vol, "Music");
    let file = lib.file(music, "Iota.mp3", None);
    let track = lib.track(&loc("E:/Music/Iota.mp3"), None);
    // Nothing is mounted.
    lib.relink(&Mounted::default());
    assert_eq!(lib.matched(track), Some((file, "path".to_owned(), 0.9)));
}

#[test]
fn two_unplugged_drives_last_at_the_same_letter_leave_the_track_unmatched() {
    let lib = Lib::new();
    for n in [1, 2] {
        let vol = lib.volume(&serial(n), Some(r"E:\"));
        let music = lib.folder(vol, "Music");
        lib.file(music, "Kappa.mp3", None);
    }
    let track = lib.track(&loc("E:/Music/Kappa.mp3"), None);
    lib.relink(&Mounted::default());
    assert_eq!(lib.matched(track), None);
}

#[test]
fn the_drive_plugged_in_at_the_letter_wins_over_one_last_seen_there() {
    let lib = Lib::new();
    let old = lib.volume(&serial(1), Some(r"E:\"));
    let now = lib.volume(&serial(2), Some(r"E:\"));
    let old_music = lib.folder(old, "Music");
    let now_music = lib.folder(now, "Music");
    lib.file(old_music, "Lambda.mp3", None);
    let here = lib.file(now_music, "Lambda.mp3", None);
    let track = lib.track(&loc("E:/Music/Lambda.mp3"), None);
    lib.relink(&Mounted::new([(serial(2), r"E:\")]));
    assert_eq!(lib.matched(track), path(here));
}

#[test]
fn an_unplugged_drive_last_at_the_letter_of_a_plugged_in_drive_never_matches_by_path() {
    let lib = Lib::new();
    let old = lib.volume(&serial(1), Some(r"E:\"));
    let now = lib.volume(&serial(2), Some(r"E:\"));
    let old_music = lib.folder(old, "Music");
    let now_music = lib.folder(now, "Music");
    lib.file(old_music, "Mu.mp3", None);
    lib.file(now_music, "Other.mp3", None);
    let track = lib.track(&loc("E:/Music/Mu.mp3"), None);
    // Two known volumes at E:, one of them now: which one the Location
    // meant can't be told.
    lib.relink(&Mounted::new([(serial(2), r"E:\")]));
    // Not by path; the name alone is only a probable guess (step 5).
    assert_eq!(lib.method(track).as_deref(), Some("filename_only"));
    assert!(lib.probable(track));
}

#[test]
fn a_path_match_never_looks_at_the_size() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Nu.mp3", None);
    lib.size(file, 9_999_999);
    // rekordbox rewrote the tags since the export: its Size is stale.
    let track = lib.track_with(&loc("E:/Music/Nu.mp3"), None, &[("Size", "1234")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(file));
}

#[test]
fn an_ambiguous_path_on_a_plugged_in_drive_doesnt_fall_back_to_an_unplugged_one() {
    let lib = Lib::new();
    let now = lib.volume(&serial(1), Some(r"E:\"));
    let old = lib.volume(&serial(2), Some(r"E:\"));
    // Twin folders on the drive plugged in, neither spelled like the
    // Location; the unplugged drive holds the path once.
    let a = lib.folder(now, "\u{c9}t\u{e9}");
    let b = lib.folder(now, "E\u{301}te\u{301}");
    lib.file(a, "x.mp3", None);
    lib.file(b, "x.mp3", None);
    let away = lib.folder(old, "\u{c9}t\u{e9}");
    lib.file(away, "x.mp3", None);
    let track = lib.track("file://localhost/E:/%c3%89te%cc%81/x.mp3", None);
    lib.relink(&Mounted::new([(serial(1), r"E:\")]));
    assert_eq!(lib.matched(track), None);
}
