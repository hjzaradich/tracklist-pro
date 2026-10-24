//! "These are the same track", "Not the same" and a split (1bE-2), and the
//! Undo of each. Built on the grouping's own test database, through the real
//! matching pass, deciding job and grouping pass, so the pairs stand where
//! the grouping puts them.

use super::super::actions::{Acted, Refused};
use super::super::split::{
    not_same, same_track, NotSame, NOT_SAME_OPERATION, SAME_TRACK_OPERATION, SPLIT_OPERATION,
};
use super::*;
use crate::grouping::tests::support::{db, hash, reencoded, track, Db, NewFile};
use crate::ops::{UndoOutcome, UndoRefusal};

/// About two minutes of made-up audio.
const LEN: usize = 900;

fn file<'a>(name: &'a str, n: u8, items: Vec<u32>) -> NewFile<'a> {
    NewFile {
        name,
        title: Some("Velo"),
        artist: Some("Halden Rook"),
        hash: Some(hash(n)),
        items: Some(items),
        blake3: None,
    }
}

/// Another take: the first half of `velo`, then other audio. The rules send
/// it to review against `velo` and its re-encodes.
fn other_take(velo: &[u32], seed: u64) -> Vec<u32> {
    [&velo[..LEN / 2], &track(seed, LEN / 2)[..]].concat()
}

struct Item {
    db: Db,
}

impl Item {
    fn kind_items(&self, kind: ReviewKind) -> Vec<ReviewItemId> {
        let page = self
            .db
            .writer
            .call(|c| items(c, &Mount, ReviewGroup::NeedsCloserLook, 0, 100))
            .unwrap();
        page.items
            .into_iter()
            .map(|i| i.id)
            .filter(|i| i.kind == kind)
            .collect()
    }

    /// The one version doubt in the queue.
    fn doubt(&self) -> ReviewItemId {
        let found = self.kind_items(ReviewKind::VersionDoubt);
        assert_eq!(found.len(), 1, "one version doubt: {found:?}");
        found[0]
    }

    fn queued(&self, id: ReviewItemId) -> bool {
        self.kind_items(id.kind).contains(&id)
    }

    fn shown(&self, id: ReviewItemId) -> Vec<i64> {
        let compared = self
            .db
            .writer
            .call(move |c| comparison(c, &Mount, id))
            .unwrap()
            .expect("the item is queued");
        compared
            .tracks
            .iter()
            .filter_map(|t| t.recording_id)
            .collect()
    }

    fn same(&self, id: ReviewItemId) -> Result<Acted, Refused> {
        let shown = self.shown(id);
        self.db
            .writer
            .call(move |c| Ok(same_track(c, id, &shown)))
            .unwrap()
    }

    fn not_same(&self, id: ReviewItemId, file: Option<i64>) -> Result<NotSame, Refused> {
        let shown = self.shown(id);
        self.db
            .writer
            .call(move |c| Ok(not_same(c, id, &shown, file)))
            .unwrap()
    }

    fn undo(&self) -> UndoOutcome {
        self.db
            .writer
            .call(|c| Ok(crate::ops::undo_for_user(c, None)))
            .unwrap()
            .unwrap()
    }

