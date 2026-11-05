#![cfg(test)]
//! Labels and reasons (ROADMAP 2.1), and the baseline after incomplete
//! walks, on a real (temp) database. Files are synthetic rows.

use super::labels::Label;
use super::tests::{all, found, hashed, intake, library, walk, INBOX, START};
use super::{count, items, InboxFilter, InboxItem};
use crate::db::Writer;
use crate::scan::walk::{finish, upsert_with, Unread};
use crate::suggest::ReasonParam;

/// A library with an Inbox folder (1) whose baseline is taken, and a scan
/// folder (2).
struct World {
    _dir: tempfile::TempDir,
    writer: Writer,
}

impl World {
    fn new() -> World {
        let (dir, writer) = library();
        walk(&writer, INBOX, vec![]);
        World { _dir: dir, writer }
    }

    fn exec(&self, sql: &str) {
        let sql = sql.to_owned();
        self.writer.call(move |c| c.execute_batch(&sql)).unwrap();
    }

    fn num(&self, sql: &str) -> i64 {
        let sql = sql.to_owned();
        self.writer
            .call(move |c| c.query_row(&sql, [], |r| r.get(0)))
            .unwrap()
    }

    /// These files arrive in the Inbox and hold still. Returns their ids.
    fn arrive(&self, names: &[&str]) -> Vec<i64> {
        walk(
            &self.writer,
            INBOX,
            names
                .iter()
                .zip(1..)
                .map(|(name, size)| found(name, size))
                .collect(),
        );
        for name in names {
            hashed(&self.writer, name);
        }
        intake(&self.writer);
        names
            .iter()
            .map(|name| self.num(&format!("SELECT id FROM file WHERE rel_path = '{name}'")))
            .collect()
    }

    fn format(&self, file: i64, codec: &str, kbps: Option<i64>) {
        let kbps = kbps.map_or("NULL".to_owned(), |k| k.to_string());
        self.exec(&format!(
            "UPDATE file SET sniffed_format = '{codec}', codec = '{codec}', bitrate = {kbps}
             WHERE id = {file}"
        ));
    }

    /// A new track (recording) with no Library track.
    fn recording(&self) -> i64 {
        self.exec("INSERT INTO recording DEFAULT VALUES");
        self.num("SELECT max(id) FROM recording")
    }

    /// Grouping has decided `file`, which is in `recording`.
    fn decide(&self, file: i64, recording: i64) {
        self.exec(&format!(
            "INSERT INTO recording_file (recording_id, file_id, role)
                 VALUES ({recording}, {file}, 'extra');
             INSERT INTO grouping_file (file_id, decided_as) VALUES ({file}, 3)
                 ON CONFLICT (file_id) DO UPDATE SET decided_as = 3;"
        ));
    }

    /// A Library track linked to a 320 kbps MP3 in the scan folder.
    /// Returns (Library track, its recording, its file).
    fn library_track(&self, name: &str) -> (i64, i64, i64) {
        let recording = self.recording();
        self.exec(&format!(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, present,
                               sniffed_format, codec, bitrate)
                 VALUES (2, '{name}', '{name}', 1, 'mp3', 'mp3', 320);"
        ));
        let file = self.num("SELECT max(id) FROM file");
        self.exec(&format!(
            "INSERT INTO recording_file (recording_id, file_id, role)
                 VALUES ({recording}, {file}, 'best');
             INSERT INTO library_track (recording_id, kind, linked_file_id)
                 VALUES ({recording}, 'linked', {file});"
        ));
        let track = self.num("SELECT max(id) FROM library_track");
        (track, recording, file)
    }

    fn link(&self, a: i64, b: i64, kind: &str, label: &str) {
        self.exec(&format!(
            "INSERT INTO version_link (recording_a, recording_b, kind, label, source)
                 VALUES ({a}, {b}, '{kind}', '{label}', 'parser');"
        ));
    }

    fn item(&self, file: i64) -> InboxItem {
        all(&self.writer)
            .into_iter()
            .find(|i| i.file_id == file)
            .expect("the file has an item")
    }
}

fn text(item: &InboxItem, name: &str) -> Option<String> {
    match item.reason.params().get(name) {
        Some(ReasonParam::Text(t)) => Some(t.clone()),
        _ => None,
    }
}

