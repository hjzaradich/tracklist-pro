//! The roles of a track's files, and "a better file is available" (1bC-4).
//! Everything is synthetic: a made-up volume and folder, made-up file names,
//! facts set by SQL, in a migrated database in a temp dir.

use serde_json::json;

use super::*;
use crate::best_file::{
    Art, FileFacts, FolderRole, Format, Place, Quality, Reason, RekordboxLink, VolumeKind,
};
use crate::quality::verdict::Verdict;
use crate::read::Codec;
use crate::tags::TagFields;

/// What kind of audio a made-up file is.
#[derive(Clone, Copy)]
enum Kind {
    Flac,
    Mp3(i64),
    /// A format the DJ app doesn't play.
    Ogg,
}

/// Three columns of a table, as a test reads them.
type Row = (i64, i64, String);

struct Rig {
    writer: Writer,
    next_track_id: std::cell::Cell<i64>,
    // Last: fields drop in order, so the connections close before the folder goes.
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let writer = Writer::open(&crate::write_guard::test_path(
            dir.path(),
            crate::db::DB_FILE_NAME,
        ))
        .unwrap();
        let rig = Rig {
            _dir: dir,
            writer,
            next_track_id: std::cell::Cell::new(1),
        };
        rig.exec(
            "INSERT INTO volume (identity, kind, last_mount_path)
             VALUES ('serial=NTFS-1A2B3C4D', 'external', 'E:\\');
             INSERT INTO music_folder (volume_id, rel_path, rel_path_key)
             VALUES (1, 'Music', 'Music');",
        );
        rig
    }

    fn exec(&self, sql: &str) {
        let sql = sql.to_owned();
        self.writer
            .call(move |c| c.execute_batch(&sql))
            .unwrap_or_else(|e| panic!("{e:?}"));
    }

    fn num(&self, sql: &str) -> i64 {
        let sql = sql.to_owned();
        self.writer
            .call(move |c| c.query_row(&sql, [], |r| r.get(0)))
            .unwrap()
    }

    fn track(&self) -> i64 {
        self.exec("INSERT INTO recording DEFAULT VALUES");
        self.num("SELECT max(id) FROM recording")
    }

    /// A file of `track`, on disk, not yet ranked.
    fn file(&self, track: i64, name: &str, kind: Kind) -> i64 {
        let (sniffed, codec, bitrate) = match kind {
            Kind::Flac => ("flac", Codec::Flac, None),
            Kind::Mp3(kbps) => ("mp3", Codec::Mp3, Some(kbps)),
            Kind::Ogg => ("ogg", Codec::Vorbis, Some(192)),
        };
        let name = name.to_owned();
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO file (music_folder_id, rel_path, rel_path_key, sniffed_format,
                                       codec, bitrate)
                     VALUES (1, ?1, ?1, ?2, ?3, ?4)",
                    rusqlite::params![name, sniffed, codec.as_str(), bitrate],
                )?;
                let file = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO recording_file (recording_id, file_id) VALUES (?1, ?2)",
                    [track, file],
                )?;
                Ok(file)
            })
            .unwrap()
    }

    /// What the grouping pass leaves a track with: one stopgap best.
    fn stopgap_best(&self) {
        self.exec(
            "UPDATE recording_file SET role = 'best'
             WHERE file_id IN (SELECT min(file_id) FROM recording_file GROUP BY recording_id)",
        );
    }

    /// Ranks every due track.
    fn settle(&self) -> Summary {
        pass(&self.writer, &mut || Ok::<(), DbError>(())).unwrap()
    }

    fn role(&self, file: i64) -> String {
        let sql = format!("SELECT role FROM recording_file WHERE file_id = {file}");
        let sql2 = sql.clone();
        self.writer
            .call(move |c| c.query_row(&sql2, [], |r| r.get(0)))
            .unwrap_or_else(|e| panic!("{sql}: {e:?}"))
    }

    fn best_of(&self, track: i64) -> i64 {
        self.num(&format!(
            "SELECT file_id FROM recording_file WHERE recording_id = {track} AND role = 'best'"
        ))
    }

    /// Every file's track and role.
    fn roles(&self) -> Vec<Row> {
        self.writer
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT file_id, recording_id, role FROM recording_file ORDER BY file_id",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                rows.collect()
            })
            .unwrap()
    }

    fn better(&self, track: i64) -> Option<(i64, String)> {
        self.writer
            .call(move |c| {
                c.query_row(
                    "SELECT file_id, step FROM better_file WHERE recording_id = ?1",
                    [track],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
            })
            .unwrap()
    }

    fn library(&self, track: i64, file: i64) {
        self.exec(&format!(
            "INSERT INTO library_track (recording_id, kind, linked_file_id)
             VALUES ({track}, 'linked', {file})"
        ));
    }

    fn linked(&self, track: i64) -> i64 {
        self.num(&format!(
            "SELECT linked_file_id FROM library_track WHERE recording_id = {track}"
        ))
    }

    /// A rekordbox entry matched to `file`.
    fn rekordbox(&self, file: i64, probable: bool, bpm: &str, key: &str) -> i64 {
        let track_id = self.next_track_id.get();
        self.next_track_id.set(track_id + 1);
        let attributes = json!({
            "TrackID": track_id.to_string(),
            "Location": format!("file://localhost/E:/Music/{track_id}.mp3"),
            "AverageBpm": bpm,
            "Tonality": key,
        })
        .to_string();
        let method = if probable { "filename_only" } else { "path" };
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT INTO rekordbox_track
                         (attributes, location_key, read_at, file_id, relink_method,
                          relink_probable, recording_id)
                     VALUES (?1, ?2, '2026-09-30T10:00:00.000Z', ?3, ?4, ?5,
                             (SELECT recording_id FROM recording_file WHERE file_id = ?3))",
                    rusqlite::params![
                        attributes,
                        format!("E:/Music/{track_id}.mp3"),
                        file,
                        method,
                        probable
                    ],
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    /// Everything the roles step writes, to see that a run changed nothing.
    fn written(&self) -> (Vec<Row>, Vec<Row>) {
        let better = self
            .writer
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT recording_id, file_id, step FROM better_file ORDER BY recording_id",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                rows.collect()
            })
            .unwrap();
        (self.roles(), better)
    }

    fn one_best_per_track(&self) -> bool {
        self.num(
            "SELECT count(*) FROM (SELECT recording_id FROM recording_file
                                   GROUP BY recording_id
                                   HAVING count(*) FILTER (WHERE role = 'best') <> 1)",
        ) == 0
    }
}

