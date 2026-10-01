//! Step 3's second signal: a title tag that agrees with rekordbox's Name
//! makes a unique-duration match trusted; without one it's probable.

use super::super::rules::title_key;
use super::*;

/// One folder in which the track's file was renamed (or deleted, leaving
/// a look-alike): the only file of that duration there. Returns the
/// library, the file, the track and what's mounted.
fn renamed(name: &str) -> (Lib, i64, i64, Mounted) {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Album/Renamed.mp3", Some(200_300));
    let track = lib.track_with(
        &loc("E:/Music/Album/Original.mp3"),
        Some("200"),
        &[("Name", name)],
    );
    (lib, file, track, mounted)
}

#[test]
fn a_look_alike_cut_with_a_different_title_becomes_probable_not_linked() {
    let (lib, file, track, mounted) = renamed("Night Drive (Dirty)");
    lib.title(file, "Night Drive (Clean)");
    let summary = lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((file, "unique_duration".to_owned(), 0.4))
    );
    assert!(lib.probable(track));
    assert_eq!((summary.unique_duration, summary.probable), (0, 1));
}

#[test]
fn a_matching_title_is_accepted() {
    let (lib, file, track, mounted) = renamed("Night Drive (Extended Mix)");
    lib.title(file, "night drive [extended mix]");
    let summary = lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((file, "unique_duration".to_owned(), 0.8))
    );
    assert!(!lib.probable(track));
    assert_eq!((summary.unique_duration, summary.probable), (1, 0));
}

#[test]
fn a_file_without_tags_is_probable() {
    for raw_tags in [None, Some("{}"), Some(r#"{"id3v2": []}"#)] {
        let (lib, file, track, mounted) = renamed("Night Drive");
        if let Some(tags) = raw_tags {
            lib.tags(file, tags);
        }
        lib.relink(&mounted);
        assert_eq!(lib.matched(track).map(|m| m.0), Some(file), "{raw_tags:?}");
        assert!(lib.probable(track), "{raw_tags:?}");
    }
}

#[test]
fn a_track_without_a_name_is_probable() {
    let (lib, file, track, mounted) = renamed("");
    lib.title(file, "");
    lib.relink(&mounted);
    assert!(lib.probable(track));
    let _ = file;
}

#[test]
fn a_title_in_any_tag_block_counts() {
    let blocks = [
        r#"{"id3v1": [{"key": "title", "value": {"type": "text", "text": "Night Drive"}}]}"#,
        r#"{"ape": [{"key": "Title", "value": {"type": "text", "text": "Night Drive"}}]}"#,
        r#"{"vorbis_comments": [{"key": "title", "value": {"type": "text", "text": "Night Drive"}}]}"#,
        r#"{"mp4_ilst": [{"key": "\u00a9nam", "value": {"type": "text", "text": "Night Drive"}}]}"#,
        r#"{"riff_info": [{"key": "INAM", "value": {"type": "text", "text": "Night Drive"}}]}"#,
        r#"{"aiff_text": [{"key": "NAME", "value": {"type": "text", "text": "Night Drive"}}]}"#,
        // Two blocks that disagree: one agreeing is enough.
        r#"{"id3v2": [{"key": "TIT2", "value": {"type": "text", "text": "Something Else"}}],
            "ape": [{"key": "TITLE", "value": {"type": "text", "text": "Night Drive"}}]}"#,
    ];
    for tags in blocks {
        let (lib, file, track, mounted) = renamed("Night Drive");
        lib.tags(file, tags);
        lib.relink(&mounted);
        assert!(!lib.probable(track), "{tags}");
    }
}

#[test]
fn a_title_under_another_key_doesnt_count() {
    let others = [
        r#"{"id3v2": [{"key": "TALB", "value": {"type": "text", "text": "Night Drive"}}]}"#,
        r#"{"id3v2": [{"key": "COMM::eng", "value": {"type": "text", "text": "Night Drive"}}]}"#,
        r#"{"id3v2": [{"key": "TIT2", "value": {"type": "binary", "len": 11}}]}"#,
        r#"{"ape": [{"key": "Album", "value": {"type": "text", "text": "Night Drive"}}]}"#,
    ];
    for tags in others {
        let (lib, file, track, mounted) = renamed("Night Drive");
        lib.tags(file, tags);
        lib.relink(&mounted);
        assert!(lib.probable(track), "{tags}");
    }
}

