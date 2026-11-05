#![cfg(test)]
//! Intake and the Inbox read (ROADMAP 2.1), on a real (temp) database with
//! the walk's own writes. Files are synthetic: rows, no audio on disk.

use super::intake::{baseline, run};
use super::{count, items, BatchKind, InboxCount, InboxFilter, ItemStatus};
use crate::db::Writer;
use crate::paths::RelPath;
use crate::scan::folders::{remove, MusicFolderId};
use crate::scan::walk::{finish, upsert_with, Found, Unread};

const START: &str = "2026-01-01T00:00:00.000Z";
const INBOX: MusicFolderId = MusicFolderId(1);
const MUSIC: MusicFolderId = MusicFolderId(2);

/// An Inbox folder (id 1) and a scan folder (id 2) on one volume, never
/// walked.
fn library() -> (tempfile::TempDir, Writer) {
    let dir = tempfile::tempdir().unwrap();
    let writer = Writer::open(&crate::write_guard::test_path(
        dir.path(),
        crate::db::DB_FILE_NAME,
    ))
    .unwrap();
    writer
        .call(|c| {
            c.execute_batch(
                "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'internal');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key, role)
                     VALUES (1, 'Downloads', 'downloads', 'inbox');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key, role)
                     VALUES (1, 'Music', 'music', 'scan');",
            )
        })
        .unwrap();
    (dir, writer)
}

fn found(rel: &str, size: i64) -> Found {
    Found {
        rel: RelPath::parse(rel).unwrap(),
        size,
        mtime_ns: 1_000 + size,
        file_id: None,
        online_only: false,
    }
}

/// A walk of `folder` that sees `files` and reaches its end.
fn walk(writer: &Writer, folder: MusicFolderId, files: Vec<Found>) {
    writer
        .call(move |c| {
            upsert_with(c, folder, &files, START, &[])?;
            finish(c, folder, &Unread::default(), START).map(|_| ())
        })
        .unwrap();
}

/// The hash stage has opened `rel` and found it as the walk stored it.
fn hashed(writer: &Writer, rel: &str) {
    let rel = rel.to_owned();
    writer
        .call(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO file_stage (file_id, stage, version, size, mtime, status)
                 SELECT id, 'hash', 1, size, mtime, 'done' FROM file WHERE rel_path = ?1",
                [rel],
            )
        })
        .unwrap();
}

fn intake(writer: &Writer) -> usize {
    writer.call(run).unwrap()
}

fn all(writer: &Writer) -> Vec<super::InboxItem> {
    let every = InboxFilter {
        include_already_there: true,
        ..Default::default()
    };
    writer.call(move |c| items(c, &every)).unwrap()
}

fn path_of(writer: &Writer, item: &super::InboxItem) -> String {
    let id = item.file_id;
    writer
        .call(move |c| {
            c.query_row("SELECT rel_path FROM file WHERE id = ?1", [id], |r| {
                r.get(0)
            })
        })
        .unwrap()
}