// ---- the roles --------------------------------------------------------------

#[test]
fn the_best_ranked_file_of_a_track_is_best_and_the_others_are_extra() {
    let rig = Rig::new();
    let track = rig.track();
    // The MP3 has the lower id, so a tie would have made it best.
    let mp3 = rig.file(track, "a.mp3", Kind::Mp3(320));
    let flac = rig.file(track, "b.flac", Kind::Flac);
    let ogg = rig.file(track, "c.ogg", Kind::Ogg);
    rig.stopgap_best();
    rig.settle();
    assert_eq!(rig.role(flac), "best");
    assert_eq!(rig.role(mp3), "extra");
    assert_eq!(rig.role(ogg), "extra");
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 0);
}

#[test]
fn a_file_placed_but_not_yet_ranked_is_undecided_until_the_step_runs() {
    let rig = Rig::new();
    let track = rig.track();
    let file = rig.file(track, "a.mp3", Kind::Mp3(320));
    assert_eq!(rig.role(file), "undecided");
    rig.settle();
    assert_eq!(rig.role(file), "best");
}

#[test]
fn a_file_that_is_gone_hands_best_to_the_copy_on_disk_and_takes_it_back_when_it_returns() {
    let rig = Rig::new();
    let track = rig.track();
    let flac = rig.file(track, "a.flac", Kind::Flac);
    let mp3 = rig.file(track, "b.mp3", Kind::Mp3(320));
    rig.settle();
    assert_eq!(rig.best_of(track), flac);

    rig.exec(&format!("UPDATE file SET present = 0 WHERE id = {flac}"));
    rig.settle();
    assert_eq!(rig.best_of(track), mp3);
    assert_eq!(rig.role(flac), "extra");

    rig.exec(&format!("UPDATE file SET present = 1 WHERE id = {flac}"));
    rig.settle();
    assert_eq!(rig.best_of(track), flac);
}

#[test]
fn a_verdict_that_settles_moves_best_to_the_copy_that_is_not_flagged() {
    let rig = Rig::new();
    let track = rig.track();
    let first = rig.file(track, "a.mp3", Kind::Mp3(320));
    let second = rig.file(track, "b.mp3", Kind::Mp3(320));
    rig.settle();
    assert_eq!(rig.best_of(track), first, "a tie ends at the lower id");

    // The read stage found the first one cut off.
    rig.exec(&format!(
        "UPDATE file SET quality_verdict = 'truncated' WHERE id = {first}"
    ));
    rig.settle();
    assert_eq!(rig.best_of(track), second);
}

#[test]
fn rekordbox_knowing_a_file_breaks_a_tie_between_copies() {
    let rig = Rig::new();
    let track = rig.track();
    let first = rig.file(track, "a.mp3", Kind::Mp3(320));
    let second = rig.file(track, "b.mp3", Kind::Mp3(320));
    rig.settle();
    assert_eq!(rig.best_of(track), first);

    rig.rekordbox(second, false, "128.00", "Am");
    rig.settle();
    assert_eq!(rig.best_of(track), second);
}

#[test]
fn a_file_the_grouping_pass_moves_into_a_track_is_ranked_with_the_others() {
    let rig = Rig::new();
    let (one, two) = (rig.track(), rig.track());
    let mp3 = rig.file(one, "a.mp3", Kind::Mp3(320));
    let flac = rig.file(two, "b.flac", Kind::Flac);
    rig.settle();
    assert_eq!((rig.best_of(one), rig.best_of(two)), (mp3, flac));

    // What a move does (`relocate`): another track, role reset.
    rig.exec(&format!(
        "UPDATE recording_file SET recording_id = {one}, role = 'undecided'
         WHERE file_id = {flac}"
    ));
    assert!(rig.one_best_per_track());
    rig.settle();
    assert_eq!(rig.best_of(one), flac);
    assert_eq!(rig.role(mp3), "extra");
    assert!(rig.one_best_per_track());
}