    fn kind_of_last_operation(&self) -> String {
        self.db
            .writer
            .call(|c| {
                c.query_row(
                    "SELECT kind FROM operation ORDER BY id DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap()
    }

    fn current(&self, file: i64) -> Vec<i64> {
        self.db
            .writer
            .call(move |c| crate::grouping::current_duplicates(c, file))
            .unwrap()
    }

    /// What an action writes and its Undo puts back, as text: every file's
    /// track, every decision, every kept track. A file's role is the
    /// ranking's (derived), so it isn't here.
    fn rows(&self) -> String {
        const TABLES: [&str; 4] = [
            "SELECT 'file ' || file_id || ' ' || recording_id FROM recording_file ORDER BY file_id",
            "SELECT 'decision ' || id || ' ' || hex(key_a) || ' ' || hex(key_b) || ' ' || decision
             FROM grouping_decision ORDER BY id",
            "SELECT 'kept ' || id || ' ' || recording_id || ' ' || into_id || ' ' || kind
             FROM track_merge ORDER BY id",
            "SELECT 'link ' || id || ' ' || recording_a || ' ' || recording_b || ' ' || source
             FROM version_link ORDER BY id",
        ];
        self.db
            .writer
            .call(|c| {
                let mut out = Vec::new();
                for sql in TABLES {
                    let mut s = c.prepare(sql)?;
                    for row in s.query_map([], |r| r.get::<_, String>(0))? {
                        out.push(row?);
                    }
                }
                Ok(out.join("\n"))
            })
            .unwrap()
    }
}

/// A Library track holding a FLAC (`a`, the linked file) and its MP3
/// re-encode (`b`), and outside the Library another take (`c`): a possible
/// version mix-up with three files.
fn three_files() -> (Item, i64, i64, i64) {
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", 2, reencoded(&velo, 1, 7)));
    db.decide();
    assert_eq!(db.stands(a, b), ("duplicate".into(), "merged".into()));
    db.to_library(a);
    let c = db.add(file("Velo (other take).flac", 3, other_take(&velo, 9)));
    db.decide();
    assert_eq!(db.stands(a, c).1, "open");
    (Item { db }, a, b, c)
}

/// A Library track (`a`) and another take outside the Library (`c`): a
/// possible version mix-up with two files.
fn two_files() -> (Item, i64, i64) {
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    let c = db.add(file("Velo (other take).flac", 3, other_take(&velo, 9)));
    db.decide();
    db.to_library(a);
    db.decide();
    assert_eq!(db.stands(a, c), ("review".into(), "open".into()));
    (Item { db }, a, c)
}

// ---- these are the same track -------------------------------------------------

#[test]
fn the_same_track_moves_the_other_take_into_the_library_track_and_undo_puts_it_back() {
    let (item, a, c) = two_files();
    let id = item.doubt();
    let (ta, tc) = (item.db.track(a).unwrap(), item.db.track(c).unwrap());
    let before = item.rows();
    let acted = item.same(id).unwrap();
    assert!(acted.operation_id.is_some());
    assert_eq!(item.kind_of_last_operation(), SAME_TRACK_OPERATION);
    assert_eq!(item.db.track(c), Some(ta), "one track, the Library track");
    assert_eq!(
        item.db.stands(a, c),
        ("review".into(), "in_one_track".into())
    );
    assert!(!item.queued(id));
    // The track that lost its file is kept, with none, for the Undo.
    assert_eq!(
        item.db
            .count("SELECT count(*) FROM track_merge WHERE kind = 'same'"),
        1
    );
    assert_eq!(
        item.db
            .count(&format!("SELECT count(*) FROM recording WHERE id = {tc}")),
        1
    );
    // A grouping job in between changes nothing: the decision sticks.
    item.db.decide();
    assert_eq!(item.db.track(c), Some(ta));

    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(
        item.rows(),
        before,
        "the decision and the move come back exactly"
    );
    assert_eq!(item.db.track(c), Some(tc));
    assert_eq!(item.db.stands(a, c), ("review".into(), "open".into()));
    assert!(item.queued(id), "the item is open again");
    item.db.decide();
    assert!(item.queued(id), "and stays open");
}

#[test]
fn the_same_track_drops_only_the_jobs_own_version_link_between_the_tracks() {
    let (item, a, c) = two_files();
    let id = item.doubt();
    let (ta, tc) = (item.db.track(a).unwrap(), item.db.track(c).unwrap());
    // A link of the job's own between the two (the grouping makes and drops
    // those), and one of the user's between the Library track and another.
    let other = item.db.file(1, "Nightdrive.mp3", Some(hash(40)));
    item.db.group();
    let to = item.db.track(other).unwrap();
    item.db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({ta}, {tc}, 'cut', 'parser'), ({ta}, {to}, 'rework', 'user')"
    ));
    item.same(id).unwrap();
    assert_eq!(item.db.track(c), Some(ta));
    assert_eq!(
        item.db
            .count("SELECT count(*) FROM version_link WHERE source = 'parser'"),
        0
    );
    assert_eq!(
        item.db.count(&format!(
            "SELECT count(*) FROM version_link WHERE source = 'user' AND recording_b = {to}"
        )),
        1,
        "a link the user made is never touched"
    );
}

#[test]
fn the_same_track_is_refused_for_tracks_the_user_linked_as_versions() {
    let (item, a, c) = two_files();
    let id = item.doubt();
    let (ta, tc) = (item.db.track(a).unwrap(), item.db.track(c).unwrap());
    let shown = item.shown(id);
    // Made after the item was read: the pair still waits.
    item.db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({ta}, {tc}, 'cut', 'user')"
    ));
    let before = item.rows();
    let refused = item
        .db
        .writer
        .call(move |c| Ok(same_track(c, id, &shown)))
        .unwrap();
    assert!(matches!(refused, Err(Refused::LinkedVersions)));
    assert_eq!(item.rows(), before, "nothing changed");
    assert_eq!(item.db.count("SELECT count(*) FROM operation"), 0);
}

#[test]
fn the_same_track_never_merges_two_library_tracks_they_become_the_same_track_twice() {
    let (item, a, c) = two_files();
    item.db.to_library(c);
    item.db.decide();
    let id = item.doubt();
    let (ta, tc) = (item.db.track(a).unwrap(), item.db.track(c).unwrap());
    item.same(id).unwrap();
    assert_eq!(item.db.track(a), Some(ta));
    assert_eq!(item.db.track(c), Some(tc));
    assert_eq!(item.db.count("SELECT count(*) FROM library_track"), 2);
    assert!(!item.queued(id));
    assert_eq!(
        item.kind_items(ReviewKind::SameRecording),
        [super::id(ReviewKind::SameRecording, ta.min(tc), None)],
        "the pair is \"Same track twice in your Library\" now, with its own Merge"
    );
    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    assert!(item.queued(id));
    assert!(item.kind_items(ReviewKind::SameRecording).is_empty());
}

#[test]
fn the_undo_of_the_same_track_is_refused_once_a_moved_file_is_a_library_tracks_file() {
    let (item, a, c) = two_files();
    let id = item.doubt();
    item.same(id).unwrap();
    // The Library track followed its file to the one that moved in.
    let ta = item.db.track(a).unwrap();
    item.db.sql(&format!(
        "UPDATE library_track SET linked_file_id = {c} WHERE recording_id = {ta}"
    ));
    let before = item.rows();
    match item.undo() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("{other:?}"),
    }
    assert_eq!(item.rows(), before, "nothing changed");
}

// ---- not the same: two files ------------------------------------------------------

#[test]
fn s_on_two_files_keeps_them_apart_and_undo_opens_the_item_again() {
    let (item, a, c) = two_files();
    let id = item.doubt();
    let before = item.rows();
    assert!(!item.not_same(id, None).unwrap().item_stays);
    assert_eq!(item.kind_of_last_operation(), NOT_SAME_OPERATION);
    assert_ne!(item.db.track(a), item.db.track(c));
    assert_eq!(item.db.stands(a, c), ("review".into(), "decided".into()));
    assert!(!item.queued(id));
    item.db.decide();
    assert!(!item.queued(id), "never asked again");
    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(item.rows(), before);
    assert!(item.queued(id));
}

// ---- a split: three files or more -----------------------------------------------