#[test]
fn titles_fold_case_punctuation_spacing_and_featuring_credits() {
    let same = [
        ("Night Drive", "night drive"),
        ("Night Drive", "  NIGHT   drive!! "),
        ("Night-Drive", "Night Drive"),
        ("Night Drive (Extended Mix)", "Night Drive [Extended Mix]"),
        ("Night Drive (feat. Someone)", "Night Drive"),
        ("Night Drive [Ft Someone]", "Night Drive"),
        ("Night Drive {featuring Someone & Other}", "Night Drive"),
        (
            "Night Drive feat. Someone (Club Mix)",
            "Night Drive (Club Mix)",
        ),
        (
            "Night Drive (feat. Someone) [Club Mix]",
            "Night Drive - Club Mix",
        ),
        // Fullwidth letters fold under NFKC.
        ("\u{ff2e}ight Drive", "Night Drive"),
        ("Caf\u{e9}", "Cafe\u{301}"),
    ];
    for (a, b) in same {
        assert_eq!(title_key(a), title_key(b), "{a:?} vs {b:?}");
    }
}

#[test]
fn titles_keep_what_tells_versions_apart() {
    let different = [
        ("Night Drive (Clean)", "Night Drive (Dirty)"),
        ("Night Drive (Original Mix)", "Night Drive (Someone Remix)"),
        ("Night Drive (Radio Edit)", "Night Drive (Extended Mix)"),
        ("Night Drive", "Night Drive VIP"),
        // "ft" inside a word is not a credit.
        ("Left Behind", "Behind"),
        ("Aftermath", "Math"),
    ];
    for (a, b) in different {
        assert_ne!(title_key(a), title_key(b), "{a:?} vs {b:?}");
    }
    assert_eq!(title_key("Left Behind"), "left behind");
    assert_eq!(title_key("Night Drive (Soft Mix)"), "night drive soft mix");
}

#[test]
fn a_title_that_is_only_punctuation_never_agrees() {
    let (lib, file, track, mounted) = renamed("!!!");
    lib.title(file, "???");
    lib.relink(&mounted);
    assert!(lib.probable(track));
}

#[test]
fn a_probable_duration_guess_gives_way_to_a_name_and_duration_match() {
    let (lib, file, track, mounted) = renamed("Night Drive");
    lib.relink(&mounted);
    assert!(lib.probable(track));
    // Same name and duration as the guessed file, from elsewhere: two
    // signals beat one, and the guess is made again from scratch.
    let other = lib.track(&loc("E:/Old/Renamed.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(other),
        Some((file, "filename_duration".to_owned(), 0.9))
    );
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_probable_match_from_a_later_step_keeps_its_file_from_other_tracks() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "New/Xi.mp3", Some(200_300));
    let track = lib.track(&loc("E:/Gone/Anything.mp3"), Some("200"));
    // A carried match (a gig stick's, 1aD-3) that's only probable.
    lib.matched_before(track, file, "gig_stick", 0.5);
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE rekordbox_track SET relink_probable = 1 WHERE id = ?1",
                [track],
            )
        })
        .unwrap();
    let other = lib.track(&loc("E:/Old/Xi.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(other), None);
    assert_eq!(lib.matched(track).map(|m| m.0), Some(file));
    assert!(lib.probable(track));
}

#[test]
fn a_probable_match_is_kept_as_it_is_on_the_next_run() {
    let (lib, _, track, mounted) = renamed("Night Drive");
    lib.relink(&mounted);
    let before = lib.all_matches();
    let again = lib.relink(&mounted);
    assert_eq!(lib.all_matches(), before);
    assert!(lib.probable(track));
    assert_eq!((again.changed, again.probable), (0, 1));
}

#[test]
fn confirming_a_probable_match_makes_it_trusted() {
    let (lib, file, track, mounted) = renamed("Night Drive");
    lib.relink(&mounted);
    assert!(lib.probable(track));
    lib.confirm(&loc("E:/Music/Album/Original.mp3"), file, "unique_duration");
    let summary = lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((file, "unique_duration".to_owned(), 1.0))
    );
    assert!(!lib.probable(track));
    assert_eq!(summary.confirmed, 1);
}

#[test]
fn a_probable_match_is_never_evidence_for_its_neighbours() {
    let (lib, music, mounted) = e_music();
    let guessed = lib.file(music, "Somewhere/Guess.mp3", Some(300_000));
    lib.file(music, "Somewhere/Renamed.mp3", Some(200_300));
    let neighbour = lib.track(&loc("D:/Old/Guess Source.mp3"), Some("300"));
    lib.matched_before(neighbour, guessed, "filename_only", 0.4);
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE rekordbox_track SET relink_probable = 1 WHERE id = ?1",
                [neighbour],
            )
        })
        .unwrap();
    let track = lib.track(&loc("D:/Old/Original.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_probable_flag_needs_a_file() {
    let lib = Lib::new();
    let track = lib.track(&loc("E:/Music/Alpha.mp3"), None);
    let refused = lib.writer.call(move |c| {
        c.execute(
            "UPDATE rekordbox_track SET relink_probable = 1 WHERE id = ?1",
            [track],
        )
    });
    assert!(refused.is_err(), "{refused:?}");
    let refused = lib.writer.call(move |c| {
        c.execute(
            "UPDATE rekordbox_track SET relink_probable = 2 WHERE id = ?1",
            [track],
        )
    });
    assert!(refused.is_err(), "{refused:?}");
}