#[test]
fn a_track_the_grouping_pass_made_is_ranked_whole() {
    let rig = Rig::new();
    // Two files with the same audio and no track yet: one grouping pass puts
    // them together.
    for (name, sniffed) in [("a.mp3", "mp3"), ("b.flac", "flac")] {
        rig.exec(&format!(
            "INSERT INTO file (music_folder_id, rel_path, rel_path_key, sniffed_format, codec,
                               audio_hash)
             VALUES (1, '{name}', '{name}', '{sniffed}', '{sniffed}', x'AA')"
        ));
    }
    rig.writer
        .call(|c| crate::grouping::regroup(c).map(|_| ()))
        .unwrap();
    rig.settle();
    assert_eq!(rig.num("SELECT count(*) FROM recording"), 1);
    assert_eq!(
        rig.best_of(1),
        2,
        "the FLAC, though the MP3 has the lower id"
    );
    assert!(rig.one_best_per_track());
}

#[test]
fn the_roles_are_the_same_whichever_drives_are_plugged_in() {
    let rig = Rig::new();
    // The first drive is external and holds the scan folder. A second one
    // is internal and holds an Inbox folder. The ranking prefers a scan
    // folder over an Inbox before it looks at the kind of volume, so the
    // external copy is the best one; a ranking that also let "is the drive
    // connected" in (or that took an external drive for an unplugged one)
    // would pick the other.
    rig.exec(
        "INSERT INTO volume (identity, kind, last_mount_path)
         VALUES ('serial=NTFS-5E6F7A8B', 'internal', 'F:\\');
         INSERT INTO music_folder (volume_id, rel_path, rel_path_key, role)
         VALUES (2, 'Inbox', 'Inbox', 'inbox');",
    );
    let track = rig.track();
    let inbox = rig.file(track, "a.mp3", Kind::Mp3(320));
    let scan = rig.file(track, "b.mp3", Kind::Mp3(320));
    rig.exec(&format!(
        "UPDATE file SET music_folder_id = 2 WHERE id = {inbox}"
    ));
    rig.settle();
    let before = rig.written();
    assert_eq!(rig.best_of(track), scan);

    // Either drive unplugged, or plugged in again: what the app stores about
    // a volume's mount changes. The track is ranked again each time (the
    // mount change itself asks for nothing), and nothing comes out
    // different.
    for (volume, mount) in [(1, "NULL"), (2, "NULL"), (1, "'E:\\'"), (2, "'F:\\'")] {
        rig.exec(&format!(
            "UPDATE volume SET last_mount_path = {mount},
                               last_seen_at = '2026-10-11T10:00:00.000Z'
             WHERE id = {volume}"
        ));
        assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 0);
        rig.exec(&format!("INSERT INTO best_file_due VALUES ({track})"));
        rig.settle();
        assert_eq!(rig.written(), before, "volume {volume} at {mount}");
    }
}

#[test]
fn a_second_run_on_an_unchanged_database_changes_nothing() {
    let rig = Rig::new();
    for n in 0..4 {
        let track = rig.track();
        let flac = rig.file(track, &format!("{n}.flac"), Kind::Flac);
        rig.file(track, &format!("{n}.mp3"), Kind::Mp3(320));
        if n % 2 == 0 {
            rig.library(track, flac + 1);
        }
    }
    let first = rig.settle();
    assert_eq!(first.ranked, 4);
    let written = rig.written();

    assert_eq!(rig.settle(), Summary::default(), "nothing is due");
    // Even when every track is asked about again, the answer is the same.
    rig.exec("INSERT OR IGNORE INTO best_file_due SELECT id FROM recording");
    let again = rig.settle();
    assert_eq!(again.ranked, 4);
    assert_eq!(again.best_changed, 0);
    assert_eq!(rig.written(), written);
}

#[test]
fn only_the_tracks_that_changed_are_ranked() {
    let rig = Rig::new();
    let (one, two) = (rig.track(), rig.track());
    let flac_one = rig.file(one, "a.flac", Kind::Flac);
    let mp3_one = rig.file(one, "a.mp3", Kind::Mp3(320));
    let flac_two = rig.file(two, "b.flac", Kind::Flac);
    let mp3_two = rig.file(two, "b.mp3", Kind::Mp3(320));
    rig.settle();

    // Track two's roles are wrong, but nothing it is made of changed.
    rig.exec(&format!(
        "UPDATE recording_file SET role = 'extra' WHERE file_id = {flac_two};
         UPDATE recording_file SET role = 'best' WHERE file_id = {mp3_two};"
    ));
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 0);
    // Track one's file went.
    rig.exec(&format!(
        "UPDATE file SET present = 0 WHERE id = {flac_one}"
    ));
    let summary = rig.settle();
    assert_eq!(summary.ranked, 1);
    assert_eq!(rig.best_of(one), mp3_one);
    assert_eq!(rig.best_of(two), mp3_two, "left as it was");
}

