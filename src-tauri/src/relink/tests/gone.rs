//! Step 4: a rekordbox track whose file is gone is matched by what the app
//! knew about that file's audio (1aD-1).

use super::super::rules::{self, Input, MAX_COMPARISONS};
use super::*;
use crate::fingerprint::compare;

/// A library where the file at `E:/Music/Old.mp3` (hashed as audio 1,
/// lasting 200 s, fingerprinted as song 1) was matched by path and has
/// since gone. Returns the library, the folder, what's mounted, the gone
/// file and the rekordbox track.
fn moved() -> (Lib, i64, Mounted, i64, i64) {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    lib.fingerprinted(old, &song(1, 200));
    let track = lib.track_with(&loc("E:/Music/Old.mp3"), Some("200"), &[("Name", "Kappa")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(old));
    lib.gone(old);
    (lib, music, mounted, old, track)
}

/// A present file that doesn't match by name or duration (steps 2, 3 and
/// 5 pass it by): other name, a folder of its own, not read yet.
fn elsewhere(lib: &Lib, music: i64, name: &str) -> i64 {
    lib.file(music, &format!("Moved {name}/{name}.flac"), None)
}

#[test]
fn the_made_up_fingerprints_compare_as_the_real_ones_do() {
    let original = song(1, 200);
    assert!(compare(&original, &reencoded(&original))
        .unwrap()
        .is_duplicate());
    assert!(!compare(&original, &song(2, 200)).unwrap().is_duplicate());
}

#[test]
fn a_moved_file_is_matched_by_its_audio_hash() {
    let (lib, music, mounted, _old, track) = moved();
    let new = elsewhere(&lib, music, "New");
    lib.hashed(new, 1);
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(new));
    assert!(!lib.probable(track));
    assert_eq!(summary.fingerprint, 1);
}

#[test]
fn a_gone_file_with_no_file_of_its_audio_left_stays_missing() {
    let (lib, music, mounted, _old, track) = moved();
    let other = elsewhere(&lib, music, "Other");
    lib.hashed(other, 2);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

/// A present file that lasts about as long as the gone file of [`moved`]
/// (200.4 s against 200 s, as another encode of it would), and that
/// nothing else leads to: other name, a folder of its own, no fingerprint.
fn encode(lib: &Lib, music: i64, name: &str) -> i64 {
    lib.file(music, &format!("Moved {name}/{name}.flac"), Some(200_400))
}

/// Another encode of the gone file's audio (another `audio_hash`), titled
/// `title`, that the grouping job merged with the gone file `old`: one
/// track, their pair a duplicate, both decided under the current rules.
/// Nothing but the track leads to it ([`encode`]).
fn mate_of(lib: &Lib, music: i64, old: i64, title: &str) -> i64 {
    mate_lasting(lib, music, old, title, Some(200_400))
}

/// [`mate_of`], lasting `lasts` ms (`None`: not read yet).
fn mate_lasting(lib: &Lib, music: i64, old: i64, title: &str, lasts: Option<i64>) -> i64 {
    let mate = lib.file(music, "Moved Mate/Mate.flac", lasts);
    lib.hashed(mate, 2);
    lib.title(mate, title);
    lib.merged(&[old, mate]);
    mate
}

#[test]
fn another_present_file_of_the_gone_files_track_is_matched_when_grouping_is_current() {
    let (lib, music, mounted, old, track) = moved();
    let mate = mate_of(&lib, music, old, "Kappa");
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_track(mate));
    assert!(!lib.probable(track));
    assert_eq!((summary.fingerprint, summary.probable), (1, 0));
    assert_eq!(lib.recording(track), lib.recording_of(mate));
    // The audio the match was made to is the mate's.
    assert_eq!(lib.evidence(track), Some(2));
}

#[test]
fn a_track_mate_whose_title_disagrees_with_rekordboxs_name_is_only_probable() {
    // The entry was renamed in rekordbox, or the mate has no title tag:
    // grouping compared the two files' names, never rekordbox's.
    for title in ["Kappa (Dirty)", ""] {
        let (lib, music, mounted, old, track) = moved();
        let mate = mate_of(&lib, music, old, title);
        let summary = lib.relink(&mounted);
        assert_eq!(
            lib.matched(track),
            Some((mate, "fingerprint".to_owned(), 0.4)),
            "{title:?}"
        );
        assert!(lib.probable(track));
        assert_eq!((summary.fingerprint, summary.probable), (0, 1));
    }
}

