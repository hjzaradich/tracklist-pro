//! The matching pass and its stored results (migration 0016): only
//! candidates are compared, results are kept, a changed fingerprint drops
//! them, and a pass after one new file works on that file alone.

use std::collections::BTreeMap;

use rusqlite::params;

use crate::db::{DbError, Writer};
use crate::fingerprint::Fingerprint;
use crate::matching::store::{self, Side};
use crate::matching::{compare, refresh, Comparison, Matcher, StoredMatch, Summary};

use super::corpus::corpus;
use super::reference::PlainIndex;
use super::synthetic::{fingerprint, reencoded, track};

/// A migrated database in a temp dir with one music folder.
pub(super) struct Db {
    pub(super) writer: Writer,
    _dir: tempfile::TempDir,
}

pub(super) fn db() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    writer
        .call(|c| {
            c.execute_batch(
                "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');",
            )
        })
        .unwrap();
    Db { writer, _dir: dir }
}

impl Db {
    /// Adds a present file named `name` holding `fingerprint`. Returns its
    /// id.
    fn add(&self, name: &str, fingerprint: &Fingerprint) -> i64 {
        let (name, blob) = (name.to_string(), fingerprint.to_blob());
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime, fingerprint)
                     VALUES (1, ?1, ?1, 1000, 1000, ?2)",
                    params![name, blob],
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    pub(super) fn add_items(&self, name: &str, items: Vec<u32>) -> i64 {
        self.add(name, &fingerprint(items))
    }

    /// Gives file `id` another fingerprint, as the fingerprint job would
    /// after its audio changed.
    fn set_fingerprint(&self, id: i64, fingerprint: &Fingerprint) {
        let blob = fingerprint.to_blob();
        self.sql("UPDATE file SET fingerprint = ?1 WHERE id = ?2", (blob, id));
    }

    fn sql<P: rusqlite::Params + Send + 'static>(&self, sql: &'static str, params: P) {
        self.writer
            .call(move |c| c.execute(sql, params).map(|_| ()))
            .unwrap();
    }

    fn all(&self) -> Vec<StoredMatch> {
        self.writer.call(|c| store::all(c)).unwrap()
    }

    pub(super) fn pairs(&self) -> Vec<(i64, i64)> {
        self.all().iter().map(|m| (m.file_a, m.file_b)).collect()
    }

    fn of_pair(&self, a: i64, b: i64) -> Option<Comparison> {
        self.writer.call(move |c| store::of_pair(c, a, b)).unwrap()
    }
}

fn pass(matcher: &Matcher, db: &Db) -> Summary {
    matcher
        .pass::<DbError>(&db.writer, &mut |_| Ok(()))
        .unwrap()
}

/// Three unrelated tracks; the first also has a re-encoded copy and a cut.
/// Ids: 1 `a`, 2 `a` re-encoded, 3 a cut of `a`, 4 `b`, 5 `c`.
fn small() -> Db {
    let db = db();
    let a = track(1, 900);
    db.add_items("a.flac", a.clone());
    db.add_items("a.mp3", reencoded(&a, 1, 7));
    db.add_items("a (radio edit).flac", a[300..700].to_vec());
    db.add_items("b.flac", track(2, 900));
    db.add_items("c.flac", track(3, 900));
    db
}

#[test]
fn a_pass_stores_a_result_for_each_candidate_pair_and_never_compares_the_rest() {
    let db = small();
    let summary = refresh(&db.writer).unwrap();
    assert_eq!(db.pairs(), [(1, 2), (1, 3), (2, 3)]);
    assert_eq!(
        summary,
        Summary {
            files: 5,
            fingerprints: 5,
            changed: 5,
            candidates: 3,
            already_compared: 0,
            compared: 3,
            stored: 3,
        }
    );
    // What's stored is what comparing the two fingerprints gives.
    let copy = db.of_pair(1, 2).unwrap();
    assert!(
        copy.coverage_a >= 0.9 && copy.coverage_b >= 0.9 && copy.score <= 4.0,
        "{copy:?}"
    );
    let cut = db.of_pair(3, 1).unwrap();
    assert!(cut.coverage_a >= 0.9 && cut.coverage_b < 0.5, "{cut:?}");
    assert_eq!(cut.segments[0].offset_b - cut.segments[0].offset_a, 300);
    let a = track(1, 900);
    let expected = compare(&fingerprint(a.clone()), &fingerprint(a[300..700].to_vec())).unwrap();
    assert_eq!(db.of_pair(1, 3).unwrap(), expected);
    // Asked the other way round, the same result with its sides swapped.
    assert_eq!(cut, expected.swapped());
    assert_eq!(
        db.of_pair(4, 5),
        None,
        "unrelated tracks were never compared"
    );
    let of_cut = db.writer.call(|c| store::of_file(c, 3)).unwrap();
    assert_eq!(of_cut.len(), 2);
}

