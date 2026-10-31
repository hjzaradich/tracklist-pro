//! Two Library tracks that are the same recording (1bE-9): keep both, or
//! merge them, and the Undo of each. Built on the grouping's own test
//! database, so the pairs stand where the real grouping puts them: two
//! files given the very same audio after both were added to the Library.

use super::super::actions::{Acted, Refused};
use super::super::merge_fields::{MergeChoice, MergeField};
use super::super::same::{
    keep_both, merge, merge_choosing, merge_questions, suggest_keep, KeepCandidate, KeepWhy,
    Merged, KEEP_BOTH_OPERATION, MERGE_OPERATION,
};
use super::*;
use crate::grouping::tests::support::{db, hash, Db};
use crate::ops::UndoOutcome;

/// Two Library tracks holding the same audio, and their files.
struct Twice {
    db: Db,
    /// The files, and their tracks.
    a: i64,
    b: i64,
    ta: i64,
    tb: i64,
}

impl Twice {
    fn new() -> Twice {
        let db = db();
        let a = db.file(1, "Velo.mp3", Some(hash(1)));
        let b = db.file(2, "Velo.mp3", Some(hash(2)));
        db.group();
        let ta = db.to_library(a);
        let tb = db.to_library(b);
        // rekordbox had two entries for it: the same audio, found later.
        db.set_hash(b, Some(hash(1)));
        db.decide();
        assert_eq!(db.stands(a, b).1, "both_in_library");
        Twice { db, a, b, ta, tb }
    }

    fn anchor(&self) -> i64 {
        self.ta.min(self.tb)
    }

    fn item(&self) -> ReviewItemId {
        id(ReviewKind::SameRecording, self.anchor(), None)
    }

    fn lt(&self, track: i64) -> i64 {
        self.db.count(&format!(
            "SELECT id FROM library_track WHERE recording_id = {track}"
        ))
    }

    fn queue(&self) -> Vec<(ReviewKind, i64, Option<i64>)> {
        self.db
            .writer
            .call(|c| items(c, &Mount, ReviewGroup::NeedsCloserLook, 0, 100))
            .unwrap()
            .items
            .into_iter()
            .map(|i| (i.id.kind, i.id.a, i.id.b))
            .collect()
    }

    fn compare(&self) -> Option<ReviewComparison> {
        let id = self.item();
        self.db
            .writer
            .call(move |c| comparison(c, &Mount, id))
            .unwrap()
    }

    fn keep_both(&self, shown: &[i64]) -> Result<Acted, Refused> {
        let (anchor, shown) = (self.anchor(), shown.to_vec());
        self.db
            .writer
            .call(move |c| Ok(keep_both(c, anchor, &shown)))
            .unwrap()
    }

    fn merge(&self, shown: &[i64], keep: i64) -> Result<Merged, Refused> {
        let (anchor, shown) = (self.anchor(), shown.to_vec());
        self.db
            .writer
            .call(move |c| Ok(merge(c, &Mount, anchor, &shown, keep)))
            .unwrap()
    }

    fn both(&self) -> Vec<i64> {
        vec![self.ta, self.tb]
    }

    fn undo(&self) -> UndoOutcome {
        self.db
            .writer
            .call(|c| Ok(crate::ops::undo_for_user(c, None)))
            .unwrap()
            .unwrap()
    }

    /// A trusted rekordbox entry on `file`, with `cues` cues, `rating` as
    /// rekordbox writes it, `plays` and a BPM.
    fn entry(&self, file: i64, cues: usize, rating: u32, plays: u32, bpm: f64) -> i64 {
        self.entry_with(file, cues, rating, plays, bpm, &[])
    }

    /// [`Twice::entry`] with more attributes (a snapshot is read-only, so
    /// they are given when the entry is made).
    fn entry_with(
        &self,
        file: i64,
        cues: usize,
        rating: u32,
        plays: u32,
        bpm: f64,
        extra: &[(&str, &str)],
    ) -> i64 {
        let more: String = extra
            .iter()
            .map(|(name, value)| format!(", '{name}', '{value}'"))
            .collect();
        let track = self.db.track(file).unwrap();
        let n = self.db.count("SELECT count(*) FROM rekordbox_track") + 1;
        let marks = serde_json::to_string(&vec![serde_json::json!({"Num": "-1"}); cues]).unwrap();
        self.db.sql(&format!(
            "INSERT INTO rekordbox_track
                 (attributes, location_key, read_at, file_id, recording_id, relink_method,
                  relink_probable, position_marks)
             VALUES (json_object('TrackID', '{n}', 'Name', 'Velo', 'Artist', 'Someone',
                                 'Location', 'file://localhost/E:/Old/{n}.mp3',
                                 'Rating', '{rating}', 'PlayCount', '{plays}',
                                 'AverageBpm', '{bpm}'{more}),
                     'E:/Old/{n}.mp3', '2026-10-01T10:00:00.000Z', {file}, {track}, 'path', 0,
                     '{marks}')"
        ));
        self.db.count("SELECT max(id) FROM rekordbox_track")
    }

    /// A crate of `kind` (static or smart) holding the given Library tracks
    /// with the given entry kinds.
    fn crate_with(&self, name: &str, kind: &str, entries: &[(i64, &str)]) -> i64 {
        let rules = if kind == "smart" { "'{}'" } else { "NULL" };
        self.db.sql(&format!(
            "INSERT INTO crate (kind, name, rules) VALUES ('{kind}', '{name}', {rules})"
        ));
        let id = self.db.count("SELECT max(id) FROM crate");
        for (track, entry_kind) in entries {
            let lt = self.lt(*track);
            self.db.sql(&format!(
                "INSERT INTO crate_entry (crate_id, library_track_id, kind)
                 VALUES ({id}, {lt}, '{entry_kind}')"
            ));
        }
        id
    }

