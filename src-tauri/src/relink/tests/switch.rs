//! Switching a Library track's linked file in Review (1bE-7;
//! [`crate::review::actions`]), through grouping's rows, relink, the follow,
//! the best-file ranking, the Review queue, a send and the operation log.
//! Everything is made up: no file is read.

use super::follow::Drives;
use super::*;
use crate::after_send::{manual_removals, RemovalReason};
use crate::library::LibraryTrackId;
use crate::ops::{UndoOutcome, UndoRefusal};
use crate::rekordbox::RekordboxXml;
use crate::rekordbox_write::{build, record_send, Node, Outgoing, SendInput, TrackInput};
use crate::review::actions::{
    accept_all_covers, accept_all_items, accept_clear_best_files, switch_linked_file, AcceptItem,
    AcceptedAll, Refused, SkipReason, SkippedItem, SwitchEffect, SwitchOffer, SwitchRefusal,
    Switched, ACCEPT_ALL_OPERATION, SWITCH_OPERATION,
};
use crate::review::{ReviewCounts, ReviewGroup, ReviewItemId, ReviewKind};
use crate::send_values::{send_values_one, SentAs};

/// What rekordbox holds for a Library track whose linked file went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rekordbox {
    /// No entry at all.
    Unknown,
    /// Its entry, and relink trusts it on the copy: the copy is merged with
    /// the gone file, lasts as long and is titled as rekordbox names it.
    TrustsTheCopy,
    /// Its entry, matched to the copy only probably (the titles disagree).
    ProbableOnTheCopy,
    /// Its entry, matched to no file (the copy was never compared with the
    /// gone file).
    NoMatch,
}

/// A Library track whose linked file `E:\Music\<name>.mp3` is gone, with
/// another encode of it on disk in its track.
#[derive(Debug, Clone, Copy)]
struct Item {
    old: i64,
    copy: i64,
    recording: i64,
    track: LibraryTrackId,
    /// rekordbox's entry at the old file's path.
    row: Option<i64>,
}

fn old_path(name: &str) -> String {
    format!("E:/Music/{name}.mp3")
}

/// Makes one [`Item`] named `name` (its audio and length come from `n`, so
/// items never look alike). Call [`settle`] once every item is made.
fn item(lib: &Lib, music: i64, mounted: &Mounted, name: &str, n: u8, rekordbox: Rekordbox) -> Item {
    let ms = 200_000 + i64::from(n) * 30_000;
    let old = lib.file(music, &format!("{name}.mp3"), Some(ms));
    lib.hashed(old, n * 2 + 1);
    let recording = lib.group(&[old]);
    let row = (rekordbox != Rekordbox::Unknown).then(|| {
        let seconds = (ms / 1000).to_string();
        let row = lib.track_with(&loc(&old_path(name)), Some(&seconds), &[("Name", name)]);
        lib.relink(mounted);
        assert_eq!(lib.matched(row), path(old));
        row
    });
    let track = lib.add(old);

    lib.gone(old);
    let copy = lib.file(music, &format!("Copies/{name}.flac"), Some(ms));
    lib.hashed(copy, n * 2 + 2);
    match rekordbox {
        Rekordbox::Unknown | Rekordbox::TrustsTheCopy => {
            lib.title(copy, name);
            lib.joins(copy, old);
        }
        Rekordbox::ProbableOnTheCopy => {
            lib.title(copy, "Something else");
            lib.joins(copy, old);
        }
        Rekordbox::NoMatch => lib.in_the_track_of(copy, old),
    }
    Item {
        old,
        copy,
        recording,
        track,
        row,
    }
}

/// The relink job and the ranking, as they run after a scan.
fn settle(lib: &Lib, mounted: &Mounted) {
    lib.job(mounted);
    lib.roles();
}

/// One item, settled, with the library it is in.
fn one(rekordbox: Rekordbox) -> (Lib, Mounted, Item) {
    let (lib, music, mounted) = e_music();
    let item = item(&lib, music, &mounted, "Kappa", 0, rekordbox);
    settle(&lib, &mounted);
    // The item is in the queue, and nothing has moved on its own.
    assert_eq!(lib.linked_file(item.track), item.old);
    assert_eq!(lib.clear_items(), [item.recording]);
    match (rekordbox, item.row) {
        (Rekordbox::Unknown, _) => assert_eq!(lib.sent(item.track), SentAs::Nothing),
        (_, Some(row)) => {
            assert_eq!(lib.sent(item.track), SentAs::Entry(row));
            let expected = match rekordbox {
                Rekordbox::TrustsTheCopy => same_track(item.copy),
                Rekordbox::ProbableOnTheCopy => Some((item.copy, "fingerprint".to_owned(), 0.4)),
                _ => None,
            };
            assert_eq!(lib.matched(row), expected);
        }
        (_, None) => unreachable!(),
    }
    (lib, mounted, item)
}