#[test]
fn a_second_pass_with_nothing_new_compares_nothing_even_after_a_restart() {
    let db = small();
    let matcher = Matcher::new();
    pass(&matcher, &db);
    let again = pass(&matcher, &db);
    assert_eq!(
        (
            again.changed,
            again.candidates,
            again.compared,
            again.stored
        ),
        (0, 0, 0, 0)
    );
    // A new matcher (the app restarted) reads every fingerprint again, but
    // every candidate pair already has its result.
    let restarted = refresh(&db.writer).unwrap();
    assert_eq!(restarted.changed, 5);
    assert_eq!((restarted.candidates, restarted.already_compared), (3, 3));
    assert_eq!((restarted.compared, restarted.stored), (0, 0));
    assert_eq!(db.pairs().len(), 3);
}

#[test]
fn a_pass_after_one_new_file_compares_only_that_files_candidates() {
    let db = small();
    let matcher = Matcher::new();
    pass(&matcher, &db);
    let new = db.add_items("b (128).mp3", reencoded(&track(2, 900), 9, 7));
    let summary = pass(&matcher, &db);
    assert_eq!(
        summary,
        Summary {
            files: 6,
            fingerprints: 6,
            changed: 1,
            candidates: 1,
            already_compared: 0,
            compared: 1,
            stored: 1,
        }
    );
    assert_eq!(db.pairs(), [(1, 2), (1, 3), (2, 3), (4, new)]);
}

#[test]
fn a_changed_fingerprint_drops_its_stored_results_and_the_next_pass_measures_the_new_audio() {
    let db = small();
    let matcher = Matcher::new();
    pass(&matcher, &db);
    // File 2 now holds other audio: a copy of `b`.
    db.set_fingerprint(2, &fingerprint(reencoded(&track(2, 900), 4, 7)));
    assert_eq!(
        db.pairs(),
        [(1, 3)],
        "both of file 2's results went with its fingerprint"
    );
    let summary = pass(&matcher, &db);
    assert_eq!(
        (summary.changed, summary.compared, summary.stored),
        (1, 1, 1)
    );
    assert_eq!(db.pairs(), [(1, 3), (2, 4)]);
}

#[test]
fn a_fingerprint_taken_away_drops_its_results_and_the_file_leaves_the_index() {
    let db = small();
    let matcher = Matcher::new();
    pass(&matcher, &db);
    db.sql("UPDATE file SET fingerprint = NULL WHERE id = 1", ());
    assert_eq!(db.pairs(), [(2, 3)]);
    let summary = pass(&matcher, &db);
    assert_eq!(
        (summary.files, summary.fingerprints, summary.compared),
        (4, 4, 0)
    );
    assert_eq!(db.pairs(), [(2, 3)]);
}

#[test]
fn a_tag_rewrite_or_a_new_modified_time_keeps_the_results_and_compares_nothing() {
    // rekordbox rewrites tags and bumps modified times (ROADMAP 5.1); the
    // fingerprint stage then carries the fingerprint forward, or decodes
    // the file again and stores the same bytes. Neither is new audio.
    let db = small();
    let matcher = Matcher::new();
    pass(&matcher, &db);
    let before = db.all();
    db.sql(
        "UPDATE file SET size = size + 4096, mtime = mtime + 5000000",
        (),
    );
    let same = fingerprint(track(1, 900)).to_blob();
    db.sql("UPDATE file SET fingerprint = ?1 WHERE id = 1", (same,));
    assert_eq!(db.all(), before);
    let summary = pass(&matcher, &db);
    assert_eq!(
        (summary.changed, summary.compared, summary.stored),
        (0, 0, 0)
    );
    assert_eq!(db.all(), before);
}

