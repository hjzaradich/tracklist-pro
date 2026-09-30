//! Every run decides every row again (review of 1aC-3), competing tracks,
//! exact copies, and the track (`recording_id`) that follows the file.

use super::super::rules::{self, Input, MAX_CANDIDATES};
use super::*;

// --- Earlier matches are re-checked ---

#[test]
fn a_duration_guess_gives_way_when_the_locations_own_file_turns_up() {
    let (lib, music, mounted) = e_music();
    let other = lib.file(music, "Album/Other.mp3", Some(200_300));
    let track = lib.track(&loc("E:/Music/Album/Original.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.method(track).as_deref(), Some("unique_duration"));
    // The scan finds the file at the exact path.
    let original = lib.file(music, "Album/Original.mp3", Some(200_000));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(original));
    assert!(!lib.probable(track));
    let _ = other;
}

#[test]
fn a_match_to_a_file_that_is_gone_is_dropped_and_the_moved_copy_found() {
    let (lib, music, mounted) = e_music();
    let was = lib.file(music, "Beta.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Beta.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(was));
    lib.gone(was);
    let moved = lib.file(music, "Moved/Beta.mp3", Some(200_000));
    let summary = lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((moved, "filename_duration".to_owned(), 0.9))
    );
    assert_eq!(summary.changed, 1);
}

#[test]
fn a_match_to_a_file_that_is_gone_with_no_replacement_is_cleared() {
    let (lib, music, mounted) = e_music();
    let was = lib.file(music, "Gamma.mp3", Some(200_000));
    lib.group(&[was]);
    let track = lib.track(&loc("E:/Music/Gamma.mp3"), Some("200"));
    lib.relink(&mounted);
    assert!(lib.recording(track).is_some());
    lib.gone(was);
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    assert_eq!(lib.recording(track), None);
    assert!(!lib.probable(track));
    assert_eq!((summary.missing, summary.changed), (1, 1));
}

#[test]
fn a_guess_never_keeps_a_file_the_user_confirmed_for_another_track() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Album/Delta.mp3", Some(200_300));
    let guessed = lib.track(&loc("E:/Music/Album/Gone.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(guessed).map(|m| m.0), Some(file));
    let owner = lib.track(&loc("E:/Elsewhere/Delta Original.mp3"), Some("200"));
    lib.confirm(&loc("E:/Elsewhere/Delta Original.mp3"), file, "user");
    lib.relink(&mounted);
    assert_eq!(lib.matched(owner), Some((file, "user".to_owned(), 1.0)));
    assert_eq!(lib.matched(guessed), None);
}

#[test]
fn confirming_the_current_match_updates_its_method_and_confidence() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Album/Renamed.mp3", Some(200_300));
    lib.title(file, "Epsilon");
    let track = lib.track_with(
        &loc("E:/Music/Album/Original.mp3"),
        Some("200"),
        &[("Name", "Epsilon")],
    );
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((file, "unique_duration".to_owned(), 0.8))
    );
    lib.confirm(&loc("E:/Music/Album/Original.mp3"), file, "user");
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), Some((file, "user".to_owned(), 1.0)));
    assert_eq!((summary.confirmed, summary.changed), (1, 1));
    // …and the same confirmation of an identical match changes nothing.
    assert_eq!(lib.relink(&mounted).changed, 0);
}

// --- Step 3 sees every competing track ---

