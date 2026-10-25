//! What an "Unclear duplicates" item says holds it (1bF-3, DOGFOOD B-3): the
//! reason line names the files in the table that hold the set, and why. One
//! test for each thing that can hold a set (`held::Holding`), and the case
//! the owner met: every file but one holds the same audio, and that one
//! file's pair with one of the others goes to review.

use super::*;
use crate::review::held::Holding;
use crate::suggest::{Reason, ReasonParam};

/// The text parameter `name` of a reason.
fn text(reason: &Reason, name: &str) -> String {
    match reason.params().get(name) {
        Some(ReasonParam::Text(text)) => text.clone(),
        other => panic!("{name} is {other:?}"),
    }
}

/// The name a comparison's row for `file` shows.
fn row_name(c: &ReviewComparison, file: i64) -> String {
    c.rows
        .iter()
        .find(|row| row.file_id == Some(file))
        .unwrap_or_else(|| panic!("no row for file {file}"))
        .name
        .clone()
}

/// The one reason a held set's line is made of.
fn held_reason(c: &ReviewComparison) -> &Reason {
    assert_eq!(c.doubt.len(), 1, "{:?}", c.doubt);
    &c.doubt[0]
}

/// Every reason of the item is a duplicate pair's own, in the words of a
/// possible version mix-up: nothing stored names what holds the set.
fn old_reason(c: &ReviewComparison) {
    assert!(!c.doubt.is_empty());
    for reason in &c.doubt {
        assert!(
            reason.key().starts_with("review:doubt."),
            "{}",
            reason.key()
        );
    }
}

/// A stored pair, with the reasons the rules gave it.
fn pair_with(lib: &Lib, x: i64, y: i64, outcome: &str, state: &str, reasons: &str) {
    let (a, b) = (x.min(y), x.max(y));
    // A cut or a rework always names the link it asks for.
    let link = match outcome {
        "cut" | "rework" => "'parser'",
        _ => "NULL",
    };
    lib.exec(&format!(
        "INSERT INTO grouping_pair (file_a, file_b, outcome, reasons, state, link_source)
         VALUES ({a}, {b}, '{outcome}', '{reasons}', '{state}', {link})"
    ));
}

/// Gives the files the audio hash `hash` (a hex literal body).
fn hash(lib: &Lib, files: &[i64], hash: &str) {
    let ids: Vec<String> = files.iter().map(i64::to_string).collect();
    lib.exec(&format!(
        "UPDATE file SET audio_hash = x'{hash}' WHERE id IN ({})",
        ids.join(", ")
    ));
}

const REWORK: &str = r#"[{"code":"sameAudioNamesDisagree"},{"code":"namesSayRework"}]"#;

/// Three files of one track hold the very same audio (the Library's file is
/// the first), and a fourth file in another track holds other audio. Its
/// pair with two of them is a duplicate; its pair with the middle one goes
/// to review. That one pair holds the whole set.
#[test]
fn one_file_whose_pair_goes_to_review_holds_a_set_of_identical_files() {
    let lib = Lib::new();
    let track = lib.track("Velo");
    let first = lib.file(track, "Velo.mp3", "mp3", true);
    let middle = lib.file(track, "Velo (copy).mp3", "mp3", true);
    let last = lib.file(track, "Velo (copy 2).mp3", "mp3", true);
    hash(&lib, &[first, middle, last], "0A");
    lib.library(track, first);
    let other_track = lib.track("Velo VIP");
    let outlier = lib.file(other_track, "Velo (VIP).mp3", "mp3", true);
    hash(&lib, &[outlier], "0B");
    lib.pair(first, outlier, "duplicate", "set_disagrees");
    lib.pair(last, outlier, "duplicate", "set_disagrees");
    pair_with(&lib, middle, outlier, "review", "open", REWORK);

    let c = lib
        .compare(id(ReviewKind::HeldSet, track.min(other_track), None))
        .unwrap();
    assert_eq!(c.rows.len(), 4);
    let reason = held_reason(&c);
    assert_eq!(reason.key(), "review:held.pairTwo");
    // The two files of the pair that disagrees, as the table names them, and
    // the pair's own reasons, the same codes a possible version mix-up shows.
    assert_eq!(text(reason, "file"), row_name(&c, middle));
    assert_eq!(text(reason, "other"), row_name(&c, outlier));
    assert_eq!(text(reason, "first"), "sameAudioNamesDisagree");
    assert_eq!(text(reason, "second"), "namesSayRework");
    // The pair is also an item of its own, with those reasons.
    let doubt = lib
        .compare(id(
            ReviewKind::VersionDoubt,
            track.min(other_track),
            Some(track.max(other_track)),
        ))
        .unwrap();
    let keys: Vec<_> = doubt.doubt.iter().map(|r| r.key().to_owned()).collect();
    assert_eq!(
        keys,
        [
            "review:doubt.sameAudioNamesDisagree",
            "review:doubt.namesSayRework"
        ]
    );
}