#[test]
fn a_file_grouping_has_not_decided_yet_is_checking() {
    let w = World::new();
    let [file, cloud, placed] = w.arrive(&["a.mp3", "cloud.mp3", "placed.mp3"])[..] else {
        panic!()
    };
    // It has a track, but grouping hasn't finished with it.
    let rec = w.recording();
    w.decide(placed, rec);
    w.exec(&format!(
        "UPDATE grouping_file SET decided_as = NULL WHERE file_id = {placed}"
    ));
    w.exec(&format!(
        "UPDATE file SET online_only = 1 WHERE id = {cloud}"
    ));
    let (plain, online, placed) = (w.item(file), w.item(cloud), w.item(placed));
    assert_eq!(plain.label, Label::Checking);
    assert_eq!(plain.reason.key(), "inbox:reason.checking");
    assert_eq!(online.label, Label::Checking);
    assert_eq!(online.reason.key(), "inbox:reason.checkingOnlineOnly");
    assert_eq!(placed.label, Label::Checking);
}

#[test]
fn a_decided_file_whose_track_and_links_lead_nowhere_in_the_library_is_new_to_you() {
    let w = World::new();
    let [file] = w.arrive(&["a.mp3"])[..] else {
        panic!()
    };
    let rec = w.recording();
    w.decide(file, rec);
    // A link to a track that isn't in the Library doesn't count.
    let other = w.recording();
    w.link(rec, other, "cut", "radio");
    let item = w.item(file);
    assert_eq!(item.label, Label::NewToYou);
    assert_eq!(item.reason.key(), "inbox:reason.newToYou");
    assert_eq!(item.library_track_id, None);
}

#[test]
fn a_file_of_a_library_track_that_is_no_better_than_its_linked_file_is_already_have() {
    let w = World::new();
    let (track, rec, _) = w.library_track("lib.mp3");
    let [file] = w.arrive(&["a.mp3"])[..] else {
        panic!()
    };
    w.format(file, "mp3", Some(320));
    w.decide(file, rec);
    let item = w.item(file);
    assert_eq!(item.label, Label::AlreadyHave);
    assert_eq!(item.reason.key(), "inbox:reason.alreadyHave");
    assert_eq!(item.library_track_id, Some(track));
}

#[test]
fn a_flac_of_a_library_mp3_is_a_better_file_and_the_formats_are_the_reason() {
    let w = World::new();
    let (track, rec, _) = w.library_track("lib.mp3");
    let [file] = w.arrive(&["a.flac"])[..] else {
        panic!()
    };
    w.format(file, "flac", None);
    w.decide(file, rec);
    let item = w.item(file);
    assert_eq!(item.label, Label::BetterFile);
    // The Review screen's reason, with the codecs as data.
    assert_eq!(item.reason.key(), "review:reason.betterFormat");
    assert_eq!(text(&item, "best").as_deref(), Some("FLAC"));
    assert_eq!(text(&item, "other").as_deref(), Some("MP3"));
    assert_eq!(item.library_track_id, Some(track));
}

#[test]
fn another_cut_of_a_library_track_is_other_cut_with_the_links_label() {
    let w = World::new();
    let (track, lib_rec, _) = w.library_track("lib.mp3");
    let [file] = w.arrive(&["radio.mp3"])[..] else {
        panic!()
    };
    let rec = w.recording();
    w.decide(file, rec);
    // The link may be written either way round.
    w.link(lib_rec, rec, "cut", "radio");
    let item = w.item(file);
    assert_eq!(item.label, Label::OtherCut);
    assert_eq!(item.reason.key(), "inbox:reason.otherCut");
    assert_eq!(text(&item, "label").as_deref(), Some("radio"));
    assert_eq!(item.library_track_id, Some(track));
}

