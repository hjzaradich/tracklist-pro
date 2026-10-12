//! A cut-off copy (quality verdict `truncated`) inside the whole,
//! same-named file is a worse duplicate, not a cut (TASKS 1bC-6), through
//! the database: the real quality store, the real matching pass, the real
//! rules, the deciding job.

use super::support::*;
use crate::grouping::{decide, remember, Verdict, PAGE};
use crate::quality;

/// About two minutes of made-up audio.
const LEN: usize = 900;

fn file<'a>(name: &'a str, title: &'a str, n: u8, items: Vec<u32>) -> NewFile<'a> {
    by("Artist A", name, title, n, items)
}

fn by<'a>(artist: &'a str, name: &'a str, title: &'a str, n: u8, items: Vec<u32>) -> NewFile<'a> {
    NewFile {
        name,
        title: Some(title),
        artist: Some(artist),
        hash: Some(hash(n)),
        items: Some(items),
        blake3: None,
    }
}

/// The audio of a copy of `whole` that stops after `items` items: its
/// beginning, encoded again.
fn cut_off(whole: &[u32], items: usize, seed: u64) -> Vec<u32> {
    reencoded(&whole[..items], 3, seed)
}

impl Db {
    /// "Velo" whole, and a copy of it that stops before the half. Nothing
    /// says yet that the copy is cut short.
    fn velo_and_its_cut_off_copy(&self) -> (i64, i64) {
        let velo = track(1, LEN);
        let whole = self.add(file("Velo.flac", "Velo", 1, velo.clone()));
        let copy = self.add(file("Velo.mp3", "Velo", 2, cut_off(&velo, 400, 7)));
        (whole, copy)
    }

    /// The read stage finds `file` cut short (or, with `false`, whole
    /// after all). The stored verdict is out of date until it's settled.
    fn read_as_cut_short(&self, file: i64, cut_short: bool) {
        let verdict = match cut_short {
            true => "'truncated'",
            false => "NULL",
        };
        self.sql(&format!(
            "UPDATE file SET quality_verdict = {verdict} WHERE id = {file}"
        ));
    }

    /// Brings every stored verdict up to date, as a quality job does.
    fn settle(&self) {
        self.writer.call(quality::settle).unwrap();
    }

    /// What follows a quality job: the files whose verdict changed in
    /// being truncated are made due, and a deciding job is asked for.
    /// Returns whether one was.
    fn quality_job_ended(&self) -> bool {
        let mut asked = false;
        crate::grouping::request_decide_after_quality(&self.writer, |_| {
            asked = true;
            Ok::<_, crate::db::DbError>(crate::jobs::JobId(99))
        })
        .unwrap();
        asked
    }

    fn verdict(&self, file: i64) -> Option<quality::verdict::Verdict> {
        self.writer
            .call(move |c| quality::stored_verdict(c, file))
            .unwrap()
            .verdict
    }

    /// The verdict `file`'s record says it was decided at.
    fn decided_at(&self, file: i64) -> Option<String> {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT quality_verdict FROM grouping_file WHERE file_id = ?1",
                    [file],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    /// Changes something `file`'s verdict is made from (a re-read gave
    /// another codec and bitrate; each call another bitrate), so the stored
    /// one is out of date until it's settled. Settled, it says what it said.
    fn make_verdict_out_of_date(&self, file: i64) {
        self.sql(&format!(
            "UPDATE file SET codec = 'mp3', bitrate = coalesce(bitrate, 319) + 1 WHERE id = {file}"
        ));
    }

    fn make_due(&self, file: i64) {
        self.sql(&format!(
            "UPDATE grouping_file SET decided_as = NULL, rev = random() WHERE file_id = {file}"
        ));
    }

    fn any_stale_verdict(&self) -> bool {
        self.writer
            .call(|c| quality::store::any_stale(c, None))
            .unwrap()
    }
}

// ---- the rule ----------------------------------------------------------------

#[test]
fn a_cut_off_copy_inside_a_same_named_file_joins_its_track() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.read_as_cut_short(copy, true);
    db.settle();
    let (_, summary) = db.decide();
    assert_eq!(db.track(whole), db.track(copy));
    assert_eq!(db.tracks(), 1);
    assert_eq!(
        db.stands(whole, copy),
        ("duplicate".into(), "merged".into())
    );
    let reasons = db.reasons(whole, copy);
    assert!(reasons.contains("truncatedCopy"), "{reasons}");
    assert!(!reasons.contains("audioInsideOther"), "{reasons}");
    assert_eq!(summary.moved, 1);
    assert_eq!(db.count("SELECT count(*) FROM version_link"), 0);
}