impl Lib {
    /// Puts `file` into the track of `mate` without a word from grouping
    /// about the two: they share a track, but were never compared.
    fn in_the_track_of(&self, file: i64, mate: i64) {
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO recording_file (recording_id, file_id)
                     SELECT recording_id, ?1 FROM recording_file WHERE file_id = ?2",
                    [file, mate],
                )
            })
            .unwrap();
    }

    fn switch(&self, recording: i64, to: i64, shown: SwitchEffect) -> Result<Switched, Refused> {
        self.writer
            .call(move |c| Ok(switch_linked_file(c, &Drives, recording, to, shown)))
            .unwrap()
    }

    /// The items "Accept all" would switch now: what its button counts.
    fn clear_cases(&self) -> Vec<AcceptItem> {
        self.writer.call(|c| accept_all_items(c, &Drives)).unwrap()
    }

    /// "Accept all", for the items the screen showed.
    fn accept(&self, shown: &[AcceptItem]) -> AcceptedAll {
        let shown = shown.to_vec();
        self.writer
            .call(move |c| Ok(accept_clear_best_files(c, &Drives, &shown)))
            .unwrap()
            .unwrap_or_else(|e| panic!("{e:?}"))
    }

    /// The track was sent in a crate, and the send was recorded.
    fn sent_and_recorded(&self, track: LibraryTrackId) {
        let (out, _read) = self.send_in_a_crate(track);
        assert_eq!(out.left_out, []);
        self.writer
            .call(move |c| record_send(c, &out.sent, out.paths()))
            .unwrap();
    }

    /// What Undo would do now, as the Undo button asks: the kind, and why
    /// it would be refused.
    fn next_undo(&self) -> (Option<String>, Option<UndoRefusal>) {
        let next = self
            .writer
            .call(|c| Ok(crate::ops::next_undo(c)))
            .unwrap()
            .unwrap();
        (next.operation.map(|o| o.kind), next.refusal)
    }

    /// Undo, as the Undo button runs it.
    fn undo_last(&self) -> UndoOutcome {
        self.writer
            .call(|c| Ok(crate::ops::undo_for_user(c, None)))
            .unwrap()
            .unwrap()
    }

    fn counts(&self) -> ReviewCounts {
        self.writer.call(|c| crate::review::counts(c)).unwrap()
    }

    /// The tracks of the "Linked file gone, copy found" items.
    fn clear_items(&self) -> Vec<i64> {
        self.writer
            .call(|c| crate::review::items(c, &Drives, ReviewGroup::ClearBestFile, 0, 100))
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.id.a)
            .collect()
    }

    /// The switch the item of `recording` offers, as the screen gets it.
    fn offer(&self, recording: i64) -> Option<SwitchOffer> {
        let id = ReviewItemId {
            kind: ReviewKind::ClearBestFile,
            a: recording,
            b: None,
        };
        self.writer
            .call(move |c| crate::review::comparison(c, &Drives, id))
            .unwrap()
            .and_then(|comparison| comparison.switch)
    }

    /// The tracks to remove in rekordbox: why, and the path shown.
    fn to_remove(&self) -> Vec<(RemovalReason, Option<String>)> {
        self.writer
            .call(|c| manual_removals(c, &Drives))
            .unwrap()
            .into_iter()
            .map(|r| (r.reason, r.path))
            .collect()
    }

    /// Sends `track` in one crate: what was written, and read back.
    fn send_in_a_crate(&self, track: LibraryTrackId) -> (Outgoing, RekordboxXml) {
        let values = self
            .writer
            .call(move |c| send_values_one(c, &Drives, track))
            .unwrap();
        let highest: Option<i64> = self
            .writer
            .call(|c| {
                c.query_row("SELECT max(track_id) FROM rekordbox_track", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        let input = SendInput {
            tracks: vec![TrackInput::from(values)],
            crates: vec![Node::Playlist {
                name: "Warm up".into(),
                entries: vec![track],
            }],
            playlists: Vec::new(),
            highest_rekordbox_track_id: u64::try_from(highest.unwrap_or(0)).unwrap(),
        };
        let out = build(&input).unwrap();
        let read = RekordboxXml::parse(out.xml()).unwrap();
        (out, read)
    }

    fn count(&self, sql: &'static str) -> i64 {
        self.writer
            .call(move |c| c.query_row(sql, [], |r| r.get(0)))
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
}

/// The path rekordbox's entry of an [`Item`] named `name` shows.
fn shown_old(name: &str) -> String {
    format!(r"E:\Music\{name}.mp3")
}

/// The `Location` a send writes for the copy of an [`Item`] named `name`.
fn copy_location(name: &str) -> String {
    format!("file://localhost/E:/Music/Copies/{name}.flac")
}

fn refusal(result: Result<Switched, Refused>) -> SwitchRefusal {
    match result {
        Err(Refused::Switch(refusal)) => refusal,
        other => panic!("not a refused switch: {other:?}"),
    }
}

// ---- the refusals -----------------------------------------------------------

#[test]
fn a_switch_is_refused_when_the_linked_file_is_on_disk_after_all() {
    let (lib, _mounted, item) = one(Rekordbox::Unknown);
    lib.back(item.old);
    assert_eq!(
        refusal(lib.switch(item.recording, item.copy, SwitchEffect::NotInRekordbox)),
        SwitchRefusal::LinkedOnDisk
    );
    assert_eq!(lib.linked_file(item.track), item.old);
    // The item says so too, before anything is pressed.
    assert_eq!(
        lib.writer
            .call(move |c| crate::review::actions::offer(c, &Drives, item.recording, item.copy))
            .unwrap(),
        Some(SwitchOffer {
            file_id: item.copy,
            effect: None,
            refusal: Some(SwitchRefusal::LinkedOnDisk),
        })
    );
}

#[test]
fn a_switch_is_refused_to_a_file_that_is_not_in_the_track() {
    let (lib, music, mounted) = e_music();
    let item = item(&lib, music, &mounted, "Kappa", 0, Rekordbox::Unknown);
    let stranger = lib.file(music, "Stranger.mp3", Some(200_000));
    lib.hashed(stranger, 90);
    lib.group(&[stranger]);
    settle(&lib, &mounted);
    assert_eq!(
        refusal(lib.switch(item.recording, stranger, SwitchEffect::NotInRekordbox)),
        SwitchRefusal::NotInTrack
    );
    // A file that doesn't exist isn't in the track either.
    assert_eq!(
        refusal(lib.switch(item.recording, 9_999, SwitchEffect::NotInRekordbox)),
        SwitchRefusal::NotInTrack
    );
    assert_eq!(lib.linked_file(item.track), item.old);
}

#[test]
fn a_switch_is_refused_to_a_file_that_is_gone_itself() {
    let (lib, _mounted, item) = one(Rekordbox::Unknown);
    lib.gone(item.copy);
    assert_eq!(
        refusal(lib.switch(item.recording, item.copy, SwitchEffect::NotInRekordbox)),
        SwitchRefusal::TargetGone
    );
    assert_eq!(lib.linked_file(item.track), item.old);
}

#[test]
fn a_switch_is_refused_to_a_file_another_library_track_is_linked_to() {
    let (lib, _mounted, item) = one(Rekordbox::Unknown);
    // Another Library track holds the copy (two Library tracks that are one
    // recording, written straight into the table).
    lib.writer
        .call(move |c| {
            c.execute("INSERT INTO recording DEFAULT VALUES", [])?;
            c.execute(
                "INSERT INTO library_track (recording_id, kind, linked_file_id, source_status)
                 VALUES (last_insert_rowid(), 'linked', ?1, 'ok')",
                [item.copy],
            )
        })
        .unwrap();
    assert_eq!(
        refusal(lib.switch(item.recording, item.copy, SwitchEffect::NotInRekordbox)),
        SwitchRefusal::TargetLinked
    );
    assert_eq!(lib.linked_file(item.track), item.old);
}

#[test]
fn a_switch_is_refused_when_it_would_take_the_entry_another_library_track_is_sent_as() {
    let (lib, music, mounted) = e_music();
    let first = item(&lib, music, &mounted, "Kappa", 0, Rekordbox::Unknown);
    let second = item(&lib, music, &mounted, "Lambda", 1, Rekordbox::NoMatch);
    settle(&lib, &mounted);
    // rekordbox's entry of the second track is matched, probably, to the
    // first track's copy; the second track is still sent as that entry.
    let row = second.row.unwrap();
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE rekordbox_track
                 SET file_id = ?2, relink_method = 'filename_only', relink_confidence = 0.5,
                     relink_probable = 1
                 WHERE id = ?1",
                [row, first.copy],
            )
        })
        .unwrap();
    assert_eq!(lib.sent(second.track), SentAs::Entry(row));

    assert_eq!(
        refusal(lib.switch(first.recording, first.copy, SwitchEffect::NotInRekordbox)),
        SwitchRefusal::EntryOfAnotherTrack
    );
    assert_eq!(lib.linked_file(first.track), first.old);
    assert_eq!(lib.sent(second.track), SentAs::Entry(row));
    // "Accept all" neither counts it nor makes it.
    assert!(!lib
        .clear_cases()
        .iter()
        .any(|i| i.recording_id == first.recording));
    let shown = [AcceptItem {
        recording_id: first.recording,
        file_id: first.copy,
    }];
    assert_eq!(
        lib.accept(&shown).skipped,
        [SkippedItem {
            recording_id: first.recording,
            reason: SkipReason::EntryOfAnotherTrack,
        }]
    );
    assert_eq!(lib.linked_file(first.track), first.old);
}

