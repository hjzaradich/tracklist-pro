//! The remembered decisions (1bE-5, 1bE-7): dismiss a better file, keep a
//! flagged file, want a better one. The switch of a gone linked file is
//! tested beside relink and the send (`relink/tests/switch.rs`).

use super::super::actions::{
    dismiss_better_file, keep_anyway, want_better_file, wants_better, Acted, Refused,
    DISMISS_OPERATION, KEEP_OPERATION, WANT_BETTER_OPERATION,
};
use super::*;
use crate::ops::UndoOutcome;
use crate::quality::verdict::Verdict;

impl Lib {
    /// Gives the file the audio `n` stands for, as the hash stage would.
    fn audio(&self, file: i64, n: u8) {
        self.writer
            .call(move |c| {
                c.execute(
                    "UPDATE file SET audio_hash = ?2 WHERE id = ?1",
                    (file, vec![n; 34]),
                )
            })
            .unwrap();
    }

    fn dismiss(&self, track: i64, file: i64) -> Result<Acted, Refused> {
        self.writer
            .call(move |c| Ok(dismiss_better_file(c, track, file)))
            .unwrap()
    }

    fn keep(&self, track: i64) -> Result<Acted, Refused> {
        self.writer
            .call(move |c| Ok(keep_anyway(c, track)))
            .unwrap()
    }

    fn want_better(&self, track: i64) -> Result<Acted, Refused> {
        self.writer
            .call(move |c| Ok(want_better_file(c, track)))
            .unwrap()
    }

    fn wants_better(&self, track: i64) -> bool {
        self.writer.call(move |c| wants_better(c, track)).unwrap()
    }

    /// Undo, as the Undo button runs it; returns the kind it took back.
    fn undo(&self) -> String {
        let outcome = self
            .writer
            .call(|c| Ok(crate::ops::undo_for_user(c, None)))
            .unwrap()
            .unwrap();
        match outcome {
            UndoOutcome::Undone { operation } => operation.kind,
            other => panic!("not undone: {other:?}"),
        }
    }

    /// The best-file ranking of `track`, as the roles job runs it.
    fn rank(&self, track: i64) {
        self.exec(&format!(
            "INSERT OR IGNORE INTO best_file_due (recording_id) VALUES ({track})"
        ));
        self.writer
            .call(move |c| crate::file_roles::settle_tracks(c, &[track]).map(|_| ()))
            .unwrap();
    }

    /// The stored "better file" of `track`: the file and the step.
    fn better_of(&self, track: i64) -> Option<(i64, String)> {
        use rusqlite::OptionalExtension;
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT file_id, step FROM better_file WHERE recording_id = ?1",
                    [track],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
            })
            .unwrap()
    }

    /// The operations that aren't undone, by kind, oldest first.
    fn kinds(&self) -> Vec<String> {
        self.writer
            .call(|c| {
                c.prepare("SELECT kind FROM operation WHERE undone_at IS NULL ORDER BY id")?
                    .query_map([], |r| r.get(0))?
                    .collect()
            })
            .unwrap()
    }

    fn quality_items(&self) -> Vec<(ReviewKind, i64, Option<i64>)> {
        self.ids(ReviewGroup::QualityProblems)
    }
}

/// A Library track linked to an mp3 on disk, with a FLAC of it in the same
/// track: the ranking calls the FLAC a better file. Returns the track, the
/// linked file and the better one.
fn with_a_better_file(lib: &Lib) -> (i64, i64, i64) {
    let (track, linked) = lib.simple("Made-up Tune");
    lib.audio(linked, 1);
    let better = lib.file(track, "Made-up Tune.flac", "flac", true);
    lib.audio(better, 2);
    lib.library(track, linked);
    lib.rank(track);
    assert_eq!(lib.better_of(track), Some((better, "format".to_owned())));
    (track, linked, better)
}

/// A Library track whose only file is on disk and flagged `low_bitrate`.
fn flagged(lib: &Lib, title: &str, audio: u8) -> (i64, i64) {
    let (track, file) = lib.simple(title);
    lib.audio(file, audio);
    lib.library(track, file);
    lib.verdict(file, "low_bitrate");
    (track, file)
}

const BETTER: ReviewKind = ReviewKind::BetterFile;
const QUALITY: ReviewKind = ReviewKind::QualityProblem;

// ---- "Better file available": dismiss ---------------------------------------