#[test]
fn a_copy_nothing_calls_truncated_is_the_cut_link_it_was() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.decide();
    assert_ne!(db.track(whole), db.track(copy));
    assert_eq!(db.stands(whole, copy), ("cut".into(), "linked".into()));
    assert!(db.reasons(whole, copy).contains("audioInsideOther"));
    assert!(db.link(whole, copy).is_some());
}

#[test]
fn a_truncated_whole_file_does_not_make_the_short_one_its_duplicate() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.read_as_cut_short(whole, true);
    db.decide();
    assert_eq!(
        db.verdict(whole),
        Some(quality::verdict::Verdict::Truncated)
    );
    assert_ne!(db.track(whole), db.track(copy));
    assert_eq!(db.stands(whole, copy), ("cut".into(), "linked".into()));
}

#[test]
fn a_truncated_file_that_is_not_the_others_beginning_stays_a_cut_link() {
    let db = db();
    let velo = track(1, LEN);
    let whole = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    // The middle of it: what a radio edit without the intro holds.
    let middle = db.add(file(
        "Velo.mp3",
        "Velo",
        2,
        reencoded(&velo[300..700], 3, 7),
    ));
    db.read_as_cut_short(middle, true);
    db.decide();
    assert_ne!(db.track(whole), db.track(middle));
    assert_eq!(db.stands(whole, middle), ("cut".into(), "linked".into()));
    assert!(db.reasons(whole, middle).contains("audioInsideOther"));
}

#[test]
fn a_truncated_radio_edit_or_clean_cut_named_as_one_is_never_merged() {
    let db = db();
    let velo = track(1, LEN);
    let whole = db.add(file("Velo (Dirty).flac", "Velo (Dirty)", 1, velo.clone()));
    let clean = db.add(file(
        "Velo (Clean).mp3",
        "Velo (Clean)",
        2,
        cut_off(&velo, 400, 7),
    ));
    let edit = db.add(file(
        "Velo (Radio Edit).mp3",
        "Velo (Radio Edit)",
        3,
        cut_off(&velo, 420, 8),
    ));
    db.read_as_cut_short(clean, true);
    db.read_as_cut_short(edit, true);
    db.decide();
    assert_eq!(db.tracks(), 3);
    assert_eq!(db.stands(whole, clean).0, "cut");
    assert_eq!(db.stands(whole, edit).0, "cut");
    for other in [clean, edit] {
        let reasons = db.reasons(whole, other);
        assert!(!reasons.contains("truncatedCopy"), "{reasons}");
    }
}

// ---- reading the verdict -------------------------------------------------------

#[test]
fn the_deciding_job_settles_verdicts_before_it_decides() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.read_as_cut_short(copy, true);
    // No quality job has been through: the stored verdict says nothing.
    assert_eq!(db.verdict(copy), None);
    db.decide();
    assert_eq!(db.verdict(copy), Some(quality::verdict::Verdict::Truncated));
    assert_eq!(db.track(whole), db.track(copy));
    assert!(!db.any_stale_verdict());
}

#[test]
fn a_deciding_job_with_no_file_due_leaves_the_verdicts_as_they_are() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.decide();
    assert!(!db.any_stale_verdict());
    // The read stage has been through the copy again; no quality job has.
    db.read_as_cut_short(copy, true);
    assert!(db.any_stale_verdict());
    let before = db.dump();
    db.decide();
    assert!(db.any_stale_verdict(), "nothing was due: nothing settled");
    assert_eq!(db.dump(), before);
    assert_ne!(db.track(whole), db.track(copy));
}