#[test]
fn a_track_that_is_not_in_the_library_has_nothing_to_switch() {
    let (lib, music, mounted) = e_music();
    let file = lib.file(music, "Loose.mp3", Some(200_000));
    let recording = lib.group(&[file]);
    settle(&lib, &mounted);
    assert!(matches!(
        lib.switch(recording, file, SwitchEffect::NotInRekordbox),
        Err(Refused::ItemGone)
    ));
}

// ---- what a switch means in rekordbox, through a send ------------------------

#[test]
fn where_relink_trusts_rekordboxs_entry_on_the_copy_the_switched_track_stays_that_entry() {
    let (lib, mounted, item) = one(Rekordbox::TrustsTheCopy);
    let row = item.row.unwrap();
    // The item says what the switch means before it is made.
    assert_eq!(
        lib.offer(item.recording),
        Some(SwitchOffer {
            file_id: item.copy,
            effect: Some(SwitchEffect::KeepsEntry),
            refusal: None,
        })
    );

    let switched = lib
        .switch(item.recording, item.copy, SwitchEffect::KeepsEntry)
        .unwrap();
    assert!(switched.operation_id.is_some());
    assert_eq!(switched.track.from_file_id, item.old);
    assert_eq!(switched.track.to_file_id, item.copy);
    assert_eq!(lib.linked_file(item.track), item.copy);
    // The item left the queue as the switch returned: no job ran.
    assert_eq!(lib.clear_items(), Vec::<i64>::new());
    assert_eq!(lib.counts().total, 0);

    // A send: rekordbox's own entry, at its old Location, in its crate.
    assert_eq!(lib.sent(item.track), SentAs::Entry(row));
    let (out, read) = lib.send_in_a_crate(item.track);
    assert_eq!(out.left_out, []);
    assert_eq!(out.sent.len(), 1);
    assert!(out.sent[0].in_rekordbox);
    assert_eq!(out.sent[0].location(), loc(&old_path("Kappa")));
    assert_eq!(read.playlists.playlists()[0].1.entries.len(), 1);
    // Nothing was left behind in rekordbox, so nothing is listed to remove.
    assert_eq!(lib.count("SELECT count(*) FROM replaced_entry"), 0);
    assert_eq!(lib.to_remove(), []);

    // The same after the jobs have run again.
    settle(&lib, &mounted);
    assert_eq!(lib.linked_file(item.track), item.copy);
    assert_eq!(lib.sent(item.track), SentAs::Entry(row));
    assert_eq!(lib.to_remove(), []);
}

#[test]
fn where_relink_does_not_trust_the_entry_the_switched_track_goes_out_new_and_the_old_entry_is_listed(
) {
    for rekordbox in [Rekordbox::ProbableOnTheCopy, Rekordbox::NoMatch] {
        let (lib, mounted, item) = one(rekordbox);
        assert_eq!(
            lib.offer(item.recording).and_then(|offer| offer.effect),
            Some(SwitchEffect::NewTrack),
            "{rekordbox:?}"
        );
        // Before the switch nothing is listed: the entry is the track's own.
        assert_eq!(lib.to_remove(), []);

        lib.switch(item.recording, item.copy, SwitchEffect::NewTrack)
            .unwrap();
        assert_eq!(lib.linked_file(item.track), item.copy);
        assert_eq!(lib.clear_items(), Vec::<i64>::new());

        // A send: a new track at the copy's place, in its crate.
        assert_eq!(lib.sent(item.track), SentAs::New, "{rekordbox:?}");
        let (out, read) = lib.send_in_a_crate(item.track);
        assert_eq!(out.left_out, []);
        assert_eq!(out.sent.len(), 1);
        assert!(!out.sent[0].in_rekordbox);
        assert_eq!(out.sent[0].location(), copy_location("Kappa"));
        assert_eq!(read.playlists.playlists()[0].1.entries.len(), 1);
        // The old entry is on the list of tracks to remove in rekordbox,
        // and it is not what was sent.
        assert_eq!(
            lib.to_remove(),
            [(RemovalReason::Replaced, Some(shown_old("Kappa")))],
            "{rekordbox:?}"
        );
        assert_ne!(out.sent[0].location(), loc(&old_path("Kappa")));

        // The same after the jobs have run again.
        settle(&lib, &mounted);
        assert_eq!(lib.linked_file(item.track), item.copy);
        assert_eq!(lib.sent(item.track), SentAs::New);
        assert_eq!(lib.to_remove().len(), 1);
    }
}

