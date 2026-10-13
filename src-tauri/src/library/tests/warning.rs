//! The warning before a low-bitrate file is added (1bE-8). Synthetic data,
//! like the rest of the tests: made-up tracks, artists and files.

use super::*;
use crate::quality::verdict::{Assessment, Finding, Verdict};
use crate::quality::store::any_stale;
use crate::quality::stored_verdict;

const CONFIRMED: Confirmed = Confirmed { low_bitrate: true };

impl Lib {
    /// A track with one best file of this codec and bitrate (the columns
    /// the read stage fills). Returns (track, file).
    fn encoded(&self, title: &str, codec: &str, kbps: Option<i64>) -> (i64, i64) {
        let track = self.track(Some(title), Some("Made Up Artist"));
        let file = self.file(track, &format!("{title}.mp3"), "best", true);
        self.set_encoding(file, codec, kbps);
        (track, file)
    }

    fn set_encoding(&self, file: i64, codec: &str, kbps: Option<i64>) {
        let codec = codec.to_owned();
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET codec = ?2, bitrate = ?3 WHERE id = ?1",
                    (file, codec, kbps),
                )
            })
            .unwrap();
    }

    /// Settles every stored verdict, as the quality job does when it starts.
    fn settle_verdicts(&self) {
        self.writer
            .call(|c| crate::quality::settle(c).map(|_| ()))
            .unwrap();
    }

    fn stale(&self, file: i64) -> bool {
        self.writer
            .call(move |c| any_stale(c, Some(&[file])))
            .unwrap()
    }

    fn assessed(&self, file: i64) -> Assessment {
        self.writer.call(move |c| stored_verdict(c, file)).unwrap()
    }
}

fn low(kbps: u32) -> PromoteOutcome {
    PromoteOutcome::NeedsConfirmation {
        warning: AddWarning::LowBitrate { kbps },
    }
}

#[test]
fn a_low_bitrate_file_is_refused_without_the_confirmation_and_nothing_is_written() {
    let lib = Lib::new();
    let (track, _) = lib.encoded("Synthetic Tune", "mp3", Some(128));

    let answer = lib.promote_confirmed(track, Confirmed::default()).unwrap();
    assert_eq!(answer, low(128));

    // No Library track, no operation, so there is nothing to undo.
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(lib.operations(), 0);
    assert!(matches!(
        undo_last_via(&lib.writer).unwrap(),
        UndoOutcome::NothingToUndo
    ));
    // Asking again, still unconfirmed, still changes nothing.
    assert_eq!(
        lib.promote_confirmed(track, Confirmed::default()).unwrap(),
        low(128)
    );
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(lib.operations(), 0);
}

