#![cfg(test)]
//! 1cD-X4: the startup checks that were made cheaper give the answer they
//! always gave.
//!
//! One small library is built "with nothing due" and then put, one change at
//! a time, into states a check could be asked in: files gone or missing,
//! records and rows deleted or of another version, a NULL hash, no
//! fingerprint, sizes and mtimes that moved, a skipped stage, an online-only
//! file. After each change every cheaper check is held to the check it
//! replaced (kept next to it for this), over the scopes and gates it is asked
//! with. Nothing due must stay nothing due and, the part that matters,
//! nothing that was due may stop being due.

use std::collections::BTreeMap;

use super::support::db;
use super::walk::{add_music, drive};
use crate::db::Writer;
use crate::scan::chain::{any_to_try, any_to_try_each};
use crate::scan::ReadGate;
use crate::scan_state::{Scope, Stage};

/// Six files in music folder 1: five fingerprinted and hashed (the fifth is
/// a copy of the fourth, standing on it in matching), and a sixth that
/// failed to hash and has no fingerprint. Everything is done, current,
/// grouped, measured and decided: nothing is due.
fn nothing_due(writer: &Writer) {
    let sql = format!(
        "INSERT INTO file (id, music_folder_id, rel_path, rel_path_key, size, mtime, audio_hash,
                           fingerprint, present)
         SELECT i, 1, 'f' || i, 'f' || i, 1000, 1000,
                CASE WHEN i < 6 THEN randomblob(34) END,
                CASE WHEN i < 6 THEN randomblob(64) END, 1
         FROM (SELECT 1 AS i UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4
               UNION ALL SELECT 5 UNION ALL SELECT 6);
         UPDATE file SET fingerprint = (SELECT fingerprint FROM file WHERE id = 4) WHERE id = 5;
         INSERT INTO file_stage (file_id, stage, version, size, mtime, status)
         SELECT id, 'read', {read}, 1000, 1000, 'done' FROM file;
         INSERT INTO file_stage (file_id, stage, version, size, mtime, status)
         SELECT id, 'hash', {hash}, 1000, 1000, 'done' FROM file WHERE id < 6;
         INSERT INTO file_stage (file_id, stage, version, size, mtime, status, reason)
         SELECT id, 'hash', {hash}, 1000, 1000, 'failed', 'damaged' FROM file WHERE id = 6;
         INSERT INTO file_stage (file_id, stage, version, size, mtime, status)
         SELECT id, 'fingerprint', {fingerprint}, 1000, 1000, 'done' FROM file WHERE id < 6;
         INSERT INTO file_quality (file_id, audio_hash, method_version, cutoff_gap, decoded_ms, ended)
         SELECT id, audio_hash, {quality}, 'too_short', 1000, 'complete' FROM file WHERE id < 6;
         INSERT INTO fingerprint_matched (file_id, version, stands)
         SELECT id, {matching}, CASE WHEN id = 5 THEN 4 ELSE id END FROM file WHERE id < 6;
         INSERT INTO recording (id) SELECT id FROM file;
         INSERT INTO recording_file (recording_id, file_id, role) SELECT id, id, 'best' FROM file;
         INSERT INTO grouping_file (file_id, rev, decided_as, placed_key)
         SELECT id, 0, {decided}, audio_hash FROM file;",
        read = crate::read::READ_VERSION,
        hash = crate::hash::DEFINITION,
        fingerprint = crate::fingerprint::VERSION,
        quality = crate::quality::VERSION,
        matching = crate::matching::VERSION,
        decided = crate::grouping::decided_as(),
    );
    writer.call(move |c| c.execute_batch(&sql)).unwrap();
}