#[test]
fn once_the_old_entry_is_trusted_on_the_linked_file_it_is_sent_again_and_no_longer_listed() {
    let (lib, mounted, item) = one(Rekordbox::ProbableOnTheCopy);
    let row = item.row.unwrap();
    lib.switch(item.recording, item.copy, SwitchEffect::NewTrack)
        .unwrap();
    assert_eq!(lib.to_remove().len(), 1);

    // The user confirms the entry on the copy (1bE-6), and relink applies it.
    lib.confirm_now(&loc(&old_path("Kappa")), item.copy, Method::Fingerprint);
    lib.relink(&mounted);
    assert!(!lib.probable(row));

    // One entry is never both sent and listed for removal.
    assert_eq!(lib.sent(item.track), SentAs::Entry(row));
    let (out, _read) = lib.send_in_a_crate(item.track);
    assert!(out.sent[0].in_rekordbox);
    assert_eq!(out.sent[0].location(), loc(&old_path("Kappa")));
    assert_eq!(lib.to_remove(), []);
    // The record is still there: it is listed again if the match goes.
    assert_eq!(lib.count("SELECT count(*) FROM replaced_entry"), 1);
}

#[test]
fn the_old_entry_leaves_the_list_once_a_read_no_longer_has_it() {
    let (lib, _mounted, item) = one(Rekordbox::NoMatch);
    lib.switch(item.recording, item.copy, SwitchEffect::NewTrack)
        .unwrap();
    assert_eq!(lib.to_remove().len(), 1);
    // The user removed it in rekordbox; the next read doesn't hold it.
    lib.writer
        .call(|c| c.execute("DELETE FROM rekordbox_track", []))
        .unwrap();
    assert_eq!(lib.to_remove(), []);
}

#[test]
fn the_old_entry_stays_listed_after_the_switched_track_is_removed_from_the_library() {
    let (lib, _mounted, item) = one(Rekordbox::NoMatch);
    lib.switch(item.recording, item.copy, SwitchEffect::NewTrack)
        .unwrap();
    crate::library::remove(&lib.writer, item.track).unwrap();
    // rekordbox still holds the old entry, whatever became of the track.
    assert_eq!(
        lib.to_remove(),
        [(RemovalReason::Replaced, Some(shown_old("Kappa")))]
    );
}

#[test]
fn an_entry_is_listed_once_when_it_is_both_replaced_and_the_entry_of_a_removed_track() {
    let (lib, mounted, item) = one(Rekordbox::NoMatch);
    lib.switch(item.recording, item.copy, SwitchEffect::NewTrack)
        .unwrap();
    // The old file is on disk again: its entry is matched to it by its
    // path, and it is a file of the switched track.
    lib.back(item.old);
    settle(&lib, &mounted);
    assert_eq!(lib.matched(item.row.unwrap()), path(item.old));
    assert_eq!(
        lib.to_remove(),
        [(RemovalReason::Replaced, Some(shown_old("Kappa")))]
    );

    // The track is removed from the Library: rekordbox's entry for it is to
    // be removed for that reason too. One entry, one line.
    crate::library::remove(&lib.writer, item.track).unwrap();
    assert_eq!(
        lib.to_remove(),
        [(RemovalReason::Removed, Some(shown_old("Kappa")))]
    );
}

#[test]
fn where_rekordbox_holds_the_copy_as_a_track_of_its_own_the_switched_track_becomes_that_entry() {
    let (lib, music, mounted) = e_music();
    let item = item(&lib, music, &mounted, "Kappa", 0, Rekordbox::NoMatch);
    // rekordbox has a second entry, at the copy's own path.
    let other = lib.track_with(
        &loc("E:/Music/Copies/Kappa.flac"),
        Some("200"),
        &[("Name", "Kappa (copy)")],
    );
    settle(&lib, &mounted);
    let row = item.row.unwrap();
    assert_eq!(lib.matched(other), path(item.copy));
    assert_eq!(lib.sent(item.track), SentAs::Entry(row));
    assert_eq!(
        lib.offer(item.recording).and_then(|offer| offer.effect),
        Some(SwitchEffect::OtherEntry)
    );
    // Not a clear case: "Accept all" leaves it, with its own reason.
    assert_eq!(lib.clear_cases(), []);
    let shown = [AcceptItem {
        recording_id: item.recording,
        file_id: item.copy,
    }];
    assert_eq!(
        lib.accept(&shown).skipped,
        [SkippedItem {
            recording_id: item.recording,
            reason: SkipReason::OtherEntry,
        }]
    );
    assert_eq!(lib.linked_file(item.track), item.old);

    lib.switch(item.recording, item.copy, SwitchEffect::OtherEntry)
        .unwrap();
    // Sent as the copy's own entry; the old one is listed to remove.
    assert_eq!(lib.sent(item.track), SentAs::Entry(other));
    let (out, _read) = lib.send_in_a_crate(item.track);
    assert!(out.sent[0].in_rekordbox);
    assert_eq!(out.sent[0].location(), loc("E:/Music/Copies/Kappa.flac"));
    assert_eq!(
        lib.to_remove(),
        [(RemovalReason::Replaced, Some(shown_old("Kappa")))]
    );
    // Nothing new arrives in rekordbox, so Undo works after a send too.
    lib.sent_and_recorded(item.track);
    assert!(matches!(lib.undo_last(), UndoOutcome::Undone { .. }));
    assert_eq!(lib.linked_file(item.track), item.old);
    assert_eq!(lib.sent(item.track), SentAs::Entry(row));
    assert_eq!(lib.to_remove(), []);
}