#[test]
fn a_run_stopped_between_batches_leaves_every_track_with_one_best_and_the_rest_due() {
    let rig = Rig::new();
    for n in 0..3 {
        let track = rig.track();
        rig.file(track, &format!("{n}.mp3"), Kind::Mp3(320));
        rig.file(track, &format!("{n}.flac"), Kind::Flac);
    }
    rig.stopgap_best();
    let step = rig.writer.call(|c| settle_batch(c, 1)).unwrap();
    assert!(step.more);
    assert!(rig.one_best_per_track());
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 2);
    assert_eq!(rig.best_of(1), 2, "the track that was ranked is whole");

    // A cancel before the next batch is the same: nothing of it is written.
    let stopped: Result<Summary, DbError> = pass(&rig.writer, &mut || Err(DbError::Reentrant));
    assert!(stopped.is_err());
    assert!(rig.one_best_per_track());
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 2);
    rig.settle();
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 0);
    assert_eq!(rig.best_of(2), 4);
    assert_eq!(rig.best_of(3), 6);
}

#[test]
fn a_step_that_swaps_the_best_file_never_leaves_two_best_or_breaks_the_unique_index() {
    let rig = Rig::new();
    let track = rig.track();
    let mp3 = rig.file(track, "a.mp3", Kind::Mp3(320));
    let flac = rig.file(track, "b.flac", Kind::Flac);
    rig.exec(&format!(
        "UPDATE recording_file SET role = 'best' WHERE file_id = {mp3};
         UPDATE recording_file SET role = 'extra' WHERE file_id = {flac};"
    ));
    rig.settle();
    assert_eq!(rig.role(flac), "best");
    assert_eq!(rig.role(mp3), "extra");
    assert!(rig.one_best_per_track());
}

#[test]
fn a_track_that_lost_all_its_files_or_was_deleted_is_cleared_from_the_due_list() {
    let rig = Rig::new();
    let track = rig.track();
    let file = rig.file(track, "a.mp3", Kind::Mp3(320));
    rig.exec(&format!(
        "DELETE FROM recording_file WHERE file_id = {file}"
    ));
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 1);
    rig.exec(&format!("DELETE FROM recording WHERE id = {track}"));
    let summary = rig.settle();
    assert_eq!(summary.best_changed, 0);
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 0);
}

#[test]
fn a_new_roles_version_makes_every_track_due_once() {
    let rig = Rig::new();
    let track = rig.track();
    rig.file(track, "a.mp3", Kind::Mp3(320));
    // The first look at a database writes the version, and ranks all.
    rig.writer.call(|c| ensure_version(c)).unwrap();
    rig.settle();
    rig.writer.call(|c| ensure_version(c)).unwrap();
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 0, "current");

    rig.exec("UPDATE setting SET value = '0' WHERE key = 'file_roles_version'");
    rig.writer.call(|c| ensure_version(c)).unwrap();
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 1);
    rig.settle();
    rig.writer.call(|c| ensure_version(c)).unwrap();
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 0);
}

#[test]
fn the_roles_job_is_asked_for_only_while_a_track_is_due_and_as_a_typed_group_target() {
    let rig = Rig::new();
    let asked = |rig: &Rig| {
        let mut jobs = Vec::new();
        request(&rig.writer, |job| {
            jobs.push(job);
            Ok::<_, DbError>(crate::jobs::JobId(1))
        })
        .unwrap();
        jobs
    };
    assert!(asked(&rig).is_empty(), "nothing is due");

    let track = rig.track();
    rig.file(track, "a.mp3", Kind::Mp3(320));
    let jobs = asked(&rig);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].kind, JobKind::Group);
    assert_eq!(GroupTarget::of(jobs[0].target.as_ref()), GroupTarget::Roles);
    assert_eq!(jobs[0].priority, PRIORITY);

    rig.settle();
    assert!(asked(&rig).is_empty(), "ranked");
}

// ---- the pinned file ---------------------------------------------------------

#[test]
fn the_linked_file_of_a_library_track_never_changes_whichever_file_is_best() {
    let rig = Rig::new();
    let track = rig.track();
    let mp3 = rig.file(track, "a.mp3", Kind::Mp3(320));
    let flac = rig.file(track, "b.flac", Kind::Flac);
    rig.library(track, mp3);
    rig.settle();
    assert_eq!(rig.best_of(track), flac);
    assert_eq!(rig.linked(track), mp3);

    rig.exec(&format!("UPDATE file SET present = 0 WHERE id = {mp3}"));
    rig.settle();
    rig.exec(&format!("UPDATE file SET present = 1 WHERE id = {mp3}"));
    rig.settle();
    assert_eq!(rig.linked(track), mp3);
}

