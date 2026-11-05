#![cfg(test)]
//! 2A-I1: an Inbox folder walked as a job on a real (temp) folder tree.

use std::fs;

use super::support::db;
use super::walk::{drive, scan};
use crate::inbox::{items, BatchKind, InboxFilter};
use crate::scan::folders::add;
use crate::scan::MusicFolderRole;

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
