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