#[test]
fn a_track_rekordbox_does_not_know_is_switched_and_sent_from_the_copy() {
    let (lib, _mounted, item) = one(Rekordbox::Unknown);
    assert_eq!(
        lib.offer(item.recording).and_then(|offer| offer.effect),
        Some(SwitchEffect::NotInRekordbox)
    );
    lib.switch(item.recording, item.copy, SwitchEffect::NotInRekordbox)
        .unwrap();
    assert_eq!(lib.linked_file(item.track), item.copy);
    assert_eq!(lib.sent(item.track), SentAs::New);
    let (out, read) = lib.send_in_a_crate(item.track);
    assert_eq!(out.left_out, []);
    assert_eq!(out.sent[0].location(), copy_location("Kappa"));
    assert_eq!(read.playlists.playlists()[0].1.entries.len(), 1);
    assert_eq!(lib.to_remove(), []);
}

#[test]
fn the_switch_is_not_made_when_it_no_longer_means_what_the_item_showed() {
    // The item showed "rekordbox keeps the track"; a relink since then no
    // longer trusts the entry on the copy.
    let (lib, mounted, item) = one(Rekordbox::TrustsTheCopy);
    lib.title(item.copy, "Renamed since");
    lib.decided(item.copy);
    lib.relink(&mounted);
    assert!(lib.probable(item.row.unwrap()));

    assert!(matches!(
        lib.switch(item.recording, item.copy, SwitchEffect::KeepsEntry),
        Err(Refused::Changed)
    ));
    assert_eq!(lib.linked_file(item.track), item.old);
    assert_eq!(lib.count("SELECT count(*) FROM replaced_entry"), 0);
    assert!(!lib.kinds().contains(&SWITCH_OPERATION.to_owned()));
    // Shown again, the item says what it means now, and that switch works.
    assert_eq!(
        lib.offer(item.recording).and_then(|offer| offer.effect),
        Some(SwitchEffect::NewTrack)
    );
    lib.switch(item.recording, item.copy, SwitchEffect::NewTrack)
        .unwrap();
    assert_eq!(lib.linked_file(item.track), item.copy);
}

// ---- undo ---------------------------------------------------------------------

#[test]
fn undo_of_a_switch_restores_the_link_brings_the_item_back_and_takes_the_record_away() {
    let (lib, _mounted, item) = one(Rekordbox::NoMatch);
    let row = item.row.unwrap();
    lib.switch(item.recording, item.copy, SwitchEffect::NewTrack)
        .unwrap();
    assert_eq!(
        lib.kinds().last().map(String::as_str),
        Some(SWITCH_OPERATION)
    );
    assert_eq!(lib.to_remove().len(), 1);

    assert!(
        matches!(lib.undo_last(), UndoOutcome::Undone { operation } if operation.kind == SWITCH_OPERATION)
    );
    assert_eq!(lib.linked_file(item.track), item.old);
    // Back in the queue as the undo returned, and sent as its entry again.
    assert_eq!(lib.clear_items(), [item.recording]);
    assert_eq!(lib.counts().clear_best_file, 1);
    assert_eq!(lib.sent(item.track), SentAs::Entry(row));
    assert_eq!(lib.count("SELECT count(*) FROM replaced_entry"), 0);
    assert_eq!(lib.to_remove(), []);
    // The next undo is the add itself, as before the switch.
    assert!(
        matches!(lib.undo_last(), UndoOutcome::Undone { operation } if operation.kind == "promote")
    );
}

#[test]
fn undo_of_a_switch_that_made_the_track_new_for_rekordbox_is_refused_once_it_was_sent() {
    // A track rekordbox knows whose entry isn't trusted on the copy, and a
    // track rekordbox doesn't know: both go out as a new track.
    for (rekordbox, effect) in [
        (Rekordbox::NoMatch, SwitchEffect::NewTrack),
        (Rekordbox::ProbableOnTheCopy, SwitchEffect::NewTrack),
        (Rekordbox::Unknown, SwitchEffect::NotInRekordbox),
    ] {
        let (lib, _mounted, item) = one(rekordbox);
        lib.switch(item.recording, item.copy, effect).unwrap();
        // Before any send the undo is on offer, as it always was.
        assert_eq!(
            lib.next_undo(),
            (Some(SWITCH_OPERATION.to_owned()), None),
            "{rekordbox:?}"
        );
        let listed = lib.to_remove();

        lib.sent_and_recorded(item.track);
        // The copy's entry is in rekordbox now: the undo is refused, with
        // its own reason, known before the button is pressed.
        assert_eq!(
            lib.next_undo(),
            (
                Some(SWITCH_OPERATION.to_owned()),
                Some(UndoRefusal::SwitchSentSince)
            ),
            "{rekordbox:?}"
        );
        assert!(
            matches!(
                lib.undo_last(),
                UndoOutcome::Refused {
                    reason: UndoRefusal::SwitchSentSince,
                    ..
                }
            ),
            "{rekordbox:?}"
        );
        // Nothing changed.
        assert_eq!(lib.linked_file(item.track), item.copy);
        assert_eq!(lib.to_remove(), listed);
        assert_eq!(
            lib.kinds().last().map(String::as_str),
            Some(SWITCH_OPERATION)
        );
        assert_eq!(lib.clear_items(), Vec::<i64>::new());
    }
}

#[test]
fn undo_of_a_switch_that_kept_rekordboxs_entry_works_after_a_send_too() {
    let (lib, _mounted, item) = one(Rekordbox::TrustsTheCopy);
    let row = item.row.unwrap();
    lib.switch(item.recording, item.copy, SwitchEffect::KeepsEntry)
        .unwrap();
    lib.sent_and_recorded(item.track);
    // The same entry before and after: a send added nothing to rekordbox.
    assert_eq!(lib.next_undo(), (Some(SWITCH_OPERATION.to_owned()), None));
    assert!(matches!(lib.undo_last(), UndoOutcome::Undone { .. }));
    assert_eq!(lib.linked_file(item.track), item.old);
    assert_eq!(lib.sent(item.track), SentAs::Entry(row));
    assert_eq!(lib.clear_items(), [item.recording]);
}