#[test]
fn dismissing_a_better_file_takes_its_item_out_of_the_queue_and_undo_brings_it_back() {
    let lib = Lib::new();
    let (track, linked, better) = with_a_better_file(&lib);
    assert_eq!(lib.quality_items(), [(BETTER, track, None)]);
    assert_eq!(lib.counts().quality_problems, 1);

    let acted = lib.dismiss(track, better).unwrap();
    assert!(acted.operation_id.is_some());
    assert_eq!(lib.quality_items(), []);
    assert_eq!(lib.counts().quality_problems, 0);
    assert_eq!(lib.counts().total, 0);
    assert_eq!(lib.compare(id(BETTER, track, None)), None);
    assert_eq!(lib.kinds(), [DISMISS_OPERATION]);
    // Nothing else changed: the link, and the stored better file.
    assert_eq!(lib.num("SELECT linked_file_id FROM library_track"), linked);
    assert_eq!(lib.better_of(track), Some((better, "format".to_owned())));

    assert_eq!(lib.undo(), DISMISS_OPERATION);
    assert_eq!(lib.quality_items(), [(BETTER, track, None)]);
    assert_eq!(lib.counts().quality_problems, 1);
    assert_eq!(lib.num("SELECT count(*) FROM review_decision"), 0);
}

#[test]
fn a_dismissal_survives_a_roles_rerun_a_retag_and_a_rescan_that_gives_the_file_a_new_id() {
    let lib = Lib::new();
    let (track, _linked, better) = with_a_better_file(&lib);
    lib.dismiss(track, better).unwrap();

    // The roles job ranks the track again.
    lib.rank(track);
    assert_eq!(lib.better_of(track), Some((better, "format".to_owned())));
    assert_eq!(lib.quality_items(), []);

    // The better file is retagged and moved: the same audio.
    lib.exec(&format!(
        "UPDATE file SET rel_path = 'Elsewhere/renamed.flac', rel_path_key = 'Elsewhere/renamed.flac',
                         raw_tags = '{{}}', blake3 = zeroblob(32)
         WHERE id = {better}"
    ));
    lib.rank(track);
    assert_eq!(lib.quality_items(), []);

    // Its folder is removed and added again: another row, another id, the
    // same audio. The decision names what the file holds, so it still holds.
    lib.exec(&format!(
        "DELETE FROM better_file WHERE file_id = {better};
         DELETE FROM recording_file WHERE file_id = {better};
         DELETE FROM file WHERE id = {better};"
    ));
    let filler = lib.file(track, "filler.mp3", "mp3", false);
    let again = lib.file(track, "Made-up Tune.flac", "flac", true);
    assert_ne!(again, better);
    lib.audio(again, 2);
    lib.audio(filler, 77);
    lib.rank(track);
    assert_eq!(lib.better_of(track), Some((again, "format".to_owned())));
    assert_eq!(lib.quality_items(), []);
    assert_eq!(lib.counts().quality_problems, 0);
}

#[test]
fn a_different_better_file_is_a_new_item_after_a_dismissal() {
    let lib = Lib::new();
    let (track, _linked, better) = with_a_better_file(&lib);
    lib.dismiss(track, better).unwrap();
    assert_eq!(lib.quality_items(), []);

    // Another file, holding other audio, is the better file now.
    lib.exec(&format!(
        "DELETE FROM better_file WHERE file_id = {better};
         DELETE FROM recording_file WHERE file_id = {better};
         DELETE FROM file WHERE id = {better};"
    ));
    let other = lib.file(track, "Made-up Tune (remaster).flac", "flac", true);
    lib.audio(other, 3);
    lib.rank(track);
    assert_eq!(lib.better_of(track), Some((other, "format".to_owned())));
    assert_eq!(lib.quality_items(), [(BETTER, track, None)]);

    // Dismissing that one too leaves two decisions, one per better file.
    lib.dismiss(track, other).unwrap();
    assert_eq!(lib.quality_items(), []);
    assert_eq!(lib.num("SELECT count(*) FROM review_decision"), 2);
}

#[test]
fn a_better_file_that_is_encoded_again_is_a_new_item() {
    // The key is the audio: the same file with other audio is another file.
    let lib = Lib::new();
    let (track, _linked, better) = with_a_better_file(&lib);
    lib.dismiss(track, better).unwrap();
    lib.audio(better, 9);
    lib.rank(track);
    assert_eq!(lib.quality_items(), [(BETTER, track, None)]);
}

#[test]
fn a_dismissal_is_for_the_file_the_track_was_linked_to() {
    // Linked to other audio later, the same better file is asked about again.
    let lib = Lib::new();
    let (track, linked, better) = with_a_better_file(&lib);
    lib.dismiss(track, better).unwrap();
    lib.audio(linked, 5);
    lib.rank(track);
    assert_eq!(lib.quality_items(), [(BETTER, track, None)]);
}