#[test]
fn a_track_mate_of_another_length_is_only_probable_whatever_its_title() {
    // The gone file lasted 200 s. A copy with a longer lead-in, a cut-off
    // one, and the nearest lengths over the limit either way: the entry's
    // cues and grid are positions in time, and wouldn't be where they were.
    for lasts in [212_000, 185_000, 201_001, 198_999] {
        let (lib, music, mounted, old, track) = moved();
        let mate = mate_lasting(&lib, music, old, "Kappa", Some(lasts));
        let summary = lib.relink(&mounted);
        assert_eq!(
            lib.matched(track),
            Some((mate, "fingerprint".to_owned(), 0.4)),
            "{lasts} ms"
        );
        assert!(lib.probable(track), "{lasts} ms");
        assert_eq!((summary.fingerprint, summary.probable), (0, 1));
    }
}

#[test]
fn a_track_mate_one_second_longer_or_shorter_than_the_gone_file_is_still_trusted() {
    for lasts in [201_000, 199_000, 200_000] {
        let (lib, music, mounted, old, track) = moved();
        let mate = mate_lasting(&lib, music, old, "Kappa", Some(lasts));
        lib.relink(&mounted);
        assert_eq!(lib.matched(track), same_track(mate), "{lasts} ms");
        assert!(!lib.probable(track), "{lasts} ms");
    }
}

#[test]
fn a_track_mate_whose_length_isnt_known_is_only_probable() {
    let (lib, music, mounted, old, track) = moved();
    // Not read yet.
    let mate = mate_lasting(&lib, music, old, "Kappa", None);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((mate, "fingerprint".to_owned(), 0.4))
    );
    assert!(lib.probable(track));
    // Read: it lasts about as long as the gone file did.
    lib.read(mate, 200_400);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_track(mate));
    assert!(!lib.probable(track));
}

#[test]
fn a_track_mate_of_a_gone_file_whose_length_was_never_known_is_only_probable() {
    let (lib, music, mounted) = e_music();
    // Gone before it was read; hashed, so its record says what it held.
    let old = lib.file(music, "Old.mp3", None);
    lib.hashed(old, 1);
    let track = lib.track_with(&loc("E:/Music/Old.mp3"), Some("200"), &[("Name", "Kappa")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(old));
    lib.gone(old);
    let mate = mate_of(&lib, music, old, "Kappa");
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((mate, "fingerprint".to_owned(), 0.4))
    );
    assert!(lib.probable(track));
}

#[test]
fn running_again_with_nothing_changed_keeps_the_track_mate_match() {
    let (lib, music, mounted, old, track) = moved();
    let mate = mate_of(&lib, music, old, "Kappa");
    lib.relink(&mounted);
    let before = lib.all_states();
    lib.relink(&mounted);
    assert_eq!(lib.all_states(), before);
    assert_eq!(lib.matched(track), same_track(mate));
    // A fresh rekordbox read decides it the same way.
    lib.fresh_read();
    lib.relink(&mounted);
    assert_eq!(lib.all_states(), before);
}

#[test]
fn a_track_mate_without_a_grouping_record_isnt_matched() {
    let (lib, music, mounted, old, track) = moved();
    // In the gone file's track, and nothing says how it got there.
    let mate = encode(&lib, music, "Mate");
    lib.hashed(mate, 2);
    lib.title(mate, "Kappa");
    lib.group(&[old, mate]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // Nor when only the pair is stored, or only the gone file is decided.
    lib.pair(old, mate, "duplicate", "merged");
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    lib.decided(old);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // With the mate decided too, it is matched.
    lib.decided(mate);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_track(mate));
}