#[test]
fn s_on_three_files_splits_off_the_selected_file_for_good_and_undo_puts_it_back() {
    let (item, a, b, c) = three_files();
    let id = item.doubt();
    let (ta, tc) = (item.db.track(a).unwrap(), item.db.track(c).unwrap());
    let tracks = item.db.tracks();
    let before = item.rows();
    assert_eq!(item.current(a), [b], "relink's same-track match, before");

    let done = item.not_same(id, Some(b)).unwrap();
    assert!(done.item_stays, "the screen stays on the item");
    assert_eq!(item.kind_of_last_operation(), SPLIT_OPERATION);
    let tb = item.db.track(b).unwrap();
    assert!(tb != ta && tb != tc, "a track of its own");
    assert_eq!(item.db.tracks(), tracks + 1);
    assert_eq!(item.db.stands(a, b), ("duplicate".into(), "decided".into()));
    assert_eq!(
        item.current(a),
        Vec::<i64>::new(),
        "no same-track match any more"
    );
    // The mix-up between the Library track and the other take still waits.
    assert!(item.queued(id));
    // For good: the grouping job never merges it back.
    item.db.decide();
    item.db.decide();
    assert_eq!(item.db.track(b), Some(tb));

    let undone = item.undo();
    assert!(matches!(undone, UndoOutcome::Undone { .. }), "{undone:?}");
    assert_eq!(
        item.rows(),
        before,
        "the decision and the move come back exactly"
    );
    assert_eq!(item.db.track(b), Some(ta));
    assert_eq!(item.db.tracks(), tracks, "the track it made is gone");
    assert_eq!(item.current(a), [b]);
    item.db.decide();
    assert_eq!(item.db.track(b), Some(ta));
    assert_eq!(item.db.stands(a, b), ("duplicate".into(), "merged".into()));
}

#[test]
fn the_linked_file_is_never_split_off_the_other_files_leave_it_together() {
    let (item, a, b, c) = three_files();
    let id = item.doubt();
    let (ta, tc) = (item.db.track(a).unwrap(), item.db.track(c).unwrap());
    let before = item.rows();
    item.not_same(id, Some(a)).unwrap();
    assert_eq!(
        item.db.track(a),
        Some(ta),
        "the linked file stays in its track"
    );
    let tb = item.db.track(b).unwrap();
    assert!(tb != ta && tb != tc);
    // Apart from the linked file's track: the mix-up is answered too.
    assert_eq!(item.db.stands(a, b).1, "decided");
    assert_eq!(item.db.stands(a, c).1, "decided");
    assert!(!item.queued(id));
    item.db.decide();
    assert_eq!(item.db.track(b), Some(tb));
    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(item.rows(), before);
    assert!(item.queued(id));
}

#[test]
fn s_on_three_files_needs_a_file_of_the_item() {
    let (item, _a, _b, _c) = three_files();
    let id = item.doubt();
    let other = item.db.file(1, "Nightdrive.mp3", Some(hash(40)));
    let before = item.rows();
    assert!(matches!(item.not_same(id, None), Err(Refused::ItemGone)));
    assert!(matches!(
        item.not_same(id, Some(other)),
        Err(Refused::ItemGone)
    ));
    assert_eq!(item.rows(), before);
}

#[test]
fn a_copy_of_the_linked_file_takes_it_along_and_its_track_is_kept_apart() {
    // Files with the very same audio are always one track: the copy of the
    // linked file goes with it (owner decision, 2026-10-24). The linked file
    // never leaves its track, so nothing moves: the track is kept apart from
    // the other take, and the item is answered.
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    let b = db.add(file("copies/Velo.flac", 1, velo.clone()));
    db.decide();
    assert_eq!(db.track(a), db.track(b));
    let ta = db.to_library(a);
    let c = db.add(file("Velo (other take).flac", 3, other_take(&velo, 9)));
    db.decide();
    let item = Item { db };
    let id = item.doubt();
    let tc = item.db.track(c).unwrap();
    let before = item.rows();
    assert!(!item.not_same(id, Some(b)).unwrap().item_stays);
    assert_eq!(item.kind_of_last_operation(), SPLIT_OPERATION);
    assert_eq!((item.db.track(a), item.db.track(b)), (Some(ta), Some(ta)));
    assert_eq!(item.db.track(c), Some(tc));
    assert_eq!(item.db.stands(a, c).1, "decided");
    assert!(!item.queued(id));
    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(item.rows(), before);
    assert!(item.queued(id));
}

#[test]
fn the_undo_of_a_split_is_refused_once_the_track_it_left_was_swept_and_its_id_made_again() {
    // The track the file left is outside the Library: once its other file is
    // gone, the grouping sweeps it, and a new track can take its id. Undo
    // must then refuse, never put the file into that other track.
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    db.decide();
    db.to_library(a);
    let take = other_take(&velo, 9);
    let c = db.add(file("Velo (other take).flac", 3, take.clone()));
    let d = db.add(file("Velo (other take).mp3", 4, reencoded(&take, 2, 7)));
    db.decide();
    assert_eq!(db.track(c), db.track(d));
    let item = Item { db };
    let id = item.doubt();
    let tc = item.db.track(c).unwrap();
    item.not_same(id, Some(c)).unwrap();
    let split_to = item.db.track(c).unwrap();
    assert_ne!(split_to, tc);

    // `d`, the file that stayed, goes (its music folder was removed), and
    // the grouping sweeps the track it leaves empty.
    item.db.sql(&format!(
        "DELETE FROM recording_file WHERE file_id = {d}; DELETE FROM file WHERE id = {d};"
    ));
    item.db.group();
    assert_eq!(
        item.db
            .count(&format!("SELECT count(*) FROM recording WHERE id = {tc}")),
        0
    );
    // A new track takes the old id, with a file of its own.
    let e = item.db.file(1, "Nightdrive.mp3", Some(hash(40)));
    item.db.group();
    item.db.sql(&format!(
        "INSERT INTO recording (id) VALUES ({tc});
         UPDATE recording_file SET recording_id = {tc} WHERE file_id = {e};"
    ));
    item.db.group();
    let before = item.rows();
    match item.undo() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("{other:?}"),
    }
    assert_eq!(item.rows(), before, "nothing changed");
    assert_eq!(item.db.track(c), Some(split_to));
    assert_eq!(item.db.track(e), Some(tc));
    // And `next_undo` says so ahead of time.
    let next = item
        .db
        .writer
        .call(|c| Ok(crate::ops::next_undo(c)))
        .unwrap()
        .unwrap();
    assert_eq!(next.refusal, Some(UndoRefusal::ChangedSince));
}

