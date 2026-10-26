//! The "Quality problem" chip means what Review's "Quality problems" group
//! shows (ROADMAP 1.11, 1.12), by the same SQL text.

use super::*;
use crate::review::{ReviewGroup, ReviewKind};

/// A track with a file, in the Library, whose file has an audio hash (the
/// key Review's decisions are about) and whose current verdict is `verdict`.
struct Case {
    track: i64,
    file: i64,
}

fn case(lib: &Lib, name: &'static str, verdict: Option<&str>) -> Case {
    let (track, file) = lib.track(Some(name), Some("Someone"), name);
    lib.library(track, file);
    lib.insert(
        "UPDATE file SET audio_hash = randomblob(16), codec = 'mp3', bitrate = 128 WHERE id = ?1",
        (file,),
    );
    if let Some(verdict) = verdict {
        current_verdict(lib, file, verdict);
    }
    Case { track, file }
}

/// A verdict as the quality job leaves it: for the rules and the inputs of
/// now.
fn current_verdict(lib: &Lib, file: i64, verdict: &str) {
    let sql = format!(
        "INSERT OR REPLACE INTO file_verdict (file_id, rules_version, inputs, verdict, findings)
         SELECT f.id, {}, {}, ?2, '[]' FROM file f
         LEFT JOIN file_quality q ON q.file_id = f.id WHERE f.id = ?1",
        crate::quality::RULES_VERSION,
        crate::quality::store::INPUTS
    );
    let verdict = verdict.to_owned();
    lib.writer
        .call(move |c| c.execute(&sql, (file, verdict)))
        .unwrap();
}

fn decide(lib: &Lib, kind: &'static str, file: i64) {
    lib.insert(
        "INSERT INTO review_decision (kind, linked_key)
         SELECT ?1, audio_hash FROM file WHERE id = ?2",
        (kind, file),
    );
}

/// The ids the chip finds in `source`, once the index is current.
fn chip(lib: &Lib, source: TableSource) -> Vec<i64> {
    let mut found = ids(&run(
        lib,
        TableQuery {
            filters: vec![TableFilter::QualityProblem],
            ..query(source)
        },
    ));
    found.sort_unstable();
    found
}

/// The tracks of Review's "Quality problems" group.
fn review_group(lib: &Lib) -> Vec<i64> {
    let mut found: Vec<i64> = lib
        .writer
        .call(|c| crate::review::items(c, &Mount, ReviewGroup::QualityProblems, 0, 1000))
        .unwrap()
        .items
        .iter()
        .inspect(|item| {
            assert!(matches!(
                item.id.kind,
                ReviewKind::QualityProblem | ReviewKind::BetterFile
            ))
        })
        .map(|item| item.id.a)
        .collect();
    found.sort_unstable();
    found
}

#[test]
fn a_flagged_track_is_a_quality_problem_until_it_has_a_clean_verdict() {
    let lib = Lib::new();
    case(&lib, "clean.mp3", Some("ok"));
    case(&lib, "unjudged.mp3", None);
    let flagged = case(&lib, "flagged.mp3", Some("low_bitrate"));
    let broken = case(&lib, "broken.mp3", Some("broken"));
    assert_eq!(
        chip(&lib, TableSource::Library),
        [flagged.track, broken.track]
    );
    current_verdict(&lib, flagged.file, "ok");
    assert_eq!(chip(&lib, TableSource::Library), [broken.track]);
}

#[test]
fn a_track_the_user_answered_keep_anyway_or_find_a_better_file_leaves_the_chip() {
    let lib = Lib::new();
    let kept = case(&lib, "kept.mp3", Some("low_bitrate"));
    let wanted = case(&lib, "wanted.mp3", Some("suspect_transcode"));
    let open = case(&lib, "open.mp3", Some("truncated"));
    assert_eq!(
        chip(&lib, TableSource::Library),
        [kept.track, wanted.track, open.track]
    );
    decide(&lib, "keep_anyway", kept.file);
    decide(&lib, "want_better", wanted.file);
    assert_eq!(chip(&lib, TableSource::Library), [open.track]);
    // The answer is about that audio: another file in its place is a problem again.
    lib.insert(
        "UPDATE file SET audio_hash = randomblob(16) WHERE id = ?1",
        (kept.file,),
    );
    // (Its verdict is made again for the new audio, as the quality job does.)
    current_verdict(&lib, kept.file, "low_bitrate");
    assert_eq!(chip(&lib, TableSource::Library), [kept.track, open.track]);
}

#[test]
fn a_stale_verdict_is_not_counted() {
    let lib = Lib::new();
    let stale_rules = case(&lib, "old rules.mp3", Some("low_bitrate"));
    let stale_inputs = case(&lib, "changed file.mp3", Some("low_bitrate"));
    let current = case(&lib, "current.mp3", Some("low_bitrate"));
    assert_eq!(
        chip(&lib, TableSource::Library),
        [stale_rules.track, stale_inputs.track, current.track]
    );
    // The rules changed since the verdict was made: no row of the track
    // changes, and the chip follows at once all the same.
    lib.insert(
        "UPDATE file_verdict SET rules_version = rules_version + 1 WHERE file_id = ?1",
        (stale_rules.file,),
    );
    // What the verdict was made from changed.
    lib.insert(
        "UPDATE file SET bitrate = 320 WHERE id = ?1",
        (stale_inputs.file,),
    );
    assert_eq!(chip(&lib, TableSource::Library), [current.track]);
}

#[test]
fn a_file_that_is_not_on_disk_is_not_a_quality_problem() {
    let lib = Lib::new();
    let gone = case(&lib, "gone.mp3", Some("broken"));
    let here = case(&lib, "here.mp3", Some("broken"));
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (gone.file,));
    assert_eq!(chip(&lib, TableSource::Library), [here.track]);
}