/// Every change a check could be asked after, each from the library with
/// nothing due. A change may be several statements.
fn changes() -> Vec<(&'static str, String)> {
    let skipped = |stage: &str, file: i64, reason: &str| {
        format!(
            "UPDATE file_stage SET status = 'skipped', reason = '{reason}'
             WHERE stage = '{stage}' AND file_id = {file}"
        )
    };
    let unreachable = crate::scan_state::UNREACHABLE;
    vec![
        ("nothing", "SELECT 1".into()),
        (
            "a file goes missing",
            "UPDATE file SET present = 0 WHERE id = 1".into(),
        ),
        (
            "a file's record is gone",
            "DELETE FROM grouping_file WHERE file_id = 2".into(),
        ),
        (
            "a missing file's record is gone",
            "UPDATE file SET present = 0 WHERE id = 2;
             DELETE FROM grouping_file WHERE file_id = 2"
                .into(),
        ),
        (
            "a record is undecided",
            "UPDATE grouping_file SET decided_as = NULL WHERE file_id = 3".into(),
        ),
        (
            "a record is of other rules",
            "UPDATE grouping_file SET decided_as = decided_as + 1 WHERE file_id = 3".into(),
        ),
        (
            "an undecided file is not ready: its match is gone",
            "UPDATE grouping_file SET decided_as = NULL WHERE file_id = 2;
             DELETE FROM fingerprint_matched WHERE file_id = 2"
                .into(),
        ),
        (
            "an undecided file has no fingerprint",
            "UPDATE grouping_file SET decided_as = NULL WHERE file_id = 6".into(),
        ),
        (
            "a match is gone",
            "DELETE FROM fingerprint_matched WHERE file_id = 2".into(),
        ),
        (
            "a match is of another version",
            "UPDATE fingerprint_matched SET version = version + 1 WHERE file_id = 2".into(),
        ),
        (
            "the file a match stands on is missing",
            "UPDATE file SET present = 0 WHERE id = 4".into(),
        ),
        (
            "the file a match stands on holds another fingerprint",
            "UPDATE file SET fingerprint = randomblob(64) WHERE id = 4".into(),
        ),
        (
            "a copy loses its fingerprint",
            "UPDATE file SET fingerprint = NULL WHERE id = 5".into(),
        ),
        (
            "a match stands on another file that goes missing",
            "UPDATE fingerprint_matched SET stands = 3 WHERE file_id = 5;
             UPDATE file SET present = 0 WHERE id = 3"
                .into(),
        ),
        (
            "a file is in no track",
            "DELETE FROM recording_file WHERE file_id = 3".into(),
        ),
        (
            "a missing file is in no track",
            "UPDATE file SET present = 0 WHERE id = 3;
             DELETE FROM recording_file WHERE file_id = 3"
                .into(),
        ),
        (
            "a measurement is gone",
            "DELETE FROM file_quality WHERE file_id = 2".into(),
        ),
        (
            "a measurement is of other audio",
            "UPDATE file_quality SET audio_hash = randomblob(34) WHERE file_id = 2".into(),
        ),
        (
            "a measurement is of another method",
            "UPDATE file_quality SET method_version = method_version + 1 WHERE file_id = 2".into(),
        ),
        (
            "a hash is of another version",
            "UPDATE file_stage SET version = version + 1 WHERE stage = 'hash' AND file_id = 2"
                .into(),
        ),
        (
            "a file changed size",
            "UPDATE file SET size = 1001 WHERE id = 3".into(),
        ),
        (
            "a file has no mtime on either side",
            "UPDATE file SET mtime = NULL WHERE id = 3;
             UPDATE file_stage SET mtime = NULL WHERE file_id = 3"
                .into(),
        ),
        (
            "a file has no mtime now",
            "UPDATE file SET mtime = NULL WHERE id = 3".into(),
        ),
        (
            "a file has no read row",
            "DELETE FROM file_stage WHERE stage = 'read' AND file_id = 1".into(),
        ),
        (
            "a missing file has no read row",
            "UPDATE file SET present = 0 WHERE id = 1;
             DELETE FROM file_stage WHERE stage = 'read' AND file_id = 1"
                .into(),
        ),
        (
            "a file has no fingerprint row",
            "DELETE FROM file_stage WHERE stage = 'fingerprint' AND file_id = 4".into(),
        ),
        (
            "a file has a NULL hash",
            "UPDATE file SET audio_hash = NULL WHERE id = 2".into(),
        ),
        (
            "a hash was skipped as unreachable",
            skipped("hash", 2, unreachable),
        ),
        (
            "a read was skipped, for another reason",
            skipped("read", 2, "online_only"),
        ),
        (
            "a fingerprint was skipped as unreachable",
            skipped("fingerprint", 5, unreachable),
        ),
        (
            "an online-only file is unmeasured",
            "UPDATE file SET online_only = 1 WHERE id = 2;
             DELETE FROM file_quality WHERE file_id = 2"
                .into(),
        ),
        (
            "tags are there and a match is lost",
            "UPDATE file SET raw_tags = '{}' WHERE id = 2;
             DELETE FROM fingerprint_matched WHERE file_id = 2"
                .into(),
        ),
        ("no file is present", "UPDATE file SET present = 0".into()),
        (
            "another stage version",
            "UPDATE file_stage SET version = version + 1 WHERE file_id = 3".into(),
        ),
    ]
}