/// A pair that links as cuts or reworks, or is called different, holds a
/// set as well as one that goes to review; a pair someone settled, or
/// between files that hold the same audio or aren't on disk, does not.
#[test]
fn every_kind_of_pair_that_disagrees_holds_a_set_and_a_settled_one_does_not() {
    for (outcome, state, holds) in [
        ("review", "open", true),
        ("cut", "linked", true),
        ("rework", "linked", true),
        ("different", "apart", true),
        ("review", "decided", false),
        ("cut", "decided", false),
    ] {
        let lib = Lib::new();
        let (t1, f1) = lib.simple("Velo");
        let (_, f2) = lib.simple("Velo b");
        let (_, f3) = lib.simple("Velo c");
        lib.library(t1, f1);
        lib.pair(f1, f2, "duplicate", "set_disagrees");
        lib.pair(f2, f3, "duplicate", "set_disagrees");
        pair_with(&lib, f1, f3, outcome, state, r#"[{"code":"namesSayCut"}]"#);
        let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
        match holds {
            true => {
                let reason = held_reason(&c);
                assert_eq!(reason.key(), "review:held.pairOne", "{outcome} {state}");
                assert_eq!(text(reason, "file"), row_name(&c, f1));
                assert_eq!(text(reason, "other"), row_name(&c, f3));
                assert_eq!(text(reason, "first"), "namesSayCut");
            }
            false => old_reason(&c),
        }
    }
}

#[test]
fn a_pair_with_a_file_that_is_not_on_disk_is_not_named() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo b");
    // A third track is in the set through a file that is on disk and a
    // duplicate pair; its other file is gone, and the pair with that one
    // goes to review. The job ignores a file that is not on disk.
    let t3 = lib.track("Velo c");
    let there = lib.file(t3, "Velo c.mp3", "mp3", true);
    let gone = lib.file(t3, "Velo c gone.mp3", "mp3", false);
    lib.library(t1, f1);
    lib.pair(f1, f2, "duplicate", "set_disagrees");
    lib.pair(f2, there, "duplicate", "set_disagrees");
    pair_with(&lib, f1, gone, "review", "open", REWORK);
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    assert_eq!(c.tracks.len(), 3);
    old_reason(&c);

    // The same pair with the file back on disk is named.
    lib.exec("UPDATE file SET present = 1");
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    assert_eq!(held_reason(&c).key(), "review:held.pairTwo");
    assert_eq!(text(held_reason(&c), "other"), row_name(&c, gone));
}

#[test]
fn a_pair_between_files_that_hold_the_same_audio_is_not_named() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo b");
    let (_, same) = lib.simple("Velo d");
    lib.library(t1, f1);
    hash(&lib, &[f1, same], "0D");
    hash(&lib, &[f2], "0E");
    lib.pair(f1, f2, "duplicate", "set_disagrees");
    lib.pair(f2, same, "duplicate", "set_disagrees");
    pair_with(&lib, f1, same, "review", "open", REWORK);
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    old_reason(&c);
}

/// The planner skips a pair only for the same `audio_hash`; files with none
/// and the same `blake3` are still compared, so their pair is named.
#[test]
fn a_pair_between_files_with_no_audio_hash_and_the_same_blake3_is_named() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo b");
    let (_, f3) = lib.simple("Velo c");
    lib.library(t1, f1);
    lib.exec(&format!(
        "UPDATE file SET blake3 = zeroblob(32) WHERE id IN ({f1}, {f3})"
    ));
    lib.pair(f1, f2, "duplicate", "set_disagrees");
    lib.pair(f2, f3, "duplicate", "set_disagrees");
    pair_with(&lib, f1, f3, "review", "open", REWORK);
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    assert_eq!(held_reason(&c).key(), "review:held.pairTwo");
}