#[test]
fn the_new_track_is_rolled_back_with_an_operation_that_recorded_nothing() {
    // `record_with_derived_first`: what `first` made goes when the write
    // records nothing, and stays with an operation.
    let (item, _a, _c) = two_files();
    let tracks = item.db.tracks();
    for record in [false, true] {
        item.db
            .writer
            .call(move |c| {
                Ok(crate::ops::record_with_derived_first(
                    c,
                    "test_first",
                    &serde_json::json!({}),
                    |c| {
                        c.execute("INSERT INTO recording DEFAULT VALUES", [])?;
                        Ok(c.last_insert_rowid())
                    },
                    |rec, made| {
                        if record {
                            rec.insert(
                                "track_merge",
                                &[
                                    ("recording_id", rusqlite::types::Value::Integer(made)),
                                    ("into_id", rusqlite::types::Value::Integer(1)),
                                ],
                            )?;
                        }
                        Ok(())
                    },
                    |_| Ok(()),
                ))
            })
            .unwrap()
            .unwrap();
    }
    assert_eq!(item.db.tracks(), tracks + 1, "only the recorded one stays");
}

// ---- fix round 1 (review of PR 98) --------------------------------------------

#[test]
fn the_undo_of_a_split_is_refused_once_the_moved_file_is_gone_and_no_new_file_or_track_takes_its_ids(
) {
    // The reviewer's case: the file split off is the newest file and its new
    // track the newest track. Before migration 0031 both ids came back for
    // the next file scanned, and only the operation's own record of what the
    // moved file held told them apart. Now neither id is ever made again.
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    db.decide();
    let ta = db.to_library(a);
    let c = db.add(file("Velo (other take).flac", 3, other_take(&velo, 9)));
    db.decide();
    let b = db.add(file("Velo.mp3", 2, reencoded(&velo, 1, 7)));
    db.decide();
    assert_eq!(db.track(b), Some(ta));
    let _ = c;
    let item = Item { db };
    let id = item.doubt();
    item.not_same(id, Some(b)).unwrap();
    let split_to = item.db.track(b).unwrap();

    // `b`'s music folder is removed (its rows go), and the grouping sweeps
    // the track it leaves empty.
    item.db.sql(&format!(
        "DELETE FROM recording_file WHERE file_id = {b}; DELETE FROM file WHERE id = {b};"
    ));
    item.db.group();
    // An unrelated file is scanned, and the grouping makes it a track.
    let e = item.db.file(1, "Nightdrive.mp3", Some(hash(40)));
    item.db.group();
    assert!(e > b, "a new file takes a new id: {e}");
    let te = item.db.track(e).unwrap();
    assert!(te > split_to, "and its track a new track id: {te}");

    let before = item.rows();
    match item.undo() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("{other:?}"),
    }
    assert_eq!(item.rows(), before, "nothing changed");
    assert_eq!(
        item.db.track(e),
        Some(te),
        "never moved into the Library track"
    );
}

#[test]
fn the_undo_of_the_same_track_is_refused_once_a_moved_file_is_gone_and_no_new_file_takes_its_id() {
    // The file the decision moved into the Library track is the newest. Its
    // music folder is removed, and a file with the Library track's audio is
    // scanned: it joins the Library track, and before migration 0031 it took
    // the moved file's id, so the Undo would have moved it out.
    let (item, a, c) = two_files();
    let id = item.doubt();
    let ta = item.db.track(a).unwrap();
    item.same(id).unwrap();
    assert_eq!(item.db.track(c), Some(ta));
    assert_eq!(
        item.db.count("SELECT max(id) FROM file"),
        c,
        "c is the newest file"
    );
    item.db.sql(&format!(
        "DELETE FROM recording_file WHERE file_id = {c}; DELETE FROM file WHERE id = {c};"
    ));
    item.db.group();
    let e = item.db.file(1, "Nightdrive.flac", Some(hash(1)));
    item.db.group();
    assert!(e > c, "a new file takes a new id: {e}");
    assert_eq!(item.db.track(e), Some(ta), "it joins the Library track");

    let before = item.rows();
    assert!(
        matches!(item.undo(), UndoOutcome::Refused { .. }),
        "the moved file is gone: the undo is refused"
    );
    assert_eq!(item.rows(), before, "nothing changed");
    assert_eq!(item.db.track(e), Some(ta), "the new file stays where it is");
}

#[test]
fn the_undo_of_the_same_track_is_refused_once_a_moved_file_holds_other_audio() {
    let (item, _a, c) = two_files();
    let id = item.doubt();
    item.same(id).unwrap();
    // The file is overwritten with other audio (its id is the same file's no
    // more, as far as a decision goes).
    item.db.sql(&format!(
        "UPDATE file SET audio_hash = x'{}' WHERE id = {c}",
        "41".repeat(34)
    ));
    let before = item.rows();
    match item.undo() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("{other:?}"),
    }
    assert_eq!(item.rows(), before);
}

#[test]
fn the_linked_files_track_loses_every_other_file_into_one_new_track() {
    // The linked file's track holds two other files: both leave, together.
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", 2, reencoded(&velo, 1, 7)));
    let b2 = db.add(file("Velo.ogg", 5, reencoded(&velo, 3, 7)));
    db.decide();
    let ta = db.to_library(a);
    assert_eq!((db.track(b), db.track(b2)), (Some(ta), Some(ta)));
    let c = db.add(file("Velo (other take).flac", 3, other_take(&velo, 9)));
    db.decide();
    let item = Item { db };
    let id = item.doubt();
    let tc = item.db.track(c).unwrap();
    let before = item.rows();
    item.not_same(id, Some(a)).unwrap();
    assert_eq!(item.db.track(a), Some(ta));
    let left = item.db.track(b).unwrap();
    assert!(left != ta && left != tc);
    assert_eq!(item.db.track(b2), Some(left), "the others leave together");
    assert_eq!(item.db.stands(b, b2).1, "merged");
    assert_eq!(item.db.stands(a, b).1, "decided");
    assert_eq!(item.db.stands(a, b2).1, "decided");
    item.db.decide();
    assert_eq!(
        (item.db.track(b), item.db.track(b2)),
        (Some(left), Some(left))
    );
    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(item.rows(), before);
}