#[test]
fn every_cheaper_startup_check_gives_the_answer_it_gave_before() {
    let (_dir, volume, music) = drive();
    let (_db, writer, _reads) = db();
    add_music(&writer, &volume, &music);
    nothing_due(&writer);
    // What each check answered over all the states: a check that never says
    // yes, or never no, proved nothing.
    let mut seen: BTreeMap<&str, [bool; 2]> = BTreeMap::new();
    for (state, change) in changes() {
        let answers = writer
            .call(move |c| {
                // Each change starts from nothing due: look, then take it back.
                let tx = c.transaction()?;
                tx.execute_batch(&change)?;
                let answers = checks(&tx);
                tx.rollback()?;
                answers
            })
            .unwrap();
        for (what, new, old) in answers {
            assert_eq!(new, old, "{what}, after: {state}");
            seen.entry(what).or_default()[usize::from(new)] = true;
        }
    }
    for (what, [no, yes]) in seen {
        assert!(
            no && yes,
            "{what} was only ever {}",
            if yes { "yes" } else { "no" }
        );
    }
}

/// Each cheaper check beside the one it replaced, for the state the
/// connection is in.
fn checks(c: &rusqlite::Connection) -> rusqlite::Result<Vec<(&'static str, bool, bool)>> {
    let mut out = vec![
        (
            "deciding",
            crate::grouping::decide::any_due(c)?,
            crate::grouping::decide::any_due_by_one_scan(c)?,
        ),
        (
            "matching",
            crate::matching::any_due(c)?,
            crate::matching::any_due_by_one_scan(c)?,
        ),
        (
            "grouping",
            crate::grouping::any_ungrouped(c)?,
            crate::grouping::any_ungrouped_by_scan(c)?,
        ),
    ];
    for allows in [false, true] {
        for reachable in [None, Some(&[1_i64][..]), Some(&[][..])] {
            out.push((
                "quality",
                crate::quality::ledger::any_due(c, allows, reachable)?,
                crate::quality::ledger::any_due_hash_first(c, allows, reachable)?,
            ));
        }
    }
    let gate = ReadGate::for_job(c)?;
    let stages = [
        (Stage::Read, crate::read::READ_VERSION),
        (Stage::Hash, i64::from(crate::hash::DEFINITION)),
        (Stage::Fingerprint, i64::from(crate::fingerprint::VERSION)),
    ];
    let scopes = [
        Scope::All,
        Scope::Folders(vec![1]),
        Scope::Folders(vec![2]),
        Scope::Files(vec![2, 3]),
        Scope::Files(vec![]),
    ];
    for scope in &scopes {
        for rescan in [false, true] {
            let together = any_to_try_each(c, &stages, scope, gate, rescan)?;
            for (i, (stage, version)) in stages.iter().enumerate() {
                out.push((
                    "the stages together",
                    together[i],
                    any_to_try(c, *stage, *version, scope, gate, rescan)?,
                ));
            }
        }
    }
    Ok(out)
}