#[test]
fn a_file_that_goes_missing_keeps_its_results_but_is_not_compared_with_new_files() {
    let db = small();
    let matcher = Matcher::new();
    pass(&matcher, &db);
    db.sql("UPDATE file SET present = 0 WHERE id = 2", ());
    let new = db.add_items("a (320).mp3", reencoded(&track(1, 900), 8, 7));
    let summary = pass(&matcher, &db);
    assert_eq!(summary.files, 5);
    // Compared with the present copy and the cut, not with the missing one.
    assert_eq!(db.pairs(), [(1, 2), (1, 3), (1, new), (2, 3), (3, new)]);
}

#[test]
fn deleting_a_file_row_deletes_its_results() {
    let db = small();
    refresh(&db.writer).unwrap();
    db.sql("DELETE FROM file WHERE id = 3", ());
    assert_eq!(db.pairs(), [(1, 2)]);
}

#[test]
fn files_with_the_very_same_fingerprint_are_matched_without_being_compared() {
    // The same audio as FLAC and WAV decodes to the same samples, so the
    // fingerprints are identical bytes: one index entry. The first file
    // stands for all of them.
    let db = db();
    let a = track(1, 900);
    for name in ["a.flac", "a.wav", "a (backup).wav"] {
        db.add_items(name, a.clone());
    }
    db.add_items("a.mp3", reencoded(&a, 1, 7));
    db.add_items("b.flac", track(2, 900));
    let summary = refresh(&db.writer).unwrap();
    assert_eq!((summary.files, summary.fingerprints), (5, 3));
    assert_eq!(
        (summary.candidates, summary.compared, summary.stored),
        (1, 1, 3)
    );
    // 1–2 and 1–3 are exact; the MP3 is compared with file 1 only.
    assert_eq!(db.pairs(), [(1, 2), (1, 3), (1, 4)]);
    for other in [2, 3] {
        let exact = db.of_pair(1, other).unwrap();
        assert_eq!(exact, Comparison::identical(900));
        assert_eq!(
            (exact.coverage_a, exact.coverage_b, exact.score),
            (1.0, 1.0, 0.0)
        );
    }
}

#[test]
fn when_the_file_standing_for_identical_ones_changes_the_next_one_takes_over() {
    let db = db();
    let a = track(1, 900);
    db.add_items("a.flac", a.clone());
    db.add_items("a.wav", a.clone());
    db.add_items("a (backup).wav", a.clone());
    db.add_items("a.mp3", reencoded(&a, 1, 7));
    let matcher = Matcher::new();
    pass(&matcher, &db);
    assert_eq!(db.pairs(), [(1, 2), (1, 3), (1, 4)]);
    // File 1 becomes something else: files 2 and 3 are still identical,
    // and file 2 now stands for them against the MP3.
    db.set_fingerprint(1, &fingerprint(track(7, 900)));
    assert_eq!(db.pairs(), []);
    pass(&matcher, &db);
    assert_eq!(db.pairs(), [(2, 3), (2, 4)]);
}

#[test]
fn a_result_is_not_stored_if_either_fingerprint_changed_while_it_was_being_compared() {
    let db = small();
    let (a, cut) = (
        fingerprint(track(1, 900)),
        fingerprint(track(1, 900)[300..700].to_vec()),
    );
    let comparison = compare(&a, &cut).unwrap();
    let (blob_a, blob_cut) = (a.to_blob(), cut.to_blob());
    // File 3's fingerprint changes after it was read for comparing.
    db.set_fingerprint(3, &fingerprint(track(9, 400)));
    let stored = db
        .writer
        .call(move |c| {
            let (a, b) = (
                Side {
                    file: 1,
                    blob: &blob_a,
                },
                Side {
                    file: 3,
                    blob: &blob_cut,
                },
            );
            store::put(c, a, b, &comparison)
        })
        .unwrap();
    assert!(!stored);
    assert_eq!(db.pairs(), []);
}

#[test]
fn a_pass_stopped_halfway_keeps_what_it_stored_and_the_next_pass_finishes() {
    let db = small();
    let matcher = Matcher::new();
    // Stop at the third progress report, as a cancelled job would.
    let mut ticks = 0;
    let stopped = matcher.pass(&db.writer, &mut |_| {
        ticks += 1;
        if ticks > 3 {
            Err(DbError::WriterGone)
        } else {
            Ok(())
        }
    });
    assert!(stopped.is_err());
    assert!(
        db.pairs().len() < 3,
        "it really stopped early: {:?}",
        db.pairs()
    );
    let summary = pass(&matcher, &db);
    assert_eq!(summary.changed, 0, "nothing is read twice");
    assert_eq!(db.pairs(), [(1, 2), (1, 3), (2, 3)]);
}