#[test]
fn a_decision_is_made_on_verdicts_brought_up_to_date_where_they_are_read() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.read_as_cut_short(copy, true);
    db.settle();
    // What the verdict was made from changes, and nothing settles it: as
    // stored, the copy has no verdict.
    db.make_verdict_out_of_date(copy);
    assert_eq!(db.verdict(copy), None);
    db.group();
    db.compare_fingerprints();
    // The run alone: no job in front of it to settle anything.
    db.decide_in_pages(PAGE);
    db.group();
    assert_eq!(db.verdict(copy), Some(quality::verdict::Verdict::Truncated));
    assert_eq!(db.track(whole), db.track(copy));
    assert_eq!(db.stands(whole, copy).0, "duplicate");
    assert_eq!(db.decided_at(copy).as_deref(), Some("truncated"));
}

#[test]
fn a_stale_verdict_makes_no_file_due_and_flips_no_pair() {
    // Decided as a cut-off copy; then the copy's verdict goes out of date
    // (a re-read gave another codec and bitrate) and nothing settles it. No
    // file is due, and a deciding job runs anyway.
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.read_as_cut_short(copy, true);
    db.decide();
    assert_eq!(
        db.stands(whole, copy),
        ("duplicate".into(), "merged".into())
    );
    let before = db.dump();
    db.make_verdict_out_of_date(copy);
    assert_eq!(db.verdict(copy), None, "as stored, it reads as none");
    let (decided, summary) = db.deciding_job();
    assert_eq!(decided.files, 0, "no file was made due");
    assert!(!summary.changed_tracks());
    assert_eq!(db.decided(), [whole, copy]);
    assert_eq!(
        db.stands(whole, copy),
        ("duplicate".into(), "merged".into())
    );
    assert!(db.reasons(whole, copy).contains("truncatedCopy"));
    assert_eq!(db.decided_at(copy).as_deref(), Some("truncated"));
    assert_eq!(db.dump(), before);
    // Nor does the end of a quality job that didn't reach the file.
    db.make_verdict_out_of_date(copy);
    assert!(!db.quality_job_ended(), "no deciding job is asked for");
    assert_eq!(db.dump(), before);

    // Two Library tracks: the pair is held for the user, and stays held.
    // No version link is made between the two tracks, and so none is
    // removed again later.
    let db = super::support::db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.group();
    db.to_library(whole);
    db.to_library(copy);
    db.read_as_cut_short(copy, true);
    db.decide();
    let held = ("duplicate".to_string(), "both_in_library".to_string());
    assert_eq!(db.stands(whole, copy), held);
    let before = db.dump();
    db.make_verdict_out_of_date(copy);
    db.deciding_job();
    assert_eq!(db.stands(whole, copy), held);
    assert_eq!(db.count("SELECT count(*) FROM version_link"), 0);
    assert_eq!(db.dump(), before);
    db.make_verdict_out_of_date(copy);
    assert!(!db.quality_job_ended());
    db.decide();
    assert_eq!(db.count("SELECT count(*) FROM version_link"), 0);
    assert_eq!(db.dump(), before);
}

#[test]
fn a_cut_off_copy_whose_file_name_carries_another_version_word_is_not_merged() {
    // The final guard, through the database: both files are tagged "Velo",
    // and only their file names say Clean and Dirty.
    let db = db();
    let velo = track(1, LEN);
    let whole = db.add(file("Velo Dirty - Artist A.flac", "Velo", 1, velo.clone()));
    let copy = db.add(file(
        "Velo Clean - Artist A.mp3",
        "Velo",
        2,
        cut_off(&velo, 400, 7),
    ));
    db.read_as_cut_short(copy, true);
    db.decide();
    assert_ne!(db.track(whole), db.track(copy));
    assert_eq!(db.stands(whole, copy), ("cut".into(), "linked".into()));
    let reasons = db.reasons(whole, copy);
    assert!(reasons.contains("audioInsideOther"), "{reasons}");
    assert!(!reasons.contains("truncatedCopy"), "{reasons}");
    // The same two files with the same word in both names are one track.
    let db = super::support::db();
    let whole = db.add(file("Velo Clean - Artist A.flac", "Velo", 1, velo.clone()));
    let copy = db.add(file(
        "Velo Clean - Artist A.mp3",
        "Velo",
        2,
        cut_off(&velo, 400, 7),
    ));
    db.read_as_cut_short(copy, true);
    db.decide();
    assert_eq!(db.track(whole), db.track(copy));
    assert!(db.reasons(whole, copy).contains("truncatedCopy"));
}

