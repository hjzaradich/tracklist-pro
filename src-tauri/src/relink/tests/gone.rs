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

#[test]
fn another_present_file_of_the_gone_files_track_is_matched() {
    let (lib, music, mounted, old, track) = moved();
    // Grouped with the gone file, though its audio_hash differs (as 1b's
    // fingerprint grouping will do).
    let mate = elsewhere(&lib, music, "Mate");
    lib.hashed(mate, 2);
    let recording = lib.group(&[old, mate]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), same_audio(mate));
    assert_eq!(lib.recording(track), Some(recording));
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
    let mate = elsewhere(&lib, music, "Mate");
    lib.hashed(mate, 1);
    lib.group(&[old, mate]);
    // The mate changed on disk and hasn't been hashed again, so grouping
    // hasn't moved it yet: it may hold anything.
    lib.edited(mate);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    // It turns out to hold other audio, and grouping moves it to a track
    // of its own: still no match, and nothing recorded as the audio a
    // match was made to.
    lib.hashed(mate, 9);
    lib.writer.call(crate::grouping::regroup).unwrap();
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    assert_eq!(lib.evidence(track), None);
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
    let plan = rules::plan(&input, |_| Ok::<_, ()>(Vec::new()), prints).unwrap();
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
    let plan = rules::plan(&input, |_| Ok::<_, ()>(Vec::new()), prints).unwrap();
    assert_eq!((plan.compared, plan.over_budget), (per_row, 1));
    assert_eq!(plan.missing, 2);
    assert!(plan.decisions.is_empty());
}