#[test]
fn the_undo_of_a_split_is_refused_when_the_track_it_left_was_made_at_another_time() {
    // The time half of the check alone: the files that stayed are still
    // there, holding what they held, but the track by that id is not the one
    // the split took the file out of.
    let (item, a, b, _c) = three_files();
    let id = item.doubt();
    let ta = item.db.track(a).unwrap();
    let made: String = item
        .db
        .writer
        .call(move |c| {
            c.query_row(
                "SELECT created_at FROM recording WHERE id = ?1",
                [ta],
                |r| r.get(0),
            )
        })
        .unwrap();
    item.not_same(id, Some(b)).unwrap();
    item.db.sql(&format!(
        "UPDATE recording SET created_at = '2000-01-01T00:00:00.000Z' WHERE id = {ta}"
    ));
    match item.undo() {
        UndoOutcome::Refused { reason, .. } => assert_eq!(reason, UndoRefusal::ChangedSince),
        other => panic!("{other:?}"),
    }
    item.db.sql(&format!(
        "UPDATE recording SET created_at = '{made}' WHERE id = {ta}"
    ));
    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(item.db.track(b), Some(ta));
}

#[test]
fn s_on_two_files_in_one_track_splits_one_off_and_relink_no_longer_matches_them() {
    // Two files that share a track (a kind with one track, S's next kinds):
    // the one that leaves becomes a track of its own (foreman's ruling,
    // 2026-10-16), and relink's same-track match between them stops.
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    let b = db.add(file("Velo.mp3", 2, reencoded(&velo, 1, 7)));
    db.decide();
    let ta = db.to_library(a);
    let item = Item { db };
    assert_eq!(item.current(a), [b]);
    let s_on = |file: Option<i64>| {
        item.db
            .writer
            .call(move |c| Ok(super::super::split::s_on(c, &[ta], file)))
            .unwrap()
    };
    // Which one leaves is the screen's to send.
    assert!(matches!(s_on(None), Err(Refused::ItemGone)));
    // The linked file never leaves: sent, the other one leaves instead.
    s_on(Some(a)).unwrap();
    assert_eq!(item.db.track(a), Some(ta));
    let tb = item.db.track(b).unwrap();
    assert_ne!(tb, ta);
    assert_eq!(item.current(a), Vec::<i64>::new());
    assert_eq!(item.current(b), Vec::<i64>::new());
    item.db.decide();
    assert_eq!(item.db.track(b), Some(tb), "for good");
    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(item.db.track(b), Some(ta));
    assert_eq!(item.current(a), [b]);
    // The other file sent: it leaves.
    s_on(Some(b)).unwrap();
    assert_ne!(item.db.track(b), Some(ta));
}

// ---- the copies go along (owner decisions, 2026-10-24; DOGFOOD B-5) -----------------

/// `name` holding the audio `n`, with the fingerprint `items`: the helper
/// `file` with another name.
fn copy<'a>(name: &'a str, n: u8, items: &[u32]) -> NewFile<'a> {
    file(name, n, items.to_vec())
}

/// A Library track holding a FLAC (`a`, the linked file) and `copies`
/// identical MP3 re-encodes of it (one audio, `hash(2)`), and outside the
/// Library another take (`c`): a possible version mix-up. Returns the item,
/// `a`, the MP3s and `c`.
fn linked_with_copies(copies: usize) -> (Item, i64, Vec<i64>, i64) {
    const NAMES: [&str; 3] = ["Velo.mp3", "copies/Velo.mp3", "more/Velo.mp3"];
    let db = db();
    let velo = track(1, LEN);
    let mp3 = reencoded(&velo, 1, 7);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    let mp3s: Vec<i64> = NAMES[..copies]
        .iter()
        .map(|name| db.add(copy(name, 2, &mp3)))
        .collect();
    db.decide();
    let ta = db.to_library(a);
    for &m in &mp3s {
        assert_eq!(db.track(m), Some(ta));
    }
    let c = db.add(file("Velo (other take).flac", 3, other_take(&velo, 9)));
    db.decide();
    assert_eq!(db.stands(a, c).1, "open");
    (Item { db }, a, mp3s, c)
}

#[test]
fn two_copies_and_two_copies_end_as_two_tracks_kept_apart_and_undo_takes_it_back() {
    // The typical item: two identical copies of a version (in the Library)
    // and two of another take. Split off on any one of the four: the two of
    // each stay together, as their own tracks, apart, and the item is
    // answered.
    for pick in 0..4 {
        let db = db();
        let velo = track(1, LEN);
        let take = other_take(&velo, 9);
        let a1 = db.add(file("Velo.flac", 1, velo.clone()));
        let a2 = db.add(file("copies/Velo.flac", 1, velo.clone()));
        let b1 = db.add(file("Velo (other take).flac", 3, take.clone()));
        let b2 = db.add(file("copies/Velo (other take).flac", 3, take.clone()));
        db.decide();
        let ta = db.to_library(a1);
        db.decide();
        let tb = db.track(b1).unwrap();
        assert_eq!((db.track(a2), db.track(b2)), (Some(ta), Some(tb)));
        let item = Item { db };
        let id = item.doubt();
        let before = item.rows();
        let tracks = item.db.tracks();

        let picked = [a1, a2, b1, b2][pick];
        let done = item.not_same(id, Some(picked)).unwrap();
        assert!(!done.item_stays, "the item is answered");
        assert_eq!(item.kind_of_last_operation(), SPLIT_OPERATION);
        assert_eq!(
            [a1, a2, b1, b2].map(|f| item.db.track(f)),
            [Some(ta), Some(ta), Some(tb), Some(tb)],
            "each two together, as they were"
        );
        assert_eq!(item.db.tracks(), tracks, "no track made");
        assert_eq!(item.db.stands(a1, b1), ("review".into(), "decided".into()));
        assert!(!item.queued(id));
        // For good: the grouping job never asks again.
        item.db.decide();
        assert!(!item.queued(id));
        assert_eq!(item.db.track(b2), Some(tb));

        assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
        assert_eq!(item.rows(), before, "the decision comes back exactly");
        assert!(item.queued(id), "the item is open again");
    }
}