#[test]
fn a_track_whose_candidate_set_is_over_the_cap_still_competes_for_a_file() {
    for with_competitor in [false, true] {
        let (lib, music, mounted) = e_music();
        let fits = lib.file(music, "Album/Zeta Renamed.mp3", Some(200_300));
        lib.file(music, "Album/N1.mp3", Some(100_000));
        lib.file(music, "Big/N2.mp3", Some(110_000));
        for n in 0..MAX_CANDIDATES + 10 {
            lib.file(
                music,
                &format!("Big/Other {n}.mp3"),
                Some(600_000 + n as i64 * 3_000),
            );
        }
        let track = lib.track(&loc("E:/Music/Album/Zeta.mp3"), Some("200"));
        // Neighbours in D:/Old moved into Album and into Big.
        lib.track(&loc("D:/Old/N1.mp3"), Some("100"));
        lib.track(&loc("D:/Old/N2.mp3"), Some("110"));
        let competitor = with_competitor.then(|| lib.track(&loc("D:/Old/B.mp3"), Some("200")));
        lib.relink(&mounted);
        if let Some(competitor) = competitor {
            assert_eq!(lib.matched(track), None);
            assert_eq!(lib.matched(competitor), None);
        } else {
            assert_eq!(lib.matched(track).map(|m| m.0), Some(fits));
        }
    }
}