#[test]
fn a_library_tracks_rekordbox_entry_and_linked_file_do_not_change_when_another_file_becomes_best() {
    let rig = Rig::new();
    let track = rig.track();
    let mp3 = rig.file(track, "a.mp3", Kind::Mp3(320));
    rig.rekordbox(mp3, false, "128.00", "Am");
    rig.library(track, mp3);
    rig.settle();
    rig.writer
        .call(|c| crate::attach::attach(c).map(|_| ()))
        .unwrap();
    let state = |rig: &Rig| {
        rig.writer
            .call(|c| {
                let entry: (i64, Option<i64>, bool) = c.query_row(
                    "SELECT id, file_id, relink_probable FROM rekordbox_track",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                let mut stmt =
                    c.prepare("SELECT recording_id, source, bpm, key FROM analysis ORDER BY id")?;
                let rows = stmt.query_map([], |r| {
                    Ok(format!(
                        "{}:{}:{:?}:{:?}",
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<f64>>(2)?,
                        r.get::<_, Option<String>>(3)?
                    ))
                })?;
                let analysis = rows.collect::<rusqlite::Result<Vec<_>>>()?;
                let linked: i64 =
                    c.query_row("SELECT linked_file_id FROM library_track", [], |r| r.get(0))?;
                Ok((entry, analysis, linked))
            })
            .unwrap()
    };
    let before = state(&rig);
    assert_eq!(before.1.len(), 1, "attached");

    // A better copy arrives and becomes the best file.
    let flac = rig.file(track, "b.flac", Kind::Flac);
    rig.settle();
    assert_eq!(rig.best_of(track), flac);
    rig.writer
        .call(|c| crate::attach::attach(c).map(|_| ()))
        .unwrap();
    assert_eq!(state(&rig), before);
}

#[test]
fn the_best_file_changing_an_attach_and_a_relink_settle_in_one_more_run() {
    let rig = Rig::new();
    let track = rig.track();
    let mp3 = rig.file(track, "a.mp3", Kind::Mp3(320));
    let entry = rig.rekordbox(mp3, false, "128.00", "Am");
    rig.settle();
    let attach = |rig: &Rig| {
        rig.writer
            .call(|c| crate::attach::attach(c).map(|_| ()))
            .unwrap();
    };
    attach(&rig);

    // The best file changes: a FLAC arrives. Attach runs.
    let flac = rig.file(track, "b.flac", Kind::Flac);
    let first = rig.settle();
    assert_eq!(first.best_changed, 1);
    attach(&rig);
    // The relink then moves rekordbox's entry to the best copy (it picks the
    // track's best file among exact copies).
    rig.exec(&format!(
        "UPDATE rekordbox_track SET file_id = {flac} WHERE id = {entry}"
    ));
    // The link is a new fact: the track is ranked again, and the best file
    // is the same one, now also known to rekordbox.
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 1);
    let second = rig.settle();
    assert_eq!(second.best_changed, 0);
    attach(&rig);
    let written = rig.written();

    // Nothing is due, and another run changes nothing at all.
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 0);
    assert_eq!(rig.settle(), Summary::default());
    attach(&rig);
    assert_eq!(rig.num("SELECT count(*) FROM best_file_due"), 0);
    assert_eq!(rig.written(), written);
    assert_eq!(rig.best_of(track), flac);
}

// ---- a better file for a Library track ----------------------------------------

#[test]
fn a_library_track_whose_linked_file_is_outranked_in_format_names_the_better_file_and_the_step() {
    let rig = Rig::new();
    let track = rig.track();
    let mp3 = rig.file(track, "a.mp3", Kind::Mp3(320));
    let flac = rig.file(track, "b.flac", Kind::Flac);
    rig.library(track, mp3);
    rig.settle();
    assert_eq!(rig.better(track), Some((flac, "format".to_owned())));
}

#[test]
fn a_track_not_in_the_library_has_no_better_file() {
    let rig = Rig::new();
    let track = rig.track();
    rig.file(track, "a.mp3", Kind::Mp3(320));
    rig.file(track, "b.flac", Kind::Flac);
    rig.settle();
    assert_eq!(rig.better(track), None);
}

#[test]
fn a_library_track_linked_to_its_best_file_has_no_better_file() {
    let rig = Rig::new();
    let track = rig.track();
    rig.file(track, "a.mp3", Kind::Mp3(320));
    let flac = rig.file(track, "b.flac", Kind::Flac);
    rig.library(track, flac);
    rig.settle();
    assert_eq!(rig.better(track), None);
}

#[test]
fn the_better_file_goes_when_the_better_copy_goes_or_the_track_leaves_the_library() {
    let rig = Rig::new();
    let track = rig.track();
    let mp3 = rig.file(track, "a.mp3", Kind::Mp3(320));
    let flac = rig.file(track, "b.flac", Kind::Flac);
    rig.library(track, mp3);
    rig.settle();
    assert!(rig.better(track).is_some());

    rig.exec(&format!("UPDATE file SET present = 0 WHERE id = {flac}"));
    rig.settle();
    assert_eq!(rig.better(track), None, "the linked file is the best now");
    rig.exec(&format!("UPDATE file SET present = 1 WHERE id = {flac}"));
    rig.settle();
    assert!(rig.better(track).is_some());

    rig.exec("DELETE FROM library_track");
    rig.settle();
    assert_eq!(rig.better(track), None);
}