#[test]
fn a_track_mate_due_to_be_decided_again_isnt_matched() {
    let (lib, music, mounted, old, track) = moved();
    let mate = mate_of(&lib, music, old, "Kappa (Clean)");
    // Its tags are rewritten: the grouping job has to decide it again.
    lib.title(mate, "Kappa");
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_track_mate_decided_under_other_rules_isnt_matched() {
    let (lib, music, mounted, old, track) = moved();
    let mate = mate_of(&lib, music, old, "Kappa");
    // As a build with another rules version finds its record.
    lib.decided_under_other_rules(mate);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_gone_file_that_was_due_when_it_went_isnt_matched_to_its_track_mate() {
    for due in [
        Lib::retagged as fn(&Lib, i64),
        Lib::decided_under_other_rules,
    ] {
        let (lib, music, mounted, old, track) = moved();
        mate_of(&lib, music, old, "Kappa");
        // What a file leaves that was retagged just before it went, or
        // that went before a build with other rules: due, and never
        // decided again while it's away.
        due(&lib, old);
        lib.relink(&mounted);
        assert_eq!(lib.matched(track), None);
    }
}

#[test]
fn a_gone_file_that_changed_before_it_went_isnt_matched_to_its_track_mate() {
    let (lib, music, mounted, old, track) = moved();
    mate_of(&lib, music, old, "Kappa");
    // Changed on disk and never hashed again before it went: grouping
    // still has it with its old audio, and nothing says what rekordbox's
    // entry was last made on.
    lib.edited(old);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_gone_file_overwritten_with_other_audio_before_it_went_isnt_matched_to_its_track_mate() {
    let (lib, music, mounted, old, track) = moved();
    mate_of(&lib, music, old, "Kappa");
    // Overwritten and hashed again, then gone before grouping ran: it
    // stays in the track (a gone file keeps its track) with other audio
    // than it was placed with.
    lib.edited(old);
    lib.hashed(old, 9);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_track_mate_the_rules_no_longer_call_a_duplicate_isnt_matched() {
    let (lib, music, mounted, old, track) = moved();
    let mate = mate_of(&lib, music, old, "Kappa");
    // A merge is never undone: the two still share the track, both decided
    // under the current rules, and the rules say they aren't duplicates.
    lib.pair(old, mate, "review", "in_one_track");
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // A duplicate again, but no grouping pass has stood by it yet.
    lib.pair(old, mate, "duplicate", "in_one_track");
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_track_mate_just_decided_not_a_duplicate_isnt_matched_before_the_grouping_pass() {
    let (lib, music, mounted, old, track) = moved();
    let mate = mate_of(&lib, music, old, "Kappa");
    // What a deciding run leaves until the pass after it: the rules' new
    // answer, and where the pair stood before. A relink can run in between.
    lib.pair(old, mate, "review", "merged");
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_track_mate_never_decided_against_the_gone_file_isnt_matched() {
    let (lib, music, mounted, old, track) = moved();
    // The gone file and a second file were merged. A third joined the
    // track on its pair with the second while the first was away: nobody
    // compared it with the gone file.
    let second = elsewhere(&lib, music, "Second");
    lib.hashed(second, 2);
    lib.merged(&[old, second]);
    let third = encode(&lib, music, "Third");
    lib.hashed(third, 3);
    lib.title(third, "Kappa");
    lib.joins(third, second);
    // The second file is held by another row, so only the third is free.
    let holder = lib.track(&loc("E:/Music/Moved Second/Second.flac"), None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(holder), path(second));
    assert_eq!(lib.recording_of(third), lib.recording_of(old));
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_track_mate_in_another_track_by_now_isnt_matched() {
    let (lib, music, mounted, old, track) = moved();
    let mate = mate_of(&lib, music, old, "Kappa");
    // Taken out of the track; the pair is as it was stored.
    lib.split(mate);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn of_several_track_mates_the_tracks_best_file_is_matched() {
    let (lib, music, mounted, old, track) = moved();
    let first = encode(&lib, music, "First");
    let best = encode(&lib, music, "Best");
    for (file, hash) in [(first, 2), (best, 3)] {
        lib.hashed(file, hash);
        lib.title(file, "Kappa");
    }
    lib.merged(&[best, old, first]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_track(best));
}

#[test]
fn of_several_track_mates_none_of_them_the_best_file_the_lowest_file_id_is_matched() {
    let (lib, music, mounted, old, track) = moved();
    let first = encode(&lib, music, "First");
    let second = encode(&lib, music, "Second");
    for (file, hash) in [(first, 2), (second, 3)] {
        lib.hashed(file, hash);
        lib.title(file, "Kappa");
    }
    // The gone file is still the track's best file.
    lib.merged(&[old, second, first]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_track(first));
}

#[test]
fn a_track_mate_another_row_holds_isnt_a_candidate() {
    let (lib, music, mounted, old, track) = moved();
    let held = lib.file(music, "Held.mp3", Some(250_000));
    lib.hashed(held, 2);
    lib.title(held, "Kappa");
    lib.merged(&[old, held]);
    let holder = lib.track(&loc("E:/Music/Held.mp3"), Some("250"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(holder), path(held));
    assert_eq!(lib.matched(track), None);
    // A second mate nobody holds is matched.
    let free = encode(&lib, music, "Free");
    lib.hashed(free, 3);
    lib.title(free, "Kappa");
    lib.joins(free, old);
    lib.relink(&mounted);
    assert_eq!(lib.matched(holder), path(held));
    assert_eq!(lib.matched(track), same_track(free));
}

#[test]
fn two_rows_dont_share_the_one_track_mate_left() {
    let (lib, music, mounted, old, first) = moved();
    let twin = lib.file(music, "Twin.mp3", Some(200_000));
    lib.hashed(twin, 3);
    let second = lib.track_with(&loc("E:/Music/Twin.mp3"), Some("200"), &[("Name", "Kappa")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(second), path(twin));
    // Two rekordbox entries, each for a file of one track; both files go,
    // and one mate is left.
    let mate = encode(&lib, music, "Mate");
    lib.hashed(mate, 2);
    lib.title(mate, "Kappa");
    lib.merged(&[old, twin, mate]);
    lib.gone(twin);
    lib.relink(&mounted);
    assert_eq!(lib.matched(first), None);
    assert_eq!(lib.matched(second), None);
}

#[test]
fn the_same_audio_is_matched_before_a_track_mate() {
    let (lib, music, mounted, old, track) = moved();
    mate_of(&lib, music, old, "Kappa");
    let copy = elsewhere(&lib, music, "Copy");
    lib.hashed(copy, 1);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(copy));
}

#[test]
fn a_track_mate_is_matched_before_any_fingerprint_is_compared() {
    let (lib, music, mounted, old, track) = moved();
    let mate = mate_of(&lib, music, old, "Kappa");
    // Two more files of the gone file's length, both fingerprint
    // duplicates of it in tracks of their own: ambiguous by themselves.
    for name in ["A.flac", "B.m4a"] {
        let file = lib.file(music, &format!("Converted/{name}"), Some(200_400));
        lib.fingerprinted(file, &reencoded(&song(1, 200)));
        lib.title(file, "Kappa");
    }
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_track(mate));
}

#[test]
fn a_track_mate_on_a_drive_that_isnt_plugged_in_is_matched_as_the_same_audio_there_is() {
    for same_audio_there in [true, false] {
        let lib = Lib::new();
        let (e, d) = (serial(1), serial(2));
        let plugged = lib.volume(&e, Some(r"E:\"));
        let unplugged = lib.volume(&d, Some(r"D:\"));
        let music = lib.folder(plugged, "Music");
        let away = lib.folder(unplugged, "Music");
        let mounted = Mounted::new([(e, r"E:\")]);
        let old = lib.file(music, "Old.mp3", Some(200_000));
        lib.hashed(old, 1);
        let track = lib.track_with(&loc("E:/Music/Old.mp3"), Some("200"), &[("Name", "Kappa")]);
        lib.relink(&mounted);
        lib.gone(old);
        // Still present: nothing walked the drive to find it gone.
        let there = lib.file(away, "Elsewhere/There.flac", Some(200_400));
        lib.title(there, "Kappa");
        if same_audio_there {
            lib.hashed(there, 1);
            lib.relink(&mounted);
            assert_eq!(lib.matched(track), same_audio(there));
        } else {
            lib.hashed(there, 2);
            lib.merged(&[old, there]);
            lib.relink(&mounted);
            assert_eq!(lib.matched(track), same_track(there));
        }
    }
}

#[test]
fn a_track_mate_overwritten_with_other_audio_isnt_matched_before_grouping_catches_up() {
    let (lib, music, mounted, old, track) = moved();
    let mate = encode(&lib, music, "Mate");
    lib.hashed(mate, 1);
    lib.title(mate, "Kappa");
    // Merged with the gone file, and grouping current for both.
    lib.merged(&[old, mate]);
    // The mate is overwritten and hashed again (current), and grouping
    // hasn't run since: it's still in the gone file's track.
    lib.edited(mate);
    lib.hashed(mate, 9);
    assert_eq!(lib.recording_of(mate), lib.recording_of(old));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // Nor after grouping moved it to a track of its own.
    lib.writer.call(crate::grouping::regroup).unwrap();
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    assert_eq!(lib.recording(track), None);
}

#[test]
fn a_gone_file_that_changed_before_it_went_isnt_evidence_by_its_old_fingerprint() {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.fingerprinted(old, &song(1, 200));
    // Replaced by other audio, then deleted before the fingerprint stage
    // ran again: the stored fingerprint is of the older content.
    lib.edited(old);
    let track = lib.track_with(&loc("E:/Music/Old.mp3"), Some("200"), &[("Name", "Kappa")]);
    lib.gone(old);
    let new = lib.file(music, "Converted/New.flac", Some(200_400));
    lib.fingerprinted(new, &reencoded(&song(1, 200)));
    lib.title(new, "Kappa");
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn rows_over_the_comparison_budget_are_counted_in_the_summary() {
    let (lib, music, mounted, _old, track) = moved();
    // More files of the gone file's length than one run compares; each was
    // looked at by the fingerprint stage (so none is waiting).
    let count = MAX_COMPARISONS as i64 + 1;
    let version = i64::from(crate::fingerprint::VERSION);
    lib.writer
        .call(move |c| {
            c.execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?2)
                 INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, duration_ms)
                 SELECT ?1, 'Many/F' || i || '.flac', 'Many/F' || i || '.flac', 1000, 200000
                 FROM n",
                params![music, count],
            )?;
            c.execute(
                "INSERT INTO file_stage (file_id, stage, version, size, mtime, status, reason)
                 SELECT id, 'fingerprint', ?1, size, mtime, 'failed', 'unsupported_codec'
                 FROM file WHERE rel_path LIKE 'Many/%'",
                [version],
            )
        })
        .unwrap();
    let summary = lib.relink(&mounted);
    assert_eq!(summary.fingerprint_undecided, 1);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn of_several_copies_of_the_audio_the_tracks_best_file_is_matched() {
    let (lib, music, mounted, _old, track) = moved();
    let first = elsewhere(&lib, music, "First");
    let best = elsewhere(&lib, music, "Best");
    lib.hashed(first, 1);
    lib.hashed(best, 1);
    lib.group(&[best, first]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(best));
}

#[test]
fn a_file_whose_hash_isnt_current_isnt_taken_for_the_gone_files_audio() {
    let (lib, music, mounted, _old, track) = moved();
    let new = elsewhere(&lib, music, "New");
    lib.hashed(new, 1);
    // It changed on disk since it was hashed: the hash says nothing now.
    lib.edited(new);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // Hashed again, same audio.
    lib.hashed(new, 1);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(new));
}

#[test]
fn a_track_mate_that_changed_since_it_was_hashed_isnt_trusted() {
    let (lib, music, mounted, old, track) = moved();
    let mate = encode(&lib, music, "Mate");
    lib.hashed(mate, 1);
    lib.title(mate, "Kappa");
    // Merged with the gone file, and grouping current for both.
    lib.merged(&[old, mate]);
    // The mate changed on disk and hasn't been hashed again: it may hold
    // anything. Grouping can't see that: to it, nothing changed.
    lib.edited(mate);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    assert_eq!(lib.evidence(track), None);
}

#[test]
fn a_track_mate_of_another_audio_hash_that_changed_since_it_was_hashed_isnt_matched() {
    let (lib, music, mounted, old, track) = moved();
    let mate = mate_of(&lib, music, old, "Kappa");
    lib.edited(mate);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // Hashed again, the same audio as before: nothing changed for
    // grouping, and it's matched.
    lib.hashed(mate, 2);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_track(mate));
}

#[test]
fn a_gone_file_that_changed_before_it_went_isnt_evidence_by_its_old_hash() {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    // Changed on disk, then deleted before it was hashed again.
    lib.edited(old);
    let track = lib.track(&loc("E:/Music/Old.mp3"), Some("200"));
    lib.gone(old);
    let copy = elsewhere(&lib, music, "Copy");
    lib.hashed(copy, 1);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_copy_another_row_holds_isnt_a_candidate() {
    let (lib, music, mounted, _old, track) = moved();
    let copy = lib.file(music, "Copy.mp3", Some(200_000));
    lib.hashed(copy, 1);
    let holder = lib.track(&loc("E:/Music/Copy.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(holder), path(copy));
    assert_eq!(lib.matched(track), None);
}

#[test]
fn two_rows_whose_gone_files_had_the_same_audio_dont_share_the_one_file_left() {
    let (lib, music, mounted, _old, first) = moved();
    let twin = lib.file(music, "Twin.mp3", Some(200_000));
    lib.hashed(twin, 1);
    let second = lib.track(&loc("E:/Music/Twin.mp3"), Some("200"));
    lib.relink(&mounted);
    lib.gone(twin);
    let left = elsewhere(&lib, music, "Left");
    lib.hashed(left, 1);
    lib.relink(&mounted);
    assert_eq!(lib.matched(first), None);
    assert_eq!(lib.matched(second), None);
}

#[test]
fn a_reencoded_file_is_matched_by_fingerprint_when_a_title_tag_agrees() {
    let (lib, music, mounted, old, track) = moved();
    let new = lib.file(music, "Converted/New.flac", Some(200_400));
    lib.hashed(new, 2);
    lib.fingerprinted(new, &reencoded(&song(1, 200)));
    lib.title(new, "Kappa");
    let summary = lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((new, "fingerprint".to_owned(), 0.85))
    );
    assert!(!lib.probable(track));
    assert_eq!(summary.fingerprint, 1);
    let _ = old;
}

#[test]
fn a_fingerprint_duplicate_no_title_tag_agrees_with_is_only_probable() {
    for title in [None, Some("Kappa (Clean)")] {
        let (lib, music, mounted, _old, track) = moved();
        let new = lib.file(music, "Converted/New.flac", Some(200_400));
        lib.fingerprinted(new, &reencoded(&song(1, 200)));
        if let Some(title) = title {
            lib.title(new, title);
        }
        let summary = lib.relink(&mounted);
        assert_eq!(
            lib.matched(track),
            Some((new, "fingerprint".to_owned(), 0.4)),
            "{title:?}"
        );
        assert!(lib.probable(track));
        assert_eq!((summary.fingerprint, summary.probable), (0, 1));
    }
}

#[test]
fn other_audio_of_the_same_length_is_no_match() {
    let (lib, music, mounted, _old, track) = moved();
    let other = lib.file(music, "Converted/Other.flac", Some(200_000));
    lib.fingerprinted(other, &song(2, 200));
    lib.title(other, "Kappa");
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_fingerprint_is_compared_only_with_files_of_about_the_same_length() {
    let (lib, music, mounted, _old, track) = moved();
    // The same fingerprint, but a duration a re-encode never gives.
    let far = lib.file(music, "Converted/Far.flac", Some(203_000));
    lib.fingerprinted(far, &reencoded(&song(1, 200)));
    lib.title(far, "Kappa");
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_changed_files_old_fingerprint_isnt_compared() {
    let (lib, music, mounted, _old, track) = moved();
    let new = lib.file(music, "Converted/New.flac", Some(200_400));
    lib.fingerprinted(new, &reencoded(&song(1, 200)));
    lib.title(new, "Kappa");
    // Changed on disk since: the fingerprint isn't this file's any more.
    lib.edited(new);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn two_different_fingerprint_duplicates_are_ambiguous() {
    let (lib, music, mounted, _old, track) = moved();
    let a = lib.file(music, "Converted/A.flac", Some(200_400));
    let b = lib.file(music, "Converted/B.m4a", Some(199_800));
    for (file, hash) in [(a, 2), (b, 3)] {
        lib.hashed(file, hash);
        lib.fingerprinted(file, &reencoded(&song(1, 200)));
        lib.title(file, "Kappa");
    }
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // In one track they're one choice: its best file.
    lib.group(&[b, a]);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((b, "fingerprint".to_owned(), 0.85))
    );
}

#[test]
fn a_fingerprint_duplicate_another_row_holds_still_counts_against_uniqueness() {
    let (lib, music, mounted, _old, track) = moved();
    let a = lib.file(music, "Converted/A.flac", Some(200_400));
    let b = lib.file(music, "Converted/B.m4a", Some(199_800));
    for (file, hash) in [(a, 2), (b, 3)] {
        lib.hashed(file, hash);
        lib.fingerprinted(file, &reencoded(&song(1, 200)));
        lib.title(file, "Kappa");
    }
    // Another row has one of the two duplicates by path. The other one
    // isn't the only duplicate for that: still ambiguous.
    let holder = lib.track(&loc("E:/Music/Converted/A.flac"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(holder), path(a));
    assert_eq!(lib.matched(track), None);
}

#[test]
fn the_only_fingerprint_duplicate_isnt_taken_when_another_row_holds_it() {
    let (lib, music, mounted, _old, track) = moved();
    let new = lib.file(music, "Converted/New.flac", Some(200_400));
    lib.fingerprinted(new, &reencoded(&song(1, 200)));
    lib.title(new, "Kappa");
    let holder = lib.track(&loc("E:/Music/Converted/New.flac"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(holder), path(new));
    assert_eq!(lib.matched(track), None);
}

#[test]
fn the_duration_window_is_one_second_either_way() {
    for (lasts, matched) in [
        (201_000, true),
        (199_000, true),
        (201_001, false),
        (198_999, false),
    ] {
        let (lib, music, mounted, _old, track) = moved();
        let new = lib.file(music, "Converted/New.flac", Some(lasts));
        lib.fingerprinted(new, &reencoded(&song(1, 200)));
        lib.title(new, "Kappa");
        lib.relink(&mounted);
        assert_eq!(lib.matched(track).is_some(), matched, "{lasts} ms");
    }
}

#[test]
fn a_file_of_about_that_length_still_waiting_for_its_fingerprint_holds_the_match_back() {
    let (lib, music, mounted, _old, track) = moved();
    let new = lib.file(music, "Converted/New.flac", Some(200_400));
    lib.fingerprinted(new, &reencoded(&song(1, 200)));
    lib.title(new, "Kappa");
    // Not fingerprinted yet: it could be a duplicate too.
    let waiting = lib.file(music, "Converted/Waiting.flac", Some(200_100));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None, "never on a partial comparison");
    // It's other audio: the duplicate is the only one.
    lib.fingerprinted(waiting, &song(2, 200));
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((new, "fingerprint".to_owned(), 0.85))
    );
}

#[test]
fn a_file_that_cant_be_fingerprinted_doesnt_hold_the_match_back() {
    let (lib, music, mounted, _old, track) = moved();
    let new = lib.file(music, "Converted/New.flac", Some(200_400));
    lib.fingerprinted(new, &reencoded(&song(1, 200)));
    lib.title(new, "Kappa");
    let broken = lib.file(music, "Converted/Broken.opus", Some(200_100));
    lib.unfingerprintable(broken);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((new, "fingerprint".to_owned(), 0.85))
    );
}

#[test]
fn the_gone_file_of_a_confirmed_relink_is_evidence_too() {
    let (lib, music, mounted) = e_music();
    let picked = lib.file(music, "Elsewhere/Picked.mp3", Some(100_000));
    lib.hashed(picked, 4);
    let location = loc("E:/Gone/Lambda.mp3");
    let track = lib.track(&location, Some("200"));
    lib.confirm_now(&location, picked, Method::User);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), Some((picked, "user".to_owned(), 1.0)));
    lib.gone(picked);
    let new = elsewhere(&lib, music, "New");
    lib.hashed(new, 4);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(new));
}

#[test]
fn a_file_on_a_drive_that_isnt_plugged_in_is_matched_by_path_not_by_audio() {
    let lib = Lib::new();
    let (e, d) = (serial(1), serial(2));
    let unplugged = lib.volume(&e, Some(r"E:\"));
    let plugged = lib.volume(&d, Some(r"D:\"));
    let music = lib.folder(unplugged, "Music");
    let other = lib.folder(plugged, "Music");
    // Still present: nothing walked the drive to find it gone.
    let file = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(file, 1);
    let copy = lib.file(other, "Elsewhere/Copy.flac", None);
    lib.hashed(copy, 1);
    let track = lib.track(&loc("E:/Music/Old.mp3"), Some("200"));
    lib.relink(&Mounted::new([(d, r"D:\")]));
    assert_eq!(lib.matched(track), Some((file, "path".to_owned(), 0.9)));
}

#[test]
fn an_audio_match_replaces_a_name_guess() {
    let (lib, music, mounted, _old, track) = moved();
    let guess = lib.file(music, "Other Folder/Old.mp3", None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), name_only(guess));
    let new = elsewhere(&lib, music, "New");
    lib.hashed(new, 1);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(new));
    assert!(!lib.probable(track));
}

#[test]
fn fingerprints_are_read_only_when_no_present_file_has_the_gone_files_audio() {
    let (lib, music, mounted, old, _track) = moved();
    let new = lib.file(music, "Converted/New.flac", Some(200_400));
    lib.fingerprinted(new, &reencoded(&song(1, 200)));
    let asked = |lib: &Lib| {
        let mounted = mounted.clone();
        lib.writer
            .call(move |c| {
                let tx = c.transaction()?;
                let input = super::super::load(&tx, &mounted)?;
                let mut asked = Vec::new();
                rules::plan(
                    &input,
                    |_| Ok::<_, ()>(Vec::new()),
                    |file| {
                        asked.push(file);
                        Ok(None)
                    },
                    no_mates,
                )
                .unwrap();
                Ok(asked)
            })
            .unwrap()
    };
    // No file of the same audio: the gone file's fingerprint is asked for
    // (and, had it had one, that of the file of about its length).
    assert_eq!(asked(&lib), [old]);
    let same = elsewhere(&lib, music, "Same");
    lib.hashed(same, 1);
    assert_eq!(asked(&lib), [0i64; 0]);
}

#[test]
fn a_run_compares_no_more_fingerprints_than_its_budget() {
    // Two gone files, each with more than half the budget of present
    // files of its length: the first is compared, the second isn't.
    let per_row = MAX_COMPARISONS / 2 + 1;
    let mut input = Input::default();
    let mut id = 0;
    for (row, duration) in [(0i64, 100_000i64), (1, 300_000)] {
        input.absent.push(rules::Absent {
            id: 1_000_000 + row,
            path: Some(format!("E:/Music/Gone {row}.mp3")),
            duration_ms: Some(duration),
            ..rules::Absent::default()
        });
        for _ in 0..per_row {
            id += 1;
            input.files.push(rules::File {
                id,
                folder: 1,
                parent: "Elsewhere".into(),
                name: format!("F{id}.flac"),
                path: Some(format!("E:/Music/Elsewhere/F{id}.flac")),
                online: true,
                duration_ms: Some(duration),
                ..rules::File::default()
            });
        }
        let location = loc(&format!("E:/Music/Gone {row}.mp3"));
        let location = crate::rekordbox::location::decode(&location).unwrap();
        input.tracks.push(rules::Track {
            id: row + 1,
            key: location.match_key(),
            location: Some(location),
            ..rules::Track::default()
        });
    }
    let mut asked = 0u64;
    let plan = rules::plan(
        &input,
        |_| Ok::<_, ()>(Vec::new()),
        |_| {
            asked += 1;
            Ok(Some(song(asked, 5)))
        },
        no_mates,
    )
    .unwrap();
    assert_eq!(plan.compared, per_row);
    assert_eq!(plan.over_budget, 1);
    assert_eq!(
        asked as usize,
        per_row + 2,
        "both gone files, and the first one's files"
    );
    assert_eq!(plan.missing, 2);
}

/// `count` present files lasting `duration` ms, ids from `first_id` up,
/// and a rekordbox row `row` whose gone file (id `1_000_000 + row`) lasted
/// that long.
fn gone_row_and_files(input: &mut Input, row: i64, duration: i64, first_id: i64, count: usize) {
    input.absent.push(rules::Absent {
        id: 1_000_000 + row,
        path: Some(format!("E:/Music/Gone {row}.mp3")),
        duration_ms: Some(duration),
        ..rules::Absent::default()
    });
    for id in first_id..first_id + count as i64 {
        input.files.push(rules::File {
            id,
            folder: 1,
            parent: "Elsewhere".into(),
            name: format!("F{id}.flac"),
            path: Some(format!("E:/Music/Elsewhere/F{id}.flac")),
            online: true,
            duration_ms: Some(duration),
            ..rules::File::default()
        });
    }
    let location = loc(&format!("E:/Music/Gone {row}.mp3"));
    let location = crate::rekordbox::location::decode(&location).unwrap();
    input.tracks.push(rules::Track {
        id: row,
        key: location.match_key(),
        location: Some(location),
        ..rules::Track::default()
    });
}

/// A fingerprint lookup where gone file `1_000_000 + n` and present file
/// `n` are song `n`, and every other file is a song of its own.
fn prints(file: i64) -> Result<Option<crate::fingerprint::Fingerprint>, ()> {
    Ok(Some(song((file % 1_000_000) as u64, 5)))
}

#[test]
fn the_budget_goes_to_the_rows_with_the_fewest_candidates_first() {
    // Row 1 has the most files of its length, row 2 fewer; together
    // they're over the budget. In row order, row 1 would use the budget up
    // and row 2 would never be compared, in any run.
    let mut input = Input::default();
    let (many, few) = (MAX_COMPARISONS * 3 / 4, MAX_COMPARISONS * 3 / 10);
    gone_row_and_files(&mut input, 1, 100_000, 10, many);
    // File 2 is row 2's gone file's song.
    gone_row_and_files(&mut input, 2, 300_000, 2, 1);
    gone_row_and_files(&mut input, 3, 300_000, 500_000, few - 1);
    // (Row 3 only brings files of row 2's length; give it no gone file.)
    input.absent.pop();
    let plan = rules::plan(&input, |_| Ok::<_, ()>(Vec::new()), prints, no_mates).unwrap();
    assert_eq!(plan.compared, few);
    assert_eq!(plan.over_budget, 1, "row 1 is left undecided");
    // Row 2 found its duplicate (probable: no title to agree).
    assert_eq!(plan.probable, 1);
    let matched: Vec<_> = plan.decisions.iter().map(|d| (d.track, d.target)).collect();
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].0, 2);
    assert_eq!(matched[0].1.map(|t| t.file), Some(2));
}

#[test]
fn a_row_the_budget_didnt_reach_still_contests_its_candidates() {
    // Two gone files of one song, and one duplicate of it left. Compared
    // in full, neither row gets it. The budget reaches only the first row:
    // it still mustn't get the file.
    let per_row = MAX_COMPARISONS / 2 + 1;
    let mut input = Input::default();
    gone_row_and_files(&mut input, 1, 100_000, 1, per_row);
    // Row 2's gone file is the same song (file 1's), the same length.
    input.absent.push(rules::Absent {
        id: 2_000_001,
        path: Some("E:/Music/Gone 2.mp3".into()),
        duration_ms: Some(100_000),
        ..rules::Absent::default()
    });
    let location = crate::rekordbox::location::decode(&loc("E:/Music/Gone 2.mp3")).unwrap();
    input.tracks.push(rules::Track {
        id: 2,
        key: location.match_key(),
        location: Some(location),
        ..rules::Track::default()
    });
    let plan = rules::plan(&input, |_| Ok::<_, ()>(Vec::new()), prints, no_mates).unwrap();
    assert_eq!((plan.compared, plan.over_budget), (per_row, 1));
    assert_eq!(plan.missing, 2);
    assert!(plan.decisions.is_empty());
}