#[test]
fn copies_that_share_a_track_with_other_files_leave_it_together_and_undo_puts_them_back() {
    // Two identical MP3s in the Library track beside its FLAC: Split off on
    // one takes the other along, into one new track.
    let (item, a, mp3s, c) = linked_with_copies(2);
    let (m1, m2) = (mp3s[0], mp3s[1]);
    let id = item.doubt();
    let (ta, tc) = (item.db.track(a).unwrap(), item.db.track(c).unwrap());
    let tracks = item.db.tracks();
    let before = item.rows();

    let done = item.not_same(id, Some(m2)).unwrap();
    assert!(
        done.item_stays,
        "the mix-up with the other take is still open"
    );
    let left = item.db.track(m1).unwrap();
    assert!(left != ta && left != tc, "a track of their own");
    assert_eq!(item.db.track(m2), Some(left), "together");
    assert_eq!(item.db.track(a), Some(ta));
    assert_eq!(item.db.track(c), Some(tc));
    assert_eq!(item.db.tracks(), tracks + 1);
    assert_eq!(item.db.stands(m1, m2).1, "merged");
    assert_eq!(item.db.stands(a, m1).1, "decided");
    assert_eq!(item.db.stands(a, m2).1, "decided");
    assert_eq!(
        item.current(a),
        Vec::<i64>::new(),
        "relink no longer matches them"
    );
    assert!(item.queued(id));
    item.db.decide();
    item.db.decide();
    assert_eq!(
        (item.db.track(m1), item.db.track(m2)),
        (Some(left), Some(left))
    );

    let undone = item.undo();
    assert!(matches!(undone, UndoOutcome::Undone { .. }), "{undone:?}");
    assert_eq!(
        item.rows(),
        before,
        "the moves and the decisions come back exactly, in one step"
    );
    assert_eq!(item.db.tracks(), tracks, "the track it made is gone");
    item.db.decide();
    assert_eq!((item.db.track(m1), item.db.track(m2)), (Some(ta), Some(ta)));
    assert_eq!(item.db.stands(a, m1).1, "merged");
}

#[test]
fn three_copies_and_one_other_file() {
    // Three identical MP3s beside the linked FLAC: all three leave together.
    let (item, a, mp3s, _c) = linked_with_copies(3);
    let id = item.doubt();
    let ta = item.db.track(a).unwrap();
    let before = item.rows();
    item.not_same(id, Some(mp3s[1])).unwrap();
    let left = item.db.track(mp3s[0]).unwrap();
    assert_ne!(left, ta);
    assert_eq!(
        [0, 1, 2].map(|n| item.db.track(mp3s[n])),
        [Some(left); 3],
        "the three together"
    );
    assert_eq!(item.db.track(a), Some(ta));
    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(item.rows(), before);

    // Three identical copies in a track of their own, and one other take:
    // Split off on any copy, or on the take, keeps the three together and
    // answers the item; nothing moves.
    for pick in 0..4 {
        let db = db();
        let velo = track(1, LEN);
        let a = db.add(file("Velo.flac", 1, velo.clone()));
        let b = db.add(file("copies/Velo.flac", 1, velo.clone()));
        let c = db.add(file("more/Velo.flac", 1, velo.clone()));
        db.decide();
        let ta = db.to_library(a);
        let d = db.add(file("Velo (other take).flac", 3, other_take(&velo, 9)));
        db.decide();
        let td = db.track(d).unwrap();
        let item = Item { db };
        let id = item.doubt();
        let before = item.rows();
        assert!(
            !item
                .not_same(id, Some([a, b, c, d][pick]))
                .unwrap()
                .item_stays
        );
        assert_eq!(
            [a, b, c, d].map(|f| item.db.track(f)),
            [Some(ta), Some(ta), Some(ta), Some(td)]
        );
        assert_eq!(item.db.stands(a, d).1, "decided");
        assert!(!item.queued(id));
        assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
        assert_eq!(item.rows(), before);
        assert!(item.queued(id));
    }
}

#[test]
fn the_copies_are_found_by_audio_never_by_name_or_size() {
    // Every file here has the same size. Beside the linked FLAC: an MP3
    // (`m`), its identical copy under another name (`renamed`), and another
    // re-encode under the very same file name as `m` (`namesake`, other
    // audio). Split off `m`: the renamed copy goes along, the namesake stays.
    let db = db();
    let velo = track(1, LEN);
    let mp3 = reencoded(&velo, 1, 7);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    let m = db.add(copy("Velo.mp3", 2, &mp3));
    let renamed = db.add(copy("elsewhere/Something else.mp3", 2, &mp3));
    let namesake = db.add(copy("other/Velo.mp3", 4, &reencoded(&velo, 5, 7)));
    db.decide();
    let ta = db.to_library(a);
    let _c = db.add(file("Velo (other take).flac", 3, other_take(&velo, 9)));
    db.decide();
    for f in [m, renamed, namesake] {
        assert_eq!(db.track(f), Some(ta));
    }
    assert_eq!(
        db.count("SELECT count(DISTINCT size) FROM file"),
        1,
        "the same size, every one"
    );
    let item = Item { db };
    let id = item.doubt();
    item.not_same(id, Some(m)).unwrap();
    let left = item.db.track(m).unwrap();
    assert_ne!(left, ta);
    assert_eq!(
        item.db.track(renamed),
        Some(left),
        "the same audio goes along"
    );
    assert_eq!(item.db.track(namesake), Some(ta), "the same name stays");
    assert_eq!(item.db.track(a), Some(ta));
}

#[test]
fn a_split_is_refused_when_the_copies_are_every_file() {
    // Nothing would be left behind: refused, with its reason, and nothing
    // changes (two files of one track, S's next kinds).
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    let b = db.add(file("copies/Velo.flac", 1, velo.clone()));
    db.decide();
    let ta = db.to_library(a);
    assert_eq!(db.track(b), Some(ta));
    let item = Item { db };
    let before = item.rows();
    for picked in [a, b] {
        let refused = item
            .db
            .writer
            .call(move |c| Ok(super::super::split::s_on(c, &[ta], Some(picked))))
            .unwrap();
        assert!(matches!(refused, Err(Refused::SameAudio)), "{refused:?}");
    }
    assert_eq!(item.rows(), before);
    assert_eq!(item.db.count("SELECT count(*) FROM operation"), 0);
}