#[test]
fn a_track_that_fits_a_file_by_name_competes_with_a_duration_match() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Album/Song.mp3", Some(200_300));
    lib.file(music, "Backup/Song.mp3", Some(200_400));
    // A's file exists twice (by name: ambiguous, so A stays open)…
    let a = lib.track(&loc("E:/Singles/Song.mp3"), Some("200"));
    // …and C's own file in Album is gone; Album's Song.mp3 fits it too.
    let c = lib.track(&loc("E:/Music/Album/Gone.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(a), None);
    assert_eq!(lib.matched(c), None);
}

#[test]
fn a_file_taken_by_name_in_step_2_isnt_taken_by_duration_in_step_3() {
    let (lib, music, mounted) = e_music();
    let song = lib.file(music, "Album/Song.mp3", Some(200_300));
    let by_name = lib.track(&loc("D:/Old/Song.mp3"), Some("200"));
    let by_duration = lib.track(&loc("E:/Music/Album/Gone.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(by_name),
        Some((song, "filename_duration".to_owned(), 0.9))
    );
    assert_eq!(lib.matched(by_duration), None);
}

#[test]
fn a_huge_candidate_set_is_skipped_before_any_of_its_files_is_looked_at() {
    // 2,000 folders of 10 files; 2,000 tracks from one rekordbox folder
    // matched by name into them, and 2,000 more whose files are gone: their
    // shared candidate set holds 20,000 files.
    let folders = 2_000;
    let mut input = Input::default();
    let mut id = 0;
    for f in 0..folders {
        for k in 0..10 {
            id += 1;
            let name = format!("N{f}_{k}.mp3");
            input.files.push(rules::File {
                id,
                folder: 1,
                parent: format!("F{f}"),
                path: Some(format!("E:/Music/F{f}/{name}")),
                name,
                online: true,
                duration_ms: Some(100_000 + k * 20_000),
                ..rules::File::default()
            });
        }
    }
    let track = |id: i64, path: String, total_s: u32| {
        let raw = format!("file://localhost/{path}");
        let location = location::decode(&raw).unwrap();
        rules::Track {
            id,
            key: location.match_key(),
            location: Some(location),
            total_s: Some(total_s),
            ..rules::Track::default()
        }
    };
    for f in 0..folders {
        input
            .tracks
            .push(track(f + 1, format!("D:/Old/N{f}_0.mp3"), 100));
    }
    for g in 0..folders {
        input
            .tracks
            .push(track(folders + g + 1, format!("D:/Old/Gone {g}.mp3"), 150));
    }
    let plan = rules::plan(&input, |_| Ok::<_, ()>(Vec::new())).unwrap();
    assert_eq!(plan.filename_duration, 2_000);
    assert_eq!(plan.missing, 2_000);
    assert_eq!(plan.examined, 0);
}

#[test]
fn a_small_candidate_set_is_looked_at_once_for_all_its_tracks() {
    let (lib, music, mounted) = e_music();
    for n in 0..5 {
        lib.file(
            music,
            &format!("Album/F{n}.mp3"),
            Some(100_000 + n * 50_000),
        );
    }
    for n in 0..20 {
        lib.track(&loc(&format!("E:/Music/Album/Gone {n}.mp3")), Some("999"));
    }
    let mounted2 = mounted.clone();
    let examined = lib
        .writer
        .call(move |c| {
            let tx = c.transaction()?;
            let input = super::super::load(&tx, &mounted2)?;
            Ok(rules::plan(&input, |_| Ok::<_, ()>(Vec::new()))
                .unwrap()
                .examined)
        })
        .unwrap();
    assert_eq!(examined, 5);
    let _ = mounted;
}

// --- Exact copies ---

#[test]
fn exact_copies_that_fit_by_name_match_the_tracks_best_file() {
    let (lib, music, mounted) = e_music();
    let a = lib.file(music, "Copy A/Eta.mp3", Some(200_300));
    let b = lib.file(music, "Copy B/Eta.mp3", Some(200_300));
    lib.audio_hash(a, 7);
    lib.audio_hash(b, 7);
    let recording = lib.group(&[b, a]);
    let track = lib.track(&loc("E:/Gone/Eta.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((b, "filename_duration".to_owned(), 0.9))
    );
    assert_eq!(lib.recording(track), Some(recording));
}

#[test]
fn exact_copies_not_yet_grouped_match_the_lowest_file_id() {
    let (lib, music, mounted) = e_music();
    let a = lib.file(music, "Copy A/Theta.mp3", Some(200_300));
    let b = lib.file(music, "Copy B/Theta.mp3", Some(200_300));
    lib.audio_hash(a, 7);
    lib.audio_hash(b, 7);
    let track = lib.track(&loc("E:/Gone/Theta.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track).map(|m| m.0), Some(a.min(b)));
    assert_eq!(lib.recording(track), None);
}

#[test]
fn copies_with_different_or_unknown_audio_stay_ambiguous() {
    for (ha, hb) in [(Some(7), Some(8)), (Some(7), None), (None, None)] {
        let (lib, music, mounted) = e_music();
        let a = lib.file(music, "Copy A/Iota.mp3", Some(200_300));
        let b = lib.file(music, "Copy B/Iota.mp3", Some(200_300));
        if let Some(h) = ha {
            lib.audio_hash(a, h);
        }
        if let Some(h) = hb {
            lib.audio_hash(b, h);
        }
        let track = lib.track(&loc("E:/Gone/Iota.mp3"), Some("200"));
        lib.relink(&mounted);
        assert_eq!(lib.matched(track), None, "{ha:?} {hb:?}");
    }
}

#[test]
fn exact_copies_in_the_candidate_set_match_by_unique_duration() {
    let (lib, music, mounted) = e_music();
    let a = lib.file(music, "Album/Kappa (1).mp3", Some(200_300));
    let b = lib.file(music, "Album/Kappa (2).mp3", Some(200_300));
    lib.audio_hash(a, 9);
    lib.audio_hash(b, 9);
    lib.group(&[b, a]);
    let track = lib.track(&loc("E:/Music/Album/Kappa.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((b, "unique_duration".to_owned(), 0.4))
    );
}

// --- The track follows the file ---

#[test]
fn the_track_follows_the_matched_file() {
    let (lib, music, mounted) = e_music();
    let first = lib.file(music, "Lambda.mp3", Some(200_000));
    let picked = lib.file(music, "Elsewhere/Lambda (Real).mp3", Some(200_000));
    let first_track = lib.group(&[first]);
    let picked_track = lib.group(&[picked]);
    let track = lib.track(&loc("E:/Music/Lambda.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.recording(track), Some(first_track));
    // The user picks the other file: the track follows it.
    lib.confirm(&loc("E:/Music/Lambda.mp3"), picked, "user");
    lib.relink(&mounted);
    assert_eq!(lib.recording(track), Some(picked_track));
}

#[test]
fn a_file_not_grouped_yet_leaves_the_track_empty_until_it_is() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Mu.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Mu.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(file));
    assert_eq!(lib.recording(track), None);
    let grouped = lib.group(&[file]);
    let summary = lib.relink(&mounted);
    assert_eq!(lib.recording(track), Some(grouped));
    assert_eq!(summary.changed, 1);
}

// --- Values of an unexpected type ---

#[test]
fn a_numeric_total_time_is_read_like_the_text_one() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "New/Nu.mp3", Some(200_300));
    let track = lib.track(&loc("E:/Old/Nu.mp3"), Some("200"));
    lib.writer
        .call(move |c| {
            c.execute("DROP TRIGGER rekordbox_track_values_are_read_only", [])?;
            c.execute(
                "UPDATE rekordbox_track SET attributes = json_set(attributes, '$.TotalTime', 200)
                 WHERE id = ?1",
                [track],
            )
        })
        .unwrap();
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((file, "filename_duration".to_owned(), 0.9))
    );
}

// --- Carried matches (from steps this run doesn't make) ---

/// Marks a row's stored match probable, as step 5 (1aD-2) will.
fn make_probable(lib: &Lib, track: i64) {
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE rekordbox_track SET relink_probable = 1 WHERE id = ?1",
                [track],
            )
        })
        .unwrap();
}