#[test]
fn a_rework_of_a_library_track_is_rework_and_a_cut_link_wins_over_a_rework_one() {
    let w = World::new();
    let (track, lib_rec, _) = w.library_track("lib.mp3");
    let (_, other_rec, _) = w.library_track("other.mp3");
    let [file] = w.arrive(&["remix.mp3"])[..] else {
        panic!()
    };
    let rec = w.recording();
    w.decide(file, rec);
    w.link(rec, lib_rec, "rework", "remix");
    let item = w.item(file);
    assert_eq!(item.label, Label::Rework);
    assert_eq!(item.reason.key(), "inbox:reason.rework");
    assert_eq!(text(&item, "label").as_deref(), Some("remix"));
    assert_eq!(item.library_track_id, Some(track));
    // It is also a cut of another track you own: the cut is what it says.
    w.link(rec, other_rec, "cut", "edit");
    assert_eq!(w.item(file).label, Label::OtherCut);
}

#[test]
fn a_broken_row_gets_checking_or_is_left_out_and_the_rest_of_the_page_returns() {
    let w = World::new();
    let (_, rec, _) = w.library_track("lib.mp3");
    let [unplaced, no_row, gap, unknown, good] =
        w.arrive(&["a.mp3", "b.mp3", "c.mp3", "d.mp3", "e.mp3"])[..]
    else {
        panic!()
    };
    // Decided, but its track row is gone from `recording_file`.
    w.exec(&format!(
        "INSERT INTO grouping_file (file_id, decided_as) VALUES ({no_row}, 3)
             ON CONFLICT (file_id) DO UPDATE SET decided_as = 3;"
    ));
    // In a Library track that has no linked file (missing).
    w.exec(&format!(
        "UPDATE library_track SET linked_file_id = NULL, source_status = 'missing'
         WHERE recording_id = {rec}"
    ));
    w.decide(gap, rec);
    // An item whose status this build doesn't know.
    w.exec(&format!(
        "PRAGMA ignore_check_constraints = ON;
         UPDATE inbox_item SET status = 'newer_kind' WHERE file_id = {unknown};
         PRAGMA ignore_check_constraints = OFF;"
    ));
    let other = w.recording();
    w.decide(good, other);
    let page = w
        .writer
        .call(|c| items(c, &InboxFilter::default()))
        .unwrap();
    let label_of = |file: i64| page.iter().find(|i| i.file_id == file).map(|i| i.label);
    assert_eq!(label_of(unplaced), Some(Label::Checking));
    assert_eq!(label_of(no_row), Some(Label::Checking));
    assert_eq!(label_of(gap), Some(Label::AlreadyHave));
    assert_eq!(label_of(unknown), None);
    assert_eq!(label_of(good), Some(Label::NewToYou));
    // The badge reads on.
    assert!(w.writer.call(|c| count(c)).is_ok());
}

#[test]
fn links_that_cannot_be_read_give_checking_not_new_to_you() {
    let w = World::new();
    let [file] = w.arrive(&["a.mp3"])[..] else {
        panic!()
    };
    let rec = w.recording();
    w.decide(file, rec);
    assert_eq!(w.item(file).label, Label::NewToYou);
    w.exec("ALTER TABLE version_link RENAME COLUMN recording_a TO gone;");
    assert_eq!(w.item(file).label, Label::Checking);
}

#[test]
fn the_filter_and_the_counts_go_by_label_and_the_counts_add_up_to_new() {
    let w = World::new();
    let (_, lib_rec, _) = w.library_track("lib.mp3");
    let [checking, fresh, have, better, hidden] =
        w.arrive(&["a.mp3", "b.mp3", "c.mp3", "d.flac", "e.mp3"])[..]
    else {
        panic!()
    };
    let rec = w.recording();
    w.decide(fresh, rec);
    w.format(have, "mp3", Some(320));
    w.decide(have, lib_rec);
    w.format(better, "flac", None);
    w.decide(better, lib_rec);
    // Decided and put aside: not counted, still filterable by its label.
    let rec = w.recording();
    w.decide(hidden, rec);
    w.exec(&format!(
        "UPDATE inbox_item SET status = 'not_now', decided_at = '2026-01-02T00:00:00.000Z'
         WHERE file_id = {hidden}"
    ));

    let by = |label: Label| -> Vec<i64> {
        let filter = InboxFilter {
            label: Some(label),
            ..Default::default()
        };
        let mut ids: Vec<i64> = w
            .writer
            .call(move |c| items(c, &filter))
            .unwrap()
            .iter()
            .map(|i| i.file_id)
            .collect();
        ids.sort_unstable();
        ids
    };
    assert_eq!(by(Label::Checking), [checking]);
    assert_eq!(by(Label::NewToYou), [fresh, hidden]);
    assert_eq!(by(Label::AlreadyHave), [have]);
    assert_eq!(by(Label::BetterFile), [better]);
    assert!(by(Label::OtherCut).is_empty() && by(Label::Rework).is_empty());
    // Together with a status filter.
    let filter = InboxFilter {
        label: Some(Label::NewToYou),
        status: Some(super::ItemStatus::New),
        ..Default::default()
    };
    let new_only = w.writer.call(move |c| items(c, &filter)).unwrap();
    assert_eq!(new_only.len(), 1);

    let counted = w.writer.call(|c| count(c)).unwrap();
    assert_eq!(counted.new, 4);
    assert_eq!(
        (
            counted.by_label.checking,
            counted.by_label.new_to_you,
            counted.by_label.already_have,
            counted.by_label.better_file,
            counted.by_label.other_cut,
            counted.by_label.rework,
        ),
        (1, 1, 1, 1, 0, 0)
    );
}