    /// The crate's entries, as (Library track, kind), in entry order.
    fn crate_entries(&self, crate_id: i64) -> Vec<(i64, String)> {
        self.db
            .writer
            .call(move |c| {
                let mut s = c.prepare(
                    "SELECT library_track_id, kind FROM crate_entry WHERE crate_id = ?1
                     ORDER BY id",
                )?;
                let rows = s.query_map([crate_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .unwrap()
    }

    /// Everything a merge writes and its Undo puts back, as text. A file's
    /// role is the ranking's (derived), so it isn't here.
    fn rows(&self) -> String {
        const TABLES: [&str; 7] = [
            "SELECT 'library_track ' || id || ' ' || recording_id || ' ' || linked_file_id
                    || ' ' || coalesce(last_sent_location, '-') || ' ' || added_at
             FROM library_track ORDER BY id",
            "SELECT 'crate_entry ' || id || ' ' || crate_id || ' ' || library_track_id || ' '
                    || kind || ' ' || added_at FROM crate_entry ORDER BY id",
            "SELECT 'sync_base ' || id || ' ' || library_track_id || ' ' || field || ' ' || value
             FROM sync_base ORDER BY id",
            "SELECT 'conflict ' || id || ' ' || library_track_id || ' ' || field
             FROM conflict ORDER BY id",
            "SELECT 'file ' || file_id || ' ' || recording_id FROM recording_file ORDER BY file_id",
            "SELECT 'replaced ' || id || ' ' || location_key || ' ' || reason
             FROM replaced_entry ORDER BY id",
            "SELECT 'merge ' || id || ' ' || recording_id || ' ' || into_id
             FROM track_merge ORDER BY id",
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

    /// Every track with files has exactly one best file.
    fn one_best_each(&self) {
        let without = self.db.count(
            "SELECT count(*) FROM (SELECT DISTINCT recording_id FROM recording_file) t
             WHERE (SELECT count(*) FROM recording_file rf
                    WHERE rf.recording_id = t.recording_id AND rf.role = 'best') <> 1",
        );
        assert_eq!(without, 0, "a track without exactly one best file");
    }
}

// ---- which track to keep ------------------------------------------------------

fn candidate(recording: i64, cues: u32, plays: u32, rank: usize) -> KeepCandidate {
    KeepCandidate {
        recording,
        cues,
        plays,
        rank,
    }
}

#[test]
fn the_track_with_more_cues_is_kept_then_more_plays_then_the_better_file_then_the_older() {
    let more_cues = [candidate(1, 2, 90, 0), candidate(2, 8, 1, 1)];
    assert_eq!(
        suggest_keep(&more_cues),
        Some((2, KeepWhy::MoreCues { cues: 8, other: 2 }))
    );
    let more_plays = [candidate(1, 4, 3, 0), candidate(2, 4, 12, 1)];
    assert_eq!(
        suggest_keep(&more_plays),
        Some((
            2,
            KeepWhy::MorePlays {
                plays: 12,
                other: 3
            }
        ))
    );
    let better_file = [candidate(1, 0, 0, 3), candidate(2, 0, 0, 0)];
    assert_eq!(
        suggest_keep(&better_file),
        Some((2, KeepWhy::BetterFile { other: 1 }))
    );
    let tied = [candidate(5, 0, 0, 1), candidate(3, 0, 0, 1)];
    assert_eq!(suggest_keep(&tied), Some((3, KeepWhy::Tied)));
    // Three tracks: against the runner-up.
    let three = [
        candidate(1, 3, 0, 0),
        candidate(2, 9, 0, 1),
        candidate(3, 5, 0, 2),
    ];
    assert_eq!(
        suggest_keep(&three),
        Some((2, KeepWhy::MoreCues { cues: 9, other: 5 }))
    );
    assert_eq!(suggest_keep(&[candidate(1, 3, 0, 0)]), None);
}

// ---- the item -------------------------------------------------------------------

#[test]
fn the_item_shows_each_tracks_cues_rating_plays_and_crates_and_suggests_keeping_more_cues() {
    let t = Twice::new();
    t.entry(t.a, 2, 204, 40, 120.0);
    t.entry(t.b, 7, 102, 3, 120.0);
    t.crate_with("Warmup", "static", &[(t.tb, "member")]);
    assert_eq!(
        t.queue(),
        vec![(ReviewKind::SameRecording, t.anchor(), None)]
    );
    let item = t.compare().expect("the item is queued");
    let offer = item.same_recording.expect("a same-recording offer");
    type Facts = (i64, Option<(u32, u32, u32)>, u32);
    let facts: Vec<Facts> = offer
        .tracks
        .iter()
        .map(|t| {
            (
                t.recording_id,
                t.rekordbox.map(|e| (e.cues, e.stars, e.plays)),
                t.crates,
            )
        })
        .collect();
    assert_eq!(
        facts,
        vec![(t.ta, Some((2, 4, 40)), 0), (t.tb, Some((7, 2, 3)), 1)]
    );
    assert_eq!(offer.keep, Some(t.tb));
    // The suggestion is the linked file of the track to keep, with why.
    let suggestion = item.suggestion.expect("a suggestion");
    assert_eq!(*suggestion.what(), t.b);
    let reason = serde_json::to_value(&suggestion.reasons()[0]).unwrap();
    assert_eq!(reason["key"], "review:keep.moreCues");
    assert_eq!(item.rows[0].file_id, Some(t.b));
}

#[test]
fn a_track_rekordbox_doesnt_know_has_no_cues_and_the_known_one_is_suggested() {
    let t = Twice::new();
    t.entry(t.b, 0, 0, 5, 120.0);
    let offer = t.compare().unwrap().same_recording.unwrap();
    assert_eq!(offer.tracks[0].rekordbox, None);
    assert!(offer.tracks[1].rekordbox.is_some());
    assert_eq!(offer.keep, Some(t.tb));
}

// ---- keep both ----------------------------------------------------------------

#[test]
fn keep_both_is_remembered_the_item_leaves_at_once_and_undo_brings_it_back() {
    let t = Twice::new();
    let acted = t.keep_both(&t.both()).unwrap();
    assert!(acted.operation_id.is_some());
    // Gone as the action returns, and for good.
    assert_eq!(t.queue(), vec![]);
    assert_eq!(t.db.stands(t.a, t.b).1, "decided");
    t.db.decide();
    assert_eq!(t.queue(), vec![]);
    assert_eq!(t.db.count("SELECT count(*) FROM library_track"), 2);
    assert_ne!(t.db.track(t.a), t.db.track(t.b));
    // Undo: the decision goes, and the item is back as the undo returns.
    match t.undo() {
        UndoOutcome::Undone { operation } => assert_eq!(operation.kind, KEEP_BOTH_OPERATION),
        other => panic!("{other:?}"),
    }
    assert_eq!(t.db.count("SELECT count(*) FROM grouping_decision"), 0);
    assert_eq!(
        t.queue(),
        vec![(ReviewKind::SameRecording, t.anchor(), None)]
    );
}

#[test]
fn after_keep_both_a_third_copy_stays_a_track_of_its_own_with_no_item() {
    let t = Twice::new();
    t.keep_both(&t.both()).unwrap();
    let c = t.db.file(1, "new/Velo.mp3", Some(hash(1)));
    t.db.decide();
    let tc = t.db.track(c).unwrap();
    assert_ne!(Some(tc), t.db.track(t.a));
    assert_ne!(Some(tc), t.db.track(t.b));
    assert_eq!(t.db.stands(t.a, c).1, "decided");
    assert_eq!(t.queue(), vec![]);
}

#[test]
fn keep_both_is_refused_for_an_item_that_changed_since_the_screen_showed_it() {
    let t = Twice::new();
    assert!(matches!(t.keep_both(&[t.ta]), Err(Refused::ItemGone)));
    assert!(matches!(
        t.keep_both(&[t.ta, t.tb, t.tb + 50]),
        Err(Refused::ItemGone)
    ));
    t.keep_both(&t.both()).unwrap();
    assert!(matches!(t.keep_both(&t.both()), Err(Refused::ItemGone)));
}

// ---- merge ------------------------------------------------------------------------

#[test]
fn a_merge_moves_crates_files_and_the_left_entry_and_undo_puts_every_row_back() {
    let t = Twice::new();
    let (keep, leave) = (t.ta, t.tb);
    let (lt_keep, lt_leave) = (t.lt(keep), t.lt(leave));
    t.entry(t.a, 4, 255, 9, 120.0);
    t.entry(t.b, 1, 51, 2, 128.0);
    // A crate holding only the track that leaves, a smart crate always
    // including it, and a crate holding both (the kept one's entry wins).
    let only = t.crate_with("Warmup", "static", &[(leave, "member")]);
    let smart = t.crate_with("Peak", "smart", &[(leave, "always_include")]);
    let both = t.crate_with("Closing", "static", &[(keep, "member"), (leave, "member")]);
    t.db.sql(&format!(
        "INSERT INTO sync_base (library_track_id, field, value) VALUES ({lt_leave}, 'Name', 'Velo');
         INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
             VALUES ({lt_leave}, 'Genre', 'House', 'Techno', 'Disco');"
    ));
    let before = t.rows();

    let merged = t.merge(&t.both(), keep).unwrap();
    assert!(merged.operation_id.is_some());
    assert_eq!((merged.kept, merged.merged.clone()), (keep, vec![leave]));
    // One Library track; the other's files are in it; its track is kept,
    // with no files, for the Undo.
    assert_eq!(t.db.count("SELECT count(*) FROM library_track"), 1);
    assert_eq!(t.db.track(t.b), Some(keep));
    assert_eq!(
        t.db.count(&format!(
            "SELECT count(*) FROM recording WHERE id = {leave}"
        )),
        1
    );
    assert_eq!(
        t.db.count(&format!(
            "SELECT count(*) FROM track_merge WHERE recording_id = {leave} AND into_id = {keep}"
        )),
        1
    );
    // Crates: the kept track with the same kind; where both were, one entry,
    // the kept track's own.
    assert_eq!(t.crate_entries(only), vec![(lt_keep, "member".into())]);
    assert_eq!(
        t.crate_entries(smart),
        vec![(lt_keep, "always_include".into())]
    );
    assert_eq!(t.crate_entries(both), vec![(lt_keep, "member".into())]);
    assert_eq!(
        t.db.count(&format!(
            "SELECT count(*) FROM sync_base WHERE library_track_id = {lt_leave}"
        )) + t.db.count(&format!(
            "SELECT count(*) FROM conflict WHERE library_track_id = {lt_leave}"
        )),
        0
    );
    // The entry the track that left was sent as goes on the list to remove
    // in rekordbox.
    assert_eq!(
        t.db.count(
            "SELECT count(*) FROM replaced_entry
             WHERE location_key = 'E:/Old/2.mp3' AND reason = 'merge'"
        ),
        1
    );
    let removals =
        t.db.writer
            .call(|c| crate::after_send::manual_removals(c, &Mount))
            .unwrap();
    assert_eq!(removals.len(), 1);
    assert_eq!(removals[0].reason, crate::after_send::RemovalReason::Merged);
    // The item is gone as the action returns, and the merged track ranked.
    assert_eq!(t.queue(), vec![]);
    t.one_best_each();
    t.db.decide();
    assert_eq!(t.queue(), vec![]);
    assert_eq!(t.db.stands(t.a, t.b).1, "merged");

    // Undo: both tracks and their crates are back, and the item.
    match t.undo() {
        UndoOutcome::Undone { operation } => assert_eq!(operation.kind, MERGE_OPERATION),
        other => panic!("{other:?}"),
    }
    assert_eq!(t.rows(), before);
    assert_eq!(t.db.track(t.b), Some(leave));
    t.one_best_each();
    assert_eq!(t.db.stands(t.a, t.b).1, "both_in_library");
    assert_eq!(
        t.queue(),
        vec![(ReviewKind::SameRecording, t.anchor(), None)]
    );
}

#[test]
fn either_track_can_stay_and_the_one_that_leaves_never_gives_the_kept_one_its_analysis() {
    // The track that leaves has more plays and its file is the best one;
    // the kept track's BPM still comes from its own entry.
    let t = Twice::new();
    t.entry(t.a, 2, 0, 50, 128.0);
    t.entry(t.b, 0, 0, 1, 120.0);
    t.merge(&t.both(), t.tb).unwrap();
    t.db.sql(&format!(
        "UPDATE recording_file SET role = CASE file_id WHEN {} THEN 'best' ELSE 'extra' END
         WHERE recording_id = {}",
        t.a, t.tb
    ));
    t.db.writer.call(crate::attach::attach).unwrap();
    assert_eq!(
        t.db.count(&format!(
            "SELECT count(*) FROM analysis
             WHERE recording_id = {} AND source = 'rekordbox' AND bpm = 120",
            t.tb
        )),
        1
    );
    assert_eq!(
        t.db.count("SELECT count(*) FROM analysis WHERE source = 'rekordbox' AND bpm = 128"),
        0
    );
    let entries =
        t.db.writer
            .call({
                let tb = t.tb;
                move |c| crate::attach::for_recording(c, tb)
            })
            .unwrap();
    let source: Vec<i64> = entries
        .iter()
        .filter(|e| e.is_analysis_source)
        .map(|e| e.file_id)
        .collect();
    assert_eq!(source, vec![t.b]);
}

#[test]
fn a_kept_track_rekordbox_doesnt_know_takes_no_analysis_from_the_track_that_left() {
    let t = Twice::new();
    t.entry(t.a, 3, 0, 5, 128.0);
    t.merge(&t.both(), t.tb).unwrap();
    t.db.writer.call(crate::attach::attach).unwrap();
    assert_eq!(
        t.db.count("SELECT count(*) FROM analysis WHERE source = 'rekordbox'"),
        0
    );
}

#[test]
fn a_track_never_sent_and_unknown_to_rekordbox_leaves_nothing_to_remove() {
    let t = Twice::new();
    t.merge(&t.both(), t.ta).unwrap();
    assert_eq!(t.db.count("SELECT count(*) FROM replaced_entry"), 0);
}

#[test]
fn a_track_sent_that_no_read_shows_yet_leaves_the_location_it_was_sent_to() {
    let t = Twice::new();
    t.db.sql(&format!(
        "UPDATE library_track SET last_sent_location = 'file://localhost/E:/B/Velo.mp3',
                                  last_exported_at = '2026-10-15T10:00:00.000Z'
         WHERE recording_id = {}",
        t.tb
    ));
    t.merge(&t.both(), t.ta).unwrap();
    assert_eq!(
        t.db.count(
            "SELECT count(*) FROM replaced_entry
             WHERE location = 'file://localhost/E:/B/Velo.mp3' AND reason = 'merge'"
        ),
        1
    );
}

#[test]
fn a_third_copy_joins_the_merged_track_and_stays_there_after_the_undo() {
    let t = Twice::new();
    let c = t.db.file(1, "new/Velo.mp3", Some(hash(1)));
    t.db.decide();
    // One item for the set, the copy with it.
    assert_eq!(
        t.queue(),
        vec![(ReviewKind::SameRecording, t.anchor(), None)]
    );
    let item = t.compare().unwrap();
    assert_eq!(item.tracks.len(), 3);
    assert_eq!(item.same_recording.unwrap().tracks.len(), 2);

    t.merge(&t.both(), t.ta).unwrap();
    // As the action returns.
    assert_eq!(t.db.track(c), Some(t.ta));
    assert_eq!(t.queue(), vec![]);

    t.undo();
    // A merge is never undone by the job: the copy stays where it went.
    assert_eq!(t.db.track(c), Some(t.ta));
    assert_eq!(t.db.track(t.b), Some(t.tb));
    assert_eq!(
        t.queue(),
        vec![(ReviewKind::SameRecording, t.anchor(), None)]
    );
    t.one_best_each();
}

#[test]
fn a_merge_can_be_undone_after_a_send() {
    let t = Twice::new();
    t.entry(t.a, 1, 0, 1, 120.0);
    t.entry(t.b, 1, 0, 1, 120.0);
    let lt_keep = t.lt(t.ta);
    t.merge(&t.both(), t.ta).unwrap();
    // A send: it marks the track it sent and records its bases, outside the
    // log.
    t.db.sql(&format!(
        "UPDATE library_track SET last_sent_location = 'file://localhost/E:/Old/1.mp3',
                                  last_exported_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = {lt_keep};
         INSERT INTO sync_base (library_track_id, field, value) VALUES ({lt_keep}, 'Name', 'Velo');"
    ));
    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(t.db.count("SELECT count(*) FROM library_track"), 2);
    assert_eq!(t.db.count("SELECT count(*) FROM replaced_entry"), 0);
    assert_eq!(
        t.queue(),
        vec![(ReviewKind::SameRecording, t.anchor(), None)]
    );
}

#[test]
fn a_merge_deletes_the_app_values_of_the_track_that_leaves_and_undo_puts_them_back() {
    // The leaving track's values are deleted with it; the one value only it
    // has (a rating the staying track has none of) carries over (1cB-20).
    let t = Twice::new();
    t.entry(t.a, 1, 0, 1, 120.0);
    t.entry(t.b, 1, 0, 1, 120.0);
    let (lt_keep, lt_leave) = (t.lt(t.ta), t.lt(t.tb));
    t.db.sql(&format!(
        "INSERT INTO app_value (library_track_id, field, value, replaced)
             VALUES ({lt_keep}, 'Genre', 'House', 'Disco'), ({lt_leave}, 'Genre', 'Techno', NULL),
                    ({lt_leave}, 'Rating', '255', '0');"
    ));
    let values = || {
        t.db.writer
            .call(|c| {
                c.prepare(
                    "SELECT id || ' ' || library_track_id || ' ' || field || ' ' || value
                     FROM app_value ORDER BY id",
                )?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()
            })
            .unwrap()
    };
    let before = values();

    t.merge(&t.both(), t.ta).unwrap();

    assert_eq!(
        values(),
        [
            format!("1 {lt_keep} Genre House"),
            format!("4 {lt_keep} Rating 255")
        ]
    );
    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(values(), before);
}

#[test]
fn a_merge_deletes_the_sent_playlist_entries_of_the_track_that_leaves_and_undo_puts_them_back() {
    // Like its sync bases: the entries were the leaving track's rekordbox
    // entry's, and the track that stays keeps its own (1cB-19).
    let t = Twice::new();
    t.entry(t.a, 1, 0, 1, 120.0);
    t.entry(t.b, 1, 0, 1, 120.0);
    let (lt_keep, lt_leave) = (t.lt(t.ta), t.lt(t.tb));
    t.db.sql(&format!(
        r#"INSERT INTO sent_entry (library_track_id, path, path_key, send_id, sent_at)
             VALUES ({lt_keep}, '["Crates","Peak"]', '["crates","peak"]', 1, 'then'),
                    ({lt_leave}, '["Crates","Peak"]', '["crates","peak"]', 1, 'then'),
                    ({lt_leave}, '["Crates","Old"]', '["crates","old"]', 1, 'then');"#
    ));
    let entries = || {
        t.db.writer
            .call(|c| {
                c.prepare(
                    "SELECT id || ' ' || library_track_id || ' ' || path_key
                     FROM sent_entry ORDER BY id",
                )?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()
            })
            .unwrap()
    };
    let before = entries();

    t.merge(&t.both(), t.ta).unwrap();

    assert_eq!(entries(), [format!(r#"1 {lt_keep} ["crates","peak"]"#)]);
    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(entries(), before);
}

#[test]
fn a_merge_is_refused_for_a_changed_item_a_track_not_in_it_or_linked_versions() {
    let t = Twice::new();
    let before = t.rows();
    assert!(matches!(t.merge(&[t.ta], t.ta), Err(Refused::ItemGone)));
    assert!(matches!(
        t.merge(&t.both(), t.tb + 50),
        Err(Refused::ItemGone)
    ));
    // A link the job made between the two, before any pass took it away.
    t.db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({}, {}, 'cut', 'parser')",
        t.ta, t.tb
    ));
    assert!(matches!(
        t.merge(&t.both(), t.ta),
        Err(Refused::LinkedVersions)
    ));
    assert_eq!(t.rows(), before);
}

#[test]
fn a_merged_item_is_gone_and_merging_it_again_is_refused() {
    let t = Twice::new();
    t.merge(&t.both(), t.ta).unwrap();
    assert!(t.compare().is_none());
    assert!(matches!(t.merge(&t.both(), t.ta), Err(Refused::ItemGone)));
}

// ---- fix round 1 (review of PR 94) -------------------------------------------

#[test]
fn the_undo_of_a_merge_is_refused_once_the_staying_track_followed_its_file_to_a_moved_one() {
    let t = Twice::new();
    let lt_keep = t.lt(t.ta);
    t.merge(&t.both(), t.ta).unwrap();
    // The kept track's file goes, and the automatic follow moves its link to
    // the same audio: the file the merge moved into its track.
    t.db.sql(&format!("UPDATE file SET present = 0 WHERE id = {}", t.a));
    let b = t.b;
    t.db.writer
        .call(move |c| {
            Ok(crate::ops::record_automatic(
                c,
                crate::ops::AutomaticKind::FollowFile,
                &serde_json::json!({}),
                |rec| rec.set("library_track", lt_keep, "linked_file_id", b),
            ))
        })
        .unwrap()
        .unwrap();
    let before = t.rows();
    match t.undo() {
        UndoOutcome::Refused { reason, .. } => {
            assert_eq!(reason, crate::ops::UndoRefusal::MergeLinkChanged)
        }
        other => panic!("not refused: {other:?}"),
    }
    // Nothing changed, and Undo says so beforehand too.
    assert_eq!(t.rows(), before);
    let next =
        t.db.writer
            .call(|c| Ok(crate::ops::next_undo(c)))
            .unwrap()
            .unwrap();
    assert_eq!(
        next.refusal,
        Some(crate::ops::UndoRefusal::MergeLinkChanged)
    );
    assert_eq!(
        t.db.count("SELECT count(*) FROM library_track"),
        1,
        "one Library track, on one file"
    );
}

#[test]
fn the_undo_of_a_merge_is_refused_once_the_staying_track_is_linked_to_another_file() {
    // Any other file: here one of its own, outside the merge.
    let t = Twice::new();
    let lt_keep = t.lt(t.ta);
    let other = t.db.file(1, "own/Velo.mp3", Some(hash(1)));
    t.db.group();
    t.merge(&t.both(), t.ta).unwrap();
    t.db.writer
        .call(move |c| {
            Ok(crate::ops::record_automatic(
                c,
                crate::ops::AutomaticKind::FollowFile,
                &serde_json::json!({}),
                |rec| rec.set("library_track", lt_keep, "linked_file_id", other),
            ))
        })
        .unwrap()
        .unwrap();
    assert!(matches!(
        t.undo(),
        UndoOutcome::Refused {
            reason: crate::ops::UndoRefusal::MergeLinkChanged,
            ..
        }
    ));
}

#[test]
fn an_action_settles_only_the_items_own_files_and_leaves_the_rest_to_the_job() {
    let t = Twice::new();
    // A file elsewhere that no pass has placed yet.
    let elsewhere = t.db.file(2, "unrelated.mp3", Some(hash(9)));
    assert_eq!(t.db.track(elsewhere), None);
    t.merge(&t.both(), t.ta).unwrap();
    assert_eq!(t.db.track(elsewhere), None, "a whole pass ran");
    t.db.group();
    assert!(t.db.track(elsewhere).is_some());
}

#[test]
fn linked_versions_get_keep_both_only_and_no_suggested_track() {
    let t = Twice::new();
    t.db.sql(&format!(
        "INSERT INTO version_link (recording_a, recording_b, kind, source)
         VALUES ({}, {}, 'cut', 'parser')",
        t.ta, t.tb
    ));
    let item = t.compare().expect("still queued until a pass");
    let offer = item.same_recording.unwrap();
    assert!(offer.linked_versions);
    assert_eq!(offer.keep, None);
    assert!(item.suggestion.is_none());
    assert!(t.keep_both(&t.both()).is_ok());
}

/// How long one merge takes beside 100,000 other files, and a whole pass for
/// comparison. Ignored: run by hand (`cargo test --lib -- --ignored
/// a_merge_beside_100000_files`).
#[test]
#[ignore]
fn a_merge_beside_100000_files() {
    let t = Twice::new();
    let decided_as = crate::grouping::decided_as();
    t.db.sql(&format!(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000)
         INSERT INTO file (music_folder_id, rel_path, rel_path_key, size, mtime, audio_hash)
         SELECT 1, 'bulk/' || i || '.mp3', 'bulk/' || i || '.mp3', 100, 1000, randomblob(34)
         FROM n;
         INSERT INTO recording (id) SELECT id + 10000000 FROM file WHERE rel_path LIKE 'bulk/%';
         INSERT INTO recording_file (recording_id, file_id, role)
         SELECT id + 10000000, id, 'best' FROM file WHERE rel_path LIKE 'bulk/%';
         INSERT INTO grouping_file (file_id, placed_key, decided_as)
         SELECT id, audio_hash, {decided_as} FROM file WHERE rel_path LIKE 'bulk/%';"
    ));
    let started = std::time::Instant::now();
    t.merge(&t.both(), t.ta).unwrap();
    let merge = started.elapsed();
    let started = std::time::Instant::now();
    t.db.group();
    let whole = started.elapsed();
    eprintln!("one merge beside 100,000 files: {merge:?}; a whole grouping pass: {whole:?}");
    assert_eq!(t.db.track(t.b), Some(t.ta));
}

/// What [`crate::send_values::sent_as`] says the Library track of `track`
/// is sent as.
fn sent(t: &Twice, track: i64) -> crate::send_values::SentAs {
    let lt = crate::library::LibraryTrackId(t.lt(track));
    t.db.writer
        .call(move |c| crate::send_values::sent_as(c, &Mount, &[(lt, None)]))
        .unwrap()[0]
}

#[test]
fn a_staying_track_with_no_entry_is_sent_as_the_leaving_tracks_byte_identical_entry_never_also_listed(
) {
    // 1bE-11: the staying track's file has no rekordbox entry, the leaving
    // track's file is a byte-identical copy of it and has one. After the
    // merge that file is in the staying track, so the staying track is sent
    // as that entry, and the entry the merge recorded isn't listed to
    // remove: one entry is never both sent and listed. With other bytes
    // (another tag), the staying track goes as new and the entry is listed.
    use crate::send_values::SentAs;
    for same_bytes in [true, false] {
        let t = Twice::new();
        let version = i64::from(crate::hash::DEFINITION);
        for (file, bytes) in [(t.a, "01"), (t.b, if same_bytes { "01" } else { "02" })] {
            let blake3 = bytes.repeat(32);
            t.db.sql(&format!(
                "UPDATE file SET blake3 = x'{blake3}' WHERE id = {file};
                 INSERT OR REPLACE INTO file_stage (file_id, stage, version, size, mtime, status)
                 SELECT id, 'hash', {version}, size, mtime, 'done' FROM file WHERE id = {file};"
            ));
        }
        let entry = t.entry(t.b, 3, 0, 5, 128.0);
        assert_eq!(sent(&t, t.ta), SentAs::New);
        assert_eq!(sent(&t, t.tb), SentAs::Entry(entry));

        t.merge(&t.both(), t.ta).unwrap();
        assert_eq!(
            t.db.count("SELECT count(*) FROM replaced_entry WHERE reason = 'merge'"),
            1
        );
        let removals =
            t.db.writer
                .call(|c| crate::after_send::manual_removals(c, &Mount))
                .unwrap();
        if same_bytes {
            assert_eq!(sent(&t, t.ta), SentAs::Entry(entry));
            assert_eq!(removals, []);
        } else {
            assert_eq!(sent(&t, t.ta), SentAs::New);
            assert_eq!(removals.len(), 1);
            assert_eq!(removals[0].reason, crate::after_send::RemovalReason::Merged);
        }

        // Undo: two tracks again, each sent as before, nothing listed.
        match t.undo() {
            UndoOutcome::Undone { operation } => assert_eq!(operation.kind, MERGE_OPERATION),
            other => panic!("{other:?}"),
        }
        assert_eq!(sent(&t, t.ta), SentAs::New);
        assert_eq!(sent(&t, t.tb), SentAs::Entry(entry));
        let removals =
            t.db.writer
                .call(|c| crate::after_send::manual_removals(c, &Mount))
                .unwrap();
        assert_eq!(removals, []);
    }
}

// ---- ids never reused (1bE-ids) ------------------------------------------------

#[test]
fn a_file_scanned_after_a_moved_files_folder_was_removed_never_takes_its_id_or_moves_on_undo() {
    // The merge moved `b`, the newest file, into the track that stays. Its
    // music folder is removed, and an unrelated file with the staying
    // track's audio is scanned: SQLite gave it `b`'s id before migration
    // 0031, and the merge's Undo moved it out of the Library track.
    let t = Twice::new();
    t.merge(&t.both(), t.ta).unwrap();
    assert_eq!(t.db.track(t.b), Some(t.ta));
    assert_eq!(
        t.db.count("SELECT max(id) FROM file"),
        t.b,
        "b is the newest file"
    );
    t.db.writer
        .call(|c| crate::scan::folders::remove(c, crate::scan::folders::MusicFolderId(2)))
        .unwrap()
        .unwrap();
    let e = t.db.file(1, "Nightdrive.mp3", Some(hash(1)));
    t.db.group();
    assert!(e > t.b, "a new file takes a new id: {e}");
    assert_eq!(t.db.track(e), Some(t.ta), "it joins the track that stayed");

    let before = t.rows();
    assert!(
        matches!(t.undo(), UndoOutcome::Refused { .. }),
        "the moved file is gone: the undo is refused"
    );
    assert_eq!(t.rows(), before, "nothing changed");
    assert_eq!(t.db.track(e), Some(t.ta), "the new file stays where it is");
}

// ---- merging asks field by field (1cB-20) ---------------------------------------

impl Twice {
    /// The app's own value of a field for a track, as the user set it
    /// (`replaced`: rekordbox showed nothing for it).
    fn own_value(&self, track: i64, field: &str, value: &str) {
        let lt = self.lt(track);
        self.db.sql(&format!(
            "INSERT INTO app_value (library_track_id, field, value, replaced)
             VALUES ({lt}, '{field}', '{value}', '')"
        ));
    }

    /// The app's own values, as text, with their ids and what they replaced.
    fn own_values(&self) -> Vec<String> {
        self.db
            .writer
            .call(|c| {
                c.prepare(
                    "SELECT id || ' ' || library_track_id || ' ' || field || ' ' || value
                            || ' ' || coalesce(replaced, '-')
                     FROM app_value ORDER BY id",
                )?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect()
            })
            .unwrap()
    }

    /// What a merge into `keep` asks.
    fn questions(&self, keep: i64) -> Vec<MergeField> {
        let (anchor, shown) = (self.anchor(), self.both());
        self.db
            .writer
            .call(move |c| Ok(merge_questions(c, &Mount, anchor, &shown, keep)))
            .unwrap()
            .unwrap()
    }

    fn merge_choosing(&self, keep: i64, choices: Vec<MergeChoice>) -> Result<Merged, Refused> {
        let (anchor, shown) = (self.anchor(), self.both());
        self.db
            .writer
            .call(move |c| Ok(merge_choosing(c, &Mount, anchor, &shown, keep, &choices)))
            .unwrap()
    }

    fn operations(&self) -> i64 {
        self.db.count("SELECT count(*) FROM operation")
    }

    /// Whether the app's own values include `field` = `value` for `track`.
    fn owns(&self, track: i64, field: &str, value: &str) -> bool {
        let lt = self.lt(track);
        let wanted = format!(" {lt} {field} {value} ");
        self.own_values()
            .iter()
            .any(|v| format!("{v} ").contains(&wanted))
    }
}

fn pick(field: &str, from: i64, value: &str) -> MergeChoice {
    MergeChoice {
        field: field.to_owned(),
        from,
        value: value.to_owned(),
    }
}

/// Two tracks that differ in every way the merge cares about. Track `a`
/// (rating 4 stars) and `b` (rating 2) both have an entry in rekordbox; the
/// user set a genre on each (different), a label on `b` only, a comment in
/// rekordbox on `b` only; their colors agree; their albums differ in
/// rekordbox, which is nobody's choice. Returns the fixture.
fn differing() -> Twice {
    let t = Twice::new();
    t.entry_with(
        t.a,
        2,
        204,
        5,
        120.0,
        &[("Colour", "0xFF0000"), ("Album", "First")],
    );
    t.entry_with(
        t.b,
        1,
        102,
        1,
        120.0,
        &[
            ("Colour", "0xFF0000"),
            ("Album", "Second"),
            ("Comments", "Warm / Peak"),
        ],
    );
    t.own_value(t.ta, "Genre", "Techno");
    t.own_value(t.tb, "Genre", "House");
    t.own_value(t.tb, "Label", "Made Up Records");
    t
}

#[test]
fn merge_rows_appear_only_for_fields_that_differ_and_hold_a_user_value() {
    let t = differing();
    let asked = t.questions(t.ta);
    // The genres differ (both the user's) and so do the ratings (rekordbox's
    // rating is the user's). Not the colors (alike), not the label or the
    // comment (one track has them), not the album (rekordbox's, nobody's).
    let names: Vec<&str> = asked.iter().map(|f| f.field.as_str()).collect();
    assert_eq!(names, ["Genre", "Rating"]);
    let rating = &asked[1];
    assert_eq!(rating.kind, crate::conflict_review::ValueKind::Rating);
    assert_eq!(
        rating
            .values
            .iter()
            .map(|v| (v.recording_id, v.value.as_deref()))
            .collect::<Vec<_>>(),
        [(t.ta, Some("204")), (t.tb, Some("102"))]
    );
}

#[test]
fn a_field_without_a_user_value_on_either_track_is_never_asked() {
    let t = Twice::new();
    t.entry_with(t.a, 0, 0, 0, 120.0, &[("Genre", "Disco"), ("Year", "1999")]);
    t.entry_with(t.b, 0, 0, 0, 120.0, &[("Genre", "Funk"), ("Year", "2001")]);
    assert!(t.questions(t.ta).is_empty());
}

#[test]
fn the_default_of_each_row_is_the_staying_tracks_value() {
    let t = differing();
    for keep in [t.ta, t.tb] {
        let asked = t.questions(keep);
        assert_eq!(asked.len(), 2);
        for field in asked {
            assert_eq!(field.chosen, keep, "{}", field.field);
            assert_eq!(field.values[0].recording_id, keep);
        }
    }
    // Merging without a choice keeps the staying track's genre.
    t.merge(&t.both(), t.tb).unwrap();
    assert!(t.owns(t.tb, "Genre", "House"), "{:?}", t.own_values());
}

#[test]
fn a_value_only_one_track_has_carries_over_without_asking() {
    let t = differing();
    t.merge(&t.both(), t.ta).unwrap();
    let own = t.own_values();
    // The label (the app's own value) and the comment (rekordbox's, My Tags
    // among it) of the track that left are now the staying track's own; the
    // rating and the genre stay what the staying track had.
    assert!(t.owns(t.ta, "Label", "Made Up Records"), "{own:?}");
    assert!(t.owns(t.ta, "Comments", "Warm / Peak"), "{own:?}");
    assert!(t.owns(t.ta, "Genre", "Techno"), "{own:?}");
    assert!(!own.iter().any(|v| v.contains("Rating")), "{own:?}");
}

#[test]
fn the_chosen_values_land_in_the_app_values_and_the_next_send_carries_them() {
    let t = differing();
    t.merge_choosing(
        t.ta,
        vec![pick("Rating", t.tb, "102"), pick("Genre", t.tb, "House")],
    )
    .unwrap();
    // The rating replaced rekordbox's value for the staying track.
    assert!(t.owns(t.ta, "Genre", "House"), "{:?}", t.own_values());
    assert!(t.owns(t.ta, "Rating", "102 204"), "{:?}", t.own_values());

    // The merge before the next send chooses them, by the 1.10 rules.
    let lt = crate::library::LibraryTrackId(t.lt(t.ta));
    let merged =
        t.db.writer
            .call(move |c| crate::conflicts::merge_before_send(c, &Mount, &[lt]))
            .unwrap();
    let chosen = &merged.chosen[&lt];
    for (field, value) in [
        ("Rating", "102"),
        ("Genre", "House"),
        ("Label", "Made Up Records"),
        ("Comments", "Warm / Peak"),
    ] {
        assert!(
            chosen.contains(&(field.to_owned(), value.to_owned())),
            "{field}: {chosen:?}"
        );
    }
    assert!(merged.open.is_empty(), "{:?}", merged.open);
}

#[test]
fn a_merge_with_choices_is_one_operation_and_undo_restores_both_tracks_and_their_values() {
    let t = differing();
    let (lt_keep, lt_leave) = (t.lt(t.ta), t.lt(t.tb));
    t.db.sql(&format!(
        "INSERT INTO sync_base (library_track_id, field, value)
             VALUES ({lt_keep}, 'Rating', '153'), ({lt_leave}, 'Rating', '102'),
                    ({lt_leave}, 'Genre', 'House');"
    ));
    let (rows, own, operations) = (t.rows(), t.own_values(), t.operations());

    t.merge_choosing(
        t.ta,
        vec![pick("Rating", t.tb, "102"), pick("Genre", t.tb, "House")],
    )
    .unwrap();

    assert_eq!(
        t.operations(),
        operations + 1,
        "one operation, values and all"
    );
    assert_ne!(t.own_values(), own);
    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(t.rows(), rows);
    assert_eq!(t.own_values(), own);
    assert_eq!(
        t.queue(),
        vec![(ReviewKind::SameRecording, t.anchor(), None)]
    );
}

#[test]
fn a_choice_the_screen_did_not_show_refuses_the_merge_and_changes_nothing() {
    let t = differing();
    let (rows, own, operations) = (t.rows(), t.own_values(), t.operations());
    // A value the track doesn't hold, a field that wasn't asked, a track
    // that isn't in the item.
    for choice in [
        pick("Rating", t.tb, "255"),
        pick("Label", t.tb, "Made Up Records"),
        pick("Rating", t.tb + 50, "102"),
    ] {
        assert!(matches!(
            t.merge_choosing(t.ta, vec![choice]),
            Err(Refused::ItemGone)
        ));
    }
    assert_eq!(t.rows(), rows);
    assert_eq!(t.own_values(), own);
    assert_eq!(t.operations(), operations);
}

// ---- a pick on top of a field rekordbox changed since the last send (PR 129, A) --

/// The staying track's rating was sent as `base`, then rekordbox's newest
/// read shows 2 stars (102); the leaving track shows 3 stars (153) and the
/// user picks `pick_value` in the merge. Returns what the next merge before
/// a send carries, the fields left open, and the fields taken from rekordbox.
fn pick_over_a_changed_field(
    base: &str,
    pick_value: &str,
) -> (Vec<(String, String)>, Vec<String>, Vec<String>) {
    let t = Twice::new();
    t.entry(t.a, 1, 102, 1, 120.0);
    t.entry(t.b, 1, 153, 1, 120.0);
    let lt_keep = t.lt(t.ta);
    t.db.sql(&format!(
        "INSERT INTO sync_base (library_track_id, field, value) VALUES ({lt_keep}, 'Rating', '{base}')"
    ));
    assert!(t.questions(t.ta).iter().any(|f| f.field == "Rating"));
    t.merge_choosing(t.ta, vec![pick("Rating", t.tb, pick_value)])
        .unwrap();
    let lt = crate::library::LibraryTrackId(lt_keep);
    let merged =
        t.db.writer
            .call(move |c| crate::conflicts::merge_before_send(c, &Mount, &[lt]))
            .unwrap();
    (
        merged.chosen.get(&lt).cloned().unwrap_or_default(),
        merged.open.get(&lt).cloned().unwrap_or_default(),
        merged.taken_in.iter().map(|(_, f)| f.clone()).collect(),
    )
}

#[test]
fn a_pick_equal_to_the_last_sent_value_is_sent_and_not_taken_back_by_rekordbox() {
    // Sent as 3 stars, rekordbox changed it to 2, the user picks 3 stars.
    let (chosen, open, taken) = pick_over_a_changed_field("153", "153");
    assert!(
        chosen.contains(&("Rating".to_owned(), "153".to_owned())),
        "chosen={chosen:?} open={open:?} taken={taken:?}"
    );
    assert!(taken.is_empty() && open.is_empty(), "{taken:?} {open:?}");
}

#[test]
fn a_pick_that_differs_from_the_last_sent_value_and_from_rekordbox_is_not_asked_again() {
    // Sent as 4 stars, rekordbox changed it to 2, the user picks 3 stars.
    let (chosen, open, _) = pick_over_a_changed_field("204", "153");
    assert!(open.is_empty(), "asked again: {open:?}");
    assert!(
        chosen.contains(&("Rating".to_owned(), "153".to_owned())),
        "{chosen:?}"
    );
}

// ---- a pick answers an open conflict on the staying track (PR 129, B) ----------

#[test]
fn a_pick_settles_an_open_conflict_on_the_staying_track_and_undo_opens_it_again() {
    let t = Twice::new();
    // The staying track: 2 stars in rekordbox, 4 stars of the app's own, 3
    // stars last sent, and the conflict that says so. The other track has 5.
    t.entry(t.a, 1, 102, 1, 120.0);
    t.entry(t.b, 1, 255, 1, 120.0);
    t.own_value(t.ta, "Rating", "204");
    let lt_keep = t.lt(t.ta);
    t.db.sql(&format!(
        "INSERT INTO sync_base (library_track_id, field, value) VALUES ({lt_keep}, 'Rating', '153');
         INSERT INTO conflict (library_track_id, field, app_value, rekordbox_value, base_value)
             VALUES ({lt_keep}, 'Rating', '204', '102', '153');"
    ));
    let open = || {
        t.db.count("SELECT count(*) FROM conflict WHERE status = 'open'")
    };
    let (rows, own) = (t.rows(), t.own_values());

    t.merge_choosing(t.ta, vec![pick("Rating", t.tb, "255")])
        .unwrap();

    // Conflicts no longer lists the value the user just replaced, so "Keep
    // app's" can't bring it back; the next send carries the pick.
    assert_eq!(open(), 0);
    assert!(t.owns(t.ta, "Rating", "255 102"), "{:?}", t.own_values());
    let lt = crate::library::LibraryTrackId(lt_keep);
    let merged =
        t.db.writer
            .call(move |c| crate::conflicts::merge_before_send(c, &Mount, &[lt]))
            .unwrap();
    assert!(merged.chosen[&lt].contains(&("Rating".to_owned(), "255".to_owned())));
    assert!(merged.open.is_empty(), "{:?}", merged.open);

    // Undo puts the conflict, the base and the old value back.
    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    assert_eq!(open(), 1);
    assert_eq!(t.rows(), rows);
    assert_eq!(t.own_values(), own);
}

// ---- the gap flags follow a merge and its Undo (1cC-U1b) ----------------------------

#[test]
fn a_merge_and_its_undo_refresh_the_missing_tags_flag_once_the_index_runs() {
    use crate::search::flag::MISSING_TAGS;
    let t = Twice::new();
    // The kept track's own file has a genre and no title or artist; the other
    // file has all three. A track may take its title and artist from another
    // of its files, so the merge brings the kept track what it lacked.
    let tags = |fields: &[(&str, &str)]| {
        let items: Vec<_> = fields
            .iter()
            .map(|(k, v)| serde_json::json!({"key": k, "value": {"type": "text", "text": v}}))
            .collect();
        serde_json::json!({ "id3v2": items }).to_string()
    };
    let genre_only = tags(&[("TCON", "House")]);
    let complete = tags(&[("TIT2", "Velo"), ("TPE1", "Someone"), ("TCON", "House")]);
    let (a, b) = (t.a, t.b);
    t.db.writer
        .call(move |c| {
            c.execute(
                "UPDATE file SET raw_tags = ?1 WHERE id = ?2",
                (genre_only, a),
            )?;
            c.execute("UPDATE file SET raw_tags = ?1 WHERE id = ?2", (complete, b))
        })
        .unwrap();
    let flag_of = |track: i64| -> bool {
        t.db.writer
            .call(|c| crate::search::settle(c, &Mount, 500))
            .unwrap();
        t.db.writer
            .call(move |c| {
                c.query_row(
                    "SELECT flags & ?2 <> 0 FROM track_row WHERE recording_id = ?1",
                    (track, MISSING_TAGS),
                    |r| r.get(0),
                )
            })
            .unwrap()
    };
    assert!(flag_of(t.ta), "no title or artist of its own");
    assert!(!flag_of(t.tb), "the other track is complete");

    t.merge(&t.both(), t.ta).unwrap();
    assert!(
        !flag_of(t.ta),
        "after the merge the kept track has the other file's title and artist"
    );

    assert!(matches!(t.undo(), UndoOutcome::Undone { .. }));
    assert!(flag_of(t.ta), "the Undo takes them away again");
    assert!(!flag_of(t.tb), "and the track that came back is complete");
}