/// The user withdrew a confirmation: its `relink` row is deleted.
fn withdraw(lib: &Lib, location: &str) {
    let key = location::decode(location).unwrap().match_key();
    lib.writer
        .call(move |c| c.execute("DELETE FROM relink WHERE location_key = ?1", [key]))
        .unwrap();
}

#[test]
fn an_accepted_match_replaces_a_carried_probable_guess() {
    let (lib, music, mounted) = e_music();
    let guess = lib.file(music, "Elsewhere/Omicron.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Omicron.mp3"), Some("200"));
    lib.matched_before(track, guess, "filename_only", 0.5);
    make_probable(&lib, track);
    lib.relink(&mounted);
    // Nothing better yet: the guess stands.
    assert_eq!(
        lib.matched(track),
        Some((guess, "filename_only".to_owned(), 0.5))
    );
    assert!(lib.probable(track));
    // The track's own file turns up at its Location.
    let own = lib.file(music, "Omicron.mp3", Some(200_000));
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), path(own));
    assert!(!lib.probable(track));
}

#[test]
fn a_probable_step_3_match_doesnt_replace_a_carried_probable_guess() {
    let (lib, music, mounted) = e_music();
    let guess = lib.file(music, "Elsewhere/Pi Guess.mp3", Some(300_000));
    lib.file(music, "Album/Renamed.mp3", Some(200_300));
    let track = lib.track(&loc("E:/Music/Album/Pi.mp3"), Some("200"));
    lib.matched_before(track, guess, "filename_only", 0.5);
    make_probable(&lib, track);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((guess, "filename_only".to_owned(), 0.5))
    );
}

#[test]
fn a_carried_trusted_match_stays_even_when_the_path_turns_up() {
    let (lib, music, mounted) = e_music();
    let earlier = lib.file(music, "Elsewhere/Rho.mp3", Some(200_000));
    lib.file(music, "Rho.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Rho.mp3"), Some("200"));
    lib.matched_before(track, earlier, "fingerprint", 0.95);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((earlier, "fingerprint".to_owned(), 0.95))
    );
}

#[test]
fn a_carried_match_gives_way_when_the_user_confirms_its_file_for_another_track() {
    for probable in [false, true] {
        let (lib, music, mounted) = e_music();
        let file = lib.file(music, "Sigma.mp3", Some(200_000));
        let carried = lib.track(&loc("E:/Gone/Sigma Old.mp3"), Some("200"));
        lib.matched_before(carried, file, "fingerprint", 0.5);
        if probable {
            make_probable(&lib, carried);
        }
        let owner = lib.track(&loc("E:/Gone/Sigma Real.mp3"), Some("200"));
        lib.confirm(&loc("E:/Gone/Sigma Real.mp3"), file, "user");
        lib.relink(&mounted);
        assert_eq!(lib.matched(owner), Some((file, "user".to_owned(), 1.0)));
        assert_eq!(lib.matched(carried), None, "probable: {probable}");
    }
}