#[test]
fn only_the_item_in_the_queue_can_be_dismissed() {
    let lib = Lib::new();
    let (track, linked, better) = with_a_better_file(&lib);
    // Not the file the item names.
    assert!(matches!(lib.dismiss(track, linked), Err(Refused::ItemGone)));
    // Not a track with no item.
    let (other, file) = lib.simple("Another Tune");
    lib.library(other, file);
    assert!(matches!(lib.dismiss(other, better), Err(Refused::ItemGone)));
    // Twice: the second finds no item.
    lib.dismiss(track, better).unwrap();
    assert!(matches!(lib.dismiss(track, better), Err(Refused::ItemGone)));
    assert_eq!(lib.kinds(), [DISMISS_OPERATION]);
}

#[test]
fn a_linked_file_gone_item_cannot_be_dismissed() {
    let lib = Lib::new();
    let track = lib.track("Made-up Tune");
    let gone = lib.file(track, "gone.mp3", "mp3", false);
    let copy = lib.file(track, "copy.mp3", "mp3", true);
    lib.audio(gone, 1);
    lib.audio(copy, 2);
    lib.library(track, gone);
    lib.better(track, copy, "onDisk");
    assert!(matches!(lib.dismiss(track, copy), Err(Refused::ItemGone)));
    assert_eq!(lib.counts().clear_best_file, 1);
    assert_eq!(lib.num("SELECT count(*) FROM review_decision"), 0);
}

#[test]
fn a_decision_cannot_be_remembered_for_a_file_with_no_hash_yet() {
    let lib = Lib::new();
    let (track, linked) = lib.simple("Made-up Tune");
    let better = lib.file(track, "Made-up Tune.flac", "flac", true);
    lib.library(track, linked);
    lib.better(track, better, "format");
    assert!(matches!(
        lib.dismiss(track, better),
        Err(Refused::NoAudioKnown)
    ));
    let (flagged, file) = lib.simple("Another Tune");
    lib.library(flagged, file);
    lib.verdict(file, "low_bitrate");
    assert!(matches!(lib.keep(flagged), Err(Refused::NoAudioKnown)));
    assert!(matches!(
        lib.want_better(flagged),
        Err(Refused::NoAudioKnown)
    ));
    assert_eq!(lib.num("SELECT count(*) FROM operation"), 0);
    // Both items are still there.
    assert_eq!(lib.counts().quality_problems, 2);
}

// ---- quality problems: "Keep anyway" -----------------------------------------

#[test]
fn keep_anyway_takes_the_track_out_of_quality_problems_and_undo_brings_it_back() {
    let lib = Lib::new();
    let (track, _file) = flagged(&lib, "Made-up Tune", 1);
    let (other, _) = flagged(&lib, "Another Tune", 2);
    assert_eq!(
        lib.quality_items(),
        [(QUALITY, track, None), (QUALITY, other, None)]
    );

    lib.keep(track).unwrap();
    assert_eq!(lib.quality_items(), [(QUALITY, other, None)]);
    assert_eq!(lib.counts().quality_problems, 1);
    assert_eq!(lib.compare(id(QUALITY, track, None)), None);
    assert_eq!(lib.kinds(), [KEEP_OPERATION]);

    assert_eq!(lib.undo(), KEEP_OPERATION);
    assert_eq!(
        lib.quality_items(),
        [(QUALITY, track, None), (QUALITY, other, None)]
    );
    assert_eq!(lib.counts().quality_problems, 2);
}