/// With several pairs that hold the set, the one with the lowest file ids.
#[test]
fn when_several_pairs_hold_a_set_the_one_with_the_lowest_files_is_named() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo b");
    let (_, f3) = lib.simple("Velo c");
    let (_, f4) = lib.simple("Velo d");
    lib.library(t1, f1);
    lib.pair(f1, f2, "duplicate", "set_disagrees");
    lib.pair(f2, f3, "duplicate", "set_disagrees");
    lib.pair(f3, f4, "duplicate", "set_disagrees");
    pair_with(&lib, f2, f4, "review", "open", REWORK);
    pair_with(&lib, f1, f3, "review", "open", REWORK);
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    let reason = held_reason(&c);
    assert_eq!(text(reason, "file"), row_name(&c, f1));
    assert_eq!(text(reason, "other"), row_name(&c, f3));
}

#[test]
fn a_pair_gives_at_most_two_reasons_and_only_those_with_words() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo b");
    let (_, f3) = lib.simple("Velo c");
    lib.library(t1, f1);
    lib.pair(f1, f2, "duplicate", "set_disagrees");
    lib.pair(f2, f3, "duplicate", "set_disagrees");
    // The first has no words in the locale files: it is left out. With the
    // sentence itself, two reasons are the three fragments copy-style allows.
    pair_with(
        &lib,
        f1,
        f3,
        "review",
        "open",
        r#"[{"code":"truncatedCopy"},{"code":"sameAudio"},{"code":"namesDiffer"},{"code":"titlesDiffer"}]"#,
    );
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    let reason = held_reason(&c);
    assert_eq!(reason.key(), "review:held.pairTwo");
    assert_eq!(text(reason, "first"), "sameAudio");
    assert_eq!(text(reason, "second"), "namesDiffer");
    assert!(reason.params().get("third").is_none());

    // With none, the sentence still names the two files.
    lib.exec(
        r#"UPDATE grouping_pair SET reasons = '[{"code":"truncatedCopy"}]' WHERE outcome = 'review'"#,
    );
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    assert_eq!(held_reason(&c).key(), "review:held.pair");
}

/// Two of the set's tracks are in the Library, and no pair is between them
/// that anyone has to decide first.
#[test]
fn two_library_tracks_hold_the_copy_that_matches_both() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (t2, f2) = lib.simple("Velo b");
    let (_, copy) = lib.simple("Velo c");
    lib.library(t1, f1);
    lib.library(t2, f2);
    lib.pair(f1, copy, "duplicate", "set_disagrees");
    lib.pair(f2, copy, "duplicate", "set_disagrees");
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    let reason = held_reason(&c);
    assert_eq!(reason.key(), "review:held.library");
    // The Library file of each of the two tracks.
    assert_eq!(text(reason, "file"), row_name(&c, f1));
    assert_eq!(text(reason, "other"), row_name(&c, f2));
    assert!(c.rows.iter().filter(|row| row.linked).count() == 2);
}

#[test]
fn a_decision_that_two_tracks_are_not_the_same_holds_their_set() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (_, f2) = lib.simple("Velo b");
    let (_, f3) = lib.simple("Velo c");
    lib.library(t1, f1);
    hash(&lib, &[f1], "01");
    hash(&lib, &[f2], "02");
    hash(&lib, &[f3], "03");
    lib.pair(f1, f3, "duplicate", "set_disagrees");
    lib.pair(f2, f3, "duplicate", "set_disagrees");
    // The user kept the first two apart (their audio, the lower key first).
    lib.exec(
        "INSERT INTO grouping_decision (key_a, key_b, decision) VALUES (x'01', x'02', 'apart')",
    );
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    let reason = held_reason(&c);
    assert_eq!(reason.key(), "review:held.keptSeparate");
    assert_eq!(text(reason, "file"), row_name(&c, f1));
    assert_eq!(text(reason, "other"), row_name(&c, f2));

    // A decision that they are the same holds nothing.
    lib.exec("UPDATE grouping_decision SET decision = 'same'");
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    old_reason(&c);
}

#[test]
fn two_tracks_linked_as_versions_hold_their_set() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (t2, f2) = lib.simple("Velo b");
    let (_, f3) = lib.simple("Velo c");
    lib.library(t1, f1);
    lib.exec(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({t1}, {t2}, 'cut', 'user')"
    ));
    lib.pair(f1, f3, "duplicate", "set_disagrees");
    lib.pair(f2, f3, "duplicate", "set_disagrees");
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    let reason = held_reason(&c);
    assert_eq!(reason.key(), "review:held.linked");
    assert_eq!(text(reason, "file"), row_name(&c, f1));
    assert_eq!(text(reason, "other"), row_name(&c, f2));
}

