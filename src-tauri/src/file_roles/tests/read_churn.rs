//! 1cD-X4: a rekordbox read makes due only the tracks whose rekordbox link
//! changed. Most reads here are the real one (`replace_snapshot`) followed by
//! the real relink, over made-up files `a.mp3`, `b.mp3`, ... in `E:\Music`.

use super::*;
use crate::rekordbox::store::{replace_snapshot, SnapshotRow, SnapshotRows};
use crate::relink::Mounted;

fn entry(track_id: i64, name: &str) -> SnapshotRow {
    let location = format!("file://localhost/E:/Music/{name}");
    SnapshotRow {
        track_id,
        attributes: json!({"TrackID": track_id.to_string(), "Location": location}).to_string(),
        location_key: format!("E:/MUSIC/{}", name.to_ascii_uppercase()),
        tempo: "[]".into(),
        position_marks: "[]".into(),
        my_tags: "[]".into(),
        playlists: "[]".into(),
        streaming: false,
    }
}

impl Rig {
    /// A read of an export holding `names` (complete), as the read job
    /// stores it, then the relink job's matching.
    fn read(&self, names: &[&str]) {
        self.read_only(names);
        self.relink();
    }

    fn read_only(&self, names: &[&str]) {
        let rows = SnapshotRows {
            rows: names
                .iter()
                .enumerate()
                .map(|(i, name)| entry(i as i64 + 1, name))
                .collect(),
            complete: true,
            ..SnapshotRows::default()
        };
        self.writer
            .call(move |c| replace_snapshot(c, &rows, |_, _, _| Ok(())))
            .unwrap();
    }