#[test]
fn results_of_another_version_of_the_comparison_are_worked_out_again() {
    let db = small();
    refresh(&db.writer).unwrap();
    db.sql(
        "UPDATE fingerprint_match SET version = version + 1, score = 31.0",
        (),
    );
    assert_eq!(db.pairs(), [], "another version's rows are never read");
    let summary = refresh(&db.writer).unwrap();
    assert_eq!((summary.compared, summary.stored), (3, 3));
    assert!(db.all().iter().all(|m| m.comparison.score < 4.0));
}

#[test]
fn a_fingerprint_this_build_cannot_read_is_left_out() {
    let db = small();
    db.sql("UPDATE file SET fingerprint = x'00010203' WHERE id = 3", ());
    let summary = refresh(&db.writer).unwrap();
    assert_eq!((summary.files, summary.compared), (4, 1));
    assert_eq!(db.pairs(), [(1, 2)]);
}

#[test]
fn it_works_as_a_job_and_reports_progress() {
    use crate::jobs::{self, JobKind, JobQueue, JobStatus, NewJob};

    let db = small();
    // No job kind is wired for matching yet: the test borrows one.
    let queue = JobQueue::builder(db.writer.clone())
        .workers(1)
        .handler(JobKind::Analyze, Matcher::new())
        .start()
        .unwrap();
    let id = queue.enqueue(NewJob::new(JobKind::Analyze)).unwrap();
    let started = std::time::Instant::now();
    let job = loop {
        assert!(started.elapsed().as_secs() < 300, "the job never finished");
        let job = db
            .writer
            .call(move |c| jobs::store::get(c, id))
            .unwrap()
            .unwrap();
        if job.status.is_finished() {
            break job;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    queue.shutdown();
    assert_eq!(job.status, JobStatus::Done, "{job:?}");
    assert_eq!(job.progress, Some(1.0));
    assert_eq!(db.pairs(), [(1, 2), (1, 3), (2, 3)]);
}

#[test]
fn on_the_ground_truth_library_every_duplicate_is_measured_as_one_and_no_other_pair_is() {
    // The whole way through: real fingerprints of generated audio in the
    // file table, one pass, then ROADMAP 1.4's rule on the stored numbers.
    let db = db();
    let mut ids: BTreeMap<&str, i64> = BTreeMap::new();
    for file in &corpus().files {
        ids.insert(file.name, db.add(file.name, &file.fingerprint));
    }
    let summary = refresh(&db.writer).unwrap();
    println!("{summary:?}");
    let stored = db.all();
    // Identical fingerprints are linked to their first file, which holds
    // the results for all of them.
    let stands_for = |id: i64| {
        stored
            .iter()
            .find(|m| m.file_b == id && m.comparison == Comparison::identical(m.comparison.items_a))
            .map_or(id, |m| m.file_a)
    };
    let measured = |a: &str, b: &str| -> Option<Comparison> {
        let (a, b) = (stands_for(ids[a]), stands_for(ids[b]));
        if a == b {
            let items = stored.iter().find(|m| m.file_a == a)?.comparison.items_a;
            return Some(Comparison::identical(items));
        }
        db.of_pair(a, b)
    };
    let is_duplicate =
        |c: &Comparison| c.coverage_a >= 0.9 && c.coverage_b >= 0.9 && c.score <= 4.0;

    for (a, b) in corpus().duplicate_pairs() {
        let c = measured(a, b).unwrap_or_else(|| panic!("{a} and {b} were never compared"));
        assert!(is_duplicate(&c), "{a} vs {b}: {c:?}");
    }
    for contains in &corpus().contains {
        let c = measured(contains.inner, contains.outer)
            .unwrap_or_else(|| panic!("{} was never compared", contains.inner));
        assert!(
            c.found_a() >= 0.9 && c.found_b() < 0.9,
            "{}: {c:?}",
            contains.inner
        );
    }
    // Besides the duplicates, only Clean/Dirty passes the rule: the version
    // check has to veto that one (ROADMAP 1.4).
    let names: Vec<&str> = corpus().files.iter().map(|f| f.name).collect();
    for (i, a) in names.iter().enumerate() {
        for b in &names[i + 1..] {
            let Some(c) = measured(a, b) else { continue };
            let either_way =
                |pairs: &[(&str, &str)]| pairs.contains(&(*a, *b)) || pairs.contains(&(*b, *a));
            let expected =
                either_way(&corpus().duplicate_pairs()) || either_way(&corpus().clean_dirty);
            assert_eq!(is_duplicate(&c), expected, "{a} vs {b}: {c:?}");
        }
    }
    // Every stored pair is between files the corpus calls related.
    for m in &stored {
        let name = |id: i64| *ids.iter().find(|(_, &v)| v == id).unwrap().0;
        assert!(corpus().related(name(m.file_a), name(m.file_b)), "{m:?}");
    }
}

#[test]
fn a_fingerprint_that_goes_away_and_comes_back_the_same_gets_its_results_back_on_the_next_pass() {
    // A failed decode clears the fingerprint and a later run writes the
    // same bytes again; or the audio changes and changes back. The database
    // dropped the file's results each time, so the same matcher must work
    // them out again, though the fingerprint it remembers is the one it
    // sees.
    let db = small();
    let matcher = Matcher::new();
    pass(&matcher, &db);
    let before = db.all();
    let same = fingerprint(reencoded(&track(1, 900), 1, 7));
    db.sql("UPDATE file SET fingerprint = NULL WHERE id = 2", ());
    db.set_fingerprint(2, &same);
    assert_eq!(db.pairs(), [(1, 3)]);
    let summary = pass(&matcher, &db);
    assert_eq!(summary.changed, 0, "the fingerprint is the one it knew");
    assert_eq!((summary.compared, summary.stored), (2, 2));
    assert_eq!(db.all(), before);

    // Other audio and back again, with no pass in between.
    db.set_fingerprint(3, &fingerprint(track(8, 400)));
    db.set_fingerprint(3, &fingerprint(track(1, 900)[300..700].to_vec()));
    assert_eq!(db.pairs(), [(1, 2)]);
    pass(&matcher, &db);
    assert_eq!(db.all(), before);
    // And nothing more to do after that.
    let settled = pass(&matcher, &db);
    assert_eq!((settled.compared, settled.stored), (0, 0));
}

#[test]
fn an_exact_copy_whose_fingerprint_comes_back_the_same_is_linked_to_the_first_file_again() {
    let db = db();
    let a = track(1, 900);
    db.add_items("a.flac", a.clone());
    db.add_items("a.wav", a.clone());
    let matcher = Matcher::new();
    pass(&matcher, &db);
    assert_eq!(db.pairs(), [(1, 2)]);
    db.sql("UPDATE file SET fingerprint = NULL WHERE id = 2", ());
    db.set_fingerprint(2, &fingerprint(a));
    assert_eq!(db.pairs(), []);
    pass(&matcher, &db);
    assert_eq!(db.pairs(), [(1, 2)]);
    assert_eq!(db.of_pair(1, 2).unwrap(), Comparison::identical(900));
}

#[test]
fn a_cut_added_after_its_original_is_stored_with_each_files_own_coverage() {
    // The original is file 1; the cut arrives later as file 2 and is the
    // one that's due. The row still has the original as A: all of the cut
    // is covered, under half of the original.
    let db = db();
    let a = track(1, 900);
    db.add_items("a.flac", a.clone());
    let matcher = Matcher::new();
    pass(&matcher, &db);
    db.add_items("a (radio edit).flac", a[300..700].to_vec());
    pass(&matcher, &db);
    let stored = db.all();
    assert_eq!(stored.len(), 1);
    let c = &stored[0].comparison;
    assert_eq!((stored[0].file_a, stored[0].file_b), (1, 2));
    assert_eq!((c.items_a, c.items_b), (900, 400));
    assert!(c.coverage_a < 0.5 && c.coverage_b >= 0.9, "{c:?}");
    assert_eq!((c.segments[0].offset_a, c.segments[0].offset_b), (300, 0));
}

#[test]
fn a_result_handed_over_with_the_higher_file_first_is_stored_turned_round() {
    let db = small();
    let (a, cut) = (
        fingerprint(track(1, 900)),
        fingerprint(track(1, 900)[300..700].to_vec()),
    );
    // The cut (file 3) as A, the original (file 1) as B.
    let comparison = compare(&cut, &a).unwrap();
    assert!(comparison.coverage_a >= 0.9 && comparison.coverage_b < 0.5);
    let (blob_a, blob_cut, handed) = (a.to_blob(), cut.to_blob(), comparison.clone());
    let stored = db
        .writer
        .call(move |c| {
            let (cut, original) = (
                Side {
                    file: 3,
                    blob: &blob_cut,
                },
                Side {
                    file: 1,
                    blob: &blob_a,
                },
            );
            store::put(c, cut, original, &handed)
        })
        .unwrap();
    assert!(stored);
    let row = &db.all()[0];
    assert_eq!((row.file_a, row.file_b), (1, 3));
    // The row's A is file 1, the original: its numbers, not the cut's.
    assert_eq!(row.comparison, comparison.swapped());
    assert_eq!((row.comparison.items_a, row.comparison.items_b), (900, 400));
    assert!(row.comparison.coverage_a < 0.5 && row.comparison.coverage_b >= 0.9);
    assert_eq!(row.comparison.segments[0].offset_a, 300);
    assert_eq!(db.of_pair(3, 1).unwrap(), comparison);
}

#[test]
fn the_stored_result_is_the_same_whichever_of_the_two_files_arrived_last() {
    // The matcher isn't symmetric on weak matches: these two give other
    // numbers when their sides are swapped. Two items of the first are
    // added to the second so the index proposes the pair.
    let x = corpus().fingerprint("harbor (extended).flac").clone();
    let shared = crate::matching::index::keys(x.items());
    let mashup = corpus().fingerprint("harbor x moth (mashup).wav");
    let y = fingerprint([mashup.items(), &shared[..2]].concat());
    let (one_way, other_way) = (compare(&x, &y).unwrap(), compare(&y, &x).unwrap().swapped());
    let numbers = |c: &Comparison| (c.coverage_a, c.coverage_b, c.score);
    assert_ne!(
        numbers(&one_way),
        numbers(&other_way),
        "the pair must be a lopsided one"
    );

    // Both files there before the first pass.
    let together = db();
    together.add("x", &x);
    together.add("y", &y);
    refresh(&together.writer).unwrap();
    // The second file arrives after the first was matched, so it alone is
    // due.
    let later = db();
    let matcher = Matcher::new();
    later.add("x", &x);
    pass(&matcher, &later);
    later.add("y", &y);
    pass(&matcher, &later);
    // And the other way round: the lower id is the one that's due.
    let earlier = db();
    let matcher = Matcher::new();
    earlier.add("x", &x);
    earlier.add("y", &y);
    earlier.sql("UPDATE file SET present = 0 WHERE id = 1", ());
    pass(&matcher, &earlier);
    earlier.sql("UPDATE file SET present = 1 WHERE id = 1", ());
    pass(&matcher, &earlier);

    let expected = together.all();
    assert_eq!(expected.len(), 1);
    assert_eq!(
        numbers(&expected[0].comparison),
        numbers(&one_way),
        "the lower id is A"
    );
    assert_eq!(later.all(), expected);
    assert_eq!(earlier.all(), expected);
}

/// What the table should hold for the files present now, worked out with
/// the index as it was before 1bA-13 ([`PlainIndex`]): files with the very
/// same fingerprint are one entry and the first stands for them; every
/// pair the index proposes is compared with the lower file id as A. The
/// rows are stored in a database of their own and read back, so both
/// sides have been through the same storage.
fn rows_by_the_plain_index(db: &Db) -> Vec<StoredMatch> {
    let files = db
        .writer
        .call(|c| store::fingerprints_after(c, 0, 100_000))
        .unwrap();
    let mut same: BTreeMap<Vec<u8>, Vec<i64>> = BTreeMap::new();
    for (id, blob) in &files {
        same.entry(blob.clone()).or_default().push(*id);
    }
    let classes: Vec<(Vec<u8>, Fingerprint, Vec<i64>)> = same
        .into_iter()
        .map(|(blob, ids)| {
            let fingerprint = Fingerprint::from_blob(&blob).unwrap();
            (blob, fingerprint, ids)
        })
        .collect();
    let mut plain = PlainIndex::new();
    // Each row: the two files and what comparing them found.
    let mut rows: Vec<(i64, i64, Comparison)> = Vec::new();
    for (entry, (_, fingerprint, ids)) in classes.iter().enumerate() {
        plain.insert(entry as u32, fingerprint.items());
        let identical = Comparison::identical(fingerprint.items().len());
        for &other in &ids[1..] {
            rows.push((ids[0], other, identical.clone()));
        }
    }
    for (a, b) in plain.pairs() {
        let (mut a, mut b) = (&classes[a as usize], &classes[b as usize]);
        if a.2[0] > b.2[0] {
            std::mem::swap(&mut a, &mut b);
        }
        if let Ok(comparison) = compare(&a.1, &b.1) {
            rows.push((a.2[0], b.2[0], comparison));
        }
    }

    let scratch = self::db();
    scratch
        .writer
        .call(move |c| {
            for (id, blob) in &files {
                c.execute(
                    "INSERT INTO file (id, music_folder_id, rel_path, rel_path_key, size, mtime, fingerprint)
                     VALUES (?1, 1, ?1, ?1, 1000, 1000, ?2)",
                    params![id, blob],
                )?;
            }
            let blobs: BTreeMap<i64, &Vec<u8>> = files.iter().map(|(id, blob)| (*id, blob)).collect();
            for (a, b, comparison) in &rows {
                let side = |file: &i64| Side {
                    file: *file,
                    blob: blobs[file],
                };
                assert!(store::put(c, side(a), side(b), comparison)?);
            }
            Ok(())
        })
        .unwrap();
    scratch.all()
}

#[test]
fn the_stored_rows_are_the_ones_the_index_gave_before_it_was_made_smaller() {
    // 300 made-up tracks that all start with the same silence. Of every
    // 25: two have a re-encoded copy, one a cut, one a copy at the
    // duplicate rule's limit, and one an exact copy.
    const LEN: usize = 500;
    let silence = track(u64::MAX, 20);
    let with_silence = |items: &[u32]| [silence.as_slice(), items].concat();
    let db = db();
    let mut ids: BTreeMap<String, i64> = BTreeMap::new();
    let mut add = |name: String, items: Vec<u32>| {
        let id = db.add_items(&name, with_silence(&items));
        ids.insert(name, id);
    };
    for seed in 1..=300u64 {
        let items = track(seed, LEN);
        add(format!("{seed}.flac"), items.clone());
        match seed % 25 {
            1 | 2 => add(format!("{seed}.mp3"), reencoded(&items, seed, 7)),
            3 => add(
                format!("{seed} (cut).flac"),
                reencoded(&items[LEN / 4..LEN * 3 / 4], seed, 7),
            ),
            4 => add(format!("{seed} (128).mp3"), reencoded(&items, seed, 40)),
            5 => add(format!("{seed}.wav"), items),
            _ => {}
        }
    }

    let matcher = Matcher::new();
    let first = pass(&matcher, &db);
    let expected = rows_by_the_plain_index(&db);
    assert!(expected.len() >= 48, "{} rows", expected.len());
    assert_eq!(db.all(), expected);
    assert_eq!(first.stored as usize, expected.len());

    // New files: a copy of a track that had none, an exact copy, and a
    // track of its own.
    add("100 (new).mp3".into(), reencoded(&track(100, LEN), 77, 7));
    add("101 (backup).flac".into(), track(101, LEN));
    add("900.flac".into(), track(900, LEN));
    // Changed audio: a copy becomes a track of its own, a track becomes a
    // copy of another, and the file standing for two identical ones
    // becomes something else.
    let other = |items: Vec<u32>| fingerprint(with_silence(&items));
    db.set_fingerprint(ids["1.mp3"], &other(track(901, LEN)));
    db.set_fingerprint(ids["50.flac"], &other(reencoded(&track(60, LEN), 78, 7)));
    db.set_fingerprint(ids["5.flac"], &other(track(902, LEN)));
    // And two files go: one loses its fingerprint, one its row.
    db.sql(
        "UPDATE file SET fingerprint = NULL WHERE id = ?1",
        (ids["2.mp3"],),
    );
    db.sql("DELETE FROM file WHERE id = ?1", (ids["3 (cut).flac"],));

    let later = pass(&matcher, &db);
    assert_eq!(later.changed, 6, "three new files and three changed ones");
    let expected = rows_by_the_plain_index(&db);
    assert_eq!(db.all(), expected);
    assert!(db.of_pair(ids["100.flac"], ids["100 (new).mp3"]).is_some());
    assert!(db.of_pair(ids["50.flac"], ids["60.flac"]).is_some());
    assert!(db.of_pair(ids["1.flac"], ids["1.mp3"]).is_none());

    // A new matcher builds its index from nothing and agrees.
    let restarted = refresh(&db.writer).unwrap();
    assert_eq!((restarted.compared, restarted.stored), (0, 0));
    assert_eq!(db.all(), expected);
}
