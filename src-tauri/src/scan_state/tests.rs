//! Which files a stage still has to do: new, changed, re-versioned and
//! skipped files are due; done and failed files that haven't changed
//! aren't.

use super::*;
use crate::db::Writer;
use crate::scan::folders::{self, MusicFolderId};

const V1: i64 = 1;

/// A migrated database with one volume, two music folders (ids 1 and 2)
/// and `files` files in each, ids 1..=files in folder 1 and the rest in
/// folder 2. Every file has size 100 and mtime 1000.
fn db(files: i64) -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    writer
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute_batch(
                "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'A', 'A');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'B', 'B');",
            )?;
            for folder in [1, 2] {
                for n in 0..files {
                    let name = format!("{n}.mp3");
                    tx.execute(
                        "INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime)
                         VALUES (?1, ?2, ?2, 100, 1000)",
                        (folder, name),
                    )?;
                }
            }
            tx.commit()
        })
        .unwrap();
    (dir, writer)
}

fn due_ids(writer: &Writer, stage: Stage, version: i64, scope: Scope) -> Vec<i64> {
    writer
        .call(move |c| due(c, stage, version, &scope, 0, 10_000))
        .unwrap()
        .into_iter()
        .map(|f| f.id)
        .collect()
}

/// Records `outcome` for every file due now, as a stage would.
fn record_all(writer: &Writer, stage: Stage, version: i64, outcome: Outcome) {
    writer
        .call(move |c| {
            let files = due(c, stage, version, &Scope::All, 0, 10_000)?;
            let results: Vec<_> = files
                .iter()
                .map(|f| Recorded::of(f, outcome.clone()))
                .collect();
            let tx = c.transaction()?;
            record(&tx, stage, version, &results)?;
            tx.commit()
        })
        .unwrap();
}

fn set(writer: &Writer, sql: &'static str) {
    writer.call(move |c| c.execute_batch(sql)).unwrap();
}