#[test]
fn a_carried_match_to_a_file_that_is_gone_is_dropped() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Elsewhere/Tau.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Gone/Tau.mp3"), Some("200"));
    lib.matched_before(track, file, "fingerprint", 0.95);
    lib.gone(file);
    lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_withdrawn_confirmation_is_decided_again() {
    for method in ["filename_only", "user"] {
        let (lib, music, mounted) = e_music();
        let picked = lib.file(music, "Elsewhere/Upsilon Pick.mp3", Some(90_000));
        let location = loc("E:/Gone/Upsilon.mp3");
        let track = lib.track(&location, Some("200"));
        lib.confirm(&location, picked, method);
        lib.relink(&mounted);
        assert_eq!(lib.matched(track), Some((picked, method.to_owned(), 1.0)));
        withdraw(&lib, &location);
        lib.relink(&mounted);
        assert_eq!(lib.matched(track), None, "{method}");
    }
}

// --- Volumes and copies ---

#[test]
fn a_plugged_in_volume_counts_at_its_mount_now_even_if_its_stored_one_is_stale() {
    let lib = Lib::new();
    let away = lib.volume(&serial(1), Some(r"E:\"));
    // Plugged in at E: now, but the library last noted it at F:.
    let here = lib.volume(&serial(2), Some(r"F:\"));
    let away_music = lib.folder(away, "Music");
    lib.folder(here, "Music");
    lib.file(away_music, "Phi.mp3", None);
    let track = lib.track(&loc("E:/Music/Phi.mp3"), None);
    lib.relink(&Mounted::new([(serial(2), r"E:\")]));
    assert_eq!(lib.matched(track), None);
}

#[test]
fn a_copy_another_track_holds_blocks_the_other_copy_too() {
    let (lib, music, mounted) = e_music();
    let a = lib.file(music, "Album/Chi (1).mp3", Some(200_300));
    let b = lib.file(music, "Album/Chi (2).mp3", Some(200_300));
    lib.audio_hash(a, 5);
    lib.audio_hash(b, 5);
    // b (the higher id, so not the one picked) belongs to another track.
    let owner = lib.track(&loc("E:/Music/Album/Chi (2).mp3"), Some("200"));
    let track = lib.track(&loc("E:/Music/Album/Chi Gone.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(owner), path(b));
    assert_eq!(lib.matched(track), None);
    let _ = a;
}

#[test]
fn title_tags_are_read_only_for_the_files_step_3_matches() {
    let (lib, music, mounted) = e_music();
    for n in 0..20 {
        let f = lib.file(music, &format!("Other/F{n}.mp3"), Some(100_000));
        // Unparseable tags on every other file: never read.
        lib.tags(f, r#"{"id3v2": "not an array"}"#);
    }
    let renamed = lib.file(music, "Album/Renamed.mp3", Some(200_300));
    lib.title(renamed, "Psi");
    let track = lib.track_with(
        &loc("E:/Music/Album/Psi.mp3"),
        Some("200"),
        &[("Name", "Psi")],
    );
    let mounted2 = mounted.clone();
    let asked = lib
        .writer
        .call(move |c| {
            let tx = c.transaction()?;
            let input = super::super::load(&tx, &mounted2)?;
            let mut asked = Vec::new();
            rules::plan(&input, |file| {
                asked.push(file);
                Ok::<_, ()>(Vec::new())
            })
            .unwrap();
            Ok(asked)
        })
        .unwrap();
    assert_eq!(asked, [renamed]);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((renamed, "unique_duration".to_owned(), 0.8))
    );
}
