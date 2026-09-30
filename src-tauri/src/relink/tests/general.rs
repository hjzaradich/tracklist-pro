//! Rules that hold across the steps.

use super::*;

#[test]
fn streaming_entries_are_never_matched() {
    let (lib, music, mounted) = e_music();
    // A file named like the stream's id, with its duration.
    let file = lib.file(music, "tracks/12345", Some(200_000));
    let stream = lib.track("soundcloud:tracks:12345", Some("200"));
    let prefixed = lib.track("file://localhost/spotify:track:12345", Some("200"));
    // Not even a confirmed relink attaches a file to one.
    lib.confirm("soundcloud:tracks:12345", file, "user");
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(stream), None);
    assert_eq!(lib.matched(prefixed), None);
    assert_eq!(summary.streaming, 2);
    assert_eq!(summary.missing, 0);
}

/// A library exercising every step, some matches and some misses.
fn mixed() -> (Lib, Mounted) {
    let (lib, music, mounted) = e_music();
    let kept = lib.file(music, "Album/Kept.mp3", Some(250_000));
    lib.file(music, "Album/Renamed.mp3", Some(200_300));
    lib.file(music, "Moved/Name.mp3", Some(180_000));
    lib.file(music, "Twins/A.mp3", Some(100_000));
    lib.file(music, "Twins/B.mp3", Some(100_200));
    let confirmed = lib.file(music, "Picked.mp3", Some(90_000));
    lib.file(music, "Unread/Name2.mp3", None);
    lib.track(&loc("E:/Music/Album/Kept.mp3"), Some("250"));
    lib.track(&loc("E:/Music/Album/Original.mp3"), Some("200"));
    lib.track(&loc("D:/Gone/Name.mp3"), Some("180"));
    lib.track(&loc("E:/Music/Twins/Gone.mp3"), Some("100"));
    lib.track(&loc("D:/Gone/Name2.mp3"), Some("180"));
    lib.track("soundcloud:tracks:1", Some("200"));
    lib.track(&loc("E:/Gone/Confirmed.mp3"), Some("90"));
    lib.confirm(&loc("E:/Gone/Confirmed.mp3"), confirmed, "user");
    let _ = kept;
    (lib, mounted)
}

#[test]
fn relinking_again_with_nothing_changed_changes_nothing() {
    let (lib, mounted) = mixed();
    let first = lib.relink(&mounted);
    let after_first = lib.all_matches();
    let want = Summary {
        confirmed: 1,
        path: 1,
        filename_duration: 1,
        unique_duration: 0,
        probable: 1,
        other: 0,
        streaming: 1,
        missing: 2,
        changed: 4,
    };
    assert_eq!(first, want);

    let second = lib.relink(&mounted);
    assert_eq!(lib.all_matches(), after_first);
    assert_eq!(second, Summary { changed: 0, ..want });
}

#[test]
fn a_match_from_a_later_step_carries_over_while_its_file_is_present() {
    let (lib, music, mounted) = e_music();
    let earlier = lib.file(music, "Earlier.mp3", Some(200_000));
    let at_path = lib.file(music, "Delta.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Delta.mp3"), Some("200"));
    // A later step (1aD) matched it elsewhere; this run doesn't remake
    // fingerprint matches, so it stands.
    lib.matched_before(track, earlier, "fingerprint", 0.95);
    let summary = lib.relink(&mounted);
    assert_eq!(
        lib.matched(track),
        Some((earlier, "fingerprint".to_owned(), 0.95))
    );
    assert_eq!((summary.other, summary.changed), (1, 0));
    let _ = at_path;
}

#[test]
fn a_file_matched_before_a_run_isnt_given_to_another_track() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "New/Epsilon.mp3", Some(200_300));
    let first = lib.track(&loc("E:/Gone/Anything.mp3"), Some("200"));
    lib.matched_before(first, file, "fingerprint", 0.95);
    let second = lib.track(&loc("E:/Old/Epsilon.mp3"), Some("200"));
    lib.relink(&mounted);
    assert_eq!(lib.matched(second), None);
}

#[test]
fn an_empty_snapshot_relinks_to_nothing() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Zeta.mp3", Some(200_000));
    assert_eq!(lib.relink(&mounted), Summary::default());
}

#[test]
fn a_location_that_doesnt_decode_stays_missing() {
    let (lib, music, mounted) = e_music();
    lib.file(music, "Eta.mp3", Some(200_000));
    let track = lib.track(&loc("E:/Music/Eta.mp3"), Some("200"));
    // A row whose stored Location no longer decodes (hand-edited).
    lib.writer
        .call(move |c| {
            c.execute("DROP TRIGGER rekordbox_track_values_are_read_only", [])?;
            c.execute(
                "UPDATE rekordbox_track
                 SET attributes = json_set(attributes, '$.Location', 'file://localhost/E:/%zz')
                 WHERE id = ?1",
                [track],
            )
        })
        .unwrap();
    let summary = lib.relink(&mounted);
    assert_eq!(lib.matched(track), None);
    assert_eq!(summary.missing, 1);
}