#[test]
fn the_verdict_a_file_was_decided_at_is_recorded() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    let low = db.add(file("Nightdrive.mp3", "Nightdrive", 3, track(2, LEN)));
    db.read_as_cut_short(copy, true);
    db.sql(&format!(
        "UPDATE file SET codec = 'mp3', bitrate = 128 WHERE id = {low}"
    ));
    db.decide();
    assert_eq!(db.decided_at(whole), None);
    assert_eq!(db.decided_at(copy).as_deref(), Some("truncated"));
    assert_eq!(db.decided_at(low).as_deref(), Some("low_bitrate"));
}

// ---- a verdict that arrives later ------------------------------------------------

#[test]
fn a_file_that_turns_truncated_is_due_again_and_keeps_its_pairs() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.decide();
    assert_eq!(db.stands(whole, copy), ("cut".into(), "linked".into()));
    assert_eq!(db.decided(), [whole, copy]);
    // The quality job finishes after the deciding job did.
    db.read_as_cut_short(copy, true);
    db.settle();
    assert!(db.quality_job_ended(), "a deciding job is asked for");
    assert_eq!(db.decided(), [whole], "the copy is due again");
    // Nothing else moved: the pair and its link are there until the copy
    // is decided again.
    assert_eq!(db.stands(whole, copy), ("cut".into(), "linked".into()));
    assert!(db.link(whole, copy).is_some());
    assert_ne!(db.track(whole), db.track(copy));

    db.deciding_job();
    assert_eq!(db.track(whole), db.track(copy));
    assert_eq!(
        db.stands(whole, copy),
        ("duplicate".into(), "merged".into())
    );
    assert_eq!(db.count("SELECT count(*) FROM version_link"), 0);
    assert_eq!(db.decided(), [whole, copy]);
    assert!(!db.quality_job_ended(), "nothing is left to ask for");
}

#[test]
fn a_file_that_turned_truncated_is_found_by_the_next_deciding_job_too() {
    // No quality job's end said so (it failed after it stored the
    // verdict): the next deciding job that has a file to decide finds it.
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.decide();
    db.read_as_cut_short(copy, true);
    db.add(file("Nightdrive.flac", "Nightdrive", 3, track(2, LEN)));
    db.decide();
    assert_eq!(db.track(whole), db.track(copy));
    assert_eq!(
        db.stands(whole, copy),
        ("duplicate".into(), "merged".into())
    );
}

#[test]
fn a_verdict_change_without_truncation_makes_no_file_due() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.decide();
    let before = db.dump();
    // Every file gets its first verdict, and none is "truncated".
    db.sql("UPDATE file SET codec = 'mp3', bitrate = 128");
    db.settle();
    assert_eq!(
        db.verdict(copy),
        Some(quality::verdict::Verdict::LowBitrate)
    );
    assert!(!db.quality_job_ended(), "no deciding job is asked for");
    assert_eq!(db.decided(), [whole, copy]);
    assert_eq!(db.dump(), before);
    db.decide();
    assert_eq!(db.dump(), before);
}

#[test]
fn a_cut_off_copy_is_a_duplicate_whichever_of_the_two_files_is_due() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.decide();
    db.read_as_cut_short(copy, true);
    db.settle();
    // Only the whole file is decided again; the copy is its partner.
    db.make_due(whole);
    db.decide_in_pages(PAGE);
    assert_eq!(db.stands(whole, copy).0, "duplicate");
    assert!(db.reasons(whole, copy).contains("truncatedCopy"));
    // The copy's own record still says what it was decided at: it's
    // decided once more, to the same answer, and then nothing is due.
    assert!(db.quality_job_ended());
    db.deciding_job();
    assert_eq!(db.track(whole), db.track(copy));
    let after = db.dump();
    assert!(!db.quality_job_ended());
    db.decide();
    assert_eq!(db.dump(), after);
}

#[test]
fn after_the_rules_change_every_file_is_decided_once_with_its_verdict() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.decide();
    // As the rules before this one left it: decided under another version,
    // with no verdict in any record. The verdict has arrived since.
    let older = crate::grouping::decided_as() - (1 << 16);
    db.sql(&format!(
        "UPDATE grouping_file SET decided_as = {older}, quality_verdict = NULL"
    ));
    db.read_as_cut_short(copy, true);
    db.settle();
    let revs = || db.count("SELECT sum(rev) FROM grouping_file");
    let before = revs();
    // Every file is due already: none is made due a second way.
    assert_eq!(
        db.writer
            .call(decide::due_again_where_truncation_changed)
            .unwrap(),
        0
    );
    assert_eq!(revs(), before);
    let (decided, _) = db.decide();
    assert_eq!(decided.files, 2);
    assert_eq!(db.track(whole), db.track(copy));
    assert_eq!(db.decided_at(copy).as_deref(), Some("truncated"));
    let after = db.dump();
    db.decide();
    assert_eq!(db.dump(), after);
}