#[test]
fn the_first_walk_of_a_new_inbox_folder_puts_what_is_there_in_one_already_there_batch() {
    let (_dir, writer) = library();
    walk(
        &writer,
        INBOX,
        vec![
            found("a.mp3", 1),
            found("Pack/b.mp3", 2),
            found("Pack/c.flac", 3),
        ],
    );
    let there = all(&writer);
    assert_eq!(there.len(), 3);
    assert!(there
        .iter()
        .all(|i| i.batch_kind == BatchKind::AlreadyThere));
    assert!(there.iter().all(|i| i.batch_id == there[0].batch_id));
    assert!(there.iter().all(|i| i.status == ItemStatus::New));
    assert!(there.iter().all(|i| i.batch_label.is_none()));
    // Not arrivals: the default read and the badge leave them out, and
    // nothing is hidden.
    assert!(writer
        .call(|c| items(c, &InboxFilter::default()))
        .unwrap()
        .is_empty());
    assert_eq!(
        writer.call(|c| count(c)).unwrap(),
        InboxCount {
            new: 0,
            already_there: 3
        }
    );
    let hidden: i64 = writer
        .call(|c| {
            c.query_row(
                "SELECT count(*) FROM file WHERE hidden_reason IS NOT NULL",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(hidden, 0);
}

#[test]
fn the_baseline_is_made_once_a_later_walk_of_the_folder_adds_no_already_there_batch() {
    let (_dir, writer) = library();
    walk(&writer, INBOX, vec![found("a.mp3", 1)]);
    walk(&writer, INBOX, vec![found("a.mp3", 1), found("b.mp3", 2)]);
    // `b.mp3` has not held still yet.
    let there = all(&writer);
    assert_eq!(there.len(), 1);
    let batches: i64 = writer
        .call(|c| c.query_row("SELECT count(*) FROM inbox_batch", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(batches, 1);
    // Called again by hand, it still makes nothing.
    assert_eq!(writer.call(|c| baseline(c, INBOX)).unwrap(), 0);
}

#[test]
fn a_later_file_becomes_an_item_in_a_loose_batch_once_it_holds_still() {
    let (_dir, writer) = library();
    walk(&writer, INBOX, vec![found("old.mp3", 1)]);
    walk(
        &writer,
        INBOX,
        vec![found("old.mp3", 1), found("new.mp3", 2)],
    );
    assert_eq!(intake(&writer), 0, "seen once, not yet looked at twice");

    hashed(&writer, "new.mp3");
    assert_eq!(intake(&writer), 1);
    let fresh = writer.call(|c| items(c, &InboxFilter::default())).unwrap();
    assert_eq!(fresh.len(), 1);
    assert_eq!(path_of(&writer, &fresh[0]), "new.mp3");
    assert_eq!(fresh[0].batch_kind, BatchKind::Loose);
    assert_eq!(fresh[0].status, ItemStatus::New);
    assert!(fresh[0].present && !fresh[0].online_only);
    assert_eq!(
        writer.call(|c| count(c)).unwrap(),
        InboxCount {
            new: 1,
            already_there: 1
        }
    );
    // Once an item, always the same one.
    assert_eq!(intake(&writer), 0);
}

#[test]
fn files_that_arrive_together_in_a_subfolder_form_one_folder_batch() {
    let (_dir, writer) = library();
    walk(&writer, INBOX, vec![]);
    walk(
        &writer,
        INBOX,
        vec![
            found("Pool Pack/one.mp3", 1),
            found("Pool Pack/two.mp3", 2),
            found("Other/three.mp3", 3),
            found("loose.mp3", 4),
        ],
    );
    for rel in [
        "Pool Pack/one.mp3",
        "Pool Pack/two.mp3",
        "Other/three.mp3",
        "loose.mp3",
    ] {
        hashed(&writer, rel);
    }
    assert_eq!(intake(&writer), 4);
    let fresh = writer.call(|c| items(c, &InboxFilter::default())).unwrap();
    let batch_of = |rel: &str| {
        fresh
            .iter()
            .find(|i| path_of(&writer, i) == rel)
            .map(|i| (i.batch_id, i.batch_kind, i.batch_label.clone()))
            .unwrap()
    };
    let pack = batch_of("Pool Pack/one.mp3");
    assert_eq!(pack, batch_of("Pool Pack/two.mp3"));
    assert_eq!(
        (pack.1, pack.2.as_deref()),
        (BatchKind::Folder, Some("Pool Pack"))
    );
    let other = batch_of("Other/three.mp3");
    assert_ne!(other.0, pack.0);
    assert_eq!(other.1, BatchKind::Folder);
    let loose = batch_of("loose.mp3");
    assert_eq!((loose.1, loose.2), (BatchKind::Loose, None));
}

#[test]
fn a_file_still_growing_is_not_an_item_until_it_holds_still() {
    let (_dir, writer) = library();
    walk(&writer, INBOX, vec![]);
    walk(&writer, INBOX, vec![found("copy.mp3", 100)]);
    hashed(&writer, "copy.mp3");
    // It grew after the hash stage looked: the walk stores the new size,
    // and the stage's record is of the old one.
    walk(&writer, INBOX, vec![found("copy.mp3", 200)]);
    assert_eq!(intake(&writer), 0);
    assert!(writer
        .call(|c| items(c, &InboxFilter::default()))
        .unwrap()
        .is_empty());
    // It is looked at again, and now it is the same.
    hashed(&writer, "copy.mp3");
    assert_eq!(intake(&writer), 1);
}

#[test]
fn a_download_in_progress_never_becomes_an_item() {
    for name in [
        "song.mp3.crdownload",
        "song.mp3.part",
        "song.mp3.tmp",
        "song.crdownload",
        "song.tmp",
    ] {
        assert!(!crate::scan::is_indexed(name), "{name} is indexed");
    }
    assert!(crate::scan::is_indexed("song.mp3"));
}

#[test]
fn a_scan_folder_never_makes_items() {
    let (_dir, writer) = library();
    walk(&writer, MUSIC, vec![found("a.mp3", 1)]);
    walk(&writer, MUSIC, vec![found("a.mp3", 1), found("b.mp3", 2)]);
    hashed(&writer, "a.mp3");
    hashed(&writer, "b.mp3");
    assert_eq!(intake(&writer), 0);
    assert!(all(&writer).is_empty());
    let batches: i64 = writer
        .call(|c| c.query_row("SELECT count(*) FROM inbox_batch", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(batches, 0);
}

#[test]
fn removing_the_inbox_folder_removes_its_items_and_batches() {
    let (_dir, writer) = library();
    walk(&writer, INBOX, vec![found("a.mp3", 1)]);
    walk(
        &writer,
        INBOX,
        vec![found("a.mp3", 1), found("Pack/b.mp3", 2)],
    );
    hashed(&writer, "Pack/b.mp3");
    assert_eq!(intake(&writer), 1);
    assert_eq!(all(&writer).len(), 2);

    assert_eq!(writer.call(|c| remove(c, INBOX)).unwrap(), Ok(()));
    assert!(all(&writer).is_empty());
    let batches: i64 = writer
        .call(|c| c.query_row("SELECT count(*) FROM inbox_batch", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(batches, 0);
}

#[test]
fn an_online_only_file_gets_an_item_without_anything_reading_its_content() {
    let (_dir, writer) = library();
    walk(&writer, INBOX, vec![]);
    let mut placeholder = found("cloud.mp3", 5);
    placeholder.online_only = true;
    walk(&writer, INBOX, vec![placeholder]);
    // Intake takes a connection and nothing else: it can't open a file.
    // No stage has touched the file either.
    assert_eq!(intake(&writer), 1);
    let stages: i64 = writer
        .call(|c| c.query_row("SELECT count(*) FROM file_stage", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(stages, 0);
    let fresh = writer.call(|c| items(c, &InboxFilter::default())).unwrap();
    assert_eq!(fresh.len(), 1);
    assert!(fresh[0].online_only);
}

#[test]
fn a_file_gone_from_disk_keeps_its_item_but_leaves_the_badge() {
    let (_dir, writer) = library();
    walk(&writer, INBOX, vec![]);
    walk(&writer, INBOX, vec![found("a.mp3", 1)]);
    hashed(&writer, "a.mp3");
    intake(&writer);
    assert_eq!(writer.call(|c| count(c)).unwrap().new, 1);
    writer
        .call(|c| c.execute("UPDATE file SET present = 0", []))
        .unwrap();
    assert_eq!(writer.call(|c| count(c)).unwrap().new, 0);
    let fresh = writer.call(|c| items(c, &InboxFilter::default())).unwrap();
    assert_eq!(fresh.len(), 1);
    assert!(!fresh[0].present);
}
