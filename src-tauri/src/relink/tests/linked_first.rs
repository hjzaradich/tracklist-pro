//! Among files that are equally right for a row, the one a Library track is
//! linked to is matched first (1bE-10): a Library track is sent as
//! rekordbox's entry only while the entry is matched to its linked file, so
//! a copy that turns up later mustn't pull the entry off it. With no linked
//! file among them the order is what it always was: the track's best file,
//! else the lowest file id.

use super::*;

/// Makes `best` its track's best file and every other file of that track
/// an extra, as the ranking would.
fn best_now(lib: &Lib, best: i64) {
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE recording_file SET role = 'extra'
                 WHERE file_id <> ?1
                   AND recording_id = (SELECT recording_id FROM recording_file
                                       WHERE file_id = ?1)",
                [best],
            )?;
            c.execute(
                "UPDATE recording_file SET role = 'best' WHERE file_id = ?1",
                [best],
            )
        })
        .unwrap();
}

/// A rekordbox row at `E:\Music\Old.mp3` whose file is gone (audio 1), as
/// relink's step 4 finds it.
fn gone_at_its_location() -> (Lib, i64, Mounted, i64) {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    let row = lib.track_with(&loc("E:/Music/Old.mp3"), Some("200"), &[("Name", "Kappa")]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), path(old));
    lib.gone(old);
    (lib, music, mounted, row)
}

fn copy(lib: &Lib, music: i64, name: &str) -> i64 {
    let file = lib.file(music, &format!("Moved {name}/{name}.flac"), None);
    lib.hashed(file, 1);
    file
}

#[test]
fn a_later_exact_copy_does_not_pull_the_entry_off_the_linked_file() {
    let (lib, music, mounted, row) = gone_at_its_location();
    let first = copy(&lib, music, "First");
    lib.group(&[first]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), same_audio(first));
    let track = lib.in_library(first);

    // An exact copy turns up later, joins the track, and the ranking calls
    // it the best file (more tags, say).
    let later = copy(&lib, music, "Later");
    lib.joins(later, first);
    best_now(&lib, later);
    for _ in 0..2 {
        lib.relink(&mounted);
        assert_eq!(lib.matched(row), same_audio(first));
    }
    assert_eq!(lib.linked_file(track), first);
    assert_eq!(
        lib.sent(track),
        crate::send_values::SentAs::Entry(row),
        "the track is still sent as rekordbox's own entry"
    );
}

#[test]
fn with_no_linked_file_among_the_copies_the_tracks_best_file_is_matched_as_before() {
    let (lib, music, mounted, row) = gone_at_its_location();
    let first = copy(&lib, music, "First");
    let later = copy(&lib, music, "Later");
    lib.group(&[first, later]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), same_audio(first));
    best_now(&lib, later);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), same_audio(later));
}

#[test]
fn with_no_linked_file_and_no_best_file_among_them_the_lowest_file_id_is_matched_as_before() {
    let (lib, music, mounted, row) = gone_at_its_location();
    let first = copy(&lib, music, "First");
    let later = copy(&lib, music, "Later");
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), same_audio(first.min(later)));
}

#[test]
fn a_linked_file_another_row_holds_is_not_taken_for_being_linked() {
    // Linked first is an order among the candidates, not a way past the
    // rule that a file another row holds isn't a candidate.
    let (lib, music, mounted, row) = gone_at_its_location();
    let linked = copy(&lib, music, "Linked");
    let free = copy(&lib, music, "Free");
    lib.group(&[free, linked]);
    lib.in_library(linked);
    let holder = lib.track(&loc("E:/Music/Moved Linked/Linked.flac"), None);
    lib.relink(&mounted);
    assert_eq!(lib.matched(holder), path(linked));
    assert_eq!(lib.matched(row), same_audio(free));
}

#[test]
fn of_several_track_mates_the_linked_one_is_matched_before_the_best_file() {
    // The same order among files of the gone file's track that hold other
    // audio (another encode each).
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    let row = lib.track_with(&loc("E:/Music/Old.mp3"), Some("200"), &[("Name", "Kappa")]);
    lib.relink(&mounted);
    lib.gone(old);
    let best = lib.file(music, "Moved Best/Best.flac", Some(200_400));
    let linked = lib.file(music, "Moved Linked/Linked.flac", Some(200_400));
    for (file, hash) in [(best, 2), (linked, 3)] {
        lib.hashed(file, hash);
        lib.title(file, "Kappa");
    }
    lib.merged(&[best, old, linked]);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), same_track(best));
    lib.in_library(linked);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), same_track(linked));
}