#[test]
fn undo_of_accept_all_is_refused_once_a_track_it_made_new_for_rekordbox_was_sent() {
    let (lib, music, mounted) = e_music();
    let unknown = item(&lib, music, &mounted, "Kappa", 0, Rekordbox::Unknown);
    let trusted = item(&lib, music, &mounted, "Lambda", 1, Rekordbox::TrustsTheCopy);
    settle(&lib, &mounted);
    let accepted = lib.accept(&lib.clear_cases());
    assert_eq!(accepted.switched.len(), 2);
    // The track that kept its entry is sent: nothing stands in the way.
    lib.sent_and_recorded(trusted.track);
    assert_eq!(
        lib.next_undo(),
        (Some(ACCEPT_ALL_OPERATION.to_owned()), None)
    );
    // The one rekordbox didn't know is sent as a new track: now it does.
    lib.sent_and_recorded(unknown.track);
    assert!(matches!(
        lib.undo_last(),
        UndoOutcome::Refused {
            reason: UndoRefusal::SwitchSentSince,
            ..
        }
    ));
    assert_eq!(lib.linked_file(unknown.track), unknown.copy);
    assert_eq!(lib.linked_file(trusted.track), trusted.copy);
}

// ---- nothing pulls a switched track back -----------------------------------------

#[test]
fn a_switched_track_is_not_followed_or_relinked_back_when_the_old_file_returns() {
    for rekordbox in [
        Rekordbox::Unknown,
        Rekordbox::TrustsTheCopy,
        Rekordbox::ProbableOnTheCopy,
    ] {
        let (lib, mounted, item) = one(rekordbox);
        let effect = lib.offer(item.recording).unwrap().effect.unwrap();
        lib.switch(item.recording, item.copy, effect).unwrap();

        // The relink job (relink, then the follow) and the ranking run.
        settle(&lib, &mounted);
        assert_eq!(lib.linked_file(item.track), item.copy, "{rekordbox:?}");
        if rekordbox == Rekordbox::TrustsTheCopy {
            // Relink's tie-break keeps the entry on the linked file.
            assert_eq!(lib.matched(item.row.unwrap()), same_track(item.copy));
        }

        // The old file is on disk again: its entry goes back to its path
        // (relink step 1), and the link stays where the user put it.
        lib.back(item.old);
        settle(&lib, &mounted);
        assert_eq!(lib.linked_file(item.track), item.copy, "{rekordbox:?}");
        if let Some(row) = item.row {
            assert_eq!(lib.matched(row), path(item.old));
        }
        // And again when it goes once more.
        lib.gone(item.old);
        settle(&lib, &mounted);
        assert_eq!(lib.linked_file(item.track), item.copy, "{rekordbox:?}");
        // No follow was ever recorded for the track.
        assert!(!lib.kinds().contains(&"follow_file".to_owned()));
    }
}

// ---- Accept all ------------------------------------------------------------------

#[test]
fn accept_all_covers_only_a_switch_that_leaves_the_track_what_it_is_in_rekordbox() {
    // The one rule, spelled out: the owner may widen it.
    assert!(accept_all_covers(SwitchEffect::NotInRekordbox));
    assert!(accept_all_covers(SwitchEffect::KeepsEntry));
    assert!(!accept_all_covers(SwitchEffect::NewTrack));
    assert!(!accept_all_covers(SwitchEffect::OtherEntry));
}