/// A walk of the Inbox folder that sees `names` but couldn't list a
/// subfolder.
fn walk_incomplete(writer: &Writer, names: &[&str]) {
    let files: Vec<_> = names.iter().zip(1..).map(|(n, s)| found(n, s)).collect();
    writer
        .call(move |c| {
            upsert_with(c, INBOX, &files, START, &[])?;
            let unread = Unread {
                folders: 1,
                ..Default::default()
            };
            finish(c, INBOX, &unread, START).map(|_| ())
        })
        .unwrap();
}

fn waiting(writer: &Writer) -> bool {
    let counted = writer.call(|c| count(c)).unwrap();
    counted
        .folders
        .iter()
        .find(|f| f.music_folder_id == INBOX.0)
        .expect("the Inbox folder is listed")
        .waiting_for_first_look
}

#[test]
fn the_third_incomplete_walk_in_a_row_takes_the_baseline_and_the_folder_stops_waiting() {
    let (_dir, writer) = library();
    assert!(waiting(&writer));
    walk_incomplete(&writer, &["a.mp3"]);
    walk_incomplete(&writer, &["a.mp3"]);
    assert!(waiting(&writer));
    assert!(all(&writer).is_empty());

    walk_incomplete(&writer, &["a.mp3"]);
    assert!(!waiting(&writer));
    let there = all(&writer);
    assert_eq!(there.len(), 1);
    assert_eq!(there[0].batch_kind, super::BatchKind::AlreadyThere);

    // What comes after is news again.
    walk_incomplete(&writer, &["a.mp3", "b.mp3"]);
    hashed(&writer, "b.mp3");
    assert_eq!(intake(&writer), 1);
    assert_eq!(writer.call(|c| count(c)).unwrap().new, 1);
}

#[test]
fn a_complete_walk_takes_the_baseline_and_the_count_of_incomplete_ones_starts_over() {
    let (_dir, writer) = library();
    walk_incomplete(&writer, &["a.mp3"]);
    walk_incomplete(&writer, &["a.mp3"]);
    walk(&writer, INBOX, vec![found("a.mp3", 1)]);
    assert!(!waiting(&writer));
    assert_eq!(all(&writer).len(), 1);
    // A scan folder is never waiting for a first look: it is not listed.
    let folders = writer.call(|c| count(c)).unwrap().folders;
    assert_eq!(folders.len(), 1);
}

#[test]
fn removing_a_folder_removes_its_count_of_incomplete_walks() {
    let (_dir, writer) = library();
    walk_incomplete(&writer, &["a.mp3"]);
    let key = format!("inbox.incompleteWalks.{}", INBOX.0);
    let rows = |w: &Writer| -> i64 {
        let key = key.clone();
        w.call(move |c| {
            c.query_row("SELECT COUNT(*) FROM setting WHERE key = ?1", [key], |r| {
                r.get(0)
            })
        })
        .unwrap()
    };
    assert_eq!(rows(&writer), 1);
    writer
        .call(|c| crate::scan::folders::remove(c, INBOX))
        .unwrap()
        .unwrap();
    assert_eq!(rows(&writer), 0);
}