#[test]
fn exact_copies_that_fit_by_name_match_the_linked_file_before_the_tracks_best_file() {
    // Steps 2, 3 and 5 pick among exact copies the same way.
    let (lib, music, mounted) = e_music();
    let a = lib.file(music, "Copy A/Eta.mp3", Some(200_300));
    let b = lib.file(music, "Copy B/Eta.mp3", Some(200_300));
    lib.audio_hash(a, 7);
    lib.audio_hash(b, 7);
    lib.group(&[b, a]);
    let row = lib.track(&loc("E:/Gone/Eta.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(row).map(|m| m.0), Some(b));
    lib.in_library(a);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(row),
        Some((a, "filename_duration".to_owned(), 0.9))
    );
}

// --- Linked first is a tie-break at equal trust, nothing more ---

/// A rekordbox row named Kappa at `E:\Music\Old.mp3`, whose file (audio 1,
/// 200 s) is gone, and two other encodes of it merged into its track: the
/// best file titled `best_title`, the other titled `other_title`. Returns
/// the library, what is mounted, the row, the best file and the other.
fn two_mates(best_title: &str, other_title: &str) -> (Lib, Mounted, i64, i64, i64) {
    let (lib, music, mounted) = e_music();
    let old = lib.file(music, "Old.mp3", Some(200_000));
    lib.hashed(old, 1);
    let row = lib.track_with(&loc("E:/Music/Old.mp3"), Some("200"), &[("Name", "Kappa")]);
    lib.relink(&mounted);
    lib.gone(old);
    let best = lib.file(music, "Moved Best/Best.flac", Some(200_400));
    let other = lib.file(music, "Moved Other/Other.flac", Some(200_400));
    for (file, hash, title) in [(best, 2, best_title), (other, 3, other_title)] {
        lib.hashed(file, hash);
        lib.title(file, title);
    }
    lib.merged(&[best, old, other]);
    (lib, mounted, row, best, other)
}

#[test]
fn a_trusted_match_stays_trusted_when_a_library_track_links_a_mate_whose_title_disagrees() {
    // Only the best file is titled as rekordbox names the entry. Matched to
    // the linked mate the entry would be probable, and its rekordbox data
    // detached until someone confirmed it: being linked never costs that.
    let (lib, mounted, row, best, other) = two_mates("Kappa", "Kappa (Dirty)");
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), same_track(best));
    assert!(!lib.probable(row));
    lib.in_library(other);
    for _ in 0..2 {
        lib.relink(&mounted);
        assert_eq!(lib.matched(row), same_track(best));
        assert!(!lib.probable(row));
    }
}

#[test]
fn a_probable_match_is_not_made_trusted_by_a_linked_mate_whose_title_agrees() {
    // The other way round: the tie-break never changes what a row gets,
    // only which of two equally trusted files it is on.
    let (lib, mounted, row, best, other) = two_mates("Kappa (Dirty)", "Kappa");
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(row),
        Some((best, "fingerprint".to_owned(), 0.4))
    );
    assert!(lib.probable(row));
    lib.in_library(other);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(row),
        Some((best, "fingerprint".to_owned(), 0.4))
    );
    assert!(lib.probable(row));
}

#[test]
fn at_equal_trust_the_linked_mate_is_matched_trusted_or_probable_alike() {
    for (titles, confidence, probable) in [("Kappa", 0.9, false), ("Kappa (Dirty)", 0.4, true)] {
        let (lib, mounted, row, best, other) = two_mates(titles, titles);
        lib.relink(&mounted);
        assert_eq!(lib.matched(row).map(|m| m.0), Some(best));
        lib.in_library(other);
        lib.relink(&mounted);
        assert_eq!(
            lib.matched(row),
            Some((other, "fingerprint".to_owned(), confidence))
        );
        assert_eq!(lib.probable(row), probable);
    }
}

/// Two exact copies in the folder a rekordbox row named Kappa still names
/// (step 3, a unique duration): the best file titled `best_title`, the other
/// titled `other_title`. Returns the library, what is mounted, the row, the
/// best file and the other.
fn two_copies_by_duration(best_title: &str, other_title: &str) -> (Lib, Mounted, i64, i64, i64) {
    let (lib, music, mounted) = e_music();
    let other = lib.file(music, "Album/Kappa (1).mp3", Some(200_300));
    let best = lib.file(music, "Album/Kappa (2).mp3", Some(200_300));
    for (file, title) in [(best, best_title), (other, other_title)] {
        lib.audio_hash(file, 9);
        lib.title(file, title);
    }
    lib.group(&[best, other]);
    let row = lib.track_with(
        &loc("E:/Music/Album/Kappa.mp3"),
        Some("200"),
        &[("Name", "Kappa")],
    );
    (lib, mounted, row, best, other)
}

#[test]
fn exact_copies_with_different_tags_keep_the_trust_of_the_usual_copy_when_the_other_is_linked() {
    // Step 3 trusts a match on the title tag of the file itself, and exact
    // copies can be tagged differently.
    let trusted = Some((0, "unique_duration".to_owned(), 0.8));
    let probable = Some((0, "unique_duration".to_owned(), 0.4));
    for (best_title, other_title, expected) in [
        ("Kappa", "Untitled", &trusted),
        ("Untitled", "Kappa", &probable),
    ] {
        let (lib, mounted, row, best, other) = two_copies_by_duration(best_title, other_title);
        let on_best = expected.clone().map(|(_, method, c)| (best, method, c));
        lib.relink(&mounted);
        assert_eq!(lib.matched(row), on_best);
        lib.in_library(other);
        lib.relink(&mounted);
        assert_eq!(lib.matched(row), on_best, "{best_title} / {other_title}");
    }
    // Tagged alike, the linked copy is matched.
    let (lib, mounted, row, _best, other) = two_copies_by_duration("Kappa", "Kappa");
    lib.in_library(other);
    lib.relink(&mounted);
    assert_eq!(
        lib.matched(row),
        Some((other, "unique_duration".to_owned(), 0.8))
    );
}

#[test]
fn a_row_none_of_whose_candidates_is_linked_gets_the_file_it_always_got() {
    // A Library track elsewhere changes nothing for this row.
    let (lib, mounted, row, best, _other) = two_mates("Kappa", "Kappa");
    let elsewhere = lib.file(1, "Unrelated/Song.mp3", Some(100_000));
    lib.hashed(elsewhere, 50);
    lib.in_library(elsewhere);
    lib.relink(&mounted);
    assert_eq!(lib.matched(row), same_track(best));
    assert!(!lib.probable(row));
}