#[test]
fn a_low_bitrate_file_is_added_with_the_confirmation_as_one_undoable_operation() {
    let lib = Lib::new();
    let (track, file) = lib.encoded("Synthetic Tune", "mp3", Some(128));

    let added = lib.promote_confirmed(track, CONFIRMED).unwrap().done();
    assert!(added.added);
    assert_eq!(added.library_track.recording_id, track);
    assert_eq!(lib.library_tracks(), 1);
    assert_eq!(lib.operations(), 1);
    let linked: i64 = lib
        .writer
        .call(|c| c.query_row("SELECT linked_file_id FROM library_track", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(linked, file);

    match undo_last_via(&lib.writer).unwrap() {
        UndoOutcome::Undone { operation } => assert_eq!(operation.kind, PROMOTE_OPERATION),
        other => panic!("{other:?}"),
    }
    assert_eq!(lib.library_tracks(), 0);
}

#[test]
fn a_cut_off_file_that_also_has_the_low_bitrate_finding_still_warns() {
    let lib = Lib::new();
    let (track, file) = lib.encoded("Synthetic Tune", "mp3", Some(128));
    lib.insert(
        "UPDATE file SET quality_verdict = 'truncated' WHERE id = ?1",
        [file],
    );

    let answer = lib.promote_confirmed(track, Confirmed::default()).unwrap();
    // The headline verdict is the worse one; the finding is still there.
    let found = lib.assessed(file);
    assert_eq!(found.verdict, Some(Verdict::Truncated));
    assert!(found
        .findings
        .iter()
        .any(|f| matches!(f, Finding::LowBitrate { kbps: 128 })));
    assert_eq!(answer, low(128));
    assert_eq!(lib.library_tracks(), 0);

    assert!(
        lib.promote_confirmed(track, CONFIRMED)
            .unwrap()
            .done()
            .added
    );
}

#[test]
fn a_file_with_no_low_bitrate_finding_is_added_without_a_warning() {
    let lib = Lib::new();
    let (good, _) = lib.encoded("Synthetic Tune", "mp3", Some(320));
    // Just above the low line: a note elsewhere, never a warning.
    let (middling, _) = lib.encoded("Another Tune", "mp3", Some(192));
    // Lossless has no bitrate level.
    let (lossless, _) = lib.encoded("Third Tune", "flac", Some(900));
    // The read stage hasn't given it a bitrate: no finding.
    let (unread, _) = lib.encoded("Fourth Tune", "mp3", None);
    for track in [good, middling, lossless, unread] {
        let added = lib.promote(track).unwrap();
        assert!(added.added, "track {track}");
    }
    assert_eq!(lib.library_tracks(), 4);
    assert_eq!(lib.operations(), 4);
}

#[test]
fn a_stale_low_verdict_that_is_no_longer_low_is_settled_before_the_check() {
    let lib = Lib::new();
    let (track, file) = lib.encoded("Synthetic Tune", "mp3", Some(128));
    lib.settle_verdicts();
    assert_eq!(lib.assessed(file).verdict, Some(Verdict::LowBitrate));

    // The file was read again: it is 320 now. The stored verdict still says low.
    lib.set_encoding(file, "mp3", Some(320));
    assert!(lib.stale(file));

    let added = lib.promote(track).unwrap();
    assert!(added.added);
    assert_eq!(lib.library_tracks(), 1);
    // The check settled it.
    assert!(!lib.stale(file));
}

#[test]
fn a_stale_fine_verdict_that_is_now_low_is_settled_before_the_check() {
    let lib = Lib::new();
    let (track, file) = lib.encoded("Synthetic Tune", "mp3", Some(320));
    lib.settle_verdicts();
    assert_ne!(lib.assessed(file).verdict, Some(Verdict::LowBitrate));

    // The file was read again: it is 128 now. The stored verdict still says fine.
    lib.set_encoding(file, "mp3", Some(128));
    assert!(lib.stale(file));

    assert_eq!(
        lib.promote_confirmed(track, Confirmed::default()).unwrap(),
        low(128)
    );
    assert_eq!(lib.library_tracks(), 0);
    assert_eq!(lib.operations(), 0);
    assert!(!lib.stale(file));
}

#[test]
fn a_file_with_no_stored_verdict_yet_is_judged_when_added() {
    // No quality job has run: there is no file_verdict row at all.
    let lib = Lib::new();
    let (track, file) = lib.encoded("Synthetic Tune", "mp3", Some(96));
    assert_eq!(lib.count("SELECT count(*) FROM file_verdict"), 0);
    assert_eq!(
        lib.promote_confirmed(track, Confirmed::default()).unwrap(),
        low(96)
    );
    assert_eq!(lib.assessed(file).verdict, Some(Verdict::LowBitrate));
}

#[test]
fn the_warning_is_about_the_file_rekordbox_uses_when_that_is_the_one_linked() {
    let lib = Lib::new();
    let track = lib.track(Some("Synthetic Tune"), Some("Made Up Artist"));
    let best = lib.file(track, "best.flac", "best", true);
    lib.set_encoding(best, "flac", None);
    let used = lib.file(track, "used.mp3", "extra", true);
    lib.set_encoding(used, "mp3", Some(128));
    lib.rekordbox_match(used, false);

    assert_eq!(
        lib.promote_confirmed(track, Confirmed::default()).unwrap(),
        low(128)
    );
    let added = lib.promote_confirmed(track, CONFIRMED).unwrap().done();
    assert_eq!(
        added.library_track.file.map(|f| f.name),
        Some("used.mp3".into())
    );
}

#[test]
fn a_track_already_in_the_library_never_warns() {
    let lib = Lib::new();
    let (track, file) = lib.encoded("Synthetic Tune", "mp3", Some(320));
    assert!(lib.promote(track).unwrap().added);

    // Its file reads as low bitrate later; adding again still changes nothing.
    lib.set_encoding(file, "mp3", Some(128));
    let again = lib
        .promote_confirmed(track, Confirmed::default())
        .unwrap()
        .done();
    assert!(!again.added);
    assert_eq!(lib.operations(), 1);
}

#[test]
fn a_refusal_that_is_not_the_warning_still_comes_first() {
    let lib = Lib::new();
    let track = lib.track(Some("Synthetic Tune"), None);
    let file = lib.file(track, "gone.mp3", "best", false);
    lib.set_encoding(file, "mp3", Some(128));
    assert!(matches!(
        lib.promote_confirmed(track, Confirmed::default()),
        Err(LibraryError::FileMissing { .. })
    ));
}

#[test]
fn only_the_library_add_and_the_rekordbox_offer_insert_linked_library_tracks() {
    // The low-bitrate rule is in `promote`. This fails if a third code path
    // starts adding linked tracks without going through it; the rekordbox
    // offer is the one named exception (`start::add_offered`).
    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    let mut callers: Vec<String> = files
        .iter()
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            !name.starts_with("tests") && !path.components().any(|c| c.as_os_str() == "tests")
        })
        .filter(|path| {
            let text = std::fs::read_to_string(path).unwrap();
            text.lines()
                .map(str::trim)
                .any(|line| !line.starts_with("//") && line.contains("insert_linked("))
        })
        .map(|path| {
            path.strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    callers.sort();
    assert_eq!(callers, ["library/mod.rs", "start/mod.rs"]);
}
