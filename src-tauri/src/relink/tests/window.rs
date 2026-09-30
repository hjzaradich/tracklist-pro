//! The duration window, path keys and mount points.

use super::super::mount_text;
use super::super::rules::{duration_fits, path_key};
use super::*;

#[test]
fn a_file_fits_from_half_a_second_under_the_total_time_to_a_second_and_a_half_over() {
    // TotalTime 200 means rekordbox measured 200.000 to 200.999 s.
    assert!(!duration_fits(200, 199_499));
    assert!(duration_fits(200, 199_500));
    assert!(duration_fits(200, 200_000));
    assert!(duration_fits(200, 200_999));
    assert!(duration_fits(200, 201_499));
    assert!(!duration_fits(200, 201_500));
}

#[test]
fn a_truncated_total_time_fits_the_file_it_was_measured_from() {
    // rekordbox writes whole seconds, dropping the fraction (§5.3).
    for ms in [240_000_i64, 240_123, 240_634, 240_965, 240_999] {
        let total = u32::try_from(ms / 1000).unwrap();
        assert!(duration_fits(total, ms), "{ms} ms as {total} s");
    }
}

#[test]
fn a_huge_total_time_doesnt_overflow() {
    assert!(duration_fits(u32::MAX, i64::from(u32::MAX) * 1000));
    assert!(!duration_fits(u32::MAX, 0));
    assert!(!duration_fits(1, i64::MAX));
}

#[test]
fn a_path_key_is_the_key_a_location_naming_that_path_has() {
    // NFD, lowercase, ß (no multi-letter uppercase), the Kelvin sign,
    // dotless i, and a character outside the BMP.
    let paths = [
        "E:/Music/Cafe\u{301}.mp3",
        "e:/music/stra\u{df}e.flac",
        "E:/\u{212a}elvin/\u{131}.wav",
        "E:/Stars \u{1f680}/\u{10428}.aiff",
        "//nas/share/\u{c5}ngstr\u{f6}m.mp3",
    ];
    for p in paths {
        let encoded: String = p
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"/:.".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02x}")
                }
            })
            .collect();
        let location = if p.starts_with("//") {
            format!("file://localhost{encoded}")
        } else {
            format!("file://localhost/{encoded}")
        };
        let key = location::decode(&location).unwrap().match_key();
        assert_eq!(path_key(p), key, "{p}");
    }
}

#[test]
fn mount_points_read_in_every_form_windows_gives_them() {
    assert_eq!(mount_text(r"E:\").as_deref(), Some("E:"));
    assert_eq!(mount_text(r"\\?\E:\").as_deref(), Some("E:"));
    assert_eq!(mount_text(r"C:\mnt\usb\").as_deref(), Some("C:/mnt/usb"));
    assert_eq!(
        mount_text(r"\\?\UNC\nas\share\").as_deref(),
        Some("//nas/share")
    );
    assert_eq!(mount_text(r"\\nas\share").as_deref(), Some("//nas/share"));
    assert_eq!(mount_text(""), None);
    assert_eq!(mount_text("/mnt/usb"), None);
    assert_eq!(mount_text(r"\\\x"), None);
}

#[test]
fn a_volume_mounted_in_a_folder_matches_by_path() {
    let lib = Lib::new();
    let vol = lib.volume(&serial(1), Some(r"C:\mnt\usb\"));
    let music = lib.folder(vol, "Music");
    let file = lib.file(music, "Alpha.mp3", None);
    let track = lib.track(&loc("C:/mnt/usb/Music/Alpha.mp3"), None);
    lib.relink(&Mounted::new([(serial(1), r"\\?\C:\mnt\usb\")]));
    assert_eq!(lib.matched(track), path(file));
}