#[test]
fn a_kept_file_stays_out_of_the_queue_for_good_while_the_ranking_still_flags_it() {
    let lib = Lib::new();
    let (track, file) = flagged(&lib, "Made-up Tune", 1);
    lib.keep(track).unwrap();

    // The verdict is untouched: the file is still flagged.
    assert_eq!(
        lib.num("SELECT count(*) FROM file_verdict WHERE verdict = 'low_bitrate'"),
        1
    );
    let facts = lib
        .writer
        .call(move |c| crate::file_roles::facts::load(c, &[track]))
        .unwrap();
    assert_eq!(
        facts[&track][0].facts.quality.verdict,
        Some(Verdict::LowBitrate)
    );

    // Measured again, and assessed again: still not in the queue.
    lib.exec(&format!(
        "DELETE FROM file_verdict WHERE file_id = {file};
         INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_hz, decoded_ms,
                                   header_ms, decode_errors, ended)
         SELECT id, audio_hash, 1, 15000, 215000, 215000, 0, 'complete' FROM file WHERE id = {file};"
    ));
    lib.verdict(file, "suspect_transcode");
    lib.rank(track);
    assert_eq!(lib.quality_items(), []);
    // Moved and retagged: still not.
    lib.exec(&format!(
        "UPDATE file SET rel_path = 'Elsewhere/kept.mp3', rel_path_key = 'Elsewhere/kept.mp3'
         WHERE id = {file}"
    ));
    assert_eq!(lib.quality_items(), []);

    // The ranking still treats it as flagged: a copy that isn't flagged is
    // ahead of it for its verdict, and that is a "Better file available"
    // item, which "Keep anyway" never covered.
    let fine = lib.file(track, "Made-up Tune (fine).mp3", "mp3", true);
    lib.audio(fine, 2);
    lib.verdict(fine, "ok");
    lib.rank(track);
    assert_eq!(lib.better_of(track), Some((fine, "verdict".to_owned())));
    assert_eq!(lib.quality_items(), [(BETTER, track, None)]);
}

#[test]
fn keep_anyway_is_for_the_audio_that_was_flagged() {
    // The track is linked to other audio later (encoded again under the
    // same name), and that is flagged too: a new item.
    let lib = Lib::new();
    let (track, file) = flagged(&lib, "Made-up Tune", 1);
    lib.keep(track).unwrap();
    assert_eq!(lib.quality_items(), []);
    lib.exec(&format!("DELETE FROM file_verdict WHERE file_id = {file}"));
    lib.audio(file, 8);
    lib.verdict(file, "low_bitrate");
    assert_eq!(lib.quality_items(), [(QUALITY, track, None)]);
}

// ---- quality problems: "Find a better file" -----------------------------------

#[test]
fn find_a_better_file_stores_the_wish_takes_the_item_out_and_undo_brings_it_back() {
    let lib = Lib::new();
    let (track, _file) = flagged(&lib, "Made-up Tune", 1);
    assert!(!lib.wants_better(track));

    lib.want_better(track).unwrap();
    assert!(lib.wants_better(track));
    assert_eq!(lib.quality_items(), []);
    assert_eq!(lib.counts().total, 0);
    assert_eq!(lib.kinds(), [WANT_BETTER_OPERATION]);
    assert_eq!(
        lib.num("SELECT count(*) FROM review_decision WHERE kind = 'want_better'"),
        1
    );

    assert_eq!(lib.undo(), WANT_BETTER_OPERATION);
    assert!(!lib.wants_better(track));
    assert_eq!(lib.quality_items(), [(QUALITY, track, None)]);
}

#[test]
fn the_wish_for_a_better_file_survives_a_new_measurement() {
    let lib = Lib::new();
    let (track, file) = flagged(&lib, "Made-up Tune", 1);
    lib.want_better(track).unwrap();
    lib.exec(&format!(
        "DELETE FROM file_verdict WHERE file_id = {file};
         INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_hz, decoded_ms,
                                   header_ms, decode_errors, ended)
         SELECT id, audio_hash, 1, 15000, 215000, 215000, 0, 'complete' FROM file WHERE id = {file};"
    ));
    lib.verdict(file, "suspect_transcode");
    lib.rank(track);
    assert!(lib.wants_better(track));
    assert_eq!(lib.quality_items(), []);
}

#[test]
fn a_track_with_no_quality_problem_has_nothing_to_keep_or_to_wish_for() {
    let lib = Lib::new();
    let (track, file) = lib.simple("Made-up Tune");
    lib.audio(file, 1);
    lib.library(track, file);
    lib.verdict(file, "ok");
    assert!(matches!(lib.keep(track), Err(Refused::ItemGone)));
    assert!(matches!(lib.want_better(track), Err(Refused::ItemGone)));
    // Answered once, an item can't be answered again.
    let (flagged, _) = flagged(&lib, "Another Tune", 2);
    lib.keep(flagged).unwrap();
    assert!(matches!(lib.want_better(flagged), Err(Refused::ItemGone)));
    assert!(matches!(lib.keep(flagged), Err(Refused::ItemGone)));
    assert_eq!(lib.kinds(), [KEEP_OPERATION]);
}

#[test]
fn the_key_a_decision_is_kept_by_is_groupings_name_for_a_files_audio() {
    use super::super::actions::content_key;
    assert_eq!(content_key("f"), crate::grouping::CONTENT_KEY);
    assert_eq!(
        content_key("lf"),
        "coalesce(nullif(lf.audio_hash, x''), lf.blake3)"
    );
}