// ---- the same again ----------------------------------------------------------------

#[test]
fn a_rerun_after_a_settle_changes_nothing() {
    let db = db();
    let (_, copy) = db.velo_and_its_cut_off_copy();
    db.add(file("Nightdrive.flac", "Nightdrive", 3, track(2, LEN)));
    db.read_as_cut_short(copy, true);
    db.decide();
    let before = db.dump();
    for run in 0..2 {
        db.settle();
        assert!(!db.quality_job_ended(), "run {run}");
        let (decided, summary) = db.decide();
        assert_eq!(decided.files, 0, "run {run}");
        assert!(!summary.changed_tracks(), "run {run}");
        assert_eq!(db.dump(), before, "run {run}");
    }
}

#[test]
fn a_rerun_after_a_late_verdict_was_taken_in_changes_nothing() {
    let db = db();
    let (_, copy) = db.velo_and_its_cut_off_copy();
    db.decide();
    db.read_as_cut_short(copy, true);
    db.settle();
    assert!(db.quality_job_ended());
    db.deciding_job();
    let before = db.dump();
    for run in 0..2 {
        db.settle();
        assert!(!db.quality_job_ended(), "run {run}");
        db.decide();
        assert_eq!(db.dump(), before, "run {run}");
    }
}

// ---- what the job never does -----------------------------------------------------

#[test]
fn a_merge_on_a_truncated_verdict_stays_when_the_verdict_goes() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.read_as_cut_short(copy, true);
    db.decide();
    assert_eq!(db.track(whole), db.track(copy));
    // The file reads as whole after all.
    db.read_as_cut_short(copy, false);
    db.settle();
    assert!(db.quality_job_ended(), "the copy is decided again");
    db.deciding_job();
    assert_eq!(db.track(whole), db.track(copy), "the track stays whole");
    assert_eq!(db.tracks(), 1);
    assert_eq!(
        db.stands(whole, copy),
        ("cut".into(), "in_one_track".into())
    );
    assert_eq!(db.count("SELECT count(*) FROM version_link"), 0);
}

#[test]
fn two_library_tracks_are_not_merged_by_a_cut_off_copy() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.group();
    let (track_whole, track_copy) = (db.to_library(whole), db.to_library(copy));
    db.read_as_cut_short(copy, true);
    db.decide();
    assert_eq!(
        (db.track(whole), db.track(copy)),
        (Some(track_whole), Some(track_copy))
    );
    assert_eq!(
        db.stands(whole, copy),
        ("duplicate".into(), "both_in_library".into())
    );
    assert!(db.reasons(whole, copy).contains("truncatedCopy"));
    assert_eq!(db.count("SELECT count(*) FROM library_track"), 2);
}

#[test]
fn an_apart_decision_outlives_a_truncated_verdict() {
    let db = db();
    let (whole, copy) = db.velo_and_its_cut_off_copy();
    db.decide();
    let said = db
        .writer
        .call(move |c| remember(c, whole, copy, Verdict::Apart))
        .unwrap();
    assert_eq!(said, Ok(()));
    db.read_as_cut_short(copy, true);
    db.settle();
    db.quality_job_ended();
    for run in 0..2 {
        db.decide();
        assert_ne!(db.track(whole), db.track(copy), "run {run}");
        assert_eq!(
            db.stands(whole, copy),
            ("duplicate".into(), "decided".into()),
            "run {run}"
        );
    }
    assert_eq!(db.tracks(), 2);
}