    fn relink(&self) {
        let mounted = Mounted::new([("serial=NTFS-1A2B3C4D", r"E:\")]);
        self.writer
            .call(move |c| crate::relink::relink(c, &mounted))
            .unwrap();
    }

    fn due(&self) -> Vec<i64> {
        self.writer
            .call(|c| {
                let mut stmt =
                    c.prepare("SELECT recording_id FROM best_file_due ORDER BY recording_id")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                rows.collect()
            })
            .unwrap()
    }

    fn held(&self) -> bool {
        self.num("SELECT count(*) FROM setting WHERE key = 'file_roles_churn'") == 1
    }

    /// Three tracks of one MP3 each, all read from rekordbox and ranked, so
    /// nothing is due.
    fn matched() -> (Rig, [i64; 3], [i64; 3]) {
        let rig = Rig::new();
        let tracks = [rig.track(), rig.track(), rig.track()];
        let files = [
            rig.file(tracks[0], "a.mp3", Kind::Mp3(320)),
            rig.file(tracks[1], "b.mp3", Kind::Mp3(320)),
            rig.file(tracks[2], "c.mp3", Kind::Mp3(320)),
        ];
        rig.read(&["a.mp3", "b.mp3", "c.mp3"]);
        assert_eq!(
            rig.num("SELECT count(*) FROM rekordbox_track WHERE file_id IS NOT NULL"),
            3,
            "the made-up files are matched by path"
        );
        rig.settle();
        assert_eq!(rig.due(), Vec::<i64>::new());
        (rig, tracks, files)
    }
}

#[test]
fn a_read_that_leaves_every_link_as_it_was_makes_no_track_due() {
    let (rig, _, _) = Rig::matched();
    let written = rig.written();
    rig.read(&["a.mp3", "b.mp3", "c.mp3"]);
    assert_eq!(rig.due(), Vec::<i64>::new());
    assert!(!rig.held(), "the relink settled what the read held");
    assert_eq!(rig.settle(), Summary::default());
    assert_eq!(rig.written(), written);
}

#[test]
fn a_read_that_loses_one_link_makes_only_that_track_due() {
    let (rig, tracks, _) = Rig::matched();
    rig.read(&["a.mp3", "c.mp3"]);
    assert_eq!(rig.due(), vec![tracks[1]]);
    assert!(!rig.held());
}

#[test]
fn a_read_that_makes_a_link_makes_only_that_track_due() {
    let (rig, tracks, _) = Rig::matched();
    rig.read(&["a.mp3", "b.mp3"]);
    rig.settle();
    rig.read(&["a.mp3", "b.mp3", "c.mp3"]);
    assert_eq!(rig.due(), vec![tracks[2]]);
}

#[test]
fn a_read_that_empties_the_snapshot_makes_every_linked_track_due() {
    let (rig, tracks, _) = Rig::matched();
    rig.read(&[]);
    assert_eq!(rig.due(), tracks.to_vec());
    assert!(!rig.held());
}

#[test]
fn a_link_that_turns_probable_makes_its_track_due_and_no_other() {
    let (rig, tracks, files) = Rig::matched();
    // The read, then a relink that finds the entries again, but the second
    // one only probably (as a filename-only match would).
    rig.read_only(&["a.mp3", "b.mp3", "c.mp3"]);
    rig.writer
        .call(move |c| {
            let tx = c.transaction()?;
            let taken = churn::take(&tx)?;
            for (file, name, probable) in [
                (files[0], "A", 0),
                (files[1], "B", 1),
                (files[2], "C", 0),
            ] {
                tx.execute(
                    "UPDATE rekordbox_track SET file_id = ?1, relink_probable = ?2,
                         relink_method = CASE WHEN ?2 THEN 'filename_only' ELSE 'path' END
                     WHERE location_key = ?3",
                    rusqlite::params![file, probable, format!("E:/MUSIC/{name}.MP3")],
                )?;
            }
            churn::settle(&tx, taken)?;
            tx.commit()
        })
        .unwrap();
    assert_eq!(rig.due(), vec![tracks[1]]);
}

#[test]
fn a_track_made_due_by_something_else_while_a_read_waits_for_its_relink_stays_due() {
    let (rig, tracks, files) = Rig::matched();
    rig.read_only(&["a.mp3", "b.mp3", "c.mp3"]);
    assert!(rig.held());
    assert_eq!(rig.due(), Vec::<i64>::new(), "the read made nothing due");
    // A track enters the Library: its better file is worked out again.
    rig.library(tracks[0], files[0]);
    assert_eq!(rig.due(), vec![tracks[0]]);
    rig.relink();
    assert_eq!(rig.due(), vec![tracks[0]]);
}

#[test]
fn ranking_while_a_read_waits_for_its_relink_puts_every_held_track_back() {
    let (rig, tracks, _) = Rig::matched();
    rig.read_only(&["a.mp3", "b.mp3", "c.mp3"]);
    assert!(rig.held());
    // A roles run starts between the read and its relink, when the links are
    // cleared: every track it held is due again, as the triggers had it, and
    // ranked on what the database holds.
    let ranked = rig.settle();
    assert_eq!(ranked.ranked, 3);
    assert!(!rig.held());
    // The relink then finds the links again and, as it always did, their
    // tracks are due once more.
    rig.relink();
    assert_eq!(rig.due(), tracks.to_vec());
}

#[test]
fn the_start_of_the_app_puts_the_tracks_a_read_held_back_in_the_list_again() {
    let (rig, tracks, _) = Rig::matched();
    rig.read_only(&["a.mp3", "b.mp3", "c.mp3"]);
    assert_eq!(rig.due(), Vec::<i64>::new());
    // The app ended before the relink ran; the next start restores them.
    rig.writer.call(|c| churn::restore(c)).unwrap();
    assert_eq!(rig.due(), tracks.to_vec());
    assert!(!rig.held());
}

#[test]
fn a_hold_nobody_can_read_makes_every_track_due() {
    let (rig, tracks, _) = Rig::matched();
    rig.read_only(&["a.mp3", "b.mp3", "c.mp3"]);
    rig.exec("UPDATE setting SET value = '\"not a hold\"' WHERE key = 'file_roles_churn'");
    rig.relink();
    assert_eq!(rig.due(), tracks.to_vec());
    assert!(!rig.held());
}

#[test]
fn two_reads_before_a_relink_compare_with_the_links_before_the_first() {
    let (rig, tracks, _) = Rig::matched();
    rig.read_only(&["a.mp3", "b.mp3", "c.mp3"]);
    rig.read_only(&["a.mp3", "b.mp3"]);
    rig.relink();
    assert_eq!(rig.due(), vec![tracks[2]]);
}

#[test]
fn what_a_file_has_of_rekordbox_is_what_the_ranking_reads() {
    // The status the hold compares is the one `facts` ranks with: a trusted
    // entry beats a probable one, and a file with none has none.
    let rig = Rig::new();
    let track = rig.track();
    let file = rig.file(track, "a.mp3", Kind::Mp3(320));
    let status = |rig: &Rig| rig.writer.call(|c| churn::status_of(c)).unwrap();
    assert!(status(&rig).is_empty());
    rig.rekordbox(file, true, "128.00", "Am");
    assert_eq!(status(&rig)[&file], 1);
    rig.rekordbox(file, false, "128.00", "Am");
    assert_eq!(status(&rig)[&file], 2);
}