#[test]
fn a_set_with_more_files_than_the_job_compares_says_how_many() {
    let lib = Lib::new();
    let t1 = lib.track("Velo");
    let t2 = lib.track("Velo b");
    let mut files = Vec::new();
    for n in 0..26 {
        files.push(lib.file(t1, &format!("a{n}.mp3"), "mp3", true));
        files.push(lib.file(t2, &format!("b{n}.mp3"), "mp3", true));
    }
    lib.library(t1, files[0]);
    lib.pair(files[0], files[1], "duplicate", "set_disagrees");
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    let reason = held_reason(&c);
    assert_eq!(reason.key(), "review:held.tooMany");
    assert_eq!(
        reason.params().get("files"),
        Some(&ReasonParam::Number(52.0))
    );
    assert_eq!(
        reason.params().get("most"),
        Some(&ReasonParam::Number(50.0))
    );

    // The same files all holding one kind of audio hold nothing. (A file's
    // pairs go with the audio it held: the pair is stored again.)
    hash(&lib, &files, "0C");
    lib.pair(files[0], files[1], "duplicate", "set_disagrees");
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    old_reason(&c);
}

/// Which of the things that can hold a set it names, when several do.
#[test]
fn a_set_held_in_several_ways_names_one_in_a_fixed_order() {
    let lib = Lib::new();
    let (t1, f1) = lib.simple("Velo");
    let (t2, f2) = lib.simple("Velo b");
    let (t3, f3) = lib.simple("Velo c");
    lib.library(t1, f1);
    lib.pair(f1, f3, "duplicate", "set_disagrees");
    lib.pair(f2, f3, "duplicate", "set_disagrees");
    lib.exec(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({t1}, {t2}, 'cut', 'user')"
    ));
    let holding = |lib: &Lib| {
        lib.writer
            .call(move |c| crate::review::held::holding(c, &[t1, t2, t3]))
            .unwrap()
    };
    assert!(matches!(
        holding(&lib),
        Some(Holding::LinkedVersions { .. })
    ));
    // A pair that disagrees comes before the link it may have made.
    pair_with(&lib, f1, f2, "cut", "linked", r#"[{"code":"namesSayCut"}]"#);
    assert!(matches!(holding(&lib), Some(Holding::Pair { .. })));
    // Two Library tracks come before both.
    lib.library(t2, f2);
    assert!(matches!(holding(&lib), Some(Holding::TwoInLibrary { .. })));
}

/// A set of `n` files of different audio, alternately in each of two tracks, with
/// one duplicate pair between them, as the job would hold it.
fn set_of(lib: &Lib, n: usize) -> (i64, Vec<i64>) {
    let t1 = lib.track("Velo");
    let t2 = lib.track("Velo b");
    let mut files = Vec::new();
    for at in 0..n {
        let track = if at % 2 == 0 { t1 } else { t2 };
        files.push(lib.file(track, &format!("f{at:03}.mp3"), "mp3", true));
    }
    lib.library(t1, files[0]);
    lib.pair(files[0], files[1], "duplicate", "set_disagrees");
    (t1, files)
}

/// The job compares sets of up to 50 files; one file more and it holds the
/// set, so only then does the reason say so.
#[test]
fn a_set_of_exactly_fifty_files_is_not_too_many() {
    let lib = Lib::new();
    let (t1, _) = set_of(&lib, 50);
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    old_reason(&c);

    let lib = Lib::new();
    let (t1, _) = set_of(&lib, 51);
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    assert_eq!(held_reason(&c).key(), "review:held.tooMany");
    assert_eq!(
        held_reason(&c).params().get("files"),
        Some(&ReasonParam::Number(51.0))
    );
}

/// The table lists 50 files; the two a reason names are in it even when
/// they rank after the 50th.
#[test]
fn the_files_a_reason_names_are_always_in_the_table() {
    let lib = Lib::new();
    let (t1, files) = set_of(&lib, 60);
    // The pair that disagrees is between two of the last files.
    pair_with(&lib, files[58], files[59], "review", "open", REWORK);
    let c = lib.compare(id(ReviewKind::HeldSet, t1, None)).unwrap();
    assert_eq!(c.rows.len(), 50);
    assert_eq!(c.more_files, 10);
    let reason = held_reason(&c);
    assert_eq!(reason.key(), "review:held.pairTwo");
    assert_eq!(text(reason, "file"), row_name(&c, files[58]));
    assert_eq!(text(reason, "other"), row_name(&c, files[59]));
    // The suggested file is still the first row.
    assert_eq!(c.suggestion.as_ref().map(|s| *s.what()), c.rows[0].file_id);
}