#[test]
fn every_present_file_is_due_until_the_stage_records_it() {
    let (_dir, writer) = db(3);
    assert_eq!(
        due_ids(&writer, Stage::Read, V1, Scope::All),
        [1, 2, 3, 4, 5, 6]
    );
    record_all(&writer, Stage::Read, V1, Outcome::Done);
    assert_eq!(due_ids(&writer, Stage::Read, V1, Scope::All), [0i64; 0]);
    let count = writer
        .call(|c| count_due(c, Stage::Read, V1, &Scope::All))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn a_changed_size_or_mtime_makes_a_done_file_due_again() {
    let (_dir, writer) = db(3);
    record_all(&writer, Stage::Read, V1, Outcome::Done);
    set(&writer, "UPDATE file SET size = 101 WHERE id = 2");
    set(&writer, "UPDATE file SET mtime = 1001 WHERE id = 5");
    assert_eq!(due_ids(&writer, Stage::Read, V1, Scope::All), [2, 5]);
    // A stat the walk couldn't read counts as a change too.
    set(&writer, "UPDATE file SET mtime = NULL WHERE id = 6");
    assert_eq!(due_ids(&writer, Stage::Read, V1, Scope::All), [2, 5, 6]);
}

#[test]
fn a_bumped_stage_version_makes_every_file_due_again() {
    let (_dir, writer) = db(2);
    record_all(&writer, Stage::Hash, V1, Outcome::Done);
    assert!(due_ids(&writer, Stage::Hash, V1, Scope::All).is_empty());
    assert_eq!(due_ids(&writer, Stage::Hash, 2, Scope::All), [1, 2, 3, 4]);
}

#[test]
fn a_failed_file_is_not_retried_until_it_changes() {
    let (_dir, writer) = db(2);
    record_all(&writer, Stage::Read, V1, Outcome::Failed("undecodable"));
    assert!(due_ids(&writer, Stage::Read, V1, Scope::All).is_empty());
    set(&writer, "UPDATE file SET mtime = 2000 WHERE id = 3");
    assert_eq!(due_ids(&writer, Stage::Read, V1, Scope::All), [3]);
    let reason: Option<String> = writer
        .call(|c| {
            c.query_row(
                "SELECT reason FROM file_stage WHERE file_id = 3 AND status = 'failed'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(reason.as_deref(), Some("undecodable"));
}

#[test]
fn a_skipped_file_is_due_again_on_the_next_run() {
    // A skip (e.g. an online-only OneDrive file) never tried the file, so
    // the next run decides again: the user may have opted in since.
    let (_dir, writer) = db(1);
    record_all(&writer, Stage::Fingerprint, V1, Outcome::online_only());
    assert_eq!(due_ids(&writer, Stage::Fingerprint, V1, Scope::All), [1, 2]);
}

#[test]
fn a_redone_file_gets_its_row_replaced_not_duplicated_with_a_fresh_done_at() {
    let (_dir, writer) = db(1);
    record_all(&writer, Stage::Read, V1, Outcome::Failed("undecodable"));
    set(
        &writer,
        "UPDATE file_stage SET done_at = '2000-01-01T00:00:00.000Z'",
    );
    set(&writer, "UPDATE file SET size = 5 WHERE id = 1");
    record_all(&writer, Stage::Read, V1, Outcome::Done);
    let stamps: Vec<String> = writer
        .call(|c| {
            let mut stmt = c.prepare("SELECT done_at FROM file_stage ORDER BY file_id")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect()
        })
        .unwrap();
    assert!(
        stamps[0].as_str() > "2000-01-01T00:00:00.000Z",
        "{stamps:?}"
    );
    assert_eq!(
        stamps[1], "2000-01-01T00:00:00.000Z",
        "not redone, not restamped"
    );
    let rows: Vec<(i64, String, Option<String>, Option<i64>)> = writer
        .call(|c| {
            let mut stmt =
                c.prepare("SELECT file_id, status, reason, size FROM file_stage ORDER BY file_id")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })
        .unwrap();
    assert_eq!(
        rows,
        [
            (1, "done".into(), None, Some(5)),
            (2, "failed".into(), Some("undecodable".into()), Some(100)),
        ]
    );
}

#[test]
fn each_stage_keeps_its_own_record_of_a_file() {
    let (_dir, writer) = db(1);
    record_all(&writer, Stage::Read, V1, Outcome::Done);
    assert!(due_ids(&writer, Stage::Read, V1, Scope::All).is_empty());
    assert_eq!(due_ids(&writer, Stage::Hash, V1, Scope::All), [1, 2]);
    assert_eq!(due_ids(&writer, Stage::Fingerprint, V1, Scope::All), [1, 2]);
}

#[test]
fn a_file_that_is_no_longer_present_is_never_due() {
    let (_dir, writer) = db(2);
    set(&writer, "UPDATE file SET present = 0 WHERE id IN (1, 4)");
    assert_eq!(due_ids(&writer, Stage::Read, V1, Scope::All), [2, 3]);
}

#[test]
fn a_scope_limits_the_due_files_to_its_music_folders_or_its_files() {
    let (_dir, writer) = db(2);
    assert_eq!(
        due_ids(&writer, Stage::Read, V1, Scope::Folders(vec![2])),
        [3, 4]
    );
    assert_eq!(
        due_ids(&writer, Stage::Read, V1, Scope::Files(vec![4, 1])),
        [1, 4]
    );
    let counted = writer
        .call(|c| count_due(c, Stage::Read, V1, &Scope::Folders(vec![1])))
        .unwrap();
    assert_eq!(counted, 2);
    assert!(due_ids(&writer, Stage::Read, V1, Scope::Folders(vec![])).is_empty());
}

#[test]
fn paging_by_the_last_id_visits_every_due_file_once_in_id_order() {
    let (_dir, writer) = db(25);
    let mut seen = Vec::new();
    let mut after = 0;
    loop {
        let page = writer
            .call(move |c| due(c, Stage::Read, V1, &Scope::All, after, 7))
            .unwrap();
        let Some(last) = page.last() else { break };
        after = last.id;
        assert!(page.len() <= 7);
        seen.extend(page.iter().map(|f| f.id));
    }
    assert_eq!(seen, (1..=50).collect::<Vec<_>>());
}

#[test]
fn a_due_file_carries_its_folder_path_and_stat() {
    let (_dir, writer) = db(1);
    let first = writer
        .call(|c| due(c, Stage::Read, V1, &Scope::All, 0, 1))
        .unwrap();
    assert_eq!(
        first,
        [DueFile {
            id: 1,
            music_folder_id: 1,
            rel_path: "0.mp3".into(),
            size: Some(100),
            mtime: Some(1000),
            online_only: false,
        }]
    );
}

#[test]
fn a_due_file_says_whether_the_walk_found_it_online_only() {
    // One file in each of two folders: ids 1 and 2.
    let (_dir, writer) = db(1);
    let marks: Vec<bool> = writer
        .call(|c| {
            c.execute("UPDATE file SET online_only = 1 WHERE id = 2", [])?;
            due(c, Stage::Read, V1, &Scope::All, 0, 10)
        })
        .unwrap()
        .iter()
        .map(|f| f.online_only)
        .collect();
    // Still due: a stage decides with its ReadGate whether to read it.
    assert_eq!(marks, [false, true]);
}

#[test]
fn recording_a_file_whose_row_is_gone_is_ignored() {
    let (_dir, writer) = db(1);
    let gone = Recorded {
        file: 999,
        size: None,
        mtime: None,
        outcome: Outcome::Done,
    };
    writer
        .call(move |c| record(c, Stage::Read, V1, &[gone]))
        .unwrap();
    let rows: i64 = writer
        .call(|c| c.query_row("SELECT COUNT(*) FROM file_stage", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn removing_a_music_folder_removes_its_files_stage_rows() {
    let (_dir, writer) = db(2);
    record_all(&writer, Stage::Read, V1, Outcome::Done);
    record_all(&writer, Stage::Hash, V1, Outcome::Failed("undecodable"));
    writer
        .call(|c| folders::remove(c, MusicFolderId(1)))
        .unwrap()
        .unwrap();
    let left: Vec<i64> = writer
        .call(|c| {
            let mut stmt = c.prepare("SELECT DISTINCT file_id FROM file_stage ORDER BY file_id")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect()
        })
        .unwrap();
    assert_eq!(left, [3, 4]);
}

#[test]
fn an_unreachable_file_is_a_skip_so_it_is_tried_again_when_it_is_back() {
    // An unplugged drive or a locked file: the size and mtime won't change
    // when it's back, so a failure would never be retried.
    let (_dir, writer) = db(1);
    record_all(&writer, Stage::Read, V1, Outcome::unreachable());
    assert_eq!(due_ids(&writer, Stage::Read, V1, Scope::All), [1, 2]);
    let reasons: Vec<(String, String)> = writer
        .call(|c| {
            let mut stmt = c.prepare("SELECT DISTINCT status, reason FROM file_stage")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .unwrap();
    assert_eq!(reasons, [("skipped".to_owned(), UNREACHABLE.to_owned())]);
}

#[test]
fn a_run_that_skips_every_file_ends_after_seeing_each_file_once() {
    // Skipped files stay due, so a run only ends because it pages forward
    // by the last id and never starts over.
    let (_dir, writer) = db(25);
    let mut seen = Vec::new();
    let mut after = 0;
    loop {
        let page = writer
            .call(move |c| {
                let page = due(c, Stage::Read, V1, &Scope::All, after, 7)?;
                let skips: Vec<_> = page
                    .iter()
                    .map(|f| Recorded::of(f, Outcome::online_only()))
                    .collect();
                let tx = c.transaction()?;
                record(&tx, Stage::Read, V1, &skips)?;
                tx.commit()?;
                Ok(page)
            })
            .unwrap();
        let Some(last) = page.last() else { break };
        after = last.id;
        seen.extend(page.iter().map(|f| f.id));
        assert!(seen.len() <= 50, "the run went round again");
    }
    assert_eq!(seen, (1..=50).collect::<Vec<_>>());
    // Every file is still due, so the count never drops to 0.
    let left = writer
        .call(|c| count_due(c, Stage::Read, V1, &Scope::All))
        .unwrap();
    assert_eq!(left, 50);
}