#[test]
fn a_linked_file_that_is_gone_while_a_copy_is_on_disk_has_a_better_file_on_disk_as_the_step() {
    let rig = Rig::new();
    let track = rig.track();
    let linked = rig.file(track, "a.mp3", Kind::Mp3(320));
    let copy = rig.file(track, "b.mp3", Kind::Mp3(320));
    rig.library(track, linked);
    rig.exec(&format!("UPDATE file SET present = 0 WHERE id = {linked}"));
    rig.settle();
    assert_eq!(rig.better(track), Some((copy, "onDisk".to_owned())));
}

#[test]
fn a_linked_file_flagged_by_the_read_stage_has_a_better_file_by_verdict() {
    let rig = Rig::new();
    let track = rig.track();
    let linked = rig.file(track, "a.mp3", Kind::Mp3(320));
    let copy = rig.file(track, "b.mp3", Kind::Mp3(320));
    rig.library(track, linked);
    rig.exec(&format!(
        "UPDATE file SET quality_verdict = 'truncated' WHERE id = {linked}"
    ));
    rig.settle();
    assert_eq!(rig.better(track), Some((copy, "verdict".to_owned())));
}

#[test]
fn a_linked_file_the_dj_app_cannot_play_has_a_better_file_by_playable() {
    let rig = Rig::new();
    let track = rig.track();
    let ogg = rig.file(track, "a.ogg", Kind::Ogg);
    // Nothing is checked on either, so the first step that separates them
    // is whether the DJ app plays the format.
    let mp3 = rig.file(track, "b.mp3", Kind::Mp3(320));
    rig.library(track, ogg);
    rig.settle();
    assert_eq!(rig.better(track), Some((mp3, "playable".to_owned())));
}

#[test]
fn a_copy_with_only_more_tags_is_not_a_better_file() {
    let rig = Rig::new();
    let track = rig.track();
    let linked = rig.file(track, "a.mp3", Kind::Mp3(320));
    let tagged = rig.file(track, "b.mp3", Kind::Mp3(320));
    rig.exec(&format!(
        "UPDATE file SET raw_tags = '{}' WHERE id = {tagged}",
        json!({"id3v2": [
            {"key": "TIT2", "value": {"type": "text", "text": "Made Up"}},
            {"key": "TPE1", "value": {"type": "text", "text": "Nobody"}},
            {"key": "TALB", "value": {"type": "text", "text": "None"}},
        ]})
    ));
    rig.library(track, linked);
    rig.settle();
    assert_eq!(rig.best_of(track), tagged, "it is the best-ranked file");
    assert_eq!(rig.better(track), None, "but its audio is no better");
}

#[test]
fn a_copy_with_only_art_is_not_a_better_file() {
    let rig = Rig::new();
    let track = rig.track();
    let linked = rig.file(track, "a.mp3", Kind::Mp3(320));
    let covered = rig.file(track, "b.mp3", Kind::Mp3(320));
    rig.exec(&format!(
        "UPDATE file SET raw_tags = '{}' WHERE id = {covered}",
        json!({"id3v2": [{"key": "APIC", "value": {"type": "binary", "len": 5000}}]})
    ));
    rig.library(track, linked);
    rig.settle();
    assert_eq!(rig.best_of(track), covered);
    assert_eq!(rig.better(track), None);
}

#[test]
fn a_copy_rekordbox_knows_better_is_not_a_better_file_and_a_tie_is_not_either() {
    let rig = Rig::new();
    let track = rig.track();
    let first = rig.file(track, "a.mp3", Kind::Mp3(320));
    let second = rig.file(track, "b.mp3", Kind::Mp3(320));
    // Linked to the file that loses the tie.
    rig.library(track, second);
    rig.settle();
    assert_eq!(rig.best_of(track), first);
    assert_eq!(rig.better(track), None, "a tie is not better");

    rig.rekordbox(first, false, "128.00", "Am");
    rig.settle();
    assert_eq!(rig.best_of(track), first);
    assert_eq!(rig.better(track), None, "a rekordbox link is not better");
}

// ---- the audio steps, one by one -------------------------------------------------

fn base(id: i64) -> FileFacts {
    FileFacts {
        file_id: id,
        on_disk: true,
        plays_in_dj_app: true,
        format: Format {
            codec: Some(Codec::Mp3),
            kbps: Some(320),
        },
        quality: Quality::NOT_CHECKED,
        tags: TagFields::default(),
        art: None,
        rekordbox: RekordboxLink::Unknown,
        place: Place {
            volume_online: true,
            folder_role: FolderRole::Scan,
            volume_kind: VolumeKind::Internal,
            folder_order: 1,
        },
    }
}

/// The step that makes `better` a better file than `worse`, if any.
fn better_by(better: &FileFacts, worse: &FileFacts) -> Option<BetterStep> {
    audio_step(&why_ahead(better, worse)?)
}

