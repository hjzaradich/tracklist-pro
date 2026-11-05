#![cfg(test)]
//! 2A-I1: an Inbox folder walked as a job on a real (temp) folder tree.

use std::fs;

use super::chain::wait_idle;
use super::support::db;
use super::walk::{drive, put, scan};
use crate::inbox::{items, BatchKind, InboxFilter};
use crate::scan::folders::add;
use crate::scan::walk::scan_job;
use crate::scan::MusicFolderRole;
use crate::tags::test_audio as audio;

#[test]
fn the_first_walk_of_an_inbox_folder_makes_no_item_of_a_download_in_progress() {
    let (_dir, volume, music) = drive();
    let (_db, writer, _reads) = db();
    fs::write(music.join("a.mp3"), b"a").unwrap();
    fs::write(music.join("b.mp3.crdownload"), b"b").unwrap();
    fs::write(music.join("c.mp3.part"), b"c").unwrap();
    fs::write(music.join("d.mp3.tmp"), b"d").unwrap();
    add(&writer, &volume, &music, MusicFolderRole::Inbox).unwrap();
    scan(&writer, &volume);

    let every = InboxFilter {
        include_already_there: true,
        ..Default::default()
    };
    let there = writer.call(move |c| items(c, &every)).unwrap();
    assert_eq!(there.len(), 1);
    assert_eq!(there[0].batch_kind, BatchKind::AlreadyThere);
    let files: i64 = writer
        .call(|c| c.query_row("SELECT count(*) FROM file", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(files, 1);
}

/// The chain of `chain.rs` with the hash stage wrapped in intake, as the
/// app registers it.
fn intaking_queue(
    writer: &crate::db::Writer,
    volume: &super::support::TempVolume,
) -> crate::jobs::JobQueue {
    let (v1, v2) = (volume.clone(), volume.clone());
    super::chain::chained_queue_with(
        writer,
        volume,
        &super::chain::LookedAt::default(),
        2,
        move |b| {
            b.handler(
                crate::jobs::JobKind::Hash,
                crate::scan::chain::after_hash(crate::inbox::intake::after_hash(
                    crate::hash::Hasher::new(move || v1.clone()),
                ))
                .reaching(move || v2.clone()),
            )
        },
    )
}

/// What the jobs after `from` were and how they ended.
fn ended(writer: &crate::db::Writer, from: usize) -> Vec<(String, String)> {
    super::chain::jobs(writer)[from..]
        .iter()
        .map(|j| (j.0.clone(), j.3.clone()))
        .collect()
}

#[test]
fn a_file_that_held_still_through_a_real_hash_job_becomes_an_item_and_the_chain_goes_on() {
    let (_dir, volume, music) = drive();
    let (_db, writer, _reads) = db();
    put(&music, "old.mp3", &audio::mp3());
    let folder = add(&writer, &volume, &music, MusicFolderRole::Inbox)
        .unwrap()
        .id;
    let queue = intaking_queue(&writer, &volume);
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    let first = super::chain::jobs(&writer).len();
    assert!(writer
        .call(|c| items(c, &InboxFilter::default()))
        .unwrap()
        .is_empty());

    put(&music, "Pack/new.mp3", &audio::mp3());
    queue.enqueue(scan_job(Some(vec![folder]))).unwrap();
    wait_idle(&queue);
    let fresh = writer.call(|c| items(c, &InboxFilter::default())).unwrap();
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].batch_kind, BatchKind::Folder);
    assert_eq!(fresh[0].batch_label.as_deref(), Some("Pack"));
    // The stages after the hash were queued and ran, as without intake.
    let after = ended(&writer, first);
    for kind in ["scan", "read", "hash", "group", "fingerprint", "quality"] {
        assert!(
            after.contains(&(kind.into(), "done".into())),
            "{kind} missing from {after:?}"
        );
    }
    queue.shutdown();
}

#[test]
fn an_intake_that_fails_leaves_the_hash_job_done_and_the_chain_going() {
    let (_dir, volume, music) = drive();
    let (_db, writer, _reads) = db();
    put(&music, "old.mp3", &audio::mp3());
    let folder = add(&writer, &volume, &music, MusicFolderRole::Inbox)
        .unwrap()
        .id;
    let queue = intaking_queue(&writer, &volume);
    queue.enqueue(scan_job(None)).unwrap();
    wait_idle(&queue);
    let first = super::chain::jobs(&writer).len();
    writer
        .call(|c| {
            c.execute_batch(
                "CREATE TRIGGER no_items BEFORE INSERT ON inbox_item
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
        })
        .unwrap();

    put(&music, "new.mp3", &audio::mp3());
    queue.enqueue(scan_job(Some(vec![folder]))).unwrap();
    wait_idle(&queue);
    assert!(writer
        .call(|c| items(c, &InboxFilter::default()))
        .unwrap()
        .is_empty());
    let after = ended(&writer, first);
    for kind in ["scan", "read", "hash", "group", "fingerprint", "quality"] {
        assert!(
            after.contains(&(kind.into(), "done".into())),
            "{kind} missing from {after:?}"
        );
    }
    queue.shutdown();
}