#[test]
fn copies_in_another_track_join_the_one_that_holds_nothing_but_copies() {
    // The grouping hasn't put two copies together yet (one sits in a track
    // of its own): Split off on the other gathers them in that track, and
    // Undo moves it back exactly.
    let (item, a, mp3s, c) = linked_with_copies(2);
    let (m1, m2) = (mp3s[0], mp3s[1]);
    let (ta, tc) = (item.db.track(a).unwrap(), item.db.track(c).unwrap());
    item.db.sql(&format!(
        "INSERT INTO recording DEFAULT VALUES;
         UPDATE recording_file SET recording_id = last_insert_rowid() WHERE file_id = {m2};"
    ));
    let tm = item.db.track(m2).unwrap();
    assert!(tm != ta && tm != tc);
    let before = item.rows();
    let s_on = |tracks: Vec<i64>, file: i64| {
        item.db
            .writer
            .call(move |c| Ok(super::super::split::s_on(c, &tracks, Some(file))))
            .unwrap()
    };
    s_on(vec![ta, tm, tc], m1).unwrap();
    assert_eq!((item.db.track(m1), item.db.track(m2)), (Some(tm), Some(tm)));
    assert_eq!(item.db.track(a), Some(ta));
    assert_eq!(item.db.stands(a, m1).1, "decided");
    let decisions = "SELECT count(*) FROM grouping_decision";
    assert!(item.db.count(decisions) > 0);
    assert!(matches!(item.undo(), UndoOutcome::Undone { .. }));
    // The file it moved is back, and the decisions are gone; the grouping
    // pass that follows the Undo then puts the two copies in one track, as
    // it does with every two files of one audio.
    assert_eq!(item.db.track(m1), Some(ta));
    assert_eq!(item.db.count(decisions), 0);
    assert!(before.contains(&format!("file {m1} {ta}")));
    assert_eq!(item.db.track(m2), Some(ta));
}

#[test]
fn the_linked_files_of_two_library_tracks_are_never_made_one_by_a_split() {
    // Two Library tracks whose linked files hold the same audio, and another
    // file beside one of them: the copies can't be one track. Refused.
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    let m = db.add(copy("Velo.mp3", 2, &reencoded(&velo, 1, 7)));
    let b = db.add(file("Velo (other take).flac", 3, other_take(&velo, 9)));
    db.decide();
    let ta = db.to_library(a);
    let tb = db.to_library(b);
    assert_eq!(db.track(m), Some(ta));
    db.set_hash(b, Some(hash(1)));
    db.decide();
    assert_ne!(db.track(a), db.track(b));
    let item = Item { db };
    let before = item.rows();
    let refused = item
        .db
        .writer
        .call(move |c| Ok(super::super::split::s_on(c, &[ta, tb], Some(a))))
        .unwrap();
    assert!(
        matches!(refused, Err(Refused::CopiesInTwoLibraryTracks)),
        "{refused:?}"
    );
    assert_eq!(item.rows(), before);
}

#[test]
fn copies_split_off_stay_apart_through_a_rescan_a_retag_and_their_folder_added_again() {
    let (item, a, mp3s, c) = linked_with_copies(2);
    let (m1, m2) = (mp3s[0], mp3s[1]);
    let id = item.doubt();
    let ta = item.db.track(a).unwrap();
    item.not_same(id, Some(m1)).unwrap();
    let left = item.db.track(m1).unwrap();

    // A rescan finds one copy gone and back, and the other retagged: both
    // are due again, and the grouping job keeps them where the split put
    // them.
    item.db.set_present(m1, false);
    item.db.decide();
    item.db.set_present(m1, true);
    item.db.retag(m2, Some("Velo"), Some("Someone Else"));
    item.db.decide();
    item.db.decide();
    assert_eq!(
        (item.db.track(m1), item.db.track(m2)),
        (Some(left), Some(left))
    );
    assert_eq!(item.db.track(a), Some(ta));
    assert_eq!(item.db.stands(a, m1).1, "decided");

    // A copy's folder removed and added again: another row, another id, the
    // same audio. The decision is kept by audio, so it joins its copy, apart
    // from the Library track.
    item.db.sql(&format!(
        "DELETE FROM recording_file WHERE file_id = {m2}; DELETE FROM file WHERE id = {m2};"
    ));
    let mp3 = reencoded(&track(1, LEN), 1, 7);
    let again = item.db.add(copy("again/Velo.mp3", 2, &mp3));
    assert!(again > m2);
    item.db.decide();
    item.db.decide();
    assert_eq!(item.db.track(again), Some(left));
    assert_eq!(item.db.track(a), Some(ta));
    assert_ne!(item.db.track(c), Some(left));
}