#[test]
fn a_copy_on_disk_is_a_better_file_than_one_that_is_gone() {
    let (better, mut worse) = (base(2), base(1));
    worse.on_disk = false;
    assert_eq!(better_by(&better, &worse), Some(BetterStep::OnDisk));
}

#[test]
fn a_copy_with_a_better_verdict_is_a_better_file() {
    let (mut better, mut worse) = (base(2), base(1));
    better.quality.verdict = Some(Verdict::Ok);
    worse.quality.verdict = Some(Verdict::Truncated);
    assert_eq!(better_by(&better, &worse), Some(BetterStep::Verdict));
}

#[test]
fn a_copy_the_dj_app_plays_is_a_better_file_than_one_it_does_not() {
    let (better, mut worse) = (base(2), base(1));
    worse.plays_in_dj_app = false;
    assert_eq!(better_by(&better, &worse), Some(BetterStep::Playable));
}

#[test]
fn a_copy_with_a_higher_cutoff_is_a_better_file() {
    let (mut better, mut worse) = (base(2), base(1));
    better.quality = Quality {
        verdict: Some(Verdict::Ok),
        cutoff_hz: Some(20_000),
        decode_errors: 0,
    };
    worse.quality = Quality {
        verdict: Some(Verdict::Ok),
        cutoff_hz: Some(16_000),
        decode_errors: 0,
    };
    assert_eq!(better_by(&better, &worse), Some(BetterStep::Cutoff));
}

#[test]
fn a_copy_with_fewer_decode_errors_is_a_better_file() {
    let (mut better, mut worse) = (base(2), base(1));
    better.quality.decode_errors = 0;
    worse.quality.decode_errors = 4;
    assert_eq!(better_by(&better, &worse), Some(BetterStep::DecodeErrors));
}

#[test]
fn a_copy_in_a_better_format_is_a_better_file() {
    let (mut better, worse) = (base(2), base(1));
    better.format = Format {
        codec: Some(Codec::Flac),
        kbps: None,
    };
    assert_eq!(better_by(&better, &worse), Some(BetterStep::Format));
}

#[test]
fn a_copy_that_only_has_more_tags_is_not_a_better_file() {
    let (mut better, worse) = (base(2), base(1));
    better.tags.title = Some("Made Up".to_owned());
    better.tags.artist = Some("Nobody".to_owned());
    assert!(matches!(
        why_ahead(&better, &worse),
        Some(Reason::MoreTags { .. })
    ));
    assert_eq!(better_by(&better, &worse), None);
}

#[test]
fn a_copy_that_only_has_art_a_rekordbox_link_a_better_place_or_a_lower_id_is_not_a_better_file() {
    let worse = base(2);
    let mut art = base(3);
    art.art = Some(Art {
        bytes: 5000,
        width: None,
        height: None,
        hash: None,
    });
    let mut known = base(3);
    known.rekordbox = RekordboxLink::Trusted;
    let mut placed = base(3);
    placed.place.folder_order = 0;
    let tied = base(1);
    for (name, better) in [
        ("art", art),
        ("rekordbox", known),
        ("place", placed),
        ("tie", tied),
    ] {
        assert!(why_ahead(&better, &worse).is_some(), "{name} ranks ahead");
        assert_eq!(better_by(&better, &worse), None, "{name}");
    }
}

// ---- reading the facts ---------------------------------------------------------

#[test]
fn art_is_a_byte_size_only_and_the_largest_picture_counts() {
    let raw = json!({
        "id3v2": [
            {"key": "APIC:front", "value": {"type": "binary", "len": 1200}},
            {"key": "APIC:back", "value": {"type": "binary", "len": 3400}},
            {"key": "GEOB:thing", "value": {"type": "binary", "len": 99999}},
        ],
        "vorbis_comments": [
            {"key": "TITLE", "value": {"type": "text", "text": "x"}},
        ],
    })
    .to_string();
    assert_eq!(
        facts::art_of(&raw),
        Some(Art {
            bytes: 3400,
            width: None,
            height: None,
            hash: None
        })
    );
    assert_eq!(facts::art_of("{}"), None);
    assert_eq!(facts::art_of("not json"), None);
}

#[test]
fn tags_are_read_from_the_stored_frames_and_album_artist_and_isrc_stay_empty() {
    let raw = json!({"id3v2": [
        {"key": "TIT2", "value": {"type": "text", "text": "Made Up"}},
        {"key": "TPE1", "value": {"type": "text", "text": "Nobody"}},
        {"key": "TDRC", "value": {"type": "text", "text": "2019"}},
        {"key": "TRCK", "value": {"type": "text", "text": "3"}},
        {"key": "TPE2", "value": {"type": "text", "text": "Somebody"}},
        {"key": "TSRC", "value": {"type": "text", "text": "XX0000000001"}},
    ]})
    .to_string();
    let tags = facts::tags_of(&raw);
    assert_eq!(tags.title.as_deref(), Some("Made Up"));
    assert_eq!(tags.artist.as_deref(), Some("Nobody"));
    assert_eq!(tags.year, Some(2019));
    assert_eq!(tags.track_no, Some(3));
    assert_eq!((tags.album_artist, tags.isrc), (None, None));
}