#[test]
fn accept_all_switches_the_clear_items_as_one_operation_and_one_undo_restores_every_link() {
    let (lib, music, mounted) = e_music();
    let unknown = item(&lib, music, &mounted, "Kappa", 0, Rekordbox::Unknown);
    let trusted = item(&lib, music, &mounted, "Lambda", 1, Rekordbox::TrustsTheCopy);
    let new_track = item(&lib, music, &mounted, "Sigma", 2, Rekordbox::NoMatch);
    // Another group's item: a better file for a track whose file is there.
    let fine = lib.file(music, "Fine.mp3", Some(100_000));
    lib.hashed(fine, 80);
    let better = lib.file(music, "Fine.flac", Some(100_000));
    lib.hashed(better, 81);
    let fine_recording = lib.group(&[fine, better]);
    let fine_track = lib.in_library(fine);
    settle(&lib, &mounted);
    lib.writer
        .call(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO better_file (recording_id, file_id, step)
                 VALUES (?1, ?2, 'format')",
                [fine_recording, better],
            )
        })
        .unwrap();

    let before = lib.counts();
    assert_eq!(before.clear_best_file, 3);
    assert_eq!(before.quality_problems, 1);
    // What the button counts: the clear cases, and not the switch that
    // would arrive in rekordbox as a new track.
    let shown = lib.clear_cases();
    assert_eq!(
        shown,
        [
            AcceptItem {
                recording_id: unknown.recording,
                file_id: unknown.copy,
            },
            AcceptItem {
                recording_id: trusted.recording,
                file_id: trusted.copy,
            },
        ]
    );

    let accepted = lib.accept(&shown);
    let switched: Vec<(i64, i64, i64)> = accepted
        .switched
        .iter()
        .map(|s| (s.recording_id, s.from_file_id, s.to_file_id))
        .collect();
    // Exactly the items sent, each to the copy it showed.
    assert_eq!(
        switched,
        [
            (unknown.recording, unknown.old, unknown.copy),
            (trusted.recording, trusted.old, trusted.copy),
        ]
    );
    assert_eq!(accepted.skipped, []);
    assert_eq!(lib.linked_file(unknown.track), unknown.copy);
    assert_eq!(lib.linked_file(trusted.track), trusted.copy);
    // The switch that would arrive in rekordbox as a new track stays for a
    // decision of its own.
    assert_eq!(lib.linked_file(new_track.track), new_track.old);
    assert_eq!(lib.clear_items(), [new_track.recording]);
    assert_eq!(lib.clear_cases(), []);
    // Nothing in another group was touched.
    assert_eq!(lib.linked_file(fine_track), fine);
    let after = lib.counts();
    assert_eq!(after.quality_problems, before.quality_problems);
    assert_eq!(after.needs_closer_look, before.needs_closer_look);
    assert_eq!(lib.count("SELECT count(*) FROM review_decision"), 0);
    assert_eq!(lib.count("SELECT count(*) FROM replaced_entry"), 0);

    // One operation, named by how many tracks it switched.
    assert_eq!(
        lib.kinds()
            .iter()
            .filter(|k| *k == ACCEPT_ALL_OPERATION)
            .count(),
        1
    );
    assert!(!lib.kinds().contains(&SWITCH_OPERATION.to_owned()));
    let details: String = lib
        .writer
        .call(|c| {
            c.query_row(
                "SELECT details FROM operation ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&details).unwrap()["tracks"],
        serde_json::json!(2)
    );

    // One Undo restores every link, and the items are back.
    assert!(matches!(lib.undo_last(), UndoOutcome::Undone { operation }
        if operation.kind == ACCEPT_ALL_OPERATION && operation.details.tracks == Some(2)));
    assert_eq!(lib.linked_file(unknown.track), unknown.old);
    assert_eq!(lib.linked_file(trusted.track), trusted.old);
    assert_eq!(
        lib.clear_items(),
        [unknown.recording, trusted.recording, new_track.recording]
    );
}

#[test]
fn accept_all_never_switches_an_item_the_screen_did_not_show() {
    let (lib, music, mounted) = e_music();
    let seen = item(&lib, music, &mounted, "Kappa", 0, Rekordbox::Unknown);
    settle(&lib, &mounted);
    let shown = lib.clear_cases();
    assert_eq!(shown.len(), 1);

    // Another clear item joins the queue before the button is pressed.
    let unseen = item(&lib, music, &mounted, "Lambda", 1, Rekordbox::Unknown);
    settle(&lib, &mounted);
    assert_eq!(lib.clear_cases().len(), 2);

    let accepted = lib.accept(&shown);
    assert_eq!(accepted.switched.len(), 1);
    assert_eq!(accepted.switched[0].recording_id, seen.recording);
    assert_eq!(accepted.skipped, []);
    assert_eq!(lib.linked_file(seen.track), seen.copy);
    assert_eq!(lib.linked_file(unseen.track), unseen.old);
    assert_eq!(lib.clear_items(), [unseen.recording]);
}

#[test]
fn an_item_that_is_no_longer_valid_when_accept_all_is_applied_is_skipped_with_its_reason() {
    let (lib, music, mounted) = e_music();
    let stays = item(&lib, music, &mounted, "Kappa", 0, Rekordbox::Unknown);
    let back = item(&lib, music, &mounted, "Lambda", 1, Rekordbox::Unknown);
    let untrusted = item(&lib, music, &mounted, "Sigma", 2, Rekordbox::TrustsTheCopy);
    let other_copy = item(&lib, music, &mounted, "Omega", 3, Rekordbox::Unknown);
    let copy_gone = item(&lib, music, &mounted, "Theta", 4, Rekordbox::Unknown);
    settle(&lib, &mounted);
    let shown = lib.clear_cases();
    assert_eq!(shown.len(), 5);

    // Before the button is pressed:
    // one linked file is on disk again (its item is stale until ranked);
    lib.back(back.old);
    // one entry is no longer trusted on the copy (a new track, by now);
    lib.title(untrusted.copy, "Renamed since");
    lib.decided(untrusted.copy);
    lib.relink(&mounted);
    // one item suggests another copy (the one shown is gone, a new one came);
    lib.gone(other_copy.copy);
    let newer = lib.file(music, "Copies/Omega (2).flac", Some(290_000));
    lib.hashed(newer, 99);
    lib.in_the_track_of(newer, other_copy.old);
    // and one copy is gone, with nothing in its place.
    lib.gone(copy_gone.copy);

    let accepted = lib.accept(&shown);
    assert_eq!(
        accepted
            .switched
            .iter()
            .map(|s| s.recording_id)
            .collect::<Vec<_>>(),
        [stays.recording]
    );
    let skipped = |recording_id, reason| SkippedItem {
        recording_id,
        reason,
    };
    assert_eq!(
        accepted.skipped,
        [
            skipped(back.recording, SkipReason::ItemGone),
            skipped(untrusted.recording, SkipReason::NewTrack),
            skipped(other_copy.recording, SkipReason::ItemGone),
            skipped(copy_gone.recording, SkipReason::ItemGone),
        ]
    );
    assert_eq!(lib.linked_file(stays.track), stays.copy);
    for item in [back, untrusted, other_copy, copy_gone] {
        assert_eq!(lib.linked_file(item.track), item.old);
    }
    // The copy that turned up is offered, and was not switched to unseen.
    assert_eq!(
        lib.offer(other_copy.recording).map(|offer| offer.file_id),
        Some(newer)
    );
}

#[test]
fn accept_all_with_nothing_clear_records_no_operation() {
    let (lib, _mounted, item) = one(Rekordbox::NoMatch);
    let kinds = lib.kinds();
    assert_eq!(lib.clear_cases(), []);
    // Even if the screen sent it: it isn't a clear case.
    let shown = [AcceptItem {
        recording_id: item.recording,
        file_id: item.copy,
    }];
    let accepted = lib.accept(&shown);
    assert_eq!(accepted.operation_id, None);
    assert_eq!(accepted.switched, []);
    assert_eq!(
        accepted.skipped,
        [SkippedItem {
            recording_id: item.recording,
            reason: SkipReason::NewTrack,
        }]
    );
    assert_eq!(lib.kinds(), kinds);
    assert_eq!(lib.linked_file(item.track), item.old);
    // Nothing shown, nothing done.
    assert_eq!(lib.accept(&[]).skipped, []);
}

// ---- a byte-identical copy's entry (1bE-11) ---------------------------------

#[test]
fn an_entry_a_switch_left_behind_is_sent_again_through_a_byte_identical_copy_and_never_also_listed()
{
    let (lib, music, mounted) = e_music();
    let item = item(&lib, music, &mounted, "Kappa", 0, Rekordbox::NoMatch);
    settle(&lib, &mounted);
    let row = item.row.unwrap();
    lib.bytes(item.copy, 7);
    lib.switch(item.recording, item.copy, SwitchEffect::NewTrack)
        .unwrap();
    // Before: listed to remove, and not sent.
    assert_eq!(lib.sent(item.track), SentAs::New);
    assert_eq!(
        lib.to_remove(),
        [(RemovalReason::Replaced, Some(shown_old("Kappa")))]
    );

    // A byte-identical copy of the linked file is in the track, and the
    // user confirms rekordbox's entry on it.
    let twin = lib.file(music, "Twin/Kappa.flac", Some(200_000));
    lib.hashed(twin, 2);
    lib.bytes(twin, 7);
    lib.in_the_track_of(twin, item.copy);
    lib.confirm_now(&loc(&old_path("Kappa")), twin, Method::Fingerprint);
    settle(&lib, &mounted);
    assert_eq!(lib.matched(row).map(|m| m.0), Some(twin));
    assert!(!lib.probable(row));
    assert_eq!(lib.linked_file(item.track), item.copy);

    // After: sent as that entry, rekordbox's own Location, and not listed.
    assert_eq!(lib.sent(item.track), SentAs::Entry(row));
    let (out, _read) = lib.send_in_a_crate(item.track);
    assert!(out.sent[0].in_rekordbox);
    assert_eq!(out.sent[0].location(), loc(&old_path("Kappa")));
    assert_eq!(lib.to_remove(), []);
    // The record stays: it is listed again if the entry goes from the copy.
    assert_eq!(lib.count("SELECT count(*) FROM replaced_entry"), 1);

    // Undo the confirmation: listed again, sent as new.
    assert!(
        matches!(lib.undo_last(), UndoOutcome::Undone { operation } if operation.kind != SWITCH_OPERATION)
    );
    settle(&lib, &mounted);
    assert_eq!(lib.sent(item.track), SentAs::New);
    assert_eq!(lib.to_remove().len(), 1);

    // Undo the switch: back on the gone file, sent as its entry at that
    // file's own Location, and the record is gone.
    assert!(
        matches!(lib.undo_last(), UndoOutcome::Undone { operation } if operation.kind == SWITCH_OPERATION)
    );
    assert_eq!(lib.linked_file(item.track), item.old);
    assert_eq!(lib.sent(item.track), SentAs::Entry(row));
    assert_eq!(lib.count("SELECT count(*) FROM replaced_entry"), 0);
    assert_eq!(lib.to_remove(), []);
}

// ---- ids never reused (1bE-ids) ------------------------------------------------

/// [`one`] with no rekordbox entry, but the copy made first: the gone file
/// is the newest, so before migration 0031 the next file scanned after its
/// row went took its id. With `twin`, another file with the copy's audio is
/// made before it too, in no track yet. Also returns the music folder and
/// the twin.
fn gone_file_newest(twin: bool) -> (Lib, i64, Mounted, Item, Option<i64>) {
    let (lib, music, mounted) = e_music();
    let twin = twin.then(|| {
        let twin = lib.file(music, "Twins/Kappa.flac", Some(200_000));
        lib.hashed(twin, 2);
        twin
    });
    let copy = lib.file(music, "Copies/Kappa.flac", Some(200_000));
    lib.hashed(copy, 2);
    let old = lib.file(music, "Kappa.mp3", Some(200_000));
    lib.hashed(old, 1);
    let recording = lib.group(&[old]);
    let track = lib.add(old);
    lib.gone(old);
    lib.title(copy, "Kappa");
    lib.joins(copy, old);
    settle(&lib, &mounted);
    assert_eq!(lib.clear_items(), [recording]);
    assert_eq!(lib.newest_file(), old);
    let item = Item {
        old,
        copy,
        recording,
        track,
        row: None,
    };
    (lib, music, mounted, item, twin)
}

/// The gone file's music folder is removed and another file is scanned;
/// the new file never has the gone file's id.
fn old_file_removed_and_another_scanned(lib: &Lib, music: i64, item: &Item) {
    assert_ne!(lib.linked_file(item.track), item.old, "nothing holds the old file");
    lib.forget(item.old);
    let other = lib.file(music, "Nightdrive.mp3", Some(100_000));
    assert!(other > item.old, "a new file takes a new id: {other}");
}

/// Undo is refused, and the track stays on `linked`.
fn undo_refused(lib: &Lib, item: &Item, linked: i64) {
    let outcome = lib.undo_last();
    assert!(
        matches!(outcome, UndoOutcome::Refused { .. }),
        "the old file is gone: {outcome:?}"
    );
    assert_eq!(lib.linked_file(item.track), linked);
}

#[test]
fn undo_of_a_switch_never_links_the_track_to_a_file_scanned_after_the_old_one_went() {
    let (lib, music, _mounted, item, _) = gone_file_newest(false);
    lib.switch(item.recording, item.copy, SwitchEffect::NotInRekordbox)
        .unwrap();
    old_file_removed_and_another_scanned(&lib, music, &item);
    undo_refused(&lib, &item, item.copy);
}

#[test]
fn undo_of_accept_all_never_links_a_track_to_a_file_scanned_after_the_old_one_went() {
    let (lib, music, _mounted, item, _) = gone_file_newest(false);
    let shown = lib.clear_cases();
    assert_eq!(shown.len(), 1);
    lib.accept(&shown);
    assert_eq!(lib.linked_file(item.track), item.copy);
    old_file_removed_and_another_scanned(&lib, music, &item);
    undo_refused(&lib, &item, item.copy);
}

#[test]
fn undo_of_a_switch_the_track_then_followed_from_never_links_it_to_a_file_scanned_later() {
    // After the switch the copy goes too, and the track follows it to a
    // file with its audio: an automatic operation, which the switch's Undo
    // takes back with it, putting the link back on the old file.
    let (lib, music, _mounted, item, twin) = gone_file_newest(true);
    let twin = twin.unwrap();
    lib.switch(item.recording, item.copy, SwitchEffect::NotInRekordbox)
        .unwrap();
    lib.joins(twin, item.copy);
    lib.gone(item.copy);
    lib.follow();
    assert_eq!(lib.linked_file(item.track), twin, "the track followed");
    old_file_removed_and_another_scanned(&lib, music, &item);
    undo_refused(&lib, &item, twin);
}