#[test]
fn names_say_versions_reads_the_reasons_of_a_version_mix_up_only() {
    use super::super::split::{names_say_versions, NAMES_SAY_VERSIONS};
    use crate::grouping::rules::Reason;
    // The codes are the rules' own.
    assert_eq!(
        NAMES_SAY_VERSIONS,
        [
            Reason::NamesSayCut {
                only_a: vec![],
                only_b: vec![]
            }
            .code(),
            Reason::NamesSayRework {
                only_a: vec![],
                only_b: vec![]
            }
            .code(),
        ]
    );
    use super::super::reasons::from_grouping;
    // What the comparison shows (at most three reasons, words only), as it
    // gets them.
    let say = |kind: ReviewKind, json: &str| names_say_versions(kind, &from_grouping(json));
    let doubt = ReviewKind::VersionDoubt;
    assert!(say(
        doubt,
        r#"[{"code":"partialAudioNamesUnclear"},{"code":"namesSayRework","only_a":[],"only_b":[]}]"#
    ));
    assert!(say(
        doubt,
        r#"[{"code":"audioInsideOtherNamesDisagree"},{"code":"namesSayCut"},{"code":"artistsDiffer"}]"#
    ));
    // Only what the screen shows counts: a code past the three it shows
    // doesn't (foreman's ruling, 2026-10-24).
    assert!(!say(
        doubt,
        r#"[{"code":"a"},{"code":"b"},{"code":"c"},{"code":"namesSayCut"}]"#
    ));
    assert!(say(
        doubt,
        r#"[{"code":"a"},{"code":"b"},{"code":"namesSayCut"}]"#
    ));
    assert!(!say(
        doubt,
        r#"[{"code":"partialAudioNamesUnclear"},{"code":"namesUnclear"}]"#
    ));
    assert!(!say(
        doubt,
        r#"[{"code":"sameAudioNamesDisagree"},{"code":"titlesDiffer"}]"#
    ));
    assert!(!say(doubt, "not json"));
    assert!(!names_say_versions(doubt, &[]));
    // "Unclear duplicates" never changes.
    assert!(!say(ReviewKind::HeldSet, r#"[{"code":"namesSayRework"}]"#));
}

#[test]
fn the_comparison_says_when_the_names_say_another_rework() {
    // A remix, credited to another artist than the Library track's and not
    // to its remixer, that shares part of its audio with the Library track:
    // the names say different reworks (DOGFOOD B-4).
    let db = db();
    let velo = track(1, LEN);
    let a = db.add(file("Velo.flac", 1, velo.clone()));
    db.decide();
    db.to_library(a);
    let c = db.add(NewFile {
        name: "Velo (Mara Quell Remix).flac",
        title: Some("Velo (Mara Quell Remix)"),
        artist: Some("Pell Orrin"),
        hash: Some(hash(3)),
        items: Some(other_take(&velo, 9)),
        blake3: None,
    });
    db.decide();
    assert_eq!(
        db.stands(a, c),
        ("review".into(), "open".into()),
        "{}",
        db.reasons(a, c)
    );
    let item = Item { db };
    let id = item.doubt();
    let shown = item
        .db
        .writer
        .call(move |c| comparison(c, &Mount, id))
        .unwrap()
        .unwrap();
    assert!(shown.names_say_versions, "{:?}", shown.doubt);

    // The other take of `two_files`: its names don't say.
    let (item, _a, _c) = two_files();
    let id = item.doubt();
    let shown = item
        .db
        .writer
        .call(move |c| comparison(c, &Mount, id))
        .unwrap()
        .unwrap();
    assert!(!shown.names_say_versions, "{:?}", shown.doubt);
}

// ---- fix round 1 (review of PR 106) --------------------------------------------

#[test]
fn the_undo_of_a_split_whose_copies_left_two_tracks_is_refused_once_the_second_changed() {
    // Two copies, each in a track that keeps another file: the split takes
    // files out of two tracks. Undo checks each one, the second too.
    for change in ["made", "audio"] {
        let (item, a, mp3s, c) = linked_with_copies(2);
        let (m1, m2) = (mp3s[0], mp3s[1]);
        let (ta, tc) = (item.db.track(a).unwrap(), item.db.track(c).unwrap());
        // `m2` sits in a track of its own with another file, `x` (the
        // grouping hasn't put the copies together yet).
        let x = item.db.file(1, "Nightdrive.mp3", Some(hash(40)));
        item.db.group();
        item.db.sql(&format!(
            "INSERT INTO recording DEFAULT VALUES;
             UPDATE recording_file SET recording_id = last_insert_rowid()
             WHERE file_id IN ({m2}, {x});"
        ));
        let tx = item.db.track(x).unwrap();
        assert_eq!(item.db.track(m2), Some(tx));
        let s_on = |tracks: Vec<i64>, file: i64| {
            item.db
                .writer
                .call(move |c| Ok(super::super::split::s_on(c, &tracks, Some(file))))
                .unwrap()
        };
        s_on(vec![ta, tx, tc], m1).unwrap();
        let left = item.db.track(m1).unwrap();
        assert!(left != ta && left != tx && left != tc);
        assert_eq!(
            item.db.track(m2),
            Some(left),
            "together, out of both tracks"
        );
        assert_eq!((item.db.track(a), item.db.track(x)), (Some(ta), Some(tx)));
        let sources = item.db.count(&format!(
            "SELECT json_array_length(details, '$.sources') FROM operation
             WHERE kind = '{SPLIT_OPERATION}'"
        ));
        assert_eq!(sources, 2);
        let first = item.db.count(&format!(
            "SELECT json_extract(details, '$.from') FROM operation
             WHERE kind = '{SPLIT_OPERATION}'"
        ));
        assert_eq!(first, ta, "the second track is only in `sources`");

        // The second track changes: it was made at another time (it may be
        // another track by that id), or the file that stayed in it holds
        // other audio now.
        let made: String = item
            .db
            .writer
            .call(move |c| {
                c.query_row(
                    "SELECT created_at FROM recording WHERE id = ?1",
                    [tx],
                    |r| r.get(0),
                )
            })
            .unwrap();
        let (break_it, mend_it) = match change {
            "made" => (
                format!(
                    "UPDATE recording SET created_at = '2000-01-01T00:00:00.000Z' WHERE id = {tx}"
                ),
                format!("UPDATE recording SET created_at = '{made}' WHERE id = {tx}"),
            ),
            _ => (
                format!(
                    "UPDATE file SET audio_hash = x'{}' WHERE id = {x}",
                    "41".repeat(34)
                ),
                format!(
                    "UPDATE file SET audio_hash = x'{}' WHERE id = {x}",
                    "28".repeat(34)
                ),
            ),
        };
        item.db.sql(&break_it);
        let before = item.rows();
        match item.undo() {
            UndoOutcome::Refused { reason, .. } => {
                assert_eq!(reason, UndoRefusal::ChangedSince, "{change}")
            }
            other => panic!("{change}: {other:?}"),
        }
        assert_eq!(item.rows(), before, "{change}: nothing changed");
        // Put back as it was, the Undo goes through.
        item.db.sql(&mend_it);
        assert!(
            matches!(item.undo(), UndoOutcome::Undone { .. }),
            "{change}"
        );
        // The moved files are back and the decisions gone (the grouping
        // pass after the Undo then puts the two copies together again).
        assert_eq!(item.db.track(m1), Some(ta), "{change}");
        assert_eq!(
            item.db.count("SELECT count(*) FROM grouping_decision"),
            0,
            "{change}"
        );
    }
}