// ---- a measurement of other audio ----------------------------------------------

/// Gives a file this audio hash and a measurement of it with this cutoff.
fn measured(rig: &Rig, file: i64, hash_byte: u8, cutoff_hz: i64) {
    let hash = format!("{hash_byte:02x}").repeat(34);
    rig.exec(&format!(
        "UPDATE file SET audio_hash = x'{hash}' WHERE id = {file};
         INSERT INTO file_quality
             (file_id, audio_hash, method_version, cutoff_hz, decoded_ms, header_ms, ended)
         VALUES ({file}, x'{hash}', {version}, {cutoff_hz}, 1000, 1000, 'complete');",
        version = crate::quality::VERSION
    ));
}

fn cutoff_of(rig: &Rig, track: i64, file: i64) -> Option<u32> {
    rig.writer
        .call(move |c| facts::load(c, &[track]))
        .unwrap()
        .remove(&track)
        .unwrap()
        .into_iter()
        .find(|l| l.facts.file_id == file)
        .unwrap()
        .facts
        .quality
        .cutoff_hz
}

#[test]
fn a_cutoff_measured_of_other_audio_reads_as_none_and_the_other_copy_is_not_outranked_by_it() {
    let rig = Rig::new();
    let track = rig.track();
    let first = rig.file(track, "a.mp3", Kind::Mp3(320));
    let second = rig.file(track, "b.mp3", Kind::Mp3(320));
    measured(&rig, first, 0x01, 20_000);
    measured(&rig, second, 0x02, 18_000);
    rig.settle();
    assert_eq!(cutoff_of(&rig, track, first), Some(20_000));
    assert_eq!(rig.best_of(track), first, "the higher cutoff");

    // The first file now holds other audio; its measurement is of the old.
    rig.exec(&format!(
        "UPDATE file SET audio_hash = x'{}' WHERE id = {first}",
        "03".repeat(34)
    ));
    rig.settle();
    assert_eq!(cutoff_of(&rig, track, first), None);
    assert_eq!(cutoff_of(&rig, track, second), Some(18_000));
    assert_eq!(
        rig.best_of(track),
        second,
        "not outranked on the old number"
    );
}

#[test]
fn the_better_file_steps_are_exactly_the_ones_the_table_accepts() {
    let rig = Rig::new();
    let track = rig.track();
    rig.file(track, "a.mp3", Kind::Mp3(320));
    let other = rig.file(track, "b.mp3", Kind::Mp3(320));
    let accepted = |name: &str| -> bool {
        let sql = format!(
            "INSERT OR REPLACE INTO better_file (recording_id, file_id, step)
             VALUES ({track}, {other}, '{name}')"
        );
        rig.writer.call(move |c| c.execute_batch(&sql)).is_ok()
    };
    for step in BetterStep::ALL {
        assert!(accepted(step.name()), "{step:?}");
        assert_eq!(BetterStep::parse(step.name()), Some(step));
    }
    // Every other step of the ranking is refused by the table, and no
    // better-file step comes from them.
    for step in [
        crate::best_file::Step::Tags,
        crate::best_file::Step::Art,
        crate::best_file::Step::Rekordbox,
        crate::best_file::Step::Place,
        crate::best_file::Step::Tied,
    ] {
        let name = serde_json::to_value(step).unwrap();
        let name = name.as_str().unwrap();
        assert!(!accepted(name), "{name}");
        assert_eq!(BetterStep::of(step), None);
    }
}

// ---- the job ------------------------------------------------------------------

#[test]
fn a_job_that_ends_asks_for_the_roles_and_they_are_worked_out_in_the_background() {
    use crate::grouping::Grouper;
    use crate::jobs::{JobContext, JobQueue};
    use crate::scan::chain::{after_group, after_other};

    let rig = Rig::new();
    let track = rig.track();
    let mp3 = rig.file(track, "a.mp3", Kind::Mp3(320));
    let flac = rig.file(track, "b.flac", Kind::Flac);
    rig.library(track, mp3);
    let queue = JobQueue::builder(rig.writer.clone())
        // Any job the chain wraps asks for the roles when it ends.
        .handler(JobKind::Relink, after_other(|_: &JobContext| Ok(())))
        .handler(JobKind::Group, after_group(Grouper::default()))
        .start()
        .unwrap();
    queue.enqueue(NewJob::new(JobKind::Relink)).unwrap();

    let begun = std::time::Instant::now();
    while rig.num("SELECT count(*) FROM best_file_due") > 0 {
        assert!(
            begun.elapsed() < std::time::Duration::from_secs(30),
            "the roles were never worked out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    queue.shutdown();
    assert_eq!(rig.best_of(track), flac);
    assert_eq!(rig.better(track), Some((flac, "format".to_owned())));
    assert_eq!(rig.linked(track), mp3);
}