#[test]
fn a_better_file_item_whose_linked_file_is_still_there_is_a_quality_problem_even_when_the_file_is_clean(
) {
    let lib = Lib::new();
    let clean_with_better = case(&lib, "clean.mp3", Some("ok"));
    let flagged_with_better = case(&lib, "flagged.mp3", Some("low_bitrate"));
    let gone_with_copy = case(&lib, "gone.mp3", Some("ok"));
    let dismissed = case(&lib, "dismissed.mp3", None);
    let (_, other_file) = lib.track(Some("Better"), Some("Someone"), "better.flac");
    lib.insert(
        "UPDATE file SET audio_hash = randomblob(16) WHERE id = ?1",
        (other_file,),
    );
    for (track, step) in [
        (clean_with_better.track, "format"),
        (flagged_with_better.track, "verdict"),
        (gone_with_copy.track, "onDisk"),
        (dismissed.track, "format"),
    ] {
        lib.insert(
            "INSERT INTO better_file (recording_id, file_id, step) VALUES (?1, ?2, ?3)",
            (track, other_file, step.to_owned()),
        );
    }
    lib.insert(
        "INSERT INTO review_decision (kind, linked_key, other_key)
         SELECT 'dismissed_better', l.audio_hash, o.audio_hash FROM file l, file o
         WHERE l.id = ?1 AND o.id = ?2",
        (dismissed.file, other_file),
    );
    // "Linked file gone, copy found" is another group's; a dismissed
    // "Better file available" is no item.
    assert_eq!(
        chip(&lib, TableSource::Library),
        [clean_with_better.track, flagged_with_better.track]
    );
}

#[test]
fn only_library_tracks_can_be_quality_problems() {
    let lib = Lib::new();
    let library = case(&lib, "library.mp3", Some("broken"));
    let (outside, outside_file) = lib.track(Some("Outside"), Some("Someone"), "outside.mp3");
    current_verdict(&lib, outside_file, "broken");
    assert_eq!(chip(&lib, TableSource::AllMusic), [library.track]);
    assert!(!chip(&lib, TableSource::AllMusic).contains(&outside));
}

#[test]
fn the_chip_finds_in_a_crate_the_tracks_of_the_crate_that_review_lists() {
    let lib = Lib::new();
    let inside = case(&lib, "inside.mp3", Some("broken"));
    let outside = case(&lib, "outside.mp3", Some("broken"));
    lib.insert(
        "INSERT INTO crate (name, kind) VALUES ('One', 'static')",
        (),
    );
    lib.insert(
        "INSERT INTO crate_entry (crate_id, library_track_id, kind)
         SELECT 1, id, 'member' FROM library_track WHERE recording_id = ?1",
        (inside.track,),
    );
    assert_eq!(
        chip(&lib, TableSource::Crate { id: CrateId(1) }),
        [inside.track]
    );
    assert_eq!(
        chip(&lib, TableSource::Library),
        [inside.track, outside.track]
    );
}

#[test]
fn the_chip_is_the_set_of_tracks_review_lists_as_quality_problems_on_a_mixed_library() {
    let lib = Lib::new();
    // Every way a track can be flagged, answered, stale, gone or improvable.
    let flagged = case(&lib, "flagged.mp3", Some("low_bitrate"));
    let transcode = case(&lib, "transcode.mp3", Some("suspect_transcode"));
    let cut_off = case(&lib, "cut off.mp3", Some("truncated"));
    let kept = case(&lib, "kept.mp3", Some("broken"));
    let wanted = case(&lib, "wanted.mp3", Some("broken"));
    let stale = case(&lib, "stale.mp3", Some("broken"));
    let gone = case(&lib, "gone.mp3", Some("broken"));
    case(&lib, "clean.mp3", Some("ok"));
    let improvable = case(&lib, "improvable.mp3", Some("ok"));
    let improvable_flagged = case(&lib, "improvable flagged.mp3", Some("broken"));
    let copy_found = case(&lib, "copy found.mp3", None);
    let (_, better_file) = lib.track(Some("Better"), Some("Someone"), "better.flac");
    decide(&lib, "keep_anyway", kept.file);
    decide(&lib, "want_better", wanted.file);
    lib.insert(
        "UPDATE file_verdict SET inputs = '[]' WHERE file_id = ?1",
        (stale.file,),
    );
    lib.insert("UPDATE file SET present = 0 WHERE id = ?1", (gone.file,));
    for (track, step) in [
        (improvable.track, "format"),
        (improvable_flagged.track, "cutoff"),
        (copy_found.track, "onDisk"),
    ] {
        lib.insert(
            "INSERT INTO better_file (recording_id, file_id, step) VALUES (?1, ?2, ?3)",
            (track, better_file, step.to_owned()),
        );
    }
    // A track outside the Library is in neither.
    let (_, outside_file) = lib.track(Some("Outside"), Some("Someone"), "outside.mp3");
    current_verdict(&lib, outside_file, "broken");

    let mut expected = vec![
        flagged.track,
        transcode.track,
        cut_off.track,
        improvable.track,
        improvable_flagged.track,
    ];
    expected.sort_unstable();
    assert_eq!(review_group(&lib), expected, "Review's group");
    assert_eq!(chip(&lib, TableSource::Library), expected, "the chip");
    assert_eq!(
        chip(&lib, TableSource::AllMusic),
        expected,
        "the chip in All music"
    );
    // And the same once the answers are taken back.
    lib.insert("DELETE FROM review_decision", ());
    lib.insert("DELETE FROM better_file", ());
    lib.insert("UPDATE file SET present = 1", ());
    assert_eq!(chip(&lib, TableSource::Library), review_group(&lib));
}