#[test]
fn a_set_with_a_cut_off_copy_merges_only_if_every_pair_is_a_duplicate() {
    // The whole file under two artists (duplicates on full audio, with a
    // note), and a cut-off copy by the first: it is a duplicate of the
    // first, and a cut of the second, whose artist differs.
    let db = db();
    let velo = track(1, LEN);
    let first = db.add(by("Artist A", "Velo.flac", "Velo", 1, velo.clone()));
    let second = db.add(by(
        "Artist B",
        "other/Velo.flac",
        "Velo",
        2,
        reencoded(&velo, 1, 5),
    ));
    let copy = db.add(by(
        "Artist A",
        "Velo.mp3",
        "Velo",
        3,
        cut_off(&velo, 400, 7),
    ));
    db.read_as_cut_short(copy, true);
    db.decide();
    assert_eq!(db.stands(first, second).0, "duplicate");
    assert_eq!(db.stands(first, copy).0, "duplicate");
    assert_eq!(db.stands(second, copy).0, "cut");
    assert_eq!(db.tracks(), 3, "one pair of the set is no duplicate");
    assert_eq!(db.stands(first, copy).1, "set_disagrees");

    // With every pair a duplicate, the three are one track.
    let db = super::support::db();
    let first = db.add(file("Velo.flac", "Velo", 1, velo.clone()));
    let second = db.add(file("other/Velo.flac", "Velo", 2, reencoded(&velo, 1, 5)));
    let copy = db.add(file("Velo.mp3", "Velo", 3, cut_off(&velo, 400, 7)));
    db.read_as_cut_short(copy, true);
    db.decide();
    assert_eq!(db.tracks(), 1);
    assert_eq!(db.track(first), db.track(copy));
    assert_eq!(db.track(second), db.track(copy));
}

// ---- the chain: what follows a quality job ---------------------------------------

mod chain {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::grouping::Grouper;
    use crate::jobs::{self, JobKind, JobQueue, JobStatus, NewJob};
    use crate::scan::chain::{after_group, after_quality};

    /// Runs a quality job that settles the verdicts (what the real one does
    /// to the database, without decoding audio), with the real chain after
    /// it and the real grouping jobs, until nothing is queued or running.
    /// Returns how many deciding jobs ran.
    fn run_a_quality_job(db: &Db) -> i64 {
        let queue = JobQueue::builder(db.writer.clone())
            .handler(
                JobKind::Quality,
                after_quality(|job: &jobs::JobContext| {
                    job.writer().call(quality::settle)?;
                    Ok(())
                }),
            )
            .handler(JobKind::Group, after_group(Grouper::default()))
            .start()
            .unwrap();
        let id = queue.enqueue(NewJob::new(JobKind::Quality)).unwrap();
        // Long enough that a passing test never hits it.
        let begun = Instant::now();
        while !queue.activity().unwrap().jobs.is_empty() {
            assert!(
                begun.elapsed() < Duration::from_secs(30),
                "the jobs never finished"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        queue.shutdown();
        let job = db
            .writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        assert_eq!(job.status, JobStatus::Done, "{job:?}");
        db.count("SELECT count(*) FROM job WHERE kind = 'group' AND target IS NOT NULL")
    }

    #[test]
    fn a_quality_job_that_finds_a_cut_off_copy_asks_for_a_deciding_job() {
        let db = db();
        let (whole, copy) = db.velo_and_its_cut_off_copy();
        db.decide();
        assert_eq!(db.stands(whole, copy), ("cut".into(), "linked".into()));
        // The quality job comes after the deciding job, and finds the copy
        // cut short.
        db.read_as_cut_short(copy, true);
        assert_eq!(run_a_quality_job(&db), 1);
        assert_eq!(db.track(whole), db.track(copy));
        assert_eq!(
            db.stands(whole, copy),
            ("duplicate".into(), "merged".into())
        );
        // The next quality job has nothing to say.
        assert_eq!(run_a_quality_job(&db), 1, "no second deciding job");
    }

    #[test]
    fn a_quality_job_that_finds_no_file_cut_short_asks_for_no_deciding_job() {
        let db = db();
        let (whole, copy) = db.velo_and_its_cut_off_copy();
        db.decide();
        let before = db.dump();
        // Every file gets a verdict, none of them "truncated".
        db.sql("UPDATE file SET codec = 'mp3', bitrate = 128");
        assert_eq!(run_a_quality_job(&db), 0);
        assert_eq!(
            db.verdict(copy),
            Some(quality::verdict::Verdict::LowBitrate)
        );
        assert_eq!(db.dump(), before);
        assert_ne!(db.track(whole), db.track(copy));
    }
}
